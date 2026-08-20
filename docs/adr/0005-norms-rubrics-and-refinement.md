# ADR-0005: Typed norms, rubrics, and bounded refinement

- Status: Accepted, staged
- Date: 2026-08-20
- Scope: Clef and `agenstro.norm/v1`
- Extends: ADR-0003

## Context

Clef checks how workflow values are wired, but a correctly wired workflow can
still produce a poor artifact. Domain conventions previously had one flat
home, `runtimeInstructions`, which is workspace-wide prose: it is not scoped,
composable, versioned, or mechanically checkable.

Quality is also not one verification regime. A single artifact can contain
decidable facets, community conventions, stable personal preferences, and
decisions that have not yet been formed. Automation must be selected per
facet. In particular, uncertainty is biased toward the less decidable regime:
mistaking an undecided preference for a convention silently optimizes toward a
median answer.

## Decision

### A norm is a typed value

`Norm artifact` carries a stable dotted identity, a human statement, severity,
optional generation guidance, provenance, and an optional check. The artifact
parameter prevents a rubric for one artifact type from being applied to
another.

The closed severity order is `Preference`, `Style`, `Correctness`, and
`Blocking`. Blocking controls delivery; it is deliberately distinct from
correctness, confidence, and auto-fixability.

### Checks are data first

The normal check is a serializable `CheckSpec`: `Existence`, `Absence`,
`Occurrence`, `Consistency`, `Sequence`, `Metric`, or `ExternalCheck`.
Parameterized data can be mined, reviewed, diffed, transported to a checker in
another language, and stored without serializing a Haskell function.

A native Haskell check remains an escape hatch. Repeated native implementations
of the same shape are evidence that the closed wire format needs a new
constructor.

### Rubrics compose and report uncertainty honestly

`Rubric artifact` is a monoid over norms. It has two projections:

- bounded guidance for generation; and
- `judge`, which returns a `Critique` containing violations plus explicit
  `checked` and `unchecked` norm identities.

An empty violation list is not a claim of quality when norms were unchecked.
External checker diagnostics that must affect refinement belong in the
plugin's terminal value, not stderr or event observations.

### Refinement is bounded

`refine` alternates generation and judgement for a caller-supplied maximum
number of rounds. A generator receives the previous critique so repair does
not require handwritten prompt logic. An unknown external outcome propagates;
it is never treated as an ordinary rejected candidate or retried implicitly.

### The wire format is a SARIF profile

The compact plugin payload is `agenstro.norm/v1`. It maps norms to SARIF rules,
violations to results, satisfied checked norms to `pass`, and unchecked norms
to notifications. Coordinates are one-based and inclusive. LSP coordinates
remain a distinct type and require explicit conversion.

`runtimeInstructions` remains supported. It can coexist with rubric guidance,
so adopting norms does not invalidate existing workspaces.

## Current implementation boundary

This stage includes the typed Clef values and combinators, the compact wire
contracts, and a language-neutral LaTeX checker with fixtures. It deliberately
does not claim:

- automatic norm mining or promotion;
- journal-derived violation economics;
- project policy overlays or SARIF export;
- a reliable whole-task regime classifier; or
- mutation-safe refinement for generators that edit shared files in place.

Mined norms must remain proposals until a person accepts them. Learning from
uncorrected generated output would otherwise turn accumulated drift into a
house style.

## Consequences

Domain expertise becomes reusable, scoped data instead of prompt fragments.
Generation and validation refer to the same norm definition, and a checker can
say what it did not verify. The closed check language is intentionally less
expressive than arbitrary code; that constraint is what makes catalogues
portable and mineable.
