{-# LANGUAGE OverloadedStrings #-}

-- | SQLite implementations for the built-in business-state plugin and the
-- driver's private lifecycle store.  They use separate database files so a
-- workflow can never rewrite scheduler state through its State handle.
module Segno.Store.SQLite
  ( SegnoPaths (..),
    segnoPaths,
    initialiseStore,
    CasResult (..),
    loadBusinessState,
    compareAndSetBusinessState,
    appendBusinessEvent,
    businessHistory,
    loadTriggerCursor,
    saveTriggerCursor,
    insertOccurrence,
    recoverExpiredLeases,
    claimNextOccurrence,
    markOccurrenceRunning,
    markOccurrenceSucceeded,
    markOccurrenceWaiting,
    markOccurrenceFailed,
    markOccurrenceFailedWithDetails,
    markOccurrenceUnknown,
    lifecycleStatus,
    lifecycleHistory,
    nextLifecycleWake,
    fenceIsCurrent,
  )
where

import Control.Exception (bracket, onException)
import Control.Monad (forM, when)
import Data.Aeson
  ( FromJSON,
    Value,
    object,
    (.=),
  )
import Data.Int (Int64)
import Data.Maybe (listToMaybe)
import Data.Text (Text)
import qualified Data.Text as Text
import Data.Time
  ( UTCTime,
    defaultTimeLocale,
    formatTime,
    parseTimeM,
  )
import Data.Word (Word32)
import Database.SQLite.Simple
  ( Connection,
    FromRow (fromRow),
    Only (Only),
    close,
    execute,
    execute_,
    field,
    open,
    query,
    query_,
  )
import Database.SQLite.Simple.Types (Query)
import Segno.Lifecycle
  ( ClaimedOccurrence (..),
    Lifecycle (..),
    OccurrenceRecord (..),
    lifecycleText,
    parseLifecycle,
  )
import Segno.Protocol
  ( StateManifest (..),
    StateSnapshot (..),
    TriggerOccurrence (..),
    decodeJsonText,
    encodeCompactText,
  )
import System.Directory (createDirectoryIfMissing)
import System.FilePath ((</>))
import Text.Read (readMaybe)

data SegnoPaths = SegnoPaths
  { pathsRoot :: FilePath,
    pathsControl :: FilePath,
    pathsJobs :: FilePath,
    pathsState :: FilePath,
    pathsTriggers :: FilePath,
    pathsBusinessDatabase :: FilePath,
    pathsLifecycleDatabase :: FilePath
  }
  deriving (Eq, Show)

segnoPaths :: FilePath -> SegnoPaths
segnoPaths root =
  let control = root </> ".tactus" </> "segno"
      stateDirectory = control </> "state"
   in SegnoPaths
        { pathsRoot = root,
          pathsControl = control,
          pathsJobs = control </> "jobs",
          pathsState = stateDirectory,
          pathsTriggers = control </> "triggers",
          pathsBusinessDatabase = stateDirectory </> "business.sqlite3",
          pathsLifecycleDatabase = stateDirectory </> "lifecycle.sqlite3"
        }

initialiseStore :: SegnoPaths -> IO ()
initialiseStore paths = do
  createDirectoryIfMissing True (pathsJobs paths)
  createDirectoryIfMissing True (pathsState paths)
  createDirectoryIfMissing True (pathsTriggers paths)
  withDatabase (pathsBusinessDatabase paths) initialiseBusinessSchema
  withDatabase (pathsLifecycleDatabase paths) initialiseLifecycleSchema

initialiseBusinessSchema :: Connection -> IO ()
initialiseBusinessSchema connection = do
  execute_
    connection
    "CREATE TABLE IF NOT EXISTS business_state (state_key TEXT PRIMARY KEY, schema_version INTEGER NOT NULL, revision INTEGER NOT NULL, value_json TEXT NOT NULL, updated_at TEXT NOT NULL)"
  execute_
    connection
    "CREATE TABLE IF NOT EXISTS state_history (sequence INTEGER PRIMARY KEY AUTOINCREMENT, state_key TEXT NOT NULL, revision INTEGER, event_kind TEXT NOT NULL, operation_id TEXT, occurrence_id TEXT, value_json TEXT NOT NULL, recorded_at TEXT NOT NULL)"
  execute_
    connection
    "CREATE TABLE IF NOT EXISTS state_operations (state_key TEXT NOT NULL, occurrence_id TEXT NOT NULL, operation_id TEXT NOT NULL, request_json TEXT NOT NULL, resulting_revision INTEGER NOT NULL, recorded_at TEXT NOT NULL, PRIMARY KEY(state_key, occurrence_id, operation_id))"

initialiseLifecycleSchema :: Connection -> IO ()
initialiseLifecycleSchema connection = do
  execute_
    connection
    "CREATE TABLE IF NOT EXISTS trigger_cursors (job_id TEXT NOT NULL, source_id TEXT NOT NULL, cursor_json TEXT, next_wake TEXT, updated_at TEXT NOT NULL, PRIMARY KEY(job_id, source_id))"
  execute_
    connection
    "CREATE TABLE IF NOT EXISTS occurrences (occurrence_id TEXT PRIMARY KEY, job_id TEXT NOT NULL, trigger_id TEXT NOT NULL, idempotency_key TEXT NOT NULL, logical_time TEXT NOT NULL, observed_time TEXT NOT NULL, cursor_json TEXT NOT NULL, payload_json TEXT NOT NULL, lifecycle TEXT NOT NULL, attempt INTEGER NOT NULL, fencing_token TEXT, lease_until TEXT, next_attempt_at TEXT, updated_at TEXT NOT NULL, error_text TEXT, output_json TEXT, UNIQUE(job_id, trigger_id, idempotency_key))"
  execute_
    connection
    "CREATE INDEX IF NOT EXISTS occurrences_runnable ON occurrences(lifecycle, next_attempt_at, logical_time)"
  execute_
    connection
    "CREATE TABLE IF NOT EXISTS lifecycle_history (sequence INTEGER PRIMARY KEY AUTOINCREMENT, occurrence_id TEXT NOT NULL, lifecycle TEXT NOT NULL, attempt INTEGER NOT NULL, details_json TEXT NOT NULL, recorded_at TEXT NOT NULL)"

data CasResult
  = CasApplied Text
  | CasConflict (Maybe Text)
  deriving (Eq, Show)

loadBusinessState :: SegnoPaths -> StateManifest -> UTCTime -> IO StateSnapshot
loadBusinessState paths manifest now = withDatabase (pathsBusinessDatabase paths) $ \connection -> do
  let key = stateKey manifest
      initialJson = encodeCompactText (stateInitial manifest)
      nowText = renderTime now
  execute
    connection
    "INSERT OR IGNORE INTO business_state(state_key, schema_version, revision, value_json, updated_at) VALUES (?, ?, 0, ?, ?)"
    (key, fromIntegral (stateSchemaVersion manifest) :: Int64, initialJson, nowText)
  rows <-
    query
      connection
      "SELECT schema_version, revision, value_json FROM business_state WHERE state_key = ?"
      (Only key) :: IO [(Int64, Int64, Text)]
  case rows of
    [(schemaVersion, revision, encoded)] -> do
      value <- decodeOrThrow "stored business state" encoded
      pure
        StateSnapshot
          { snapshotKey = key,
            snapshotRevision = Just (Text.pack (show revision)),
            snapshotSchemaVersion = fromIntegral schemaVersion,
            snapshotValue = value
          }
    _ -> ioError (userError "business state disappeared after initialization")

compareAndSetBusinessState :: SegnoPaths -> Text -> Maybe Text -> Word32 -> Value -> Text -> Text -> Text -> Word32 -> UTCTime -> IO CasResult
compareAndSetBusinessState paths key expectedRevision schemaVersion value operationId occurrenceId fencingToken fencingEpoch now =
  withDatabase (pathsBusinessDatabase paths) $ \connection -> do
    execute connection "ATTACH DATABASE ? AS lifecycle_store" (Only (pathsLifecycleDatabase paths))
    withImmediateTransaction connection (performCas connection)
  where
    performCas connection = do
      fenceRows <-
        query
          connection
          "SELECT COUNT(*) FROM lifecycle_store.occurrences WHERE occurrence_id = ? AND lifecycle = 'running' AND fencing_token = ? AND attempt = ?"
          (occurrenceId, fencingToken, fromIntegral fencingEpoch :: Int64) :: IO [Only Int64]
      if fenceRows /= [Only 1]
        then CasConflict <$> currentRevision connection key
        else continueCas connection

    continueCas connection = do
      let requestJson =
            encodeCompactText $
              object
                  [ "expected_revision" .= expectedRevision,
                    "schema_version" .= schemaVersion,
                    "value" .= value,
                    "fencing_epoch" .= fencingEpoch
                  ]
      previous <-
        query
          connection
          "SELECT request_json, resulting_revision FROM state_operations WHERE state_key = ? AND occurrence_id = ? AND operation_id = ?"
          (key, occurrenceId, operationId) :: IO [(Text, Int64)]
      case previous of
        [(storedRequest, revision)]
          | storedRequest == requestJson -> pure (CasApplied (Text.pack (show revision)))
          | otherwise -> CasConflict <$> currentRevision connection key
        _ -> do
          actual <- currentRevision connection key
          if actual /= expectedRevision
            then pure (CasConflict actual)
            else do
              let nextRevision = maybe 0 ((+ 1) . parseRevisionUnsafe) actual
                  encoded = encodeCompactText value
                  nowText = renderTime now
              case actual of
                Nothing ->
                  execute
                    connection
                    "INSERT INTO business_state(state_key, schema_version, revision, value_json, updated_at) VALUES (?, ?, ?, ?, ?)"
                    (key, fromIntegral schemaVersion :: Int64, nextRevision, encoded, nowText)
                Just revisionText ->
                  execute
                    connection
                    "UPDATE business_state SET schema_version = ?, revision = ?, value_json = ?, updated_at = ? WHERE state_key = ? AND revision = ?"
                    (fromIntegral schemaVersion :: Int64, nextRevision, encoded, nowText, key, parseRevisionUnsafe revisionText)
              execute
                connection
                "INSERT INTO state_operations(state_key, occurrence_id, operation_id, request_json, resulting_revision, recorded_at) VALUES (?, ?, ?, ?, ?, ?)"
                (key, occurrenceId, operationId, requestJson, nextRevision, nowText)
              execute
                connection
                "INSERT INTO state_history(state_key, revision, event_kind, operation_id, occurrence_id, value_json, recorded_at) VALUES (?, ?, 'set', ?, ?, ?, ?)"
                (key, nextRevision, operationId, occurrenceId, encoded, nowText)
              pure (CasApplied (Text.pack (show nextRevision)))

appendBusinessEvent :: SegnoPaths -> Text -> Text -> Value -> Maybe Text -> UTCTime -> IO ()
appendBusinessEvent paths key eventKind payload occurrenceId now =
  withDatabase (pathsBusinessDatabase paths) $ \connection ->
    execute
      connection
      "INSERT INTO state_history(state_key, revision, event_kind, operation_id, occurrence_id, value_json, recorded_at) VALUES (?, NULL, ?, NULL, ?, ?, ?)"
      (key, eventKind, occurrenceId, encodeCompactText payload, renderTime now)

businessHistory :: SegnoPaths -> Maybe Text -> Int -> IO [Value]
businessHistory paths selectedKey limit = withDatabase (pathsBusinessDatabase paths) $ \connection -> do
  rows <- case selectedKey of
    Nothing ->
      query
        connection
        "SELECT sequence, state_key, revision, event_kind, operation_id, occurrence_id, value_json, recorded_at FROM state_history ORDER BY sequence DESC LIMIT ?"
        (Only limit)
    Just key ->
      query
        connection
        "SELECT sequence, state_key, revision, event_kind, operation_id, occurrence_id, value_json, recorded_at FROM state_history WHERE state_key = ? ORDER BY sequence DESC LIMIT ?"
        (key, limit)
  traverse encodeBusinessHistory rows

data BusinessHistoryRow = BusinessHistoryRow Int64 Text (Maybe Int64) Text (Maybe Text) (Maybe Text) Text Text

instance FromRow BusinessHistoryRow where
  fromRow = BusinessHistoryRow <$> field <*> field <*> field <*> field <*> field <*> field <*> field <*> field

encodeBusinessHistory :: BusinessHistoryRow -> IO Value
encodeBusinessHistory (BusinessHistoryRow sequenceNumber key revision kind operation occurrence encoded recordedAt) = do
  payload <- decodeOrThrow "business history value" encoded :: IO Value
  pure $
    object
      [ "sequence" .= sequenceNumber,
        "state_key" .= key,
        "revision" .= fmap show revision,
        "event_kind" .= kind,
        "operation_id" .= operation,
        "occurrence_id" .= occurrence,
        "value" .= payload,
        "recorded_at" .= recordedAt
      ]

loadTriggerCursor :: SegnoPaths -> Text -> Text -> IO (Maybe Value)
loadTriggerCursor paths jobId sourceIdentity = withDatabase (pathsLifecycleDatabase paths) $ \connection -> do
  rows <-
    query
      connection
      "SELECT cursor_json FROM trigger_cursors WHERE job_id = ? AND source_id = ?"
      (jobId, sourceIdentity) :: IO [Only (Maybe Text)]
  case rows of
    [Only (Just encoded)] -> Just <$> decodeOrThrow "trigger cursor" encoded
    _ -> pure Nothing

saveTriggerCursor :: SegnoPaths -> Text -> Text -> Maybe Value -> Maybe UTCTime -> UTCTime -> IO ()
saveTriggerCursor paths jobId sourceIdentity cursor nextWake now =
  withDatabase (pathsLifecycleDatabase paths) $ \connection ->
    execute
      connection
      "INSERT INTO trigger_cursors(job_id, source_id, cursor_json, next_wake, updated_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(job_id, source_id) DO UPDATE SET cursor_json = excluded.cursor_json, next_wake = excluded.next_wake, updated_at = excluded.updated_at"
      ( jobId,
        sourceIdentity,
        fmap encodeCompactText cursor,
        fmap renderTime nextWake,
        renderTime now
      )

insertOccurrence :: SegnoPaths -> Text -> TriggerOccurrence -> UTCTime -> IO Bool
insertOccurrence paths jobId occurrence now = withDatabase (pathsLifecycleDatabase paths) $ \connection -> do
  withImmediateTransaction connection $ do
    execute
      connection
      "INSERT OR IGNORE INTO occurrences(occurrence_id, job_id, trigger_id, idempotency_key, logical_time, observed_time, cursor_json, payload_json, lifecycle, attempt, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'ready', 0, ?)"
      ( occurrenceId occurrence,
        jobId,
        occurrenceTriggerId occurrence,
        occurrenceIdempotencyKey occurrence,
        renderTime (occurrenceLogicalTime occurrence),
        renderTime (occurrenceObservedTime occurrence),
        encodeCompactText (occurrenceCursor occurrence),
        encodeCompactText (occurrencePayload occurrence),
        renderTime now
      )
    inserted <- sqliteChanges connection
    if inserted == 1
      then appendLifecycleHistory connection (occurrenceId occurrence) Ready 0 (object []) now >> pure True
      else pure False

recoverExpiredLeases :: SegnoPaths -> UTCTime -> IO Int
recoverExpiredLeases paths now =
  withDatabase (pathsLifecycleDatabase paths) $ \connection ->
    withImmediateTransaction connection $ do
      expired <-
        query
          connection
          "SELECT occurrence_id, lifecycle, attempt FROM occurrences WHERE lifecycle IN ('claimed', 'running') AND lease_until IS NOT NULL AND lease_until <= ? ORDER BY occurrence_id"
          (Only (renderTime now)) :: IO [(Text, Text, Int64)]
      changed <- forM expired $ \(occurrenceIdentity, previousLifecycle, attempt) -> do
        let nextLifecycle = if previousLifecycle == "claimed" then Ready else OutcomeUnknown
            message :: Text
            message =
              if previousLifecycle == "claimed"
                then "claim lease expired before workflow execution; occurrence made runnable again"
                else "running lease expired; workflow side effects may already have occurred"
        execute
          connection
          "UPDATE occurrences SET lifecycle = ?, fencing_token = NULL, lease_until = NULL, next_attempt_at = NULL, updated_at = ?, error_text = ? WHERE occurrence_id = ? AND lifecycle = ? AND lease_until IS NOT NULL AND lease_until <= ?"
          ( lifecycleText nextLifecycle,
            renderTime now,
            message,
            occurrenceIdentity,
            previousLifecycle,
            renderTime now
          )
        updated <- sqliteChanges connection
        when (updated == 1) $
          appendLifecycleHistory
            connection
            occurrenceIdentity
            nextLifecycle
            attempt
            (object ["reason" .= message, "expired_from" .= previousLifecycle])
            now
        pure updated
      pure (fromIntegral (sum changed))

claimNextOccurrence :: SegnoPaths -> UTCTime -> UTCTime -> IO (Maybe ClaimedOccurrence)
claimNextOccurrence paths now leaseUntil = withDatabase (pathsLifecycleDatabase paths) $ \connection -> withImmediateTransaction connection $ do
  candidates <-
    query
      connection
      "SELECT occurrence_id, attempt FROM occurrences WHERE lifecycle = 'ready' OR (lifecycle = 'waiting' AND next_attempt_at IS NOT NULL AND next_attempt_at <= ?) ORDER BY logical_time, occurrence_id LIMIT 1"
      (Only (renderTime now)) :: IO [(Text, Int64)]
  case candidates of
    [] -> pure Nothing
    (selectedId, previousAttempt) : _ -> do
      let attempt = previousAttempt + 1
          fence = selectedId <> ":" <> Text.pack (show attempt)
      execute
        connection
        "UPDATE occurrences SET lifecycle = 'claimed', attempt = ?, fencing_token = ?, lease_until = ?, next_attempt_at = NULL, updated_at = ?, error_text = NULL WHERE occurrence_id = ? AND (lifecycle = 'ready' OR (lifecycle = 'waiting' AND next_attempt_at IS NOT NULL AND next_attempt_at <= ?))"
        (attempt, fence, renderTime leaseUntil, renderTime now, selectedId, renderTime now)
      changed <- sqliteChanges connection
      if changed /= 1
        then pure Nothing
        else do
          appendLifecycleHistory connection selectedId Claimed attempt (object ["fencing_token" .= fence, "lease_until" .= leaseUntil]) now
          rows <- query connection occurrenceSelectById (Only selectedId)
          case rows of
            [row] -> Just <$> rowToClaimed row
            _ -> pure Nothing

markOccurrenceRunning :: SegnoPaths -> Text -> Text -> UTCTime -> UTCTime -> IO Bool
markOccurrenceRunning paths occurrenceIdentity fence leaseUntil now =
  withDatabase (pathsLifecycleDatabase paths) $ \connection ->
    withImmediateTransaction connection $ do
      execute
        connection
        "UPDATE occurrences SET lifecycle = 'running', lease_until = ?, updated_at = ? WHERE occurrence_id = ? AND lifecycle = 'claimed' AND fencing_token = ?"
        (renderTime leaseUntil, renderTime now, occurrenceIdentity, fence)
      changed <- sqliteChanges connection
      if changed == 1
        then do
          attempts <- query connection "SELECT attempt FROM occurrences WHERE occurrence_id = ?" (Only occurrenceIdentity) :: IO [Only Int64]
          let attempt = maybe 0 (\(Only value) -> value) (listToMaybe attempts)
          appendLifecycleHistory connection occurrenceIdentity Running attempt (object []) now
          pure True
        else pure False

markOccurrenceSucceeded :: SegnoPaths -> Text -> Text -> Value -> UTCTime -> IO Bool
markOccurrenceSucceeded paths occurrenceIdentity fence output now =
  transitionOccurrence paths occurrenceIdentity fence Succeeded Nothing (Just output) Nothing now

markOccurrenceWaiting :: SegnoPaths -> Text -> Text -> Text -> UTCTime -> UTCTime -> IO Bool
markOccurrenceWaiting paths occurrenceIdentity fence reason retryAt now =
  transitionOccurrence paths occurrenceIdentity fence Waiting (Just reason) Nothing (Just retryAt) now

markOccurrenceFailed :: SegnoPaths -> Text -> Text -> Text -> Maybe UTCTime -> UTCTime -> IO Bool
markOccurrenceFailed paths occurrenceIdentity fence message retryAt now =
  transitionOccurrence paths occurrenceIdentity fence Failed (Just message) Nothing retryAt now

markOccurrenceFailedWithDetails :: SegnoPaths -> Text -> Text -> Text -> Maybe Value -> UTCTime -> IO Bool
markOccurrenceFailedWithDetails paths occurrenceIdentity fence message details now =
  transitionOccurrence paths occurrenceIdentity fence Failed (Just message) details Nothing now

markOccurrenceUnknown :: SegnoPaths -> Text -> Text -> Text -> UTCTime -> IO Bool
markOccurrenceUnknown paths occurrenceIdentity fence message now =
  transitionOccurrence paths occurrenceIdentity fence OutcomeUnknown (Just message) Nothing Nothing now

transitionOccurrence :: SegnoPaths -> Text -> Text -> Lifecycle -> Maybe Text -> Maybe Value -> Maybe UTCTime -> UTCTime -> IO Bool
transitionOccurrence paths occurrenceIdentity fence lifecycle failure output retryAt now =
  withDatabase (pathsLifecycleDatabase paths) $ \connection -> do
    withImmediateTransaction connection $ do
      execute
        connection
        "UPDATE occurrences SET lifecycle = ?, lease_until = NULL, next_attempt_at = ?, updated_at = ?, error_text = ?, output_json = ? WHERE occurrence_id = ? AND lifecycle = 'running' AND fencing_token = ?"
        ( lifecycleText lifecycle,
          fmap renderTime retryAt,
          renderTime now,
          failure,
          fmap encodeCompactText output,
          occurrenceIdentity,
          fence
        )
      changed <- sqliteChanges connection
      if changed == 1
        then do
          attempts <- query connection "SELECT attempt FROM occurrences WHERE occurrence_id = ?" (Only occurrenceIdentity) :: IO [Only Int64]
          let attempt = maybe 0 (\(Only value) -> value) (listToMaybe attempts)
              details = object ["error" .= failure, "output" .= output, "retry_at" .= retryAt]
          appendLifecycleHistory connection occurrenceIdentity lifecycle attempt details now
          pure True
        else pure False

lifecycleStatus :: SegnoPaths -> Maybe Text -> IO [OccurrenceRecord]
lifecycleStatus paths selectedJob = withDatabase (pathsLifecycleDatabase paths) $ \connection -> do
  rows <- case selectedJob of
    Nothing -> query_ connection occurrenceSelectAll
    Just jobId -> query connection occurrenceSelectByJob (Only jobId)
  traverse rowToRecord rows

lifecycleHistory :: SegnoPaths -> Maybe Text -> Int -> IO [Value]
lifecycleHistory paths selectedOccurrence limit = withDatabase (pathsLifecycleDatabase paths) $ \connection -> do
  rows <- case selectedOccurrence of
    Nothing ->
      query
        connection
        "SELECT sequence, occurrence_id, lifecycle, attempt, details_json, recorded_at FROM lifecycle_history ORDER BY sequence DESC LIMIT ?"
        (Only limit)
    Just occurrenceIdentity ->
      query
        connection
        "SELECT sequence, occurrence_id, lifecycle, attempt, details_json, recorded_at FROM lifecycle_history WHERE occurrence_id = ? ORDER BY sequence DESC LIMIT ?"
        (occurrenceIdentity, limit)
  traverse encodeLifecycleHistory rows

data LifecycleHistoryRow = LifecycleHistoryRow Int64 Text Text Int64 Text Text

instance FromRow LifecycleHistoryRow where
  fromRow = LifecycleHistoryRow <$> field <*> field <*> field <*> field <*> field <*> field

encodeLifecycleHistory :: LifecycleHistoryRow -> IO Value
encodeLifecycleHistory (LifecycleHistoryRow sequenceNumber occurrenceIdentity lifecycle attempt detailsJson recordedAt) = do
  details <- decodeOrThrow "lifecycle history details" detailsJson :: IO Value
  pure $
    object
      [ "sequence" .= sequenceNumber,
        "occurrence_id" .= occurrenceIdentity,
        "lifecycle" .= lifecycle,
        "attempt" .= attempt,
        "details" .= details,
        "recorded_at" .= recordedAt
      ]

nextLifecycleWake :: SegnoPaths -> IO (Maybe UTCTime)
nextLifecycleWake paths = withDatabase (pathsLifecycleDatabase paths) $ \connection -> do
  retryRows <- query_ connection "SELECT MIN(next_attempt_at) FROM occurrences WHERE lifecycle = 'waiting' AND next_attempt_at IS NOT NULL" :: IO [Only (Maybe Text)]
  triggerRows <- query_ connection "SELECT MIN(next_wake) FROM trigger_cursors WHERE next_wake IS NOT NULL" :: IO [Only (Maybe Text)]
  let encoded = [value | Only (Just value) <- retryRows <> triggerRows]
  parsed <- traverse parseTimeOrThrow encoded
  pure $ case parsed of
    [] -> Nothing
    values -> Just (minimum values)

fenceIsCurrent :: SegnoPaths -> Text -> Text -> IO Bool
fenceIsCurrent paths occurrenceIdentity fence = withDatabase (pathsLifecycleDatabase paths) $ \connection -> do
  rows <-
    query
      connection
      "SELECT COUNT(*) FROM occurrences WHERE occurrence_id = ? AND lifecycle = 'running' AND fencing_token = ?"
      (occurrenceIdentity, fence) :: IO [Only Int64]
  pure (rows == [Only 1])

data OccurrenceRow = OccurrenceRow Text Text Text Text Text Text Text Text Text Int64 (Maybe Text) (Maybe Text) (Maybe Text) Text (Maybe Text) (Maybe Text)

instance FromRow OccurrenceRow where
  fromRow =
    OccurrenceRow
      <$> field <*> field <*> field <*> field <*> field <*> field <*> field <*> field
      <*> field <*> field <*> field <*> field <*> field <*> field <*> field <*> field

occurrenceSelectById, occurrenceSelectByJob, occurrenceSelectAll :: Query
occurrenceSelectById = "SELECT occurrence_id, job_id, trigger_id, idempotency_key, logical_time, observed_time, cursor_json, payload_json, lifecycle, attempt, fencing_token, lease_until, next_attempt_at, updated_at, error_text, output_json FROM occurrences WHERE occurrence_id = ?"
occurrenceSelectByJob = "SELECT occurrence_id, job_id, trigger_id, idempotency_key, logical_time, observed_time, cursor_json, payload_json, lifecycle, attempt, fencing_token, lease_until, next_attempt_at, updated_at, error_text, output_json FROM occurrences WHERE job_id = ? ORDER BY logical_time DESC"
occurrenceSelectAll = "SELECT occurrence_id, job_id, trigger_id, idempotency_key, logical_time, observed_time, cursor_json, payload_json, lifecycle, attempt, fencing_token, lease_until, next_attempt_at, updated_at, error_text, output_json FROM occurrences ORDER BY logical_time DESC"

rowToRecord :: OccurrenceRow -> IO OccurrenceRecord
rowToRecord (OccurrenceRow occurrenceIdentity jobId triggerId idempotency logical observed cursorJson payloadJson lifecycle attempt fence lease retry updated failure outputJson) = do
  logicalTime <- parseTimeOrThrow logical
  observedTime <- parseTimeOrThrow observed
  cursor <- decodeOrThrow "occurrence cursor" cursorJson
  payload <- decodeOrThrow "occurrence payload" payloadJson
  lifecycleValue <- either (ioError . userError . Text.unpack) pure (parseLifecycle lifecycle)
  leaseTime <- traverse parseTimeOrThrow lease
  retryTime <- traverse parseTimeOrThrow retry
  updatedTime <- parseTimeOrThrow updated
  output <- traverse (decodeOrThrow "occurrence output") outputJson
  pure
    OccurrenceRecord
      { recordJobId = jobId,
        recordOccurrence =
          TriggerOccurrence
            { occurrenceTriggerId = triggerId,
              occurrenceId = occurrenceIdentity,
              occurrenceLogicalTime = logicalTime,
              occurrenceObservedTime = observedTime,
              occurrenceCursor = cursor,
              occurrenceIdempotencyKey = idempotency,
              occurrencePayload = payload
            },
        recordLifecycle = lifecycleValue,
        recordAttempt = fromIntegral attempt,
        recordFencingToken = fence,
        recordLeaseUntil = leaseTime,
        recordNextAttemptAt = retryTime,
        recordUpdatedAt = updatedTime,
        recordError = failure,
        recordOutput = output
      }

rowToClaimed :: OccurrenceRow -> IO ClaimedOccurrence
rowToClaimed row = do
  record <- rowToRecord row
  case (recordFencingToken record, recordLeaseUntil record) of
    (Just fence, Just lease) ->
      pure
        ClaimedOccurrence
          { claimedJobId = recordJobId record,
            claimedOccurrence = recordOccurrence record,
            claimedAttempt = recordAttempt record,
            claimedFencingToken = fence,
            claimedLeaseUntil = lease
          }
    _ -> ioError (userError "claimed occurrence is missing its fence or lease")

appendLifecycleHistory :: Connection -> Text -> Lifecycle -> Int64 -> Value -> UTCTime -> IO ()
appendLifecycleHistory connection occurrenceIdentity lifecycle attempt details now =
  execute
    connection
    "INSERT INTO lifecycle_history(occurrence_id, lifecycle, attempt, details_json, recorded_at) VALUES (?, ?, ?, ?, ?)"
    (occurrenceIdentity, lifecycleText lifecycle, attempt, encodeCompactText details, renderTime now)

currentRevision :: Connection -> Text -> IO (Maybe Text)
currentRevision connection key = do
  rows <- query connection "SELECT revision FROM business_state WHERE state_key = ?" (Only key) :: IO [Only Int64]
  pure $ case rows of
    [Only revision] -> Just (Text.pack (show revision))
    _ -> Nothing

parseRevisionUnsafe :: Text -> Int64
parseRevisionUnsafe encoded = maybe (error "database revision was not decimal") id (readMaybe (Text.unpack encoded))

withDatabase :: FilePath -> (Connection -> IO value) -> IO value
withDatabase path action = bracket (open path) close $ \connection -> do
  execute_ connection "PRAGMA journal_mode = WAL"
  execute_ connection "PRAGMA foreign_keys = ON"
  execute_ connection "PRAGMA busy_timeout = 5000"
  execute_ connection "PRAGMA synchronous = FULL"
  execute_ connection "PRAGMA user_version = 1"
  action connection

beginImmediate, commit, rollback :: Connection -> IO ()
beginImmediate connection = execute_ connection "BEGIN IMMEDIATE"
commit connection = execute_ connection "COMMIT"
rollback connection = execute_ connection "ROLLBACK"

withImmediateTransaction :: Connection -> IO value -> IO value
withImmediateTransaction connection action = do
  beginImmediate connection
  value <- action `onException` rollback connection
  commit connection
  pure value

-- sqlite-simple does not expose changes uniformly across historic versions;
-- querying the connection works on every supported SQLite build.
sqliteChanges :: Connection -> IO Int64
sqliteChanges connection = do
  rows <- query_ connection "SELECT changes()" :: IO [Only Int64]
  pure $ maybe 0 (\(Only value) -> value) (listToMaybe rows)

renderTime :: UTCTime -> Text
renderTime = Text.pack . formatTime defaultTimeLocale "%Y-%m-%dT%H:%M:%S%QZ"

parseTimeOrThrow :: Text -> IO UTCTime
parseTimeOrThrow encoded = case parseTimeM True defaultTimeLocale "%Y-%m-%dT%H:%M:%S%QZ" (Text.unpack encoded) of
  Nothing -> ioError (userError ("invalid UTC timestamp in Segno SQLite store: " <> Text.unpack encoded))
  Just value -> pure value

decodeOrThrow :: FromJSON value => String -> Text -> IO value
decodeOrThrow label encoded = case decodeJsonText encoded of
  Left message -> ioError (userError (label <> " is invalid JSON: " <> message))
  Right value -> pure value
