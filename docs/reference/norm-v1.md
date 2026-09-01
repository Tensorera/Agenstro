---
title: Norm catalogue and check result v1
status: alpha
owners: [clef]
last_verified: 2026-09-01
applies_to: "agenstro.norm/v1"
platforms: [windows, ubuntu]
---

# Norm catalogue and check result v1

`agenstro.norm/v1` is the language-neutral compact representation of domain
norms and checker results. It travels as the `params` and terminal `value` of
an ordinary `agenstro.plugin/v1` call. It is a profile of SARIF 2.1.0 rather
than a competing diagnostics model.

## Catalogue records

A serializable norm uses this shape:

```json
{
  "id": "math.notation.upright-differential",
  "statement": "Use an upright differential operator.",
  "severity": "Style",
  "guidance": "Write \\,\\mathrm{d}x in integrands.",
  "spec": {
    "kind": "Absence",
    "specPattern": "(?<!\\\\mathrm\\{)\\bd(?=[a-zA-Z]\\b)",
    "specIgnoreCase": false
  },
  "provenance": {"kind": "Authored", "author": "project"},
  "supersedes": []
}
```

`id` is stable and should use a dotted hierarchy. `guidance` and `spec` are
optional. A missing spec means guidance-only; it must not be reported as a
pass. `severity` is the closed set `Preference`, `Style`, `Correctness`, and
`Blocking`.

Provenance uses an open `kind`; the current producers use:

- `{"kind":"Authored","author":"..."}`;
- `{"kind":"MinedFromCorpus","corpus":"...","support":38,"total":40}`;
  and
- `{"kind":"MinedFromEdits","observations":12}`.

Consumers must preserve provenance they do not interpret.

## Check specifications

`spec.kind` is a closed discriminator. Adding a kind changes the wire format;
adding another norm does not.

| Kind | Fields | Violation condition |
| --- | --- | --- |
| `Existence` | `specPattern`, `specIgnoreCase` | Pattern is absent. |
| `Absence` | `specPattern`, `specIgnoreCase` | Pattern is present. |
| `Occurrence` | `specPattern`, `specBound` | Match count is outside the inclusive bound. |
| `Consistency` | `specGroups` | More than one member of a group occurs. |
| `Sequence` | `specOrdered` | Patterns do not occur in the listed relative order. |
| `Metric` | `specMetric`, `specBound` | The named measurement is outside the inclusive bound. |
| `ExternalCheck` | `specPlugin`, `specMethod`, optional `specParams` | Routed to a norm-v1 wrapper method on another plugin. |

A bound is:

```json
{"boundMinimum": 1, "boundMaximum": null}
```

At least one endpoint must be non-null, and if both are present the minimum
must not exceed the maximum. Pattern-bearing specs use non-empty patterns of
at most 4,096 UTF-8 bytes. `Consistency` contains at least one group, and each
group contains at least two distinct valid patterns. A norm check source is at
most 524,288 UTF-8 bytes; larger requests are rejected rather than partially
checked.

An unsupported kind, unsupported metric, or malformed pattern is `unchecked`.
It is never converted to zero violations. A malformed specification is a norm
defect and may also emit a non-authoritative `norm.check_failed` event.
Norm-v1 deliberately does not standardize a regex dialect or engine. A checker
must bound its own evaluation. The reference Python checker uses a killable
one-second worker process, so malformed, catastrophically backtracking, and
otherwise over-budget patterns become `unchecked`. This is a resource-safety
classification, not proof that the same pattern is unsafe in every engine.

`ExternalCheck` does not call an arbitrary legacy method directly. The named
plugin method must accept the same `NormCheckRequest` and return the same
`NormCheckResult` shown below. It reads adapter-specific configuration from
each routed norm's `specParams`; norms sharing plugin, method, and projected
source may arrive in one batch. A compiler, ChkTeX, or similar tool therefore
needs a thin norm-v1 adapter that maps its native diagnostics into violations.

## Plugin call

The checker method is `check`:

```json
{
  "api": "agenstro.plugin/v1",
  "id": "clef-7",
  "method": "check",
  "params": {
    "artifact": "article.tex",
    "source": "...",
    "norms": [{"id": "...", "statement": "...", "severity": "Style", "spec": {}}]
  }
}
```

Its terminal value is:

```json
{
  "api": "agenstro.norm/v1",
  "artifact": "article.tex",
  "violations": [
    {
      "norm": "math.notation.upright-differential",
      "severity": "Style",
      "message": "Use an upright differential operator.",
      "locus": {
        "artifact": "article.tex",
        "startLine": 2,
        "startColumn": 11,
        "endLine": 2,
        "endColumn": 11,
        "snippet": "d"
      },
      "evidence": {}
    }
  ],
  "checked": ["math.notation.upright-differential"],
  "unchecked": ["math.style.proof-voice"]
}
```

`checked` and `unchecked` are required. Checker progress can use
`norm.checked` events, but only the terminal value is authoritative workflow
data.

Line and column coordinates are one-based and inclusive, matching SARIF
regions. LSP positions are zero-based with an exclusive end and commonly use
UTF-16 code units; an implementation supporting both must convert between
distinct coordinate types.

`artifact` is non-empty. `startColumn` requires `startLine`; `endLine` requires
`startLine`; and `endColumn` requires `endLine`. Coordinates are positive. An
end line cannot precede the start line, and columns cannot run backwards when
both endpoints are on the same line. Artifact-only and partial line-only loci
remain valid.

## Clef validation gates

Clef can project an existing `Critique` into gate-ready
`ValidationFailure` values. This is a local typed API, not a new norm checker
payload and not a plugin protocol revision. Every projected failure reuses:

- the Norm id as `rule`;
- the Norm statement, guidance, and check spec as `expected`;
- the Violation message, locus, and evidence as `observed`; and
- the Norm's existing `provenance`.

The caller supplies the validator `stage`: `structure`, `readability`,
`domain`, or `reviewer`. A severity threshold is inclusive. A `Correctness`
gate therefore rejects `Correctness` and `Blocking`; a `Blocking` gate rejects
only `Blocking`. Failed gates use workflow diagnostic code
`workflow.validation_failed` and place the projections under
`validation_failed`. The `agenstro.norm/v1` request/result above and
`agenstro.plugin/v1` remain unchanged.

## SARIF mapping

| Norm v1 | SARIF 2.1.0 |
| --- | --- |
| norm record | `run.tool.driver.rules[]` reporting descriptor |
| `id` | `reportingDescriptor.id` |
| `statement` | `shortDescription.text` |
| `guidance` | `help.markdown` |
| `supersedes` | `deprecatedIds` |
| violation | `run.results[]` |
| checked without a violation | result with `kind: "pass"` |
| unchecked | omitted result plus notification |

`Blocking` and `Correctness` both map to SARIF `error`, `Style` to `warning`,
and `Preference` to `note`. Preserve the original severity in properties
because SARIF level alone cannot round-trip the distinction between blocking
and correctness.

## Compatibility

- Reject an unknown top-level `api`.
- Tolerate additive fields in a recognized v1 external document.
- Reject unknown severity values.
- Treat an unknown check kind as unchecked.
- Give a materially changed norm a new identity and list replaced identities
  in `supersedes`.

The reference implementation is
`plugins/latex-norm-check/latex_norm_check.py`; its fixtures verify domain
results plus JSONL correlation, one terminal frame, and no frames after the
terminal.
