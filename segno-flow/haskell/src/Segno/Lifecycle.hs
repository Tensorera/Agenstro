{-# LANGUAGE OverloadedStrings #-}

-- | Runtime-owned lifecycle.  This is deliberately separate from the typed
-- business state supplied to a Clef workflow.
module Segno.Lifecycle
  ( Lifecycle (..),
    lifecycleText,
    parseLifecycle,
    OccurrenceRecord (..),
    ClaimedOccurrence (..),
  )
where

import Data.Aeson (Value)
import Data.Text (Text)
import Data.Time (UTCTime)
import Data.Word (Word32)
import Segno.Protocol (TriggerOccurrence)

data Lifecycle
  = Dormant
  | Ready
  | Claimed
  | Running
  | Waiting
  | Succeeded
  | Failed
  | OutcomeUnknown
  deriving (Bounded, Enum, Eq, Ord, Show)

lifecycleText :: Lifecycle -> Text
lifecycleText lifecycle = case lifecycle of
  Dormant -> "dormant"
  Ready -> "ready"
  Claimed -> "claimed"
  Running -> "running"
  Waiting -> "waiting"
  Succeeded -> "succeeded"
  Failed -> "failed"
  OutcomeUnknown -> "outcome_unknown"

parseLifecycle :: Text -> Either Text Lifecycle
parseLifecycle value = case value of
  "dormant" -> Right Dormant
  "ready" -> Right Ready
  "claimed" -> Right Claimed
  "running" -> Right Running
  "waiting" -> Right Waiting
  "succeeded" -> Right Succeeded
  "failed" -> Right Failed
  "outcome_unknown" -> Right OutcomeUnknown
  _ -> Left ("unknown lifecycle state: " <> value)

data OccurrenceRecord = OccurrenceRecord
  { recordJobId :: Text,
    recordOccurrence :: TriggerOccurrence,
    recordLifecycle :: Lifecycle,
    recordAttempt :: Word32,
    recordFencingToken :: Maybe Text,
    recordLeaseUntil :: Maybe UTCTime,
    recordNextAttemptAt :: Maybe UTCTime,
    recordUpdatedAt :: UTCTime,
    recordError :: Maybe Text,
    recordOutput :: Maybe Value
  }
  deriving (Eq, Show)

data ClaimedOccurrence = ClaimedOccurrence
  { claimedJobId :: Text,
    claimedOccurrence :: TriggerOccurrence,
    claimedAttempt :: Word32,
    claimedFencingToken :: Text,
    claimedLeaseUntil :: UTCTime
  }
  deriving (Eq, Show)
