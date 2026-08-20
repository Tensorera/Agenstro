{-# LANGUAGE OverloadedStrings #-}

-- | Composable rubrics, prompt guidance, judgement, and bounded refinement.
module Clef.Rubric
  ( -- * Rubrics
    Rubric,
    rubric,
    rubricNorms,

    -- * Prompt projection
    GuidanceBudget (..),
    Selector,
    mostViolatedFirst,
    bySeverity,
    rubricGuidance,
    rubricGuidanceWithHistory,

    -- * Judgement
    NormChecker (..),
    defaultNormChecker,
    Critique (..),
    judge,
    judgeWith,
    critiqueWorst,
    noViolationAbove,
    renderCritique,

    -- * Refinement
    RefinePolicy (..),
    defaultRefinePolicy,
    refine,
    refineWith,
  )
where

import Control.Exception (throwIO)
import Control.Monad (unless)
import Data.Aeson (ToJSON (toJSON), Value, encode, object, (.=))
import qualified Data.ByteString.Lazy as LazyByteString
import Data.Function (on)
import Data.List (find, sortBy, sortOn)
import qualified Data.Map.Strict as Map
import Data.Maybe (fromMaybe)
import qualified Data.Set as Set
import Data.Text (Text)
import qualified Data.Text as Text
import qualified Data.Text.Encoding as Text.Encoding
import Clef.Error (WorkflowError (RequirementFailed))
import Clef.Norm
  ( Check (..),
    CheckSpec (..),
    Locus (..),
    Norm (..),
    NormCheckRequest (..),
    NormCheckResult (..),
    NormId (..),
    NormRecord (..),
    Severity (..),
    Violation (..),
    normRecord,
    validateNormCheckResult,
  )
import Clef.Workflow
  ( Plugin,
    Workflow,
    call,
    jsonPlugin,
    liftIO,
  )

-- | Ordered, composable norms over one artefact type.  Order affects prompt
-- presentation only, never whether a checker is run.
newtype Rubric artifact = Rubric {rubricNorms :: [Norm artifact]}

instance Semigroup (Rubric artifact) where
  Rubric left <> Rubric right = Rubric (left <> right)

instance Monoid (Rubric artifact) where
  mempty = Rubric []

rubric :: [Norm artifact] -> Rubric artifact
rubric = Rubric

-- | The selector receives historical violation counts and durable norm
-- metadata.  Using 'NormRecord' avoids pretending an artefact projection or a
-- native Haskell function can be erased safely to @Norm ()@.
type Selector = [(NormId, Int)] -> [NormRecord] -> [NormId]

data GuidanceBudget = GuidanceBudget
  { budgetCharacters :: Int,
    budgetSelector :: Selector
  }

-- | Higher observed violation counts come first; ties retain authoring order.
mostViolatedFirst :: Selector
mostViolatedFirst history records = recordId <$> sortBy compareFrequency records
  where
    frequencies = Map.fromListWith (+) history
    compareFrequency left right =
      compare
        (Map.findWithDefault 0 (recordId right) frequencies)
        (Map.findWithDefault 0 (recordId left) frequencies)

-- | Higher severities come first; ties retain authoring order.
bySeverity :: Selector
bySeverity _ records = recordId <$> sortBy (flip compare `on` recordSeverity) records

rubricGuidance :: GuidanceBudget -> Rubric artifact -> Text
rubricGuidance = rubricGuidanceWithHistory []

-- | Render whole guidance entries without exceeding the character budget.
-- Oversized entries are skipped rather than cut into misleading fragments.
rubricGuidanceWithHistory :: [(NormId, Int)] -> GuidanceBudget -> Rubric artifact -> Text
rubricGuidanceWithHistory history budget selectedRubric
  | budgetCharacters budget <= 0 = ""
  | otherwise = Text.intercalate "\n" (reverse selectedLines)
  where
    records = normRecord <$> rubricNorms selectedRubric
    selectedIds = deduplicate (budgetSelector budget history records)
    selectedRecords = [record | identity <- selectedIds, Just record <- [find ((== identity) . recordId) records]]
    (_, selectedLines) = foldl' addLine (0, []) selectedRecords

    addLine (used, accepted) record =
      let rendered = renderGuidance record
          separatorLength = if null accepted then 0 else 1
          nextLength = used + separatorLength + Text.length rendered
       in if nextLength <= budgetCharacters budget
            then (nextLength, rendered : accepted)
            else (used, accepted)

renderGuidance :: NormRecord -> Text
renderGuidance record =
  "- ["
    <> unNormId (recordId record)
    <> "] "
    <> fromMaybe (recordStatement record) (recordGuidance record)

deduplicate :: Ord value => [value] -> [value]
deduplicate = reverse . snd . foldl' step (Set.empty, [])
  where
    step (seen, values) value
      | value `Set.member` seen = (seen, values)
      | otherwise = (Set.insert value seen, value : values)

-- | A configured ordinary plugin plus the artefact label placed in its check
-- request.  The plugin method is @check@; 'ExternalCheck' specs override both
-- plugin and method for their own batches.  Those override methods must still
-- implement the norm-v1 request/result wrapper; plugin-specific parameters
-- remain attached to each routed norm record.
data NormChecker = NormChecker
  { normCheckerPlugin :: Text,
    normCheckerArtifact :: Text
  }
  deriving (Eq, Show)

defaultNormChecker :: NormChecker
defaultNormChecker =
  NormChecker
    { normCheckerPlugin = "norm-check",
      normCheckerArtifact = "artifact"
    }

data Critique = Critique
  { critiqueViolations :: [Violation],
    critiqueChecked :: [NormId],
    critiqueUnchecked :: [NormId]
  }
  deriving (Eq, Show)

instance Semigroup Critique where
  left <> right =
    Critique
      { critiqueViolations = critiqueViolations left <> critiqueViolations right,
        critiqueChecked = critiqueChecked left <> critiqueChecked right,
        critiqueUnchecked = critiqueUnchecked left <> critiqueUnchecked right
      }

instance Monoid Critique where
  mempty = Critique [] [] []

instance ToJSON Critique where
  toJSON selectedCritique =
    object
      [ "violations" .= critiqueViolations selectedCritique,
        "checked" .= critiqueChecked selectedCritique,
        "unchecked" .= critiqueUnchecked selectedCritique
      ]

judge :: Rubric artifact -> artifact -> Workflow Critique
judge = judgeWith defaultNormChecker

-- | Apply all serialisable specs through ordinary configured plugins, grouped
-- by plugin, method, and projected source.  An 'ExternalCheck' destination is
-- therefore a norm-v1-aware adapter, not an arbitrary legacy plugin method.
-- Guidance-only norms are explicitly unchecked; native checks are checked
-- only after they return successfully.
judgeWith :: NormChecker -> Rubric artifact -> artifact -> Workflow Critique
judgeWith checker selectedRubric artifact = do
  ensureRubricIdentity (rubricNorms selectedRubric)
  batchCritiques <- mapM runBatch (pendingBatches checker (rubricNorms selectedRubric) artifact)
  nativeCritiques <- mapM (runNative artifact) (nativeNorms (rubricNorms selectedRubric))
  let guidanceCritique =
        mempty
          { critiqueUnchecked =
              [ normId selectedNorm
                | selectedNorm <- rubricNorms selectedRubric,
                  case normCheck selectedNorm of
                    Nothing -> True
                    Just _ -> False
              ]
          }
      merged = mconcat (guidanceCritique : batchCritiques <> nativeCritiques)
  pure (orderCritique (normId <$> rubricNorms selectedRubric) merged)
  where
    runBatch pending = do
      let request =
            NormCheckRequest
              { checkRequestArtifact = normCheckerArtifact checker,
                checkRequestSource = pendingSource pending,
                checkRequestNorms = pendingNorms pending
              }
          selectedPlugin = jsonPlugin (pendingPlugin pending) (pendingMethod pending) :: Plugin NormCheckRequest NormCheckResult
      result <- call selectedPlugin request
      validated <- either workflowFailure pure (validateNormCheckResult request result)
      pure
        Critique
          { critiqueViolations = checkResultViolations validated,
            critiqueChecked = checkResultChecked validated,
            critiqueUnchecked = checkResultUnchecked validated
          }

data PendingBatch = PendingBatch
  { pendingPlugin :: Text,
    pendingMethod :: Text,
    pendingSource :: Text,
    pendingNorms :: [NormRecord]
  }

pendingBatches :: NormChecker -> [Norm artifact] -> artifact -> [PendingBatch]
pendingBatches checker norms artifact = foldl' addPending [] serialisable
  where
    serialisable =
      [ (destination checker spec, projection artifact, (normRecord selectedNorm) {recordSpec = Just spec})
        | selectedNorm <- norms,
          Just (SpecCheck projection spec) <- [normCheck selectedNorm]
      ]

    addPending batches ((pluginName, method), source, record) =
      case break (sameBatch pluginName method source) batches of
        (before, existing : after) ->
          before <> (existing {pendingNorms = pendingNorms existing <> [record]} : after)
        _ -> batches <> [PendingBatch pluginName method source [record]]

sameBatch :: Text -> Text -> Text -> PendingBatch -> Bool
sameBatch pluginName method source pending =
  pendingPlugin pending == pluginName
    && pendingMethod pending == method
    && pendingSource pending == source

destination :: NormChecker -> CheckSpec -> (Text, Text)
destination _ (ExternalCheck pluginName method _) = (pluginName, method)
destination checker _ = (normCheckerPlugin checker, "check")

nativeNorms :: [Norm artifact] -> [Norm artifact]
nativeNorms = filter isNative
  where
    isNative selectedNorm = case normCheck selectedNorm of
      Just NativeCheck {} -> True
      _ -> False

runNative :: artifact -> Norm artifact -> Workflow Critique
runNative artifact selectedNorm = case normCheck selectedNorm of
  Just (NativeCheck check) -> do
    violations <- check artifact
    unless (all validViolation violations) . workflowFailure $
      "native check returned a violation with the wrong norm id or severity for "
        <> unNormId (normId selectedNorm)
    pure
      Critique
        { critiqueViolations = violations,
          critiqueChecked = [normId selectedNorm],
          critiqueUnchecked = []
        }
  _ -> pure mempty
  where
    validViolation selectedViolation =
      violationNorm selectedViolation == normId selectedNorm
        && violationSeverity selectedViolation == normSeverity selectedNorm

ensureRubricIdentity :: [Norm artifact] -> Workflow ()
ensureRubricIdentity norms =
  unless (Set.size identities == length norms) $
    workflowFailure "rubric contains duplicate norm ids"
  where
    identities = Set.fromList (normId <$> norms)

workflowFailure :: Text -> Workflow value
workflowFailure = liftIO . throwIO . RequirementFailed

orderCritique :: [NormId] -> Critique -> Critique
orderCritique order selectedCritique =
  selectedCritique
    { critiqueViolations = sortOn (position . violationNorm) (critiqueViolations selectedCritique),
      critiqueChecked = filter (`Set.member` checked) order,
      critiqueUnchecked = filter (`Set.member` unchecked) order
    }
  where
    positions = Map.fromList (zip order [(0 :: Int) ..])
    position identity = Map.findWithDefault (length order) identity positions
    checked = Set.fromList (critiqueChecked selectedCritique)
    unchecked = Set.fromList (critiqueUnchecked selectedCritique)

critiqueWorst :: Critique -> Maybe Severity
critiqueWorst selectedCritique = case violationSeverity <$> critiqueViolations selectedCritique of
  [] -> Nothing
  severities -> Just (maximum severities)

-- | True when no violation is strictly more severe than the supplied ceiling.
-- A 'Style' violation is therefore accepted by @noViolationAbove Style@.
noViolationAbove :: Severity -> Critique -> Bool
noViolationAbove severityCeiling = maybe True (<= severityCeiling) . critiqueWorst

renderCritique :: Critique -> Text
renderCritique selectedCritique = Text.intercalate "\n" (violationSection <> uncheckedSection)
  where
    violationSection = case critiqueViolations selectedCritique of
      [] -> ["No checked norm reported a violation."]
      violations -> "Checked norm violations:" : (renderViolation <$> violations)
    uncheckedSection = case critiqueUnchecked selectedCritique of
      [] -> []
      identities ->
        [ "Unchecked norms: "
            <> Text.intercalate ", " (unNormId <$> identities)
        ]

renderViolation :: Violation -> Text
renderViolation selectedViolation =
  "- ["
    <> Text.pack (show (violationSeverity selectedViolation))
    <> "] "
    <> unNormId (violationNorm selectedViolation)
    <> ": "
    <> violationMessage selectedViolation
    <> renderLocus
    <> renderEvidence
  where
    renderLocus = case violationLocus selectedViolation of
      Nothing -> ""
      Just locus ->
        " ("
          <> locusArtifact locus
          <> maybe "" ((":" <>) . Text.pack . show) (locusStartLine locus)
          <> maybe "" ((":" <>) . Text.pack . show) (locusStartColumn locus)
          <> ")"
    renderEvidence = case violationEvidence selectedViolation of
      Nothing -> ""
      Just evidence -> " Evidence: " <> renderJson evidence

renderJson :: Value -> Text
renderJson = Text.Encoding.decodeUtf8 . LazyByteString.toStrict . encode

data RefinePolicy = RefinePolicy
  { refineMaxRounds :: Int,
    refineAcceptable :: Critique -> Bool,
    -- | The generator should use this same budget when projecting its rubric.
    -- It remains explicit policy even though the generator receives the
    -- structured critique rather than pre-rendered prompt text.
    refineBudget :: GuidanceBudget
  }

defaultRefinePolicy :: RefinePolicy
defaultRefinePolicy =
  RefinePolicy
    { refineMaxRounds = 3,
      refineAcceptable = noViolationAbove Style,
      refineBudget = GuidanceBudget 4000 bySeverity
    }

refine :: RefinePolicy -> Rubric artifact -> (Maybe Critique -> Workflow artifact) -> Workflow (artifact, Critique)
refine = refineWith defaultNormChecker

-- | Generate, judge, and repair for at most 'refineMaxRounds' total candidate
-- rounds.  An unacceptable final candidate is returned with its critique so a
-- caller can make the terminal policy decision.  Workflow failures, including
-- @PluginOutcomeUnknown@, propagate naturally and are never treated as a
-- rejected candidate.  Each round must produce a fresh candidate; this
-- combinator cannot detect an in-place workspace mutation.  Rejected-round
-- runtime transitions are intentionally deferred until 'Workflow' exposes a
-- narrow record-emission combinator without revealing its runtime constructor.
refineWith :: NormChecker -> RefinePolicy -> Rubric artifact -> (Maybe Critique -> Workflow artifact) -> Workflow (artifact, Critique)
refineWith checker policy selectedRubric generate
  | refineMaxRounds policy <= 0 = workflowFailure "refineMaxRounds must be positive"
  | otherwise = loop 1 Nothing
  where
    loop roundNumber previous = do
      candidate <- generate previous
      critique <- judgeWith checker selectedRubric candidate
      if refineAcceptable policy critique || roundNumber >= refineMaxRounds policy
        then pure (candidate, critique)
        else loop (roundNumber + 1) (Just critique)
