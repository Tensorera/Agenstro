# Run outcomes

Inspect recorded evidence without executing a workflow again:

```sh
tactus studio inspect --root /path/to/project
tactus studio events <run-id> --root /path/to/project --after 0 --limit 100
```

A successful `check` establishes compilation. A successful `run` establishes
that the selected local entry returned successfully. Neither proves that a
model's report or an external business outcome is correct.

After a timeout, interruption, lost terminal response, or `OutcomeUnknown`,
retain the run ID and diagnostics, inspect the relevant artifacts or external
system, and reconcile the outcome before another attempt. Journals are evidence,
not deterministic replay or rollback. Do not rewrite runtime journals or Segno
lifecycle databases to change an outcome.

Report what was checked, what the records show, and what remains unknown.
Preserve useful error codes and source locations. Tactus's presentation categories
are `state`, `info`, `warning`, and `error`; stderr alone is not proof of failure.
