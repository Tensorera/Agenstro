[CmdletBinding()]
param(
    [ValidateSet("Fast", "Full", "Release", "Audit", "Bootstrap", "Clean")]
    [string] $Profile = "Fast",
    [switch] $Staged,
    [switch] $RequireClean,
    [switch] $Clean,
    [ValidateRange(1, 2147483648)]
    [long] $CacheBudgetBytes = 2147483648,
    [ValidateRange(0, [double]::MaxValue)]
    [double] $CleanIfOverGiB = 0,
    [switch] $FailFast
)

$ErrorActionPreference = "Stop"
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$cargoTarget = [System.IO.Path]::GetFullPath((Join-Path $repositoryRoot "Build\cargo"))
$qualityRoot = Join-Path $repositoryRoot "Build\quality"
$startedAt = (Get-Date).ToUniversalTime()
$runId = $startedAt.ToString("yyyyMMddTHHmmssfffZ") + "-$PID"
$runRoot = Join-Path $qualityRoot $runId
$summaryPath = Join-Path $runRoot "summary.json"
$results = [System.Collections.Generic.List[object]]::new()
$commands = [System.Collections.Generic.List[object]]::new()
$hasFailures = $false
$currentStepName = $null
$unhandledError = $null
$cacheReport = $null
. (Join-Path $PSScriptRoot "build-cache.ps1") -CacheBudgetBytes $CacheBudgetBytes
if ($CleanIfOverGiB -gt 0) {
    # Legacy callers may request a tighter limit, never relax the 2 GiB policy.
    $CacheBudgetBytes = [math]::Max(1, [math]::Min($CacheBudgetBytes, [math]::Floor($CleanIfOverGiB * 1GB)))
}
$pythonCommand = if (Get-Command "python" -ErrorAction SilentlyContinue) {
    "python"
}
elseif (Get-Command "python3" -ErrorAction SilentlyContinue) {
    "python3"
}
else {
    "python"
}

New-Item -ItemType Directory -Force -Path $runRoot | Out-Null

