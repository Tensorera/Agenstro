{-# LANGUAGE OverloadedStrings #-}

-- Copy to WORKSPACE/.tactus/segno.hs after `segno init --root WORKSPACE`.
-- This organizer owns timestamp/work-directory state, while business code
-- accepts an explicit working directory and can also run independently.
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
  -- A process failure can represent ambiguous external effects. Let it throw
  -- so Segno pauses the schedule instead of inferring a known business result.
  callProcess "tactus"
    [ "run", "--root", root, "--script", root </> ".tactus" </> "scripts" </> "010_business.hs",
      "--", workDirectory
    ]
  let details = object ["timestamp" .= timestamp, "work_directory" .= workDirectory, "status_code" .= (0 :: Int)]
      temporary = stateDirectory </> "latest.tmp"
  LBS.writeFile temporary (encode details)
  renameFile temporary (stateDirectory </> "latest.json")
  pure (0, details)
