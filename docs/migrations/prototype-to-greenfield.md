---
title: Migrate prototype data to the greenfield alpha
status: alpha
owners: [clef, runtime, segno]
last_verified: 2026-08-01
applies_to: "prototype Clef workflow v1 to 0.2.0 workflow v2"
platforms: [windows, ubuntu]
---

# Migrate Prototype Data to the Greenfield Alpha

This is a one-time user-data conversion, not Python internal API compatibility.
Only prototype Clef `WorkflowPlan.to_dict()` JSON is convertible today.
Prototype Tactus databases/notebooks and Segno registries/run history are not
imported.

## What Is and Is Not Imported

| Input | Result | Status |
| --- | --- | --- |
| Clef `clef.workflow/v1` workflow JSON | `clef.workflow/v2` JSON | Implemented by `convert_legacy_workflow` |
| Legacy artifact constraints | None | Rejected; requires an explicit semantic rewrite |
| Python runtime/profile/storage/provider command fields | None | Deliberately discarded |
| Tactus `.tactus` SQLite, notebooks, Git checkpoints | None | No importer |
| Segno JSON registry, scheduler state, logs, artifacts | None | No importer |
| Provider credentials/configuration | None | Never read or modified |

The converter accepts at most 4 MiB of JSON and bounds maps/sequences to 4,096
items. It rejects duplicate keys, non-standard constants, malformed identities,
and unsupported values.

## 1. Preflight and Backup

Stop prototype writers. Copy the complete project and state directories to a
read-only backup on a different path or volume. Record source hashes before
conversion.

Windows PowerShell:

```powershell
Get-FileHash -Algorithm SHA256 D:\migration\workflow-v1.json
Copy-Item D:\migration\workflow-v1.json D:\migration\backup\workflow-v1.json
```

Ubuntu Bash:

```bash
sha256sum /srv/migration/workflow-v1.json
cp --preserve=all /srv/migration/workflow-v1.json /srv/migration/backup/workflow-v1.json
```

Reserve enough free space for the backup, converted JSON, and any later new
state. This alpha has no reliable estimator for prototype Tactus/Segno data.

## 2. Dry Run

Run the converter without writing output:

```powershell
.\.venv-clef\Scripts\python.exe -c "import json,pathlib; from clef_sdk import convert_legacy_workflow; p=pathlib.Path(r'D:\migration\workflow-v1.json'); w=convert_legacy_workflow(p.read_text(encoding='utf-8')); print(json.dumps({'schema_version': w.to_dict()['schema_version'], 'tasks': len(w.tasks)}, sort_keys=True))"
```

```bash
./.venv-clef/bin/python -c "import json,pathlib; from clef_sdk import convert_legacy_workflow; p=pathlib.Path('/srv/migration/workflow-v1.json'); w=convert_legacy_workflow(p.read_text(encoding='utf-8')); print(json.dumps({'schema_version': w.to_dict()['schema_version'], 'tasks': len(w.tasks)}, sort_keys=True))"
```

The expected `schema_version` is `clef.workflow/v2`. A
`LEGACY_CONVERSION_REQUIRED` error means behavior cannot be preserved
automatically; stop and rewrite that workflow explicitly.

## 3. Convert Once

Use an adjacent temporary file and rename only after validation. The following
script is identical on Windows and Ubuntu apart from paths:

```python
import json
from pathlib import Path

from clef_sdk import convert_legacy_workflow

source = Path("workflow-v1.json")
target = Path("workflow-v2.json")
temporary = target.with_suffix(".json.tmp")
workflow = convert_legacy_workflow(source.read_text(encoding="utf-8"))
payload = json.dumps(workflow.to_dict(), ensure_ascii=False, indent=2, sort_keys=True) + "\n"
temporary.write_text(payload, encoding="utf-8", newline="\n")
json.loads(temporary.read_text(encoding="utf-8"))
temporary.replace(target)
```

Do not replace the v1 backup. Remove the conversion call from normal runtime
code after saving v2; the new system must not dual-read prototype JSON as an
authority.

## 4. Verify

Record the source and result SHA-256 hashes, compare workflow/task/output counts,
and load the v2 result with current public builders or the checked-in fixture
tests. Re-run conversion from the unchanged v1 backup into another temporary
path and compare bytes to establish deterministic rerun behavior.

Do not claim a completed system migration if any Tactus or Segno state remains
needed. Archive those inputs and recreate supported workflow/package definitions
manually; there is no importer that can validate their state invariants.

## Interruption and Rerun

Interruption before the final rename leaves the v1 source unchanged and, at
most, a `.tmp` file. Delete only that known temporary file and rerun. Conversion
is deterministic for identical valid input. No daemon database is opened.

## Rollback and Irreversible Points

Rollback means selecting the untouched prototype project and backup state with
the prototype software. The v2 JSON does not mutate v1. New daemon databases,
if created during separate experiments, must use separate state roots and must
not be presented to prototype binaries.

There is no tested downgrade from v2 to v1, no reverse database migration, and
no safe point at which new SQLite/CAS state becomes prototype-compatible. Keep
the backup until the greenfield release supplies and passes data migration,
backup/restore, and rollback tests on the relevant platform.
