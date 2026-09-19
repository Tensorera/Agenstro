{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE ScopedTypeVariables #-}

module Main (main) where

import Control.Concurrent (threadDelay)
import Control.Concurrent.Async (AsyncCancelled (..))
import Control.Exception (AsyncException (ThreadKilled), IOException, SomeAsyncException, SomeException, bracket, catch, fromException, throwIO, toException, try)
import Control.Monad (forM_, unless, when)
import Data.Aeson
  ( FromJSON (parseJSON),
    ToJSON,
    Object,
    Value (..),
    encode,
    eitherDecode,
    object,
    toJSON,
    withObject,
    (.:),
    (.:?),
    (.=),
  )
import qualified Data.Aeson.KeyMap as KeyMap
import Data.Aeson.Types (Parser, parseEither)
import qualified Data.ByteString.Lazy as LazyByteString
import Data.IORef (IORef, modifyIORef', newIORef, readIORef, writeIORef)
import Data.Text (Text)
import qualified Data.Text as Text
import qualified Data.Text.IO as TextIO
import Data.Time (UTCTime, addUTCTime, getCurrentTime)
import Data.Word (Word32)
import Segno.Driver
  ( Clock (..),
    DriverEnvironment (..),
    ProcessOutcome (..),
    Runner (..),
    RunSummary (..),
    defaultDriverEnvironment,
    initialiseWorkspaceWithSdk,
    runOnceWith,
  )
import Segno.Lifecycle
  ( ClaimedOccurrence (..),
    Lifecycle (..),
    OccurrenceRecord (..),
  )
import Segno.Internal.Exception (trySynchronous)
import Segno.Protocol
  ( InstalledJob (..),
    PluginFailure (..),
    PluginRequest,
    PollResult (..),
    PlannedOccurrence (..),
    SourceManifest (..),
    StateCasResult (..),
    StateCasRequest (..),
    OperationId (..),
    OccurrenceId (..),
    FencingToken (..),
    FencingEpoch (..),
    FencingContext (..),
    StateManifest (..),
    StateSnapshot (..),
    TaskManifest (..),
    TriggerOccurrence (..),
    decodeJsonText,
    resultApi,
  )
import Segno.Store.SQLite
  ( CasResult (..),
    SegnoPaths (..),
    businessHistory,
    claimNextOccurrence,
    compareAndSetBusinessState,
    initialiseStore,
    insertOccurrence,
    lifecycleStatus,
    loadBusinessState,
    markOccurrenceRunning,
    markOccurrenceSucceeded,
    recoverExpiredLeases,
    segnoPaths,
  )
import Segno.Trigger.Time
  ( IntervalConfig (..),
    planTimeSource,
  )
import System.Directory
  ( canonicalizePath,
    createDirectory,
    createDirectoryIfMissing,
    doesFileExist,
    findExecutable,
    getCurrentDirectory,
    getTemporaryDirectory,
    listDirectory,
    removeDirectoryRecursive,
  )
import System.Exit (ExitCode (ExitFailure, ExitSuccess))
import System.Environment (getEnv, setEnv)
import System.Timeout (timeout)
import System.FilePath (makeRelative, searchPathSeparator, takeDirectory, (</>))
import System.Info (os)
import System.Process (readProcessWithExitCode)
import ServiceTests (runServiceTests)

main :: IO ()
main = do
  useRepositoryTactus
  run "strict plugin request ids" testStrictRequestIds
  run "strict trigger and state plugin payloads" testStrictPluginPayloads
  run "pure interval planning" testIntervalPlanning
  run "idempotency is scoped by job and trigger" testCrossJobIdempotency
  run "checkpoint operation id is scoped by occurrence" testCheckpointOperationScope
  run "stale fencing token cannot checkpoint after reclaim" testStaleFence
  run "expired running work becomes outcome unknown" testRunningLeaseExpiryUnknown
  run "relative Clef SDK discovers sibling Segno package" testRelativeSdkDiscovery
  run "plugin registration parses TOML and preserves custom commands" testPluginRegistration
  run "bad trigger does not starve a ready occurrence" testBadTriggerIsolation
  run "outcome unknown is not automatically retried" testOutcomeUnknownStops
  run "synchronous handlers preserve cancellation and timeout" testSynchronousBoundary
  run "driver cancellation propagates through every execution phase" testDriverCancellation
  run "virtual clock and fake runner execute one persistent occurrence" testVirtualDriver
  run "virtual minute task checkpoints three windows and survives restart" testVirtualWindowPersistence
  runServiceTests
  putStrLn "segno-flow: all tests passed"

run :: String -> IO () -> IO ()
run label test = do
  putStr ("[test] " <> label <> " ... ")
  test
  putStrLn "ok"

assert :: Bool -> String -> IO ()
assert condition message = unless condition (ioError (userError message))

-- Initialization crosses the same local Tactus configuration boundary as the
-- installed executable. Prefer this checkout's build over an older PATH copy.
useRepositoryTactus :: IO ()
useRepositoryTactus = do
  sdk <- locateSegnoSource
  let directory = takeDirectory sdk </> "Build" </> "cargo" </> "debug"
      executable = directory </> if os == "mingw32" then "tactus.exe" else "tactus"
  built <- doesFileExist executable
  if built
    then do
      inherited <- getEnv "PATH"
      setEnv "PATH" (directory <> [searchPathSeparator] <> inherited)
    else do
      installed <- findExecutable "tactus"
      when (installed == Nothing) (fail "Segno tests require current Tactus; run cargo build --workspace --locked first")

testPluginRegistration :: IO ()
testPluginRegistration = forM_ cases $ \(label, pluginConfig, custom) ->
  withTemporaryWorkspace ("registration-" <> label) $ \root -> do
    sdk <- locateSegnoSource
    createTactusWorkspace root sdk
    let configPath = root </> ".tactus" </> "tactus.toml"
    TextIO.writeFile configPath (testWorkspaceConfig pluginConfig)
    _ <- initialiseWorkspaceWithSdk root (Just sdk)
    configured <- TextIO.readFile configPath
    _ <- initialiseWorkspaceWithSdk root (Just sdk)
    repeated <- TextIO.readFile configPath
    assert (configured == repeated) "repeat registration rewrote the configuration"
    when custom $ assert (Text.isInfixOf "custom-command" configured) "custom plugin command was replaced"
    (status, output, errors) <- readProcessWithExitCode "tactus" ["runtime-json", "--root", root] ""
    assert (status == ExitSuccess) ("registered TOML was invalid: " <> errors)
    document <- either fail pure (decodeJsonText (Text.pack output) :: Either String Value)
    plugins <- either fail pure (parseEither (withObject "runtime" (.: "plugins")) document :: Either String Object)
    assert (all (`KeyMap.member` plugins) ["time.interval", "time.cron", "segno.state", "system.active-window"])
      "a comment, string, or alternate TOML key syntax hid a missing plugin"
  where
    cases =
      [ ("comment", "# Example: \"time.interval\"\n[plugins]\n", False),
        ("literal", "[plugins.'time.interval']\ncommand = ['custom-command']\n", True),
        ("dotted", "[plugins]\n\"time.interval\".command = ['custom-command']\n", True),
        ("escaped", "[plugins.\"time\\u002einterval\"]\ncommand = ['custom-command']\n", True),
        ("inline", "plugins = { 'time.interval' = { command = ['custom-command'] } }\n", True),
        ("string", "[plugins.extra]\ncommand = ['unused']\n[plugins.extra.options]\nnote = '''\"time.interval\"'''\n", False)
      ]

testSynchronousBoundary :: IO ()
testSynchronousBoundary = do
  ordinary <- trySynchronous (ioError (userError "ordinary failure")) :: IO (Either SomeException ())
  assert (isLeft ordinary) "synchronous failure was not captured"
  forM_ [toException ThreadKilled, toException AsyncCancelled] $ \cancellation -> do
    outcome <- try (trySynchronous (throwIO cancellation)) :: IO (Either SomeException (Either SomeException ()))
    case outcome of
      Left exception -> assert (isAsync exception) "cancellation changed exception type"
      Right _ -> fail "asynchronous cancellation was captured as a normal failure"
  timed <- timeout 1000 (trySynchronous (threadDelay 1000000))
  case timed of
    Nothing -> pure ()
    Just _ -> fail "System.Timeout cancellation was captured as a normal failure"
  where
    isAsync exception = case fromException exception :: Maybe SomeAsyncException of
      Just _ -> True
      Nothing -> False

testDriverCancellation :: IO ()
testDriverCancellation = forM_ ["poll", "load", "task", "compare-and-set", "cleanup-failure"] $ \phase ->
  withTemporaryWorkspace ("cancel-" <> Text.unpack phase) $ \root -> do
    sdk <- locateSegnoSource
    createTactusWorkspace root sdk
    paths <- initialiseWorkspaceWithSdk root (Just sdk)
    cleanupClockFailed <- newIORef False
    let now = read "2026-08-16 12:00:00 UTC" :: UTCTime
        manifest = TaskManifest "cancelled-task" [SourceManifest "quiet" "quiet.trigger" (object [])]
          (StateManifest "cancelled-state" 1 "segno.state" "compare-and-set" (object []))
        baseRunner = fakeRunner now
        runTask workspace script mode invocationPath resultPath context deadline
          | phase == "task" = throwIO ThreadKilled
          | phase == "cleanup-failure" = writeIORef cleanupClockFailed True >> throwIO ThreadKilled
          | otherwise = do
              outcome <- runnerTask baseRunner workspace script mode invocationPath resultPath context deadline
              path <- maybe (fail "missing invocation") pure invocationPath
              invocation <- readJsonValue path
              (task, identity) <- either fail pure (parseEither parseInvocationIdentity invocation)
              writeJson resultPath (object
                [ "api" .= resultApi, "task" .= task, "occurrence_id" .= identity,
                  "decision" .= object
                    [ "kind" .= ("complete" :: Text), "output" .= Null,
                      "state" .= object
                        [ "kind" .= ("set" :: Text), "expected_revision" .= ("0" :: Text),
                          "schema_version" .= (1 :: Int), "value" .= object []
                        ]
                    ]
                ])
              pure outcome
        callPlugin workspace plugin method params
          | method == phase = throwIO ThreadKilled
          | method == "poll" = pure (Right (toJSON (PollResult [] (Just (addUTCTime 60 now)))))
          | otherwise = runnerPlugin baseRunner workspace plugin method params
        environment = defaultDriverEnvironment
          { driverClock = Clock
              (do
                failed <- readIORef cleanupClockFailed
                if failed then ioError (userError "cleanup clock unavailable") else pure now)
              (\_ -> pure ()),
            driverRunner = Runner runTask callPlugin
          }
    writeJson (pathsJobs paths </> "cancelled-task.json") (InstalledJob "unused.hs" manifest)
    _ <- insertOccurrence paths "cancelled-task" (occurrence "cancelled-occurrence" "cancelled-key" now) now
    result <- try (runOnceWith environment root) :: IO (Either SomeException RunSummary)
    case result of
      Left exception -> case fromException exception of
        Just ThreadKilled -> pure ()
        _ -> fail ("driver changed cancellation into " <> show exception)
      Right _ -> fail ("driver swallowed cancellation during " <> Text.unpack phase)
    records <- lifecycleStatus paths (Just "cancelled-task")
    let expected = case phase of
          "poll" -> Ready
          "cleanup-failure" -> Running
          _ -> OutcomeUnknown
    assert (map recordLifecycle records == [expected]) "interrupted occurrence became retryable"
    when (phase == "cleanup-failure") $ do
      _ <- recoverExpiredLeases paths (addUTCTime 100000 now)
      recovered <- lifecycleStatus paths (Just "cancelled-task")
      assert (map recordLifecycle recovered == [OutcomeUnknown]) "failed cancellation cleanup was not recovered as unknown"

testStrictRequestIds :: IO ()
testStrictRequestIds = do
  let booleanId = decodeJsonText "{\"api\":\"agenstro.plugin/v1\",\"id\":true,\"method\":\"smoke\",\"params\":{}}" :: Either String PluginRequest
      duplicate = decodeJsonText "{\"api\":\"agenstro.plugin/v1\",\"id\":\"a\",\"id\":\"b\",\"method\":\"smoke\",\"params\":{}}" :: Either String PluginRequest
  assert (isLeft booleanId) "boolean plugin request id was accepted"
  assert (isLeft duplicate) "duplicate JSON key was accepted"

testStrictPluginPayloads :: IO ()
testStrictPluginPayloads = do
  let now = read "2026-08-16 12:00:00 UTC" :: UTCTime
      invalidLongKey =
        object
          [ "occurrences" .= [PlannedOccurrence now Null (Text.replicate 513 "x") Null],
            "next_wake" .= addUTCTime 60 now
          ]
      invalidControlKey =
        object
          [ "occurrences" .= [PlannedOccurrence now Null "minute\nkey" Null],
            "next_wake" .= addUTCTime 60 now
          ]
      missingAppliedRevision = object ["applied" .= True]
      contradictoryConflict = object ["applied" .= False, "revision" .= ("8" :: Text)]
      unknownCasField = object ["applied" .= True, "revision" .= ("8" :: Text), "extra" .= True]
      validApplied = object ["applied" .= True, "revision" .= ("8" :: Text)]
  assert (isLeft (parseEither parseJSON invalidLongKey :: Either String PollResult)) "overlong trigger idempotency key was accepted"
  assert (isLeft (parseEither parseJSON invalidControlKey :: Either String PollResult)) "control character in trigger idempotency key was accepted"
  assert (isLeft (parseEither parseJSON missingAppliedRevision :: Either String StateCasResult)) "applied CAS response without revision was accepted"
  assert (isLeft (parseEither parseJSON contradictoryConflict :: Either String StateCasResult)) "contradictory CAS response was accepted"
  assert (isLeft (parseEither parseJSON unknownCasField :: Either String StateCasResult)) "unknown CAS response field was accepted"
  assert (parseEither parseJSON validApplied == Right (StateCasApplied "8")) "valid applied CAS response was rejected"

testIntervalPlanning :: IO ()
testIntervalPlanning = do
  let now = read "2026-08-16 12:00:00 UTC" :: UTCTime
      config = toJSON (IntervalConfig 60000)
      first = planTimeSource "time.interval" "each-minute" config Nothing now 10
  case first of
    Left message -> ioError (userError (Text.unpack message))
    Right result -> do
      assert (length (pollOccurrences result) == 1) "first interval poll did not emit immediately"
      assert (pollNextWake result == Just (addUTCTime 60 now)) "interval next wake is wrong"
      cursor <- case pollOccurrences result of
        [onlyOccurrence] -> pure (plannedCursor onlyOccurrence)
        _ -> ioError (userError "unexpected first interval occurrence count")
      let later = planTimeSource "time.interval" "each-minute" config (Just cursor) (addUTCTime 180 now) 10
      case later of
        Left message -> ioError (userError (Text.unpack message))
        Right catchup -> assert (length (pollOccurrences catchup) == 3) "interval catch-up did not enumerate missed occurrences"

testCrossJobIdempotency :: IO ()
testCrossJobIdempotency = withTemporaryWorkspace "cross-job" $ \root -> do
  let paths = segnoPaths root
      now = read "2026-08-16 12:00:00 UTC" :: UTCTime
      first = occurrence "occ-job-a" "same-key" now
      second = occurrence "occ-job-b" "same-key" now
  initialiseStore paths
  insertedA <- insertOccurrence paths "job-a" first now
  insertedB <- insertOccurrence paths "job-b" second now
  records <- lifecycleStatus paths Nothing
  assert insertedA "first occurrence was not inserted"
  assert insertedB "second job was incorrectly deduplicated"
  assert (length records == 2) "cross-job occurrences did not coexist"

testCheckpointOperationScope :: IO ()
testCheckpointOperationScope = withTemporaryWorkspace "operation-scope" $ \root -> do
  let paths = segnoPaths root
      now = read "2026-08-16 12:00:00 UTC" :: UTCTime
      manifest = StateManifest "window-log" 1 "segno.state" "compare-and-set" (object ["count" .= (0 :: Int)])
  initialiseStore paths
  firstRevision <- checkpointOccurrence paths manifest now "occ-one" "key-one" Nothing 1
  secondRevision <- checkpointOccurrence paths manifest (addUTCTime 2 now) "occ-two" "key-two" (Just firstRevision) 2
  assert (firstRevision == "1") "first checkpoint revision was not 1"
  assert (secondRevision == "2") "static operation id collided across occurrences"

testStaleFence :: IO ()
testStaleFence = withTemporaryWorkspace "stale-fence" $ \root -> do
  let paths = segnoPaths root
      now = read "2026-08-16 12:00:00 UTC" :: UTCTime
      later = addUTCTime 2 now
      manifest = StateManifest "fenced-state" 1 "segno.state" "compare-and-set" (object [])
      candidate = occurrence "fenced-occurrence" "fenced-key" now
  initialiseStore paths
  _ <- insertOccurrence paths "fenced-job" candidate now
  firstClaim <- claimNextOccurrence paths now (addUTCTime 1 now) >>= maybe (fail "first claim missing") pure
  _ <- recoverExpiredLeases paths later
  secondClaim <- claimNextOccurrence paths later (addUTCTime 300 later) >>= maybe (fail "reclaim missing") pure
  _ <- markOccurrenceRunning paths "fenced-occurrence" (claimedFencingToken secondClaim) (addUTCTime 300 later) later
  snapshot <- loadBusinessState paths manifest later
  let request owner claimed = StateCasRequest
        { casStateKey = "fenced-state",
          casExpectedRevision = snapshotRevision snapshot,
          casSchemaVersion = 1,
          casValue = object ["owner" .= (owner :: Text)],
          casOperationId = OperationId "checkpoint",
          casFencing = FencingContext
            { fenceOccurrenceId = OccurrenceId "fenced-occurrence",
              fenceToken = FencingToken (claimedFencingToken claimed),
              fenceEpoch = FencingEpoch (claimedAttempt claimed)
            }
        }
  stale <-
    compareAndSetBusinessState paths (request "stale" firstClaim) later
  assert (isConflict stale) "expired fencing token changed business state"
  current <-
    compareAndSetBusinessState paths (request "current" secondClaim) later
  assert (current == CasApplied "1") "current fencing token could not checkpoint"
  decodedRequest <- either fail pure (parseEither parseJSON (toJSON (request "current" secondClaim)))
  replayed <- compareAndSetBusinessState paths decodedRequest later
  assert (replayed == current) "a serialized request lost its idempotent operation identity"
  changed <- compareAndSetBusinessState paths (decodedRequest {casValue = object ["owner" .= ("different" :: Text)]}) later
  assert (isConflict changed) "reused operation identity accepted a different state write"
  history <- businessHistory paths (Just "fenced-state") 10
  assert (length history == 1) "replayed or conflicting CAS added business history"

testRunningLeaseExpiryUnknown :: IO ()
testRunningLeaseExpiryUnknown = withTemporaryWorkspace "running-lease" $ \root -> do
  let paths = segnoPaths root
      now = read "2026-08-16 12:00:00 UTC" :: UTCTime
      later = addUTCTime 2 now
      candidate = occurrence "running-expired" "running-key" now
  initialiseStore paths
  _ <- insertOccurrence paths "running-job" candidate now
  claimed <- claimNextOccurrence paths now (addUTCTime 1 now) >>= maybe (fail "claim missing") pure
  running <- markOccurrenceRunning paths "running-expired" (claimedFencingToken claimed) (addUTCTime 1 now) now
  assert running "claim did not enter Running"
  recovered <- recoverExpiredLeases paths later
  records <- lifecycleStatus paths (Just "running-job")
  runnable <- claimNextOccurrence paths later (addUTCTime 300 later)
  assert (recovered == 1) "expired running occurrence was not recovered"
  assert (fmap recordLifecycle records == [OutcomeUnknown]) "expired running occurrence was made retryable"
  case runnable of
    Nothing -> pure ()
    Just _ -> fail "expired running occurrence was automatically reclaimed"

checkpointOccurrence :: SegnoPaths -> StateManifest -> UTCTime -> Text -> Text -> Maybe Text -> Int -> IO Text
checkpointOccurrence paths manifest now occurrenceIdentity idempotency expected count = do
  let candidate = occurrence occurrenceIdentity idempotency now
  _ <- insertOccurrence paths "window-task" candidate now
  claimed <- claimNextOccurrence paths now (addUTCTime 300 now)
  selected <- maybe (ioError (userError "occurrence was not claimed")) pure claimed
  running <- markOccurrenceRunning paths occurrenceIdentity (claimedFencingToken selected) (addUTCTime 300 now) now
  assert running "claim did not enter Running"
  snapshot <- loadBusinessState paths manifest now
  assert (snapshotRevision snapshot == (expected <|> Just "0")) "unexpected state revision before checkpoint"
  result <-
    compareAndSetBusinessState
      paths
      StateCasRequest
        { casStateKey = "window-log",
          casExpectedRevision = snapshotRevision snapshot,
          casSchemaVersion = 1,
          casValue = object ["count" .= count],
          casOperationId = OperationId "record-window",
          casFencing = FencingContext
            { fenceOccurrenceId = OccurrenceId occurrenceIdentity,
              fenceToken = FencingToken (claimedFencingToken selected),
              fenceEpoch = FencingEpoch (claimedAttempt selected)
            }
        }
      now
  revision <- case result of
    CasApplied value -> pure value
    CasConflict actual -> ioError (userError ("checkpoint conflicted: " <> show actual))
  _ <- markOccurrenceSucceeded paths occurrenceIdentity (claimedFencingToken selected) (object []) now
  pure revision

testVirtualDriver :: IO ()
testVirtualDriver = withTemporaryWorkspace "virtual-driver" $ \root -> do
  sdk <- locateSegnoSource
  createTactusWorkspace root sdk
  paths <- initialiseWorkspaceWithSdk root (Just sdk)
  let now = read "2026-08-16 12:00:00 UTC" :: UTCTime
      manifest =
        TaskManifest
          "virtual-window-log"
          [SourceManifest "each-minute" "time.interval" (toJSON (IntervalConfig 60000))]
          (StateManifest "virtual-window-state" 1 "segno.state" "compare-and-set" (object ["count" .= (0 :: Int)]))
      job = InstalledJob ".tactus/scripts/900_virtual.hs" manifest
  writeJson (pathsJobs paths </> "virtual-window-log.json") job
  writeFile (root </> ".tactus" </> "scripts" </> "900_virtual.hs") "main = pure ()\n"
  nowRef <- newIORef now
  let clock = Clock (readIORef nowRef) (writeIORef nowRef)
      runner = fakeRunner now
      environment =
        defaultDriverEnvironment
          { driverClock = clock,
            driverRunner = runner
          }
  summary <- runOnceWith environment root
  assert (summaryPlanned summary == 1) "driver did not plan one occurrence"
  assert (summaryExecuted summary == 1 && summarySucceeded summary == 1) "fake workflow did not succeed"
  exchanges <- listDirectory (pathsTriggers paths </> "exchanges")
  assert (null exchanges) "temporary invocation/result exchange was retained"
  records <- lifecycleStatus paths (Just "virtual-window-log")
  assert (fmap recordLifecycle records == [Succeeded]) "lifecycle did not finish Succeeded"

testVirtualWindowPersistence :: IO ()
testVirtualWindowPersistence = withTemporaryWorkspace "virtual-window-persistence" $ \root -> do
  sdk <- locateSegnoSource
  createTactusWorkspace root sdk
  paths <- initialiseWorkspaceWithSdk root (Just sdk)
  let initialTime = read "2026-08-16 12:00:00 UTC" :: UTCTime
      longTrigger = Text.replicate 200 "t"
      stateManifest =
        StateManifest
          "example.active-window"
          1
          "segno.state"
          "compare-and-set"
          (object ["capturedWindows" .= (0 :: Int), "latestWindow" .= Null, "lastLogicalTime" .= Null])
      manifest =
        TaskManifest
          "record-active-window"
          [SourceManifest longTrigger "time.interval" (toJSON (IntervalConfig 60000))]
          stateManifest
      job = InstalledJob ".tactus/scripts/900_virtual_window.hs" manifest
  writeJson (pathsJobs paths </> "record-active-window.json") job
  writeFile (root </> ".tactus" </> "scripts" </> "900_virtual_window.hs") "main = pure ()\n"
  nowRef <- newIORef initialTime
  windowCalls <- newIORef (0 :: Int)
  pluginCalls <- newIORef ([] :: [Text])
  let clock = Clock (readIORef nowRef) (writeIORef nowRef)
      runner = persistentWindowRunner paths nowRef windowCalls pluginCalls
      environment = defaultDriverEnvironment {driverClock = clock, driverRunner = runner}
  forM_ [0, 60, 120] $ \offset -> do
    writeIORef nowRef (addUTCTime offset initialTime)
    summary <- runOnceWith environment root
    assert (summaryPlanned summary == 1) "minute trigger did not plan exactly one occurrence"
    assert (summaryExecuted summary == 1 && summarySucceeded summary == 1) "minute occurrence did not checkpoint successfully"
  -- A fresh environment models a driver restart. The durable cursor must
  -- prevent the last logical minute from being emitted twice.
  let restarted = defaultDriverEnvironment {driverClock = clock, driverRunner = runner}
  afterRestart <- runOnceWith restarted root
  assert (summaryPlanned afterRestart == 0 && summaryExecuted afterRestart == 0) "driver restart duplicated the acknowledged minute"
  history <- businessHistory paths (Just "example.active-window") 10
  finalSnapshot <- loadBusinessState paths stateManifest (addUTCTime 120 initialTime)
  captured <- either fail pure (parseEither parseCapturedWindows (snapshotValue finalSnapshot))
  recordedWindowCalls <- readIORef windowCalls
  observedPlugins <- readIORef pluginCalls
  records <- lifecycleStatus paths (Just "record-active-window")
  assert (length history == 3) "business-state history did not retain all three checkpoints"
  assert (snapshotRevision finalSnapshot == Just "3" && captured == 3) "typed business state did not advance to revision three"
  assert (recordedWindowCalls == 3) "fake active-window source was not called once per logical minute"
  assert (length records == 3 && all ((== Succeeded) . recordLifecycle) records) "minute occurrences did not all finish Succeeded"
  assert (all ((== 68) . Text.length . occurrenceId . recordOccurrence) records) "long plugin identities did not produce bounded occurrence ids"
  assert (all (`elem` ["time.interval", "segno.state"]) observedPlugins) "persistent task unexpectedly called a provider"

parseCapturedWindows :: Value -> Parser Int
parseCapturedWindows = withObject "window log" (.: "capturedWindows")

persistentWindowRunner :: SegnoPaths -> IORef UTCTime -> IORef Int -> IORef [Text] -> Runner
persistentWindowRunner paths nowRef windowCalls pluginCalls = Runner runTask callPlugin
  where
    runTask _ _ mode invocationPath resultPath _context _timeoutSeconds = do
      when (mode /= "execute") (fail "unexpected fake task mode")
      path <- maybe (fail "missing fake invocation") pure invocationPath
      invocationValue <- readJsonValue path
      (task, occurrenceIdentity, fence, epoch, revision, count, logicalTime) <-
        either fail pure (parseEither parseWindowInvocation invocationValue)
      modifyIORef' windowCalls (+ 1)
      let nextCount = count + 1
          activeWindow =
            object
              [ "title" .= ("fake-window-" <> Text.pack (show nextCount)),
                "captured_at" .= logicalTime,
                "platform" .= ("test" :: Text)
              ]
          nextState =
            object
              [ "capturedWindows" .= nextCount,
                "latestWindow" .= activeWindow,
                "lastLogicalTime" .= logicalTime
              ]
      now <- readIORef nowRef
      checkpointResult <-
        compareAndSetBusinessState
          paths
          StateCasRequest
            { casStateKey = "example.active-window",
              casExpectedRevision = revision,
              casSchemaVersion = 1,
              casValue = nextState,
              casOperationId = OperationId "capture-active-window",
              casFencing = FencingContext
                { fenceOccurrenceId = OccurrenceId occurrenceIdentity,
                  fenceToken = FencingToken fence,
                  fenceEpoch = FencingEpoch epoch
                }
            }
          now
      nextRevision <- case checkpointResult of
        CasApplied value -> pure value
        CasConflict actual -> fail ("fake checkpoint conflicted: " <> show actual)
      writeJson
        resultPath
        ( object
            [ "api" .= resultApi,
              "task" .= task,
              "occurrence_id" .= occurrenceIdentity,
              "decision"
                .= object
                  [ "kind" .= ("complete" :: Text),
                    "state" .= object ["kind" .= ("keep" :: Text), "expected_revision" .= nextRevision],
                    "output" .= activeWindow
                  ]
            ]
        )
      pure (ProcessOutcome ExitSuccess "" "" False)

    callPlugin _ plugin method params = do
      modifyIORef' pluginCalls (plugin :)
      now <- readIORef nowRef
      case (plugin, method) of
        ("time.interval", "poll") -> case parseEither parseTimePoll params of
          Left message -> pure (Left (PluginFailure "invalid_test_poll" (Text.pack message) Nothing))
          Right (sourceIdentity, configuration, cursor, requestedNow, limit) ->
            pure $ case planTimeSource plugin sourceIdentity configuration cursor requestedNow limit of
              Left message -> Left (PluginFailure "test_poll_failed" message Nothing)
              Right result -> Right (toJSON result)
        ("time.interval", "acknowledge") -> pure (Right (object ["acknowledged" .= True]))
        ("segno.state", "load") -> case parseEither parseStateLoad params of
          Left message -> pure (Left (PluginFailure "invalid_test_state" (Text.pack message) Nothing))
          Right selectedState -> Right . toJSON <$> loadBusinessState paths selectedState now
        _ -> pure (Left (unexpectedPlugin plugin method))

parseTimePoll :: Object -> Parser (Text, Value, Maybe Value, UTCTime, Int)
parseTimePoll fields =
  (,,,,)
    <$> fields .: "source_id"
    <*> fields .: "config"
    <*> fields .:? "cursor"
    <*> fields .: "now"
    <*> fields .: "limit"

parseStateLoad :: Object -> Parser StateManifest
parseStateLoad fields =
  StateManifest
    <$> fields .: "state_key"
    <*> fields .: "schema_version"
    <*> pure "segno.state"
    <*> pure "compare-and-set"
    <*> fields .: "initial"

parseWindowInvocation :: Value -> Parser (Text, Text, Text, Word32, Maybe Text, Int, UTCTime)
parseWindowInvocation = withObject "window invocation" $ \fields -> do
  task <- fields .: "task"
  fence <- fields .: "fencing_token"
  epoch <- fields .: "attempt"
  trigger <- fields .: "trigger"
  selectedState <- fields .: "state"
  occurrenceIdentity <- withObject "trigger" (.: "occurrence_id") trigger
  logicalTime <- withObject "trigger" (.: "logical_time") trigger
  revision <- withObject "state" (.:? "revision") selectedState
  stateValue <- withObject "state" (.: "value") selectedState
  count <- withObject "window log" (.: "capturedWindows") stateValue
  pure (task, occurrenceIdentity, fence, epoch, revision, count, logicalTime)

testRelativeSdkDiscovery :: IO ()
testRelativeSdkDiscovery = withTemporaryWorkspace "relative-sdk" $ \root -> do
  sdk <- locateSegnoSource
  createTactusWorkspace root sdk
  _ <- initialiseWorkspaceWithSdk root Nothing
  project <- TextIO.readFile (root </> ".tactus" </> "cabal.project")
  let packageLines = filter (Text.isPrefixOf "  ") (Text.lines project)
  case packageLines of
    firstPackage : _ -> assert (Text.isInfixOf "clef-sdk" firstPackage) "Segno was inserted before the Clef SDK package"
    [] -> ioError (userError "cabal.project lost its package entries")

testBadTriggerIsolation :: IO ()
testBadTriggerIsolation = withTemporaryWorkspace "bad-trigger" $ \root -> do
  sdk <- locateSegnoSource
  createTactusWorkspace root sdk
  paths <- initialiseWorkspaceWithSdk root (Just sdk)
  let now = read "2026-08-16 12:00:00 UTC" :: UTCTime
      manifest =
        TaskManifest
          "ready-despite-bad-trigger"
          [ SourceManifest "broken" "bad.trigger" (object []),
            SourceManifest "past-wake" "bad.wake" (object [])
          ]
          (StateManifest "ready-state" 1 "segno.state" "compare-and-set" (object []))
      job = InstalledJob ".tactus/scripts/901_ready.hs" manifest
      baseRunner = fakeRunner now
      isolatedRunner =
        baseRunner
          { runnerPlugin = \workspace plugin method params ->
              if plugin == "bad.trigger"
                then ioError (userError "deliberate trigger transport failure")
                else
                  if plugin == "bad.wake" && method == "poll"
                    then pure (Right (toJSON (PollResult [] (Just now))))
                    else runnerPlugin baseRunner workspace plugin method params
          }
      environment =
        defaultDriverEnvironment
          { driverClock = Clock (pure now) (\_ -> pure ()),
            driverRunner = isolatedRunner
          }
  writeJson (pathsJobs paths </> "ready-despite-bad-trigger.json") job
  writeFile (root </> ".tactus" </> "scripts" </> "901_ready.hs") "main = pure ()\n"
  _ <- insertOccurrence paths "ready-despite-bad-trigger" (occurrence "ready-occurrence" "ready-key" now) now
  summary <- runOnceWith environment root
  assert (summaryTriggerFailures summary == 2) "invalid trigger results were not isolated and reported"
  assert (summaryExecuted summary == 1 && summarySucceeded summary == 1) "ready occurrence was starved by bad trigger"

testOutcomeUnknownStops :: IO ()
testOutcomeUnknownStops = withTemporaryWorkspace "outcome-unknown" $ \root -> do
  sdk <- locateSegnoSource
  createTactusWorkspace root sdk
  paths <- initialiseWorkspaceWithSdk root (Just sdk)
  let now = read "2026-08-16 12:00:00 UTC" :: UTCTime
      manifest =
        TaskManifest
          "unknown-task"
          [SourceManifest "quiet" "quiet.trigger" (object [])]
          (StateManifest "unknown-state" 1 "segno.state" "compare-and-set" (object []))
      job = InstalledJob ".tactus/scripts/902_unknown.hs" manifest
      baseRunner = fakeRunner now
  taskCalls <- newIORef (0 :: Int)
  let unknownRunner =
        baseRunner
          { runnerTask = \_ _ _ _ _ _context _timeoutSeconds -> do
              calls <- readIORef taskCalls
              writeIORef taskCalls (calls + 1)
              pure (ProcessOutcome (ExitFailure 9) "" "workflow crashed after starting" False),
            runnerPlugin = \workspace plugin method params ->
              if plugin == "quiet.trigger" && method == "poll"
                then pure (Right (toJSON (PollResult [] (Just (addUTCTime 60 now)))))
                else runnerPlugin baseRunner workspace plugin method params
          }
      environment = defaultDriverEnvironment {driverClock = Clock (pure now) (\_ -> pure ()), driverRunner = unknownRunner}
  writeJson (pathsJobs paths </> "unknown-task.json") job
  writeFile (root </> ".tactus" </> "scripts" </> "902_unknown.hs") "main = pure ()\n"
  _ <- insertOccurrence paths "unknown-task" (occurrence "unknown-occurrence" "unknown-key" now) now
  first <- runOnceWith environment root
  second <- runOnceWith environment root
  calls <- readIORef taskCalls
  records <- lifecycleStatus paths (Just "unknown-task")
  assert (summaryUnknown first == 1) "failed spawned workflow was not OutcomeUnknown"
  assert (summaryExecuted second == 0 && calls == 1) "OutcomeUnknown was automatically retried"
  assert (fmap recordLifecycle records == [OutcomeUnknown]) "OutcomeUnknown lifecycle was not durable"

fakeRunner :: UTCTime -> Runner
fakeRunner now = Runner runTask callPlugin
  where
    runTask _ _ mode invocationPath resultPath _context _timeoutSeconds = do
      exists <- doesFileExist resultPath
      assert (not exists) "SEGNO_RESULT_PATH was not unique and nonexistent"
      when (mode /= "execute") (ioError (userError "unexpected fake task mode"))
      path <- maybe (ioError (userError "missing fake invocation")) pure invocationPath
      invocationValue <- readJsonValue path
      (task, occurrenceIdentity) <- case parseEither parseInvocationIdentity invocationValue of
        Left message -> ioError (userError message)
        Right value -> pure value
      writeJson
        resultPath
        ( object
            [ "api" .= resultApi,
              "task" .= task,
              "occurrence_id" .= occurrenceIdentity,
              "decision" .= object ["kind" .= ("ignore" :: Text)]
            ]
        )
      pure (ProcessOutcome ExitSuccess "" "" False)

    callPlugin _ plugin method params
      | plugin == "time.interval" && method == "poll" =
          pure . Right . toJSON $
            case KeyMap.lookup "cursor" params of
              Just Null -> PollResult [planned] (Just (addUTCTime 60 now))
              Nothing -> PollResult [planned] (Just (addUTCTime 60 now))
              _ -> PollResult [] (Just (addUTCTime 60 now))
      | plugin == "time.interval" && method == "acknowledge" = pure (Right (object ["acknowledged" .= True]))
      | plugin == "segno.state" && method == "load" =
          let key = case KeyMap.lookup "state_key" params of
                Just (String value) -> value
                _ -> "missing-state-key"
           in pure . Right . toJSON $ StateSnapshot key (Just "0") 1 (object ["count" .= (0 :: Int)])
      | otherwise = pure (Left (unexpectedPlugin plugin method))

    planned =
      PlannedOccurrence
        { plannedLogicalTime = now,
          plannedCursor = object ["logical_time" .= now],
          plannedIdempotencyKey = "each-minute:virtual",
          plannedPayload = object ["logical_time" .= now]
        }

parseInvocationIdentity :: Value -> Parser (Text, Text)
parseInvocationIdentity = withObject "invocation" $ \fields -> do
  task <- fields .: "task"
  trigger <- fields .: "trigger"
  occurrenceIdentity <- withObject "trigger" (.: "occurrence_id") trigger
  pure (task, occurrenceIdentity)

unexpectedPlugin :: Text -> Text -> PluginFailure
unexpectedPlugin plugin method =
  PluginFailure "unexpected_plugin" (plugin <> "/" <> method) Nothing

occurrence :: Text -> Text -> UTCTime -> TriggerOccurrence
occurrence identity idempotency now =
  TriggerOccurrence
    { occurrenceTriggerId = "each-minute",
      occurrenceId = identity,
      occurrenceLogicalTime = now,
      occurrenceObservedTime = now,
      occurrenceCursor = object ["logical_time" .= now],
      occurrenceIdempotencyKey = idempotency,
      occurrencePayload = object ["logical_time" .= now]
    }

createTactusWorkspace :: FilePath -> FilePath -> IO ()
createTactusWorkspace root sdk = do
  createDirectoryIfMissing True (root </> ".tactus" </> "scripts")
  createDirectoryIfMissing True (root </> ".tactus" </> "runs")
  TextIO.writeFile (root </> ".tactus" </> "tactus.toml") (testWorkspaceConfig "[plugins]\n")
  writeFile (root </> ".tactus" </> "PROMPT.md") "Offline fixture.\n"
  let clef = takeDirectory sdk </> "clef-sdk"
      relativeClef = fmap (\character -> if character == '\\' then '/' else character) (makeRelative (root </> ".tactus") clef)
  writeFile (root </> ".tactus" </> "cabal.project") ("packages:\n  \"" <> relativeClef <> "\"\n")

testWorkspaceConfig :: Text -> Text
testWorkspaceConfig plugins =
  "api = \"clef.runtime/v1\"\ndefault_provider = \"fake\"\ninstructions = \".tactus/PROMPT.md\"\n"
    <> plugins <> "\n[providers.fake]\ncommand = ['unused-provider']\n"

locateSegnoSource :: IO FilePath
locateSegnoSource = do
  current <- getCurrentDirectory
  search current
  where
    search directory = do
      direct <- doesFileExist (directory </> "segno-flow.cabal")
      nested <- doesFileExist (directory </> "segno-flow" </> "segno-flow.cabal")
      if direct
        then canonicalizePath directory
        else
          if nested
            then canonicalizePath (directory </> "segno-flow")
            else do
              let parent = takeDirectory directory
              if parent == directory then ioError (userError "could not locate segno-flow source") else search parent

withTemporaryWorkspace :: String -> (FilePath -> IO value) -> IO value
withTemporaryWorkspace label = bracket create cleanup
  where
    create = do
      temporary <- getTemporaryDirectory
      now <- getCurrentTime
      let path = temporary </> ("agenstro-segno-test-" <> label <> "-" <> filter (/= ':') (show now))
      createDirectory path
      pure path
    cleanup path = removeDirectoryRecursive path `catch` (\(_ :: IOException) -> pure ())

writeJson :: ToJSON value => FilePath -> value -> IO ()
writeJson path = LazyByteString.writeFile path . (<> "\n") . encode

readJsonValue :: FilePath -> IO Value
readJsonValue path = do
  encoded <- LazyByteString.readFile path
  case eitherDecode encoded of
    Left message -> ioError (userError message)
    Right value -> pure value

isLeft :: Either left right -> Bool
isLeft (Left _) = True
isLeft (Right _) = False

isConflict :: CasResult -> Bool
isConflict (CasConflict _) = True
isConflict (CasApplied _) = False

infixr 3 <|>
(<|>) :: Maybe value -> Maybe value -> Maybe value
left <|> right = case left of
  Just _ -> left
  Nothing -> right
