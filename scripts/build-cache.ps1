[CmdletBinding()]
param(
    [string] $CacheRepositoryRoot = (Join-Path $PSScriptRoot ".."),
    [ValidateRange(1, 2147483648)]
    [long] $CacheBudgetBytes = 2147483648,
    [switch] $Force,
    [switch] $MeasureOnly
)

# Only these rebuildable directories may be removed. Never clean Build itself:
# it also holds reports, logs, packaged artifacts and local runtime data.
function Get-BuildCacheDirectories {
    param([Parameter(Mandatory)] [string] $Root)

    @("Build/cargo", "Build/cabal", "dist-newstyle", "Build/site",
        "motivo-studio/.vite", "motivo-studio/node_modules/.vite")
    $qualityRoot = Join-Path $Root 'Build/quality'
    Assert-CachePathHasNoLinks $qualityRoot
    if (Test-Path -LiteralPath $qualityRoot -PathType Container) {
        foreach ($run in Get-ChildItem -LiteralPath $qualityRoot -Directory -Force) {
            if ($run.Name -match '^\d{8}T\d{9}Z-\d+$') {
                "Build/quality/$($run.Name)/site"
            }
        }
    }
}

function Assert-CachePathHasNoLinks {
    param([Parameter(Mandatory)] [string] $Path)

    $cursor = [System.IO.Path]::GetFullPath($Path)
    while (-not [string]::IsNullOrEmpty($cursor)) {
        $item = Get-Item -LiteralPath $cursor -Force -ErrorAction SilentlyContinue
        if ($null -ne $item -and
            ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Refusing a cache/artifact path containing a link or reparse point: $cursor"
        }
        $cursor = [System.IO.Path]::GetDirectoryName($cursor)
    }
}

