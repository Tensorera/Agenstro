$ErrorActionPreference = "Stop"

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
Push-Location $repositoryRoot
try {
    git config --local core.hooksPath .githooks
    if ($LASTEXITCODE -ne 0) {
        throw "git config failed with exit code $LASTEXITCODE"
    }
    Write-Output "Installed Agenstro hooks from .githooks."
}
finally {
    Pop-Location
}
