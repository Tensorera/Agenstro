---
title: Install Agenstro 0.3
status: alpha
owners: [release]
last_verified: 2026-09-05
applies_to: "Clef/Segno 0.3.0.0, Tactus/Motivo 0.3.0"
platforms: [windows, ubuntu]
---

# Install Agenstro 0.3

Agenstro is built from a checkout. The user works in an existing coding-agent
CLI; Motivo supplies a skill, Haskell templates, and offline HTML reports.
There is no desktop client or background server to install.

## Prerequisites

- Git, stable Rust and Cargo.
- GHC and Cabal with `base >=4.20 && <4.23`.
- A coding-agent CLI of your choice, configured in its own interface for live work.
- Linux Bubblewrap 0.8 or newer with usable user namespaces for strictly bounded Motivo experiments.
- Node.js >=22.12 only for report-template development/tests; Python/MkDocs only for repository checks/docs.

Clef is a library. Tactus workspaces reference its source package through
`.tactus/cabal.project`; it has no separate global executable.

## Install commands from the checkout

On Ubuntu or another supported Linux environment:

```sh
cargo install --path tactus-runtime --bin tactus --locked --force
cargo install --path plugins/motivo-test --locked --force

tactus --version
tactus run --help
```

Put Cargo's binary directory and the chosen GHC/Cabal tools on `PATH`. When
using a toolchain manager, preserve its selected executable order. The Motivo
experiment backend also requires `bwrap`; the plugin checks whether isolation
actually works and refuses execution if it does not.

The same Cargo commands can be used from Windows PowerShell. Install GHC/Cabal
through a supported toolchain manager. The `motivo-test` executable can report
its capabilities on Windows, but the strict experiment backend is currently
Linux only. Methods that record, analyze, research, or present evidence do not
require this experiment backend.

## Initialize and start your coding agent

Choose the desired project and initialize it using an explicit Clef source path:

```sh
cd /path/to/project
tactus init --sdk /path/to/Agenstro/clef-sdk
tactus doctor
tactus list
```

Start your chosen coding agent from this folder and ask it to use Motivo. The
canonical skills are in `.tactus/skills/{motivo,tactus}`. Initialization installs
project-local discovery pointers for supported hosts and preserves existing
files. If your host does not expose skills, ask it to read the canonical skill
path directly.

Open `.tactus/motivo/index.html` in a browser to observe recorded work. All task
instructions and feedback stay in the original coding-agent conversation.

`tactus smoke` is offline by default. Live probes are explicit and may contact
a provider. Unsupported Motivo experiment isolation must not be mistaken for a
broken Haskell or provider installation.

## Upgrade an existing project

Update the checkout and rerun the two Cargo installation commands. Clef changes
are picked up from the SDK path recorded in the project's Cabal configuration;
update that reference if it points at an older copied SDK.

`tactus init --sdk ...` creates missing assets and preserves existing files,
including user-edited templates, skills and `tactus.toml`. Inspect its created,
preserved and skipped entries. It does not silently upgrade existing method
code or overwrite a user's provider settings.

If an existing configuration lacks the new experiment effect, add only this
entry while preserving other settings:

```toml
[effects."motivo.test"]
command = ["motivo-test"]
```

The old Electron installation is no longer used. Existing `.motivo/tasks`
records remain historical files; initialization neither deletes nor resumes
them. Use the new report files through your browser.

## Optional Segno installation

Segno remains an independent, experimental persistent scheduler:

```sh
cabal update
cabal build --builddir=Build/cabal all --enable-tests
cabal install segno-flow:exe:segno \
  --builddir=Build/cabal \
  --installdir "$HOME/.local/bin" \
  --overwrite-policy=always
```

On Windows, use your selected executable directory and PowerShell line
continuations. See the [Segno guide](segno.md) for its separate initialization.

## Verify the checkout

```sh
cargo test --workspace --locked
cabal test --builddir=Build/cabal all --test-show-details=direct
npm --prefix motivo-studio ci
npm --prefix motivo-studio test
npm --prefix motivo-studio run build
python -m mkdocs build --strict
```

The canonical complete checks are in [CONTRIBUTING.md](https://github.com/Tensorera/Agenstro/blob/main/CONTRIBUTING.md).
Offline checks do not invoke a real model. The strict experiment tests launch
local sandboxed programs to verify their actual write and deadline boundaries.

Continue with [Motivo methods](motivo-studio.md), [First workflow](getting-started.md),
and [Provider setup](providers.md).
