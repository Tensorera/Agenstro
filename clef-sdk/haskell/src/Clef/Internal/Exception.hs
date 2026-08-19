module Clef.Internal.Exception
  ( isAsynchronousException,
  )
where

import Control.Exception (SomeAsyncException, SomeException, fromException)

-- | Recognize every exception registered below 'SomeAsyncException', not only
-- the RTS-owned @AsyncException@ constructors.  This includes cancellation
-- types such as @System.Timeout.Timeout@ and @AsyncCancelled@.
isAsynchronousException :: SomeException -> Bool
isAsynchronousException exception =
  case fromException exception :: Maybe SomeAsyncException of
    Just _ -> True
    Nothing -> False
