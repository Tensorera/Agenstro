$ErrorActionPreference = "Stop"

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$publishedFiles = @(
    "README.md",
    "CHANGELOG.md",
    "SECURITY.md",
    "CONTRIBUTING.md",
    "clef-sdk/README.md",
    "tactus-runtime/README.md",
    "motivo-studio/README.md",
    "segno-flow/README.md"
) + @(
    Get-ChildItem -LiteralPath (Join-Path $repositoryRoot "docs") -Recurse -File -Filter "*.md" |
        ForEach-Object {
            [System.IO.Path]::GetRelativePath($repositoryRoot, $_.FullName).Replace('\', '/')
        } |
        Sort-Object
)

function Assert-Contract {
    param(
        [Parameter(Mandatory)] [bool] $Condition,
        [Parameter(Mandatory)] [string] $Message
    )
    if (-not $Condition) {
        throw $Message
    }
}

function Get-MarkdownH1Count {
    param([Parameter(Mandatory)] [string] $Text)

    $insideFence = $false
    $count = 0
    foreach ($line in ($Text -split "\r?\n")) {
        $trimmed = $line.TrimStart()
        if ($trimmed.StartsWith('```') -or $trimmed.StartsWith('~~~')) {
            $insideFence = -not $insideFence
        }
        elseif (-not $insideFence -and $line -match '^# [^#]') {
            $count++
        }
    }
    return $count
}

function Get-FrontMatter {
    param(
        [Parameter(Mandatory)] [string] $RelativePath,
        [Parameter(Mandatory)] [string] $Text
    )

    $lines = $Text -split "\r?\n"
    Assert-Contract ($lines.Count -gt 2 -and $lines[0] -eq '---') "Missing front matter in $RelativePath"
    $end = -1
    for ($index = 1; $index -lt $lines.Count; $index++) {
        if ($lines[$index] -eq '---') {
            $end = $index
            break
        }
    }
    Assert-Contract ($end -gt 1) "Unterminated front matter in $RelativePath"

    $metadata = @{}
    foreach ($line in $lines[1..($end - 1)]) {
        if ($line -match '^([a-z_]+):\s*(.*)$') {
            Assert-Contract (-not $metadata.ContainsKey($Matches[1])) "Duplicate metadata key in ${RelativePath}: $($Matches[1])"
            $metadata[$Matches[1]] = $Matches[2].Trim()
        }
    }
    return $metadata
}

function Get-InlineList {
    param(
        [Parameter(Mandatory)] [string] $RelativePath,
        [Parameter(Mandatory)] [string] $Key,
        [Parameter(Mandatory)] [string] $Value
    )

    Assert-Contract ($Value -match '^\[([^]]+)\]$') "Expected an inline list for $Key in $RelativePath"
    return @($Matches[1] -split ',' | ForEach-Object { $_.Trim() } | Where-Object { $_ })
}

$documents = @{}
foreach ($relativePath in $publishedFiles) {
    $path = Join-Path $repositoryRoot $relativePath
    Assert-Contract (Test-Path -LiteralPath $path -PathType Leaf) "Missing published file: $relativePath"
    $text = [System.IO.File]::ReadAllText($path, [System.Text.Encoding]::UTF8)
    Assert-Contract ((Get-MarkdownH1Count $text) -eq 1) "Expected exactly one H1 in $relativePath"
    $documents[$relativePath] = $text
}

$metadataExemptFiles = @(
    "docs/index.md",
    "docs/how-to/write-documentation.md",
    "docs/adr/0003-haskell-dsl-and-local-plugins.md",
    "docs/adr/0004-haskell-segno-persistent-tasks.md",
    "docs/adr/0005-norms-rubrics-and-refinement.md",
    "docs/adr/0006-motivo-session-pattern.md"
)
$allowedStatuses = @("alpha", "experimental", "historical", "working decision record")
$allowedPlatforms = @("windows", "ubuntu", "all")
$requiredMetadata = @("title", "status", "owners", "last_verified", "applies_to", "platforms")
foreach ($relativePath in $publishedFiles | Where-Object { $_.StartsWith('docs/') -and $_ -notin $metadataExemptFiles }) {
    $metadata = Get-FrontMatter $relativePath $documents[$relativePath]
    foreach ($key in $requiredMetadata) {
        Assert-Contract ($metadata.ContainsKey($key) -and -not [string]::IsNullOrWhiteSpace($metadata[$key])) "Missing metadata key '$key' in $relativePath"
    }
    Assert-Contract ($metadata.status -in $allowedStatuses) "Unsupported status '$($metadata.status)' in $relativePath"

    $owners = Get-InlineList $relativePath "owners" $metadata.owners
    Assert-Contract ($owners.Count -gt 0) "Expected at least one owner in $relativePath"
    foreach ($owner in $owners) {
        Assert-Contract ($owner -match '^[a-z0-9-]+$') "Invalid owner '$owner' in $relativePath"
    }

    $platforms = Get-InlineList $relativePath "platforms" $metadata.platforms
    Assert-Contract ($platforms.Count -gt 0) "Expected at least one platform in $relativePath"
    foreach ($platform in $platforms) {
        Assert-Contract ($platform -in $allowedPlatforms) "Unsupported platform '$platform' in $relativePath"
    }

    $verifiedDate = [datetime]::MinValue
    $dateValid = [datetime]::TryParseExact(
        $metadata.last_verified,
        'yyyy-MM-dd',
        [System.Globalization.CultureInfo]::InvariantCulture,
        [System.Globalization.DateTimeStyles]::AssumeUniversal,
        [ref]$verifiedDate
    )
    Assert-Contract $dateValid "Invalid last_verified date in ${relativePath}: $($metadata.last_verified)"
    Assert-Contract ($verifiedDate.Date -le [datetime]::UtcNow.Date.AddDays(1)) "Future last_verified date in ${relativePath}: $($metadata.last_verified)"
    if (([datetime]::UtcNow.Date - $verifiedDate.Date).TotalDays -gt 90) {
        Write-Warning "Documentation verification is older than 90 days: $relativePath ($($metadata.last_verified))"
    }
}

$publishedText = ($documents.Values -join "`n")
foreach ($name in @("clef-sdk", "tactus-runtime", "motivo-studio", "segno-flow")) {
    Assert-Contract ($publishedText.Contains($name)) "Missing current component name: $name"
}
foreach ($contract in @("agenstro.plugin/v1", "OutcomeUnknown", "at least once", "TypeScript")) {
    Assert-Contract ($publishedText.Contains($contract)) "Missing current contract: $contract"
}
foreach ($removedClaim in @("segno-flow service start", "segno-flow-ui", "~/.segno-flow")) {
    Assert-Contract (-not $publishedText.Contains($removedClaim)) "Removed claim remains published: $removedClaim"
}

$linkPattern = [regex]'\[[^\]]+\]\(([^)]+)\)'
foreach ($relativePath in $publishedFiles) {
    $sourcePath = Join-Path $repositoryRoot $relativePath
    foreach ($match in $linkPattern.Matches($documents[$relativePath])) {
        $target = $match.Groups[1].Value
        if ($target.StartsWith('https://') -or
            $target.StartsWith('http://') -or
            $target.StartsWith('mailto:') -or
            $target.StartsWith('#')) {
            continue
        }
        $localTarget = ($target -split '#', 2)[0]
        if ([string]::IsNullOrEmpty($localTarget)) {
            continue
        }
        $resolved = [System.IO.Path]::GetFullPath(
            (Join-Path (Split-Path $sourcePath -Parent) $localTarget)
        )
        Assert-Contract (Test-Path -LiteralPath $resolved) "Missing link target in ${relativePath}: $target"
    }
}

Write-Output "Documentation contract checks passed for $($publishedFiles.Count) files."
