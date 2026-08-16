# Use Segno Flow

Segno Flow registers trusted workflow packages, schedules them with a
five-field cron expression, and preserves run records, logs, workspaces, and
artifacts across process restarts.

## Install

From the Agentro repository root:

```powershell
py -3.12 -m venv segno-flow\.venv
.\segno-flow\.venv\Scripts\python.exe -m pip install -e ".\segno-flow[ui]"
.\segno-flow\.venv\Scripts\Activate.ps1
```

Use the `[dev,ui]` extra while developing Segno Flow itself.

## Prepare a package

A ZIP package must contain `segno-flow.json` at its root. The manifest names
three distinct Python entry points:

```json
{
  "schema_version": 1,
  "id": "daily-summary",
  "name": "Daily summary",
  "schedule": {"cron": "0 8 * * *", "timezone": "local"},
  "working_directory": "working",
  "scripts": {
    "pre": "scripts/pre.py",
    "main": "scripts/main.py",
    "post": "scripts/post.py"
  },
  "enabled": true,
  "timeout_seconds": 3600
}
```

Import validates archive paths, size limits, the manifest, cron and timezone
values, and Python compilation. It does not sandbox the package; import only
code you trust.

## Import and run

```powershell
segno-flow init
segno-flow import .\workflow.zip
segno-flow list
segno-flow run daily-summary
segno-flow runs daily-summary
```

Replacing an existing package is explicit:

```powershell
segno-flow import .\workflow.zip --replace
```

## Run the scheduler

```powershell
segno-flow service start
segno-flow service status
segno-flow service stop
```

Launch `segno-flow-ui` for the desktop interface. Minimizing or closing the UI
does not stop the independent scheduler service.

The default state root is `~/.segno-flow`. Override it with `--root` or
`SEGNO_FLOW_HOME`. Task scripts receive `SEGNO_TASK_DIR`,
`SEGNO_PACKAGE_DIR`, `SEGNO_WORKING_DIRECTORY`, `SEGNO_RUN_WORKSPACE`,
`SEGNO_ARTIFACTS_DIR`, and the corresponding run and phase-status variables.

Segno Flow does not automatically delete historical runs or catch up cron
occurrences missed while the service was offline. Apply an external retention
policy when a workflow runs frequently.
