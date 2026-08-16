# Recover a Tactus runtime

Use this procedure when a Tactus host exits while a cell remains
`RUNNING`. Recovery requires evidence that the old process tree no longer owns
the runtime.

## 1. Inspect the runtime

```powershell
$runtime = "D:\TactusRuns\first-run"

tactus status --root $runtime
tactus doctor --root $runtime
```

Stop if the reported run or recovery owner is still alive. Do not run two
recovery processes for the same root.

## 2. Recover the cell

```powershell
tactus recover --root $runtime
```

Recovery claims the running cell before reading resource snapshots or changing
Git state. It reconciles a proven checkpoint or rolls tracked files back to the
recorded base commit.

Use `--force` only after independently confirming that an owner which cannot be
verified has stopped:

```powershell
tactus recover --root $runtime --force
```

`--force` cannot take ownership from a process that Tactus can prove is
alive. It cannot bypass kernel containment checks.

## 3. Review warnings

```powershell
tactus warnings --root $runtime --all
```

Git-tracked files can be restored. Modified or deleted non-Git resources may
only produce warnings because path snapshots do not preserve their bytes.

## 4. Rebuild the Notebook projection

SQLite is the state authority. Rebuild a missing or stale human-readable
Notebook after recovery:

```powershell
tactus rebuild-notebook --root $runtime
```

See [Tactus architecture](../explanation/tactus-runtime-architecture.md) for the
claim, kernel containment, and rollback protocols.
