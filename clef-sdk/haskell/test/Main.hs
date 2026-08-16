{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE ScopedTypeVariables #-}
{-# LANGUAGE LambdaCase #-}

module Main (main) where

import Clef
import qualified Clef.Effect.WorkspacePaths as WorkspacePaths
import Clef.Plugin.Protocol
  ( ParsedPluginOutput (..),
    PluginTerminal (..),
    parsePluginOutput,
  )
import Control.Concurrent (forkIO, threadDelay)
import Control.Concurrent.MVar (newEmptyMVar, putMVar, takeMVar, tryPutMVar, tryReadMVar)
import Control.Exception (SomeException, throwIO, try)
import Control.Monad (forM, unless)
import Data.Aeson
  ( FromJSON (parseJSON),
    Value (..),
    encode,
    eitherDecodeStrict',
    object,
    withObject,
    (.:),
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
import System.Directory (getCurrentDirectory)
import System.Environment (getArgs, getExecutablePath)
import System.Exit (exitFailure)
import System.IO (stderr)
import qualified System.IO as IO
import System.Timeout (timeout)

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
          ("plugin process exit is checked", testPluginExit workspace executable),
          ("plugin pipes drain concurrently", testPluginPipeBackpressure workspace executable),
          ("plugin events stream before terminal across UTF-8 chunks", testPluginEventStreaming workspace executable),
          ("blocked event sinks fail boundedly without hiding plugin outcome", testBlockedEventSink workspace executable),
          ("runWorkflow flushes the final typed value projection", testFinalSinkProjection workspace executable),
          ("generic plugin call is statically typed", testGenericPlugin workspace executable),
          ("invoke records provider value and observer evidence separately", testInvokeAndObserve workspace executable),
          ("observer begin cancellation cleans up earlier observers", testObserverBeginCancellation workspace executable),
          ("observer end cancellation does not skip remaining observers", testObserverEndCancellation workspace executable),
          ("typed workspace path operations", testWorkspaceOperations workspace executable)
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
    second <- pure 22
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
    (\case PluginOutcomeUnknown "reported-unknown" "compute" message -> "timeout" `Text.isInfixOf` message; _ -> False)
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
    Just (Left (RuntimeSinkFailed _)) -> pure ()
    Just (Left other) -> failTest $ "blocked sink returned the wrong failure: " <> show other
    Just (Right value) -> failTest $ "blocked sink unexpectedly returned " <> show value
    Nothing -> failTest "blocked sink did not fail within its bounded flush deadline"

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
