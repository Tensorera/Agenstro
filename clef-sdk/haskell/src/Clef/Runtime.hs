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
  )
import Control.Concurrent (forkIO)
import Control.Concurrent.Async (concurrently)
import Control.Concurrent.STM
  ( STM,
    TBQueue,
    TMVar,
    TVar,
    atomically,
    newEmptyTMVarIO,
    newTBQueueIO,
    newTVarIO,
    putTMVar,
    readTBQueue,
    readTVar,
    takeTMVar,
    isFullTBQueue,
    writeTBQueue,
    writeTVar,
  )
import Control.Exception
  ( AsyncException,
    IOException,
    SomeException,
    displayException,
    finally,
    fromException,
    throwIO,
    try,
  )
import Control.Monad (forever, when)
import Data.Aeson (Object, Value)
import qualified Data.Aeson as Aeson
import qualified Data.ByteString as ByteString
import qualified Data.ByteString.Lazy as LazyByteString
import Data.Text (Text)
import qualified Data.Text as Text
import qualified Data.Text.IO as Text.IO
import Data.Text.Encoding (decodeUtf8, decodeUtf8With)
import Data.Text.Encoding.Error (lenientDecode)
import Numeric.Natural (Natural)
import System.Exit (ExitCode (..))
import System.IO (Handle, hClose, hSetBinaryMode, stderr)
import System.Process
  ( CreateProcess (cwd, std_err, std_in, std_out),
    StdStream (CreatePipe),
    proc,
    waitForProcess,
    withCreateProcess,
  )
import System.Timeout (timeout)
import Clef.Error (WorkflowError (..))
import Clef.Plugin.Protocol
  ( ParsedPluginOutput (..),
    PluginFailure (..),
    PluginOutputParser,
    PluginRequest (..),
    PluginTerminal (..),
    decodeStrictJSON,
    encodePluginRequest,
    finishPluginOutput,
    initialPluginOutputParser,
    parsePluginFrame,
  )
import Clef.Runtime.Config (RuntimeConfig (runtimeWorkspace))

data Runtime = Runtime
  { internalRuntimeConfig :: RuntimeConfig,
    internalNextRequestId :: MVar Integer,
    internalRecords :: MVar [RuntimeRecord],
    internalSinkQueue :: TBQueue SinkMessage,
    internalSinkFailure :: TVar (Maybe Text)
  }

data SinkMessage
  = ProjectRecord RuntimeRecord
  | FlushProjection (TMVar (Either Text ()))

sinkQueueCapacity :: Natural
sinkQueueCapacity = 128

sinkFlushTimeoutMicros :: Int
sinkFlushTimeoutMicros = 1000000

-- | Provider values and effect evidence deliberately use different
-- constructors.  Neither is promoted into an artifact model by the core.
data RuntimeRecord
  = PluginEventRecord Text Text Value
  | PluginValueRecord Text Text Text Value
  | ProviderValueRecord Text Text Value
  | EffectEvidenceRecord Text Text Text Value
  | PluginDiagnosticRecord Text Text
  deriving (Eq, Show)

-- | An orthogonal projection of runtime records.  A bounded runtime-owned
-- queue serializes calls to the handler, so plugin pipe readers never execute
-- user sink code.  Plugin events are enqueued as soon as their terminating LF
-- arrives; the workflow still returns only its statically typed terminal
-- value.  A custom handler should return promptly.
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
newRuntime config = newRuntimeWithSink config defaultEventSink

newRuntimeWithSink :: RuntimeConfig -> EventSink -> IO Runtime
newRuntimeWithSink config sink = do
  nextRequestId <- newMVar 1
  records <- newMVar []
  queue <- newTBQueueIO sinkQueueCapacity
  failure <- newTVarIO Nothing
  _ <- forkIO (projectSinkRecords sink queue failure)
  pure (Runtime config nextRequestId records queue failure)

runtimeConfig :: Runtime -> RuntimeConfig
runtimeConfig = internalRuntimeConfig

readRuntimeRecords :: Runtime -> IO [RuntimeRecord]
readRuntimeRecords runtime = reverse <$> readMVar (internalRecords runtime)

recordRuntime :: Runtime -> RuntimeRecord -> IO ()
recordRuntime runtime record =
  modifyMVar_ (internalRecords runtime) $ \records -> do
    atomically $ enqueueSinkRecord runtime record
    pure (record : records)

enqueueSinkRecord :: Runtime -> RuntimeRecord -> STM ()
enqueueSinkRecord runtime record = do
  failure <- readTVar (internalSinkFailure runtime)
  case failure of
    Just _ -> pure ()
    Nothing -> do
      full <- isFullTBQueue (internalSinkQueue runtime)
      if full
        then writeTVar (internalSinkFailure runtime) (Just "event sink queue exceeded 128 records")
        else writeTBQueue (internalSinkQueue runtime) (ProjectRecord record)

