{-# LANGUAGE GADTs #-}
{-# LANGUAGE LambdaCase #-}
{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE ScopedTypeVariables #-}

-- | The small typed boundary between a persistent Segno driver and one Clef
-- workflow occurrence.  Scheduling, leases, cursors, and lifecycle state stay
-- in Segno; this module only describes sources, decodes one occurrence, and
-- returns one explicit state transition.
module Clef.Segno
  ( Trigger,
    TriggerId (..),
    triggerSource,
    mapTrigger,
    filterTrigger,
    mergeTrigger,
    gate,
    Occurrence (..),
    State,
    StateKey (..),
    SchemaVersion (..),
    StateRevision (..),
    StateBackend (..),
    ConflictPolicy (..),
    defaultStateBackend,
    state,
    stateWithBackend,
    stateWithMigration,
    StateHandle,
    currentState,
    stateRevision,
    CheckpointId (..),
    StateConflict (..),
    checkpoint,
    StateTransition (..),
    RetrySpec (..),
    TaskFailure (..),
    Decision (..),
    PersistentTask,
    persistentTask,
    taskManifest,
    SegnoError (..),
    runPersistentTask,
  )
where

import Control.Exception
  ( Exception,
    IOException,
    catch,
    mask_,
    onException,
    throwIO,
  )
import Control.Monad (unless, when)
import Data.Aeson
  ( FromJSON (parseJSON),
    Object,
    ToJSON (toJSON),
    Value,
    encode,
    object,
    withObject,
    (.:),
    (.:?),
    (.=),
  )
import qualified Data.Aeson.Key as Key
import qualified Data.Aeson.KeyMap as KeyMap
import Data.Aeson.Types (Parser, parseEither)
import qualified Data.ByteString as ByteString
import qualified Data.ByteString.Lazy as LazyByteString
import Data.Foldable (traverse_)
import qualified Data.Set as Set
import Data.Text (Text)
import qualified Data.Text as Text
import Data.Time (UTCTime)
import Data.Word (Word32, Word64)
import Clef.Plugin.Protocol (decodeStrictJSON)
import Clef.Workflow
  ( Plugin,
    Workflow,
    call,
    jsonPlugin,
    runTactus,
  )
import System.Directory (removeFile, renameFile)
import System.Environment (lookupEnv)
import System.FilePath (takeDirectory)
import System.IO (hClose, hFlush, openBinaryTempFile)

manifestApi, invocationApi, resultApi :: Text
manifestApi = "agenstro.segno.task/v1"
invocationApi = "agenstro.segno.invocation/v1"
resultApi = "agenstro.segno.result/v1"

maximumDocumentBytes :: Int
maximumDocumentBytes = 4 * 1024 * 1024

newtype TriggerId = TriggerId {unTriggerId :: Text}
  deriving (Eq, Ord, Show)

newtype StateKey = StateKey {unStateKey :: Text}
  deriving (Eq, Ord, Show)

newtype SchemaVersion = SchemaVersion {unSchemaVersion :: Word32}
  deriving (Eq, Ord, Show)

-- | Backend revisions are deliberately opaque: SQLite integers, PostgreSQL
-- versions, and Redis tokens can all share the same DSL.
newtype StateRevision = StateRevision {unStateRevision :: Text}
  deriving (Eq, Ord, Show)

newtype StateBackend = StateBackend {stateBackendPlugin :: Text}
  deriving (Eq, Ord, Show)

-- | Version one has one honest conflict semantic.  Retry policy belongs to
-- the persistent task/driver, not inside an implicit state write.
data ConflictPolicy = CompareAndSet
  deriving (Eq, Ord, Show)

defaultStateBackend :: StateBackend
defaultStateBackend = StateBackend "segno.state"

data TriggerSource = TriggerSource
  { sourceIdentity :: TriggerId,
    sourcePlugin :: Text,
    sourceConfiguration :: Value
  }

-- | The state type is an index because a state-aware gate cannot be made
-- statically honest with only @Trigger event@.
data Trigger stateValue event where
  SourceTrigger :: FromJSON event => TriggerSource -> Trigger stateValue event
  MapTrigger :: (left -> right) -> Trigger stateValue left -> Trigger stateValue right
  FilterTrigger :: (event -> Bool) -> Trigger stateValue event -> Trigger stateValue event
  MergeTrigger :: Trigger stateValue event -> Trigger stateValue event -> Trigger stateValue event
  GateTrigger :: (stateValue -> event -> Bool) -> Trigger stateValue event -> Trigger stateValue event

triggerSource :: (ToJSON configuration, FromJSON event) => TriggerId -> Text -> configuration -> Trigger stateValue event
triggerSource identity pluginName configuration =
  SourceTrigger
    TriggerSource
      { sourceIdentity = identity,
        sourcePlugin = pluginName,
        sourceConfiguration = toJSON configuration
      }

mapTrigger :: (left -> right) -> Trigger stateValue left -> Trigger stateValue right
mapTrigger = MapTrigger

filterTrigger :: (event -> Bool) -> Trigger stateValue event -> Trigger stateValue event
filterTrigger = FilterTrigger

mergeTrigger :: Trigger stateValue event -> Trigger stateValue event -> Trigger stateValue event
mergeTrigger = MergeTrigger

gate :: (stateValue -> event -> Bool) -> Trigger stateValue event -> Trigger stateValue event
gate = GateTrigger

data Occurrence event = Occurrence
  { occurrenceTriggerId :: TriggerId,
    occurrenceId :: Text,
    occurrenceLogicalTime :: UTCTime,
    occurrenceObservedTime :: UTCTime,
    occurrenceCursor :: Value,
    occurrenceIdempotencyKey :: Text,
    occurrenceAttempt :: Word32,
    occurrencePayload :: event
  }
  deriving (Eq, Show)

-- | A typed, versioned business-state declaration.  Lifecycle state is never
-- represented by this value and therefore cannot be corrupted by user code.
data State stateValue = State
  { internalStateKey :: StateKey,
    internalSchemaVersion :: SchemaVersion,
    internalStateBackend :: StateBackend,
    internalConflictPolicy :: ConflictPolicy,
    internalInitialState :: stateValue,
    internalEncodeState :: stateValue -> Value,
    internalDecodeState :: SchemaVersion -> Value -> Either Text stateValue
  }

state :: (ToJSON stateValue, FromJSON stateValue) => StateKey -> SchemaVersion -> stateValue -> State stateValue
state = stateWithBackend defaultStateBackend

stateWithBackend :: (ToJSON stateValue, FromJSON stateValue) => StateBackend -> StateKey -> SchemaVersion -> stateValue -> State stateValue
stateWithBackend backend key version initialValue =
  stateWithMigration backend key version initialValue $ \storedVersion value ->
    if storedVersion == version
      then decodeValue value
      else Left "stored state schema requires an explicit migration"

stateWithMigration :: (ToJSON stateValue, FromJSON stateValue) => StateBackend -> StateKey -> SchemaVersion -> stateValue -> (SchemaVersion -> Value -> Either Text stateValue) -> State stateValue
stateWithMigration backend key version initialValue migrate =
  State
    { internalStateKey = key,
      internalSchemaVersion = version,
      internalStateBackend = backend,
      internalConflictPolicy = CompareAndSet,
      internalInitialState = initialValue,
      internalEncodeState = toJSON,
      internalDecodeState = \storedVersion value ->
        if storedVersion == version then decodeValue value else migrate storedVersion value
    }

decodeValue :: FromJSON value => Value -> Either Text value
decodeValue value = case parseEither parseJSON value of
  Left message -> Left (Text.pack message)
  Right decoded -> Right decoded

-- | Immutable checkpoint handles make version flow visible in Haskell.  A
-- successful checkpoint returns a new handle; stale handles remain stale.
data StateHandle stateValue = StateHandle
  { internalHandleState :: State stateValue,
    internalHandleValue :: stateValue,
    internalHandleRevision :: Maybe StateRevision,
    internalHandleOccurrence :: Text,
    internalHandleFence :: Text,
    internalHandleFenceEpoch :: Word32
  }

currentState :: StateHandle stateValue -> stateValue
currentState = internalHandleValue

stateRevision :: StateHandle stateValue -> Maybe StateRevision
stateRevision = internalHandleRevision

newtype CheckpointId = CheckpointId {unCheckpointId :: Text}
  deriving (Eq, Ord, Show)

data StateConflict = StateConflict
  { conflictExpectedRevision :: Maybe StateRevision,
    conflictActualRevision :: Maybe StateRevision
  }
  deriving (Eq, Show)

data CheckpointRequest = CheckpointRequest
  { checkpointRequestKey :: StateKey,
    checkpointRequestExpected :: Maybe StateRevision,
    checkpointRequestSchema :: SchemaVersion,
    checkpointRequestValue :: Value,
    checkpointRequestOperation :: CheckpointId,
    checkpointRequestOccurrence :: Text,
    checkpointRequestFence :: Text,
    checkpointRequestFenceEpoch :: Word32
  }

instance ToJSON CheckpointRequest where
  toJSON request =
    object
      [ "state_key" .= unStateKey (checkpointRequestKey request),
        "expected_revision" .= fmap unStateRevision (checkpointRequestExpected request),
        "schema_version" .= unSchemaVersion (checkpointRequestSchema request),
        "value" .= checkpointRequestValue request,
        "conflict" .= ("compare-and-set" :: Text),
        "operation_id" .= unCheckpointId (checkpointRequestOperation request),
        "occurrence_id" .= checkpointRequestOccurrence request,
        "fencing_token" .= checkpointRequestFence request,
        "fencing_epoch" .= checkpointRequestFenceEpoch request
      ]

data CheckpointResponse
  = CheckpointApplied StateRevision
  | CheckpointRejected (Maybe StateRevision)

instance FromJSON CheckpointResponse where
  parseJSON = withObject "Segno compare-and-set response" $ \fields -> do
    rejectUnknown ["applied", "revision", "current_revision"] fields
    applied <- fields .: "applied"
    revisionValue <- fmap StateRevision <$> fields .:? "revision"
    current <- fmap StateRevision <$> fields .:? "current_revision"
    case (applied, revisionValue, current) of
      (True, Just revision, Nothing) -> pure (CheckpointApplied revision)
      (False, Nothing, actual) -> pure (CheckpointRejected actual)
      (True, Nothing, _) -> fail "applied response requires revision"
      (True, Just _, Just _) -> fail "applied response must not contain current_revision"
      (False, Just _, _) -> fail "rejected response must not contain revision"

checkpoint :: CheckpointId -> StateHandle stateValue -> stateValue -> Workflow (Either StateConflict (StateHandle stateValue))
checkpoint checkpointIdentity handle nextValue = do
  let selectedState = internalHandleState handle
      backend = stateBackendPlugin (internalStateBackend selectedState)
      request =
        CheckpointRequest
          { checkpointRequestKey = internalStateKey selectedState,
            checkpointRequestExpected = internalHandleRevision handle,
            checkpointRequestSchema = internalSchemaVersion selectedState,
            checkpointRequestValue = internalEncodeState selectedState nextValue,
            checkpointRequestOperation = checkpointIdentity,
            checkpointRequestOccurrence = internalHandleOccurrence handle,
            checkpointRequestFence = internalHandleFence handle,
            checkpointRequestFenceEpoch = internalHandleFenceEpoch handle
          }
      method = jsonPlugin backend "compare-and-set" :: Plugin CheckpointRequest CheckpointResponse
  response <- call method request
  case response of
    CheckpointApplied nextRevision ->
      pure . Right $
        handle
          { internalHandleValue = nextValue,
            internalHandleRevision = Just nextRevision
          }
    CheckpointRejected actualRevision ->
      pure . Left $
        StateConflict
          { conflictExpectedRevision = internalHandleRevision handle,
            conflictActualRevision = actualRevision
          }

-- | The handle identifies the latest durable checkpoint.  @SetState@ asks the
-- driver to perform one final CAS; @KeepState@ performs no final business-state
-- write.  Earlier successful checkpoints are never rolled back.
data StateTransition stateValue
  = KeepState (StateHandle stateValue)
  | SetState (StateHandle stateValue) stateValue

data RetrySpec = RetrySpec
  { retryAfterMilliseconds :: Word64,
    retryReason :: Text
  }
  deriving (Eq, Show)

data TaskFailure = TaskFailure
  { taskFailureCode :: Text,
    taskFailureMessage :: Text,
    taskFailureDetails :: Maybe Value
  }
  deriving (Eq, Show)

data Decision stateValue output
  = Ignore
  | Complete (StateTransition stateValue) output
  | Retry RetrySpec (StateTransition stateValue)
  | Fail TaskFailure

data PersistentTask stateValue event output where
  PersistentTask ::
    ToJSON output =>
    { internalTaskName :: Text,
      internalTaskTrigger :: Trigger stateValue event,
      internalTaskState :: State stateValue,
      internalTaskWorkflow :: Occurrence event -> StateHandle stateValue -> Workflow (Decision stateValue output)
    } ->
    PersistentTask stateValue event output

persistentTask :: ToJSON output => Text -> Trigger stateValue event -> State stateValue -> (Occurrence event -> StateHandle stateValue -> Workflow (Decision stateValue output)) -> PersistentTask stateValue event output
persistentTask = PersistentTask

data SegnoError
  = InvalidSegnoDefinition Text
  | InvalidSegnoEnvironment Text
  | InvalidSegnoDocument Text
  | SegnoDocumentTooLarge
  deriving (Eq, Show)

instance Exception SegnoError

taskManifest :: PersistentTask stateValue event output -> Either SegnoError Value
taskManifest selectedTask = do
  validateText "task name" (internalTaskName selectedTask)
  let selectedState = internalTaskState selectedTask
      sources = collectSources (internalTaskTrigger selectedTask)
  validateText "state key" (unStateKey (internalStateKey selectedState))
  validateText "state backend" (stateBackendPlugin (internalStateBackend selectedState))
  when (unSchemaVersion (internalSchemaVersion selectedState) == 0) $
    Left (InvalidSegnoDefinition "state schema version must be non-zero")
  traverse_ validateSource sources
  let identities = fmap sourceIdentity sources
  when (Set.size (Set.fromList identities) /= length identities) $
    Left (InvalidSegnoDefinition "trigger source identities must be unique")
  pure $
    object
      [ "api" .= manifestApi,
        "task" .= internalTaskName selectedTask,
        "sources" .= fmap encodeSource sources,
        "state"
          .= object
            [ "key" .= unStateKey (internalStateKey selectedState),
              "schema_version" .= unSchemaVersion (internalSchemaVersion selectedState),
              "backend" .= stateBackendPlugin (internalStateBackend selectedState),
              "conflict" .= ("compare-and-set" :: Text),
              "initial" .= internalEncodeState selectedState (internalInitialState selectedState)
            ]
      ]
  where
    validateSource source = do
      validateText "trigger identity" (unTriggerId (sourceIdentity source))
      validateText "trigger plugin" (sourcePlugin source)
    encodeSource source =
      object
        [ "id" .= unTriggerId (sourceIdentity source),
          "plugin" .= sourcePlugin source,
          "config" .= sourceConfiguration source
        ]

validateText :: Text -> Text -> Either SegnoError ()
validateText label value
  | Text.null value = Left (InvalidSegnoDefinition (label <> " must not be empty"))
  | Text.length value > 256 = Left (InvalidSegnoDefinition (label <> " exceeds 256 characters"))
  | Text.any (\character -> character < ' ' || character == '\DEL') value =
      Left (InvalidSegnoDefinition (label <> " contains a control character"))
  | otherwise = Right ()

collectSources :: Trigger stateValue event -> [TriggerSource]
collectSources = \case
  SourceTrigger source -> [source]
  MapTrigger _ nested -> collectSources nested
  FilterTrigger _ nested -> collectSources nested
  MergeTrigger left right -> collectSources left <> collectSources right
  GateTrigger _ nested -> collectSources nested

data TriggerMatch event
  = UnmatchedTrigger
  | RejectedTrigger
  | MatchedTrigger event

evaluateTrigger :: stateValue -> Trigger stateValue event -> TriggerId -> Value -> Either SegnoError (TriggerMatch event)
evaluateTrigger stateValue selectedTrigger selectedIdentity rawPayload = case selectedTrigger of
  SourceTrigger source
    | sourceIdentity source /= selectedIdentity -> Right UnmatchedTrigger
    | otherwise -> case decodeValue rawPayload of
        Left message -> Left (InvalidSegnoDocument ("trigger payload failed to decode: " <> message))
        Right value -> Right (MatchedTrigger value)
  MapTrigger transform nested -> fmap (fmapMatch transform) (evaluateTrigger stateValue nested selectedIdentity rawPayload)
  FilterTrigger predicate nested -> do
    nestedMatch <- evaluateTrigger stateValue nested selectedIdentity rawPayload
    pure $ case nestedMatch of
      MatchedTrigger value | not (predicate value) -> RejectedTrigger
      other -> other
  MergeTrigger left right -> do
    leftMatch <- evaluateTrigger stateValue left selectedIdentity rawPayload
    rightMatch <- evaluateTrigger stateValue right selectedIdentity rawPayload
    case (leftMatch, rightMatch) of
      (UnmatchedTrigger, other) -> Right other
      (other, UnmatchedTrigger) -> Right other
      _ -> Left (InvalidSegnoDefinition "trigger identity matches more than one merged source")
  GateTrigger predicate nested -> do
    nestedMatch <- evaluateTrigger stateValue nested selectedIdentity rawPayload
    pure $ case nestedMatch of
      MatchedTrigger value | not (predicate stateValue value) -> RejectedTrigger
      other -> other
  where
    fmapMatch transform = \case
      UnmatchedTrigger -> UnmatchedTrigger
      RejectedTrigger -> RejectedTrigger
      MatchedTrigger value -> MatchedTrigger (transform value)

data InvocationWire = InvocationWire
  { invocationTask :: Text,
    invocationAttempt :: Word32,
    invocationFence :: Text,
    invocationTrigger :: TriggerWire,
    invocationState :: StateWire
  }

data TriggerWire = TriggerWire
  { wireTriggerId :: TriggerId,
    wireOccurrenceId :: Text,
    wireLogicalTime :: UTCTime,
    wireObservedTime :: UTCTime,
    wireCursor :: Value,
    wireIdempotencyKey :: Text,
    wirePayload :: Value
  }

data StateWire = StateWire
  { wireStateKey :: StateKey,
    wireStateRevision :: Maybe StateRevision,
    wireSchemaVersion :: SchemaVersion,
    wireStateValue :: Value
  }

instance FromJSON InvocationWire where
  parseJSON = withObject "Segno invocation" $ \fields -> do
    rejectUnknown ["api", "task", "attempt", "fencing_token", "trigger", "state"] fields
    api <- fields .: "api"
    unless (api == invocationApi) $ fail "unsupported Segno invocation api"
    InvocationWire
      <$> fields .: "task"
      <*> fields .: "attempt"
      <*> fields .: "fencing_token"
      <*> fields .: "trigger"
      <*> fields .: "state"

instance FromJSON TriggerWire where
  parseJSON = withObject "Segno trigger occurrence" $ \fields -> do
    rejectUnknown ["trigger_id", "occurrence_id", "logical_time", "observed_time", "cursor", "idempotency_key", "payload"] fields
    TriggerWire
      <$> (TriggerId <$> fields .: "trigger_id")
      <*> fields .: "occurrence_id"
      <*> fields .: "logical_time"
      <*> fields .: "observed_time"
      <*> fields .: "cursor"
      <*> fields .: "idempotency_key"
      <*> fields .: "payload"

instance FromJSON StateWire where
  parseJSON = withObject "Segno business state" $ \fields -> do
    rejectUnknown ["key", "revision", "schema_version", "value"] fields
    StateWire
      <$> (StateKey <$> fields .: "key")
      <*> (fmap StateRevision <$> fields .:? "revision")
      <*> (SchemaVersion <$> fields .: "schema_version")
      <*> fields .: "value"

rejectUnknown :: [Text] -> Object -> Parser ()
rejectUnknown allowed fields =
  case filter (`Set.notMember` accepted) (fmap Key.toText (KeyMap.keys fields)) of
    [] -> pure ()
    unknown : _ -> fail ("unknown field: " <> Text.unpack unknown)
  where
    accepted = Set.fromList allowed

runPersistentTask :: PersistentTask stateValue event output -> IO ()
runPersistentTask selectedTask = do
  mode <- requireEnvironment "SEGNO_MODE"
  resultPath <- requireEnvironment "SEGNO_RESULT_PATH"
  result <- case mode of
    "describe" -> either throwIO pure (taskManifest selectedTask)
    "execute" -> executeOne selectedTask
    _ -> throwIO (InvalidSegnoEnvironment "SEGNO_MODE must be describe or execute")
  atomicWriteJson resultPath result

executeOne :: PersistentTask stateValue event output -> IO Value
executeOne selectedTask@PersistentTask {} = do
  invocationPath <- requireEnvironment "SEGNO_INVOCATION_PATH"
  encoded <- ByteString.readFile invocationPath
  when (ByteString.length encoded > maximumDocumentBytes) (throwIO SegnoDocumentTooLarge)
  raw <- either (throwIO . InvalidSegnoDocument . Text.pack) pure (decodeStrictJSON encoded)
  invocation <- case parseEither parseJSON raw of
    Left message -> throwIO (InvalidSegnoDocument (Text.pack message))
    Right decoded -> pure decoded
  validateInvocation selectedTask invocation
  let selectedState = internalTaskState selectedTask
  stateValue <-
    either (throwIO . InvalidSegnoDocument) pure $
      internalDecodeState selectedState (wireSchemaVersion (invocationState invocation)) (wireStateValue (invocationState invocation))
  triggerMatch <-
    either throwIO pure $
      evaluateTrigger stateValue (internalTaskTrigger selectedTask) (wireTriggerId (invocationTrigger invocation)) (wirePayload (invocationTrigger invocation))
  case triggerMatch of
    UnmatchedTrigger -> throwIO (InvalidSegnoDocument "invocation trigger does not belong to this task")
    RejectedTrigger -> pure (encodeResult selectedTask invocation (object ["kind" .= ("ignore" :: Text)]))
    MatchedTrigger event -> do
      let triggerWire = invocationTrigger invocation
          handle =
            StateHandle
              { internalHandleState = selectedState,
                internalHandleValue = stateValue,
                internalHandleRevision = wireStateRevision (invocationState invocation),
                internalHandleOccurrence = wireOccurrenceId triggerWire,
                internalHandleFence = invocationFence invocation,
                internalHandleFenceEpoch = invocationAttempt invocation
              }
          occurrence =
            Occurrence
              { occurrenceTriggerId = wireTriggerId triggerWire,
                occurrenceId = wireOccurrenceId triggerWire,
                occurrenceLogicalTime = wireLogicalTime triggerWire,
                occurrenceObservedTime = wireObservedTime triggerWire,
                occurrenceCursor = wireCursor triggerWire,
                occurrenceIdempotencyKey = wireIdempotencyKey triggerWire,
                occurrenceAttempt = invocationAttempt invocation,
                occurrencePayload = event
              }
      decision <- runTactus (internalTaskWorkflow selectedTask occurrence handle)
      encodedDecision <- either throwIO pure (encodeDecision selectedTask invocation decision)
      pure (encodeResult selectedTask invocation encodedDecision)

validateInvocation :: PersistentTask stateValue event output -> InvocationWire -> IO ()
validateInvocation selectedTask invocation = do
  unless (invocationTask invocation == internalTaskName selectedTask) $
    throwIO (InvalidSegnoDocument "task identity mismatch")
  let expectedState = internalTaskState selectedTask
      receivedState = invocationState invocation
  unless (wireStateKey receivedState == internalStateKey expectedState) $
    throwIO (InvalidSegnoDocument "state key mismatch")
  when (invocationAttempt invocation == 0) $
    throwIO (InvalidSegnoDocument "attempt must be non-zero")
  when (Text.null (invocationFence invocation)) $
    throwIO (InvalidSegnoDocument "fencing token must not be empty")
  let triggerWire = invocationTrigger invocation
  validateInvocationText "occurrence identity" (wireOccurrenceId triggerWire)
  validateInvocationText "idempotency key" (wireIdempotencyKey triggerWire)
  case wireStateRevision receivedState of
    Just (StateRevision revisionValue) -> validateInvocationText "state revision" revisionValue
    Nothing -> pure ()

validateInvocationText :: Text -> Text -> IO ()
validateInvocationText label value
  | Text.null value = throwIO (InvalidSegnoDocument (label <> " must not be empty"))
  | Text.length value > 512 = throwIO (InvalidSegnoDocument (label <> " exceeds 512 characters"))
  | Text.any (\character -> character < ' ' || character == '\DEL') value =
      throwIO (InvalidSegnoDocument (label <> " contains a control character"))
  | otherwise = pure ()

encodeDecision :: PersistentTask stateValue event output -> InvocationWire -> Decision stateValue output -> Either SegnoError Value
encodeDecision selectedTask@PersistentTask {} invocation = \case
  Ignore -> Right (object ["kind" .= ("ignore" :: Text)])
  Complete transition output -> do
    encodedTransition <- encodeTransition selectedTask invocation transition
    Right $
      object
        [ "kind" .= ("complete" :: Text),
          "state" .= encodedTransition,
          "output" .= toJSON output
        ]
  Retry specification transition -> do
    encodedTransition <- encodeTransition selectedTask invocation transition
    Right $
      object
        [ "kind" .= ("retry" :: Text),
          "state" .= encodedTransition,
          "after_ms" .= Text.pack (show (retryAfterMilliseconds specification)),
          "reason" .= retryReason specification
        ]
  Fail failure ->
    Right $
      object
        [ "kind" .= ("fail" :: Text),
          "error"
            .= object
              ( [ "code" .= taskFailureCode failure,
                  "message" .= taskFailureMessage failure
                ]
                  <> maybe [] (\details -> ["details" .= details]) (taskFailureDetails failure)
              )
        ]

encodeTransition :: PersistentTask stateValue event output -> InvocationWire -> StateTransition stateValue -> Either SegnoError Value
encodeTransition selectedTask invocation transition = do
  let (kind, handle, nextValue) = case transition of
        KeepState selectedHandle -> ("keep" :: Text, selectedHandle, Nothing)
        SetState selectedHandle value -> ("set", selectedHandle, Just value)
      expectedState = internalTaskState selectedTask
      expectedOccurrence = wireOccurrenceId (invocationTrigger invocation)
  unless (internalStateKey (internalHandleState handle) == internalStateKey expectedState) $
    Left (InvalidSegnoDocument "decision returned a handle for another state key")
  unless (internalHandleOccurrence handle == expectedOccurrence) $
    Left (InvalidSegnoDocument "decision returned a handle for another occurrence")
  unless (internalHandleFence handle == invocationFence invocation) $
    Left (InvalidSegnoDocument "decision returned a stale fencing token")
  unless (internalHandleFenceEpoch handle == invocationAttempt invocation) $
    Left (InvalidSegnoDocument "decision returned a stale fencing epoch")
  pure $
    object $
      [ "kind" .= kind,
        "expected_revision" .= fmap unStateRevision (internalHandleRevision handle)
      ]
        <> case nextValue of
          Nothing -> []
          Just value ->
            [ "schema_version" .= unSchemaVersion (internalSchemaVersion expectedState),
              "value" .= internalEncodeState expectedState value
            ]

encodeResult :: PersistentTask stateValue event output -> InvocationWire -> Value -> Value
encodeResult selectedTask invocation decision =
  object
    [ "api" .= resultApi,
      "task" .= internalTaskName selectedTask,
      "occurrence_id" .= wireOccurrenceId (invocationTrigger invocation),
      "decision" .= decision
    ]

requireEnvironment :: String -> IO FilePath
requireEnvironment name = do
  value <- lookupEnv name
  case value of
    Just found | not (null found) -> pure found
    _ -> throwIO (InvalidSegnoEnvironment (Text.pack name <> " is required"))

atomicWriteJson :: FilePath -> Value -> IO ()
atomicWriteJson destination value = do
  let encoded = LazyByteString.toStrict (encode value) <> ByteString.singleton 10
  when (ByteString.length encoded > maximumDocumentBytes) (throwIO SegnoDocumentTooLarge)
  mask_ $ do
    let directory = takeDirectory destination
    (temporary, handle) <- openBinaryTempFile directory ".segno-result.tmp"
    let ignoreIo action = action `catch` (\(_ :: IOException) -> pure ())
        cleanup = ignoreIo (hClose handle) >> ignoreIo (removeFile temporary)
        publish = do
          ByteString.hPut handle encoded
          hFlush handle
          hClose handle
          renameFile temporary destination
    publish `onException` cleanup
