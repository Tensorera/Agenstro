{-# LANGUAGE OverloadedStrings #-}
module Main (main) where

-- Install at .tactus/segno.hs. Segno supplies the timestamp argument and
-- SEGNO_SERVICE_RESULT_PATH; business code receives its own work directory.
import Data.Aeson (encode, object, (.=))
import qualified Data.ByteString.Lazy as LBS
import Data.Time (defaultTimeLocale, formatTime)
import Segno.Service.Protocol (serviceMain)
import System.Directory (createDirectoryIfMissing, getCurrentDirectory, renameFile)
import System.FilePath ((</>))
import System.Process (callProcess)

main :: IO ()
main = serviceMain $ \timestamp -> do
  root <- getCurrentDirectory
  let workDirectory = root </> ".tactus" </> "segno-work" </> formatTime defaultTimeLocale "%Y%m%dT%H%M%S%qZ" timestamp
      stateDirectory = root </> ".tactus" </> "segno-service-state"
  createDirectoryIfMissing True workDirectory
  createDirectoryIfMissing True stateDirectory
  -- A failed subprocess may have changed external state. Propagate the
  -- exception so Segno records uncertainty instead of guessing a known result.
  callProcess "tactus"
    [ "run", "--root", root, "--script", root </> ".tactus" </> "scripts" </> "010_business.hs",
      "--", workDirectory
    ]
  let details = object ["timestamp" .= timestamp, "work_directory" .= workDirectory, "status_code" .= (0 :: Int)]
      temporary = stateDirectory </> "latest.tmp"
  LBS.writeFile temporary (encode details)
  renameFile temporary (stateDirectory </> "latest.json")
  pure (0, details)
