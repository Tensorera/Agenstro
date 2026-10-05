[CmdletBinding()]
param([string] $ScratchParent = [System.IO.Path]::GetTempPath())

$ErrorActionPreference = 'Stop'
$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
. (Join-Path $repositoryRoot 'scripts/build-cache.ps1')
$scratch = Join-Path ([System.IO.Path]::GetFullPath($ScratchParent)) "agenstro-cache-tests-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Force -Path $scratch | Out-Null

function Assert-CacheTest {
    param([bool] $Condition, [string] $Message)
    if (-not $Condition) { throw $Message }
}

function Write-Fixture {
    param([string] $Root, [string] $Relative, [string] $Content)
    $path = Join-Path $Root $Relative
    New-Item -ItemType Directory -Force -Path (Split-Path $path -Parent) | Out-Null
    [System.IO.File]::WriteAllText($path, $Content)
}

function Assert-Refused {
    param([scriptblock] $Action)
    $message = $null
    try { & $Action | Out-Null }
    catch { $message = $_.Exception.Message }
    Assert-CacheTest ($null -ne $message -and $message -match 'link or reparse point') "Expected link-safety refusal, got: $message"
}

try {
    $root = Join-Path $scratch 'budget'
    $executableSuffix = if ($IsWindows) { '.exe' } else { '' }
    $foreignSuffix = if ($IsWindows) { '' } else { '.exe' }
    Write-Fixture $root "Build/cargo/debug/tactus$executableSuffix" 'binary'
    Write-Fixture $root "Build/cargo/release/motivo-test$executableSuffix" 'release'
    Write-Fixture $root "Build/cabal/build/platform/ghc/segno-flow/x/segno/build/segno/segno$executableSuffix" 'segno'
    Write-Fixture $root "Build/cargo/debug/tactus$foreignSuffix" 'foreign'
    Write-Fixture $root "Build/cabal/build/platform/ghc/segno-flow/x/segno/build/segno/segno$foreignSuffix" 'foreign'
    Write-Fixture $root 'Build/cabal/sdist/clef-sdk-0.3.0.0.tar.gz' 'archive'
    Write-Fixture $root 'dist-newstyle/cache/plan.json' 'plan'
    Write-Fixture $root 'motivo-studio/node_modules/library/index.js' 'dependency'
    Write-Fixture $root 'motivo-studio/node_modules/.vite/cache' 'vite'
    Write-Fixture $root 'Build/feature-checks-venv/pyvenv.cfg' 'python-env'
    Write-Fixture $root 'nested/.venv/pyvenv.cfg' 'nested-env'
    Write-Fixture $root 'Build/quality/20260918T120000000Z-42/site/index.html' 'site'
    Write-Fixture $root 'Build/quality/20260918T120000000Z-42/summary.json' 'report'
    Write-Fixture $root 'Build/local-runtime/state.db' 'state'
    Write-Fixture $root 'src/main.rs' 'source'

    $initial = Invoke-BuildCacheBudget -Root $root -InspectOnly
    Assert-CacheTest ($initial.budget_bytes -eq 2147483648) 'The default retained-cache budget must be 2 GiB.'
    Assert-CacheTest ($initial.before_bytes -eq 81) "Count dependencies, tool environments and site/cache files once: $($initial.before_bytes)"
    Assert-CacheTest (-not (Test-Path (Join-Path $root 'Build/artifacts'))) 'InspectOnly must not copy or delete anything.'
    $cliInventory = & pwsh -NoProfile -File (Join-Path $repositoryRoot 'scripts/build-cache.ps1') `
        -CacheRepositoryRoot $root -MeasureOnly | ConvertFrom-Json
    Assert-CacheTest ($LASTEXITCODE -eq 0 -and $cliInventory.budget_bytes -eq 2147483648) 'The standalone entry point must default to 2 GiB.'
    $customInventory = & pwsh -NoProfile -File (Join-Path $repositoryRoot 'scripts/build-cache.ps1') `
        -CacheRepositoryRoot $root -CacheBudgetBytes 1073741824 -MeasureOnly | ConvertFrom-Json
    Assert-CacheTest ($LASTEXITCODE -eq 0 -and $customInventory.budget_bytes -eq 1073741824) 'An explicit 1 GiB budget must be accepted.'
    $small = Invoke-BuildCacheBudget -Root $root -BudgetBytes 1000
    Assert-CacheTest ($small.removed.Count -eq 0) 'Below-budget builds must keep reusable caches.'
    $report = Invoke-BuildCacheBudget -Root $root -BudgetBytes $initial.before_bytes
    Assert-CacheTest ($report.after_bytes -eq 30 -and $report.within_budget) 'The exact budget boundary must trigger cleanup, retaining all tool dependencies.'
    foreach ($path in @('Build/cargo', 'Build/cabal', 'dist-newstyle', 'motivo-studio/node_modules/.vite',
        'Build/quality/20260918T120000000Z-42/site')) {
        Assert-CacheTest (-not (Test-Path (Join-Path $root $path))) "Cache was not removed: $path"
    }
    foreach ($pair in @(
        @("Build/artifacts/cargo/debug/tactus$executableSuffix", 'binary'),
        @("Build/artifacts/cargo/release/motivo-test$executableSuffix", 'release'),
        @("Build/artifacts/segno/segno$executableSuffix", 'segno'),
        @('Build/artifacts/cabal/clef-sdk.tar.gz', 'archive'),
        @('Build/quality/20260918T120000000Z-42/summary.json', 'report'),
        @('Build/local-runtime/state.db', 'state'),
        @('src/main.rs', 'source')
    )) {
        Assert-CacheTest ([System.IO.File]::ReadAllText((Join-Path $root $pair[0])) -eq $pair[1]) "Preservation failed: $($pair[0])"
    }
    foreach ($path in @("Build/artifacts/cargo/debug/tactus$foreignSuffix", "Build/artifacts/segno/segno$foreignSuffix")) {
        Assert-CacheTest (-not (Test-Path (Join-Path $root $path))) "A stale executable from another platform was retained: $path"
    }
    $retainedFailure = $null
    try { Invoke-BuildCacheBudget -Root $root -BudgetBytes 10 | Out-Null }
    catch { $retainedFailure = $_.Exception.Message }
    Assert-CacheTest ($retainedFailure -match 'Retained project dependencies') 'An impossible budget must fail without deleting dependencies.'
    Assert-CacheTest (Test-Path (Join-Path $root 'motivo-studio/node_modules/library/index.js')) 'Dependencies must survive budget failure.'

    # Cargo's broad hidden fingerprint tree exercises deletion after a complete
    # snapshot. -ScratchParent allows this regression on the actual build volume.
    $wideRoot = Join-Path $scratch 'wide-tree'
    for ($index = 0; $index -lt 2048; $index++) {
        $fingerprint = 'Build/cargo/debug/.fingerprint/tactus-runtime-{0:x16}' -f $index
        Write-Fixture $wideRoot "$fingerprint/invoked.timestamp" 'timestamp'
        Write-Fixture $wideRoot "$fingerprint/lib-tactus_runtime.json" '{}'
    }
    Write-Fixture $wideRoot 'Build/quality/keep.log' 'report'
    $wideReport = Invoke-BuildCacheBudget -Root $wideRoot -ForceClean
    Assert-CacheTest ($wideReport.after_bytes -eq 0 -and -not (Test-Path (Join-Path $wideRoot 'Build/cargo'))) 'Wide fingerprint directories must be completely removed.'
    Assert-CacheTest (Test-Path (Join-Path $wideRoot 'Build/quality/keep.log')) 'Wide-tree cleanup removed an unrelated log.'

    $outside = Join-Path $scratch 'outside'
    Write-Fixture $outside 'keep' 'external'
    foreach ($scenario in @('ancestor', 'nested', 'artifact', 'dangling', 'quality-run')) {
        $linkRoot = Join-Path $scratch $scenario
        Write-Fixture $linkRoot 'Build/cargo/cache' 'cache'
        Write-Fixture $linkRoot 'Build/cabal/cache' 'cache'
        $link = switch ($scenario) {
            'ancestor' { Remove-Item (Join-Path $linkRoot 'Build') -Recurse -Force; Join-Path $linkRoot 'Build' }
            'nested' { Join-Path $linkRoot 'Build/cabal/outside' }
            'artifact' { Join-Path $linkRoot 'Build/artifacts' }
            'dangling' { Join-Path $linkRoot 'Build/cargo/outside' }
            'quality-run' {
                New-Item -ItemType Directory -Path (Join-Path $linkRoot 'Build/quality') | Out-Null
                Join-Path $linkRoot 'Build/quality/20260918T120000000Z-42'
            }
        }
        $target = if ($scenario -eq 'dangling') { Join-Path $outside 'missing' } else { $outside }
        if ($IsWindows) {
            # Directory junctions exercise ReparsePoint protection without
            # requiring administrator rights or Windows Developer Mode.
            if ($scenario -eq 'dangling') {
                New-Item -ItemType Directory -Path $target | Out-Null
            }
            New-Item -ItemType Junction -Path $link -Target $target | Out-Null
            if ($scenario -eq 'dangling') { Remove-Item -LiteralPath $target -Force }
        }
        else {
            New-Item -ItemType SymbolicLink -Path $link -Target $target | Out-Null
        }
        try { Assert-Refused { Invoke-BuildCacheBudget -Root $linkRoot -ForceClean } }
        catch { throw "Link scenario '$scenario' failed: $($_.Exception.Message)" }
        Assert-CacheTest ([System.IO.File]::ReadAllText((Join-Path $outside 'keep')) -eq 'external') 'A link target was modified.'
        if ($scenario -notin @('ancestor', 'artifact')) {
            Assert-CacheTest (Test-Path (Join-Path $linkRoot 'Build/cargo/cache')) 'Safety preflight must prevent partial cache deletion.'
        }
        Remove-Item -LiteralPath $link -Force
    }

    # Exercise the actual quality entry point: dirty-worktree + FailFast must
    # still clean in finally, preserve its report, and honor a smaller budget.
    $qualityRoot = Join-Path $scratch 'quality-failure'
    Write-Fixture $qualityRoot 'README.md' 'fixture'
    New-Item -ItemType Directory -Path (Join-Path $qualityRoot 'scripts') | Out-Null
    Copy-Item (Join-Path $repositoryRoot 'scripts/quality.ps1') (Join-Path $qualityRoot 'scripts/quality.ps1')
    Copy-Item (Join-Path $repositoryRoot 'scripts/build-cache.ps1') (Join-Path $qualityRoot 'scripts/build-cache.ps1')
    & git -C $qualityRoot init --quiet
    & git -C $qualityRoot add README.md
    & git -C $qualityRoot -c user.name=CacheTests -c user.email=cache-tests@example.invalid commit --quiet -m fixture
    Write-Fixture $qualityRoot "Build/cargo/debug/tactus$executableSuffix" ('x' * 1500)
    $defaultOutput = @(& pwsh -NoProfile -File (Join-Path $qualityRoot 'scripts/quality.ps1') `
        -Profile Fast -RequireClean -FailFast 2>&1)
    Assert-CacheTest ($LASTEXITCODE -ne 0) 'The deliberately dirty quality fixture must fail.'
    $defaultSummaryFile = Get-ChildItem (Join-Path $qualityRoot 'Build/quality') -Recurse -Filter summary.json | Select-Object -First 1
    Assert-CacheTest ($null -ne $defaultSummaryFile) "The default-budget failure path lost its summary: $($defaultOutput -join "`n")"
    $defaultSummary = Get-Content $defaultSummaryFile.FullName -Raw | ConvertFrom-Json
    Assert-CacheTest ($defaultSummary.build_cache.budget_bytes -eq 2147483648 -and $defaultSummary.build_cache.after_bytes -eq 1500) 'Quality must apply the 2 GiB default and keep reusable caches below it.'
    Assert-CacheTest (Test-Path (Join-Path $qualityRoot "Build/cargo/debug/tactus$executableSuffix")) 'A below-budget cache must survive the default quality policy.'
    $output = @(& pwsh -NoProfile -File (Join-Path $qualityRoot 'scripts/quality.ps1') `
        -Profile Fast -RequireClean -FailFast -CacheBudgetBytes 1000 2>&1)
    Assert-CacheTest ($LASTEXITCODE -ne 0) 'The deliberately dirty quality fixture must fail.'
    $summaryFile = Get-ChildItem (Join-Path $qualityRoot 'Build/quality') -Recurse -Filter summary.json |
        Sort-Object FullName -Descending | Select-Object -First 1
    Assert-CacheTest ($null -ne $summaryFile) "The failure path lost its summary: $($output -join "`n")"
    $summary = Get-Content $summaryFile.FullName -Raw | ConvertFrom-Json
    Assert-CacheTest (-not $summary.passed -and $summary.build_cache.budget_bytes -eq 1000 -and $summary.build_cache.after_bytes -eq 0) 'FailFast must enforce the caller budget and report its result.'
    Assert-CacheTest (-not (Test-Path (Join-Path $qualityRoot 'Build/cargo'))) 'Failure path left an oversized cache.'
    Assert-CacheTest (Test-Path (Join-Path $qualityRoot "Build/artifacts/cargo/debug/tactus$executableSuffix")) 'Failure path lost the last built executable.'
    Write-Output 'Build-cache safety checks passed: accounting, threshold, preservation, wide trees, links, retained dependencies, and quality FailFast cleanup.'
}
finally {
    Remove-Item -LiteralPath $scratch -Recurse -Force
}
