{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE ScopedTypeVariables #-}

-- | JSONL plugin hosts shipped with Segno.  The driver reaches these through
-- @tactus dispatch@ exactly like any third-party implementation.
module Segno.Plugin
  ( runTimePluginHost,
    runStatePluginHost,
    runActiveWindowPluginHost,
  )
where

import Control.Exception (SomeException, displayException, try)
import Control.Monad (unless, when)
import Data.Aeson
  ( FromJSON,
    Object,
    ToJSON (toJSON),
    Value (..),
    encode,
    object,
    (.:),
    (.:?),
    (.!=),
    (.=),
  )
import qualified Data.Aeson.Key as Key
import Data.Aeson.Types (Parser, parseEither)
import qualified Data.ByteString as ByteString
import qualified Data.ByteString.Lazy as LazyByteString
import Data.Text (Text)
import qualified Data.Text as Text
import Data.Text.Encoding (decodeUtf8')
import Data.Time (UTCTime, getCurrentTime)
import Data.Word (Word32)
import Segno.ActiveWindow (captureActiveWindow)
import Segno.Protocol
  ( PluginFailure (..),
    PluginRequest (..),
    PollResult,
    StateManifest (..),
    decodeJsonText,
    pluginFailure,
    pluginSuccess,
  )
import Segno.Store.SQLite
  ( CasResult (..),
    appendBusinessEvent,
    businessHistory,
    compareAndSetBusinessState,
    initialiseStore,
    loadBusinessState,
    segnoPaths,
  )
import Segno.Trigger.Time (planTimeSource)
import System.Exit (exitFailure)
import System.IO (hFlush, hPutStrLn, stderr, stdin, stdout)

maximumRequestBytes :: Int
maximumRequestBytes = 4 * 1024 * 1024

type PluginHandler = PluginRequest -> IO (Either PluginFailure Value)

runTimePluginHost :: Text -> IO ()
runTimePluginHost pluginName = runPluginHost (timeHandler pluginName)

runStatePluginHost :: IO ()
runStatePluginHost = runPluginHost stateHandler

runActiveWindowPluginHost :: IO ()
runActiveWindowPluginHost = runPluginHost activeWindowHandler

runPluginHost :: PluginHandler -> IO ()
runPluginHost handler = do
  encodedResult <- readBounded maximumRequestBytes
  encoded <- case encodedResult of
    Left () -> do
      hPutStrLn stderr "Segno plugin request exceeded 4 MiB"
      exitFailure
    Right value -> pure value
  let lines' = filter (not . ByteString.null) (fmap stripCarriageReturn (ByteString.split 10 encoded))
  line <- case lines' of
    [single] -> pure single
    _ -> do
      hPutStrLn stderr "Segno plugin requires exactly one non-empty JSONL request frame"
      exitFailure
  request <- case decodeUtf8' line of
    Left _ -> do
      hPutStrLn stderr "invalid plugin request: stdin was not UTF-8"
      exitFailure
    Right text -> case decodeJsonText text of
      Left message -> do
        hPutStrLn stderr ("invalid plugin request: " <> message)
        exitFailure
      Right value -> pure value
  outcome <- try (handler request)
  let response = case outcome of
        Left (exception :: SomeException) ->
          pluginFailure
            (pluginRequestId request)
            (PluginFailure "plugin_internal" (Text.pack (displayException exception)) Nothing)
        Right (Left failure) -> pluginFailure (pluginRequestId request) failure
        Right (Right value) -> pluginSuccess (pluginRequestId request) value
  LazyByteString.hPut stdout (encode response <> "\n")
  hFlush stdout

readBounded :: Int -> IO (Either () ByteString.ByteString)
readBounded limit = go ByteString.empty
  where
    go accumulated = do
      chunk <- ByteString.hGetSome stdin 65536
      if ByteString.null chunk
        then pure (Right accumulated)
        else do
          let combinedLength = ByteString.length accumulated + ByteString.length chunk
          if combinedLength > limit
            then pure (Left ())
            else go (accumulated <> chunk)

stripCarriageReturn :: ByteString.ByteString -> ByteString.ByteString
stripCarriageReturn value
  | ByteString.null value = value
  | ByteString.last value == 13 = ByteString.init value
  | otherwise = value

timeHandler :: Text -> PluginHandler
timeHandler pluginName request = case pluginRequestMethod request of
  "describe" ->
    pure . Right $
      object
        [ "api" .= ("agenstro.plugin/v1" :: Text),
          "plugin" .= pluginName,
          "capabilities" .= (["describe", "plan", "poll", "acknowledge", "smoke"] :: [Text]),
          "waits" .= False
        ]
  "smoke" -> pure (Right (object ["ok" .= True, "plugin" .= pluginName, "waits" .= False]))
  "acknowledge" -> pure (Right (object ["acknowledged" .= True]))
  "plan" -> plan
  "poll" -> plan
  method -> pure (Left (unknownMethod method))
  where
    plan = case parseEither parsePlan (pluginRequestParams request) of
      Left message -> pure (Left (invalidRequest message))
      Right (identity, configuration, cursor, now, limit) ->
        pure $ case planTimeSource pluginName identity configuration cursor now limit of
          Left message -> Left (PluginFailure "trigger_plan_failed" message Nothing)
          Right result -> Right (toJSON (result :: PollResult))

parsePlan :: Object -> Parser (Text, Value, Maybe Value, UTCTime, Int)
parsePlan fields =
  (,,,,)
    <$> required "source_id" fields
    <*> required "config" fields
    <*> optional "cursor" fields
    <*> required "now" fields
    <*> boundedLimit fields

stateHandler :: PluginHandler
stateHandler request = case pluginRequestMethod request of
  "describe" ->
    pure . Right $
      object
        [ "api" .= ("agenstro.plugin/v1" :: Text),
          "plugin" .= ("segno.state" :: Text),
          "capabilities" .= (["describe", "load", "compare-and-set", "append", "history", "smoke"] :: [Text])
        ]
  "smoke" -> withWorkspace $ \_ -> pure (Right (object ["ok" .= True, "backend" .= ("sqlite" :: Text)]))
  "load" -> withWorkspace $ \paths -> case parseEither parseLoad (pluginRequestParams request) of
    Left message -> pure (Left (invalidRequest message))
    Right manifest -> do
      now <- getCurrentTime
      Right . toJSON <$> loadBusinessState paths manifest now
  "compare-and-set" -> withWorkspace $ \paths -> case parseEither parseCas (pluginRequestParams request) of
    Left message -> pure (Left (invalidRequest message))
    Right (key, expected, schemaVersion, value, operationId, occurrenceId, fence, fenceEpoch) -> do
      now <- getCurrentTime
      result <- compareAndSetBusinessState paths key expected schemaVersion value operationId occurrenceId fence fenceEpoch now
      pure . Right $ case result of
        CasApplied revision -> object ["applied" .= True, "revision" .= revision]
        CasConflict current -> object ["applied" .= False, "current_revision" .= current]
  "append" -> withWorkspace $ \paths -> case parseEither parseAppend (pluginRequestParams request) of
    Left message -> pure (Left (invalidRequest message))
    Right (key, kind, payload, occurrence) -> do
      now <- getCurrentTime
      appendBusinessEvent paths key kind payload occurrence now
      pure (Right (object ["appended" .= True]))
  "history" -> withWorkspace $ \paths -> case parseEither parseHistory (pluginRequestParams request) of
    Left message -> pure (Left (invalidRequest message))
    Right (key, limit) -> do
      entries <- businessHistory paths key limit
      pure (Right (object ["entries" .= entries]))
  method -> pure (Left (unknownMethod method))
  where
    withWorkspace action = case parseEither (required "workspace") (pluginRequestParams request) of
      Left message -> pure (Left (invalidRequest message))
      Right root -> do
        let paths = segnoPaths root
        initialiseStore paths
        action paths

parseLoad :: Object -> Parser StateManifest
parseLoad fields =
  StateManifest
    <$> required "state_key" fields
    <*> required "schema_version" fields
    <*> pure "segno.state"
    <*> pure "compare-and-set"
    <*> required "initial" fields

parseCas :: Object -> Parser (Text, Maybe Text, Word32, Value, Text, Text, Text, Word32)
parseCas fields = do
  conflict <- required "conflict" fields
  unless (conflict == ("compare-and-set" :: Text)) (fail "unsupported conflict policy")
  (,,,,,,,)
    <$> required "state_key" fields
    <*> optional "expected_revision" fields
    <*> required "schema_version" fields
    <*> required "value" fields
    <*> required "operation_id" fields
    <*> required "occurrence_id" fields
    <*> required "fencing_token" fields
    <*> required "fencing_epoch" fields

parseAppend :: Object -> Parser (Text, Text, Value, Maybe Text)
parseAppend fields =
  (,,,)
    <$> required "state_key" fields
    <*> required "event_kind" fields
    <*> required "value" fields
    <*> optional "occurrence_id" fields

parseHistory :: Object -> Parser (Maybe Text, Int)
parseHistory fields =
  (,)
    <$> optional "state_key" fields
    <*> boundedLimit fields

activeWindowHandler :: PluginHandler
activeWindowHandler request = case pluginRequestMethod request of
  "describe" ->
    pure . Right $
      object
        [ "api" .= ("agenstro.plugin/v1" :: Text),
          "plugin" .= ("system.active-window" :: Text),
          "capabilities" .= (["describe", "current", "smoke"] :: [Text])
        ]
  "smoke" ->
    pure . Right $
      object
        [ "ok" .= True,
          "live_probe" .= False,
          "note" .= ("current performs the opt-in window-title read" :: Text)
        ]
  "current" -> fmap (fmap toJSON) captureActiveWindow
  method -> pure (Left (unknownMethod method))

required :: FromJSON value => Text -> Object -> Parser value
required name fields = fields .: Key.fromText name

optional :: FromJSON value => Text -> Object -> Parser (Maybe value)
optional name fields = fields .:? Key.fromText name

boundedLimit :: Object -> Parser Int
boundedLimit fields = do
  value <- optional "limit" fields .!= 100
  when (value < 1 || value > 1000) (fail "limit must be between 1 and 1000")
  pure value

unknownMethod :: Text -> PluginFailure
unknownMethod method =
  PluginFailure
    "unknown_method"
    ("unsupported plugin method: " <> method)
    Nothing

invalidRequest :: String -> PluginFailure
invalidRequest message = PluginFailure "invalid_request" (Text.pack message) Nothing
