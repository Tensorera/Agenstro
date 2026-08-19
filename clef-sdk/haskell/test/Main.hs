{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE ScopedTypeVariables #-}
{-# LANGUAGE LambdaCase #-}

module Main (main) where

import Clef
import qualified Clef.Effect.WorkspacePaths as WorkspacePaths
import Clef.Plugin.Protocol
  ( ParsedPluginOutput (..),
    PluginOutputStreamParser,
    PluginTerminal (..),
    feedPluginOutputChunk,
    finishPluginOutputStream,
    initialPluginOutputStreamParser,
    parsePluginOutput,
  )
import Control.Concurrent (forkIO, threadDelay)
import qualified Control.Concurrent.Async as Async
import Control.Concurrent.MVar
  ( modifyMVar_,
    newEmptyMVar,
    newMVar,
    putMVar,
    readMVar,
    takeMVar,
    tryPutMVar,
    tryReadMVar,
    tryTakeMVar,
  )
import Control.Exception
  ( AsyncException (ThreadKilled),
    SomeException,
    bracket,
    catch,
    finally,
    fromException,
    throwIO,
    try,
  )
import Control.Monad (foldM, forM, forM_, unless)
import Data.Aeson
  ( FromJSON (parseJSON),
    ToJSON (toJSON),
    Value (..),
    encode,
    eitherDecodeStrict',
    object,
    withObject,
    (.:),
    (.:?),
    (.=),
  )
import Data.Aeson.Types (Parser, parseEither)
import qualified Data.Aeson.KeyMap as KeyMap
import qualified Data.ByteString as ByteString
import qualified Data.ByteString.Char8 as ByteString.Char8
import qualified Data.ByteString.Lazy as LazyByteString
import qualified Data.ByteString.Lazy.Char8 as LazyChar8
import qualified Data.Map.Strict as Map
import Data.Scientific (scientific)
import Data.Text (Text)
import qualified Data.Text as Text
import qualified Data.Text.Encoding as Text.Encoding
import System.Directory (getCurrentDirectory, removeFile)
import System.Environment (getArgs, getExecutablePath, lookupEnv, setEnv, unsetEnv)
import System.Exit (ExitCode (..), exitFailure)
import System.IO (hClose, openBinaryTempFile, stderr)
import qualified System.IO as IO
import System.FilePath ((</>))
import System.Process (readProcessWithExitCode)
import System.Timeout (timeout)
import MonadIOCompatibility (standardLiftIO)

main :: IO ()
main = do
  arguments <- getArgs
  case arguments of
    ["--fake-plugin"] -> fakePlugin
    ["--fake-exit"] -> exitFailure
    ["--fake-reported-exit"] -> fakeReportedExit
    ["--fake-outcome-unknown"] -> fakeOutcomeUnknown
    ["--fake-backpressure"] -> fakeBackpressure
    ["--fake-stream"] -> fakeStream
    ["--fake-block"] -> fakeBlock
    ["--fake-block-end"] -> fakeBlockEnd
    ["--fake-cli-error", configPath] -> fakeCliError configPath
    ["--fake-cli-provider", configPath] -> fakeCliProvider configPath
    ["--fake-cli-unknown", configPath] -> fakeCliUnknown configPath
    ["--fake-cli-unexpected", configPath] -> fakeCliUnexpected configPath
    _ -> runTests

runTests :: IO ()
runTests = do
  executable <- getExecutablePath
  workspace <- getCurrentDirectory
  let tests =
        [ ("Workflow monad and require", testWorkflowMonad workspace executable),
          ("parallel uses structured concurrency", testParallel workspace executable),
          ("text and JSON task decoders", testTaskDecoders),
          ("runtime config schema", testRuntimeConfig executable workspace),
          ("JSONL terminal protocol", testProtocolRules),
          ("shared Rust and Haskell protocol conformance vectors", testProtocolConformance workspace),
          ("incremental protocol parsing is chunk-boundary independent", testProtocolChunkBoundaries),
          ("plugin process exit is checked", testPluginExit workspace executable),
          ("plugin pipes drain concurrently", testPluginPipeBackpressure workspace executable),
          ("plugin events stream before terminal across UTF-8 chunks", testPluginEventStreaming workspace executable),
          ("runtime transitions explain state, trigger, guard, and result", testRuntimeTransitions workspace executable),
          ("timeout and async cancellation remain cancelled", testCancellationClassification workspace executable),
          ("default presentation excludes raw structured diagnostics", testHumanProjection),
          ("outcome unknown is natural language with a structured cause", testOutcomeUnknownRendering),
          ("runTactus renders expected errors without a call stack", testRunTactusPresentation workspace executable),
          ("blocked event sinks fail boundedly without hiding plugin outcome", testBlockedEventSink workspace executable),
          ("sink overload drops events but preserves terminal records", testEventSinkOverload workspace executable),
          ("runWorkflow flushes the final typed value projection", testFinalSinkProjection workspace executable),
          ("generic plugin call is statically typed", testGenericPlugin workspace executable),
          ("runtime-owned plugin parameters cannot be shadowed", testPluginParameterConflicts workspace executable),
          ("invoke records provider value and observer evidence separately", testInvokeAndObserve workspace executable),
          ("observer begin cancellation cleans up earlier observers", testObserverBeginCancellation workspace executable),
          ("observer end cancellation does not skip remaining observers", testObserverEndCancellation workspace executable),
          ("typed workspace path operations", testWorkspaceOperations workspace executable),
          ("Segno trigger manifest is typed and deterministic", testSegnoManifest),
          ("Segno describe publishes an atomic result document", testSegnoDescribe workspace),
          ("Segno execute checkpoints through the generic state plugin", testSegnoExecute workspace executable),
          ("Segno gate ignores before loading the Clef runtime", testSegnoGate workspace)
        ]
  outcomes <- forM tests runOne
  unless (and outcomes) exitFailure
  where
    runOne (name, action) = do
      result <- try action :: IO (Either SomeException ())
      case result of
        Left exception -> do
          IO.hPutStrLn stderr $ "FAIL " <> name <> ": " <> show exception
          pure False
        Right () -> do
          putStrLn $ "PASS " <> name
          pure True

testWorkflowMonad :: FilePath -> FilePath -> IO ()
testWorkflowMonad workspace executable = do
  runtime <- newRuntime (testConfig workspace executable)
  result <- runWorkflow runtime $ do
    first <- pure (20 :: Int)
    second <- standardLiftIO (pure 22)
    require (== 42) (first + second)
  assertEqual "monadic result" 42 result
  assertWorkflowError
    "requireBecause"
    (\workflowError -> workflowError == RequirementFailed "explanation")
    (runWorkflow runtime (requireBecause "explanation" (const False) ()))
  captured <-
    runWorkflow runtime (attempt (requireBecause "captured" (const False) ()))
  assertEqual
    "attempt captures only WorkflowError values"
    (Left (RequirementFailed "captured"))
    captured

testParallel :: FilePath -> FilePath -> IO ()
testParallel workspace executable = do
  runtime <- newRuntime (testConfig workspace executable)
  leftReady <- newEmptyMVar
  rightReady <- newEmptyMVar
  outcome <-
    timeout 2000000 . runWorkflow runtime $
      parallel
        (liftIO (putMVar leftReady () >> takeMVar rightReady >> pure ("left" :: Text)))
        (liftIO (putMVar rightReady () >> takeMVar leftReady >> pure (42 :: Int)))
  assertEqual "heterogeneous parallel result" (Just ("left", 42)) outcome
  allResult <- runWorkflow runtime (parallelAll [pure (1 :: Int), pure 2, pure 3])
  assertEqual "homogeneous parallelAll result" [1, 2, 3] allResult

data Answer = Answer
  { answer :: Int
  }
  deriving (Eq, Show)

instance FromJSON Answer where
  parseJSON = withObject "answer" $ \objectValue -> Answer <$> objectValue .: "answer"

testTaskDecoders :: IO ()
testTaskDecoders = do
  let plainTask = textTask "plain" (id :: Text -> Text)
      structuredTask = jsonTask "structured" (id :: Text -> Text) :: Task Text Answer
      customTask =
        task
          "custom"
          (id :: Text -> Text)
          (\text -> if text == "accepted" then Right True else Left "not accepted")
  assertEqual "text decoder" (Right "hello") (decodeTaskResult plainTask "hello")
  assertEqual
    "JSON decoder"
    (Right (Answer 42))
    (decodeTaskResult structuredTask "{\"answer\":42}")
  assertEqual "custom decoder" (Right True) (decodeTaskResult customTask "accepted")
  case decodeTaskResult structuredTask "not-json" of
    Left (TaskDecodeFailed "structured" _) -> pure ()
    other -> failTest $ "expected TaskDecodeFailed, received " <> show other
  case decodeTaskResult structuredTask "{\"answer\":1,\"answer\":2}" of
    Left (TaskDecodeFailed "structured" _) -> pure ()
    other -> failTest $ "duplicate JSON task key should fail, received " <> show other
  let floatingTask = jsonTask "floating" (id :: Text -> Text) :: Task Text Double
  case decodeTaskResult floatingTask "1e999" of
    Left (TaskDecodeFailed "floating" _) -> pure ()
    other -> failTest $ "overflowing JSON task number should fail, received " <> show other
  case decodeTaskResult floatingTask "1e-999" of
    Left (TaskDecodeFailed "floating" _) -> pure ()
    other -> failTest $ "underflowing JSON task number should fail, received " <> show other

testRuntimeConfig :: FilePath -> FilePath -> IO ()
testRuntimeConfig executable workspace = do
  let valid =
        object
          [ "api" .= ("clef.runtime/v1" :: Text),
            "workspace" .= workspace,
            "default_provider" .= ("fake" :: Text),
            "providers"
              .= object
                [ "fake"
                    .= object
                      [ "command" .= [executable, "--fake-plugin"],
                        "model" .= ("model-any-string" :: Text),
                        "effort" .= ("effort-any-string" :: Text),
                        "options" .= object []
                      ]
                ],
            "effects" .= object [],
            "plugins"
              .= object
                [ "generic"
                    .= object
                      [ "command" .= [executable, "--fake-plugin"],
                        "options" .= object ["mode" .= ("test" :: Text)]
                      ]
                ],
            "instructions" .= ("system instructions" :: Text)
          ]
      legacyWithoutPlugins = case valid of
        Object fields -> Object (KeyMap.delete "plugins" fields)
        _ -> valid
      invalidRelativeWorkspace =
        object
          [ "api" .= ("clef.runtime/v1" :: Text),
            "workspace" .= ("relative/path" :: FilePath),
            "default_provider" .= ("fake" :: Text),
            "providers" .= object ["fake" .= object ["command" .= [executable]]],
            "effects" .= object [],
            "instructions" .= ("" :: Text)
          ]
  case decodeRuntimeConfig (strictEncode valid) of
    Left workflowError -> failTest $ "valid config failed: " <> show workflowError
    Right config -> do
      assertEqual "runtime api" "clef.runtime/v1" (runtimeApi config)
      assertEqual "default provider" "fake" (runtimeDefaultProvider config)
      assertEqual "generic plugin registry" True (Map.member "generic" (runtimePlugins config))
  case decodeRuntimeConfig (strictEncode legacyWithoutPlugins) of
    Right config -> assertEqual "legacy config defaults plugins" Map.empty (runtimePlugins config)
    Left workflowError -> failTest $ "legacy config without plugins failed: " <> show workflowError
  case decodeRuntimeConfig (strictEncode invalidRelativeWorkspace) of
    Left (RuntimeConfigError _) -> pure ()
    other -> failTest $ "relative workspace should fail, received " <> show other
  case decodeRuntimeConfig "{\"api\":\"clef.runtime/v1\",\"api\":\"clef.runtime/v1\"}" of
    Left (RuntimeConfigError _) -> pure ()
    other -> failTest $ "duplicate runtime key should fail, received " <> show other
  case decodeRuntimeConfig "{\"options\":1e-999}" of
    Left (RuntimeConfigError _) -> pure ()
    other -> failTest $ "underflowing runtime number should fail, received " <> show other

testProtocolRules :: IO ()
testProtocolRules = do
  let event = "{\"type\":\"event\",\"id\":\"req-1\",\"event\":{\"type\":\"progress\"}}"
      futureEvent = "{\"type\":\"event\",\"id\":\"req-1\",\"event\":{\"type\":\"future.unknown\",\"payload\":42}}"
      success = "{\"type\":\"result\",\"id\":\"req-1\",\"ok\":true,\"value\":null}"
      otherSuccess = "{\"type\":\"result\",\"id\":\"other\",\"ok\":true,\"value\":null}"
  case parsePluginOutput "fake" "req-1" (event <> "\n" <> futureEvent <> "\n" <> success <> "\n") of
    Right (ParsedPluginOutput [_, _] (PluginSucceeded Null)) -> pure ()
    other -> failTest $ "valid event/result stream failed: " <> show other
  let numericSuccess =
        "{\"type\":\"result\",\"id\":\"req-1\",\"ok\":true,\"value\":[1.7976931348623157e308,5e-324,-0.0,18446744073709551615,-9223372036854775808]}"
  case parsePluginOutput "fake" "req-1" numericSuccess of
    Right (ParsedPluginOutput [] (PluginSucceeded _)) -> pure ()
    other -> failTest $ "boundary JSON numbers should succeed, received " <> show other
  assertProtocolFailure "missing terminal" (parsePluginOutput "fake" "req-1" event)
  assertProtocolFailure
    "terminal after data"
    (parsePluginOutput "fake" "req-1" (success <> "\n" <> event))
  assertProtocolFailure
    "duplicate terminal"
    (parsePluginOutput "fake" "req-1" (success <> "\n" <> success))
  assertProtocolFailure "correlation mismatch" (parsePluginOutput "fake" "req-1" otherSuccess)
  assertProtocolFailure
    "missing event body"
    ( parsePluginOutput
        "fake"
        "req-1"
        ("{\"type\":\"event\",\"id\":\"req-1\"}\n" <> success)
    )
  assertProtocolFailure
    "event subtype must be a string"
    ( parsePluginOutput
        "fake"
        "req-1"
        ("{\"type\":\"event\",\"id\":\"req-1\",\"event\":{\"type\":42}}\n" <> success)
    )
  assertProtocolFailure
    "value/error exclusivity"
    ( parsePluginOutput
        "fake"
        "req-1"
        "{\"type\":\"result\",\"id\":\"req-1\",\"ok\":true,\"value\":null,\"error\":{}}"
    )
  assertProtocolFailure
    "failure envelope requires code and message"
    ( parsePluginOutput
        "fake"
        "req-1"
        "{\"type\":\"result\",\"id\":\"req-1\",\"ok\":false,\"error\":{\"code\":\"broken\"}}"
    )
  assertProtocolFailure
    "duplicate JSON object key"
    ( parsePluginOutput
        "fake"
        "req-1"
        "{\"type\":\"result\",\"id\":\"req-1\",\"ok\":true,\"value\":{\"answer\":1,\"answer\":2}}"
    )
  assertProtocolFailure
    "overflowing JSON number"
    ( parsePluginOutput
        "fake"
        "req-1"
        "{\"type\":\"result\",\"id\":\"req-1\",\"ok\":true,\"value\":1e999}"
    )
  assertProtocolFailure
    "underflowing JSON number"
    ( parsePluginOutput
        "fake"
        "req-1"
        "{\"type\":\"result\",\"id\":\"req-1\",\"ok\":true,\"value\":1e-999}"
    )
  assertProtocolFailure
    "positive integer outside the shared wire domain"
    ( parsePluginOutput
        "fake"
        "req-1"
        "{\"type\":\"result\",\"id\":\"req-1\",\"ok\":true,\"value\":18446744073709551616}"
    )
  assertProtocolFailure
    "negative integer outside the shared wire domain"
    ( parsePluginOutput
        "fake"
        "req-1"
        "{\"type\":\"result\",\"id\":\"req-1\",\"ok\":true,\"value\":-9223372036854775809}"
    )

data ProtocolConformanceDocument = ProtocolConformanceDocument Text [ProtocolConformanceCase]

data ProtocolConformanceCase = ProtocolConformanceCase
  { protocolCaseName :: Text,
    protocolCaseExpectedId :: Text,
    protocolCaseFrames :: [Text],
    protocolCaseAccepted :: Bool,
    protocolCaseTerminal :: Maybe Text,
    protocolCaseEventCount :: Maybe Int
  }

instance FromJSON ProtocolConformanceDocument where
  parseJSON = withObject "protocol conformance document" $ \fields ->
    ProtocolConformanceDocument <$> fields .: "api" <*> fields .: "cases"

instance FromJSON ProtocolConformanceCase where
  parseJSON = withObject "protocol conformance case" $ \fields ->
    ProtocolConformanceCase
      <$> fields .: "name"
      <*> fields .: "expected_id"
      <*> fields .: "frames"
      <*> fields .: "accepted"
      <*> fields .:? "terminal"
      <*> fields .:? "event_count"

testProtocolConformance :: FilePath -> IO ()
testProtocolConformance workspace = do
  encoded <-
    ByteString.readFile
      (workspace </> ".." </> "Test" </> "fixtures" </> "plugin-protocol-v1" </> "cases.json")
  ProtocolConformanceDocument api cases <-
    case eitherDecodeStrict' encoded of
      Left message -> failTest $ "invalid shared protocol fixture: " <> message
      Right document -> pure document
  assertEqual "protocol fixture api" "agenstro.plugin.conformance/v1" api
  forM_ cases $ \protocolCase -> do
    let output = Text.intercalate "\n" (protocolCaseFrames protocolCase) <> "\n"
        parsed = parsePluginOutput "conformance" (protocolCaseExpectedId protocolCase) output
        label = Text.unpack (protocolCaseName protocolCase)
    case (protocolCaseAccepted protocolCase, parsed) of
      (False, Left _) -> pure ()
      (False, Right result) -> failTest $ label <> ": rejected fixture succeeded with " <> show result
      (True, Left workflowError) -> failTest $ label <> ": accepted fixture failed with " <> show workflowError
      (True, Right result) -> do
        assertEqual (label <> " event count") (protocolCaseEventCount protocolCase) (Just (length (pluginOutputEvents result)))
        let terminalKind = case pluginOutputTerminal result of
              PluginSucceeded _ -> "success"
              PluginFailed _ -> "failure"
        assertEqual (label <> " terminal") (protocolCaseTerminal protocolCase) (Just terminalKind)

testProtocolChunkBoundaries :: IO ()
testProtocolChunkBoundaries = do
  let output =
        Text.intercalate
          "\n"
          [ "{\"type\":\"event\",\"id\":\"chunks\",\"event\":{\"type\":\"progress\",\"message\":\"边界🌊\"}}",
            "{\"type\":\"result\",\"id\":\"chunks\",\"ok\":true,\"value\":{\"answer\":42}}"
          ]
          <> "\n"
      encoded = Text.Encoding.encodeUtf8 output
      expected = parsePluginOutput "chunks" "chunks" output
      twoWayPartitions =
        [ [ByteString.take index encoded, ByteString.drop index encoded]
          | index <- [0 .. ByteString.length encoded]
        ]
      bytewisePartition = fmap ByteString.singleton (ByteString.unpack encoded)
  forM_ (bytewisePartition : twoWayPartitions) $ \chunks ->
    assertEqual "chunked parser result" expected (snd <$> parseProtocolChunks chunks)
  assertProtocolFailure
    "streaming parser rejects invalid UTF-8"
    (snd <$> parseProtocolChunks [ByteString.pack [255, 10]])

parseProtocolChunks :: [ByteString.ByteString] -> Either WorkflowError ([Value], ParsedPluginOutput)
parseProtocolChunks chunks = do
  (parser, events) <-
    foldM feed (initialPluginOutputStreamParser, []) chunks
  (finalEvents, output) <- finishPluginOutputStream "chunks" "chunks" parser
  pure (events <> finalEvents, output)
  where
    feed :: (PluginOutputStreamParser, [Value]) -> ByteString.ByteString -> Either WorkflowError (PluginOutputStreamParser, [Value])
    feed (parser, events) chunk = do
      (nextParser, nextEvents) <- feedPluginOutputChunk "chunks" "chunks" parser chunk
      pure (nextParser, events <> nextEvents)

testPluginExit :: FilePath -> FilePath -> IO ()
testPluginExit workspace executable = do
  runtime <- newRuntime (testConfig workspace executable)
  assertWorkflowError
    "post-spawn failure of any plugin method is outcome unknown"
    (\case PluginOutcomeUnknown "bad-exit" "probe" _ -> True; _ -> False)
    (callPlugin runtime "bad-exit" [Text.pack executable, "--fake-exit"] "probe" mempty)
  assertWorkflowError
    "reported failure wins over non-zero exit"
    (\case PluginReportedFailure "reported-exit" "probe" _ -> True; _ -> False)
    (callPlugin runtime "reported-exit" [Text.pack executable, "--fake-reported-exit"] "probe" mempty)
  assertWorkflowError
    "invoke transport failure is outcome unknown"
    (\case PluginOutcomeUnknown "bad-invoke" "invoke" _ -> True; _ -> False)
    (callPlugin runtime "bad-invoke" [Text.pack executable, "--fake-exit"] "invoke" mempty)
  assertWorkflowError
    "reported outcome unknown retains its classification for a generic method"
    (\case PluginOutcomeUnknown "reported-unknown" "compute" cause -> "timeout" `Text.isInfixOf` workflowCauseMessage cause; _ -> False)
    (callPlugin runtime "reported-unknown" [Text.pack executable, "--fake-outcome-unknown"] "compute" mempty)
  assertWorkflowError
    "empty plugin method"
    (\case PluginProtocolFailed "empty-method" _ -> True; _ -> False)
    (callPlugin runtime "empty-method" [Text.pack executable, "--fake-plugin"] "" mempty)
  assertWorkflowError
    "outbound request uses the strict JSON domain"
    (\case PluginProtocolFailed "bad-json" _ -> True; _ -> False)
    ( callPlugin
        runtime
        "bad-json"
        [Text.pack executable, "--fake-plugin"]
        "probe"
        (KeyMap.singleton "number" (Number (scientific (10 ^ (1000 :: Int) + 1) (-1))))
    )

testPluginPipeBackpressure :: FilePath -> FilePath -> IO ()
testPluginPipeBackpressure workspace executable = do
  runtime <- newRuntime (testConfig workspace executable)
  let largeInput = Text.replicate (1024 * 1024) "x"
  result <-
    timeout 5000000 $
      callPlugin
        runtime
        "backpressure"
        [Text.pack executable, "--fake-backpressure"]
        "probe"
        (KeyMap.singleton "blob" (String largeInput))
  case result of
    Just pluginResult -> assertEqual "backpressure result" (String "ok") (pluginCallValue pluginResult)
    Nothing -> failTest "plugin pipe exchange deadlocked"

testPluginEventStreaming :: FilePath -> FilePath -> IO ()
testPluginEventStreaming workspace executable = do
  eventSeen <- newEmptyMVar
  terminalSeen <- newEmptyMVar
  runtime <-
    newRuntimeWithSink (testConfig workspace executable) . EventSink $ \record ->
      case record of
        PluginEventRecord "stream" _ event -> do
          _ <- tryPutMVar eventSeen event
          pure ()
        _ -> pure ()
  _ <- forkIO $ do
    outcome <-
      try
        (callPlugin runtime "stream" [Text.pack executable, "--fake-stream"] "probe" mempty) :: IO (Either SomeException PluginCallResult)
    putMVar terminalSeen outcome
  maybeEvent <- timeout 1000000 (takeMVar eventSeen)
  event <- case maybeEvent of
    Nothing -> failTest "stream event was not emitted promptly"
    Just value -> pure value
  message <-
    parseFakeParams
      (withObject "event frame" $ \frame -> frame .: "event" >>= withObject "event" (.: "message"))
      event :: IO Text
  assertEqual "UTF-8 event split across reads" "跨块🌊" message
  prematureTerminal <- tryReadMVar terminalSeen
  case prematureTerminal of
    Nothing -> pure ()
    Just _ -> failTest "terminal completed before the event sink observed the event"
  completed <- timeout 3000000 (takeMVar terminalSeen)
  case completed of
    Just (Right result) -> assertEqual "stream terminal result" (String "done") (pluginCallValue result)
    Just (Left exception) -> failTest $ "stream call failed: " <> show exception
    Nothing -> failTest "stream plugin did not complete"

testRuntimeTransitions :: FilePath -> FilePath -> IO ()
testRuntimeTransitions workspace executable = do
  runtime <-
    newRuntimeWithSink
      (testConfig workspace executable)
      (EventSink (const (pure ())))
  let addPlugin = jsonPlugin "calculator" "add" :: Plugin (Int, Int) Int
  result <- runWorkflow runtime (call addPlugin (19, 23))
  assertEqual "typed result" 42 result
  records <- readRuntimeRecords runtime
  let transitions =
        [ transition
          | RuntimeTransitionRecord transition <- records,
            "plugin." `Text.isPrefixOf` stateTransitionCode transition
        ]
  case transitions of
    started : completed : _ -> do
      assertEqual "start state_before" "ready" (stateTransitionStateBefore started)
      assertEqual "start state_after" "running" (stateTransitionStateAfter started)
      assertEqual "start trigger kind" RequestTrigger (transitionTriggerKind (stateTransitionTrigger started))
      assertEqual "start guard passed" True (transitionGuardPassed (stateTransitionGuard started))
      unless (not (Text.null (transitionGuardCondition (stateTransitionGuard started)))) $
        failTest "transition guard condition must be present"
      unless (not (Text.null (transitionGuardReason (stateTransitionGuard started)))) $
        failTest "transition guard reason must be present"
      assertEqual "terminal state_before" "running" (stateTransitionStateBefore completed)
      assertEqual "terminal state_after" "succeeded" (stateTransitionStateAfter completed)
      assertEqual "terminal trigger kind" InternalResultTrigger (transitionTriggerKind (stateTransitionTrigger completed))
      case toJSON started of
        Object fields -> do
          let transitionFields =
                [ "type",
                  "code",
                  "level",
                  "message",
                  "subject",
                  "state_before",
                  "trigger",
                  "guard",
                  "state_after",
                  "context"
                ]
          unless (KeyMap.size fields == length transitionFields && all (`KeyMap.member` fields) transitionFields) $
            failTest "persistent transition JSON drifted from its fixed schema"
          assertEqual "transition presentation category" (Just (String "state")) (KeyMap.lookup "level" fields)
          case KeyMap.lookup "trigger" fields of
            Just (Object triggerFields) -> do
              let triggerSchema = ["kind", "source", "code", "details"]
              unless (KeyMap.size triggerFields == length triggerSchema && all (`KeyMap.member` triggerFields) triggerSchema) $
                failTest "transition trigger drifted from kind/source/code/details"
              unless (not (KeyMap.member "detail" triggerFields)) $
                failTest "transition trigger retained the obsolete singular detail field"
            other -> failTest $ "transition trigger did not encode as an object: " <> show other
          case KeyMap.lookup "guard" fields of
            Just (Object guardFields) -> do
              let guardSchema = ["condition", "passed", "reason"]
              unless (KeyMap.size guardFields == length guardSchema && all (`KeyMap.member` guardFields) guardSchema) $
                failTest "transition guard drifted from condition/passed/reason"
            other -> failTest $ "transition guard did not encode as an object: " <> show other
        other -> failTest $ "transition did not encode as an object: " <> show other
    _ -> failTest $ "expected start and terminal transitions, received " <> show transitions

testCancellationClassification :: FilePath -> FilePath -> IO ()
testCancellationClassification workspace executable = do
  timeoutRuntime <- newRuntime (testConfig workspace executable)
  timeoutOutcome <-
    timeout 100000 (runWorkflow timeoutRuntime (liftIO (threadDelay 5000000)))
  assertEqual "timeout returns Nothing" Nothing timeoutOutcome
  timeoutRecords <- readRuntimeRecords timeoutRuntime
  assertCancelledOnly "timeout" timeoutRecords

  asyncRuntime <- newRuntime (testConfig workspace executable)
  started <- newEmptyMVar
  worker <-
    Async.async
      (runWorkflow asyncRuntime (liftIO (putMVar started () >> threadDelay 5000000)))
  takeMVar started
  Async.cancel worker
  asyncOutcome <- Async.waitCatch worker
  case asyncOutcome of
    Left exception ->
      case fromException exception :: Maybe Async.AsyncCancelled of
        Just Async.AsyncCancelled -> pure ()
        Nothing -> failTest $ "async cancellation changed exception: " <> show exception
    Right () -> failTest "async cancellation produced a successful workflow"
  asyncRecords <- readRuntimeRecords asyncRuntime
  assertCancelledOnly "Async.cancel" asyncRecords
  where
    assertCancelledOnly label records = do
      let transitionCodes =
            [ stateTransitionCode transition
              | RuntimeTransitionRecord transition <- records
            ]
      unless ("workflow.control.cancelled" `elem` transitionCodes) $
        failTest $ label <> " did not record the cancelled transition"
      unless
        (not (any (`elem` transitionCodes) ["workflow.result.error", "workflow.result.exception"])) $
        failTest $ label <> " also recorded a failed transition"

testHumanProjection :: IO ()
testHumanProjection = do
  let rawEvent =
        PluginEventRecord
          "provider:fake"
          "clef-1"
          (object ["event" .= object ["type" .= ("provider.raw" :: Text), "secret" .= (42 :: Int)]])
      pluginDiagnostic = PluginDiagnosticRecord "provider:fake" "raw stderr {\"secret\":true}"
      normalizedMessage =
        RuntimeMessageRecord
          RuntimeMessage
            { runtimeMessageCode = "workflow.progress",
              runtimeMessageLevel = InfoLevel,
              runtimeMessageText = "The workflow is preparing its next step.",
              runtimeMessageContext = mempty
            }
  assertEqual "raw provider events have no default projection" Nothing (renderRuntimeRecord rawEvent)
  assertEqual "plugin stderr has no default projection" Nothing (renderRuntimeRecord pluginDiagnostic)
  case renderRuntimeRecord normalizedMessage of
    Just line -> do
      assertEqual "normalized info line" "[info] The workflow is preparing its next step." line
      unless (not ("{" `Text.isInfixOf` line)) $ failTest "human projection exposed structured JSON"
    Nothing -> failTest "normalized message was not projected"

testOutcomeUnknownRendering :: IO ()
testOutcomeUnknownRendering = do
  let cause =
        WorkflowCause
          { workflowCauseCode = "outcome_unknown",
            workflowCauseMessage = "the provider event channel closed",
            workflowCauseDetails = Just (object ["cause" .= ("frame_limit" :: Text), "frames" .= (10000 :: Int)])
          }
      workflowError = PluginOutcomeUnknown "provider:fake" "invoke" cause
      rendered = renderWorkflowError workflowError
      structured = LazyByteString.toStrict (encode (workflowErrorDiagnostic workflowError))
  unless ("did not retry" `Text.isInfixOf` rendered) $
    failTest "outcome unknown did not explain the retry policy"
  unless (not ("{" `Text.isInfixOf` rendered)) $
    failTest "outcome unknown rendered its structured details to the user"
  unless (ByteString.Char8.pack "frame_limit" `ByteString.isInfixOf` structured) $
    failTest "outcome unknown lost its structured diagnostic cause"

testRunTactusPresentation :: FilePath -> FilePath -> IO ()
testRunTactusPresentation workspace executable = do
  configPath <- vacantTemporaryPath workspace "clef-runtime-*.json"
  unknownConfigPath <- vacantTemporaryPath workspace "clef-runtime-unknown-*.json"
  diagnosticPath <- vacantTemporaryPath workspace "clef-diagnostics-*.jsonl"
  let makeConfig command =
        object
          [ "api" .= ("clef.runtime/v1" :: Text),
            "workspace" .= workspace,
            "default_provider" .= ("fake" :: Text),
            "providers"
              .= object
                [ "fake"
                    .= object
                      [ "command" .= command,
                        "options" .= object []
                      ]
                ],
            "effects" .= object [],
            "plugins" .= object [],
            "instructions" .= ("" :: Text)
          ]
      config = makeConfig [executable, "--fake-plugin"]
      unknownConfig = makeConfig [executable, "--fake-outcome-unknown"]
  LazyByteString.writeFile configPath (encode config)
  LazyByteString.writeFile unknownConfigPath (encode unknownConfig)
  ( (failureExit, _, failureError),
    (successExit, _, successError),
    (unknownExit, _, unknownError),
    (unexpectedExit, _, unexpectedError),
    diagnosticRecords,
    asyncOutcome,
    timeoutOutcome
    ) <-
    ( do
        (failed, succeeded, unknown, unexpected, records) <-
          withEnvironment [("TACTUS_DIAGNOSTIC_PATH", diagnosticPath)] $ do
            failed <- readProcessWithExitCode executable ["--fake-cli-error", configPath] ""
            succeeded <- readProcessWithExitCode executable ["--fake-cli-provider", configPath] ""
            unknown <- readProcessWithExitCode executable ["--fake-cli-unknown", unknownConfigPath] ""
            unexpected <- readProcessWithExitCode executable ["--fake-cli-unexpected", configPath] ""
            encodedRecords <- LazyByteString.readFile diagnosticPath
            records <-
              forM
                (filter (not . LazyByteString.null) (LazyChar8.lines encodedRecords))
                ( \line -> case eitherDecodeStrict' (LazyByteString.toStrict line) of
                    Left message -> failTest $ "invalid Clef diagnostic JSONL: " <> message
                    Right value -> pure value
                )
            pure (failed, succeeded, unknown, unexpected, records)
        asynchronous <-
          withEnvironment [("TACTUS_RUNTIME_CONFIG", configPath)] $
            (try (runTactus (liftIO (throwIO ThreadKilled))) :: IO (Either SomeException ()))
        timedOut <-
          withEnvironment [("TACTUS_RUNTIME_CONFIG", configPath)] $
            timeout 100000 (runTactus (liftIO (threadDelay 5000000)))
        pure (failed, succeeded, unknown, unexpected, records, asynchronous, timedOut)
    )
      `finally` mapM_ removeIfPresent [configPath, unknownConfigPath, diagnosticPath]
  assertEqual "expected workflow error exit" (ExitFailure 1) failureExit
  unless ("[error] workflow requirement failed" `Text.isInfixOf` Text.pack failureError) $
    failTest $ "runTactus did not use the human error projection: " <> failureError
  unless (not ("HasCallStack" `Text.isInfixOf` Text.pack failureError)) $
    failTest "runTactus exposed a Haskell HasCallStack"
  unless (not ("called at" `Text.isInfixOf` Text.pack failureError)) $
    failTest "runTactus exposed a Haskell source call site"
  assertEqual "provider workflow exit" ExitSuccess successExit
  let presentationLines = filter (not . Text.null) (Text.lines (Text.pack successError))
      hasAllowedTag line = any (`Text.isPrefixOf` line) ["[state] ", "[info] ", "[warning] ", "[error] "]
  unless (not (null presentationLines) && all hasAllowedTag presentationLines) $
    failTest $ "provider workflow emitted a non-presentation line: " <> successError
  unless (not ("provider.raw" `Text.isInfixOf` Text.pack successError)) $
    failTest "provider workflow exposed a raw provider event"
  unless (not ("{" `Text.isInfixOf` Text.pack successError)) $
    failTest "provider workflow exposed structured JSON"
  assertEqual "outcome-unknown workflow exit" (ExitFailure 1) unknownExit
  let unknownText = Text.pack unknownError
  unless ("[state] " `Text.isInfixOf` unknownText && "outcome-unknown state" `Text.isInfixOf` unknownText) $
    failTest $ "outcome unknown did not expose its state transition: " <> unknownError
  unless ("[warning] " `Text.isInfixOf` unknownText) $
    failTest $ "outcome unknown did not expose a warning: " <> unknownError
  unless (not ("[error] " `Text.isInfixOf` unknownText)) $
    failTest $ "outcome unknown was incorrectly presented as an error: " <> unknownError
  unless (all (\marker -> not (marker `Text.isInfixOf` unknownText)) ["HasCallStack", "called at", "{"]) $
    failTest $ "outcome unknown leaked technical diagnostics: " <> unknownError
  assertEqual "unexpected synchronous exception exit" (ExitFailure 1) unexpectedExit
  let unexpectedText = Text.pack unexpectedError
      unexpectedLines = filter (not . Text.null) (Text.lines unexpectedText)
      unexpectedErrorLines = filter ("[error] " `Text.isPrefixOf`) unexpectedLines
  unless
    ( "[error] Workflow execution stopped because of an unexpected Haskell runtime error." `Text.isInfixOf` unexpectedText
        && all hasAllowedTag unexpectedLines
        && length unexpectedErrorLines == 1
    ) $
    failTest $ "unexpected synchronous exception did not use one concise human projection: " <> unexpectedError
  unless
    (all (\marker -> not (marker `Text.isInfixOf` unexpectedText)) ["UNSAFE_SYNC_EXCEPTION", "HasCallStack", "called at", "{"]) $
    failTest $ "unexpected synchronous exception leaked technical details: " <> unexpectedError
  let transitionObjects =
        [ objectValue
          | Object objectValue <- diagnosticRecords,
            KeyMap.lookup "type" objectValue == Just (String "state_transition")
        ]
      allowedRecord value = case value of
        Object objectValue ->
          KeyMap.lookup "type" objectValue
            `elem` [Just (String "state_transition"), Just (String "message")]
        _ -> False
  unless (not (null transitionObjects) && all allowedRecord diagnosticRecords) $
    failTest "Clef sidecar contained no transitions or included raw runtime records"
  unless
    ( all
        (\objectValue -> all (`KeyMap.member` objectValue) ["state_before", "trigger", "guard", "state_after"])
        transitionObjects
    ) $
    failTest "Clef sidecar transition omitted one of the four required fields"
  case asyncOutcome of
    Left exception -> case fromException exception :: Maybe AsyncException of
      Just ThreadKilled -> pure ()
      other -> failTest $ "runTactus changed the asynchronous exception: " <> show other
    Right () -> failTest "runTactus swallowed an asynchronous exception"
  assertEqual "runTactus preserves timeout cancellation" Nothing timeoutOutcome

fakeCliError :: FilePath -> IO ()
fakeCliError configPath = do
  setEnv "TACTUS_RUNTIME_CONFIG" configPath
  _ <- runTactus (requireBecause "expected presentation failure" (const False) ())
  pure ()

fakeCliProvider :: FilePath -> IO ()
fakeCliProvider configPath = do
  setEnv "TACTUS_RUNTIME_CONFIG" configPath
  let selectedProvider =
        (providerRef "fake") {providerRefExtraArgs = ["--adapter-owned-flag"]}
  _ <- runTactus (invokeWith selectedProvider (textTask "echo" id) "provider output stays structured")
  pure ()

fakeCliUnknown :: FilePath -> IO ()
fakeCliUnknown configPath = do
  setEnv "TACTUS_RUNTIME_CONFIG" configPath
  _ <- runTactus (invoke (textTask "unknown" id) "external result may be unknown")
  pure ()

fakeCliUnexpected :: FilePath -> IO ()
fakeCliUnexpected configPath = do
  setEnv "TACTUS_RUNTIME_CONFIG" configPath
  _ <-
    runTactus
      (liftIO (throwIO (userError "UNSAFE_SYNC_EXCEPTION {raw-json}\nHasCallStack called at Internal.hs:1")))
  pure ()

testBlockedEventSink :: FilePath -> FilePath -> IO ()
testBlockedEventSink workspace executable = do
  releaseSink <- newEmptyMVar
  runtime <-
    newRuntimeWithSink (testConfig workspace executable) . EventSink $ \record ->
      case record of
        PluginEventRecord "plugin:calculator" _ _ -> takeMVar releaseSink
        _ -> pure ()
  let addPlugin = jsonPlugin "calculator" "add" :: Plugin (Int, Int) Int
  outcome <-
    timeout 2500000
      (try (runWorkflow runtime (call addPlugin (19, 23))) :: IO (Either WorkflowError Int))
  putMVar releaseSink ()
  case outcome of
    Just (Right 42) -> do
      records <- readRuntimeRecords runtime
      assertEqual
        "sink failure is retained as an internal diagnostic"
        1
        (length [() | RuntimeInternalDiagnosticRecord message <- records, runtimeMessageCode message == "runtime.sink_failed"])
    Just (Left other) -> failTest $ "blocked sink returned the wrong failure: " <> show other
    Just (Right value) -> failTest $ "blocked sink changed the typed value to " <> show value
    Nothing -> failTest "blocked sink did not fail within its bounded flush deadline"

testEventSinkOverload :: FilePath -> FilePath -> IO ()
testEventSinkOverload workspace executable = do
  blockFirstEvent <- newMVar ()
  releaseSink <- newEmptyMVar
  deliveredEvents <- newMVar (0 :: Int)
  terminalSeen <- newEmptyMVar
  degradationSeen <- newEmptyMVar
  runtime <-
    newRuntimeWithSink (testConfig workspace executable) . EventSink $ \record ->
      case record of
        PluginEventRecord "overload" _ _ -> do
          firstEvent <- tryTakeMVar blockFirstEvent
          case firstEvent of
            Just () -> takeMVar releaseSink
            Nothing -> pure ()
          modifyMVar_ deliveredEvents (pure . (+ 1))
        PluginValueRecord _ "overload" "complete" _ -> do
          _ <- tryPutMVar terminalSeen ()
          pure ()
        RuntimeMessageRecord message
          | runtimeMessageCode message == "runtime.sink_degraded" -> do
              _ <- tryPutMVar degradationSeen ()
              pure ()
        _ -> pure ()

  mapM_
    (\index -> recordRuntime runtime (PluginEventRecord "overload" "request" (toJSON index)))
    [1 .. (300 :: Int)]
  recordRuntime runtime (PluginValueRecord "request" "overload" "complete" Null)
  putMVar releaseSink ()
  flushResult <- timeout 2500000 (flushRuntimeSink runtime)
  assertEqual "overloaded sink flush" (Just (Right ())) flushResult
  deliveredCount <- readMVar deliveredEvents
  unless (deliveredCount < 300) $ failTest "overloaded sink did not drop low-priority events"
  assertEqual "terminal record reached overloaded sink" (Just ()) =<< tryReadMVar terminalSeen
  assertEqual "sink degradation reached overloaded sink" (Just ()) =<< tryReadMVar degradationSeen
  records <- readRuntimeRecords runtime
  assertEqual
    "sink degradation retained in runtime records"
    1
    (length [() | RuntimeMessageRecord message <- records, runtimeMessageCode message == "runtime.sink_degraded"])

testFinalSinkProjection :: FilePath -> FilePath -> IO ()
testFinalSinkProjection workspace executable = do
  projected <- newEmptyMVar
  runtime <-
    newRuntimeWithSink (testConfig workspace executable) . EventSink $ \record ->
      case record of
        PluginValueRecord _ "calculator" "add" value -> do
          _ <- tryPutMVar projected value
          pure ()
        _ -> pure ()
  let addPlugin = jsonPlugin "calculator" "add" :: Plugin (Int, Int) Int
  result <- runWorkflow runtime (call addPlugin (19, 23))
  assertEqual "typed result" 42 result
  observed <- tryReadMVar projected
  assertEqual "final typed value reached sink before return" (Just (Number 42)) observed

testGenericPlugin :: FilePath -> FilePath -> IO ()
testGenericPlugin workspace executable = do
  runtime <- newRuntime (testConfig workspace executable)
  let addPlugin = jsonPlugin "calculator" "add" :: Plugin (Int, Int) Int
  result <- runWorkflow runtime (call addPlugin (19, 23))
  assertEqual "typed generic plugin result" 42 result
  records <- readRuntimeRecords runtime
  assertEqual
    "generic plugin value retained"
    1
    (length [() | PluginValueRecord _ "calculator" "add" _ <- records])

testPluginParameterConflicts :: FilePath -> FilePath -> IO ()
testPluginParameterConflicts workspace executable = do
  pluginRuntime <- newRuntime (testConfig workspace executable)
  let conflictingPlugin = rawPlugin "calculator" "probe"
  assertWorkflowError
    "generic plugin reserved fields"
    (\case
      PluginParameterConflict "plugin:calculator" "probe" ["workspace", "options"] -> True
      _ -> False
    )
    ( runWorkflow
        pluginRuntime
        (call conflictingPlugin (object ["workspace" .= ("caller" :: Text), "options" .= object []]))
    )

  effectRuntime <- newRuntime (testConfig workspace executable)
  let conflictingEffect =
        operation
          "workspace.paths"
          "snapshot"
          (object ["options" .= object []]) :: Operation Value
  assertWorkflowError
    "effect reserved fields"
    (\case
      PluginParameterConflict "effect:workspace.paths" "snapshot" ["options"] -> True
      _ -> False
    )
    (runWorkflow effectRuntime (perform conflictingEffect))

testInvokeAndObserve :: FilePath -> FilePath -> IO ()
testInvokeAndObserve workspace executable = do
  runtime <- newRuntime (testConfig workspace executable)
  let selectedProvider =
        (providerRef "fake")
          { providerRefModel = Just "custom-model",
            providerRefEffort = Just "unbounded-effort-name",
            providerRefOptions = mempty,
            providerRefExtraArgs = ["--adapter-owned-flag"]
          }
      echoTask = textTask "echo" id
  result <- runWorkflow runtime (invokeWith selectedProvider echoTask "user prompt")
  assertEqual "instructions are prepended" "test instructions\n\nuser prompt" result
  unicodeRuntime <- newRuntime (testConfig workspace executable)
  unicodeResult <- runWorkflow unicodeRuntime (invokeWith selectedProvider echoTask "你好，工作区/路径")
  assertEqual
    "plugin transport preserves UTF-8"
    "test instructions\n\n你好，工作区/路径"
    unicodeResult
  records <- readRuntimeRecords runtime
  let providerValues = [value | ProviderValueRecord _ "fake" value <- records]
      beginEvidence = [value | EffectEvidenceRecord _ "observer" "observe.begin" value <- records]
      endEvidence = [value | EffectEvidenceRecord _ "observer" "observe.end" value <- records]
      events = [value | PluginEventRecord "provider:fake" _ value <- records]
  assertEqual "one provider value" 1 (length providerValues)
  assertEqual "one begin evidence" 1 (length beginEvidence)
  assertEqual "one end evidence" 1 (length endEvidence)
  assertEqual "provider event retained" 1 (length events)
  failedRuntime <- newRuntime (testConfig workspace executable)
  assertWorkflowError
    "provider failure"
    (\case PluginReportedFailure "provider:fake" "invoke" _ -> True; _ -> False)
    (runWorkflow failedRuntime (invoke echoTask "fail-provider"))
  failedRecords <- readRuntimeRecords failedRuntime
  assertEqual
    "observer end runs after provider failure"
    1
    (length [() | EffectEvidenceRecord _ "observer" "observe.end" _ <- failedRecords])

testObserverBeginCancellation :: FilePath -> FilePath -> IO ()
testObserverBeginCancellation workspace executable = do
  let baseConfig = testConfig workspace executable
      normalObserver =
        EffectConfig
          { effectCommand = [Text.pack executable, "--fake-plugin"],
            effectOptions = mempty,
            effectObserveInvocations = True
          }
      blockedObserver =
        EffectConfig
          { effectCommand = [Text.pack executable, "--fake-block"],
            effectOptions = mempty,
            effectObserveInvocations = True
          }
      config =
        baseConfig
          { runtimeEffects =
              Map.fromList
                [ ("a-observer", normalObserver),
                  ("z-blocked", blockedObserver)
                ]
          }
  runtime <- newRuntime config
  outcome <- timeout 500000 (runWorkflow runtime (invoke (textTask "echo" id) "never invoked"))
  assertEqual "blocked observer is cancelled" Nothing outcome
  records <- readRuntimeRecords runtime
  assertEqual
    "earlier observer is ended after begin cancellation"
    1
    (length [() | EffectEvidenceRecord _ "a-observer" "observe.end" _ <- records])

testObserverEndCancellation :: FilePath -> FilePath -> IO ()
testObserverEndCancellation workspace executable = do
  let baseConfig = testConfig workspace executable
      normalObserver =
        EffectConfig
          { effectCommand = [Text.pack executable, "--fake-plugin"],
            effectOptions = mempty,
            effectObserveInvocations = True
          }
      blockedObserver =
        EffectConfig
          { effectCommand = [Text.pack executable, "--fake-block-end"],
            effectOptions = mempty,
            effectObserveInvocations = True
          }
      config =
        baseConfig
          { runtimeEffects =
              Map.fromList
                [ ("a-observer", normalObserver),
                  ("b-blocked", blockedObserver),
                  ("c-observer", normalObserver)
                ]
          }
  runtime <- newRuntime config
  let selectedProvider =
        (providerRef "fake") {providerRefExtraArgs = ["--adapter-owned-flag"]}
  outcome <-
    timeout 500000 $
      runWorkflow runtime (invokeWith selectedProvider (textTask "echo" id) "action completes")
  assertEqual "blocked observer end is cancelled" Nothing outcome
  records <- readRuntimeRecords runtime
  assertEqual
    "observers after the cancelled end are still attempted"
    ["c-observer", "a-observer"]
    [name | EffectEvidenceRecord _ name "observe.end" _ <- records]

testWorkspaceOperations :: FilePath -> FilePath -> IO ()
testWorkspaceOperations workspace executable = do
  runtime <- newRuntime (testConfig workspace executable)
  before <- runWorkflow runtime (perform WorkspacePaths.snapshot)
  after <- runWorkflow runtime (perform WorkspacePaths.snapshot)
  changes <- runWorkflow runtime (perform (WorkspacePaths.diff before after))
  forgotten <- runWorkflow runtime (perform (WorkspacePaths.forget before))
  assertEqual "snapshot id" "snapshot-1" (WorkspacePaths.workspaceSnapshotId before)
  assertEqual "added paths" ["created.txt"] (WorkspacePaths.workspaceAddedPaths changes)
  assertEqual "forget result" True (WorkspacePaths.workspaceSnapshotForgotten forgotten)

segnoCounterTask :: PersistentTask Int Int Int
segnoCounterTask =
  persistentTask "minute-counter" selectedTrigger selectedState $ \occurrence handle -> do
    checkpointed <- checkpoint (CheckpointId "record-window") handle (occurrencePayload occurrence)
    case checkpointed of
      Left conflict ->
        pure . Fail $
          TaskFailure
            { taskFailureCode = "state_conflict",
              taskFailureMessage = Text.pack (show conflict),
              taskFailureDetails = Nothing
            }
      Right nextHandle -> pure (Complete (KeepState nextHandle) (occurrencePayload occurrence))
  where
    selectedState = state (StateKey "window-count") (SchemaVersion 2) 0
    selectedTrigger =
      gate (\counter tick -> tick > counter) . mapTrigger (+ 1) . filterTrigger (> 0) $
        triggerSource
          (TriggerId "each-minute")
          "time.interval"
          (object ["milliseconds" .= ("60000" :: Text)])

testSegnoManifest :: IO ()
testSegnoManifest = do
  manifest <- either throwIO pure (taskManifest segnoCounterTask)
  (api, taskName, sourceCount, backend, initialValue) <-
    parseFakeParams
      ( withObject "task manifest" $ \fields -> do
          api <- fields .: "api"
          taskName <- fields .: "task"
          sources <- fields .: "sources"
          stateValue <- fields .: "state"
          (backend, initialValue) <-
            withObject "state manifest" (\stateFields -> (,) <$> stateFields .: "backend" <*> stateFields .: "initial") stateValue
          pure (api, taskName, length (sources :: [Value]), backend, initialValue)
      )
      manifest :: IO (Text, Text, Int, Text, Int)
  assertEqual "manifest api" "agenstro.segno.task/v1" api
  assertEqual "manifest task" "minute-counter" taskName
  assertEqual "manifest source count" 1 sourceCount
  assertEqual "manifest default state backend" "segno.state" backend
  assertEqual "manifest initial state" 0 initialValue
  let duplicateTrigger =
        mergeTrigger
          (triggerSource (TriggerId "same") "manual" (object []) :: Trigger Int Int)
          (triggerSource (TriggerId "same") "manual" (object []) :: Trigger Int Int)
      duplicateTask :: PersistentTask Int Int Int
      duplicateTask = persistentTask "duplicate" duplicateTrigger (state (StateKey "counter") (SchemaVersion 1) (0 :: Int)) (\_ _ -> pure Ignore)
  case taskManifest duplicateTask of
    Left (InvalidSegnoDefinition message) ->
      unless ("unique" `Text.isInfixOf` message) (failTest "duplicate trigger error lost its reason")
    other -> failTest $ "duplicate trigger identities should fail, received " <> showManifestResult other

testSegnoDescribe :: FilePath -> IO ()
testSegnoDescribe workspace = do
  resultPath <- vacantTemporaryPath workspace "segno-describe-result.json"
  let environment =
        [ ("SEGNO_MODE", "describe"),
          ("SEGNO_RESULT_PATH", resultPath)
        ]
  flip finally (removeIfPresent resultPath) $ do
    withEnvironment environment (runPersistentTask segnoCounterTask)
    encoded <- ByteString.readFile resultPath
    actual <- case eitherDecodeStrict' encoded of
      Left message -> failTest $ "Segno describe result is not JSON: " <> message
      Right value -> pure value
    expected <- either throwIO pure (taskManifest segnoCounterTask)
    assertEqual "describe result" expected actual

testSegnoExecute :: FilePath -> FilePath -> IO ()
testSegnoExecute workspace executable = do
  invocationPath <- temporaryFile workspace "segno-invocation.json"
  runtimePath <- temporaryFile workspace "segno-runtime.json"
  resultPath <- vacantTemporaryPath workspace "segno-result.json"
  LazyByteString.writeFile invocationPath (encode (segnoInvocation 0 1))
  LazyByteString.writeFile runtimePath (encode (segnoRuntimeDocument workspace executable))
  let environment =
        [ ("SEGNO_MODE", "execute"),
          ("SEGNO_INVOCATION_PATH", invocationPath),
          ("SEGNO_RESULT_PATH", resultPath),
          ("TACTUS_RUNTIME_CONFIG", runtimePath)
        ]
      cleanup = mapM_ removeIfPresent [invocationPath, runtimePath, resultPath]
  flip finally cleanup $ do
    withEnvironment environment (runPersistentTask segnoCounterTask)
    encoded <- ByteString.readFile resultPath
    result <- case eitherDecodeStrict' encoded of
      Left message -> failTest $ "Segno result is not JSON: " <> message
      Right value -> pure value
    (api, occurrenceIdentity, kind, transitionKind, revisionValue, output) <-
      parseFakeParams parseSegnoResult result :: IO (Text, Text, Text, Text, Maybe Text, Int)
    assertEqual "execute result api" "agenstro.segno.result/v1" api
    assertEqual "execute occurrence" "occ-minute-1" occurrenceIdentity
    assertEqual "execute decision" "complete" kind
    assertEqual "execute state transition" "keep" transitionKind
    assertEqual "checkpoint revision flows into final transition" (Just "checkpoint-1") revisionValue
    assertEqual "mapped trigger payload reaches workflow" 2 output

testSegnoGate :: FilePath -> IO ()
testSegnoGate workspace = do
  invocationPath <- temporaryFile workspace "segno-gated-invocation.json"
  resultPath <- vacantTemporaryPath workspace "segno-gated-result.json"
  LazyByteString.writeFile invocationPath (encode (segnoInvocation 10 1))
  let environment =
        [ ("SEGNO_MODE", "execute"),
          ("SEGNO_INVOCATION_PATH", invocationPath),
          ("SEGNO_RESULT_PATH", resultPath),
          ("TACTUS_RUNTIME_CONFIG", workspace <> "/intentionally-missing-segno-runtime.json")
        ]
      cleanup = mapM_ removeIfPresent [invocationPath, resultPath]
  flip finally cleanup $ do
    withEnvironment environment (runPersistentTask segnoCounterTask)
    encoded <- ByteString.readFile resultPath
    result <- case eitherDecodeStrict' encoded of
      Left message -> failTest $ "gated Segno result is not JSON: " <> message
      Right value -> pure value
    kind <-
      parseFakeParams
        ( withObject "Segno result" $ \fields -> do
            decision <- fields .: "decision"
            withObject "decision" (.: "kind") decision
        )
        result :: IO Text
    assertEqual "gate rejection becomes Ignore" "ignore" kind

segnoInvocation :: Int -> Int -> Value
segnoInvocation storedValue payloadValue =
  object
    [ "api" .= ("agenstro.segno.invocation/v1" :: Text),
      "task" .= ("minute-counter" :: Text),
      "attempt" .= (1 :: Int),
      "fencing_token" .= ("fence-1" :: Text),
      "trigger"
        .= object
          [ "trigger_id" .= ("each-minute" :: Text),
            "occurrence_id" .= ("occ-minute-1" :: Text),
            "logical_time" .= ("2026-08-16T12:00:00Z" :: Text),
            "observed_time" .= ("2026-08-16T12:00:01Z" :: Text),
            "cursor" .= object ["tick" .= (1 :: Int)],
            "idempotency_key" .= ("minute:2026-08-16T12:00:00Z" :: Text),
            "payload" .= payloadValue
          ],
      "state"
        .= object
          [ "key" .= ("window-count" :: Text),
            "revision" .= (Just "state-1" :: Maybe Text),
            "schema_version" .= (2 :: Int),
            "value" .= storedValue
          ]
    ]

segnoRuntimeDocument :: FilePath -> FilePath -> Value
segnoRuntimeDocument workspace executable =
  object
    [ "api" .= ("clef.runtime/v1" :: Text),
      "workspace" .= workspace,
      "default_provider" .= ("fake" :: Text),
      "providers"
        .= object
          [ "fake"
              .= object
                [ "command" .= [executable, "--fake-plugin"],
                  "options" .= object []
                ]
          ],
      "effects" .= object [],
      "plugins"
        .= object
          [ "segno.state"
              .= object
                [ "command" .= [executable, "--fake-plugin"],
                  "options" .= object []
                ]
          ],
      "instructions" .= ("" :: Text)
    ]

parseSegnoResult :: Value -> Parser (Text, Text, Text, Text, Maybe Text, Int)
parseSegnoResult = withObject "Segno result" $ \fields -> do
  api <- fields .: "api"
  occurrenceIdentity <- fields .: "occurrence_id"
  decision <- fields .: "decision"
  (kind, transitionKind, revisionValue, output) <-
    withObject "decision" (\decisionFields -> do
      kind <- decisionFields .: "kind"
      transition <- decisionFields .: "state"
      (transitionKind, revisionValue) <-
        withObject "state transition" (\stateFields -> (,) <$> stateFields .: "kind" <*> stateFields .:? "expected_revision") transition
      output <- decisionFields .: "output"
      pure (kind, transitionKind, revisionValue, output)) decision
  pure (api, occurrenceIdentity, kind, transitionKind, revisionValue, output)

temporaryFile :: FilePath -> String -> IO FilePath
temporaryFile directory template = do
  (path, handle) <- openBinaryTempFile directory template
  hClose handle
  pure path

vacantTemporaryPath :: FilePath -> String -> IO FilePath
vacantTemporaryPath directory template = do
  path <- temporaryFile directory template
  removeFile path
  pure path

withEnvironment :: [(String, String)] -> IO value -> IO value
withEnvironment assignments action = bracket capture restore $ \_ -> do
  mapM_ (uncurry setEnv) assignments
  action
  where
    capture = mapM captureOne assignments
    captureOne (name, _) = do
      previous <- lookupEnv name
      pure (name, previous)
    restore = mapM_ $ \(name, previous) -> case previous of
      Nothing -> unsetEnv name
      Just value -> setEnv name value

removeIfPresent :: FilePath -> IO ()
removeIfPresent path = removeFile path `catch` (\(_ :: SomeException) -> pure ())

showManifestResult :: Either SegnoError Value -> String
showManifestResult (Left errorValue) = show errorValue
showManifestResult (Right value) = show value

testConfig :: FilePath -> FilePath -> RuntimeConfig
testConfig workspace executable =
  RuntimeConfig
    { runtimeApi = "clef.runtime/v1",
      runtimeWorkspace = workspace,
      runtimeDefaultProvider = "fake",
      runtimeProviders =
        Map.singleton
          "fake"
          ProviderConfig
            { providerCommand = pluginCommand,
              providerModel = Just "default-model",
              providerEffort = Just "default-effort",
              providerOptions = mempty
            },
      runtimeEffects =
        Map.fromList
          [ ( "observer",
              EffectConfig
                { effectCommand = pluginCommand,
                  effectOptions = mempty,
                  effectObserveInvocations = True
                }
            ),
            ( "workspace.paths",
              EffectConfig
                { effectCommand = pluginCommand,
                  effectOptions = mempty,
                  effectObserveInvocations = False
                }
            )
          ],
      runtimePlugins =
        Map.fromList
          [ ( "calculator",
              PluginConfig
                { pluginCommand = pluginCommand,
                  pluginOptions = mempty
                }
            )
          ],
      runtimeInstructions = "test instructions"
    }
  where
    pluginCommand = [Text.pack executable, "--fake-plugin"]

fakePlugin :: IO ()
fakePlugin = do
  requestLine <- ByteString.Char8.getLine
  request <- case eitherDecodeStrict' requestLine of
    Left message -> failTest $ "fake plugin received invalid JSON: " <> message
    Right value -> pure value
  (requestId, method, params) <-
    case
      parseEither
        ( withObject "plugin request" $ \objectValue ->
            (,,) <$> objectValue .: "id" <*> objectValue .: "method" <*> objectValue .: "params"
        )
        request of
      Left message -> failTest $ "fake plugin received invalid request: " <> message
      Right fields -> pure fields
  emit $
    object
      [ "type" .= ("event" :: Text),
        "id" .= requestId,
        "event" .= object ["type" .= ("accepted" :: Text)]
      ]
  case (method :: Text) of
    "invoke" -> do
      (prompt, hasExtraArgs) <- case params of
        Object objectValue -> case parseEither (.: "prompt") objectValue of
          Left message -> failTest message
          Right value -> pure (value, KeyMap.member "extra_args" objectValue)
        _ -> failTest "invoke params must be an object"
      if (prompt :: Text) == "test instructions\n\nfail-provider"
        then do
          unless (not hasExtraArgs) $ failTest "empty extra_args override must be omitted"
          emit $
            object
              [ "type" .= ("result" :: Text),
                "id" .= requestId,
                "ok" .= False,
                "error" .= object ["code" .= ("fake_failure" :: Text), "message" .= ("requested failure" :: Text)]
              ]
        else do
          unless hasExtraArgs $ failTest "non-empty extra_args override must be sent"
          succeed requestId (object ["text" .= prompt])
    "observe.begin" -> do
      invocation <-
        parseFakeParams (withObject "observe.begin params" (.: "invocation")) params :: IO Text
      succeed
        requestId
        (object ["token" .= ("opaque-begin" :: Text), "invocation" .= invocation])
    "observe.end" -> do
      _ <-
        parseFakeParams
          ( withObject "observe.end params" $ \objectValue ->
              (,) <$> objectValue .: "invocation" <*> objectValue .: "begin"
          )
          params :: IO (Text, Value)
      succeed
        requestId
        (object ["created" .= (["observed.txt"] :: [FilePath]), "modified" .= ([] :: [FilePath]), "deleted" .= ([] :: [FilePath])])
    "snapshot" -> do
      _ <- parseFakeParams (withObject "snapshot params" (.: "workspace")) params :: IO FilePath
      succeed requestId (object ["snapshot_id" .= ("snapshot-1" :: Text)])
    "diff" -> do
      _ <-
        parseFakeParams
          ( withObject "diff params" $ \objectValue ->
              (,) <$> objectValue .: "before" <*> objectValue .: "after"
          )
          params :: IO (Value, Value)
      succeed
        requestId
        ( object
            [ "added" .= (["created.txt"] :: [FilePath]),
              "modified" .= ([] :: [FilePath]),
              "deleted" .= ([] :: [FilePath]),
              "type_changed" .= ([] :: [FilePath])
            ]
        )
    "forget" -> do
      _ <- parseFakeParams (withObject "forget params" (.: "snapshot_id")) params :: IO Text
      succeed requestId (object ["forgotten" .= True])
    "add" -> do
      (left, right) <-
        parseFakeParams (withObject "add params" (.: "input")) params :: IO (Int, Int)
      succeed requestId (left + right)
    "compare-and-set" -> do
      (stateKey, expectedRevision, operationIdentity, occurrenceIdentity, fencingToken, fencingEpoch) <-
        parseFakeParams
          ( withObject "compare-and-set params" $ \objectValue ->
              (,,,,,)
                <$> objectValue .: "state_key"
                <*> objectValue .:? "expected_revision"
                <*> objectValue .: "operation_id"
                <*> objectValue .: "occurrence_id"
                <*> objectValue .: "fencing_token"
                <*> objectValue .: "fencing_epoch"
          )
          params :: IO (Text, Maybe Text, Text, Text, Text, Int)
      assertEqual "checkpoint state key" "window-count" stateKey
      assertEqual "checkpoint expected revision" (Just "state-1") expectedRevision
      assertEqual "checkpoint operation" "record-window" operationIdentity
      assertEqual "checkpoint occurrence" "occ-minute-1" occurrenceIdentity
      assertEqual "checkpoint fence" "fence-1" fencingToken
      assertEqual "checkpoint fence epoch" 1 fencingEpoch
      succeed requestId (object ["applied" .= True, "revision" .= ("checkpoint-1" :: Text)])
    _ ->
      emit $
        object
          [ "type" .= ("result" :: Text),
            "id" .= requestId,
            "ok" .= False,
            "error" .= object ["code" .= ("unknown_method" :: Text), "message" .= method]
          ]
  where
    succeed requestId value =
      emit $
        object
          [ "type" .= ("result" :: Text),
            "id" .= (requestId :: Text),
            "ok" .= True,
            "value" .= value
          ]

    emit value = LazyChar8.putStrLn (encode value)

fakeBlock :: IO ()
fakeBlock = do
  _ <- ByteString.Char8.getLine
  threadDelay 10000000

fakeBlockEnd :: IO ()
fakeBlockEnd = do
  requestLine <- ByteString.Char8.getLine
  request <- case eitherDecodeStrict' requestLine of
    Left message -> failTest $ "fake block-end plugin received invalid JSON: " <> message
    Right value -> pure value
  (requestId, method) <-
    parseFakeParams
      (withObject "plugin request" $ \objectValue -> (,) <$> objectValue .: "id" <*> objectValue .: "method")
      request :: IO (Text, Text)
  if method == "observe.end"
    then threadDelay 10000000
    else
      LazyChar8.putStrLn . encode $
        object
          [ "type" .= ("result" :: Text),
            "id" .= requestId,
            "ok" .= True,
            "value" .= object ["token" .= ("block-end-token" :: Text)]
          ]

fakeReportedExit :: IO ()
fakeReportedExit = do
  requestLine <- ByteString.Char8.getLine
  request <- case eitherDecodeStrict' requestLine of
    Left message -> failTest $ "fake reported-exit plugin received invalid JSON: " <> message
    Right value -> pure value
  requestId <-
    parseFakeParams (withObject "plugin request" (.: "id")) request :: IO Text
  LazyChar8.putStrLn . encode $
    object
      [ "type" .= ("result" :: Text),
        "id" .= requestId,
        "ok" .= False,
        "error" .= object ["code" .= ("reported" :: Text), "message" .= ("structured failure" :: Text)]
      ]
  exitFailure

fakeBackpressure :: IO ()
fakeBackpressure = do
  ByteString.Char8.hPutStr IO.stdout (ByteString.replicate (1024 * 1024) 32)
  IO.hFlush IO.stdout
  requestLine <- ByteString.Char8.getLine
  request <- case eitherDecodeStrict' requestLine of
    Left message -> failTest $ "fake backpressure plugin received invalid JSON: " <> message
    Right value -> pure value
  requestId <-
    parseFakeParams (withObject "plugin request" (.: "id")) request :: IO Text
  LazyChar8.putStrLn . encode $
    object
      [ "type" .= ("result" :: Text),
        "id" .= requestId,
        "ok" .= True,
        "value" .= ("ok" :: Text)
      ]

fakeStream :: IO ()
fakeStream = do
  requestLine <- ByteString.Char8.getLine
  request <- case eitherDecodeStrict' requestLine of
    Left message -> failTest $ "fake stream plugin received invalid JSON: " <> message
    Right value -> pure value
  requestId <-
    parseFakeParams (withObject "plugin request" (.: "id")) request :: IO Text
  let eventBytes =
        strictEncode $
          object
            [ "type" .= ("event" :: Text),
              "id" .= requestId,
              "event"
                .= object
                  [ "type" .= ("progress" :: Text),
                    "message" .= ("跨块🌊" :: Text)
                  ]
            ]
      marker = Text.Encoding.encodeUtf8 "🌊"
      (beforeMarker, markerAndRest) = ByteString.breakSubstring marker eventBytes
      splitPoint = ByteString.length beforeMarker + 2
      (firstChunk, secondChunk) = ByteString.splitAt splitPoint eventBytes
  unless (not (ByteString.null markerAndRest)) $ failTest "stream fixture lost its UTF-8 marker"
  ByteString.hPut IO.stdout firstChunk
  IO.hFlush IO.stdout
  threadDelay 100000
  ByteString.hPut IO.stdout (secondChunk <> ByteString.singleton 10)
  IO.hFlush IO.stdout
  threadDelay 700000
  LazyChar8.putStrLn . encode $
    object
      [ "type" .= ("result" :: Text),
        "id" .= requestId,
        "ok" .= True,
        "value" .= ("done" :: Text)
      ]

fakeOutcomeUnknown :: IO ()
fakeOutcomeUnknown = do
  requestLine <- ByteString.Char8.getLine
  request <- case eitherDecodeStrict' requestLine of
    Left message -> failTest $ "fake outcome-unknown plugin received invalid JSON: " <> message
    Right value -> pure value
  requestId <-
    parseFakeParams (withObject "plugin request" (.: "id")) request :: IO Text
  LazyChar8.putStrLn . encode $
    object
      [ "type" .= ("result" :: Text),
        "id" .= requestId,
        "ok" .= False,
        "error"
          .= object
            [ "code" .= ("outcome_unknown" :: Text),
              "message" .= ("provider timeout" :: Text),
              "details" .= object ["cause" .= ("timeout" :: Text)]
            ]
      ]
  exitFailure

strictEncode :: Value -> ByteString.ByteString
strictEncode = LazyByteString.toStrict . encode

parseFakeParams :: (Value -> Parser value) -> Value -> IO value
parseFakeParams parser value =
  case parseEither parser value of
    Left message -> failTest message
    Right result -> pure result

assertProtocolFailure :: String -> Either WorkflowError value -> IO ()
assertProtocolFailure label result = case result of
  Left (PluginProtocolFailed _ _) -> pure ()
  other -> failTest $ label <> ": expected PluginProtocolFailed, received " <> showResult other

assertWorkflowError :: forall value. String -> (WorkflowError -> Bool) -> IO value -> IO ()
assertWorkflowError label predicate action = do
  result <- try action :: IO (Either WorkflowError value)
  case result of
    Left workflowError | predicate workflowError -> pure ()
    Left workflowError -> failTest $ label <> ": unexpected WorkflowError " <> show workflowError
    Right _ -> failTest $ label <> ": expected WorkflowError, action succeeded"

assertEqual :: (Eq value, Show value) => String -> value -> value -> IO ()
assertEqual label expected actual =
  unless (expected == actual) . failTest $
    label <> ": expected " <> show expected <> ", received " <> show actual

showResult :: Either WorkflowError value -> String
showResult (Left workflowError) = show workflowError
showResult (Right _) = "Right <value>"

failTest :: String -> IO value
failTest = throwIO . userError
