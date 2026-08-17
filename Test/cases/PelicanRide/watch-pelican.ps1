param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$')]
    [string]$RunName,
    [ValidateRange(1, 60)]
    [int]$PollSeconds = 2,
    [switch]$NoClear,
    [switch]$Once
)

$ErrorActionPreference = "Stop"
$caseRoot = $PSScriptRoot
$runsRoot = Join-Path $caseRoot "runs"
$stateRoot = Join-Path $caseRoot ".clef-state"
$workfolderName = "output-$RunName"
$workfolder = Join-Path $runsRoot $workfolderName
$statePattern = "$workfolderName-*"
$stdoutLog = Join-Path $runsRoot "live-$RunName.stdout.log"
$summaryLog = Join-Path $runsRoot "live-$RunName.summary.log"
$completionLog = Join-Path $runsRoot "live-$RunName.completion.json"
$statusPath = Join-Path $workfolder "run-status.json"
$expectedExe = Join-Path $workfolder "0400_delivery\delivery-bundle\PelicanRide.exe"
$terminalStates = @(
    "SUCCEEDED", "FAILED", "PARTIAL", "BLOCKED",
    "CANCELLED", "TIMED_OUT", "INTERRUPTED"
)
$processMissingPolls = 0

$friendly = [ordered]@{
    "compose-vector-art-direction" = "Vector art direction"
    "compose-gameplay-architecture" = "Gameplay architecture"
    "build-playable-prototype" = "Playable prototype"
    "review-playability-and-visuals" = "Independent review"
    "polish-and-package-exe" = "Polish + single EXE"
}

function Find-WorkflowTrace {
    if (-not (Test-Path -LiteralPath $stateRoot)) {
        return $null
    }
    $stateDirectory = Get-ChildItem -LiteralPath $stateRoot -Directory `
        -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like $statePattern } |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if ($null -eq $stateDirectory) {
        return $null
    }
    return Get-ChildItem -LiteralPath $stateDirectory.FullName `
        -Recurse -Filter "workflow.jsonl" -File `
        -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
}

function Find-RunSummary {
    $runRoot = Join-Path $workfolder "9900_run"
    if (-not (Test-Path -LiteralPath $runRoot)) {
        return $null
    }
    return Get-ChildItem -LiteralPath $runRoot `
        -Recurse -Filter "run-summary.json" -File `
        -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
}

function Test-RunProcess {
    $needle = "*$workfolderName*"
    return $null -ne (
        Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
        Where-Object {
            $_.CommandLine -like $needle -and
            (
                $_.CommandLine -like "*PelicanRide*run.py*" -or
                $_.CommandLine -like "*run-pelican-detached.ps1*"
            )
        } |
        Select-Object -First 1
    )
}

function Read-TraceRecords([System.IO.FileInfo]$Trace) {
    if ($null -eq $Trace) {
        return @()
    }
    $records = @()
    foreach ($line in Get-Content -LiteralPath $Trace.FullName -Tail 160) {
        try {
            $record = $line | ConvertFrom-Json -Depth 100
            if ($null -ne $record) {
                $records += $record
            }
        }
        catch {
            continue
        }
    }
    return $records
}

function State-Color([string]$State) {
    switch ($State) {
        "SUCCEEDED" { return "Green" }
        "FAILED" { return "Red" }
        "BLOCKED" { return "Red" }
        "INTERRUPTED" { return "Red" }
        "REPAIRING" { return "Yellow" }
        "VERIFYING" { return "Magenta" }
        "PUBLISHING" { return "Cyan" }
        "RUNNING" { return "Cyan" }
        "SCHEDULED" { return "Blue" }
        "SKIPPED" { return "DarkGray" }
        default { return "Gray" }
    }
}

function Format-Elapsed([double]$Seconds) {
    $span = [TimeSpan]::FromSeconds([Math]::Max(0, $Seconds))
    if ($span.TotalHours -ge 1) {
        return "{0:00}:{1:00}:{2:00}" -f `
            [int]$span.TotalHours, $span.Minutes, $span.Seconds
    }
    return "{0:00}:{1:00}" -f $span.Minutes, $span.Seconds
}

function Event-Label([string]$EventName) {
    switch ($EventName) {
        "workflow_started" { return "workflow composed" }
        "task_scheduled" { return "cell scheduled" }
        "attempt_started" { return "cell run started" }
        "task_started" { return "cell run started" }
        "agent_session_started" { return "agent turn started" }
        "agent_session_completed" { return "agent turn completed" }
        "verification_started" { return "verification started" }
        "verification_passed" { return "verification passed" }
        "verification_failed" { return "verification failed" }
        "repair_started" { return "repair cell started" }
        "artifact_published" { return "artifact published" }
        "task_succeeded" { return "cell succeeded" }
        "task_failed" { return "cell failed" }
        "workflow_completed" { return "workflow completed" }
        default { return $EventName.Replace("_", " ") }
    }
}

