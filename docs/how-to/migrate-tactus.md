# Migrate a Tactus v1 runtime

This procedure is for legacy detached-worktree runtime roots. A new
project-local Studio runtime is initialized or resumed in place and does not
need this migration.

Migration is explicit. Ordinary load, status, run, loop, and recover operations
do not migrate a v1 database.

## 1. Stop v1 hosts

Stop every process that can use the runtime root. Confirm this outside
Tactus when the owner is on another machine or cannot be inspected.

## 2. Check the runtime

```powershell
$runtime = "D:\TactusRuns\legacy"
tactus doctor --root $runtime
```

## 3. Migrate

```powershell
tactus migrate `
  --root $runtime `
  --confirm-hosts-stopped
```

The confirmation is a user assertion. It does not terminate a host and cannot
override an owner that Tactus can prove is alive.

Migration first validates the dedicated linked worktree. It then backfills
phase and attempt state in one SQLite transaction and runs integrity checks.
Any validation failure rolls the transaction back.

## 4. Verify and recover

```powershell
tactus doctor --root $runtime
tactus rebuild-notebook --root $runtime
```

A v1 running cell has no kernel launch-generation evidence. After migration,
confirm again that the old process stopped before using:

```powershell
tactus recover --root $runtime --force
```
