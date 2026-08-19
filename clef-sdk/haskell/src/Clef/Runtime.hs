{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE ScopedTypeVariables #-}

module Clef.Runtime
  ( Runtime,
    RuntimeRecord (..),
    EventSink (..),
    RuntimeSink,
    PluginCallResult (..),
    newRuntime,
    newRuntimeWithSink,
    runtimeConfig,
    readRuntimeRecords,
    recordRuntime,
    recordRuntimeDiagnostic,
    renderRuntimeRecord,
    writeRuntimePresentation,
    flushRuntimeSink,
    freshRuntimeId,
    callPlugin,
  )
where

import Control.Concurrent.MVar
  ( MVar,
    modifyMVar,
    modifyMVar_,
    newMVar,
    readMVar,
    withMVar,
  )
import Control.Concurrent (forkIO)
import Control.Concurrent.Async (concurrently)
import Control.Concurrent.STM
  ( STM,
    TMVar,
    TVar,
    atomically,
    modifyTVar',
    newEmptyTMVarIO,
    newTVarIO,
    putTMVar,
    readTVar,
    retry,
    takeTMVar,
    writeTVar,
  )
import Control.Exception
  ( IOException,
    SomeException,
    displayException,
    finally,
    throwIO,
    try,
  )
import Control.Monad (forever, unless, when)
import Data.Aeson
  ( Object,
    ToJSON (toJSON),
    Value,
    encode,
    object,
    (.=),
  )
import qualified Data.ByteString as ByteString
import qualified Data.ByteString.Lazy as LazyByteString
import qualified Data.Aeson.KeyMap as KeyMap
import Data.Sequence (Seq, ViewL (..), (|>))
import qualified Data.Sequence as Seq
import Data.Text (Text)
import qualified Data.Text as Text
import qualified Data.Text.IO as Text.IO
import Data.Text.Encoding (decodeUtf8With)
import Data.Text.Encoding.Error (lenientDecode)
import System.Environment (lookupEnv)
import System.Exit (ExitCode (..))
import System.IO (Handle, hClose, hSetBinaryMode, stderr)
import System.IO.Unsafe (unsafePerformIO)
import System.Process
  ( CreateProcess (cwd, std_err, std_in, std_out),
    StdStream (CreatePipe),
    proc,
    waitForProcess,
    withCreateProcess,
  )
import System.Timeout (timeout)
import Clef.Diagnostic
  ( PresentationLevel (..),
    RuntimeMessage (..),
    RuntimeStateTransition (..),
    TransitionGuard (..),
    TransitionTrigger (..),
    TriggerKind (..),
    renderRuntimeMessage,
    renderStateTransition,
  )
import Clef.Error
  ( WorkflowCause (..),
    WorkflowError (..),
    workflowErrorDiagnostic,
  )
import Clef.Internal.Exception (isAsynchronousException)
import Clef.Plugin.Protocol
  ( ParsedPluginOutput (..),
    PluginFailure (..),
    PluginRequest (..),
    PluginTerminal (..),
    decodeStrictJSON,
    encodePluginRequest,
    finishPluginOutputStream,
    feedPluginOutputChunk,
    initialPluginOutputStreamParser,
  )
import Clef.Runtime.Config (RuntimeConfig (runtimeWorkspace))

data Runtime = Runtime
  { internalRuntimeConfig :: RuntimeConfig,
    internalNextRequestId :: MVar Integer,
    internalRecords :: MVar [RuntimeRecord],
    internalShouldProject :: RuntimeRecord -> Bool,
    internalSinkQueue :: TVar (Seq SinkMessage),
    internalSinkFailure :: TVar (Maybe Text),
    internalSinkDroppedEvents :: TVar Int
  }

data SinkMessage
  = ProjectRecord RuntimeRecord
  | FlushProjection (TMVar (Either Text ()))

sinkQueueCapacity :: Int
sinkQueueCapacity = 128

sinkEventQueueCapacity :: Int
sinkEventQueueCapacity = 112

sinkFlushTimeoutMicros :: Int
sinkFlushTimeoutMicros = 1000000

-- | Provider values and effect evidence deliberately use different
-- constructors.  Neither is promoted into an artifact model by the core.
data RuntimeRecord
  = RuntimeTransitionRecord RuntimeStateTransition
  | RuntimeMessageRecord RuntimeMessage
  | RuntimeInternalDiagnosticRecord RuntimeMessage
  | PluginEventRecord Text Text Value
  | PluginValueRecord Text Text Text Value
  | ProviderValueRecord Text Text Value
  | EffectEvidenceRecord Text Text Text Value
  | PluginDiagnosticRecord Text Text
  deriving (Eq, Show)

instance ToJSON RuntimeRecord where
  toJSON record = case record of
    RuntimeTransitionRecord transition -> toJSON transition
    RuntimeMessageRecord message -> toJSON message
    RuntimeInternalDiagnosticRecord message ->
      object
        [ "type" .= ("internal_diagnostic" :: Text),
          "diagnostic" .= message
        ]
    PluginEventRecord pluginName requestId event ->
      object
        [ "type" .= ("plugin_event" :: Text),
          "plugin" .= pluginName,
          "request_id" .= requestId,
          "event" .= event
        ]
    PluginValueRecord requestId pluginName method value ->
      object
        [ "type" .= ("plugin_value" :: Text),
          "plugin" .= pluginName,
          "request_id" .= requestId,
          "method" .= method,
          "value" .= value
        ]
    ProviderValueRecord requestId providerName value ->
      object
        [ "type" .= ("provider_value" :: Text),
          "provider" .= providerName,
          "request_id" .= requestId,
          "value" .= value
        ]
    EffectEvidenceRecord requestId effectName method value ->
      object
        [ "type" .= ("effect_evidence" :: Text),
          "effect" .= effectName,
          "request_id" .= requestId,
          "method" .= method,
          "value" .= value
        ]
    PluginDiagnosticRecord pluginName diagnostic ->
      object
        [ "type" .= ("plugin_diagnostic" :: Text),
          "plugin" .= pluginName,
          "diagnostic" .= diagnostic
        ]

-- | An orthogonal projection of runtime records.  A bounded runtime-owned
-- queue serializes calls to the handler, so plugin pipe readers never execute
-- user sink code.  A custom sink receives plugin events as soon as their
-- terminating LF arrives; the default human sink only queues records that it
-- can actually display.  The workflow still returns only its statically typed
-- terminal value.  A custom handler should return promptly.
newtype EventSink = EventSink
  { emitRuntimeRecord :: RuntimeRecord -> IO ()
  }

-- | Alternative name emphasizing that the sink can also receive provider
-- values, effect evidence, and diagnostics.
type RuntimeSink = EventSink

data PluginCallResult = PluginCallResult
  { pluginCallId :: Text,
    pluginCallValue :: Value,
    pluginCallEvents :: [Value]
  }
  deriving (Eq, Show)

newRuntime :: RuntimeConfig -> IO Runtime
newRuntime config = do
  sink <- newDefaultEventSink
  newRuntimeWithSinkPolicy config (maybe False (const True) . renderRuntimeRecord) sink

newRuntimeWithSink :: RuntimeConfig -> EventSink -> IO Runtime
newRuntimeWithSink config = newRuntimeWithSinkPolicy config (const True)

newRuntimeWithSinkPolicy :: RuntimeConfig -> (RuntimeRecord -> Bool) -> EventSink -> IO Runtime
newRuntimeWithSinkPolicy config shouldProject sink = do
  nextRequestId <- newMVar 1
  records <- newMVar []
  queue <- newTVarIO Seq.empty
  failure <- newTVarIO Nothing
  droppedEvents <- newTVarIO 0
  _ <- forkIO (projectSinkRecords sink queue failure)
  pure (Runtime config nextRequestId records shouldProject queue failure droppedEvents)

runtimeConfig :: Runtime -> RuntimeConfig
runtimeConfig = internalRuntimeConfig

readRuntimeRecords :: Runtime -> IO [RuntimeRecord]
readRuntimeRecords runtime = reverse <$> readMVar (internalRecords runtime)

recordRuntime :: Runtime -> RuntimeRecord -> IO ()
recordRuntime runtime record =
  modifyMVar_ (internalRecords runtime) $ \records -> do
    atomically $ enqueueSinkRecord runtime record
    pure (record : records)

-- | Retain a diagnostic without routing it through the sink that failed.
-- This is intentionally separate from 'recordRuntime': presentation and
-- journal projection failures must not recursively report through themselves.
recordRuntimeDiagnostic :: Runtime -> RuntimeMessage -> IO ()
recordRuntimeDiagnostic runtime diagnostic =
  modifyMVar_ (internalRecords runtime) $ \records ->
    pure (RuntimeInternalDiagnosticRecord diagnostic : records)

enqueueSinkRecord :: Runtime -> RuntimeRecord -> STM ()
enqueueSinkRecord runtime record
  | not (internalShouldProject runtime record) = pure ()
  | otherwise = do
      _ <- enqueueSinkMessage runtime (ProjectRecord record)
      pure ()

enqueueSinkMessage :: Runtime -> SinkMessage -> STM Bool
enqueueSinkMessage runtime message = do
  failure <- readTVar (internalSinkFailure runtime)
  case failure of
    Just _ -> pure False
    Nothing -> do
      queue <- readTVar (internalSinkQueue runtime)
      if isLowPrioritySinkMessage message && Seq.length queue >= sinkEventQueueCapacity
        then dropLowPriorityMessage >> pure True
        else
          if Seq.length queue < sinkQueueCapacity
            then writeTVar (internalSinkQueue runtime) (queue |> message) >> pure True
            else case Seq.findIndexL isLowPrioritySinkMessage queue of
              Just index -> do
                dropLowPriorityMessage
                writeTVar (internalSinkQueue runtime) (Seq.deleteAt index queue |> message)
                pure True
              Nothing -> do
                writeTVar
                  (internalSinkFailure runtime)
                  (Just "event sink queue exhausted its high-priority capacity")
                pure False
  where
    dropLowPriorityMessage = modifyTVar' (internalSinkDroppedEvents runtime) (+ 1)

isLowPrioritySinkMessage :: SinkMessage -> Bool
isLowPrioritySinkMessage (ProjectRecord PluginEventRecord {}) = True
isLowPrioritySinkMessage _ = False

readSinkMessage :: TVar (Seq SinkMessage) -> STM SinkMessage
readSinkMessage queueVariable = do
  queue <- readTVar queueVariable
  case Seq.viewl queue of
    EmptyL -> retry
    message :< remaining -> do
      writeTVar queueVariable remaining
      pure message

projectSinkRecords :: EventSink -> TVar (Seq SinkMessage) -> TVar (Maybe Text) -> IO ()
projectSinkRecords sink queue failure = forever $ do
  message <- atomically (readSinkMessage queue)
  case message of
    ProjectRecord record -> do
      currentFailure <- atomically (readTVar failure)
      case currentFailure of
        Just _ -> pure ()
        Nothing -> do
          projected <- try (emitRuntimeRecord sink record) :: IO (Either SomeException ())
          case projected of
            Right () -> pure ()
            Left exception
              | isAsynchronousException exception -> throwIO exception
              | otherwise -> atomically $ do
                let failureMessage = Text.pack ("event sink failed: " <> displayException exception)
                writeTVar failure (Just failureMessage)
    FlushProjection acknowledgement -> do
      result <- maybe (Right ()) Left <$> atomically (readTVar failure)
      atomically (putTMVar acknowledgement result)

flushRuntimeSink :: Runtime -> IO (Either Text ())
flushRuntimeSink runtime = do
  reportSinkDegradation
  result <- flushProjection
  case result of
    Right () -> pure ()
    Left message ->
      recordRuntimeDiagnostic
        runtime
        RuntimeMessage
          { runtimeMessageCode = "runtime.sink_failed",
            runtimeMessageLevel = WarningLevel,
            runtimeMessageText = "The runtime presentation sink failed; the workflow outcome was preserved.",
            runtimeMessageContext = KeyMap.singleton "cause" (toJSON message)
          }
  pure result
  where
    reportSinkDegradation = do
      droppedEvents <- atomically $ do
        count <- readTVar (internalSinkDroppedEvents runtime)
        writeTVar (internalSinkDroppedEvents runtime) 0
        pure count
      when (droppedEvents > 0) . recordRuntime runtime . RuntimeMessageRecord $
        RuntimeMessage
          { runtimeMessageCode = "runtime.sink_degraded",
            runtimeMessageLevel = WarningLevel,
            runtimeMessageText =
              "The runtime sink dropped low-priority plugin events under load; terminal records were preserved.",
            runtimeMessageContext = KeyMap.singleton "events_dropped" (toJSON droppedEvents)
          }

    flushProjection = do
      existingFailure <- atomically (readTVar (internalSinkFailure runtime))
      case existingFailure of
        Just message -> pure (Left message)
        Nothing -> do
          acknowledgement <- newEmptyTMVarIO
          queued <- atomically (enqueueSinkMessage runtime (FlushProjection acknowledgement))
          if queued
            then do
              completed <- timeout sinkFlushTimeoutMicros (atomically (takeTMVar acknowledgement))
              case completed of
                Nothing -> failFlush "event sink did not finish within one second"
                Just projectionResult -> pure projectionResult
            else do
              failure <- atomically (readTVar (internalSinkFailure runtime))
              pure (Left (maybe "event sink queue did not accept a flush marker" id failure))

    failFlush message = do
      atomically (writeTVar (internalSinkFailure runtime) (Just message))
      pure (Left message)

callPlugin :: Runtime -> Text -> [Text] -> Text -> Object -> IO PluginCallResult
callPlugin runtime pluginName command method params = do
  when (Text.null method) . throwWorkflow $
    PluginProtocolFailed pluginName "request method must be non-empty"
  requestId <- freshRuntimeId runtime
  let request = PluginRequest requestId method params
      encodedRequest = LazyByteString.toStrict (encodePluginRequest request)
  case decodeStrictJSON encodedRequest :: Either String Value of
    Left message ->
      throwWorkflow . PluginProtocolFailed pluginName $
        "request is outside the agenstro.plugin/v1 JSON domain: " <> Text.pack message
    Right _ -> pure ()
  let requestInput = encodedRequest <> ByteString.singleton 10
  (executable, arguments) <- case command of
    [] -> throwWorkflow $ PluginProcessFailed pluginName "empty command"
    firstArgument : remainingArguments ->
      pure (Text.unpack firstArgument, fmap Text.unpack remainingArguments)
  recordPluginTransition
    runtime
    pluginName
    method
    requestId
    "ready"
    "running"
    RequestTrigger
    "plugin.request.accepted"
    "clef.workflow"
    ("Plugin '" <> pluginName <> "' started operation '" <> method <> "'.")
    "The method is non-empty, the request is valid JSON, and a command is configured."
    "Clef accepted the plugin invocation request."
    Nothing
  processResult <-
    try
      ( readPluginProcess
          (proc executable arguments) {cwd = Just (runtimeWorkspace (internalRuntimeConfig runtime))}
          requestInput
          pluginName
          requestId
          (emitEvent runtime pluginName requestId)
      ) :: IO (Either IOException (ExitCode, Either WorkflowError ParsedPluginOutput, ByteString.ByteString))
  (exitCode, parsedOutput, standardErrorBytes) <- case processResult of
    Left exception -> do
      let cause =
            WorkflowCause
              { workflowCauseCode = "plugin.transport_failed",
                workflowCauseMessage = Text.pack (displayException exception),
                workflowCauseDetails = Nothing
              }
      recordPluginOutcomeUnknown runtime pluginName method requestId cause
      throwWorkflow (PluginOutcomeUnknown pluginName method cause)
    Right result -> pure result
  let standardError = decodeUtf8With lenientDecode standardErrorBytes
  unlessEmpty standardError $ \diagnostic -> do
    recordRuntime runtime (PluginDiagnosticRecord pluginName diagnostic)
  case parsedOutput of
    Left protocolError -> do
      let message = case exitCode of
            ExitSuccess -> protocolErrorMessage protocolError
            ExitFailure code ->
              exitMessage code standardError
                <> "; stdout also violated the protocol: "
                <> protocolErrorMessage protocolError
          cause =
            WorkflowCause
              { workflowCauseCode = "plugin.protocol_failed",
                workflowCauseMessage = message,
                workflowCauseDetails = Just (workflowErrorDiagnostic protocolError)
              }
      recordPluginOutcomeUnknown runtime pluginName method requestId cause
      throwWorkflow (PluginOutcomeUnknown pluginName method cause)
    Right parsed -> do
      case pluginOutputTerminal parsed of
        -- A well-formed reported failure is the most useful result even when
        -- a cross-language adapter also uses a non-zero process exit.
        PluginFailed failure -> do
          case exitCode of
            ExitSuccess -> pure ()
            ExitFailure code ->
              recordRuntime
                runtime
                ( PluginDiagnosticRecord
                    pluginName
                    ("reported a structured failure and exited with code " <> Text.pack (show code))
                )
          if pluginFailureCode failure == "outcome_unknown"
            then do
              let cause = pluginFailureCause failure
              recordPluginOutcomeUnknown runtime pluginName method requestId cause
              throwWorkflow (PluginOutcomeUnknown pluginName method cause)
            else do
              let cause = pluginFailureCause failure
              recordPluginTransition
                runtime
                pluginName
                method
                requestId
                "running"
                "failed"
                InternalResultTrigger
                "plugin.result.failure"
                pluginName
                ("Plugin '" <> pluginName <> "' operation '" <> method <> "' entered the failed state.")
                "The plugin emitted a valid terminal failure result."
                "The reported failure is authoritative."
                (Just cause)
              recordPluginMessage
                runtime
                pluginName
                method
                requestId
                "plugin.failure.reported"
                ErrorLevel
                ("Plugin '" <> pluginName <> "' operation '" <> method <> "' failed: " <> pluginFailureMessage failure)
                cause
              throwWorkflow (PluginReportedFailure pluginName method cause)
        PluginSucceeded value -> case exitCode of
          ExitFailure code -> do
            let cause =
                  WorkflowCause
                    { workflowCauseCode = "plugin.process_exit_failed",
                      workflowCauseMessage = exitMessage code standardError,
                      workflowCauseDetails = Nothing
                    }
            recordPluginOutcomeUnknown runtime pluginName method requestId cause
            throwWorkflow (PluginOutcomeUnknown pluginName method cause)
          ExitSuccess -> do
            recordPluginTransition
              runtime
              pluginName
              method
              requestId
              "running"
              "succeeded"
              InternalResultTrigger
              "plugin.result.success"
              pluginName
              ("Plugin '" <> pluginName <> "' operation '" <> method <> "' completed successfully.")
              "The plugin emitted a valid success result and exited successfully."
              "The terminal result is authoritative."
              Nothing
            pure
              PluginCallResult
                { pluginCallId = requestId,
                  pluginCallValue = value,
                  pluginCallEvents = pluginOutputEvents parsed
                }

freshRuntimeId :: Runtime -> IO Text
freshRuntimeId runtime =
  modifyMVar (internalNextRequestId runtime) $ \nextId ->
    pure (nextId + 1, "clef-" <> Text.pack (show nextId))

emitEvent :: Runtime -> Text -> Text -> Value -> IO ()
emitEvent runtime pluginName requestId event =
  recordRuntime runtime (PluginEventRecord pluginName requestId event)

recordPluginOutcomeUnknown :: Runtime -> Text -> Text -> Text -> WorkflowCause -> IO ()
recordPluginOutcomeUnknown runtime pluginName method requestId cause = do
  recordPluginTransition
    runtime
    pluginName
    method
    requestId
    "running"
    "outcome_unknown"
    InternalResultTrigger
    "plugin.result.unknown"
    pluginName
    ( "The result of plugin '"
        <> pluginName
        <> "' operation '"
        <> method
        <> "' entered the outcome-unknown state."
    )
    "No authoritative terminal outcome was available."
    "The external operation may have completed, so automatic retry is unsafe."
    (Just cause)
  recordPluginMessage
    runtime
    pluginName
    method
    requestId
    "plugin.outcome_unknown"
    WarningLevel
    ( "The external operation may have completed, so Clef did not retry plugin '"
        <> pluginName
        <> "' automatically; inspect the workspace before retrying."
    )
    cause

recordPluginTransition :: Runtime -> Text -> Text -> Text -> Text -> Text -> TriggerKind -> Text -> Text -> Text -> Text -> Text -> Maybe WorkflowCause -> IO ()
recordPluginTransition runtime pluginName method requestId stateBefore stateAfter triggerKind triggerCode triggerSource message condition reason maybeCause =
  recordRuntime
    runtime
    ( RuntimeTransitionRecord
        ( RuntimeStateTransition
            { stateTransitionCode = triggerCode,
              stateTransitionMessage = message,
              stateTransitionSubject = requestId,
              stateTransitionStateBefore = stateBefore,
              stateTransitionTrigger =
                TransitionTrigger
                  { transitionTriggerKind = triggerKind,
                    transitionTriggerSource = triggerSource,
                    transitionTriggerCode = triggerCode,
                    transitionTriggerDetails = Just ("plugin=" <> pluginName <> "; method=" <> method)
                  },
              stateTransitionGuard =
                TransitionGuard
                  { transitionGuardCondition = condition,
                    transitionGuardPassed = True,
                    transitionGuardReason = reason
                  },
              stateTransitionStateAfter = stateAfter,
              stateTransitionContext =
                maybe
                  baseContext
                  (\cause -> KeyMap.insert "cause" (toJSON cause) baseContext)
                  maybeCause
            }
        )
    )
  where
    baseContext =
      KeyMap.fromList
        [ "plugin" .= pluginName,
          "method" .= method,
          "request_id" .= requestId
        ]

recordPluginMessage :: Runtime -> Text -> Text -> Text -> Text -> PresentationLevel -> Text -> WorkflowCause -> IO ()
recordPluginMessage runtime pluginName method requestId messageCode level message cause =
  recordRuntime
    runtime
    ( RuntimeMessageRecord
        RuntimeMessage
          { runtimeMessageCode = messageCode,
            runtimeMessageLevel = level,
            runtimeMessageText = message,
            runtimeMessageContext =
              KeyMap.fromList
                [ "plugin" .= pluginName,
                  "method" .= method,
                  "request_id" .= requestId,
                  "cause" .= cause
                ]
          }
    )

pluginFailureCause :: PluginFailure -> WorkflowCause
pluginFailureCause failure =
  WorkflowCause
    { workflowCauseCode = pluginFailureCode failure,
      workflowCauseMessage = pluginFailureMessage failure,
      workflowCauseDetails = pluginFailureDetails failure
    }

diagnosticPathEnvironment :: String
diagnosticPathEnvironment = "TACTUS_DIAGNOSTIC_PATH"

-- | Construct the default projection owned by one runtime.  Human output and
-- the optional structured sidecar are observations: a sidecar write failure
-- is reported once and never prevents later human state messages or changes a
-- workflow result.
newDefaultEventSink :: IO EventSink
newDefaultEventSink = do
  sidecarDegraded <- newMVar False
  pure . EventSink $ \record ->
    withMVar presentationLock $ \_ -> do
      diagnosticPath <- lookupEnv diagnosticPathEnvironment
      case diagnosticPath of
        Just path | not (null path) -> do
          persisted <-
            try
              ( LazyByteString.appendFile
                  path
                  (encode record <> LazyByteString.singleton 10)
              ) :: IO (Either IOException ())
          case persisted of
            Right () -> pure ()
            Left _ ->
              modifyMVar_ sidecarDegraded $ \alreadyReported -> do
                unless alreadyReported $
                  Text.IO.hPutStrLn
                    stderr
                    "[warning] Persistent workflow diagnostics are unavailable; execution will continue."
                pure True
        _ -> pure ()
      maybe (pure ()) (Text.IO.hPutStrLn stderr) (renderRuntimeRecord record)

-- A process can host more than one Clef runtime.  Text output must still be
-- emitted atomically or independently serialized sink workers can interleave
-- UTF-8 and even corrupt the four user-facing tags.
{-# NOINLINE presentationLock #-}
presentationLock :: MVar ()
presentationLock = unsafePerformIO (newMVar ())

writeRuntimePresentation :: Text -> IO ()
writeRuntimePresentation line =
  withMVar presentationLock $ \_ -> Text.IO.hPutStrLn stderr line

-- | Project only typed, human-facing observations.  Raw provider events,
-- plugin stderr, values, evidence, and internal sink diagnostics remain
-- available through 'readRuntimeRecords' but never appear in default output.
renderRuntimeRecord :: RuntimeRecord -> Maybe Text
renderRuntimeRecord record = case record of
  RuntimeTransitionRecord transition -> Just (renderStateTransition transition)
  RuntimeMessageRecord message -> Just (renderRuntimeMessage message)
  _ -> Nothing

readPluginProcess :: CreateProcess -> ByteString.ByteString -> Text -> Text -> (Value -> IO ()) -> IO (ExitCode, Either WorkflowError ParsedPluginOutput, ByteString.ByteString)
readPluginProcess process requestInput pluginName requestId onEvent =
  withCreateProcess
    process
      { std_in = CreatePipe,
        std_out = CreatePipe,
        std_err = CreatePipe
      }
    $ \maybeInput maybeOutput maybeError processHandle ->
      case (maybeInput, maybeOutput, maybeError) of
        (Just inputHandle, Just outputHandle, Just errorHandle) -> do
          mapM_ (`hSetBinaryMode` True) [inputHandle, outputHandle, errorHandle]
          (_, (standardOutput, standardError)) <-
            concurrently
              (ByteString.hPut inputHandle requestInput `finally` hClose inputHandle)
              ( concurrently
                  (readPluginOutput outputHandle pluginName requestId onEvent)
                  (ByteString.hGetContents errorHandle)
              )
          exitCode <- waitForProcess processHandle
          pure (exitCode, standardOutput, standardError)
        _ -> ioError (userError "plugin process pipes were not created")

-- Read bytes rather than Text so a UTF-8 code point may span arbitrary OS
-- pipe chunks.  Only complete LF-delimited frames enter the strict protocol
-- decoder.  After the first protocol error we still drain stdout, allowing the
-- child and its stderr pipe to finish without deadlock.
readPluginOutput :: Handle -> Text -> Text -> (Value -> IO ()) -> IO (Either WorkflowError ParsedPluginOutput)
readPluginOutput handle pluginName requestId onEvent =
  go (Right initialPluginOutputStreamParser)
  where
    go parser = do
      chunk <- ByteString.hGetSome handle 32768
      if ByteString.null chunk
        then case parser >>= finishPluginOutputStream pluginName requestId of
          Left workflowError -> pure (Left workflowError)
          Right (events, output) -> mapM_ onEvent events >> pure (Right output)
        else case parser of
          Left workflowError -> go (Left workflowError)
          Right streamParser ->
            case feedPluginOutputChunk pluginName requestId streamParser chunk of
              Left workflowError -> go (Left workflowError)
              Right (nextParser, events) -> mapM_ onEvent events >> go (Right nextParser)

unlessEmpty :: Text -> (Text -> IO ()) -> IO ()
unlessEmpty value action =
  if Text.null value then pure () else action (Text.stripEnd value)

diagnosticSuffix :: Text -> Text
diagnosticSuffix standardError
  | Text.null standardError = ""
  | otherwise = "; stderr: " <> Text.strip standardError

exitMessage :: Int -> Text -> Text
exitMessage code standardError =
  "exited with code " <> Text.pack (show code) <> diagnosticSuffix standardError

protocolErrorMessage :: WorkflowError -> Text
protocolErrorMessage (PluginProtocolFailed _ message) = message
protocolErrorMessage other = Text.pack (show other)

throwWorkflow :: WorkflowError -> IO a
throwWorkflow = throwIO