function Invoke-NativeCommand {
    param(
        [Parameter(Mandatory)] [string] $File,
        [string[]] $Arguments = @(),
        [string] $WorkingDirectory = $repositoryRoot
    )

    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $exitCode = $null
    $errorMessage = $null
    Push-Location $WorkingDirectory
    try {
        try {
            & $File @Arguments
            $exitCode = $LASTEXITCODE
            if ($null -ne $exitCode -and $exitCode -ne 0) {
                throw "$File exited with code $exitCode"
            }
        }
        catch {
            $errorMessage = $_.Exception.Message
            throw
        }
        finally {
            $watch.Stop()
            $relativeWorkingDirectory = [System.IO.Path]::GetRelativePath(
                $repositoryRoot,
                [System.IO.Path]::GetFullPath($WorkingDirectory)
            ).Replace('\', '/')
            $commands.Add([ordered]@{
                step = $currentStepName
                executable = $File
                arguments = @($Arguments)
                working_directory = $relativeWorkingDirectory
                exit_code = $exitCode
                duration_seconds = [math]::Round($watch.Elapsed.TotalSeconds, 3)
                error = $errorMessage
            })
        }
    }
    finally {
        Pop-Location
    }
}

function Invoke-QualityStep {
    param(
        [Parameter(Mandatory)] [string] $Name,
        [Parameter(Mandatory)] [scriptblock] $Action
    )

    $safeName = ($Name -replace '[^A-Za-z0-9._-]', '-').Trim('-')
    $logPath = Join-Path $runRoot "$safeName.log"
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $status = "passed"
    $message = $null

    Write-Host "[quality] $Name"
    $previousStepName = $currentStepName
    $script:currentStepName = $Name
    try {
        & $Action 2>&1 | Tee-Object -FilePath $logPath
    }
    catch {
        $status = "failed"
        $message = $_.Exception.Message
        $script:hasFailures = $true
        Write-Error "$Name failed: $message" -ErrorAction Continue
    }
    finally {
        $script:currentStepName = $previousStepName
        $watch.Stop()
        $results.Add([ordered]@{
            name = $Name
            status = $status
            duration_seconds = [math]::Round($watch.Elapsed.TotalSeconds, 3)
            message = $message
            log = [System.IO.Path]::GetRelativePath($repositoryRoot, $logPath).Replace('\', '/')
        })
    }

    if ($status -eq "failed" -and $FailFast) {
        throw "$Name failed"
    }
}

function Invoke-CacheMaintenance {
    Invoke-QualityStep "build-cache-budget" {
        $script:cacheReport = Invoke-BuildCacheBudget -Root $repositoryRoot `
            -BudgetBytes $CacheBudgetBytes -ForceClean:($Clean -or $Profile -eq "Clean")
        Write-Output "Project cache: $($cacheReport.before_bytes) -> $($cacheReport.after_bytes) bytes (limit < $CacheBudgetBytes)."
        if ($cacheReport.removed.Count -gt 0) {
            Write-Output "Removed rebuildable directories: $($cacheReport.removed -join ', ')"
        }
    }
}

function Invoke-RepositoryDiffCheck {
    if ($Staged) {
        Invoke-NativeCommand "git" @("diff", "--cached", "--check")
        Invoke-NativeCommand "git" @("diff", "--quiet", "--exit-code", "--")
        $untracked = @(Invoke-NativeCommand "git" @("ls-files", "--others", "--exclude-standard"))
        if ($untracked.Count -gt 0) {
            throw "Staged validation requires no untracked files; stage or remove them first."
        }
    }
    else {
        Invoke-NativeCommand "git" @("diff", "--check")
    }
}

function Invoke-FastSteps {
    Invoke-QualityStep "repository-diff-check" { Invoke-RepositoryDiffCheck }
    Invoke-QualityStep "build-cache-safety-tests" {
        Invoke-NativeCommand "pwsh" @(
            "-NoProfile", "-File", "Test/repository/test-build-cache.ps1"
        )
    }
    Invoke-QualityStep "rust-format" {
        Invoke-NativeCommand "cargo" @("fmt", "--all", "--check")
    }
    Invoke-QualityStep "rust-check" {
        Invoke-NativeCommand "cargo" @("check", "--workspace", "--all-targets", "--locked")
    }
    Invoke-QualityStep "haskell-build-werror" {
        Invoke-NativeCommand "cabal" @(
            "build", "--builddir=Build/cabal", "all", "--enable-tests", "--ghc-options=-Werror"
        )
    }
    Invoke-QualityStep "norm-fixtures" {
        Invoke-NativeCommand $pythonCommand @("plugins/latex-norm-check/run_fixtures.py")
    }
    Invoke-QualityStep "documentation-contract" {
        Invoke-NativeCommand "pwsh" @(
            "-NoProfile", "-File", "Test/repository/test-documentation-contract.ps1"
        )
    }
    Invoke-QualityStep "motivo-format" {
        Invoke-NativeCommand "npm" @("run", "format:check") (Join-Path $repositoryRoot "motivo-studio")
    }
    Invoke-QualityStep "motivo-lint" {
        Invoke-NativeCommand "npm" @("run", "lint") (Join-Path $repositoryRoot "motivo-studio")
    }
}

function Invoke-FullSteps {
    Invoke-FastSteps
    Invoke-QualityStep "rust-clippy" {
        Invoke-NativeCommand "cargo" @(
            "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"
        )
    }
    Invoke-QualityStep "rust-tests" {
        Invoke-NativeCommand "cargo" @("test", "--workspace", "--locked")
    }
    Invoke-QualityStep "rust-msrv-check" {
        Invoke-NativeCommand "cargo" @(
            "+1.88.0", "check", "--workspace", "--all-targets", "--locked"
        )
    }
    Invoke-QualityStep "rust-msrv-tests" {
        Invoke-NativeCommand "cargo" @("+1.88.0", "test", "--workspace", "--locked")
    }
    Invoke-QualityStep "topology-reference-tests" {
        Invoke-NativeCommand "cargo" @(
            "test", "--manifest-path", "examples/topology-holes/reference/Cargo.toml", "--locked"
        )
    }
    Invoke-QualityStep "cross-language-generic-plugin" {
        Invoke-NativeCommand "cargo" @(
            "test", "--locked", "-p", "tactus-runtime", "--test", "runtime",
            "haskell_generic_plugin_routes_through_absolute_tactus_dispatch", "--", "--ignored", "--exact"
        )
    }
    Invoke-QualityStep "cross-language-topology" {
        Invoke-NativeCommand "cargo" @(
            "test", "--locked", "-p", "tactus-runtime", "--test", "runtime",
            "haskell_topology_workflow_runs_all_stages_with_parallel_reviews", "--", "--ignored", "--exact"
        )
    }
    Invoke-QualityStep "haskell-tests-werror" {
        Invoke-NativeCommand "cabal" @(
            "test", "--builddir=Build/cabal", "all", "--test-show-details=direct", "--ghc-options=-Werror"
        )
    }
    Invoke-QualityStep "haskell-example-typechecks" {
        Invoke-NativeCommand "cabal" @(
            "exec", "--builddir=Build/cabal", "--", "ghc", "-fno-code",
            "-package", "clef-sdk", "-package", "segno-flow",
            "segno-flow/examples/active-window/900_record_active_window.hs"
        )
        foreach ($script in Get-ChildItem -LiteralPath (Join-Path $repositoryRoot "examples\topology-holes\workflow") -Filter "*.hs") {
            Invoke-NativeCommand "cabal" @(
                "exec", "--builddir=Build/cabal", "--", "ghc", "-fno-code",
                "-package", "clef-sdk", $script.FullName
            )
        }
    }
    Invoke-QualityStep "documentation-build" {
        Invoke-NativeCommand $pythonCommand @(
            "-m", "mkdocs", "build", "--strict", "--site-dir", (Join-Path $runRoot "site")
        )
    }
    Invoke-QualityStep "segno-service-acceptance" {
        $segnoBinary = & cabal list-bin --builddir=Build/cabal segno-flow:exe:segno
        if ($LASTEXITCODE -ne 0) {
            throw "Could not locate the built Segno executable."
        }
        $tactusName = if ($IsWindows) { "tactus.exe" } else { "tactus" }
        Invoke-NativeCommand $pythonCommand @(
            "Test/repository/test-segno-service.py",
            "--tactus", (Join-Path $cargoTarget "debug/$tactusName"),
            "--segno", ([string]$segnoBinary).Trim()
        )
    }
    Invoke-QualityStep "documentation-examples" {
        Invoke-NativeCommand "pwsh" @(
            "-NoProfile", "-File", "Test/repository/test-clef-documentation-examples.ps1"
        )
    }
    Invoke-QualityStep "motivo-tests" {
        Invoke-NativeCommand "npm" @("test") (Join-Path $repositoryRoot "motivo-studio")
    }
    Invoke-QualityStep "motivo-installed-methods" {
        # init embeds the templates, skill and observer: test a freshly built binary.
        Invoke-NativeCommand "cargo" @("build", "--workspace", "--locked")
        $tactusName = if ($IsWindows) { "tactus.exe" } else { "tactus" }
        Invoke-NativeCommand $pythonCommand @(
            "motivo/tests/run.py", "--tactus", (Join-Path $cargoTarget "debug/$tactusName"), "--installed-only"
        )
    }
    Invoke-QualityStep "motivo-package" {
        Invoke-NativeCommand "npm" @("run", "package") (Join-Path $repositoryRoot "motivo-studio")
    }
    Invoke-QualityStep "cabal-package-checks" {
        Invoke-NativeCommand "cabal" @("check") (Join-Path $repositoryRoot "clef-sdk")
        Invoke-NativeCommand "cabal" @("check") (Join-Path $repositoryRoot "segno-flow")
    }
}

function Invoke-VersionAlignmentCheck {
    $cargoText = Get-Content -LiteralPath (Join-Path $repositoryRoot "Cargo.toml") -Raw
    $npmManifest = Get-Content -LiteralPath (Join-Path $repositoryRoot "motivo-studio\package.json") -Raw | ConvertFrom-Json
    $clefText = Get-Content -LiteralPath (Join-Path $repositoryRoot "clef-sdk\clef-sdk.cabal") -Raw
    $segnoText = Get-Content -LiteralPath (Join-Path $repositoryRoot "segno-flow\segno-flow.cabal") -Raw
    $cargoVersion = [regex]::Match($cargoText, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
    $clefVersion = [regex]::Match($clefText, '(?m)^version:\s*([^\s]+)').Groups[1].Value
    $segnoVersion = [regex]::Match($segnoText, '(?m)^version:\s*([^\s]+)').Groups[1].Value
    $expectedHaskell = "$cargoVersion.0"
    if ($npmManifest.version -ne $cargoVersion -or
        $clefVersion -ne $expectedHaskell -or
        $segnoVersion -ne $expectedHaskell) {
        throw "Version mismatch: Cargo=$cargoVersion npm=$($npmManifest.version) Clef=$clefVersion Segno=$segnoVersion"
    }
    Write-Output "Version manifests agree on $cargoVersion / $expectedHaskell."
}

function Invoke-AuditSteps {
    Invoke-QualityStep "npm-audit" {
        Invoke-NativeCommand "npm" @("audit", "--audit-level=high") (Join-Path $repositoryRoot "motivo-studio")
    }
    Invoke-QualityStep "cargo-audit" {
        if (-not (Get-Command "cargo-audit" -ErrorAction SilentlyContinue)) {
            throw "cargo-audit is required; run the Bootstrap profile."
        }
        Invoke-NativeCommand "cargo" @("audit")
    }
    Invoke-QualityStep "cargo-deny" {
        if (-not (Get-Command "cargo-deny" -ErrorAction SilentlyContinue)) {
            throw "cargo-deny is required; run the Bootstrap profile."
        }
        Invoke-NativeCommand "cargo" @("deny", "--locked", "check", "bans", "licenses", "sources")
    }
    Invoke-QualityStep "cabal-outdated" {
        Invoke-NativeCommand "cabal" @("outdated", "--project-context", "--exit-code")
    }
}

function Invoke-BootstrapSteps {
    Invoke-QualityStep "install-rust-msrv" {
        Invoke-NativeCommand "rustup" @("toolchain", "install", "1.88.0", "--profile", "minimal")
    }
    Invoke-QualityStep "update-hackage-index" {
        Invoke-NativeCommand "cabal" @("update")
    }
    Invoke-QualityStep "install-node-dependencies" {
        Invoke-NativeCommand "npm" @("ci") (Join-Path $repositoryRoot "motivo-studio")
    }
    Invoke-QualityStep "install-documentation-tool" {
        Invoke-NativeCommand $pythonCommand @("-m", "pip", "install", "mkdocs>=1.6,<2")
    }
    Invoke-QualityStep "install-cargo-audit" {
        if (-not (Get-Command "cargo-audit" -ErrorAction SilentlyContinue)) {
            Invoke-NativeCommand "cargo" @(
                "install", "cargo-audit", "--version", "0.22.2", "--locked"
            )
        }
        else {
            Write-Output "cargo-audit is already installed."
        }
    }
    Invoke-QualityStep "install-cargo-deny" {
        if (-not (Get-Command "cargo-deny" -ErrorAction SilentlyContinue)) {
            Invoke-NativeCommand "cargo" @(
                "install", "cargo-deny", "--version", "0.20.2", "--locked"
            )
        }
        else {
            Write-Output "cargo-deny is already installed."
        }
    }
}

function Get-ToolVersion {
    param(
        [Parameter(Mandatory)] [string] $File,
        [string[]] $Arguments = @("--version")
    )

    if (-not (Get-Command $File -ErrorAction SilentlyContinue)) {
        return $null
    }
    try {
        $output = @(& $File @Arguments 2>&1)
        if ($LASTEXITCODE -ne 0 -or $output.Count -eq 0) {
            return $null
        }
        return $output[0].ToString().Trim()
    }
    catch {
        return $null
    }
}

function Get-ToolVersions {
    return [ordered]@{
        os = [System.Runtime.InteropServices.RuntimeInformation]::OSDescription
        architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
        powershell = $PSVersionTable.PSVersion.ToString()
        git = Get-ToolVersion "git"
        rustc = Get-ToolVersion "rustc"
        rustc_msrv = Get-ToolVersion "rustc" @("+1.88.0", "--version")
        cargo = Get-ToolVersion "cargo"
        cargo_audit = Get-ToolVersion "cargo-audit"
        cargo_deny = Get-ToolVersion "cargo-deny"
        ghc = Get-ToolVersion "ghc"
        cabal = Get-ToolVersion "cabal"
        node = Get-ToolVersion "node"
        npm = Get-ToolVersion "npm"
        python = Get-ToolVersion $pythonCommand
        mkdocs = Get-ToolVersion $pythonCommand @("-m", "mkdocs", "--version")
    }
}

$initialStatus = (& git -C $repositoryRoot status --porcelain=v1) -join "`n"
$initialHead = (& git -C $repositoryRoot rev-parse HEAD).Trim()
$originalEnvironment = @{}
foreach ($name in @("CARGO_TARGET_DIR", "CARGO_INCREMENTAL", "CARGO_PROFILE_DEV_DEBUG", "CARGO_PROFILE_TEST_DEBUG")) {
    $item = Get-Item "Env:$name" -ErrorAction SilentlyContinue
    $originalEnvironment[$name] = if ($null -eq $item) { $null } else { $item.Value }
}

try {
    if ($Clean -and -not $PSBoundParameters.ContainsKey("Profile")) {
        $Profile = "Clean"
    }
    $env:CARGO_TARGET_DIR = $cargoTarget
    $env:CARGO_INCREMENTAL = "0"
    $env:CARGO_PROFILE_DEV_DEBUG = "0"
    $env:CARGO_PROFILE_TEST_DEBUG = "0"

    if ($RequireClean -or $Profile -eq "Release") {
        Invoke-QualityStep "clean-worktree" {
            if (-not [string]::IsNullOrWhiteSpace($initialStatus)) {
                throw "$Profile validation requires a clean worktree."
            }
        }
        if ($hasFailures) {
            throw "$Profile validation stopped at the clean-worktree prerequisite."
        }
    }

    switch ($Profile) {
        "Fast" { Invoke-FastSteps }
        "Full" { Invoke-FullSteps }
        "Audit" { Invoke-AuditSteps }
        "Bootstrap" { Invoke-BootstrapSteps }
        "Clean" { } # The finally block performs forced cleanup.
        "Release" {
            Invoke-QualityStep "release-node-dependencies" {
                Invoke-NativeCommand "npm" @("ci") (Join-Path $repositoryRoot "motivo-studio")
            }
            if ($hasFailures) {
                throw "Release validation stopped because npm ci failed."
            }
            Invoke-FullSteps
            if ($hasFailures) {
                throw "Release validation stopped because the Full gate failed."
            }
            Invoke-QualityStep "version-alignment" { Invoke-VersionAlignmentCheck }
            if ($hasFailures) {
                throw "Release validation stopped because version alignment failed."
            }
            Invoke-AuditSteps
            if ($hasFailures) {
                throw "Release validation stopped because the dependency audit failed."
            }
            Invoke-QualityStep "rust-release-build" {
                Invoke-NativeCommand "cargo" @("build", "--release", "--locked", "--workspace")
            }
            Invoke-QualityStep "cabal-source-distributions" {
                $cabalBuildDirectory = "--builddir=$(Join-Path $repositoryRoot 'Build/cabal')"
                Invoke-NativeCommand "cabal" @("sdist", $cabalBuildDirectory) (Join-Path $repositoryRoot "clef-sdk")
                Invoke-NativeCommand "cabal" @("sdist", $cabalBuildDirectory) (Join-Path $repositoryRoot "segno-flow")
            }
            Invoke-QualityStep "motivo-offline-report" {
                Invoke-NativeCommand "npm" @("run", "build") (Join-Path $repositoryRoot "motivo-studio")
            }
        }
    }
}
catch {
    $script:hasFailures = $true
    $script:unhandledError = $_.Exception.Message
    throw
}
finally {
    # Run even after failed prerequisites, FailFast, or compiler errors. Keep
    # summary generation independent of FailFast and cleanup failures.
    try { Invoke-CacheMaintenance }
    catch {
        $script:hasFailures = $true
        if ($null -eq $unhandledError) { $script:unhandledError = $_.Exception.Message }
    }
    foreach ($name in $originalEnvironment.Keys) {
        if ($null -eq $originalEnvironment[$name]) {
            Remove-Item "Env:$name" -ErrorAction SilentlyContinue
        }
        else {
            Set-Item "Env:$name" $originalEnvironment[$name]
        }
    }

    $finalStatus = (& git -C $repositoryRoot status --porcelain=v1) -join "`n"
    $toolVersions = Get-ToolVersions
    $summary = [ordered]@{
        schema = "agenstro.quality/v1"
        profile = $Profile.ToLowerInvariant()
        started_at = $startedAt.ToString("o")
        commit = $initialHead
        dirty_before = -not [string]::IsNullOrWhiteSpace($initialStatus)
        dirty_after = -not [string]::IsNullOrWhiteSpace($finalStatus)
        cargo_target_gib = if ($null -eq $cacheReport) { $null } else {
            [math]::Round((($cacheReport.directories | Where-Object path -eq 'Build/cargo').bytes / 1GB), 3)
        }
        build_cache = $cacheReport
        passed = -not $hasFailures
        error = $unhandledError
        environment = $toolVersions
        steps = $results
        commands = $commands
    }
    $summary | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $summaryPath -Encoding utf8
    Write-Host "[quality] summary: $summaryPath"
}

if ($hasFailures) {
    exit 1
}
