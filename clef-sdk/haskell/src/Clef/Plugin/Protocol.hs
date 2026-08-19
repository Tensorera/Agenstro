{-# LANGUAGE OverloadedStrings #-}

module Clef.Plugin.Protocol
  ( PluginRequest (..),
    PluginFailure (..),
    PluginTerminal (..),
    ParsedPluginOutput (..),
    PluginOutputParser,
    PluginOutputStreamParser,
    initialPluginOutputParser,
    initialPluginOutputStreamParser,
    parsePluginFrame,
    feedPluginOutputChunk,
    finishPluginOutput,
    finishPluginOutputStream,
    decodeStrictJSON,
    encodePluginRequest,
    parsePluginOutput,
  )
where

import Data.Aeson
  ( FromJSON (parseJSON),
    Object,
    ToJSON (toJSON),
    Value (Object),
    eitherDecodeStrict',
    encode,
    object,
    (.:),
    (.:?),
    (.=),
  )
import Data.Aeson.Decoding.ByteString (bsToTokens)
import Data.Aeson.Decoding.Tokens
  ( Number (..),
    TkArray (..),
    TkRecord (..),
    Tokens (..),
  )
import Data.Aeson.Key (Key)
import qualified Data.Aeson.Key as Key
import qualified Data.Aeson.KeyMap as KeyMap
import Data.Aeson.Types (parseEither)
import Data.ByteString (ByteString)
import qualified Data.ByteString as ByteString
import qualified Data.ByteString.Lazy as LazyByteString
import qualified Data.Set as Set
import Data.Scientific (Scientific, toRealFloat)
import Data.Text (Text)
import qualified Data.Text as Text
import Data.Text.Encoding (decodeUtf8', encodeUtf8)
import Clef.Error (WorkflowError (..))

data PluginRequest = PluginRequest
  { pluginRequestId :: Text,
    pluginRequestMethod :: Text,
    pluginRequestParams :: Object
  }
  deriving (Eq, Show)

-- | Stable failure envelope shared by Clef and the Rust Tactus runtime.
-- Plugin-specific evidence remains open in 'pluginFailureDetails'.
data PluginFailure = PluginFailure
  { pluginFailureCode :: Text,
    pluginFailureMessage :: Text,
    pluginFailureDetails :: Maybe Value
  }
  deriving (Eq, Show)

instance FromJSON PluginFailure where
  parseJSON value = do
    objectValue <- parseJSON value
    PluginFailure
      <$> objectValue .: "code"
      <*> objectValue .: "message"
      <*> objectValue .:? "details"

instance ToJSON PluginFailure where
  toJSON failure =
    object $
      [ "code" .= pluginFailureCode failure,
        "message" .= pluginFailureMessage failure
      ]
        <> maybe [] (\details -> ["details" .= details]) (pluginFailureDetails failure)

data PluginTerminal
  = PluginSucceeded Value
  | PluginFailed PluginFailure
  deriving (Eq, Show)

data ParsedPluginOutput = ParsedPluginOutput
  { pluginOutputEvents :: [Value],
    pluginOutputTerminal :: PluginTerminal
  }
  deriving (Eq, Show)

-- | Incremental state for one plugin stdout stream.  It is deliberately
-- abstract: callers may feed complete LF-delimited frames as bytes, while
-- preserving UTF-8 sequences split across arbitrary process read chunks.
data PluginOutputParser = PluginOutputParser
  { parserLineNumber :: Int,
    parserEventsReversed :: [Value],
    parserTerminal :: Maybe PluginTerminal
  }
  deriving (Eq, Show)

initialPluginOutputParser :: PluginOutputParser
initialPluginOutputParser = PluginOutputParser 1 [] Nothing

-- | Byte-stream state above the complete-frame parser.  Keeping the partial
-- line as bytes makes arbitrary UTF-8 and JSON token chunking harmless.
data PluginOutputStreamParser = PluginOutputStreamParser
  { streamFrameParser :: PluginOutputParser,
    streamBufferedBytes :: ByteString
  }
  deriving (Eq, Show)

initialPluginOutputStreamParser :: PluginOutputStreamParser
initialPluginOutputStreamParser =
  PluginOutputStreamParser initialPluginOutputParser ByteString.empty

instance ToJSON PluginRequest where
  toJSON request =
    object
      [ "api" .= ("agenstro.plugin/v1" :: Text),
        "id" .= pluginRequestId request,
        "method" .= pluginRequestMethod request,
        "params" .= pluginRequestParams request
      ]

encodePluginRequest :: PluginRequest -> LazyByteString.ByteString
encodePluginRequest = encode

parsePluginOutput :: Text -> Text -> Text -> Either WorkflowError ParsedPluginOutput
parsePluginOutput pluginName expectedId output = do
  (parser, _) <-
    feedPluginOutputChunk pluginName expectedId initialPluginOutputStreamParser (encodeUtf8 output)
  snd <$> finishPluginOutputStream pluginName expectedId parser

-- | Feed an arbitrary process-read chunk and return complete event frames in
-- arrival order.  Partial lines remain buffered until a later chunk or EOF.
feedPluginOutputChunk :: Text -> Text -> PluginOutputStreamParser -> ByteString -> Either WorkflowError (PluginOutputStreamParser, [Value])
feedPluginOutputChunk pluginName expectedId parser chunk =
  consumeCompleteFrames
    (streamFrameParser parser)
    (streamBufferedBytes parser <> chunk)
    []
  where
    consumeCompleteFrames frameParser buffered events =
      case ByteString.elemIndex 10 buffered of
        Nothing ->
          Right
            ( PluginOutputStreamParser frameParser buffered,
              reverse events
            )
        Just delimiter -> do
          let frame = ByteString.take delimiter buffered
              remaining = ByteString.drop (delimiter + 1) buffered
          (nextParser, maybeEvent) <- parsePluginFrame pluginName expectedId frameParser frame
          consumeCompleteFrames nextParser remaining (maybe events (: events) maybeEvent)

-- | Finish a byte stream at EOF, accepting a final frame without LF and then
-- requiring exactly one terminal result.
finishPluginOutputStream :: Text -> Text -> PluginOutputStreamParser -> Either WorkflowError ([Value], ParsedPluginOutput)
finishPluginOutputStream pluginName expectedId parser = do
  (frameParser, finalEvents) <-
    if ByteString.null (streamBufferedBytes parser)
      then Right (streamFrameParser parser, [])
      else do
        (nextParser, maybeEvent) <-
          parsePluginFrame
            pluginName
            expectedId
            (streamFrameParser parser)
            (streamBufferedBytes parser)
        Right (nextParser, maybe [] pure maybeEvent)
  output <- finishPluginOutput pluginName frameParser
  Right (finalEvents, output)

-- | Validate one complete JSONL frame.  Event frames are returned immediately
-- so the runtime can publish them before the terminal result arrives.
parsePluginFrame :: Text -> Text -> PluginOutputParser -> ByteString -> Either WorkflowError (PluginOutputParser, Maybe Value)
parsePluginFrame pluginName expectedId parser encodedLine
  | ByteString.null encodedLine = protocolFailure $ "empty JSONL frame at line " <> renderLine lineNumber
  | Just _ <- parserTerminal parser =
      protocolFailure $ "received data after the terminal result at line " <> renderLine lineNumber
  | otherwise = do
      case decodeUtf8' encodedLine of
        Left exception ->
          protocolFailure $
            "stdout was not valid UTF-8 at line " <> renderLine lineNumber <> ": " <> Text.pack (show exception)
        Right _ -> pure ()
      frame <- case decodeUniqueJSON encodedLine of
        Left message ->
          protocolFailure $
            "invalid JSON at line " <> renderLine lineNumber <> ": " <> Text.pack message
        Right value -> Right value
      (frameType, frameId) <- case frame of
        Object objectValue ->
          case parseEither (\value -> (,) <$> value .: "type" <*> value .: "id") objectValue of
            Left message ->
              protocolFailure $
                "invalid frame at line " <> renderLine lineNumber <> ": " <> Text.pack message
            Right fields -> Right fields
        _ -> protocolFailure $ "frame at line " <> renderLine lineNumber <> " must be an object"
      if frameId /= expectedId
        then
          protocolFailure $
            "correlation id mismatch at line "
              <> renderLine lineNumber
              <> ": expected '"
              <> expectedId
              <> "' but received '"
              <> frameId
              <> "'"
        else case (frameType :: Text) of
          "event" -> do
            case frame of
              Object objectValue -> case KeyMap.lookup "event" objectValue of
                Just (Object eventObject) ->
                  case (parseEither (.: "type") eventObject :: Either String Text) of
                    Left message ->
                      protocolFailure $
                        "event frame at line "
                          <> renderLine lineNumber
                          <> " has an invalid event subtype: "
                          <> Text.pack message
                    Right _ -> pure ()
                _ ->
                  protocolFailure $
                    "event frame at line "
                      <> renderLine lineNumber
                      <> " must contain an event object"
              _ -> protocolFailure "event frame must be an object"
            pure
              ( parser
                  { parserLineNumber = lineNumber + 1,
                    parserEventsReversed = frame : parserEventsReversed parser
                  },
                Just frame
              )
          "result" -> do
            terminal <- parseTerminal pluginName lineNumber frame
            pure (parser {parserLineNumber = lineNumber + 1, parserTerminal = Just terminal}, Nothing)
          other ->
            protocolFailure $
              "unknown frame type '" <> other <> "' at line " <> renderLine lineNumber
  where
    lineNumber = parserLineNumber parser
    protocolFailure = Left . PluginProtocolFailed pluginName

-- | Complete an incremental stream.  Exactly one terminal result is required.
finishPluginOutput :: Text -> PluginOutputParser -> Either WorkflowError ParsedPluginOutput
finishPluginOutput pluginName parser = case parserTerminal parser of
  Nothing -> Left (PluginProtocolFailed pluginName "missing terminal result")
  Just terminal -> Right (ParsedPluginOutput (reverse (parserEventsReversed parser)) terminal)

parseTerminal :: Text -> Int -> Value -> Either WorkflowError PluginTerminal
parseTerminal pluginName lineNumber (Object objectValue) = do
  ok <- case parseEither (.: "ok") objectValue of
    Left message ->
      protocolFailure $
        "invalid terminal result at line " <> renderLine lineNumber <> ": " <> Text.pack message
    Right result -> Right result
  let hasValue = KeyMap.member "value" objectValue
      hasError = KeyMap.member "error" objectValue
  case (ok, hasValue, hasError) of
    (True, True, False) ->
      maybe
        (protocolFailure "terminal result lost its value field")
        (Right . PluginSucceeded)
        (KeyMap.lookup "value" objectValue)
    (False, False, True) ->
      maybe
        (protocolFailure "terminal result lost its error field")
        (\failure ->
          case parseEither parseJSON failure of
            Left message ->
              protocolFailure $
                "failed terminal result at line "
                  <> renderLine lineNumber
                  <> " has an invalid error object: "
                  <> Text.pack message
            Right structured -> Right (PluginFailed structured)
        )
        (KeyMap.lookup "error" objectValue)
    (True, _, _) ->
      protocolFailure $
        "successful terminal result at line " <> renderLine lineNumber <> " must contain value and no error"
    (False, _, _) ->
      protocolFailure $
        "failed terminal result at line " <> renderLine lineNumber <> " must contain error and no value"
  where
    protocolFailure = Left . PluginProtocolFailed pluginName
parseTerminal pluginName lineNumber _ =
  Left . PluginProtocolFailed pluginName $
    "terminal result at line " <> renderLine lineNumber <> " must be an object"

renderLine :: Int -> Text
renderLine = Text.pack . show

decodeUniqueJSON :: ByteString -> Either String Value
decodeUniqueJSON = decodeStrictJSON

-- | Decode JSON after enforcing duplicate-key and numeric-domain rules.
decodeStrictJSON :: FromJSON value => ByteString -> Either String value
decodeStrictJSON encoded = do
  _ <- validateTokens (bsToTokens encoded)
  eitherDecodeStrict' encoded

validateTokens :: Tokens continuation String -> Either String continuation
validateTokens (TkLit _ continuation) = Right continuation
validateTokens (TkText _ continuation) = Right continuation
validateTokens (TkNumber number continuation)
  | finiteJSONNumber number = Right continuation
  | otherwise = Left "JSON number is outside the finite float range"
validateTokens (TkArrayOpen array) = validateArray array
validateTokens (TkRecordOpen record) = validateRecord Set.empty record
validateTokens (TkErr message) = Left message

validateArray :: TkArray continuation String -> Either String continuation
validateArray (TkItem tokens) = validateTokens tokens >>= validateArray
validateArray (TkArrayEnd continuation) = Right continuation
validateArray (TkArrayErr message) = Left message

validateRecord :: Set.Set Key -> TkRecord continuation String -> Either String continuation
validateRecord seen (TkPair key tokens)
  | Set.member key seen = Left $ "duplicate object key '" <> Text.unpack (Key.toText key) <> "'"
  | otherwise = validateTokens tokens >>= validateRecord (Set.insert key seen)
validateRecord _ (TkRecordEnd continuation) = Right continuation
validateRecord _ (TkRecordErr message) = Left message

finiteJSONNumber :: Number -> Bool
-- Match serde_json's lossless integer domain used by the Rust runtime.  A
-- larger integer must not silently cross the boundary through an f64.
finiteJSONNumber (NumInteger value) =
  value >= -9223372036854775808 && value <= 18446744073709551615
finiteJSONNumber (NumDecimal value) = finiteScientific value
finiteJSONNumber (NumScientific value) = finiteScientific value

finiteScientific :: Scientific -> Bool
finiteScientific value =
  let converted = toRealFloat value :: Double
   in not (isInfinite converted || isNaN converted)
        && (converted /= 0 || value == 0)
