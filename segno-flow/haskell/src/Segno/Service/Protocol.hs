{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE CPP #-}

-- | Contract for the workspace's ordinary @.tactus/segno.hs@ organizer.
module Segno.Service.Protocol
  ( ServiceResult (..),
    serviceResultApi,
    serviceMain,
    decodeServiceResult,
    readServiceResultFile,
    renderTimestamp,
  ) where

import Control.Exception (IOException, bracketOnError, catch)
import Control.Monad (unless, when)
import Data.Aeson (FromJSON (parseJSON), ToJSON (toJSON), Value, encode, object, withObject, (.:), (.=))
import qualified Data.Aeson.KeyMap as KeyMap
import qualified Data.ByteString as BS
import qualified Data.ByteString.Lazy as LBS
import Data.Text (Text)
import Data.Text.Encoding (decodeUtf8')
import Data.Time (UTCTime, defaultTimeLocale, formatTime, parseTimeM)
import Segno.Protocol (decodeJsonText)
import System.Directory (removeFile, renameFile)
import System.Environment (getArgs, getEnv)
import System.FilePath (takeDirectory)
import System.IO (IOMode (ReadMode), hClose, openBinaryTempFile, withBinaryFile)
#ifdef mingw32_HOST_OS
import System.Directory (doesFileExist, pathIsSymbolicLink)
#else
import System.Posix.Files (getSymbolicLinkStatus, isRegularFile)
#endif

data ServiceResult = ServiceResult
  { resultTimestamp :: UTCTime,
    resultStatusCode :: Int,
    resultInfo :: Value
  }
  deriving (Eq, Show)

serviceResultApi :: Text
serviceResultApi = "agenstro.segno-service-result/v1"

instance ToJSON ServiceResult where
  toJSON result = object
    [ "api" .= serviceResultApi,
      "timestamp" .= resultTimestamp result,
      "status_code" .= resultStatusCode result,
      "info" .= resultInfo result
    ]

instance FromJSON ServiceResult where
  parseJSON = withObject "Segno service result" $ \fields -> do
    unless (all (`elem` ["api", "timestamp", "status_code", "info"]) (KeyMap.keys fields)) $
      fail "unknown Segno service result field"
    api <- fields .: "api"
    unless (api == serviceResultApi) (fail "unsupported Segno service result API")
    timestamp <- fields .: "timestamp"
    code <- fields .: "status_code"
    unless (code >= 0 && code <= 255) (fail "status_code must be an integer in 0..255")
    ServiceResult timestamp code <$> fields .: "info"

-- | Decode a bounded, strict JSON result and correlate its logical timestamp.
decodeServiceResult :: UTCTime -> BS.ByteString -> Either String ServiceResult
decodeServiceResult timestamp bytes = do
  when (BS.length bytes > 1024 * 1024) (Left "Segno service result exceeded 1 MiB")
  encoded <- either (const (Left "Segno service result is not UTF-8")) Right (decodeUtf8' bytes)
  result <- decodeJsonText encoded
  unless (resultTimestamp result == timestamp) (Left "Segno service result timestamp does not match invocation")
  pure result

-- | Reject pipes, devices, directories and symlinks before opening the result.
-- A FIFO produced by a buggy organizer must not hang the scheduler after the
-- supervised process deadline has already ended. Scripts are trusted local
-- code; this file check is not a defense against a malicious filesystem race.
readServiceResultFile :: UTCTime -> FilePath -> IO (Either String ServiceResult)
readServiceResultFile timestamp path = do
#ifdef mingw32_HOST_OS
  regular <- (&&) <$> doesFileExist path <*> (not <$> pathIsSymbolicLink path)
#else
  regular <- isRegularFile <$> getSymbolicLinkStatus path
#endif
  if not regular
    then pure (Left "Segno service result must be a regular file, not a symlink or special file")
    else do
      bytes <- withBinaryFile path ReadMode (\handle -> BS.hGet handle (1024 * 1024 + 1))
      pure (decodeServiceResult timestamp bytes)

renderTimestamp :: UTCTime -> String
renderTimestamp = formatTime defaultTimeLocale "%Y-%m-%dT%H:%M:%S%QZ"

-- | Parse the sole UTC timestamp argument and publish a correlated result
-- atomically. A nonzero business status is a known failure; the script itself
-- still exits successfully after publication. Diagnostics may use stdout.
serviceMain :: (UTCTime -> IO (Int, Value)) -> IO ()
serviceMain action = do
  arguments <- getArgs
  timestamp <- case arguments of
    [encoded] -> maybe (fail "expected an ISO-8601 UTC timestamp") pure $
      parseTimeM True defaultTimeLocale "%Y-%m-%dT%H:%M:%S%QZ" encoded
    _ -> fail "usage: segno.hs TIMESTAMP"
  destination <- getEnv "SEGNO_SERVICE_RESULT_PATH"
  (code, details) <- action timestamp
  let result = ServiceResult timestamp code details
      bytes = LBS.toStrict (encode result)
  either fail (const (pure ())) (decodeServiceResult timestamp bytes)
  bracketOnError
    (openBinaryTempFile (takeDirectory destination) "segno-result.tmp")
    (\(path, handle) -> (hClose handle `catch` ignoreIO) >> (removeFile path `catch` ignoreIO))
    (\(path, handle) -> BS.hPut handle bytes >> hClose handle >> renameFile path destination)
  where
    ignoreIO :: IOException -> IO ()
    ignoreIO _ = pure ()
