{-# LANGUAGE DeriveGeneric #-}
{-# LANGUAGE OverloadedStrings #-}

-- A model-free persistent task: capture the foreground window once a minute,
-- checkpoint the typed business state, and let Segno retain every revision in
-- SQLite history.  The driver, not this script or the time plugin, performs
-- waiting and durable cursor advancement.
module Main (main) where

import Clef
import Data.Aeson (FromJSON, ToJSON)
import Data.Text (Text)
import Data.Time (UTCTime)
import Data.Word (Word64)
import GHC.Generics (Generic)
import Segno.ActiveWindow (ActiveWindow, currentActiveWindow)
import Segno.Trigger.Time
  ( TimeEvent (..),
    intervalTrigger,
  )

data WindowLog = WindowLog
  { capturedWindows :: Word64,
    latestWindow :: Maybe ActiveWindow,
    lastLogicalTime :: Maybe UTCTime
  }
  deriving (Eq, Generic, Show)

instance FromJSON WindowLog

instance ToJSON WindowLog

initialWindowLog :: WindowLog
initialWindowLog =
  WindowLog
    { capturedWindows = 0,
      latestWindow = Nothing,
      lastLogicalTime = Nothing
    }

activeWindowTask :: PersistentTask WindowLog TimeEvent ActiveWindow
activeWindowTask =
  persistentTask
    "record-active-window"
    everyMinute
    windowState
    recordWindow
  where
    everyMinute =
      gate
        (\stored event -> lastLogicalTime stored /= Just (timeEventLogicalTime event))
        (intervalTrigger (TriggerId "each-minute") 60000)
    windowState =
      state
        (StateKey "example.active-window")
        (SchemaVersion 1)
        initialWindowLog

recordWindow :: Occurrence TimeEvent -> StateHandle WindowLog -> Workflow (Decision WindowLog ActiveWindow)
recordWindow occurrence handle = do
  activeWindow <- currentActiveWindow
  let logicalTime = timeEventLogicalTime (occurrencePayload occurrence)
      previous = currentState handle
      next =
        previous
          { capturedWindows = capturedWindows previous + 1,
            latestWindow = Just activeWindow,
            lastLogicalTime = Just logicalTime
          }
  checkpointResult <-
    checkpoint
      (CheckpointId "capture-active-window")
      handle
      next
  pure $ case checkpointResult of
    Right durable -> Complete (KeepState durable) activeWindow
    Left conflict ->
      Retry
        RetrySpec
          { retryAfterMilliseconds = 5000,
            retryReason = conflictMessage conflict
          }
        (KeepState handle)

conflictMessage :: StateConflict -> Text
conflictMessage conflict =
  "business state changed while recording the active window: "
    <> maybe "<none>" unStateRevision (conflictExpectedRevision conflict)
    <> " -> "
    <> maybe "<none>" unStateRevision (conflictActualRevision conflict)

main :: IO ()
main = runPersistentTask activeWindowTask
