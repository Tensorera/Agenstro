$ErrorActionPreference = "Stop"

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$examples = @(
    foreach ($documentPath in Get-ChildItem -LiteralPath (Join-Path $repositoryRoot "docs") -Recurse -File -Filter "*.md" | Sort-Object FullName) {
        $document = [System.IO.File]::ReadAllText($documentPath.FullName, [System.Text.Encoding]::UTF8)
        $matches = [regex]::Matches(
            $document,
            '(?ms)^```haskell compile[^\S\r\n]*\r?\n(?<source>.*?)^```[^\S\r\n]*\r?$'
        )
        for ($index = 0; $index -lt $matches.Count; $index++) {
            [pscustomobject]@{
                Path = [System.IO.Path]::GetRelativePath($repositoryRoot, $documentPath.FullName).Replace('\', '/')
                Number = $index + 1
                Source = $matches[$index].Groups['source'].Value
            }
        }
    }
)

if ($examples.Count -eq 0) {
    throw "No compile-checked Haskell examples found under docs/"
}

$temporaryRoot = Join-Path (
    [System.IO.Path]::GetTempPath()
) ("agenstro-clef-docs-" + [System.Guid]::NewGuid().ToString("N"))
[System.IO.Directory]::CreateDirectory($temporaryRoot) | Out-Null

try {
    for ($index = 0; $index -lt $examples.Count; $index++) {
        $sourcePath = Join-Path $temporaryRoot ("Example{0}.hs" -f ($index + 1))
        $example = $examples[$index]
        # Complete modules are checked verbatim; short declaration examples
        # retain the historical Clef wrapper used by docs/clef.md.
        $module = if ($example.Source -match '(?m)^module\s+Main\b') {
            $example.Source
        }
        else {
            @"
{-# LANGUAGE OverloadedStrings #-}
module Main where

import Clef

$($example.Source)

main :: IO ()
main = pure ()
"@
        }
        [System.IO.File]::WriteAllText($sourcePath, $module, [System.Text.UTF8Encoding]::new($false))

        Write-Output "Checking $($example.Path) example $($example.Number)"
        & cabal exec --builddir=Build/cabal -- ghc -fno-code -Wall -Werror -package clef-sdk -package segno-flow $sourcePath
        if ($LASTEXITCODE -ne 0) {
            throw "$($example.Path) example $($example.Number) did not compile"
        }
    }
}
finally {
    if ([System.IO.Directory]::Exists($temporaryRoot)) {
        [System.IO.Directory]::Delete($temporaryRoot, $true)
    }
}

Write-Output "Compiled $($examples.Count) Haskell documentation example(s)."
