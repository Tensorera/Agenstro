# Choosing and filling one method

These files are independent commands in `.tactus/motivoscript`. Their numbers
help find them; they do not impose an execution order. The calling agent does the
reasoning and writes the notes; the template publishes artifacts. It does not certify the content. An explicit
`--provider` optionally asks one separate agent context to perform the method;
the default records current-agent notes without a model call.

The English `##` headings below help organize method-specific panels, but are
optional. Write their contents in the user's language. Missing fields remain
unknown and the full original material is always readable. Empty inputs and
ambiguous duplicate headings are rejected; ordinary prose is accepted. A
`## Local reflection` section can answer the three questions in order, one bullet
per answer. Do not invoke a model just to complete missing headings or normalize
format. The renderer escapes raw HTML and does not execute source references.

| Template | Use when | Suggested `##` headings |
| --- | --- | --- |
| `010_clarify.hs` | An ambiguity changes what should be delivered | Goal; Constraints; Success signals; Unknowns; Next step |
| `020_investigate.hs` | A concrete unknown needs inspection | Question; Sources; Findings; Unknowns; Next step |
| `030_analyze.hs` | Several explanations or choices need comparison | Problem; Evidence; Hypotheses; Decision; Next step |
| `040_research.hs` | A decision depends on sourced external or unfamiliar material | Question; Sources; Comparison; Limits; Next step |
| `050_probe.hs` | A small local experiment can distinguish a hypothesis | Question; Hypothesis; Setup; Expected observation; Next step |
| `060_organize.hs` | Work needs coherent units and dependency order | Goal; Work units; Dependencies; Execution order; Next step |
| `070_retrospect.hs` | Observed results differ from expectations, or a completed attempt needs review | Expected; Observed; Evidence; Causes; Keep; Change; Next step |
| `080_handoff.hs` | Another session or person must continue without rebuilding all context | Goal; Current state; Changes; Checks; Open issues; Next step |

## Clarify

Record the intended result in the user's terms. Describe observable success
signals without inventing an objective test for every subjective outcome. Ask
only about a choice that materially changes the work.

Local reflection:
1. Which ambiguity was resolved?
2. Which assumption still needs confirmation?
3. What would change the goal?

The report presents goal, constraints, success signals, unknowns and next step
when supplied, alongside the reflection.

## Investigate

Give source paths/line numbers, commands or document locations and distinguish
observations from interpretations. Prefer the smallest source set that answers
the question. An entire-repository summary is not the default deliverable.

Local reflection:
1. Which observation changed the picture?
2. What remains unobserved?
3. Which investigation is no longer necessary?

The report presents the question, sources, findings, unknowns and next step.

## Analyze

Use an evidence or hypothesis table when it helps compare explanations. Include
what would contradict the leading explanation. Record the decision's practical
consequence; a plausible narrative alone is not a finding.

Local reflection:
1. Which explanation gained or lost support?
2. Where might the reasoning be wrong?
3. What evidence would reverse the decision?

The report presents the problem, evidence, hypotheses, decision and next step.

## Research

State which sources support which claims, their dates where relevant, and
whether primary evidence agrees. Separate observed facts, source claims and your
inference. Access sources using tools already available in the current agent;
this template does not provide browsing or silently invoke another provider.

Local reflection:
1. Which source changed the comparison?
2. Where are the evidence gaps or conflicts?
3. What remains worth researching?

The report presents the question, sources, comparison, limits and next step.

## Probe

Write the hypothesis and expected distinguishing observation **before** running.
Keep fixtures small and within the sample directory. The template preserves the
submitted notes and adds the actual effect result and logs after execution.
The initial reflection concerns experiment design; do not rewrite it as if an
unexpected outcome was predicted. Interpret the actual result in the current
agent conversation. If it warrants a review, record a separate retrospect with
`--parent-run-id` pointing to the probe.

Local reflection:
1. What would distinguish the expected outcomes?
2. Which limitation could invalidate the experiment?
3. How will the result change the next step?

The report presents the question, hypothesis, setup, expected observation and
next step. Its artifacts additionally include the raw structured `experiment.json` result (or `experiment-error.json`). A nonzero
exit is preserved as a failed experiment observation; it is not automatically
retried. See [runs.md](runs.md) for the sandbox command and fixture layout.

## Organize

Describe coherent work units, real dependencies and an order justified by those
dependencies. Identify shared write locations before proposing concurrency.
A list of roles is not a work decomposition. Keep source-changing work out of
Motivo experiment directories; execute it as normal authorized business work.

Local reflection:
1. Which dependency determines the order?
2. Where could shared changes conflict?
3. What can be removed from the plan?

The report presents goal, work units, dependencies, execution order and next step.

## Retrospect

Compare an explicit expectation with actual events and artifacts. Link the
relevant prior run, source diff, test output or experiment. Distinguish causes
supported by evidence from explanations that still require investigation.
Choose concrete practices to keep or change, with the next occasion to use them.
Do not create a reusable workflow or plugin just because a review exists.

Local reflection:
1. Which expectation was contradicted?
2. Which cause is supported rather than guessed?
3. What single practice should change next time?

The report presents expected/observed outcomes, evidence, causes, practices to
keep/change and next step. Artifacts include `retrospective.md`, an extracted
`handoff.md`, and `decision-history.json`. The history retains source headings
and does not invent decision timestamps or infer chronological/causal order.

## Handoff

Record current state, exact changed artifacts, checks actually performed, known
unknowns and a useful next action. A handoff restores facts, not an agent's hidden
memory or a filesystem snapshot. Mark incomplete work without presenting it as
finished or requiring the next agent to rediscover the uncertainty.

Local reflection:
1. What would a new agent otherwise miss?
2. Which result still needs verification?
3. What is the smallest useful next action?

The report presents goal, current state, changes, checks, open issues and next step.
