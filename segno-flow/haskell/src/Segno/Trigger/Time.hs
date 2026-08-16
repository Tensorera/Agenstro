{-# LANGUAGE OverloadedStrings #-}

-- | Pure interval and cron planning.  No function in this module sleeps; the
-- Segno driver owns waiting and durable cursor advancement.
module Segno.Trigger.Time
  ( TimeEvent (..),
    IntervalConfig (..),
    CronConfig (..),
    intervalTrigger,
    cronTrigger,
    planTimeSource,
  )
where

import Control.Monad (unless, when)
import Data.Aeson
  ( FromJSON (parseJSON),
    ToJSON (toJSON),
    Value,
    object,
    withObject,
    (.:),
    (.:?),
    (.!=),
    (.=),
  )
import Data.Aeson.Types (Parser, parseEither)
import Data.Text (Text)
import qualified Data.Text as Text
import Data.Time
  ( UTCTime,
    addUTCTime,
    diffUTCTime,
    formatTime,
    defaultTimeLocale,
  )
import Data.Word (Word64)
import System.Cron (CronSchedule, nextMatch, parseCronSchedule)
import Clef.Segno (Trigger, TriggerId, triggerSource)
import Segno.Protocol (PlannedOccurrence (..), PollResult (..))

data TimeEvent = TimeEvent
  { timeEventLogicalTime :: UTCTime
  }
  deriving (Eq, Show)

instance FromJSON TimeEvent where
  parseJSON = withObject "time event" $ \fields -> TimeEvent <$> fields .: "logical_time"

instance ToJSON TimeEvent where
  toJSON event = object ["logical_time" .= timeEventLogicalTime event]

newtype IntervalConfig = IntervalConfig
  { intervalMilliseconds :: Word64
  }
  deriving (Eq, Show)

instance ToJSON IntervalConfig where
  toJSON config =
    object
      [ "every_ms" .= Text.pack (show (intervalMilliseconds config))
      ]

instance FromJSON IntervalConfig where
  parseJSON = withObject "interval trigger config" $ \fields -> do
    encoded <- fields .: "every_ms"
    value <- decimalWord64 encoded
    when (value == 0) (fail "every_ms must be greater than zero")
    pure (IntervalConfig value)

data CronConfig = CronConfig
  { cronExpression :: Text,
    cronTimezone :: Text
  }
  deriving (Eq, Show)

instance ToJSON CronConfig where
  toJSON config =
    object
      [ "expression" .= cronExpression config,
        "timezone" .= cronTimezone config
      ]

instance FromJSON CronConfig where
  parseJSON = withObject "cron trigger config" $ \fields ->
    CronConfig
      <$> fields .: "expression"
      <*> fields .:? "timezone" .!= "UTC"

intervalTrigger :: TriggerId -> Word64 -> Trigger state TimeEvent
intervalTrigger identity everyMilliseconds =
  triggerSource identity "time.interval" (IntervalConfig everyMilliseconds)

cronTrigger :: TriggerId -> Text -> Trigger state TimeEvent
cronTrigger identity expression =
  triggerSource identity "time.cron" (CronConfig expression "UTC")

-- | Plan due occurrences for one time plugin leaf.  @cursor@ is the cursor
-- acknowledged by the driver after durable occurrence insertion.
planTimeSource :: Text -> Text -> Value -> Maybe Value -> UTCTime -> Int -> Either Text PollResult
planTimeSource pluginName identity configuration cursor now limit
  | limit <= 0 = Left "poll limit must be positive"
  | pluginName == "time.interval" = do
      config <- decodeValue configuration
      planInterval identity config cursor now limit
  | pluginName == "time.cron" = do
      config <- decodeValue configuration
      planCron identity config cursor now limit
  | otherwise = Left ("unsupported time plugin: " <> pluginName)

planInterval :: Text -> IntervalConfig -> Maybe Value -> UTCTime -> Int -> Either Text PollResult
planInterval identity config cursor now limit = do
  let milliseconds = intervalMilliseconds config
  when (milliseconds == 0) (Left "every_ms must be greater than zero")
  lastLogical <- traverse decodeCursor cursor
  let step = fromRational (toRational milliseconds / 1000)
      first = maybe now (addUTCTime step) lastLogical
      dueCount
        | first > now = 0
        | otherwise = 1 + floor (diffUTCTime now first / step)
      selectedCount = min limit dueCount
      dueTimes = take selectedCount (iterate (addUTCTime step) first)
      occurrences = fmap (makeOccurrence identity) dueTimes
      base = case reverse dueTimes of
        latest : _ -> latest
        [] -> maybe now id lastLogical
      nextWake
        | selectedCount < dueCount = Just (addUTCTime step base)
        | otherwise = Just (if first > now then first else addUTCTime step base)
  pure (PollResult occurrences nextWake)

planCron :: Text -> CronConfig -> Maybe Value -> UTCTime -> Int -> Either Text PollResult
planCron identity config cursor now limit = do
  unless (cronTimezone config == "UTC") (Left "Segno v1 cron supports timezone UTC only")
  schedule <- firstText (parseCronSchedule (cronExpression config))
  lastLogical <- traverse decodeCursor cursor
  let searchFrom = maybe (addUTCTime (-60) now) id lastLogical
      candidates = take (limit + 1) (unfoldMatches schedule searchFrom)
      dueTimes = take limit (takeWhile (<= now) candidates)
      nextWake = case drop (length dueTimes) candidates of
        value : _ -> Just value
        [] -> nextMatch schedule (maybe searchFrom id (safeLast dueTimes))
  pure (PollResult (fmap (makeOccurrence identity) dueTimes) nextWake)

unfoldMatches :: CronSchedule -> UTCTime -> [UTCTime]
unfoldMatches schedule start = case nextMatch schedule start of
  Nothing -> []
  Just value -> value : unfoldMatches schedule value

makeOccurrence :: Text -> UTCTime -> PlannedOccurrence
makeOccurrence identity logicalTime =
  PlannedOccurrence
    { plannedLogicalTime = logicalTime,
      plannedCursor = object ["logical_time" .= logicalTime],
      plannedIdempotencyKey = identity <> ":" <> renderTime logicalTime,
      plannedPayload = toJSON (TimeEvent logicalTime)
    }

decodeCursor :: Value -> Either Text UTCTime
decodeCursor value = case parseEither parser value of
  Left message -> Left (Text.pack message)
  Right decoded -> Right decoded
  where
    parser = withObject "time cursor" (.: "logical_time")

decodeValue :: FromJSON value => Value -> Either Text value
decodeValue value = case parseEither parseJSON value of
  Left message -> Left (Text.pack message)
  Right decoded -> Right decoded

decimalWord64 :: Text -> Parser Word64
decimalWord64 encoded = case reads (Text.unpack encoded) of
  [(value, "")] -> pure value
  _ -> fail "expected an unsigned decimal string"

firstText :: Either String value -> Either Text value
firstText = either (Left . Text.pack) Right

safeLast :: [value] -> Maybe value
safeLast [] = Nothing
safeLast values = Just (last values)

renderTime :: UTCTime -> Text
renderTime = Text.pack . formatTime defaultTimeLocale "%Y%m%dT%H%M%S%QZ"
