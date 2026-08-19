$ErrorActionPreference = "Stop"

$repositoryRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$documentPath = Join-Path $repositoryRoot "docs\clef.md"
$document = [System.IO.File]::ReadAllText($documentPath, [System.Text.Encoding]::UTF8)
$examples = [regex]::Matches(
    $document,
    '(?ms)^```haskell compile\s*\r?\n(?<source>.*?)^```\s*$'
)

if ($examples.Count -eq 0) {
    throw "No compile-checked Haskell examples found in docs/clef.md"
}

$temporaryRoot = Join-Path (
    [System.IO.Path]::GetTempPath()
) ("agenstro-clef-docs-" + [System.Guid]::NewGuid().ToString("N"))
[System.IO.Directory]::CreateDirectory($temporaryRoot) | Out-Null

try {
    for ($index = 0; $index -lt $examples.Count; $index++) {
        $sourcePath = Join-Path $temporaryRoot ("Example{0}.hs" -f ($index + 1))
        $module = @"
{-# LANGUAGE OverloadedStrings #-}
module Main where

import Clef

$($examples[$index].Groups['source'].Value)

main :: IO ()
main = pure ()
"@
        [System.IO.File]::WriteAllText($sourcePath, $module, [System.Text.UTF8Encoding]::new($false))

        & cabal exec --builddir=Build/cabal -- ghc -fno-code -Wall -Werror -package clef-sdk $sourcePath
        if ($LASTEXITCODE -ne 0) {
            throw "Clef documentation example $($index + 1) did not compile"
        }
    }
}
finally {
    if ([System.IO.Directory]::Exists($temporaryRoot)) {
        [System.IO.Directory]::Delete($temporaryRoot, $true)
    }
}

Write-Output "Compiled $($examples.Count) Clef documentation example(s)."
