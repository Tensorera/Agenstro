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
    "segno-flow/README.md",
    "docs/index.md",
    "docs/install.md",
    "docs/getting-started.md",
    "docs/clef.md",
    "docs/tactus-workspace.md",
    "docs/providers.md",
    "docs/plugin-authoring.md",
    "docs/observability.md",
    "docs/operations.md",
    "docs/architecture.md",
    "docs/segno.md",
    "docs/motivo-studio.md",
    "docs/troubleshooting.md",
    "docs/roadmap.md",
    "docs/reference/cli-v0.3.md",
    "docs/reference/plugin-protocol-v1.md",
    "docs/reference/segno-plugin-wire-v1.md",
    "docs/reference/studio-control-v1.md",
    "docs/reference/support-matrix.md",
    "docs/reference/glossary.md",
    "docs/adr/0003-haskell-dsl-and-local-plugins.md",
    "docs/adr/0004-haskell-segno-persistent-tasks.md",
    "docs/migrations/0.2-to-haskell-0.3.md",
    "docs/how-to/write-documentation.md"
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

$documents = @{}
foreach ($relativePath in $publishedFiles) {
    $path = Join-Path $repositoryRoot $relativePath
    Assert-Contract (Test-Path -LiteralPath $path -PathType Leaf) "Missing published file: $relativePath"
    $text = [System.IO.File]::ReadAllText($path, [System.Text.Encoding]::UTF8)
    Assert-Contract ((Get-MarkdownH1Count $text) -eq 1) "Expected exactly one H1 in $relativePath"
    $documents[$relativePath] = $text
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
