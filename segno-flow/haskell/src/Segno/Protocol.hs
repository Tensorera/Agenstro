{-# LANGUAGE DeriveGeneric #-}
{-# LANGUAGE OverloadedStrings #-}

-- | Stable JSON documents exchanged by Segno, Clef jobs, and open plugins.
-- The driver intentionally keeps payloads and backend revisions opaque.
module Segno.Protocol
  ( pluginApi,
    taskApi,
    installApi,
    invocationApi,
    resultApi,
    RequestId (..),
    PluginRequest (..),
    PluginFailure (..),
    pluginSuccess,
    pluginFailure,
    SourceManifest (..),
    StateManifest (..),
    TaskManifest (..),
    InstalledJob (..),
    StateSnapshot (..),
    StateCasResult (..),
    TriggerOccurrence (..),
    Invocation (..),
    Transition (..),
    Decision (..),
    TaskResult (..),
    PlannedOccurrence (..),
    PollResult (..),
    encodeCompactText,
    decodeJsonText,
  )
where

import Control.Monad (unless, when)
import Data.Foldable (traverse_)
import Data.Aeson
  ( FromJSON (parseJSON),
    Object,
    ToJSON (toJSON),
    Value (..),
    encode,
    object,
    withObject,
    (.:),
    (.:?),
    (.=),
  )
import qualified Data.Aeson.Key as Key
import qualified Data.Aeson.KeyMap as KeyMap
import Data.Aeson.Types (Parser)
import qualified Data.ByteString.Lazy as LazyByteString
import qualified Data.Set as Set
import Data.Text (Text)
import qualified Data.Text as Text
import Data.Text.Encoding (decodeUtf8, encodeUtf8)
import Data.Time (UTCTime)
import Data.Word (Word32)
import Data.Int (Int64)
import Clef.Plugin.Protocol (decodeStrictJSON)

pluginApi, taskApi, installApi, invocationApi, resultApi :: Text
pluginApi = "agenstro.plugin/v1"
taskApi = "agenstro.segno.task/v1"
installApi = "agenstro.segno.install/v1"
invocationApi = "agenstro.segno.invocation/v1"
resultApi = "agenstro.segno.result/v1"

data RequestId
  = TextRequestId Text
  | IntegerRequestId Int64
  deriving (Eq, Show)

instance FromJSON RequestId where
  parseJSON (String value) = pure (TextRequestId value)
  parseJSON value@(Number _) = IntegerRequestId <$> parseJSON value
  parseJSON _ = fail "plugin request id must be text or a signed integer"

instance ToJSON RequestId where
  toJSON requestId = case requestId of
    TextRequestId value -> toJSON value
    IntegerRequestId value -> toJSON value

data PluginRequest = PluginRequest
  { pluginRequestApi :: Text,
    pluginRequestId :: RequestId,
    pluginRequestMethod :: Text,
    pluginRequestParams :: Object
  }
  deriving (Eq, Show)

instance FromJSON PluginRequest where
  parseJSON = withObject "plugin request" $ \fields -> do
    rejectUnknown ["api", "id", "method", "params"] fields
    request <-
      PluginRequest
        <$> fields .: "api"
        <*> fields .: "id"
        <*> fields .: "method"
        <*> fields .: "params"
    unless (pluginRequestApi request == pluginApi) (fail "unsupported plugin api")
    when (Text.null (pluginRequestMethod request)) (fail "plugin method must not be empty")
    pure request

instance ToJSON PluginRequest where
  toJSON request =
    object
      [ "api" .= pluginRequestApi request,
        "id" .= pluginRequestId request,
        "method" .= pluginRequestMethod request,
        "params" .= pluginRequestParams request
      ]

data PluginFailure = PluginFailure
  { pluginFailureCode :: Text,
    pluginFailureMessage :: Text,
    pluginFailureDetails :: Maybe Value
  }
  deriving (Eq, Show)

instance ToJSON PluginFailure where
  toJSON failure =
    object $
      [ "code" .= pluginFailureCode failure,
        "message" .= pluginFailureMessage failure
      ]
        <> maybe [] (\details -> ["details" .= details]) (pluginFailureDetails failure)

pluginSuccess :: RequestId -> Value -> Value
pluginSuccess requestId value =
  object
    [ "type" .= ("result" :: Text),
      "id" .= requestId,
      "ok" .= True,
      "value" .= value
    ]

pluginFailure :: RequestId -> PluginFailure -> Value
pluginFailure requestId failure =
  object
    [ "type" .= ("result" :: Text),
      "id" .= requestId,
      "ok" .= False,
      "error" .= failure
    ]

data SourceManifest = SourceManifest
  { sourceId :: Text,
    sourcePlugin :: Text,
    sourceConfig :: Value
  }
  deriving (Eq, Show)

instance FromJSON SourceManifest where
  parseJSON = withObject "trigger source" $ \fields -> do
    rejectUnknown ["id", "plugin", "config"] fields
    SourceManifest <$> fields .: "id" <*> fields .: "plugin" <*> fields .: "config"

instance ToJSON SourceManifest where
  toJSON source =
    object
      [ "id" .= sourceId source,
        "plugin" .= sourcePlugin source,
        "config" .= sourceConfig source
      ]

data StateManifest = StateManifest
  { stateKey :: Text,
    stateSchemaVersion :: Word32,
    stateBackend :: Text,
    stateConflict :: Text,
    stateInitial :: Value
  }
  deriving (Eq, Show)

instance FromJSON StateManifest where
  parseJSON = withObject "business state manifest" $ \fields -> do
    rejectUnknown ["key", "schema_version", "backend", "conflict", "initial"] fields
    manifest <-
      StateManifest
        <$> fields .: "key"
        <*> fields .: "schema_version"
        <*> fields .: "backend"
        <*> fields .: "conflict"
        <*> fields .: "initial"
    when (stateSchemaVersion manifest == 0) (fail "schema_version must be non-zero")
    unless (stateConflict manifest == "compare-and-set") (fail "unsupported state conflict policy")
    pure manifest

instance ToJSON StateManifest where
  toJSON state =
    object
      [ "key" .= stateKey state,
        "schema_version" .= stateSchemaVersion state,
        "backend" .= stateBackend state,
        "conflict" .= stateConflict state,
        "initial" .= stateInitial state
      ]

data TaskManifest = TaskManifest
  { manifestTask :: Text,
    manifestSources :: [SourceManifest],
    manifestState :: StateManifest
  }
  deriving (Eq, Show)

instance FromJSON TaskManifest where
  parseJSON = withObject "Segno task manifest" $ \fields -> do
    rejectUnknown ["api", "task", "sources", "state"] fields
    api <- fields .: "api"
    unless (api == taskApi) (fail "unsupported task manifest api")
    manifest <- TaskManifest <$> fields .: "task" <*> fields .: "sources" <*> fields .: "state"
    when (Text.null (manifestTask manifest)) (fail "task must not be empty")
    when (null (manifestSources manifest)) (fail "task must declare at least one trigger source")
    let identities = fmap sourceId (manifestSources manifest)
    unless (Set.size (Set.fromList identities) == length identities) (fail "trigger source ids must be unique")
    pure manifest

instance ToJSON TaskManifest where
  toJSON manifest =
    object
      [ "api" .= taskApi,
        "task" .= manifestTask manifest,
        "sources" .= manifestSources manifest,
        "state" .= manifestState manifest
      ]

data InstalledJob = InstalledJob
  { installedScript :: FilePath,
    installedManifest :: TaskManifest
  }
  deriving (Eq, Show)

instance FromJSON InstalledJob where
  parseJSON = withObject "installed Segno job" $ \fields -> do
    rejectUnknown ["api", "script", "manifest"] fields
    api <- fields .: "api"
    unless (api == installApi) (fail "unsupported installed job api")
    InstalledJob <$> fields .: "script" <*> fields .: "manifest"

instance ToJSON InstalledJob where
  toJSON job =
    object
      [ "api" .= installApi,
        "script" .= installedScript job,
        "manifest" .= installedManifest job
      ]

data StateSnapshot = StateSnapshot
  { snapshotKey :: Text,
    snapshotRevision :: Maybe Text,
    snapshotSchemaVersion :: Word32,
    snapshotValue :: Value
  }
  deriving (Eq, Show)

instance FromJSON StateSnapshot where
  parseJSON = withObject "business state snapshot" $ \fields -> do
    rejectUnknown ["key", "revision", "schema_version", "value"] fields
    StateSnapshot
      <$> fields .: "key"
      <*> fields .:? "revision"
      <*> fields .: "schema_version"
      <*> fields .: "value"

instance ToJSON StateSnapshot where
  toJSON snapshot =
    object
      [ "key" .= snapshotKey snapshot,
        "revision" .= snapshotRevision snapshot,
        "schema_version" .= snapshotSchemaVersion snapshot,
        "value" .= snapshotValue snapshot
      ]

-- | Strict result returned by a business-state compare-and-set operation.
-- Keeping this in the shared protocol module prevents the driver and Clef
-- checkpoint path from assigning different meanings to the same wire value.
data StateCasResult
  = StateCasApplied Text
  | StateCasConflict (Maybe Text)
  deriving (Eq, Show)

instance FromJSON StateCasResult where
  parseJSON = withObject "business state compare-and-set result" $ \fields -> do
    rejectUnknown ["applied", "revision", "current_revision"] fields
    applied <- fields .: "applied"
    revision <- fields .:? "revision"
    current <- fields .:? "current_revision"
    case (applied, revision, current) of
      (True, Just nextRevision, Nothing) -> do
        validateOpaqueText "revision" 512 nextRevision
        pure (StateCasApplied nextRevision)
      (False, Nothing, actualRevision) -> do
        traverse_ (validateOpaqueText "current_revision" 512) actualRevision
        pure (StateCasConflict actualRevision)
      (True, Nothing, _) -> fail "applied response requires revision"
      (True, Just _, Just _) -> fail "applied response must not contain current_revision"
      (False, Just _, _) -> fail "conflict response must not contain revision"

instance ToJSON StateCasResult where
  toJSON result = case result of
    StateCasApplied revision -> object ["applied" .= True, "revision" .= revision]
    StateCasConflict current -> object ["applied" .= False, "current_revision" .= current]

data TriggerOccurrence = TriggerOccurrence
  { occurrenceTriggerId :: Text,
    occurrenceId :: Text,
    occurrenceLogicalTime :: UTCTime,
    occurrenceObservedTime :: UTCTime,
    occurrenceCursor :: Value,
    occurrenceIdempotencyKey :: Text,
    occurrencePayload :: Value
  }
  deriving (Eq, Show)

instance FromJSON TriggerOccurrence where
  parseJSON = withObject "trigger occurrence" $ \fields -> do
    rejectUnknown ["trigger_id", "occurrence_id", "logical_time", "observed_time", "cursor", "idempotency_key", "payload"] fields
    TriggerOccurrence
      <$> fields .: "trigger_id"
      <*> fields .: "occurrence_id"
      <*> fields .: "logical_time"
      <*> fields .: "observed_time"
      <*> fields .: "cursor"
      <*> fields .: "idempotency_key"
      <*> fields .: "payload"

instance ToJSON TriggerOccurrence where
  toJSON occurrence =
    object
      [ "trigger_id" .= occurrenceTriggerId occurrence,
        "occurrence_id" .= occurrenceId occurrence,
        "logical_time" .= occurrenceLogicalTime occurrence,
        "observed_time" .= occurrenceObservedTime occurrence,
        "cursor" .= occurrenceCursor occurrence,
        "idempotency_key" .= occurrenceIdempotencyKey occurrence,
        "payload" .= occurrencePayload occurrence
      ]

data Invocation = Invocation
  { invocationTask :: Text,
    invocationAttempt :: Word32,
    invocationFencingToken :: Text,
    invocationTrigger :: TriggerOccurrence,
    invocationState :: StateSnapshot
  }
  deriving (Eq, Show)

instance ToJSON Invocation where
  toJSON invocation =
    object
      [ "api" .= invocationApi,
        "task" .= invocationTask invocation,
        "attempt" .= invocationAttempt invocation,
        "fencing_token" .= invocationFencingToken invocation,
        "trigger" .= invocationTrigger invocation,
        "state" .= invocationState invocation
      ]

data Transition
  = KeepTransition (Maybe Text)
  | SetTransition (Maybe Text) Word32 Value
  deriving (Eq, Show)

instance FromJSON Transition where
  parseJSON = withObject "state transition" $ \fields -> do
    kind <- fields .: "kind"
    case (kind :: Text) of
      "keep" -> do
        rejectUnknown ["kind", "expected_revision"] fields
        KeepTransition <$> fields .:? "expected_revision"
      "set" -> do
        rejectUnknown ["kind", "expected_revision", "schema_version", "value"] fields
        SetTransition <$> fields .:? "expected_revision" <*> fields .: "schema_version" <*> fields .: "value"
      _ -> fail "unsupported state transition"

data Decision
  = IgnoreDecision
  | CompleteDecision Transition Value
  | RetryDecision Transition Integer Text
  | FailDecision PluginFailure
  deriving (Eq, Show)

instance FromJSON Decision where
  parseJSON = withObject "task decision" $ \fields -> do
    kind <- fields .: "kind"
    case (kind :: Text) of
      "ignore" -> rejectUnknown ["kind"] fields >> pure IgnoreDecision
      "complete" -> do
        rejectUnknown ["kind", "state", "output"] fields
        CompleteDecision <$> fields .: "state" <*> fields .: "output"
      "retry" -> do
        rejectUnknown ["kind", "state", "after_ms", "reason"] fields
        transition <- fields .: "state"
        encodedDelay <- fields .: "after_ms"
        delay <- parseDecimal encodedDelay
        RetryDecision transition delay <$> fields .: "reason"
      "fail" -> do
        rejectUnknown ["kind", "error"] fields
        failureFields <- fields .: "error"
        FailDecision <$> parseFailure failureFields
      _ -> fail "unsupported task decision"
    where
      parseDecimal encoded = case reads (Text.unpack encoded) of
        [(value, "")] | value >= (0 :: Integer) -> pure value
        _ -> fail "after_ms must be a non-negative decimal string"
      parseFailure = withObject "task failure" $ \failureFields -> do
        rejectUnknown ["code", "message", "details"] failureFields
        PluginFailure
          <$> failureFields .: "code"
          <*> failureFields .: "message"
          <*> failureFields .:? "details"

data TaskResult = TaskResult
  { resultTask :: Text,
    resultOccurrenceId :: Text,
    resultDecision :: Decision
  }
  deriving (Eq, Show)

instance FromJSON TaskResult where
  parseJSON = withObject "Segno task result" $ \fields -> do
    rejectUnknown ["api", "task", "occurrence_id", "decision"] fields
    api <- fields .: "api"
    unless (api == resultApi) (fail "unsupported task result api")
    TaskResult <$> fields .: "task" <*> fields .: "occurrence_id" <*> fields .: "decision"

data PlannedOccurrence = PlannedOccurrence
  { plannedLogicalTime :: UTCTime,
    plannedCursor :: Value,
    plannedIdempotencyKey :: Text,
    plannedPayload :: Value
  }
  deriving (Eq, Show)

instance FromJSON PlannedOccurrence where
  parseJSON = withObject "planned occurrence" $ \fields -> do
    rejectUnknown ["logical_time", "cursor", "idempotency_key", "payload"] fields
    logicalTime <- fields .: "logical_time"
    cursor <- fields .: "cursor"
    idempotencyKey <- fields .: "idempotency_key"
    validateOpaqueText "idempotency_key" 512 idempotencyKey
    PlannedOccurrence logicalTime cursor idempotencyKey <$> fields .: "payload"

instance ToJSON PlannedOccurrence where
  toJSON occurrence =
    object
      [ "logical_time" .= plannedLogicalTime occurrence,
        "cursor" .= plannedCursor occurrence,
        "idempotency_key" .= plannedIdempotencyKey occurrence,
        "payload" .= plannedPayload occurrence
      ]

data PollResult = PollResult
  { pollOccurrences :: [PlannedOccurrence],
    pollNextWake :: Maybe UTCTime
  }
  deriving (Eq, Show)

instance FromJSON PollResult where
  parseJSON = withObject "trigger poll result" $ \fields -> do
    rejectUnknown ["occurrences", "next_wake"] fields
    PollResult <$> fields .: "occurrences" <*> fields .:? "next_wake"

instance ToJSON PollResult where
  toJSON result =
    object
      [ "occurrences" .= pollOccurrences result,
        "next_wake" .= pollNextWake result
      ]

encodeCompactText :: ToJSON value => value -> Text
encodeCompactText = decodeUtf8 . LazyByteString.toStrict . encode

decodeJsonText :: FromJSON value => Text -> Either String value
decodeJsonText = decodeStrictJSON . encodeUtf8

rejectUnknown :: [Text] -> Object -> Parser ()
rejectUnknown allowed fields =
  case filter (`Set.notMember` accepted) (fmap Key.toText (KeyMap.keys fields)) of
    [] -> pure ()
    unknown : _ -> fail ("unknown field: " <> Text.unpack unknown)
  where
    accepted = Set.fromList allowed

validateOpaqueText :: Text -> Int -> Text -> Parser ()
validateOpaqueText label maximumLength value
  | Text.null value = fail (Text.unpack label <> " must not be empty")
  | Text.length value > maximumLength = fail (Text.unpack label <> " exceeds " <> show maximumLength <> " characters")
  | Text.any (\character -> character < ' ' || character == '\DEL') value =
      fail (Text.unpack label <> " contains a control character")
  | otherwise = pure ()
