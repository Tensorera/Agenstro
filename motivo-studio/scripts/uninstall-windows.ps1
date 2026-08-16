[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Uninstall-MotivoStudio {
if ($env:OS -ne "Windows_NT") {
    throw "Motivo Studio's repository uninstaller currently supports Windows only."
}

$localAppData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
if ([string]::IsNullOrWhiteSpace($localAppData)) {
    throw "Windows did not provide a LocalApplicationData directory."
}
$programsRoot = [IO.Path]::GetFullPath((Join-Path $localAppData "Programs"))
$installRoot = [IO.Path]::GetFullPath((Join-Path $programsRoot "MotivoStudio"))
$expectedInstallRoot = [IO.Path]::GetFullPath("$programsRoot\MotivoStudio")
if (-not $installRoot.Equals($expectedInstallRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to remove anything outside the fixed per-user MotivoStudio directory."
}

$running = @(Get-Process -Name "motivo-studio" -ErrorAction SilentlyContinue)
if ($running.Count -gt 0) {
    throw "Close Motivo Studio before uninstalling it."
}

if ([IO.Directory]::Exists($installRoot)) {
    $marker = Join-Path $installRoot ".agenstro-motivo-install"
    $executable = Join-Path $installRoot "motivo-studio.exe"
    if (-not [IO.File]::Exists($marker) -or -not [IO.File]::Exists($executable)) {
        throw "The fixed install directory is not a recognized Agenstro Motivo Studio installation."
    }
    [IO.Directory]::Delete($installRoot, $true)
}

Remove-MotivoUserPath -InstallRoot $installRoot
Publish-EnvironmentChange
Write-Host "Removed Motivo Studio from $installRoot"
}

function Remove-MotivoUserPath {
    param([Parameter(Mandatory = $true)][string]$InstallRoot)

    $current = [Environment]::GetEnvironmentVariable("Path", "User")
    $entries = if ([string]::IsNullOrWhiteSpace($current)) { @() } else { @($current.Split(";")) }
    $kept = @(
        $entries | Where-Object {
            if ([string]::IsNullOrWhiteSpace($_)) { return $false }
            -not (Test-SamePath -Left $_ -Right $InstallRoot)
        }
    )
    [Environment]::SetEnvironmentVariable("Path", ($kept -join ";"), "User")
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

Uninstall-MotivoStudio
