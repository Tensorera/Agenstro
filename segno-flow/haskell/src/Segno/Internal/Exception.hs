-- | Shared by the driver, plugin hosts, and executable boundary. Cancellation
-- is control flow and must never become a retryable plugin or task failure.
module Segno.Internal.Exception (trySynchronous) where

import Control.Exception (SomeAsyncException, SomeException, fromException, tryJust)

trySynchronous :: IO value -> IO (Either SomeException value)
trySynchronous = tryJust synchronous
  where
    synchronous exception = case fromException exception :: Maybe SomeAsyncException of
      Just _ -> Nothing
      Nothing -> Just exception
