{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE ScopedTypeVariables #-}

-- | Single-node persistent driver. Trigger and business-state calls always
-- cross the same Tactus/plugin-v1 boundary used by third-party plugins.
module Segno.Driver
  ( Clock (..),
    Runner (..),
    ProcessOutcome (..),
    DriverEnvironment (..),
    RunSummary (..),
    systemClock,
    processRunner,
    defaultDriverEnvironment,
    discoverWorkspaceRoot,
    initialiseWorkspace,
    initialiseWorkspaceWithSdk,
    installJob,
    listJobs,
    runOnce,
    runOnceWith,
    runDriver,
    runDriverWith,
  )
where

import Control.Concurrent (threadDelay)
import Control.Concurrent.Async (async, wait)
import Control.Applicative ((<|>))
import Control.Exception
  ( Exception,
    IOException,
    SomeException,
    bracket,
    catch,
    displayException,
    mask_,
    onException,
    throwIO,
    try,
  )
import Control.Monad (forM, forM_, unless, when)
import Data.Aeson
  ( FromJSON (parseJSON),
    Object,
    ToJSON (toJSON),
    Value,
    encode,
    object,
    (.=),
  )
import qualified Data.Aeson.KeyMap as KeyMap
import Data.Aeson.Types (parseEither)
import qualified Data.ByteString as ByteString
import qualified Data.ByteString.Lazy as LazyByteString
import Data.Char (isAlphaNum)
import Data.List (sort, sortOn)
import qualified Data.Map.Strict as Map
import Data.Maybe (catMaybes)
import qualified Data.Set as Set
import Data.Text (Text)
import qualified Data.Text as Text
import qualified Data.Text.IO as TextIO
import Data.Text.Encoding (decodeUtf8', decodeUtf8With)
import qualified Data.Text.Encoding as TextEncoding
import Data.Text.Encoding.Error (lenientDecode)
import Data.Time
  ( NominalDiffTime,
    UTCTime,
    addUTCTime,
    diffUTCTime,
    formatTime,
    getCurrentTime,
    defaultTimeLocale,
  )
import qualified Clef.Plugin.Protocol as ClefProtocol
import qualified Crypto.Hash.SHA256 as SHA256
import Segno.Lifecycle
  ( ClaimedOccurrence (..)
  )
import Segno.Protocol
  ( Decision (..),
    InstalledJob (..),
    Invocation (..),
    PlannedOccurrence (..),
    PluginFailure (..),
    PollResult (..),
    SourceManifest (..),
    StateCasResult (..),
    StateManifest (..),
    StateSnapshot (..),
    TaskManifest (..),
    TaskResult (..),
    Transition (..),
    TriggerOccurrence (..),
    decodeJsonText,
  )
import Segno.Store.SQLite
  ( SegnoPaths (..),
    claimNextOccurrence,
    initialiseStore,
    insertOccurrence,
    loadTriggerCursor,
    markOccurrenceFailed,
    markOccurrenceFailedWithDetails,
    markOccurrenceRunning,
    markOccurrenceSucceeded,
    markOccurrenceUnknown,
    markOccurrenceWaiting,
    nextLifecycleWake,
    recoverExpiredLeases,
    saveTriggerCursor,
    segnoPaths,
  )
import System.Directory
  ( canonicalizePath,
    createDirectory,
    createDirectoryIfMissing,
    doesFileExist,
    getCurrentDirectory,
    getModificationTime,
    listDirectory,
    renameFile,
    removeFile,
    removeDirectoryRecursive,
  )
import System.Environment (getEnvironment, getExecutablePath, lookupEnv)
import System.Exit (ExitCode (..))
import System.FilePath
  ( isAbsolute,
    makeRelative,
    normalise,
    splitDirectories,
    takeDirectory,
    takeExtension,
    (</>),
  )
import System.IO
  ( BufferMode (NoBuffering),
    Handle,
    IOMode (ReadMode),
    hClose,
    hPutStrLn,
    hSetBinaryMode,
    hSetBuffering,
    openBinaryTempFile,
    stderr,
    withBinaryFile,
  )
import System.Process
  ( CreateProcess (..),
    ProcessHandle,
    StdStream (CreatePipe),
    proc,
    terminateProcess,
    waitForProcess,
    withCreateProcess,
  )
import System.IO.Error (isAlreadyExistsError)

data DriverError
  = WorkspaceNotInitialized FilePath
  | InvalidJob Text
  | DriverProtocolError Text
  | ExternalCommandFailed Text
  deriving (Eq, Show)

instance Exception DriverError

data Clock = Clock
  { clockNow :: IO UTCTime,
    clockSleepUntil :: UTCTime -> IO ()
  }

data ProcessOutcome = ProcessOutcome
  { processExitCode :: ExitCode,
    processStdout :: ByteString.ByteString,
    processStderr :: ByteString.ByteString,
    processOutputTruncated :: Bool
  }
  deriving (Eq, Show)

data Runner = Runner
  { runnerTask :: FilePath -> FilePath -> Text -> Maybe FilePath -> FilePath -> Int -> IO ProcessOutcome,
    runnerPlugin :: FilePath -> Text -> Text -> Object -> IO (Either PluginFailure Value)
  }

data DriverEnvironment = DriverEnvironment
  { driverClock :: Clock,
    driverRunner :: Runner,
    driverLeaseSeconds :: NominalDiffTime,
    driverRetrySeconds :: NominalDiffTime,
    driverMaximumAttempts :: Int,
    driverCatchupLimit :: Int,
    driverTaskTimeoutSeconds :: Int
  }

data RunSummary = RunSummary
  { summaryPlanned :: Int,
    summaryTriggerFailures :: Int,
    summaryExecuted :: Int,
    summarySucceeded :: Int,
    summaryWaiting :: Int,
    summaryFailed :: Int,
    summaryUnknown :: Int
  }
  deriving (Eq, Show)

instance ToJSON RunSummary where
  toJSON summary =
    object
      [ "planned" .= summaryPlanned summary,
        "trigger_failures" .= summaryTriggerFailures summary,
        "executed" .= summaryExecuted summary,
        "succeeded" .= summarySucceeded summary,
        "waiting" .= summaryWaiting summary,
        "failed" .= summaryFailed summary,
        "outcome_unknown" .= summaryUnknown summary
      ]

instance Semigroup RunSummary where
  left <> right =
    RunSummary
      { summaryPlanned = summaryPlanned left + summaryPlanned right,
        summaryTriggerFailures = summaryTriggerFailures left + summaryTriggerFailures right,
        summaryExecuted = summaryExecuted left + summaryExecuted right,
        summarySucceeded = summarySucceeded left + summarySucceeded right,
        summaryWaiting = summaryWaiting left + summaryWaiting right,
        summaryFailed = summaryFailed left + summaryFailed right,
        summaryUnknown = summaryUnknown left + summaryUnknown right
      }

instance Monoid RunSummary where
  mempty = RunSummary 0 0 0 0 0 0 0

systemClock :: Clock
systemClock = Clock getCurrentTime sleepUntil
  where
    sleepUntil wake = do
      now <- getCurrentTime
      let remaining = diffUTCTime wake now
      when (remaining > 0) $ do
        let microseconds = min 60000000 (floor (remaining * 1000000))
        threadDelay microseconds
        sleepUntil wake

processRunner :: Runner
processRunner = Runner runTaskProcess dispatchPluginProcess

defaultDriverEnvironment :: DriverEnvironment
defaultDriverEnvironment =
  DriverEnvironment
    { driverClock = systemClock,
      driverRunner = processRunner,
      driverLeaseSeconds = 7200,
      driverRetrySeconds = 30,
      driverMaximumAttempts = 3,
      driverCatchupLimit = 100,
      driverTaskTimeoutSeconds = 1800
    }

discoverWorkspaceRoot :: FilePath -> IO FilePath
discoverWorkspaceRoot start = do
  canonical <- canonicalizePath start
  isFile <- doesFileExist canonical
  search (if isFile then takeDirectory canonical else canonical)
  where
    search current = do
      initialized <- doesFileExist (current </> ".tactus" </> "tactus.toml")
      if initialized
        then pure current
        else do
          let parent = takeDirectory current
          if parent == current
            then throwIO (WorkspaceNotInitialized start)
            else search parent

initialiseWorkspace :: FilePath -> IO SegnoPaths
initialiseWorkspace start = initialiseWorkspaceWithSdk start Nothing

initialiseWorkspaceWithSdk :: FilePath -> Maybe FilePath -> IO SegnoPaths
initialiseWorkspaceWithSdk start configuredSdk = do
  root <- discoverWorkspaceRoot start
  let paths = segnoPaths root
  initialiseStore paths
  registerBuiltinPlugins root
  registerSegnoPackage root configuredSdk
  pure paths

installJob :: DriverEnvironment -> FilePath -> FilePath -> IO InstalledJob
installJob environment start script = do
  paths <- initialiseWorkspace start
  absoluteScript <- canonicalizePath (if isAbsolute script then script else pathsRoot paths </> script)
  exists <- doesFileExist absoluteScript
  unless exists (throwIO (InvalidJob "job script does not exist"))
  let relativeScript = normalise (makeRelative (pathsRoot paths) absoluteScript)
  when (outsideWorkspace relativeScript) (throwIO (InvalidJob "job script must be inside the Tactus workspace"))
  manifest <- withExchange paths "install" $ \exchange -> do
    let resultPath = exchange </> "result.json"
    outcome <- runnerTask (driverRunner environment) (pathsRoot paths) absoluteScript "describe" Nothing resultPath (driverTaskTimeoutSeconds environment)
    unless (processExitCode outcome == ExitSuccess) $
      throwIO (ExternalCommandFailed (renderProcessFailure "tactus describe" outcome))
    readJsonFile resultPath
  validateJobIdentifier (manifestTask manifest)
  let installed = InstalledJob relativeScript manifest
      destination = pathsJobs paths </> Text.unpack (manifestTask manifest) <> ".json"
  atomicWriteJson destination installed
  pure installed

listJobs :: FilePath -> IO [InstalledJob]
listJobs start = do
  paths <- initialiseWorkspace start
  entries <- listDirectory (pathsJobs paths)
  jobs <- forM [pathsJobs paths </> entry | entry <- entries, takeExtension entry == ".json"] $ \path -> do
    modified <- getModificationTime path
    job <- readJsonFile path
    pure (modified, job)
  pure (fmap snd (sortOn fst jobs))

runOnce :: FilePath -> IO RunSummary
runOnce = runOnceWith defaultDriverEnvironment

runOnceWith :: DriverEnvironment -> FilePath -> IO RunSummary
runOnceWith environment start = do
  paths <- initialiseWorkspace start
  now <- clockNow (driverClock environment)
  _ <- recoverExpiredLeases paths now
  jobs <- listJobs (pathsRoot paths)
  planning <- mconcat <$> mapM (pollJob environment paths now) jobs
  executed <- drainRunnable environment paths (Map.fromList [(manifestTask (installedManifest job), job) | job <- jobs])
  pure (planning <> executed)

runDriver :: FilePath -> NominalDiffTime -> IO ()
runDriver = runDriverWith defaultDriverEnvironment

runDriverWith :: DriverEnvironment -> FilePath -> NominalDiffTime -> IO ()
runDriverWith environment start fallbackSeconds = do
  paths <- initialiseWorkspace start
  let loop = do
        _ <- runOnceWith environment (pathsRoot paths)
        now <- clockNow (driverClock environment)
        storedWake <- nextLifecycleWake paths
        let fallback = addUTCTime fallbackSeconds now
            requestedWake = maybe fallback (min fallback) storedWake
            -- A malformed plugin or a stale persisted wake must never turn the
            -- single-node driver into a tight spawn/poll loop.
            wake = max (addUTCTime 1 now) requestedWake
        clockSleepUntil (driverClock environment) wake
        loop
  loop

pollJob :: DriverEnvironment -> SegnoPaths -> UTCTime -> InstalledJob -> IO RunSummary
pollJob environment paths now job = mconcat <$> mapM pollIsolated (manifestSources manifest)
  where
    manifest = installedManifest job
    taskIdentity = manifestTask manifest

    pollIsolated source = do
      outcome <- try (pollSource source)
      case outcome of
        Left (exception :: SomeException) -> do
          hPutStrLn stderr ("segno: trigger " <> Text.unpack taskIdentity <> "/" <> Text.unpack (sourceId source) <> " failed: " <> displayException exception)
          pure (mempty {summaryTriggerFailures = 1})
        Right planned -> pure (mempty {summaryPlanned = planned})

    pollSource source = do
      cursor <- loadTriggerCursor paths taskIdentity (sourceId source)
      let params =
            KeyMap.fromList
              [ "workspace" .= pathsRoot paths,
                "source_id" .= sourceId source,
                "config" .= sourceConfig source,
                "cursor" .= cursor,
                "now" .= now,
                "limit" .= driverCatchupLimit environment
              ]
      responseValue <- callPluginOrThrow environment paths (sourcePlugin source) "poll" params
      result <- decodeValue "trigger poll result" responseValue
      validatePollResult (driverCatchupLimit environment) now result
      inserted <- forM (pollOccurrences result) $ \planned -> do
        let occurrence = makeOccurrence taskIdentity source now planned
        created <- insertOccurrence paths taskIdentity occurrence now
        let acknowledgeParams =
              KeyMap.fromList
                [ "workspace" .= pathsRoot paths,
                  "source_id" .= sourceId source,
                  "occurrence_id" .= occurrenceId occurrence,
                  "idempotency_key" .= occurrenceIdempotencyKey occurrence,
                  "cursor" .= occurrenceCursor occurrence
                ]
        _ <- callPluginOrThrow environment paths (sourcePlugin source) "acknowledge" acknowledgeParams
        pure (if created then 1 else 0)
      let latestCursor = plannedCursor <$> safeLast (pollOccurrences result)
          nextCursor = latestCursor <|> cursor
      saveTriggerCursor paths taskIdentity (sourceId source) nextCursor (pollNextWake result) now
      pure (sum inserted)

validatePollResult :: Int -> UTCTime -> PollResult -> IO ()
validatePollResult limit now result = do
  let occurrences = pollOccurrences result
      keys = fmap plannedIdempotencyKey occurrences
      logicalTimes = fmap plannedLogicalTime occurrences
  when (length occurrences > limit) $
    throwIO (DriverProtocolError "trigger poll returned more occurrences than the requested limit")
  forM_ occurrences $ \planned -> do
    let key = plannedIdempotencyKey planned
    when (Text.null key || Text.length key > 512) $
      throwIO (DriverProtocolError "trigger idempotency keys must contain 1..512 characters")
    when (Text.any (\character -> character < ' ' || character == '\DEL') key) $
      throwIO (DriverProtocolError "trigger idempotency keys must not contain control characters")
    when (plannedLogicalTime planned > now) $
      throwIO (DriverProtocolError "trigger poll returned an occurrence whose logical time is in the future")
  when (Set.size (Set.fromList keys) /= length keys) $
    throwIO (DriverProtocolError "trigger poll returned duplicate idempotency keys")
  unless (logicalTimes == sort logicalTimes) $
    throwIO (DriverProtocolError "trigger poll occurrences must be ordered by logical time")
  when (length occurrences < limit) $
    forM_ (pollNextWake result) $ \nextWake ->
      when (nextWake <= now) $
        throwIO (DriverProtocolError "trigger next_wake must be later than now when the poll did not fill its limit")

drainRunnable :: DriverEnvironment -> SegnoPaths -> Map.Map Text InstalledJob -> IO RunSummary
drainRunnable environment paths jobs = go mempty
  where
    go summary = do
      now <- clockNow (driverClock environment)
      let claimLeaseSeconds = max 1 (min 300 (driverLeaseSeconds environment))
      claimed <- claimNextOccurrence paths now (addUTCTime claimLeaseSeconds now)
      case claimed of
        Nothing -> pure summary
        Just occurrence -> do
          let taskBudget = fromIntegral (max 1 (driverTaskTimeoutSeconds environment))
              runningLeaseSeconds = max (driverLeaseSeconds environment) (taskBudget * 2 + 120)
          running <-
            markOccurrenceRunning
              paths
              (occurrenceId (claimedOccurrence occurrence))
              (claimedFencingToken occurrence)
              (addUTCTime runningLeaseSeconds now)
              now
          result <-
            if not running
              then pure (mempty {summaryExecuted = 1, summaryUnknown = 1})
              else case Map.lookup (claimedJobId occurrence) jobs of
                Nothing -> do
                  _ <- markOccurrenceUnknown paths (occurrenceId (claimedOccurrence occurrence)) (claimedFencingToken occurrence) "installed job definition is missing" now
                  pure (mempty {summaryExecuted = 1, summaryUnknown = 1})
                Just job -> executeClaimed environment paths job occurrence
          go (summary <> result)

executeClaimed :: DriverEnvironment -> SegnoPaths -> InstalledJob -> ClaimedOccurrence -> IO RunSummary
executeClaimed environment paths job claimed = do
  startedAt <- clockNow (driverClock environment)
  snapshotResult <- loadState environment paths (manifestState manifest)
  case snapshotResult of
    Left failure -> infrastructureFailure ("state load failed: " <> pluginFailureMessage failure) startedAt
    Right snapshot -> withExchange paths (manifestTask manifest <> "-" <> Text.pack (show (claimedAttempt claimed))) $ \exchange -> do
      let invocationPath = exchange </> "invocation.json"
          resultPath = exchange </> "result.json"
          invocation =
            Invocation
              { invocationTask = manifestTask manifest,
                invocationAttempt = claimedAttempt claimed,
                invocationFencingToken = claimedFencingToken claimed,
                invocationTrigger = claimedOccurrence claimed,
                invocationState = snapshot
              }
      atomicWriteJson invocationPath invocation
      outcome <-
        try $
          runnerTask
            (driverRunner environment)
            (pathsRoot paths)
            (pathsRoot paths </> installedScript job)
            "execute"
            (Just invocationPath)
            resultPath
            (driverTaskTimeoutSeconds environment)
      finishedAt <- clockNow (driverClock environment)
      case outcome of
        Left (exception :: SomeException) -> unknown ("tactus execution outcome is unknown: " <> Text.pack (displayException exception)) finishedAt
        Right processOutcome | processExitCode processOutcome /= ExitSuccess -> unknown (renderProcessFailure "tactus run" processOutcome) finishedAt
        Right _ -> do
          resultExists <- doesFileExist resultPath
          if not resultExists
            then unknown "workflow exited successfully without atomically publishing SEGNO_RESULT_PATH" finishedAt
            else do
              decoded <- try (readJsonFile resultPath)
              case decoded of
                Left (exception :: SomeException) -> unknown (Text.pack (displayException exception)) finishedAt
                Right taskResult
                  | resultTask taskResult /= manifestTask manifest -> unknown "task result identity mismatch" finishedAt
                  | resultOccurrenceId taskResult /= occurrenceId (claimedOccurrence claimed) -> unknown "task result occurrence mismatch" finishedAt
                  | otherwise -> applyDecision finishedAt taskResult
  where
    manifest = installedManifest job
    occurrenceIdentity = occurrenceId (claimedOccurrence claimed)
    fence = claimedFencingToken claimed

    infrastructureFailure message now
      | fromIntegral (claimedAttempt claimed) < driverMaximumAttempts environment = do
          _ <- markOccurrenceWaiting paths occurrenceIdentity fence message (addUTCTime (driverRetrySeconds environment) now) now
          pure (mempty {summaryExecuted = 1, summaryWaiting = 1})
      | otherwise = do
          _ <- markOccurrenceFailed paths occurrenceIdentity fence message Nothing now
          pure (mempty {summaryExecuted = 1, summaryFailed = 1})

    unknown message now = do
      _ <- markOccurrenceUnknown paths occurrenceIdentity fence message now
      pure (mempty {summaryExecuted = 1, summaryUnknown = 1})

    applyDecision now result = case resultDecision result of
      IgnoreDecision -> succeed now (object ["decision" .= ("ignore" :: Text)])
      CompleteDecision transition output -> do
        applied <- applyTransition environment paths manifest claimed "complete" transition
        case applied of
          TransitionConflict -> definiteConflict now
          TransitionUncertain message -> unknown message now
          TransitionApplied -> succeed now output
      RetryDecision transition afterMilliseconds reason -> do
        applied <- applyTransition environment paths manifest claimed "retry" transition
        case applied of
          TransitionConflict -> definiteConflict now
          TransitionUncertain message -> unknown message now
          TransitionApplied
            | fromIntegral (claimedAttempt claimed) >= driverMaximumAttempts environment -> do
                _ <- markOccurrenceFailed paths occurrenceIdentity fence (reason <> "; maximum attempts reached") Nothing now
                pure (mempty {summaryExecuted = 1, summaryFailed = 1})
            | otherwise -> do
                let safeDelay = max 1 afterMilliseconds
                    retryAt = addUTCTime (fromRational (toRational safeDelay / 1000)) now
                _ <- markOccurrenceWaiting paths occurrenceIdentity fence reason retryAt now
                pure (mempty {summaryExecuted = 1, summaryWaiting = 1})
      FailDecision failure -> do
        _ <-
          markOccurrenceFailedWithDetails
            paths
            occurrenceIdentity
            fence
            (pluginFailureCode failure <> ": " <> pluginFailureMessage failure)
            (pluginFailureDetails failure)
            now
        pure (mempty {summaryExecuted = 1, summaryFailed = 1})

    succeed now output = do
      transitioned <- markOccurrenceSucceeded paths occurrenceIdentity fence output now
      if transitioned
        then pure (mempty {summaryExecuted = 1, summarySucceeded = 1})
        else unknown "fencing token became stale before final lifecycle commit" now

    definiteConflict now
      | fromIntegral (claimedAttempt claimed) < driverMaximumAttempts environment = do
          _ <- markOccurrenceWaiting paths occurrenceIdentity fence "business state compare-and-set conflict" (addUTCTime (driverRetrySeconds environment) now) now
          pure (mempty {summaryExecuted = 1, summaryWaiting = 1})
      | otherwise = do
          _ <- markOccurrenceFailed paths occurrenceIdentity fence "business state compare-and-set conflict; maximum attempts reached" Nothing now
          pure (mempty {summaryExecuted = 1, summaryFailed = 1})

data TransitionOutcome
  = TransitionApplied
  | TransitionConflict
  | TransitionUncertain Text

applyTransition :: DriverEnvironment -> SegnoPaths -> TaskManifest -> ClaimedOccurrence -> Text -> Transition -> IO TransitionOutcome
applyTransition _ _ _ _ _ (KeepTransition _) = pure TransitionApplied
applyTransition environment paths manifest claimed label (SetTransition expected schemaVersion value) = do
  let backend = stateBackend (manifestState manifest)
      operationIdentity =
        occurrenceId (claimedOccurrence claimed)
          <> ":final:"
          <> Text.pack (show (claimedAttempt claimed))
          <> ":"
          <> label
      params =
        KeyMap.fromList
          [ "workspace" .= pathsRoot paths,
            "state_key" .= stateKey (manifestState manifest),
            "expected_revision" .= expected,
            "schema_version" .= schemaVersion,
            "value" .= value,
            "conflict" .= ("compare-and-set" :: Text),
            "operation_id" .= operationIdentity,
            "occurrence_id" .= occurrenceId (claimedOccurrence claimed),
            "fencing_token" .= claimedFencingToken claimed,
            "fencing_epoch" .= claimedAttempt claimed
          ]
  responseResult <- try (runnerPlugin (driverRunner environment) (pathsRoot paths) backend "compare-and-set" params)
  pure $ case responseResult of
    Left (exception :: SomeException) -> TransitionUncertain ("state transition transport outcome is unknown: " <> Text.pack (displayException exception))
    Right (Left failure) -> TransitionUncertain ("state transition outcome is unknown: " <> pluginFailureCode failure <> ": " <> pluginFailureMessage failure)
    Right (Right responseValue) -> case parseEither parseJSON responseValue of
      Left message -> TransitionUncertain ("state transition response was invalid: " <> Text.pack message)
      Right (StateCasApplied _) -> TransitionApplied
      Right (StateCasConflict _) -> TransitionConflict

loadState :: DriverEnvironment -> SegnoPaths -> StateManifest -> IO (Either PluginFailure StateSnapshot)
loadState environment paths manifest = do
  let params =
        KeyMap.fromList
          [ "workspace" .= pathsRoot paths,
            "state_key" .= stateKey manifest,
            "schema_version" .= stateSchemaVersion manifest,
            "initial" .= stateInitial manifest
          ]
  responseResult <- try (runnerPlugin (driverRunner environment) (pathsRoot paths) (stateBackend manifest) "load" params)
  pure $ case responseResult of
    Left (exception :: SomeException) -> Left (PluginFailure "state_transport_failed" (Text.pack (displayException exception)) Nothing)
    Right response -> response >>= \responseValue -> case parseEither parseJSON responseValue of
      Left message -> Left (PluginFailure "invalid_state_response" (Text.pack message) Nothing)
      Right snapshot
        | snapshotKey snapshot /= stateKey manifest -> Left (PluginFailure "invalid_state_response" "state backend returned another state key" Nothing)
        | otherwise -> Right snapshot

callPluginOrThrow :: DriverEnvironment -> SegnoPaths -> Text -> Text -> Object -> IO Value
callPluginOrThrow environment paths pluginName method params = do
  responseResult <- try (runnerPlugin (driverRunner environment) (pathsRoot paths) pluginName method params)
  case responseResult of
    Left (exception :: SomeException) -> throwIO (DriverProtocolError (pluginName <> "/" <> method <> " transport failed: " <> Text.pack (displayException exception)))
    Right (Left failure) -> throwIO (DriverProtocolError (pluginName <> "/" <> method <> ": " <> pluginFailureMessage failure))
    Right (Right responseValue) -> pure responseValue

dispatchPluginProcess :: FilePath -> Text -> Text -> Object -> IO (Either PluginFailure Value)
dispatchPluginProcess root pluginName method params = do
  requestId <- freshRequestId
  let request = ClefProtocol.PluginRequest requestId method params
      input = LazyByteString.toStrict (ClefProtocol.encodePluginRequest request) <> "\n"
      command =
        proc
          "tactus"
          [ "dispatch",
            "--namespace",
            "plugin",
            "--name",
            Text.unpack pluginName,
            "--root",
            root,
            "--timeout-seconds",
            "60"
          ]
  outcome <- runProcessBytes command input
  if processOutputTruncated outcome
    then pure . Left $ PluginFailure "plugin_output_limit" "plugin output exceeded the 4 MiB transport limit" Nothing
    else case decodeUtf8' (processStdout outcome) of
      Left _ -> pure (Left (PluginFailure "plugin_protocol_failed" "plugin stdout was not UTF-8" Nothing))
      Right output -> case ClefProtocol.parsePluginOutput pluginName requestId output of
        Left failure ->
          pure . Left $
            PluginFailure
              (if processExitCode outcome == ExitSuccess then "plugin_protocol_failed" else "plugin_process_failed")
              (if processExitCode outcome == ExitSuccess then Text.pack (show failure) else renderProcessFailure pluginName outcome)
              Nothing
        Right parsed -> case ClefProtocol.pluginOutputTerminal parsed of
          ClefProtocol.PluginSucceeded value -> pure (Right value)
          ClefProtocol.PluginFailed failure ->
            pure . Left $
              PluginFailure
                (ClefProtocol.pluginFailureCode failure)
                (ClefProtocol.pluginFailureMessage failure)
                (ClefProtocol.pluginFailureDetails failure)

runTaskProcess :: FilePath -> FilePath -> Text -> Maybe FilePath -> FilePath -> Int -> IO ProcessOutcome
runTaskProcess root script mode invocationPath resultPath timeoutSeconds = do
  inherited <- getEnvironment
  let overrides =
        [ ("SEGNO_MODE", Text.unpack mode),
          ("SEGNO_RESULT_PATH", resultPath)
        ]
          <> maybe [] (\path -> [("SEGNO_INVOCATION_PATH", path)]) invocationPath
      environment = mergeEnvironment inherited overrides
      command =
        ( proc
            "tactus"
            [ "run",
              "--package",
              "segno-flow",
              "--script",
              script,
              "--root",
              root,
              "--timeout-seconds",
              show (max 1 timeoutSeconds)
            ]
        )
          { env = Just environment
          }
  runProcessBytes command ByteString.empty

runProcessBytes :: CreateProcess -> ByteString.ByteString -> IO ProcessOutcome
runProcessBytes command input =
  withCreateProcess
    command
      { std_in = CreatePipe,
        std_out = CreatePipe,
        std_err = CreatePipe
      }
    $ \maybeInput maybeOutput maybeError processHandle -> case (maybeInput, maybeOutput, maybeError) of
      (Just inputHandle, Just outputHandle, Just errorHandle) -> do
        forM_ [inputHandle, outputHandle, errorHandle] $ \handle -> do
          hSetBinaryMode handle True
          hSetBuffering handle NoBuffering
        outputReader <- async (readProcessOutput processHandle outputHandle)
        errorReader <- async (readProcessOutput processHandle errorHandle)
        ByteString.hPut inputHandle input
        hClose inputHandle
        exitCode <- waitForProcess processHandle
        (output, outputTruncated) <- wait outputReader
        (errors, errorTruncated) <- wait errorReader
        pure (ProcessOutcome exitCode output errors (outputTruncated || errorTruncated))
      _ -> throwIO (ExternalCommandFailed "failed to create binary pipes for child process")

readProcessOutput :: ProcessHandle -> Handle -> IO (ByteString.ByteString, Bool)
readProcessOutput processHandle handle = go ByteString.empty False
  where
    maximumBytes = 4 * 1024 * 1024
    go retained truncated = do
      chunk <- ByteString.hGetSome handle 65536
      if ByteString.null chunk
        then pure (retained, truncated)
        else
          if truncated
            then go retained True
            else do
              let remaining = maximumBytes - ByteString.length retained
              if ByteString.length chunk <= remaining
                then go (retained <> chunk) False
                else do
                  terminateProcess processHandle
                  go (retained <> ByteString.take (max 0 remaining) chunk) True

makeOccurrence :: Text -> SourceManifest -> UTCTime -> PlannedOccurrence -> TriggerOccurrence
makeOccurrence taskIdentity source observed planned =
  TriggerOccurrence
    { occurrenceTriggerId = sourceId source,
      occurrenceId = encodeOccurrenceId taskIdentity (sourceId source) (plannedIdempotencyKey planned),
      occurrenceLogicalTime = plannedLogicalTime planned,
      occurrenceObservedTime = observed,
      occurrenceCursor = plannedCursor planned,
      occurrenceIdempotencyKey = plannedIdempotencyKey planned,
      occurrencePayload = plannedPayload planned
    }

encodeOccurrenceId :: Text -> Text -> Text -> Text
encodeOccurrenceId taskIdentity triggerIdentity idempotency =
  "occ:" <> hexDigest (SHA256.hash encodedIdentity)
  where
    encodedIdentity =
      TextEncoding.encodeUtf8 $
        component taskIdentity <> component triggerIdentity <> component idempotency
    component value = Text.pack (show (Text.length value)) <> ":" <> value

hexDigest :: ByteString.ByteString -> Text
hexDigest = Text.pack . concatMap encodeByte . ByteString.unpack
  where
    encodeByte byte =
      let digits = "0123456789abcdef"
          high = fromIntegral byte `div` 16
          low = fromIntegral byte `mod` 16
       in [digits !! high, digits !! low]

registerBuiltinPlugins :: FilePath -> IO ()
registerBuiltinPlugins root = do
  let configPath = root </> ".tactus" </> "tactus.toml"
  existing <- TextIO.readFile configPath
  executable <- canonicalizePath =<< getExecutablePath
  let definition name arguments =
        "[plugins.\""
          <> name
          <> "\"]\ncommand = "
          <> encodeCompactStringArray (Text.pack executable : arguments)
          <> "\n"
      definitions =
        [ ("time.interval", definition "time.interval" ["time-plugin", "interval"]),
          ("time.cron", definition "time.cron" ["time-plugin", "cron"]),
          ("segno.state", definition "segno.state" ["state-plugin"]),
          ("system.active-window", definition "system.active-window" ["active-window-plugin"])
        ]
      missing = [configuredDefinition | (name, configuredDefinition) <- definitions, not (Text.isInfixOf ("\"" <> name <> "\"") existing)]
  unless (null missing) $ TextIO.appendFile configPath ("\n" <> Text.intercalate "\n" missing)

registerSegnoPackage :: FilePath -> Maybe FilePath -> IO ()
registerSegnoPackage root configuredSdk = do
  let projectPath = root </> ".tactus" </> "cabal.project"
  existing <- TextIO.readFile projectPath
  alreadyConfigured <- firstExistingPackage (projectPackageCandidates (takeDirectory projectPath) existing)
  case alreadyConfigured of
    Just _ -> pure ()
    Nothing -> do
      packagePath <- locateSegnoPackage configuredSdk (takeDirectory projectPath) existing
      canonical <- canonicalizePath packagePath
      let encoded = Text.pack canonical
      TextIO.writeFile projectPath (addPackageEntry existing (quoteCabalPath encoded))

locateSegnoPackage :: Maybe FilePath -> FilePath -> Text -> IO FilePath
locateSegnoPackage configuredSdk projectDirectory project = do
  configured <- lookupEnv "SEGNO_FLOW_SDK"
  executable <- getExecutablePath
  current <- getCurrentDirectory
  let siblingCandidates =
        [ takeDirectory clefPath </> "segno-flow"
          | clefPath <- projectNamedPackageCandidates "clef-sdk" projectDirectory project
        ]
      candidates =
        catMaybes [configuredSdk, configured]
          <> siblingCandidates
          <> [ current </> "segno-flow",
               takeDirectory executable </> "segno-flow",
               takeDirectory (takeDirectory executable) </> "segno-flow"
             ]
  found <- firstExistingPackage candidates
  maybe
    (throwIO (InvalidJob "cannot locate segno-flow package; pass `segno init --sdk PATH` or set SEGNO_FLOW_SDK"))
    pure
    found

addPackageEntry :: Text -> Text -> Text
addPackageEntry project packagePath =
  Text.unlines $ case break (Text.isPrefixOf "packages:" . Text.stripStart) (Text.lines project) of
    (before, []) -> before <> ["packages: " <> packagePath]
    (before, declaration : after) ->
      let inlineValue = Text.strip (Text.drop 1 (Text.dropWhile (/= ':') declaration))
          replacement
            | Text.null inlineValue =
                let (existingPackages, remainingFields) = span isContinuation after
                 in [declaration] <> existingPackages <> ["  " <> packagePath] <> remainingFields
            | otherwise = ["packages:", "  " <> inlineValue, "  " <> packagePath]
       in if Text.null inlineValue then before <> replacement else before <> replacement <> after
  where
    isContinuation line = Text.null line || maybe False (`elem` [' ', '\t']) (Text.find (const True) line)

projectPackageCandidates :: FilePath -> Text -> [FilePath]
projectPackageCandidates = projectNamedPackageCandidates "segno-flow"

projectNamedPackageCandidates :: Text -> FilePath -> Text -> [FilePath]
projectNamedPackageCandidates packageName projectDirectory project =
  [ resolve (stripPathQuotes (Text.unpack candidate))
    | line <- Text.lines project,
      let stripped = Text.strip line,
      let candidate =
            if Text.isPrefixOf "packages:" stripped
              then Text.strip (Text.drop (Text.length ("packages:" :: Text)) stripped)
              else stripped,
      Text.isInfixOf packageName candidate,
      not (Text.null candidate)
  ]
  where
    resolve path = if isAbsolute path then path else projectDirectory </> path

stripPathQuotes :: FilePath -> FilePath
stripPathQuotes path = case (decodeJsonText (Text.pack path) :: Either String Text) of
  Right decoded -> Text.unpack decoded
  Left _ -> case path of
    '"' : remainder -> case reverse remainder of
      '"' : reversedValue -> reverse reversedValue
      _ -> path
    _ -> path

quoteCabalPath :: Text -> Text
quoteCabalPath = decodeUtf8With lenientDecode . LazyByteString.toStrict . encode

encodeCompactStringArray :: [Text] -> Text
encodeCompactStringArray = decodeUtf8With lenientDecode . LazyByteString.toStrict . encode

firstExistingPackage :: [FilePath] -> IO (Maybe FilePath)
firstExistingPackage [] = pure Nothing
firstExistingPackage (candidate : remaining) = do
  exists <- doesFileExist (candidate </> "segno-flow.cabal")
  if exists then pure (Just candidate) else firstExistingPackage remaining

allocateExchange :: SegnoPaths -> Text -> IO FilePath
allocateExchange paths label = do
  now <- getCurrentTime
  let base = sanitizeName label <> "-" <> formatTime defaultTimeLocale "%Y%m%d%H%M%S%q" now
      directory = pathsTriggers paths </> "exchanges"
  createDirectoryIfMissing True directory
  choose directory base (0 :: Int)
  where
    choose directory base ordinal = do
      when (ordinal >= 1000) (throwIO (ExternalCommandFailed "could not allocate a unique Segno exchange directory"))
      let suffix = if ordinal == 0 then "" else "-" <> show ordinal
          candidate = directory </> base <> suffix
      created <- try (createDirectory candidate)
      case created of
        Right () -> pure candidate
        Left exception
          | isAlreadyExistsError exception -> choose directory base (ordinal + 1)
          | otherwise -> throwIO (exception :: IOException)

withExchange :: SegnoPaths -> Text -> (FilePath -> IO value) -> IO value
withExchange paths label action =
  bracket
    (allocateExchange paths label)
    (\directory -> removeDirectoryRecursive directory `catch` (\(_ :: IOException) -> pure ()))
    action

atomicWriteJson :: ToJSON value => FilePath -> value -> IO ()
atomicWriteJson destination value = mask_ $ do
  createDirectoryIfMissing True (takeDirectory destination)
  (temporary, handle) <- openBinaryTempFile (takeDirectory destination) ".segno-write.tmp"
  let cleanup = (hClose handle `catch` ignoreIo) >> (removeIfExists temporary)
      publish = do
        hSetBinaryMode handle True
        ByteString.hPut handle (LazyByteString.toStrict (encode value) <> "\n")
        hClose handle
        renameFile temporary destination
  publish `onException` cleanup
  where
    ignoreIo (_ :: IOException) = pure ()

removeIfExists :: FilePath -> IO ()
removeIfExists path = do
  exists <- doesFileExist path
  when exists (removeFile path `catch` (\(_ :: IOException) -> pure ()))

readJsonFile :: FromJSON value => FilePath -> IO value
readJsonFile path = do
  encoded <- withBinaryFile path ReadMode (\handle -> ByteString.hGet handle (4 * 1024 * 1024 + 1))
  when (ByteString.length encoded > 4 * 1024 * 1024) $
    throwIO (DriverProtocolError "JSON document exceeded 4 MiB")
  case decodeUtf8' encoded of
    Left _ -> throwIO (DriverProtocolError "JSON document was not UTF-8")
    Right text -> case decodeJsonText text of
      Left message -> throwIO (DriverProtocolError (Text.pack message))
      Right value -> pure value

decodeValue :: FromJSON value => Text -> Value -> IO value
decodeValue label value = case parseEither parseJSON value of
  Left message -> throwIO (DriverProtocolError (label <> ": " <> Text.pack message))
  Right decoded -> pure decoded

freshRequestId :: IO Text
freshRequestId = Text.pack . formatTime defaultTimeLocale "segno-%Y%m%d%H%M%S%q" <$> getCurrentTime

renderProcessFailure :: Text -> ProcessOutcome -> Text
renderProcessFailure label outcome =
  label
    <> " failed with "
    <> Text.pack (show (processExitCode outcome))
    <> ": "
    <> decodeUtf8With lenientDecode (ByteString.take 16384 (processStderr outcome))

mergeEnvironment :: [(String, String)] -> [(String, String)] -> [(String, String)]
mergeEnvironment inherited overrides =
  Map.toList (foldr (uncurry Map.insert) (Map.fromList inherited) overrides)

outsideWorkspace :: FilePath -> Bool
outsideWorkspace relative = isAbsolute relative || ".." `elem` splitDirectories relative

validateJobIdentifier :: Text -> IO ()
validateJobIdentifier value =
  unless
    ( not (Text.null value)
        && Text.length value <= 128
        && value /= "."
        && value /= ".."
        && Text.all (\character -> isAlphaNum character || character `elem` ("._-" :: String)) value
    )
    (throwIO (InvalidJob "task id must use 1-128 letters, digits, dot, underscore, or hyphen"))

sanitizeName :: Text -> FilePath
sanitizeName = Text.unpack . Text.map (\character -> if isAlphaNum character || character `elem` ("_-" :: String) then character else '_')

safeLast :: [value] -> Maybe value
safeLast [] = Nothing
safeLast values = Just (last values)
