# Security policy

Agenstro `0.3` is an AGPL-3.0-only source alpha for trusted local development.
Its general workflow runtime is not a security boundary. The optional
`motivo.test` effect has the narrower execution boundary described below.

## Execution model

Haskell workflow programs, configured plugins, and native coding-agent CLIs
run with the operating-system authority and environment of the user who starts
Tactus. They may read or modify files, launch processes, access inherited
credentials, and contact external services.

The reference provider adapters intentionally request their most permissive
non-interactive modes. Argument arrays avoid shell-string parsing, but they do
not authenticate, authorize, or sandbox the selected executable. GHC type
checking verifies Haskell value wiring; it does not establish that a program is
safe to execute.

The `workspace.paths` effect reports selected before/after path metadata. It is
not access control, complete auditing, attribution, backup, or rollback.

Session storage rejects static symlink, hardlink, and Windows reparse-point
substitutions that it observes. Filesystem operations are not yet uniformly
relative to a pinned directory handle on every supported platform; a hostile
same-authority process racing directory replacement is therefore outside the
trusted-local-workspace threat model.

## Supported security claims

The current project aims to:

- reject malformed or mismatched `agenstro.plugin/v1` frames;
- keep protocol stdout separate from human diagnostics;
- preserve an explicit `outcome_unknown` classification when an external
  provider may have acted before transport failure;
- avoid shell parsing for configured command arrays; and
- keep credentials, runtime state, private notes, and generated transcripts out
  of the repository.

Ordinary workflow execution does not claim hostile-code isolation, plugin
signing, credential brokering, exactly-once execution, complete termination of
arbitrary detached process trees, or deterministic replay of Haskell `IO`.

## Motivo experiment boundary

On Linux, `motivo.test` uses Bubblewrap namespaces and read-only mounts to
restrict experiment writes to `.tactus/motivotest/<run-id>/<sample-id>`. It
isolates the network, withholds the host environment and user configuration,
and supervises all experiment descendants within one deadline of at most
600 seconds. Missing isolation support is an error; no unrestricted fallback
is provided. Other platforms currently cannot execute this effect.

This boundary applies to the experiment command, not the enclosing Haskell
program, main coding agent, or optional independent provider call. Project
files remain readable, including any secrets stored in the project. Time and
retained log limits are not general disk, RAM, or CPU quotas. The host-side
workspace and plugin are trusted; a same-authority host process racing path
replacement is outside this boundary. See the
[plugin specification](plugins/motivo-test/README.md) for mount and supervision
details and the real Linux isolation tests.

Motivo reports embed escaped text and require no server or external resources.
They still contain user-supplied task evidence, which should be reviewed before
sharing. A rendered report is neither verification of that evidence nor an
execution capability.

## Handling credentials

Do not store provider tokens or machine-local configuration in
`.tactus/tactus.toml`, workflow sources, test fixtures, issue attachments, or
Git history. Prefer each native provider CLI's normal credential mechanism and
pass only the minimum required environment to a workflow session.

Before sharing diagnostics, remove prompts, environment dumps, home-directory
paths, account identifiers, and provider output that may contain private
source. Rotate a credential immediately if it was committed or included in a
model transcript; deleting the latest file revision is not sufficient.

## Reporting a vulnerability

Report suspected vulnerabilities to the repository owner through a private
channel. Include the affected commit, platform, minimal reproduction, observed
impact, and whether any provider call or external side effect occurred. Do not
open a public issue containing an unredacted exploit, credential, prompt, or
private workspace content.
