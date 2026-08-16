# Segno Flow

> **0.3 refactor status: frozen.** The scheduler below is 0.2 migration
> evidence, not the new replay abstraction. Replay work starts only after the
> Clef plugin trace format is stable, and must distinguish recorded-result
> replay from explicit live re-invocation.

`segno-flow` is the `0.2.0` alpha scheduling project. The distribution remains
`segno-flow`, the import remains `segno_flow`, and the commands remain
`segno-flow` and `segno-flow-ui`. Rust `segnod` owns revisions, schedules,
occurrences, leases, dispatch intent, and its private SQLite database.

## Minimal Success Path

After installing this source tree into Python 3.11 or 3.12, build the included
task package offline:

```powershell
segno-flow package build examples/daily-summary examples/daily-summary/dist/daily-summary.zip
```

The builder validates bounded portable paths and archive budgets without
importing or executing package scripts. Daemon-facing `import`, `list`, `run`,
and `status` commands currently fail with `UNAVAILABLE` because authenticated
discovery and generated Python bindings are not shipped.

## References

- [`segno-flow` CLI source](src/segno_flow/cli.py)
- [Package budgets](src/segno_flow/package.py)
- [Scheduler limits and lifecycle](rust/segnod/src/service.rs)
- [SQLite schema and errors](rust/segnod/src/store.rs)
- [Current support matrix](../docs/reference/support-matrix.md)
