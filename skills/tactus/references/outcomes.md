# Ambiguous outcomes and diagnostics

## Diagnose without replaying

When execution fails, times out, is interrupted, loses its terminal response, or reports `OutcomeUnknown`:

1. Do not rerun automatically.
2. Record the command, selected script, occurrence or run ID, exit state, and bounded diagnostic.
3. Inspect read-only evidence: Motivo's run projection or `tactus studio inspect`, the selected run's `tactus studio events`, Segno history, the last durable business-state revision, and the external system keyed by the occurrence's idempotency key.
4. Distinguish a definite failure from an ambiguous result. A transport failure can occur after the external effect completed.
5. Ask the user or operator to reconcile the external outcome before choosing a new attempt.

`OutcomeUnknown` is terminal and intentionally not retried automatically. Do not mark it succeeded or failed, edit the private Segno SQLite lifecycle database, or assume at-least-once delivery authorizes duplication. A successful earlier checkpoint remains durable even if a later step is ambiguous.

## Present diagnostics

Prefer canonical runtime presentation fields when available:

```text
[state] Run started.
[info] Type-check completed.
[warning] The terminal outcome could not be proven; no retry was attempted.
[error] GHC rejected the selected source.
```

Keep stable error codes, exit codes, file and line locations, integrity (`ok`, `partial`, or `corrupt`), and bounded raw payloads as technical details. `partial` means incomplete evidence, not failure; `corrupt` means the trace cannot be trusted as complete. Never expose credentials, provider options, prompts, absolute private paths, or unrestricted journal contents in a summary.