$terminal = $false
while (-not $terminal) {
    $trace = Find-WorkflowTrace
    $records = @(Read-TraceRecords $trace)
    $latestProgress = $records |
        Where-Object { $null -ne $_.data.progress.summary } |
        Select-Object -Last 1
    $summary = if ($null -ne $latestProgress) {
        $latestProgress.data.progress.summary
    }
    else {
        $null
    }
    $processRunning = Test-RunProcess

    if (-not $NoClear) {
        Clear-Host
    }
    Write-Host "╭──────────────────────────────────────────────────────────────────────────────╮" -ForegroundColor DarkCyan
    Write-Host "│  PELICAN RIDE  •  CLEF LIVE                                               │" -ForegroundColor Cyan
    Write-Host "╰──────────────────────────────────────────────────────────────────────────────╯" -ForegroundColor DarkCyan
    Write-Host ("  run   {0}" -f $RunName) -ForegroundColor White

    if ($null -eq $summary) {
        $bootstrapState = "STARTING"
        if (Test-Path -LiteralPath $statusPath) {
            try {
                $bootstrapState = (
                    Get-Content -LiteralPath $statusPath -Raw |
                    ConvertFrom-Json -Depth 30
                ).state
            }
            catch {
                $bootstrapState = "STARTING"
            }
        }
        Write-Host ("  state {0}  • waiting for durable workflow trace" -f $bootstrapState) -ForegroundColor Yellow
        Write-Host ""
        Write-Host "  SDK / WORKFLOW" -ForegroundColor DarkCyan
        Write-Host "  ──────────────────────────────────────────────────────────────────────────"
        Write-Host "  Initializing profile, compiler and trace writer…"
    }
    else {
        $done = @($summary.succeeded_tasks).Count +
            @($summary.failed_tasks).Count +
            @($summary.skipped_tasks).Count
        $total = [Math]::Max(1, [int]$summary.total_tasks)
        $filled = [Math]::Min(32, [int][Math]::Floor(32 * $done / $total))
        $bar = ("━" * $filled) + ("─" * (32 - $filled))
        $elapsedSeconds = [double]$summary.elapsed_seconds
        try {
            # ConvertFrom-Json may already materialize ISO timestamps as a UTC
            # DateTime. Cast directly so string localization cannot discard Z.
            $startedAt = [DateTimeOffset]$summary.started_at
            if ($terminalStates -notcontains [string]$summary.state) {
                $liveElapsed = (
                    [DateTimeOffset]::UtcNow - $startedAt.ToUniversalTime()
                ).TotalSeconds
                $elapsedSeconds = [Math]::Max($elapsedSeconds, $liveElapsed)
            }
        }
        catch {
            # Durable elapsed_seconds remains authoritative if timestamp parsing
            # is unavailable in an older PowerShell host.
        }
        $elapsed = Format-Elapsed $elapsedSeconds
        $stateColor = State-Color ([string]$summary.state)

        Write-Host -NoNewline "  state "
        Write-Host -NoNewline $summary.state -ForegroundColor $stateColor
        Write-Host ("  • {0}  • {1}/{2} cells  • {3} artifacts" -f `
            $elapsed, $done, $total, $summary.published_outputs)
        Write-Host -NoNewline "  "
        Write-Host -NoNewline $bar -ForegroundColor Cyan
        Write-Host ("  {0,3}%" -f [int](100 * $done / $total))
        Write-Host ""
        Write-Host "  SDK / WORKFLOW" -ForegroundColor DarkCyan
        Write-Host "  ──────────────────────────────────────────────────────────────────────────"
        Write-Host (
            "  attempts {0}   repairs {1}   verify ✓ {2} / ✕ {3}   process {4}" -f
            $summary.attempts,
            $summary.repair_turns,
            $summary.verification_passes,
            $summary.verification_failures,
            $(if ($processRunning) { "alive" } else { "exited" })
        )
        Write-Host ""
        Write-Host "  STUDIO / CELL LAYER" -ForegroundColor DarkCyan
        Write-Host "  ──────────────────────────────────────────────────────────────────────────"
        foreach ($taskId in $friendly.Keys) {
            $task = $summary.tasks.$taskId
            if ($null -eq $task) {
                $taskState = "PENDING"
                $attempt = 0
                $turn = "—"
                $repairs = 0
                $taskElapsed = "00:00"
            }
            else {
                $taskState = [string]$task.state
                $attempt = [int]$task.attempt
                $turn = if ($task.turn_kind) { [string]$task.turn_kind } else { "—" }
                $repairs = [int]$task.repair_turns
                $taskElapsedSeconds = [double]$task.elapsed_seconds
                if ($taskState -in @(
                    "SCHEDULED", "RUNNING", "VERIFYING",
                    "REPAIRING", "PUBLISHING"
                )) {
                    $taskStartRecord = $records |
                        Where-Object {
                            $_.task_id -eq $taskId -and
                            $_.event -in @("task_started", "attempt_started")
                        } |
                        Select-Object -Last 1
                    if ($null -ne $taskStartRecord) {
                        try {
                            $taskStartedAt = [DateTimeOffset]$taskStartRecord.timestamp
                            $liveTaskElapsed = (
                                [DateTimeOffset]::UtcNow -
                                $taskStartedAt.ToUniversalTime()
                            ).TotalSeconds
                            $taskElapsedSeconds = [Math]::Max(
                                $taskElapsedSeconds,
                                $liveTaskElapsed
                            )
                        }
                        catch {
                            # Fall back to the last durable per-task duration.
                        }
                    }
                }
                $taskElapsed = Format-Elapsed $taskElapsedSeconds
            }
            $icon = switch ($taskState) {
                "SUCCEEDED" { "●" }
                "FAILED" { "×" }
                "SKIPPED" { "–" }
                "PENDING" { "○" }
                default { "◆" }
            }
            Write-Host -NoNewline ("  {0} " -f $icon) -ForegroundColor (State-Color $taskState)
            Write-Host -NoNewline ("{0,-28}" -f $friendly[$taskId])
            Write-Host -NoNewline (" {0,-11}" -f $taskState) -ForegroundColor (State-Color $taskState)
            Write-Host (" a{0}  {1,-6}  repair {2}  {3}" -f `
                $attempt, $turn, $repairs, $taskElapsed)
        }

        Write-Host ""
        Write-Host "  RECENT DURABLE EVENTS" -ForegroundColor DarkCyan
        Write-Host "  ──────────────────────────────────────────────────────────────────────────"
        $events = @($records | Select-Object -Last 7)
        foreach ($record in $events) {
            $progress = $record.data.progress
            $offset = if ($null -ne $progress) {
                Format-Elapsed ([double]$progress.elapsed_seconds)
            }
            else {
                "--:--"
            }
            $taskName = if ($record.task_id -and $friendly[$record.task_id]) {
                $friendly[$record.task_id]
            }
            elseif ($record.task_id) {
                $record.task_id
            }
            else {
                "workflow"
            }
            $color = if ($record.level -eq "warning") {
                "Yellow"
            }
            elseif ($record.level -eq "error") {
                "Red"
            }
            else {
                "Gray"
            }
            Write-Host (
                "  {0,5}  {1,-24}  {2}" -f
                $offset,
                (Event-Label ([string]$record.event)),
                $taskName
            ) -ForegroundColor $color
        }
    }

    Write-Host ""
    if (Test-Path -LiteralPath $expectedExe) {
        $exe = Get-Item -LiteralPath $expectedExe
        Write-Host (
            "  EXE READY  {0:N1} MB  {1}" -f
            ($exe.Length / 1MB),
            $exe.FullName
        ) -ForegroundColor Green
    }
    else {
        Write-Host "  EXE TARGET " -NoNewline -ForegroundColor DarkGray
        Write-Host $expectedExe -ForegroundColor Gray
    }
    Write-Host "  Ctrl+C stops only this monitor; the detached run keeps working." -ForegroundColor DarkGray

    $runSummary = Find-RunSummary
    if ($null -ne $runSummary) {
        $result = Get-Content -LiteralPath $runSummary.FullName -Raw |
            ConvertFrom-Json -Depth 100
        if ($terminalStates -contains [string]$result.workflow_state) {
            $terminal = $true
            Write-Host ""
            Write-Host (
                "  TERMINAL  {0}  • run_id {1}" -f
                $result.workflow_state,
                $result.run_id
            ) -ForegroundColor (State-Color ([string]$result.workflow_state))
            Write-Host ("  summary   {0}" -f $runSummary.FullName)
            continue
        }
    }

    if ($processRunning) {
        $processMissingPolls = 0
    }
    else {
        $processMissingPolls++
    }

    if (-not $processRunning -and (Test-Path -LiteralPath $completionLog)) {
        $completion = Get-Content -LiteralPath $completionLog -Raw |
            ConvertFrom-Json -Depth 30
        if ([int]$completion.exit_code -ne 0) {
            Write-Host ""
            Write-Host (
                "  PROCESS EXITED with code {0}; inspect {1}" -f
                $completion.exit_code,
                $summaryLog
            ) -ForegroundColor Red
            $terminal = $true
            continue
        }
        if ($processMissingPolls -ge 3) {
            Write-Host ""
            Write-Host (
                "  INCONSISTENT RUN: launcher exited 0 but no durable " +
                "run-summary.json was published."
            ) -ForegroundColor Red
            Write-Host ("  completion {0}" -f $completionLog)
            $terminal = $true
            continue
        }
    }
    elseif (-not $processRunning -and $processMissingPolls -ge 3) {
        Write-Host ""
        Write-Host (
            "  ORPHANED RUN: no live process, completion record, or " +
            "terminal run summary. Last durable trace is preserved."
        ) -ForegroundColor Red
        Write-Host (
            "  trace      {0}" -f
            $(if ($null -ne $trace) { $trace.FullName } else { "<not created>" })
        )
        Write-Host ("  status     {0}" -f $statusPath)
        $terminal = $true
        continue
    }

    if ($Once) {
        return
    }
    Start-Sleep -Seconds $PollSeconds
}
