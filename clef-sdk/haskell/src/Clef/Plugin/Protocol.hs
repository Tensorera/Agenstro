{-# LANGUAGE OverloadedStrings #-}

module Clef.Plugin.Protocol
  ( PluginRequest (..),
    PluginTerminal (..),
    ParsedPluginOutput (..),
    decodeStrictJSON,
    encodePluginRequest,
    parsePluginOutput,
  )
where

import Data.Aeson
  ( FromJSON,
    ToJSON (toJSON),
    Value (Object),
    eitherDecodeStrict',
    encode,
    object,
    (.:),
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
import qualified Data.ByteString.Lazy as LazyByteString
import qualified Data.Set as Set
import Data.Scientific (Scientific, toRealFloat)
import Data.Text (Text)
import qualified Data.Text as Text
import Data.Text.Encoding (encodeUtf8)
import Clef.Error (WorkflowError (..))

data PluginRequest = PluginRequest
  { pluginRequestId :: Text,
    pluginRequestMethod :: Text,
    pluginRequestParams :: Value
  }
  deriving (Eq, Show)

data PluginTerminal
  = PluginSucceeded Value
  | PluginFailed Value
  deriving (Eq, Show)

data ParsedPluginOutput = ParsedPluginOutput
  { pluginOutputEvents :: [Value],
    pluginOutputTerminal :: PluginTerminal
  }
  deriving (Eq, Show)

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
parsePluginOutput pluginName expectedId output =
  go 1 [] Nothing (Text.lines output)
  where
    protocolFailure :: Text -> Either WorkflowError a
    protocolFailure = Left . PluginProtocolFailed pluginName

    go :: Int -> [Value] -> Maybe PluginTerminal -> [Text] -> Either WorkflowError ParsedPluginOutput
    go _ _ Nothing [] = protocolFailure "missing terminal result"
    go _ events (Just terminal) [] = Right (ParsedPluginOutput (reverse events) terminal)
    go lineNumber _ (Just _) (_ : _) =
      protocolFailure $ "received data after the terminal result at line " <> renderLine lineNumber
    go lineNumber events Nothing (line : remainingLines)
      | Text.null line = protocolFailure $ "empty JSONL frame at line " <> renderLine lineNumber
      | otherwise = do
          frame <- case decodeUniqueJSON (encodeUtf8 line) of
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
                    Just (Object _) -> pure ()
                    _ ->
                      protocolFailure $
                        "event frame at line "
                          <> renderLine lineNumber
                          <> " must contain an event object"
                  _ -> protocolFailure "event frame must be an object"
                go (lineNumber + 1) (frame : events) Nothing remainingLines
              "result" -> do
                terminal <- parseTerminal lineNumber frame
                go (lineNumber + 1) events (Just terminal) remainingLines
              other ->
                protocolFailure $
                  "unknown frame type '" <> other <> "' at line " <> renderLine lineNumber

    parseTerminal :: Int -> Value -> Either WorkflowError PluginTerminal
    parseTerminal lineNumber (Object objectValue) = do
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
            (Right . PluginFailed)
            (KeyMap.lookup "error" objectValue)
        (True, _, _) ->
          protocolFailure $
            "successful terminal result at line " <> renderLine lineNumber <> " must contain value and no error"
        (False, _, _) ->
          protocolFailure $
            "failed terminal result at line " <> renderLine lineNumber <> " must contain error and no value"
    parseTerminal lineNumber _ =
      protocolFailure $ "terminal result at line " <> renderLine lineNumber <> " must be an object"

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
finiteJSONNumber (NumInteger _) = True
finiteJSONNumber (NumDecimal value) = finiteScientific value
finiteJSONNumber (NumScientific value) = finiteScientific value

finiteScientific :: Scientific -> Bool
finiteScientific value =
  let converted = toRealFloat value :: Double
   in not (isInfinite converted || isNaN converted)
        && (converted /= 0 || value == 0)
