param(
    [string]$RunName = "fair-contract-restart"
)

$ErrorActionPreference = "Stop"

$reproductionRoot = $PSScriptRoot
$repositoryRoot = (Resolve-Path (Join-Path $reproductionRoot "..\..")).Path
$python = (Resolve-Path (
    Join-Path $repositoryRoot "clef-sdk\.venv\Scripts\python.exe"
)).Path
$runner = Join-Path $reproductionRoot "run.py"
$profile = Join-Path $reproductionRoot "reproduction_profile.toml"
$workfolder = Join-Path $reproductionRoot "output-agent-$RunName"
$stdoutLog = Join-Path $reproductionRoot "live-$RunName.stdout.log"
$summaryLog = Join-Path $reproductionRoot "live-$RunName.summary.log"

foreach ($target in @($workfolder, $stdoutLog, $summaryLog)) {
    if (Test-Path -LiteralPath $target) {
        throw "Detached run target already exists: $target"
    }
}

Set-Location -LiteralPath $repositoryRoot
& $python $runner `
    --mode agent `
    --profile $profile `
    --workfolder $workfolder `
    1> $stdoutLog `
    2> $summaryLog

exit $LASTEXITCODE
