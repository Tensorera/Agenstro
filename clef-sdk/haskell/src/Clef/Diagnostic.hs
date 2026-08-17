{-# LANGUAGE OverloadedStrings #-}

-- | Typed observations emitted by the Clef runtime.
--
-- A state transition is deliberately more specific than a log message.  It
-- records the state on both sides, the trigger that requested the change, and
-- the guard that justified it.  Provider progress and raw protocol frames are
-- not transitions and must use a different record type.
module Clef.Diagnostic
  ( PresentationLevel (..),
    TriggerKind (..),
    TransitionTrigger (..),
    TransitionGuard (..),
    RuntimeStateTransition (..),
    RuntimeMessage (..),
    renderPresentationLine,
    renderStateTransition,
    renderRuntimeMessage,
  )
where

import Data.Aeson
  ( Object,
    ToJSON (toJSON),
    object,
    (.=),
  )
import Data.Maybe (catMaybes)
import Data.Text (Text)
import qualified Data.Text as Text

-- | The only four labels admitted by the human-facing projection.
data PresentationLevel
  = StateLevel
  | InfoLevel
  | WarningLevel
  | ErrorLevel
  deriving (Eq, Show)

instance ToJSON PresentationLevel where
  toJSON = toJSON . presentationLevelName

-- | Closed trigger kinds keep transition histories comparable across
-- providers while their source, code, and optional detail remain open text.
data TriggerKind
  = RequestTrigger
  | EventTrigger
  | TimerTrigger
  | InternalResultTrigger
  | ControlTrigger
  deriving (Eq, Show)

instance ToJSON TriggerKind where
  toJSON = toJSON . triggerKindName

data TransitionTrigger = TransitionTrigger
  { transitionTriggerKind :: TriggerKind,
    transitionTriggerSource :: Text,
    transitionTriggerCode :: Text,
    transitionTriggerDetails :: Maybe Text
  }
  deriving (Eq, Show)

instance ToJSON TransitionTrigger where
  toJSON trigger =
    object . catMaybes $
      [ Just ("kind" .= transitionTriggerKind trigger),
        Just ("source" .= transitionTriggerSource trigger),
        Just ("code" .= transitionTriggerCode trigger),
        ("details" .=) <$> transitionTriggerDetails trigger
      ]

data TransitionGuard = TransitionGuard
  { transitionGuardCondition :: Text,
    transitionGuardPassed :: Bool,
    transitionGuardReason :: Text
  }
  deriving (Eq, Show)

instance ToJSON TransitionGuard where
  toJSON guard =
    object
      [ "condition" .= transitionGuardCondition guard,
        "passed" .= transitionGuardPassed guard,
        "reason" .= transitionGuardReason guard
      ]

-- | A genuine state change.  All four explanatory parts are mandatory.
-- 'stateTransitionContext' carries diagnostic correlation data but is not
-- rendered to a user's terminal.
data RuntimeStateTransition = RuntimeStateTransition
  { stateTransitionCode :: Text,
    stateTransitionMessage :: Text,
    stateTransitionSubject :: Text,
    stateTransitionStateBefore :: Text,
    stateTransitionTrigger :: TransitionTrigger,
    stateTransitionGuard :: TransitionGuard,
    stateTransitionStateAfter :: Text,
    stateTransitionContext :: Object
  }
  deriving (Eq, Show)

instance ToJSON RuntimeStateTransition where
  toJSON transition =
    object
      [ "type" .= ("state_transition" :: Text),
        "code" .= stateTransitionCode transition,
        "level" .= StateLevel,
        "message" .= stateTransitionMessage transition,
        "subject" .= stateTransitionSubject transition,
        "state_before" .= stateTransitionStateBefore transition,
        "trigger" .= stateTransitionTrigger transition,
        "guard" .= stateTransitionGuard transition,
        "state_after" .= stateTransitionStateAfter transition,
        "context" .= stateTransitionContext transition
      ]

-- | A normalized observation that is useful to a person but does not claim a
-- state change.  Raw provider frames are intentionally not 'RuntimeMessage's.
data RuntimeMessage = RuntimeMessage
  { runtimeMessageCode :: Text,
    runtimeMessageLevel :: PresentationLevel,
    runtimeMessageText :: Text,
    runtimeMessageContext :: Object
  }
  deriving (Eq, Show)

instance ToJSON RuntimeMessage where
  toJSON message =
    object
      [ "type" .= ("message" :: Text),
        "code" .= runtimeMessageCode message,
        "level" .= runtimeMessageLevel message,
        "message" .= runtimeMessageText message,
        "context" .= runtimeMessageContext message
      ]

renderPresentationLine :: PresentationLevel -> Text -> Text
renderPresentationLine level message =
  "[" <> presentationLevelName level <> "] " <> singleLine message

renderStateTransition :: RuntimeStateTransition -> Text
renderStateTransition transition =
  renderPresentationLine
    StateLevel
    (stateTransitionMessage transition)

renderRuntimeMessage :: RuntimeMessage -> Text
renderRuntimeMessage message =
  renderPresentationLine (runtimeMessageLevel message) (runtimeMessageText message)

presentationLevelName :: PresentationLevel -> Text
presentationLevelName level = case level of
  StateLevel -> "state"
  InfoLevel -> "info"
  WarningLevel -> "warning"
  ErrorLevel -> "error"

triggerKindName :: TriggerKind -> Text
triggerKindName kind = case kind of
  RequestTrigger -> "request"
  EventTrigger -> "event"
  TimerTrigger -> "timer"
  InternalResultTrigger -> "internal_result"
  ControlTrigger -> "control"

singleLine :: Text -> Text
singleLine = Text.unwords . Text.words
