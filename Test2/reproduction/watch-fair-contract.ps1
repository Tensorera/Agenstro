param(
    [string]$RunName = "fair-contract-restart",
    [int]$PollSeconds = 2
)

$ErrorActionPreference = "Stop"

$reproductionRoot = $PSScriptRoot
$stateRoot = Join-Path $reproductionRoot ".clef-state"
$workfolderName = "output-agent-$RunName"
$workfolder = Join-Path $reproductionRoot $workfolderName
$statePattern = "$workfolderName-*"
$lastSequence = 0
$terminalReported = $false

function Find-WorkflowTrace {
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
    return Get-ChildItem -LiteralPath (Join-Path $workfolder "9900_run") `
        -Recurse -Filter "run-summary.json" -File `
        -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
}

function Test-RunProcess {
    $needle = "*$workfolderName*"
    return $null -ne (
        Get-CimInstance Win32_Process |
        Where-Object { $_.CommandLine -like $needle } |
        Select-Object -First 1
    )
}

Write-Host "Watching $workfolderName (Ctrl+C only stops this watcher)."

while (-not $terminalReported) {
    $trace = Find-WorkflowTrace
    if ($null -ne $trace) {
        foreach ($line in Get-Content -LiteralPath $trace.FullName) {
            try {
                $record = $line | ConvertFrom-Json -Depth 100
            }
            catch {
                continue
            }
            if ($record.sequence -le $lastSequence) {
                continue
            }
            $lastSequence = $record.sequence
            $progress = $record.data.progress
            if ($null -eq $progress) {
                continue
            }
            $summary = $progress.summary
            $done = @($summary.succeeded_tasks).Count +
                @($summary.failed_tasks).Count +
                @($summary.skipped_tasks).Count
            $task = if ($record.task_id) {
                " task=$($record.task_id)"
            }
            else {
                ""
            }
            $attempt = if ($null -ne $progress.attempt) {
                " attempt=$($progress.attempt)"
            }
            else {
                ""
            }
            $active = if (@($summary.active_tasks).Count) {
                " active=$(@($summary.active_tasks) -join ',')"
            }
            else {
                ""
            }
            Write-Host (
                "[{0:D4} +{1:N2}s] {2}{3}{4} | done={5}/{6}{7}" -f
                $record.sequence,
                [double]$progress.elapsed_seconds,
                $progress.event,
                $task,
                $attempt,
                $done,
                $summary.total_tasks,
                $active
            )
        }
    }

    $runSummary = Find-RunSummary
    if ($null -ne $runSummary) {
        $result = Get-Content -LiteralPath $runSummary.FullName -Raw |
            ConvertFrom-Json -Depth 100
        Write-Host ""
        Write-Host (
            "TERMINAL: {0}; run_id={1}; summary={2}" -f
            $result.workflow_state,
            $result.run_id,
            $runSummary.FullName
        )
        $terminalReported = $true
        continue
    }

    if (-not (Test-RunProcess)) {
        Write-Warning (
            "Process exited without run-summary.json. The run is incomplete. " +
            "Last durable sequence=$lastSequence; trace=$($trace.FullName)"
        )
        exit 2
    }

    Start-Sleep -Seconds $PollSeconds
}
