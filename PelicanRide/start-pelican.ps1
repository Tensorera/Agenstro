param(
    [string]$RunName = ("pelican-{0}" -f (Get-Date -Format "yyyyMMdd-HHmmss"))
)

$ErrorActionPreference = "Stop"
if ($RunName -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$') {
    throw "RunName must match ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$"
}

$caseRoot = $PSScriptRoot
$runsRoot = Join-Path $caseRoot "runs"
$workfolder = Join-Path $runsRoot "output-$RunName"
$launcher = Join-Path $caseRoot "run-pelican-detached.ps1"
$pidRecord = Join-Path $runsRoot "live-$RunName.pid.json"
New-Item -ItemType Directory -Path $runsRoot -Force | Out-Null

foreach ($target in @($workfolder, $pidRecord)) {
    if (Test-Path -LiteralPath $target) {
        throw "Run target already exists: $target"
    }
}

$pwsh = if (Get-Command pwsh.exe -ErrorAction SilentlyContinue) {
    (Get-Command pwsh.exe).Source
}
else {
    (Get-Command powershell.exe -ErrorAction Stop).Source
}
$script = "& '$($launcher.Replace("'", "''"))' -RunName '$RunName'"
$encoded = [Convert]::ToBase64String(
    [Text.Encoding]::Unicode.GetBytes($script)
)
$commandLine = (
    '"{0}" -NoLogo -NoProfile -NonInteractive -EncodedCommand {1}' -f
    $pwsh,
    $encoded
)
$created = Invoke-CimMethod `
    -ClassName Win32_Process `
    -MethodName Create `
    -Arguments @{ CommandLine = $commandLine }
if ($created.ReturnValue -ne 0 -or -not $created.ProcessId) {
    throw "Win32_Process.Create failed with code $($created.ReturnValue)"
}

[ordered]@{
    schema_version = "1.0"
    run_name = $RunName
    process_id = [int]$created.ProcessId
    launched_at_utc = [DateTime]::UtcNow.ToString("o")
    workfolder = $workfolder
    watcher = (Join-Path $caseRoot "watch-pelican.ps1")
} | ConvertTo-Json -Depth 8 |
    Set-Content -LiteralPath $pidRecord -Encoding utf8

Write-Host ""
Write-Host "  Pelican Ride run launched" -ForegroundColor Cyan
Write-Host "  Run name : $RunName"
Write-Host "  PID      : $($created.ProcessId)"
Write-Host "  Workspace: $workfolder"
Write-Host ""
Write-Host "Monitor in another terminal:" -ForegroundColor Yellow
Write-Host (
    "  pwsh -NoProfile -File `"$caseRoot\watch-pelican.ps1`" " +
    "-RunName `"$RunName`""
)
