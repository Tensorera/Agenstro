# Security policy

Agenstro `0.3` is an AGPL-3.0-only source alpha for trusted local development.
It is not a security boundary.

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

## Supported security claims

The current project aims to:

- reject malformed or mismatched `agenstro.plugin/v1` frames;
- keep protocol stdout separate from human diagnostics;
- preserve an explicit `outcome_unknown` classification when an external
  provider may have acted before transport failure;
- avoid shell parsing for configured command arrays; and
- keep credentials, runtime state, private notes, and generated transcripts out
  of the repository.

It does not currently claim hostile-code isolation, plugin signing, credential
brokering, exactly-once execution, reliable process-tree termination, or
deterministic replay of arbitrary Haskell `IO`.

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
