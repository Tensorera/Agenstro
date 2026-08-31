---
title: Norm and session design-bundle implementation status
status: working decision record
owners: [architecture]
last_verified: 2026-08-20
applies_to: feature/norms-rubrics-motivo-sessions
platforms: [windows, ubuntu]
---

# Norm and session design-bundle implementation status

This page records how the 2026-08-20 `agenstro-design-bundle` was applied. It
is deliberately more explicit than a roadmap: implemented items are separated
from adaptations and unresolved decisions so a later release decision does not
have to reconstruct intent from a diff.

## Implemented in this branch

| Area | Decision |
| --- | --- |
| Clef norms | Add typed norm, serializable check-spec, provenance, violation, rubric, critique, bounded guidance, judgement, and refinement APIs. |
| Checker boundary | Keep check specifications as data and send serializable catalogues through an ordinary `agenstro.plugin/v1` checker. |
| Reference checker | Add the Python LaTeX checker and its protocol/domain fixture suite. |
| Tactus sessions | Add bounded `session list`, `session show`, and compare-and-set `session answer` control commands over a workspace-owned store. |
| Motivo sessions | Add validated session IPC, multi-session selection, decision and roadmap projection, sourced/unsourced findings, comparable option coordinates, stakes, notes, and stale-turn refetch. |

## Adaptations made for compatibility or correctness

| Bundle proposal | Implemented interpretation | Reason |
| --- | --- | --- |
| Two to six options in the TypeScript contract; three to six in one Haskell comment | Two to six | The normative reference and the bundle's own desk example require a valid binary choice. |
| Strict Zod schemas and additive v1 compatibility | Strip additive fields only while parsing the external Tactus envelope, then project into strict internal/IPC schemas | This preserves forward-compatible Rust DTOs without weakening the hostile-renderer boundary. |
| Sessions navigation assumes one live session while the schema exposes `list` | Provide a picker | It preserves current data without silently choosing a session and does not settle future concurrency policy. |
| Haskell-local interpretation of all `CheckSpec` patterns | Use a language-neutral checker for serializable specs; retain native Haskell checks as an escape hatch | It avoids silently giving one norm different regex semantics in Haskell and Python. |
| `mostViolatedFirst` guidance selection | Keep history-free guidance deterministic, and add `rubricGuidanceWithHistory` for an explicit caller-supplied violation history | The proposed selector has no automatic journal input; an explicit parameter is honest without coupling rubrics to runtime records. |

## Preserved for final adjudication

These items are intentionally not represented by placeholder buttons or APIs:

1. **`session advance` and the Clef planner API.** The bundle specifies planner
   purity and a `Selective` ceiling but does not specify how a durable session
   identifies and invokes its planner executable. Implementing the action now
   would expose a dead path or invent a second runtime contract. Consequently,
   this stage can validate, list, and answer producer-authored documents but
   has no supported command that creates the first session; the Motivo surface
   is staged until that producer contract is accepted.
2. **Transcript projection and preference mining.** Tactus preserves answer
   evidence, but `SessionView` has no transcript payload and the control API has
   no transcript query. Defaulted-choice exclusion and mining need that contract
   before Motivo can present or consume them.
3. **Automatic default application and expiry.** The desktop must never forge a
   preference. A scheduled runner policy, timestamps, and audit semantics need
   a separate decision.
4. **SARIF export, catalogue policy overlays, and norm-id migration tooling.**
   The compact norm wire format keeps the required information, but no export
   command or project policy store is selected yet.
5. **Norm mining and journal economics.** Corpus/edit mining must produce
   reviewable proposals; automatic promotion remains prohibited.
6. **Rejected-round workflow diagnostics.** `refine` can propagate failures and
   return each critique, but the public `Workflow` abstraction does not expose
   `Runtime` to library combinators. Emitting a special rejected transition is
   deferred until a narrow diagnostic operation can be added without breaking
   that abstraction.
7. **Release numbering.** The bundle targets Motivo `0.4`, while the repository
   still identifies the coordinated release as `0.3`. This branch does not
   perform a repository-wide version bump without a release decision.
8. **Multi-session lifecycle policy.** Storage and UI support multiple
   sessions, but limits on simultaneous live sessions, abandonment, and expiry
   remain open.
9. **Hostile namespace races.** Session I/O rejects static symlinks, Windows
   reparse-point directories, hard-linked auxiliary files, and observed path/
   handle mismatches. It is not yet implemented with directory-handle-relative
   open/rename on both Unix and Windows, so a same-authority hostile process
   racing parent-directory replacement remains outside the trusted-workspace
   threat model. Closing that gap requires a dedicated cross-platform storage
   backend rather than another path recheck.

## Existing functionality retained

The change does not alter Segno, replace `Workflow`, remove
`runtimeInstructions`, grant the Motivo renderer path or filesystem authority,
or introduce a daemon. Existing Overview, Workflow, Plugins, Runs, action
streaming, cancellation, trace projection, and plugin protocols continue to
use their established boundaries.
