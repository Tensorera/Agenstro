[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Install-MotivoStudio {
if ($env:OS -ne "Windows_NT") {
    throw "Motivo Studio's repository installer currently supports Windows only."
}

$studioRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$sourceRoot = [IO.Path]::GetFullPath((Join-Path $studioRoot "out\Motivo Studio-win32-x64"))
$sourceExe = Join-Path $sourceRoot "motivo-studio.exe"
$sourceResources = Join-Path $sourceRoot "resources"
if (-not [IO.File]::Exists($sourceExe) -or -not [IO.Directory]::Exists($sourceResources)) {
    throw "The packaged Motivo Studio application is incomplete. Run npm run package first."
}

$localAppData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
if ([string]::IsNullOrWhiteSpace($localAppData)) {
    throw "Windows did not provide a LocalApplicationData directory."
}
$programsRoot = [IO.Path]::GetFullPath((Join-Path $localAppData "Programs"))
$installRoot = [IO.Path]::GetFullPath((Join-Path $programsRoot "MotivoStudio"))
$expectedInstallRoot = [IO.Path]::GetFullPath("$programsRoot\MotivoStudio")
if (-not $installRoot.Equals($expectedInstallRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to install outside the fixed per-user MotivoStudio directory."
}
if ([IO.Directory]::Exists($installRoot)) {
    $existingMarker = Join-Path $installRoot ".agenstro-motivo-install"
    $existingExecutable = Join-Path $installRoot "motivo-studio.exe"
    if (-not [IO.File]::Exists($existingMarker) -or -not [IO.File]::Exists($existingExecutable)) {
        throw "The fixed install directory is not a recognized Agenstro Motivo Studio installation."
    }
}

$running = @(Get-Process -Name "motivo-studio" -ErrorAction SilentlyContinue)
if ($running.Count -gt 0) {
    throw "Close Motivo Studio before installing or upgrading it."
}

[IO.Directory]::CreateDirectory($programsRoot) | Out-Null
$nonce = [Guid]::NewGuid().ToString("N")
$stagingRoot = Join-Path $programsRoot ".MotivoStudio.install-$nonce"
$backupRoot = Join-Path $programsRoot ".MotivoStudio.backup-$nonce"

try {
    Copy-Item -LiteralPath $sourceRoot -Destination $stagingRoot -Recurse
    $stagedExe = Join-Path $stagingRoot "motivo-studio.exe"
    $stagedResources = Join-Path $stagingRoot "resources"
    if (-not [IO.File]::Exists($stagedExe) -or -not [IO.Directory]::Exists($stagedResources)) {
        throw "The staged Motivo Studio application failed validation."
    }
    Set-Content -LiteralPath (Join-Path $stagingRoot ".agenstro-motivo-install") -Value "motivo-studio/0.3" -Encoding Ascii

    $hadPreviousInstall = [IO.Directory]::Exists($installRoot)
    if ($hadPreviousInstall) {
        Move-Item -LiteralPath $installRoot -Destination $backupRoot
    }
    try {
        Move-Item -LiteralPath $stagingRoot -Destination $installRoot
    }
    catch {
        if ($hadPreviousInstall -and -not [IO.Directory]::Exists($installRoot) -and [IO.Directory]::Exists($backupRoot)) {
            Move-Item -LiteralPath $backupRoot -Destination $installRoot
        }
        throw
    }

    if ([IO.Directory]::Exists($backupRoot)) {
        [IO.Directory]::Delete($backupRoot, $true)
    }
}
finally {
    if ([IO.Directory]::Exists($stagingRoot)) {
        [IO.Directory]::Delete($stagingRoot, $true)
    }
}

Set-MotivoUserPath -InstallRoot $installRoot
Publish-EnvironmentChange

Write-Host "Installed Motivo Studio to $installRoot"
Write-Host "Open a new terminal, then run: motivo-studio [WORKSPACE]"
}

function Set-MotivoUserPath {
    param([Parameter(Mandatory = $true)][string]$InstallRoot)

    $current = [Environment]::GetEnvironmentVariable("Path", "User")
    $entries = if ([string]::IsNullOrWhiteSpace($current)) { @() } else { @($current.Split(";")) }
    $kept = @(
        $entries | Where-Object {
            if ([string]::IsNullOrWhiteSpace($_)) { return $false }
            -not (Test-SamePath -Left $_ -Right $InstallRoot)
        }
    )
    $next = (@($kept) + $InstallRoot) -join ";"
    [Environment]::SetEnvironmentVariable("Path", $next, "User")
}

function Test-SamePath {
    param(
        [Parameter(Mandatory = $true)][string]$Left,
        [Parameter(Mandatory = $true)][string]$Right
    )

    try {
        $leftFull = [IO.Path]::GetFullPath($Left.Trim().Trim('"')).TrimEnd("\")
        $rightFull = [IO.Path]::GetFullPath($Right.Trim().Trim('"')).TrimEnd("\")
        return $leftFull.Equals($rightFull, [StringComparison]::OrdinalIgnoreCase)
    }
    catch {
        return $false
    }
}

function Publish-EnvironmentChange {
    if (-not ("MotivoStudio.NativeMethods" -as [type])) {
        Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
namespace MotivoStudio {
    public static class NativeMethods {
        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        public static extern IntPtr SendMessageTimeout(
            IntPtr hWnd, uint message, UIntPtr wParam, string lParam,
            uint flags, uint timeout, out UIntPtr result);
    }
}
"@
    }
    $result = [UIntPtr]::Zero
    [void][MotivoStudio.NativeMethods]::SendMessageTimeout(
        [IntPtr]0xffff, 0x001a, [UIntPtr]::Zero, "Environment", 0x0002, 5000, [ref]$result
    )
}

Install-MotivoStudio
