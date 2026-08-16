{-# LANGUAGE CPP #-}
{-# LANGUAGE OverloadedStrings #-}

-- | A typed Clef wrapper plus the built-in active-window plugin
-- implementation.  The implementation uses the maintained @Win32@ package;
-- Segno contains no handwritten foreign imports.
module Segno.ActiveWindow
  ( ActiveWindow (..),
    currentActiveWindow,
    captureActiveWindow,
  )
where

import Data.Aeson
  ( FromJSON (parseJSON),
    ToJSON (toJSON),
    Value (Object),
    object,
    withObject,
    (.:),
    (.=),
  )
import qualified Data.Aeson.KeyMap as KeyMap
import Data.Text (Text)
import qualified Data.Text as Text
import Data.Time (UTCTime, getCurrentTime)
import Clef.Workflow (Plugin, Workflow, call, jsonPlugin)
import Segno.Protocol (PluginFailure (..))

#if defined(mingw32_HOST_OS)
import Foreign.Ptr (nullPtr)
import Graphics.Win32.Window (getForegroundWindow, getWindowText, getWindowTextLength)
#endif

data ActiveWindow = ActiveWindow
  { activeWindowTitle :: Text,
    activeWindowCapturedAt :: UTCTime,
    activeWindowPlatform :: Text
  }
  deriving (Eq, Show)

instance FromJSON ActiveWindow where
  parseJSON = withObject "active window" $ \fields ->
    ActiveWindow
      <$> fields .: "title"
      <*> fields .: "captured_at"
      <*> fields .: "platform"

instance ToJSON ActiveWindow where
  toJSON window =
    object
      [ "title" .= activeWindowTitle window,
        "captured_at" .= activeWindowCapturedAt window,
        "platform" .= activeWindowPlatform window
      ]

data EmptyRequest = EmptyRequest

instance ToJSON EmptyRequest where
  toJSON _ = Object KeyMap.empty

currentActiveWindow :: Workflow ActiveWindow
currentActiveWindow =
  call
    (jsonPlugin "system.active-window" "current" :: Plugin EmptyRequest ActiveWindow)
    EmptyRequest

captureActiveWindow :: IO (Either PluginFailure ActiveWindow)
#if defined(mingw32_HOST_OS)
captureActiveWindow = do
  capturedAt <- getCurrentTime
  handle <- getForegroundWindow
  if handle == nullPtr
    then
      pure . Left $
        PluginFailure
          "active_window_unavailable"
          "Windows did not report a foreground window"
          Nothing
    else do
      titleLength <- getWindowTextLength handle
      title <- getWindowText handle (max 1 (titleLength + 1))
      pure . Right $
        ActiveWindow
          { activeWindowTitle = Text.pack title,
            activeWindowCapturedAt = capturedAt,
            activeWindowPlatform = "windows"
          }
#else
captureActiveWindow =
  pure . Left $
    PluginFailure
      "unsupported_platform"
      "the built-in active-window plugin currently supports Windows only"
      (Just (object ["platform" .= ("non-windows" :: Text)]))
#endif
