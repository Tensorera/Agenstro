# Test2 live-run lessons

## Hidden output-contract failure

Run `run-0bc20e8ac87a480ea13e86f9e67db207` used the real OpenCode adapter
against the original blind bundle. Inventory succeeded. The numeric and
methods branches initially failed deterministic verification, repaired their
artifacts in-session, then passed and published.

The theory branch produced the requested polynomial derivation and oblique
operator discussion, but repeatedly failed
`theory_inference_consistency`. The host verifier required fixed
`section_id`, `validation_id`, and semantic-anchor strings that were not
declared by the prompt, schema, benchmark spec, or upstream artifact. Its
repair evidence could report that the IDs were wrong, but could not reveal
the accepted IDs. After five same-class repair turns, the run was stopped
instead of spending more agent calls on an under-specified contract.

This is a framework-test finding, not a scientific negative result. A
deterministic verifier may keep independent numerical goldens and
anti-fabrication checks private, but structural output requirements must be
visible to the producer.

## Resolution

`benchmark-spec.json` now contains a public `theory_output_contract` with:

- the two required section IDs, statuses, and validation IDs;
- required evidence, reconstruction, operator-contract, and report anchors;
- forbidden claims that would promote an unidentifiable historical basis.

The theory prompt tells the agent to follow this public contract, and the
host verifier reads the same immutable benchmark input instead of carrying a
second hidden copy. Scientific values are still derived from the manuscript
and independently checked by host-side numerics; no supplementary answer or
golden numerical result was exposed.

This resolves the producer/verifier contract mismatch in the benchmark
design. It does not, by itself, establish that a later live agent run
succeeds or that its scientific candidates are historically identical to the
publisher SI.

The incomplete output and raw trace were preserved under
`output-agent-hidden-contract-failure` and the corresponding
`.clef-state/output-agent-hidden-contract-failure-*` directory for local
diagnosis. They are not accepted reproduction artifacts.

## Current live status

The public-contract live v2 run is still in progress. No success claim is made
for it here. Its eventual workflow state, typed `execution_summary`, task
repair/verification history, scientific assessment, and historical-identity
boundary must be reported from that run's own artifacts and traces.

`output-final` is a deterministic offline reference and cannot be used as a
substitute for that live evidence.