projectSinkRecords :: EventSink -> TBQueue SinkMessage -> TVar (Maybe Text) -> IO ()
projectSinkRecords sink queue failure = forever $ do
  message <- atomically (readTBQueue queue)
  case message of
    ProjectRecord record -> do
      currentFailure <- atomically (readTVar failure)
      case currentFailure of
        Just _ -> pure ()
        Nothing -> do
          projected <- try (emitRuntimeRecord sink record) :: IO (Either SomeException ())
          case projected of
            Right () -> pure ()
            Left exception -> case fromException exception :: Maybe AsyncException of
              Just _ -> throwIO exception
              Nothing -> atomically $ do
                let failureMessage = Text.pack ("event sink failed: " <> displayException exception)
                writeTVar failure (Just failureMessage)
    FlushProjection acknowledgement -> do
      result <- maybe (Right ()) Left <$> atomically (readTVar failure)
      atomically (putTMVar acknowledgement result)

flushRuntimeSink :: Runtime -> IO (Either Text ())
flushRuntimeSink runtime = do
  existingFailure <- atomically (readTVar (internalSinkFailure runtime))
  case existingFailure of
    Just message -> pure (Left message)
    Nothing -> do
      acknowledgement <- newEmptyTMVarIO
      queued <-
        timeout sinkFlushTimeoutMicros . atomically $
          writeTBQueue (internalSinkQueue runtime) (FlushProjection acknowledgement)
      case queued of
        Nothing -> failFlush "event sink queue did not accept a flush marker"
        Just () -> do
          completed <- timeout sinkFlushTimeoutMicros (atomically (takeTMVar acknowledgement))
          case completed of
            Nothing -> failFlush "event sink did not finish within one second"
            Just result -> pure result
  where
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
    Left exception ->
      throwWorkflow $ transportFailure pluginName method (Text.pack $ show exception)
    Right result -> pure result
  let standardError = decodeUtf8With lenientDecode standardErrorBytes
  unlessEmpty standardError $ \diagnostic -> do
    recordRuntime runtime (PluginDiagnosticRecord pluginName diagnostic)
  case parsedOutput of
    Left protocolError ->
      let message = case exitCode of
            ExitSuccess -> protocolErrorMessage protocolError
            ExitFailure code ->
              exitMessage code standardError
                <> "; stdout also violated the protocol: "
                <> protocolErrorMessage protocolError
       in throwWorkflow $ PluginOutcomeUnknown pluginName method message
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
            then
              throwWorkflow
                ( PluginOutcomeUnknown
                    pluginName
                    method
                    (decodeUtf8 . LazyByteString.toStrict $ Aeson.encode failure)
                )
            else throwWorkflow (PluginReportedFailure pluginName method (Aeson.toJSON failure))
        PluginSucceeded value -> case exitCode of
          ExitFailure code ->
            throwWorkflow $ transportFailure pluginName method (exitMessage code standardError)
          ExitSuccess ->
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

defaultEventSink :: EventSink
defaultEventSink = EventSink $ \record -> case record of
  PluginEventRecord pluginName _ event -> do
    let encodedEvent = decodeUtf8 . LazyByteString.toStrict $ Aeson.encode event
    Text.IO.hPutStrLn stderr $ "[clef:" <> pluginName <> ":event] " <> encodedEvent
  PluginDiagnosticRecord pluginName diagnostic ->
    Text.IO.hPutStrLn stderr $ "[clef:" <> pluginName <> ":stderr] " <> diagnostic
  _ -> pure ()

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
  go (Right initialPluginOutputParser) ByteString.empty
  where
    go parser buffered = do
      chunk <- ByteString.hGetSome handle 32768
      if ByteString.null chunk
        then do
          finalParser <-
            if ByteString.null buffered
              then pure parser
              else consumeFrame parser buffered
          pure $ finalParser >>= finishPluginOutput pluginName
        else do
          (nextParser, remaining) <- consumeCompleteFrames parser (buffered <> chunk)
          go nextParser remaining

    consumeCompleteFrames parser buffered =
      case ByteString.elemIndex 10 buffered of
        Nothing -> pure (parser, buffered)
        Just delimiter -> do
          let frame = ByteString.take delimiter buffered
              remaining = ByteString.drop (delimiter + 1) buffered
          nextParser <- consumeFrame parser frame
          consumeCompleteFrames nextParser remaining

    consumeFrame :: Either WorkflowError PluginOutputParser -> ByteString.ByteString -> IO (Either WorkflowError PluginOutputParser)
    consumeFrame failed@(Left _) _ = pure failed
    consumeFrame (Right parser) frame =
      case parsePluginFrame pluginName requestId parser frame of
        Left workflowError -> pure (Left workflowError)
        Right (nextParser, maybeEvent) -> do
          maybe (pure ()) onEvent maybeEvent
          pure (Right nextParser)

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

transportFailure :: Text -> Text -> Text -> WorkflowError
transportFailure pluginName method message =
  PluginOutcomeUnknown pluginName method message

throwWorkflow :: WorkflowError -> IO a
throwWorkflow = throwIO
