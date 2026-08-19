{-# OPTIONS_GHC -Wno-unused-imports #-}

module MonadIOCompatibility
  ( standardLiftIO,
  )
where

import Clef
import Control.Monad.IO.Class

-- Importing both modules unqualified must expose the same class method rather
-- than two unrelated liftIO definitions.
standardLiftIO :: IO value -> Workflow value
standardLiftIO = liftIO
