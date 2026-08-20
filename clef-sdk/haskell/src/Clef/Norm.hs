{-# LANGUAGE DerivingStrategies #-}
{-# LANGUAGE LambdaCase #-}
{-# LANGUAGE OverloadedStrings #-}

-- | Typed domain norms and the compact @agenstro.norm/v1@ wire profile.
--
-- A norm keeps generation guidance and its optional check in one value.  The
-- serialisable check shapes are data; 'NativeCheck' is the deliberately
-- non-serialisable escape hatch.
module Clef.Norm
  ( -- * Identity and severity
    NormId (..),
    Severity (..),
    severityAtLeast,

    -- * Violations
    Bound (..),
    Locus (..),
    Violation (..),
    violation,

    -- * Serialisable checks
    CheckSpec (..),
    Projection,
    Check (..),

    -- * Norms
    Provenance (..),
    NormFixtures (..),
    Norm (..),
    norm,
    guidanceOnly,
    normIsCheckable,

    -- * Durable wire records
    normApi,
    NormRecord (..),
    normRecord,
    NormCatalogue (..),
    NormCheckRequest (..),
    NormCheckResult (..),
    validateNormCheckResult,
  )
where

import Control.Monad (unless, when)
import Data.Aeson
  ( FromJSON (parseJSON),
    Object,
    ToJSON (toJSON),
    Value (Object, String),
    object,
    withObject,
    withText,
    (.:),
    (.:?),
    (.!=),
    (.=),
  )
import qualified Data.Aeson.KeyMap as KeyMap
import Data.Aeson.Types (Parser)
import Data.List (find)
import Data.Maybe (catMaybes)
import Data.Scientific (Scientific)
import qualified Data.Set as Set
import Data.Text (Text)
import qualified Data.Text as Text
import Clef.Workflow (Workflow)

normApi :: Text
normApi = "agenstro.norm/v1"

-- | Stable, citable identity.  Dotted names are recommended by the wire
-- profile, for example @math.notation.upright-differential@.
newtype NormId = NormId {unNormId :: Text}
  deriving stock (Eq, Ord, Show)

instance ToJSON NormId where
  toJSON = String . unNormId

instance FromJSON NormId where
  parseJSON = withText "norm id" $ \identity -> do
    when (Text.null identity) $ fail "norm id must not be empty"
    pure (NormId identity)

-- | Ordered from least to most consequential.  Blocking is kept distinct
-- from correctness even though both project to SARIF's @error@ level.
data Severity
  = Preference
  | Style
  | Correctness
  | Blocking
  deriving stock (Eq, Ord, Show, Enum, Bounded)

instance ToJSON Severity where
  toJSON = String . severityName

instance FromJSON Severity where
  parseJSON = withText "norm severity" $ \case
    "Preference" -> pure Preference
    "Style" -> pure Style
    "Correctness" -> pure Correctness
    "Blocking" -> pure Blocking
    other -> fail $ "unknown norm severity: " <> Text.unpack other

severityAtLeast :: Severity -> Severity -> Bool
severityAtLeast floorLevel level = level >= floorLevel

severityName :: Severity -> Text
severityName = \case
  Preference -> "Preference"
  Style -> "Style"
  Correctness -> "Correctness"
  Blocking -> "Blocking"

-- | Inclusive numeric bounds.  Scientific keeps catalogue thresholds exact
-- at the JSON boundary instead of silently rounding through 'Double'.
data Bound = Bound
  { boundMinimum :: Maybe Scientific,
    boundMaximum :: Maybe Scientific
  }
  deriving stock (Eq, Show)

instance ToJSON Bound where
  toJSON selectedBound =
    object
      [ "boundMinimum" .= boundMinimum selectedBound,
        "boundMaximum" .= boundMaximum selectedBound
      ]

instance FromJSON Bound where
  parseJSON = withObject "norm bound" $ \objectValue -> do
    minimumValue <- objectValue .:? "boundMinimum"
    maximumValue <- objectValue .:? "boundMaximum"
    case (minimumValue, maximumValue) of
      (Just low, Just high) | low > high -> fail "boundMinimum must not exceed boundMaximum"
      _ -> pure (Bound minimumValue maximumValue)

-- | One-based, inclusive coordinates, matching the Agenstro SARIF profile.
-- This type must not be reused directly for zero-based LSP ranges.
data Locus = Locus
  { locusArtifact :: Text,
    locusStartLine :: Maybe Int,
    locusStartColumn :: Maybe Int,
    locusEndLine :: Maybe Int,
    locusEndColumn :: Maybe Int,
    locusSnippet :: Maybe Text
  }
  deriving stock (Eq, Show)

instance ToJSON Locus where
  toJSON selectedLocus =
    object . catMaybes $
      [ Just ("artifact" .= locusArtifact selectedLocus),
        ("startLine" .=) <$> locusStartLine selectedLocus,
        ("startColumn" .=) <$> locusStartColumn selectedLocus,
        ("endLine" .=) <$> locusEndLine selectedLocus,
        ("endColumn" .=) <$> locusEndColumn selectedLocus,
        ("snippet" .=) <$> locusSnippet selectedLocus
      ]

instance FromJSON Locus where
  parseJSON = withObject "norm locus" $ \objectValue -> do
    artifact <- objectValue .: "artifact"
    startLine <- objectValue .:? "startLine"
    startColumn <- objectValue .:? "startColumn"
    endLine <- objectValue .:? "endLine"
    endColumn <- objectValue .:? "endColumn"
    snippet <- objectValue .:? "snippet"
    mapM_ positiveCoordinate [startLine, startColumn, endLine, endColumn]
    case (startLine, startColumn, endLine, endColumn) of
      (Just firstLine, Just firstColumn, Just lastLine, Just lastColumn) ->
        when ((lastLine, lastColumn) < (firstLine, firstColumn)) $
          fail "locus end must not precede its start"
      _ -> pure ()
    pure
      Locus
        { locusArtifact = artifact,
          locusStartLine = startLine,
          locusStartColumn = startColumn,
          locusEndLine = endLine,
          locusEndColumn = endColumn,
          locusSnippet = snippet
        }
    where
      positiveCoordinate Nothing = pure ()
      positiveCoordinate (Just coordinate) =
        unless (coordinate > 0) $ fail "locus coordinates must be positive and one-based"

data Violation = Violation
  { violationNorm :: NormId,
    violationSeverity :: Severity,
    violationMessage :: Text,
    violationLocus :: Maybe Locus,
    violationEvidence :: Maybe Value
  }
  deriving stock (Eq, Show)

instance ToJSON Violation where
  toJSON selectedViolation =
    object . catMaybes $
      [ Just ("norm" .= violationNorm selectedViolation),
        Just ("severity" .= violationSeverity selectedViolation),
        Just ("message" .= violationMessage selectedViolation),
        ("locus" .=) <$> violationLocus selectedViolation,
        ("evidence" .=) <$> violationEvidence selectedViolation
      ]

instance FromJSON Violation where
  parseJSON = withObject "norm violation" $ \objectValue ->
    Violation
      <$> objectValue .: "norm"
      <*> objectValue .: "severity"
      <*> objectValue .: "message"
      <*> objectValue .:? "locus"
      <*> objectValue .:? "evidence"

violation :: NormId -> Severity -> Text -> Violation
violation identity severity message =
  Violation
    { violationNorm = identity,
      violationSeverity = severity,
      violationMessage = message,
      violationLocus = Nothing,
      violationEvidence = Nothing
    }

-- | Closed authored check shapes plus a read-only forward-compatibility case.
-- 'UnknownCheckSpec' is produced only when a newer @kind@ is decoded; judges
-- send it to the configured checker, which must report it as unchecked.
data CheckSpec
  = Existence {specPattern :: Text, specIgnoreCase :: Bool}
  | Absence {specPattern :: Text, specIgnoreCase :: Bool}
  | Occurrence {specPattern :: Text, specBound :: Bound}
  | Consistency {specGroups :: [[Text]]}
  | Sequence {specOrdered :: [Text]}
  | Metric {specMetric :: Text, specBound :: Bound}
  -- | Route this norm to another plugin destination.  The destination method
  -- still implements the standard norm-v1 wrapper: it receives a
  -- 'NormCheckRequest', finds its plugin-specific parameters in this record's
  -- @specParams@, and returns a 'NormCheckResult'.  It is not a raw adapter to
  -- an arbitrary pre-existing method.
  | ExternalCheck {specPlugin :: Text, specMethod :: Text, specParams :: Maybe Value}
  | UnknownCheckSpec {specUnknownKind :: Text, specUnknownFields :: Object}
  deriving stock (Eq, Show)

instance ToJSON CheckSpec where
  toJSON = \case
    Existence patternText ignoreCase ->
      object
        [ "kind" .= ("Existence" :: Text),
          "specPattern" .= patternText,
          "specIgnoreCase" .= ignoreCase
        ]
    Absence patternText ignoreCase ->
      object
        [ "kind" .= ("Absence" :: Text),
          "specPattern" .= patternText,
          "specIgnoreCase" .= ignoreCase
        ]
    Occurrence patternText selectedBound ->
      object
        [ "kind" .= ("Occurrence" :: Text),
          "specPattern" .= patternText,
          "specBound" .= selectedBound
        ]
    Consistency groups ->
      object
        [ "kind" .= ("Consistency" :: Text),
          "specGroups" .= groups
        ]
    Sequence ordered ->
      object
        [ "kind" .= ("Sequence" :: Text),
          "specOrdered" .= ordered
        ]
    Metric metricName selectedBound ->
      object
        [ "kind" .= ("Metric" :: Text),
          "specMetric" .= metricName,
          "specBound" .= selectedBound
        ]
    ExternalCheck pluginName method params ->
      object . catMaybes $
        [ Just ("kind" .= ("ExternalCheck" :: Text)),
          Just ("specPlugin" .= pluginName),
          Just ("specMethod" .= method),
          ("specParams" .=) <$> params
        ]
    UnknownCheckSpec kind fields ->
      Object (KeyMap.insert "kind" (String kind) (KeyMap.delete "kind" fields))

instance FromJSON CheckSpec where
  parseJSON = withObject "norm check specification" $ \objectValue -> do
    kind <- objectValue .: "kind"
    case (kind :: Text) of
      "Existence" ->
        Existence
          <$> objectValue .: "specPattern"
          <*> objectValue .:? "specIgnoreCase" .!= False
      "Absence" ->
        Absence
          <$> objectValue .: "specPattern"
          <*> objectValue .:? "specIgnoreCase" .!= False
      "Occurrence" -> Occurrence <$> objectValue .: "specPattern" <*> objectValue .: "specBound"
      "Consistency" -> Consistency <$> objectValue .: "specGroups"
      "Sequence" -> Sequence <$> objectValue .: "specOrdered"
      "Metric" -> Metric <$> objectValue .: "specMetric" <*> objectValue .: "specBound"
      "ExternalCheck" -> do
        pluginName <- objectValue .: "specPlugin"
        method <- objectValue .: "specMethod"
        when (Text.null pluginName || Text.null method) $
          fail "ExternalCheck plugin and method must not be empty"
        ExternalCheck pluginName method <$> objectValue .:? "specParams"
      other -> pure (UnknownCheckSpec other (KeyMap.delete "kind" objectValue))

-- | How an artefact is projected into source text for a serialisable checker.
type Projection artifact = artifact -> Text

data Check artifact
  = SpecCheck (Projection artifact) CheckSpec
  | NativeCheck (artifact -> Workflow [Violation])

-- | Provenance is open on the wire.  Unknown kinds retain all of their fields
-- so a read/write cycle does not destroy evidence introduced by a newer tool.
data Provenance
  = Authored {provenanceAuthor :: Text}
  | MinedFromCorpus
      { provenanceCorpus :: Text,
        provenanceSupport :: Int,
        provenanceTotal :: Int
      }
  | MinedFromEdits {provenanceObservations :: Int}
  | OtherProvenance
      { provenanceKind :: Text,
        provenanceFields :: Object
      }
  deriving stock (Eq, Show)

instance ToJSON Provenance where
  toJSON = \case
    Authored author -> object ["kind" .= ("Authored" :: Text), "author" .= author]
    MinedFromCorpus corpus support total ->
      object
        [ "kind" .= ("MinedFromCorpus" :: Text),
          "corpus" .= corpus,
          "support" .= support,
          "total" .= total
        ]
    MinedFromEdits observations ->
      object ["kind" .= ("MinedFromEdits" :: Text), "observations" .= observations]
    OtherProvenance kind fields ->
      Object (KeyMap.insert "kind" (String kind) (KeyMap.delete "kind" fields))

instance FromJSON Provenance where
  parseJSON = withObject "norm provenance" $ \objectValue -> do
    kind <- objectValue .: "kind"
    case (kind :: Text) of
      "Authored" -> Authored <$> objectValue .: "author"
      "MinedFromCorpus" ->
        MinedFromCorpus
          <$> objectValue .: "corpus"
          <*> objectValue .: "support"
          <*> objectValue .: "total"
      "MinedFromEdits" -> MinedFromEdits <$> objectValue .: "observations"
      other -> pure (OtherProvenance other (KeyMap.delete "kind" objectValue))

data NormFixtures = NormFixtures
  { fixtureMustFlag :: [Text],
    fixtureMustNotFlag :: [Text]
  }
  deriving stock (Eq, Show)

instance ToJSON NormFixtures where
  toJSON fixtures =
    object
      [ "mustFlag" .= fixtureMustFlag fixtures,
        "mustNotFlag" .= fixtureMustNotFlag fixtures
      ]

instance FromJSON NormFixtures where
  parseJSON = withObject "norm fixtures" $ \objectValue ->
    NormFixtures
      <$> objectValue .:? "mustFlag" .!= []
      <*> objectValue .:? "mustNotFlag" .!= []

data Norm artifact = Norm
  { normId :: NormId,
    normStatement :: Text,
    normSeverity :: Severity,
    normGuidance :: Maybe Text,
    normCheck :: Maybe (Check artifact),
    normProvenance :: Provenance,
    normSupersedes :: [NormId],
    normFixtures :: Maybe NormFixtures
  }

norm :: NormId -> Text -> Severity -> Norm artifact
norm identity statement severity =
  Norm
    { normId = identity,
      normStatement = statement,
      normSeverity = severity,
      normGuidance = Nothing,
      normCheck = Nothing,
      normProvenance = Authored "unknown",
      normSupersedes = [],
      normFixtures = Nothing
    }

guidanceOnly :: NormId -> Text -> Severity -> Text -> Norm artifact
guidanceOnly identity statement severity guidance =
  (norm identity statement severity) {normGuidance = Just guidance}

normIsCheckable :: Norm artifact -> Bool
normIsCheckable = maybe False (const True) . normCheck

data NormRecord = NormRecord
  { recordId :: NormId,
    recordStatement :: Text,
    recordSeverity :: Severity,
    recordGuidance :: Maybe Text,
    recordSpec :: Maybe CheckSpec,
    recordProvenance :: Provenance,
    recordSupersedes :: [NormId],
    recordFixtures :: Maybe NormFixtures
  }
  deriving stock (Eq, Show)

instance ToJSON NormRecord where
  toJSON record =
    object . catMaybes $
      [ Just ("id" .= recordId record),
        Just ("statement" .= recordStatement record),
        Just ("severity" .= recordSeverity record),
        ("guidance" .=) <$> recordGuidance record,
        ("spec" .=) <$> recordSpec record,
        Just ("provenance" .= recordProvenance record),
        Just ("supersedes" .= recordSupersedes record),
        ("fixtures" .=) <$> recordFixtures record
      ]

instance FromJSON NormRecord where
  parseJSON = withObject "norm record" $ \objectValue ->
    NormRecord
      <$> objectValue .: "id"
      <*> objectValue .: "statement"
      <*> objectValue .: "severity"
      <*> objectValue .:? "guidance"
      <*> objectValue .:? "spec"
      <*> objectValue .: "provenance"
      <*> objectValue .:? "supersedes" .!= []
      <*> objectValue .:? "fixtures"

normRecord :: Norm artifact -> NormRecord
normRecord selectedNorm =
  NormRecord
    { recordId = normId selectedNorm,
      recordStatement = normStatement selectedNorm,
      recordSeverity = normSeverity selectedNorm,
      recordGuidance = normGuidance selectedNorm,
      recordSpec = case normCheck selectedNorm of
        Just (SpecCheck _ spec) -> Just spec
        _ -> Nothing,
      recordProvenance = normProvenance selectedNorm,
      recordSupersedes = normSupersedes selectedNorm,
      recordFixtures = normFixtures selectedNorm
    }

data NormCatalogue = NormCatalogue
  { catalogueName :: Text,
    catalogueNorms :: [NormRecord]
  }
  deriving stock (Eq, Show)

instance ToJSON NormCatalogue where
  toJSON catalogue =
    object
      [ "api" .= normApi,
        "catalogue" .= catalogueName catalogue,
        "norms" .= catalogueNorms catalogue
      ]

instance FromJSON NormCatalogue where
  parseJSON = withObject "norm catalogue" $ \objectValue -> do
    api <- objectValue .: "api"
    unless (api == normApi) $ fail "api must be exactly agenstro.norm/v1"
    catalogue <- objectValue .: "catalogue"
    when (Text.null catalogue) $ fail "catalogue must not be empty"
    records <- objectValue .: "norms"
    ensureUniqueIds "catalogue" (recordId <$> records)
    pure (NormCatalogue catalogue records)

-- | Params supplied to an ordinary @agenstro.plugin/v1@ @check@ method.
data NormCheckRequest = NormCheckRequest
  { checkRequestArtifact :: Text,
    checkRequestSource :: Text,
    checkRequestNorms :: [NormRecord]
  }
  deriving stock (Eq, Show)

instance ToJSON NormCheckRequest where
  toJSON request =
    object
      [ "artifact" .= checkRequestArtifact request,
        "source" .= checkRequestSource request,
        "norms" .= checkRequestNorms request
      ]

instance FromJSON NormCheckRequest where
  parseJSON = withObject "norm check request" $ \objectValue -> do
    artifact <- objectValue .: "artifact"
    source <- objectValue .: "source"
    records <- objectValue .: "norms"
    ensureUniqueIds "norm check request" (recordId <$> records)
    pure (NormCheckRequest artifact source records)

data NormCheckResult = NormCheckResult
  { checkResultArtifact :: Text,
    checkResultViolations :: [Violation],
    checkResultChecked :: [NormId],
    checkResultUnchecked :: [NormId]
  }
  deriving stock (Eq, Show)

instance ToJSON NormCheckResult where
  toJSON result =
    object
      [ "api" .= normApi,
        "artifact" .= checkResultArtifact result,
        "violations" .= checkResultViolations result,
        "checked" .= checkResultChecked result,
        "unchecked" .= checkResultUnchecked result
      ]

instance FromJSON NormCheckResult where
  parseJSON = withObject "norm check result" $ \objectValue -> do
    api <- objectValue .: "api"
    unless (api == normApi) $ fail "api must be exactly agenstro.norm/v1"
    result <-
      NormCheckResult
        <$> objectValue .: "artifact"
        <*> objectValue .: "violations"
        <*> objectValue .: "checked"
        <*> objectValue .: "unchecked"
    either (fail . Text.unpack) pure (validateResultShape result)

-- | Validate a terminal checker value against the exact batch that was sent.
-- This is the honesty boundary: every requested norm must be classified once,
-- every violation must cite a checked norm, and severities may not drift from
-- the catalogue.
validateNormCheckResult :: NormCheckRequest -> NormCheckResult -> Either Text NormCheckResult
validateNormCheckResult request result = do
  _ <- validateResultShape result
  unlessEither
    (checkResultArtifact result == checkRequestArtifact request)
    "checker result artifact does not match the request"
  let records = checkRequestNorms request
      expectedIds = recordId <$> records
      classifiedIds = checkResultChecked result <> checkResultUnchecked result
  ensureUniqueIdsEither "norm check request" expectedIds
  unlessEither
    (Set.fromList expectedIds == Set.fromList classifiedIds)
    "checker result must classify every requested norm exactly once"
  mapM_ (validateViolation records) (checkResultViolations result)
  pure result

validateResultShape :: NormCheckResult -> Either Text NormCheckResult
validateResultShape result = do
  ensureUniqueIdsEither "checked" (checkResultChecked result)
  ensureUniqueIdsEither "unchecked" (checkResultUnchecked result)
  let checked = Set.fromList (checkResultChecked result)
      unchecked = Set.fromList (checkResultUnchecked result)
  unlessEither (Set.null (Set.intersection checked unchecked)) "checked and unchecked must be disjoint"
  mapM_
    ( \selectedViolation ->
        unlessEither
          (violationNorm selectedViolation `Set.member` checked)
          "every violation must cite a checked norm"
    )
    (checkResultViolations result)
  pure result

validateViolation :: [NormRecord] -> Violation -> Either Text ()
validateViolation records selectedViolation =
  case find ((== violationNorm selectedViolation) . recordId) records of
    Nothing -> Left "checker returned a violation for a norm outside the request"
    Just record ->
      unlessEither
        (recordSeverity record == violationSeverity selectedViolation)
        ("checker changed the configured severity for " <> unNormId (recordId record))

ensureUniqueIds :: String -> [NormId] -> Parser ()
ensureUniqueIds label identities =
  either (fail . Text.unpack) pure (ensureUniqueIdsEither (Text.pack label) identities)

ensureUniqueIdsEither :: Text -> [NormId] -> Either Text ()
ensureUniqueIdsEither label identities =
  unlessEither
    (Set.size (Set.fromList identities) == length identities)
    (label <> " contains duplicate norm ids")

unlessEither :: Bool -> Text -> Either Text ()
unlessEither condition message = unless condition (Left message)
