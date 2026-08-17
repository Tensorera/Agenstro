---
title: Install Agenstro 0.3
status: alpha
last_verified: 2026-08-17
applies_to: "Clef/Segno Haskell 0.3.0.0, Tactus Rust 0.3.0, Motivo Studio 0.3.0"
---

# Install Agenstro 0.3

This page is the canonical source-install and upgrade procedure. Install
Tactus for ordinary workflows; add Segno only for persistent tasks and Motivo
Studio only when a desktop projection is useful.

## Supported source paths

| Surface | Windows | Ubuntu | Notes |
| --- | --- | --- | --- |
| Tactus | Supported | Supported | Built with stable Rust |
| Clef workflows | Supported | Supported | GHC/Cabal, `base >=4.20 && <4.23` |
| Segno | Supported | Supported | Single-node Haskell driver |
| Motivo Studio development/package gate | Supported | Supported | Node.js >=22.12 |
| Motivo per-user launcher | Windows x64 | Not supplied | Installs below `%LOCALAPPDATA%` |
| Real active-window capture | Supported | Not supplied | Other platforms can use a replacement plugin |

The current distribution is built from a checkout. It does not yet provide a
signed system package, MSI, Homebrew formula, or Linux desktop installer.

## Prerequisites

Install these tools before building:

- Git;
- stable Rust and Cargo;
- GHC and Cabal through GHCup, with a GHC whose bundled `base` is in the
  declared range;
- Node.js 22.12 or newer only for Motivo Studio; and
- Python plus MkDocs only when building this documentation site.

A coding-agent CLI is not required for compilation or offline tests. Install
and authenticate `codex`, `claude`, or `opencode` only for the provider that
will receive live requests.

## Windows PowerShell

Clone or open the repository, then install Tactus into Cargo's per-user binary
directory. Adjust the checkout path once and reuse the variable:

```powershell
$repoRoot = (Resolve-Path D:\src\Agenstro).Path
$toolBin = Join-Path $env:USERPROFILE ".cargo\bin"
$env:PATH = "C:\ghcup\bin;$toolBin;$env:PATH"
Set-Location $repoRoot

cargo install --path tactus-runtime --bin tactus --locked --force

Get-Command tactus -All
tactus --version
tactus check --help | Select-String -Pattern '--package'
```

The `--package` check distinguishes the current binary from an earlier
`0.3.0` build that used the same version number before package extension was
added. Open a new terminal if the old executable still resolves first.

Install Segno when persistent tasks are needed:

```powershell
Set-Location $repoRoot
cabal update
cabal build --builddir=Build/cabal all --enable-tests
cabal install segno-flow:exe:segno `
  --builddir=Build/cabal `
  --installdir $toolBin `
  --overwrite-policy=always

Get-Command segno -All
segno --version
```

Clef is a library, not a second global command. Each Tactus workspace links
to the `clef-sdk` source package through `.tactus/cabal.project`.

Install Motivo Studio for the current Windows user:

```powershell
Set-Location $repoRoot
npm --prefix motivo-studio ci
npm --prefix motivo-studio run install:windows

Get-Command motivo-studio -All
motivo-studio --version
```

The installer packages the application into
`%LOCALAPPDATA%\Programs\MotivoStudio` and adds that exact directory to the
user `PATH`. Start a new terminal before testing the command if necessary.

## Ubuntu shell

Put Cargo and the selected Cabal install directory on `PATH`, then run from the
repository root:

```bash
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"

cargo install --path tactus-runtime --bin tactus --locked --force
cabal update
cabal build --builddir=Build/cabal all --enable-tests
cabal install segno-flow:exe:segno \
  --builddir=Build/cabal \
  --installdir "$HOME/.local/bin" \
  --overwrite-policy=always

tactus --version
segno --version
tactus check --help | grep -- '--package'
```

Motivo can be developed and packaged on Ubuntu, but this release does not
install a global Linux desktop launcher. Use `npm --prefix motivo-studio start`
for development only.

## Initialize the first workspace

From any project directory, pass the Clef SDK explicitly on first setup:

```powershell
$projectRoot = "D:\work\my-project"
New-Item -ItemType Directory -Force $projectRoot | Out-Null
Set-Location $projectRoot

tactus init --sdk (Join-Path $repoRoot "clef-sdk")
tactus doctor
tactus list
tactus smoke
```

`init` creates missing files and preserves existing ones. Tactus commands
search upward from `--root` or the current directory for
`.tactus/tactus.toml`.

Continue with [First workflow](getting-started.md). Configure live model calls
with [Provider setup](providers.md).

## Upgrade

Pull the desired commit, then rerun the same install commands:

```powershell
Set-Location $repoRoot
git pull --ff-only origin main
cargo install --path tactus-runtime --bin tactus --locked --force
cabal install segno-flow:exe:segno `
  --builddir=Build/cabal `
  --installdir $toolBin `
  --overwrite-policy=always
npm --prefix motivo-studio run install:windows
```

Close Motivo before replacing it. Existing `.tactus` content is not rewritten
implicitly; run `tactus doctor` after an upgrade and `segno init` again when a
moved checkout changes the Segno package path.

## Uninstall

Remove the source-installed commands without deleting project workspaces:

```powershell
cargo uninstall tactus-runtime
Remove-Item (Join-Path $toolBin "segno.exe") -ErrorAction SilentlyContinue
npm --prefix motivo-studio run uninstall:windows
```

Deleting `.tactus` is a separate destructive action: it removes workflow
scripts, run diagnostics, configuration, skills, and any Segno state below
that project. Use the [operations guide](operations.md) before doing so.

## Verify the checkout before release use

The complete source gate is described in the repository's
[CONTRIBUTING.md](https://github.com/Tensorera/Agenstro/blob/main/CONTRIBUTING.md).
The minimal local verification is:

```powershell
cargo test -p tactus-runtime --locked
cabal test --builddir=Build/cabal all --test-show-details=direct
npm --prefix motivo-studio test
python -m mkdocs build --strict
```

These tests use local fakes and do not authenticate with a model provider.
