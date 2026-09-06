{-# LANGUAGE OverloadedStrings #-}

-- | Small, independent methods. The calling coding agent supplies the thinking.
module Motivo.Method
  ( Method (..), methodSlug, methodTitle, methodPurpose, methodSections,
    reflectionQuestions, allMethods
  ) where

import Data.Text (Text)

data Method = Clarify | Investigate | Analyze | Research | Probe | Organize | Retrospect | Handoff
  deriving (Eq, Show, Enum, Bounded)

allMethods :: [Method]
allMethods = [minBound .. maxBound]

methodSlug :: Method -> Text
methodSlug method = case method of
  Clarify -> "clarify"
  Investigate -> "investigate"
  Analyze -> "analyze"
  Research -> "research"
  Probe -> "probe"
  Organize -> "organize"
  Retrospect -> "retrospect"
  Handoff -> "handoff"

methodTitle :: Method -> Text
methodTitle method = case method of
  Clarify -> "Clarify · 澄清"
  Investigate -> "Investigate · 调查"
  Analyze -> "Analyze · 分析"
  Research -> "Research · 研究"
  Probe -> "Probe · 最小实验"
  Organize -> "Organize · 组织"
  Retrospect -> "Retrospect · 复盘"
  Handoff -> "Handoff · 交接"

methodPurpose :: Method -> Text
methodPurpose method = case method of
  Clarify -> "Make the intended outcome, constraints, and unresolved choices explicit."
  Investigate -> "Connect a concrete question to inspected sources and observations."
  Analyze -> "Compare explanations and separate evidence from interpretation."
  Research -> "Compare sourced alternatives against the question being decided."
  Probe -> "Run one bounded local experiment to distinguish a specific hypothesis."
  Organize -> "Arrange coherent work units and their actual dependencies."
  Retrospect -> "Compare expectations with observations and choose what to retain or change."
  Handoff -> "Preserve the state and evidence needed for the next person or agent."

-- | These are input headings, not a sequence of agent calls or quality gates.
methodSections :: Method -> [Text]
methodSections method = case method of
  Clarify -> ["Goal", "Constraints", "Success signals", "Unknowns", "Next step"]
  Investigate -> ["Question", "Sources", "Findings", "Unknowns", "Next step"]
  Analyze -> ["Problem", "Evidence", "Hypotheses", "Decision", "Next step"]
  Research -> ["Question", "Sources", "Comparison", "Limits", "Next step"]
  Probe -> ["Question", "Hypothesis", "Setup", "Expected observation", "Next step"]
  Organize -> ["Goal", "Work units", "Dependencies", "Execution order", "Next step"]
  Retrospect -> ["Expected", "Observed", "Evidence", "Causes", "Keep", "Change", "Next step"]
  Handoff -> ["Goal", "Current state", "Changes", "Checks", "Open issues", "Next step"]

-- | Every method has its own three local reflection prompts.
reflectionQuestions :: Method -> [Text]
reflectionQuestions method = case method of
  Clarify -> ["Which ambiguity was resolved?", "Which assumption still needs confirmation?", "What would change the goal?"]
  Investigate -> ["Which observation changed the picture?", "What remains unobserved?", "Which investigation is no longer necessary?"]
  Analyze -> ["Which explanation gained or lost support?", "Where might the reasoning be wrong?", "What evidence would reverse the decision?"]
  Research -> ["Which source changed the comparison?", "Where are the evidence gaps or conflicts?", "What remains worth researching?"]
  Probe -> ["What would distinguish the expected outcomes?", "Which limitation could invalidate the experiment?", "How will the result change the next step?"]
  Organize -> ["Which dependency determines the order?", "Where could shared changes conflict?", "What can be removed from the plan?"]
  Retrospect -> ["Which expectation was contradicted?", "Which cause is supported rather than guessed?", "What single practice should change next time?"]
  Handoff -> ["What would a new agent otherwise miss?", "Which result still needs verification?", "What is the smallest useful next action?"]
