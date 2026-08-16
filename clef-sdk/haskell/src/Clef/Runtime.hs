{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE ScopedTypeVariables #-}

module Clef.Runtime
  ( Runtime,
    RuntimeRecord (..),
    PluginCallResult (..),
    newRuntime,
    runtimeConfig,
    readRuntimeRecords,
    recordRuntime,
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
import Control.Concurrent.Async (concurrently)
import Control.Exception (IOException, finally, throwIO, try)
import Data.Aeson (Value)
import qualified Data.Aeson as Aeson
import qualified Data.Aeson.KeyMap as KeyMap
import qualified Data.ByteString as ByteString
import qualified Data.ByteString.Lazy as LazyByteString
import Data.Text (Text)
import qualified Data.Text as Text
import qualified Data.Text.IO as Text.IO
import Data.Text.Encoding (decodeUtf8, decodeUtf8', decodeUtf8With)
import Data.Text.Encoding.Error (lenientDecode)
import System.Exit (ExitCode (..))
import System.IO (hClose, hSetBinaryMode, stderr)
import System.Process
  ( CreateProcess (cwd, std_err, std_in, std_out),
    StdStream (CreatePipe),
    proc,
    waitForProcess,
    withCreateProcess,
  )
import Clef.Error (WorkflowError (..))
import Clef.Plugin.Protocol
  ( ParsedPluginOutput (..),
    PluginRequest (..),
    PluginTerminal (..),
    decodeStrictJSON,
    encodePluginRequest,
    parsePluginOutput,
  )
import Clef.Runtime.Config (RuntimeConfig (runtimeWorkspace))

data Runtime = Runtime
  { internalRuntimeConfig :: RuntimeConfig,
    internalNextRequestId :: MVar Integer,
    internalRecords :: MVar [RuntimeRecord]
  }

-- | Provider values and effect evidence deliberately use different
-- constructors.  Neither is promoted into an artifact model by the core.
data RuntimeRecord
  = PluginEventRecord Text Text Value
  | ProviderValueRecord Text Text Value
  | EffectEvidenceRecord Text Text Text Value
  | PluginDiagnosticRecord Text Text
  deriving (Eq, Show)

data PluginCallResult = PluginCallResult
  { pluginCallId :: Text,
    pluginCallValue :: Value,
    pluginCallEvents :: [Value]
  }
  deriving (Eq, Show)

newRuntime :: RuntimeConfig -> IO Runtime
newRuntime config =
  Runtime config <$> newMVar 1 <*> newMVar []

runtimeConfig :: Runtime -> RuntimeConfig
runtimeConfig = internalRuntimeConfig

readRuntimeRecords :: Runtime -> IO [RuntimeRecord]
readRuntimeRecords runtime = reverse <$> readMVar (internalRecords runtime)

recordRuntime :: Runtime -> RuntimeRecord -> IO ()
recordRuntime runtime record =
  modifyMVar_ (internalRecords runtime) (pure . (record :))

callPlugin :: Runtime -> Text -> [Text] -> Text -> Value -> IO PluginCallResult
callPlugin runtime pluginName command method params = do
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
      ) :: IO (Either IOException (ExitCode, ByteString.ByteString, ByteString.ByteString))
  (exitCode, standardOutputBytes, standardErrorBytes) <- case processResult of
    Left exception ->
      throwWorkflow $ transportFailure pluginName method (Text.pack $ show exception)
    Right result -> pure result
  let standardError = decodeUtf8With lenientDecode standardErrorBytes
      parsedOutput = case decodeUtf8' standardOutputBytes of
        Left exception ->
          Left . PluginProtocolFailed pluginName $
            "stdout was not valid UTF-8: " <> Text.pack (show exception)
        Right standardOutput -> parsePluginOutput pluginName requestId standardOutput
  unlessEmpty standardError $ \diagnostic -> do
    recordRuntime runtime (PluginDiagnosticRecord pluginName diagnostic)
    Text.IO.hPutStrLn stderr $ "[clef:" <> pluginName <> ":stderr] " <> diagnostic
  case parsedOutput of
    Left protocolError ->
      let message = case exitCode of
            ExitSuccess -> protocolErrorMessage protocolError
            ExitFailure code ->
              exitMessage code standardError
                <> "; stdout also violated the protocol: "
                <> protocolErrorMessage protocolError
       in if method == "invoke"
            then throwWorkflow $ PluginOutcomeUnknown pluginName method message
            else case exitCode of
              ExitSuccess -> throwWorkflow protocolError
              ExitFailure _ -> throwWorkflow $ PluginProcessFailed pluginName message
    Right parsed -> do
      mapM_ (emitEvent runtime pluginName requestId) (pluginOutputEvents parsed)
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
          if method == "invoke" && isOutcomeUnknown failure
            then
              throwWorkflow
                ( PluginOutcomeUnknown
                    pluginName
                    method
                    (decodeUtf8 . LazyByteString.toStrict $ Aeson.encode failure)
                )
            else throwWorkflow (PluginReportedFailure pluginName method failure)
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
emitEvent runtime pluginName requestId event = do
  recordRuntime runtime (PluginEventRecord pluginName requestId event)
  let encodedEvent = decodeUtf8 . LazyByteString.toStrict $ Aeson.encode event
  Text.IO.hPutStrLn stderr $ "[clef:" <> pluginName <> ":event] " <> encodedEvent

readPluginProcess :: CreateProcess -> ByteString.ByteString -> IO (ExitCode, ByteString.ByteString, ByteString.ByteString)
readPluginProcess process requestInput =
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
                  (ByteString.hGetContents outputHandle)
                  (ByteString.hGetContents errorHandle)
              )
          exitCode <- waitForProcess processHandle
          pure (exitCode, standardOutput, standardError)
        _ -> ioError (userError "plugin process pipes were not created")

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
transportFailure pluginName method message
  | method == "invoke" = PluginOutcomeUnknown pluginName method message
  | otherwise = PluginProcessFailed pluginName message

isOutcomeUnknown :: Value -> Bool
isOutcomeUnknown (Aeson.Object failure) =
  KeyMap.lookup "code" failure == Just (Aeson.String "outcome_unknown")
isOutcomeUnknown _ = False

throwWorkflow :: WorkflowError -> IO a
throwWorkflow = throwIO