function Get-CacheTree {
    param(
        [Parameter(Mandatory)] [string] $Path,
        [switch] $AllowLinks
    )

    $item = Get-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
    if ($null -eq $item) { return }
    if (($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
        if ($AllowLinks) { return }
        throw "Refusing a cache/artifact tree containing a link or reparse point: $Path"
    }
    if (-not $item.PSIsContainer) { throw "Expected a cache directory: $Path" }

    $pending = [System.Collections.Generic.Stack[string]]::new()
    $pending.Push($Path)
    while ($pending.Count -gt 0) {
        foreach ($child in Get-ChildItem -LiteralPath $pending.Pop() -Force -ErrorAction Stop) {
            if (($child.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                if ($AllowLinks) { continue }
                throw "Refusing a cache/artifact tree containing a link or reparse point: $($child.FullName)"
            }
            if ($child.PSIsContainer) {
                $pending.Push($child.FullName)
            }
            else {
                $child
            }
        }
    }
}

function Get-BuildCacheInventory {
    param([Parameter(Mandatory)] [string] $Root)

    $Root = [System.IO.Path]::GetFullPath($Root)
    Assert-CachePathHasNoLinks $Root
    $entries = [System.Collections.Generic.List[object]]::new()
    $cleanable = @(Get-BuildCacheDirectories $Root)
    foreach ($relativePath in $cleanable) {
        $path = Join-Path $Root $relativePath
        Assert-CachePathHasNoLinks $path
        $files = @(Get-CacheTree $path)
        $bytes = [long]0
        foreach ($file in $files) { $bytes += $file.Length }
        $entries.Add([pscustomobject]@{
            path = $relativePath; bytes = $bytes; removable = $true; files = $files
        })
    }

    # Dependencies share the budget but must remain usable after a build. Do not
    # follow node_modules/.bin links or virtual-environment interpreter links.
    $pending = [System.Collections.Generic.Stack[string]]::new()
    $pending.Push($Root)
    while ($pending.Count -gt 0) {
        foreach ($child in Get-ChildItem -LiteralPath $pending.Pop() -Directory -Force -ErrorAction Stop) {
            if (($child.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) { continue }
            $relativePath = [System.IO.Path]::GetRelativePath($Root, $child.FullName).Replace('\', '/')
            if ($relativePath -eq '.git' -or $relativePath -in $cleanable -or
                $relativePath -eq 'Build/artifacts') { continue }
            if ($child.Name -in @('node_modules', '.venv', 'venv') -or
                $relativePath -eq 'Build/feature-checks-venv') {
                $bytes = [long]0
                foreach ($file in Get-CacheTree $child.FullName -AllowLinks) { $bytes += $file.Length }
                # A removable cache nested in retained dependencies is counted
                # separately and must not consume the budget twice.
                foreach ($nested in $entries | Where-Object { $_.removable -and $_.path.StartsWith("$relativePath/") }) {
                    $bytes -= $nested.bytes
                }
                $entries.Add([pscustomobject]@{
                    path = $relativePath; bytes = $bytes; removable = $false; files = @()
                })
            }
            else { $pending.Push($child.FullName) }
        }
    }

    $total = [long]0
    foreach ($entry in $entries) { $total += $entry.bytes }
    return [pscustomobject]@{ bytes = $total; entries = @($entries.ToArray()) }
}

function Save-BuildArtifacts {
    param(
        [Parameter(Mandatory)] [string] $Root,
        [Parameter(Mandatory)] [object] $Inventory
    )

    $artifactRoot = Join-Path $Root 'Build/artifacts'
    Assert-CachePathHasNoLinks $artifactRoot
    # Validate all destinations before touching any of them.
    $null = @(Get-CacheTree $artifactRoot)
    $copies = [System.Collections.Generic.List[object]]::new()
    $executableSuffix = if ($IsWindows) { '.exe' } else { '' }
    foreach ($profile in @('debug', 'release')) {
        foreach ($name in @("tactus$executableSuffix", "motivo-test$executableSuffix")) {
            $source = Join-Path $Root "Build/cargo/$profile/$name"
            if (Test-Path -LiteralPath $source -PathType Leaf) {
                $copies.Add(@{ source = $source; relative = "cargo/$profile/$name" })
            }
        }
    }
    $cabalFiles = @($Inventory.entries | Where-Object { $_.path -in @('Build/cabal', 'dist-newstyle') } |
        ForEach-Object { $_.files })
    foreach ($name in @("segno$executableSuffix")) {
        $binary = $cabalFiles | Where-Object {
            $_.Name -eq $name -and $_.FullName.Replace('\', '/') -match '/x/segno/build/segno/segno(\.exe)?$'
        } | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
        if ($null -ne $binary) {
            $copies.Add(@{ source = $binary.FullName; relative = "segno/$name" })
        }
    }
    foreach ($package in @('clef-sdk', 'segno-flow')) {
        $archive = $cabalFiles | Where-Object {
            $_.Name -match "^$package-[0-9].*\.tar\.gz$" -and $_.Directory.Name -eq 'sdist'
        } | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
        if ($null -ne $archive) {
            $copies.Add(@{ source = $archive.FullName; relative = "cabal/$package.tar.gz" })
        }
    }
    foreach ($copy in $copies) {
        Assert-CachePathHasNoLinks $copy.source
        $destination = Join-Path $artifactRoot $copy.relative
        Assert-CachePathHasNoLinks $destination
        New-Item -ItemType Directory -Force -Path (Split-Path $destination -Parent) | Out-Null
        $temporary = "$destination.$([guid]::NewGuid().ToString('N')).tmp"
        try {
            Copy-Item -LiteralPath $copy.source -Destination $temporary -ErrorAction Stop
            [System.IO.File]::Move($temporary, $destination, $true)
        }
        finally {
            if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
        }
    }
    return @($copies | ForEach-Object { "Build/artifacts/$($_.relative)" })
}

function Remove-CacheTreeSnapshot {
    param([Parameter(Mandatory)] [string] $Path)

    Assert-CachePathHasNoLinks $Path
    $files = [System.Collections.Generic.List[string]]::new()
    $directories = [System.Collections.Generic.List[string]]::new()
    $pending = [System.Collections.Generic.Stack[string]]::new()
    $pending.Push($Path)
    # Finish enumeration before removing anything. Recursive Remove-Item can
    # miss entries while mutating a large directory on Linux ntfs3 volumes.
    while ($pending.Count -gt 0) {
        $directory = $pending.Pop()
        $directories.Add($directory)
        foreach ($child in Get-ChildItem -LiteralPath $directory -Force -ErrorAction Stop) {
            if (($child.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Refusing a cache/artifact tree containing a link or reparse point: $($child.FullName)"
            }
            if ($child.PSIsContainer) { $pending.Push($child.FullName) }
            else { $files.Add($child.FullName) }
        }
    }
    foreach ($file in $files) {
        Remove-Item -LiteralPath $file -Force -ErrorAction Stop
    }
    for ($index = $directories.Count - 1; $index -ge 0; $index--) {
        # Nonrecursive deletion refuses any unexpected new directory entries.
        [System.IO.Directory]::Delete($directories[$index], $false)
    }
}

function Invoke-BuildCacheBudget {
    param(
        [Parameter(Mandatory)] [string] $Root,
        [ValidateRange(1, 2147483648)] [long] $BudgetBytes = 2147483648,
        [switch] $ForceClean,
        [switch] $InspectOnly
    )

    $Root = [System.IO.Path]::GetFullPath($Root)
    $before = Get-BuildCacheInventory $Root
    $removed = [System.Collections.Generic.List[string]]::new()
    $artifacts = @()
    if (-not $InspectOnly) {
        $artifacts = @(Save-BuildArtifacts $Root $before)
        if ($ForceClean -or $before.bytes -ge $BudgetBytes) {
            # Preflight every deletion before deleting the first directory.
            foreach ($relativePath in Get-BuildCacheDirectories $Root) {
                $path = Join-Path $Root $relativePath
                Assert-CachePathHasNoLinks $path
                $null = @(Get-CacheTree $path)
            }
            foreach ($relativePath in Get-BuildCacheDirectories $Root) {
                $path = Join-Path $Root $relativePath
                if (Test-Path -LiteralPath $path -PathType Container) {
                    Remove-CacheTreeSnapshot $path
                    $removed.Add($relativePath)
                }
            }
        }
    }
    $after = if ($removed.Count -gt 0) { Get-BuildCacheInventory $Root } else { $before }
    $report = [pscustomobject]@{
        budget_bytes = $BudgetBytes
        before_bytes = $before.bytes
        after_bytes = $after.bytes
        within_budget = $after.bytes -lt $BudgetBytes
        removed = @($removed.ToArray())
        artifacts = $artifacts
        directories = @($after.entries | Select-Object path, bytes, removable)
    }
    if (-not $InspectOnly -and -not $report.within_budget) {
        throw "Retained project dependencies use $($after.bytes) bytes, exceeding the cache budget (< $BudgetBytes). No tool dependencies were deleted."
    }
    return $report
}

if ($MyInvocation.InvocationName -ne '.') {
    $ErrorActionPreference = 'Stop'
    Invoke-BuildCacheBudget -Root $CacheRepositoryRoot -BudgetBytes $CacheBudgetBytes `
        -ForceClean:$Force -InspectOnly:$MeasureOnly | ConvertTo-Json -Depth 5
}
