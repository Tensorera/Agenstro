param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$')]
    [string]$RunName
)

$ErrorActionPreference = "Stop"

$caseRoot = $PSScriptRoot
$repositoryRoot = (Resolve-Path (Join-Path $caseRoot "..")).Path
$python = (Resolve-Path (
    Join-Path $repositoryRoot "clef-sdk\.venv\Scripts\python.exe"
)).Path
$runner = Join-Path $caseRoot "run.py"
$profile = Join-Path $caseRoot "pelican_profile.toml"
$runsRoot = Join-Path $caseRoot "runs"
$workfolder = Join-Path $runsRoot "output-$RunName"
$stdoutLog = Join-Path $runsRoot "live-$RunName.stdout.log"
$summaryLog = Join-Path $runsRoot "live-$RunName.summary.log"
$completionLog = Join-Path $runsRoot "live-$RunName.completion.json"

New-Item -ItemType Directory -Path $runsRoot -Force | Out-Null
foreach ($target in @($workfolder, $stdoutLog, $summaryLog, $completionLog)) {
    if (Test-Path -LiteralPath $target) {
        throw "Detached run target already exists: $target"
    }
}

$env:PYTHONUNBUFFERED = "1"
$env:DOTNET_CLI_TELEMETRY_OPTOUT = "1"
$env:DOTNET_NOLOGO = "1"

$startedAt = [DateTime]::UtcNow.ToString("o")
$exitCode = 255
try {
    Set-Location -LiteralPath $repositoryRoot
    & $python $runner `
        --profile $profile `
        --workfolder $workfolder `
        1> $stdoutLog `
        2> $summaryLog
    $exitCode = $LASTEXITCODE
}
catch {
    $_ | Out-String | Add-Content -LiteralPath $summaryLog -Encoding utf8
    $exitCode = 254
}
finally {
    [ordered]@{
        schema_version = "1.0"
        run_name = $RunName
        started_at_utc = $startedAt
        completed_at_utc = [DateTime]::UtcNow.ToString("o")
        exit_code = $exitCode
        workfolder = $workfolder
        stdout_log = $stdoutLog
        summary_log = $summaryLog
    } | ConvertTo-Json -Depth 8 |
        Set-Content -LiteralPath $completionLog -Encoding utf8
}

exit $exitCode
