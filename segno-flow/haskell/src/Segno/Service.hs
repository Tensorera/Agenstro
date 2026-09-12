{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE ScopedTypeVariables #-}

module Segno.Service
  ( ServiceEnvironment (..),
    defaultServiceEnvironment,
    serviceOnceWith,
    runServiceLoop,
    serviceProcessRunner,
  ) where

import Control.Exception (IOException, bracket, catch, displayException, onException)
import Control.Monad (void)
import Data.Aeson (Value, object, (.=))
import qualified Data.ByteString as BS
import qualified Data.Map.Strict as Map
import Data.Text (Text)
import qualified Data.Text as Text
import Data.Text.Encoding (decodeUtf8With)
import Data.Text.Encoding.Error (lenientDecode)
import Data.Time (addUTCTime)
import Segno.Driver (Clock (..), ProcessOutcome (..), runProcessBytes, systemClock)
import Segno.Service.Protocol (ServiceResult, readServiceResultFile, renderTimestamp)
import Segno.Service.Registry
import System.Directory (createDirectory, createDirectoryIfMissing, makeAbsolute, removeDirectoryRecursive)
import System.Environment (getEnvironment)
import System.Exit (ExitCode (ExitSuccess))
import System.FilePath ((</>))
import System.Process (CreateProcess (..), proc)

data ServiceEnvironment = ServiceEnvironment
  { serviceClock :: Clock,
    serviceRunner :: ServiceRun -> IO (Either Text ServiceResult)
  }

defaultServiceEnvironment :: FilePath -> ServiceEnvironment
defaultServiceEnvironment registry = ServiceEnvironment systemClock (serviceProcessRunner registry)

-- | Work is claimed transactionally. Each invocation runs outside the database
-- transaction, and the clock/runner boundary permits offline virtual-time tests.
serviceOnceWith :: ServiceEnvironment -> FilePath -> IO Value
serviceOnceWith environment registry = do
  initialiseRegistry registry
  now <- clockNow (serviceClock environment)
  recovered <- recoverServiceRuns registry now
  executed <- drain now (0 :: Int)
  pure (object ["recovered_unknown" .= recovered, "executed" .= executed])
  where
    drain snapshot count
      | count >= 100 = pure count
      | otherwise = do
          claimedAt <- clockNow (serviceClock environment)
          claimed <- claimServiceRunBefore registry snapshot claimedAt
          case claimed of
            Nothing -> pure count
            Just run -> do
              let interrupted = do
                    finished <- clockNow (serviceClock environment)
                    finishServiceRun registry run (Left "service interrupted; inspect external effects before resume") finished
              outcome <- (serviceRunner environment run `catch` ioFailure) `onException` interrupted
              finished <- clockNow (serviceClock environment)
              finishServiceRun registry run outcome finished
              drain snapshot (count + 1)
    ioFailure (exception :: IOException) = pure (Left (Text.pack (displayException exception)))

runServiceLoop :: ServiceEnvironment -> FilePath -> IO ()
runServiceLoop environment registry = loop
  where
    loop = do
      void (serviceOnceWith environment registry)
      now <- clockNow (serviceClock environment)
      clockSleepUntil (serviceClock environment) (addUTCTime 1 now)
      loop

-- | All business code executes through Tactus. Only a valid correlated result
-- and successful process exit constitute a known outcome. A build error is
-- conservatively unknown too: process failure alone proves no business result.
serviceProcessRunner :: FilePath -> ServiceRun -> IO (Either Text ServiceResult)
serviceProcessRunner registry run = do
  absoluteRegistry <- makeAbsolute registry
  let exchangeRoot = absoluteRegistry <> ".exchanges"
      exchange = exchangeRoot </> show (serviceRunId run)
      resultPath = exchange </> "result.json"
      job = serviceRunJob run
      root = serviceJobRoot job
  createDirectoryIfMissing True exchangeRoot
  bracket (createDirectory exchange) (const (removeDirectoryRecursive exchange `catch` (\(_ :: IOException) -> pure ()))) $ \() -> do
    inherited <- getEnvironment
    let environment = Map.toList $
          Map.insert "SEGNO_SERVICE_RESULT_PATH" resultPath $
          Map.insert "SEGNO_SERVICE_JOB" (Text.unpack (serviceJobName job)) $
          Map.insert "TACTUS_TASK_NAME" (Text.unpack (serviceJobName job)) $
          Map.insert "TACTUS_OCCURRENCE_ID" ("segno-service:" <> show (serviceRunId run)) $
          foldr Map.delete (Map.fromList inherited) ["SEGNO_MODE", "SEGNO_RESULT_PATH", "SEGNO_INVOCATION_PATH"]
        command = (proc "tactus"
          [ "run", "--root", root, "--script", root </> ".tactus" </> "segno.hs",
            "--package", "segno-flow", "--timeout-seconds", show (serviceJobTimeoutSeconds job),
            "--", renderTimestamp (serviceRunTimestamp run)
          ]) {cwd = Just root, env = Just environment}
    process <- runProcessBytes command BS.empty
    if processExitCode process /= ExitSuccess || processOutputTruncated process
      then pure (Left ("Tactus did not complete successfully: " <> Text.pack (show (processExitCode process)) <> "; " <>
        decodeUtf8With lenientDecode (BS.take 16384 (processStderr process))))
      else do
        result <- readServiceResultFile (serviceRunTimestamp run) resultPath
        pure (either (Left . Text.pack) Right result)
