{-# LANGUAGE OverloadedStrings #-}

-- | Machine-local schedule/lifecycle storage, separate from workspace business
-- state and the existing typed task driver's databases.
module Segno.Service.Registry
  ( ServiceJob (..),
    ServiceRun (..),
    defaultRegistryPath,
    initialiseRegistry,
    registerServiceJob,
    listServiceJobs,
    setServiceJobEnabled,
    claimServiceRun,
    claimServiceRunBefore,
    finishServiceRun,
    recoverServiceRuns,
    serviceRunHistory,
  ) where

import Control.Exception (bracket, onException)
import Control.Monad (unless, when)
import Data.Aeson (ToJSON (toJSON), Value (Null), decodeStrict, encode, object, (.=))
import qualified Data.ByteString.Lazy as LBS
import Data.Char (isAlphaNum)
import Data.Int (Int64)
import Data.Text (Text)
import qualified Data.Text as Text
import Data.Text.Encoding (decodeUtf8, encodeUtf8)
import Data.Time (UTCTime, addUTCTime, diffUTCTime)
import Data.Time.Clock.POSIX (posixSecondsToUTCTime, utcTimeToPOSIXSeconds)
import Database.SQLite.Simple
  ( Connection, Only (..), close, execute, execute_, lastInsertRowId, open, query, query_ )
import Database.SQLite.Simple.FromRow (FromRow (fromRow), field)
import Segno.Service.Protocol (ServiceResult (..))
import System.Directory (XdgDirectory (XdgData), canonicalizePath, createDirectoryIfMissing, doesFileExist, getXdgDirectory)
import System.Environment (lookupEnv)
import System.FilePath ((</>), takeDirectory)

data ServiceJob = ServiceJob
  { serviceJobName :: Text,
    serviceJobRoot :: FilePath,
    serviceJobEverySeconds :: Int,
    serviceJobTimeoutSeconds :: Int,
    serviceJobEnabled :: Bool,
    serviceJobNextAt :: UTCTime
  }
  deriving (Eq, Show)

instance FromRow ServiceJob where
  fromRow = ServiceJob <$> field <*> field <*> field <*> field <*> field <*> (fromEpoch <$> field)

instance ToJSON ServiceJob where
  toJSON job = object
    [ "name" .= serviceJobName job, "root" .= serviceJobRoot job,
      "every_seconds" .= serviceJobEverySeconds job, "task_timeout_seconds" .= serviceJobTimeoutSeconds job,
      "enabled" .= serviceJobEnabled job, "next_at" .= serviceJobNextAt job
    ]

data ServiceRun = ServiceRun
  { serviceRunId :: Int64,
    serviceRunJob :: ServiceJob,
    serviceRunTimestamp :: UTCTime
  }
  deriving (Eq, Show)

defaultRegistryPath :: IO FilePath
defaultRegistryPath = do
  configured <- lookupEnv "SEGNO_REGISTRY"
  case configured of
    Just path | not (null path) -> pure path
    _ -> (</> "registry.sqlite3") <$> getXdgDirectory XdgData ("agenstro" </> "segno")

initialiseRegistry :: FilePath -> IO ()
initialiseRegistry path = do
  createDirectoryIfMissing True (takeDirectory path)
  withRegistry path $ \connection -> do
    execute_ connection "CREATE TABLE IF NOT EXISTS service_jobs (name TEXT PRIMARY KEY, root TEXT NOT NULL UNIQUE, every_seconds INTEGER NOT NULL, timeout_seconds INTEGER NOT NULL, enabled INTEGER NOT NULL, next_at REAL NOT NULL)"
    execute_ connection "CREATE TABLE IF NOT EXISTS service_runs (id INTEGER PRIMARY KEY AUTOINCREMENT, job TEXT NOT NULL REFERENCES service_jobs(name), logical_at REAL NOT NULL, started_at REAL NOT NULL, lease_until REAL NOT NULL, state TEXT NOT NULL, status_code INTEGER, info_json TEXT, finished_at REAL, UNIQUE(job, logical_at))"
    execute_ connection "CREATE INDEX IF NOT EXISTS service_runs_active ON service_runs(state, lease_until)"

registerServiceJob :: FilePath -> Text -> FilePath -> Int -> Int -> UTCTime -> IO ServiceJob
registerServiceJob path name root everySeconds timeoutSeconds now = do
  unless (not (Text.null name) && Text.length name <= 128 && Text.all (\c -> isAlphaNum c || c `elem` ("._-" :: String)) name) $
    fail "task name must contain 1..128 letters, digits, dots, underscores or hyphens"
  unless (everySeconds >= 1 && everySeconds <= 31536000) (fail "every-seconds must be in 1..31536000")
  unless (timeoutSeconds >= 1 && timeoutSeconds <= 604800) (fail "task-timeout-seconds must be in 1..604800")
  absoluteRoot <- canonicalizePath root
  scriptExists <- doesFileExist (absoluteRoot </> ".tactus" </> "segno.hs")
  configExists <- doesFileExist (absoluteRoot </> ".tactus" </> "tactus.toml")
  unless (scriptExists && configExists) (fail "registration requires an initialized workspace with .tactus/segno.hs")
  let job = ServiceJob name absoluteRoot everySeconds timeoutSeconds True now
  initialiseRegistry path
  withRegistry path $ \connection -> immediate connection $ do
    existing <- query connection "SELECT COUNT(*) FROM service_jobs WHERE name = ? OR root = ?" (name, absoluteRoot) :: IO [Only Int]
    unless (existing == [Only 0]) (fail "task name or canonical workspace is already registered")
    execute connection "INSERT INTO service_jobs(name, root, every_seconds, timeout_seconds, enabled, next_at) VALUES (?, ?, ?, ?, 1, ?)"
      (name, absoluteRoot, everySeconds, timeoutSeconds, epoch now)
  pure job

listServiceJobs :: FilePath -> IO [ServiceJob]
listServiceJobs path = withRegistry path $ \connection ->
  query_ connection "SELECT name, root, every_seconds, timeout_seconds, enabled, next_at FROM service_jobs ORDER BY name"

-- | Resume schedules a fresh future occurrence; it never replays an unknown
-- timestamp. Pause leaves any invocation already executing alone.
setServiceJobEnabled :: FilePath -> Text -> Bool -> UTCTime -> IO ()
setServiceJobEnabled path name enabled now = withRegistry path $ \connection -> immediate connection $ do
  exists <- query connection "SELECT COUNT(*) FROM service_jobs WHERE name = ?" (Only name) :: IO [Only Int]
  unless (exists == [Only 1]) (fail "unknown registered task")
  if enabled
    then execute connection "UPDATE service_jobs SET enabled = 1, next_at = ? + every_seconds WHERE name = ?" (epoch now, name)
    else execute connection "UPDATE service_jobs SET enabled = 0 WHERE name = ?" (Only name)

-- | Each short transaction chooses only one due workspace. Missed intervals
-- coalesce into the latest due timestamp, preventing unbounded catch-up.
claimServiceRun :: FilePath -> UTCTime -> IO (Maybe ServiceRun)
claimServiceRun path now = claimServiceRunBefore path now now

-- | A batch's fixed due cutoff is separate from the actual claim time. Slow
-- previous jobs must never consume the lease of a task that has not started.
claimServiceRunBefore :: FilePath -> UTCTime -> UTCTime -> IO (Maybe ServiceRun)
claimServiceRunBefore path cutoff claimedAt = withRegistry path $ \connection -> immediate connection $ do
  jobs <- query connection "SELECT name, root, every_seconds, timeout_seconds, enabled, next_at FROM service_jobs WHERE enabled = 1 AND next_at <= ? AND NOT EXISTS (SELECT 1 FROM service_runs WHERE job = service_jobs.name AND state = 'running') ORDER BY next_at, name LIMIT 1" (Only (epoch cutoff))
  case jobs of
    [] -> pure Nothing
    job : _ -> do
      let interval = fromIntegral (serviceJobEverySeconds job)
          missed = floor (diffUTCTime cutoff (serviceJobNextAt job) / interval) :: Integer
          timestamp = addUTCTime (fromInteger missed * interval) (serviceJobNextAt job)
          nextAt = addUTCTime interval timestamp
          leaseUntil = addUTCTime (fromIntegral (2 * serviceJobTimeoutSeconds job + 120)) claimedAt
      execute connection "INSERT INTO service_runs(job, logical_at, started_at, lease_until, state) VALUES (?, ?, ?, ?, 'running')"
        (serviceJobName job, epoch timestamp, epoch claimedAt, epoch leaseUntil)
      identity <- lastInsertRowId connection
      execute connection "UPDATE service_jobs SET next_at = ? WHERE name = ?" (epoch nextAt, serviceJobName job)
      pure (Just (ServiceRun identity job timestamp))

-- | A late process cannot overwrite a recovered unknown outcome. Missing or
-- invalid results disable the schedule until an operator explicitly resumes.
finishServiceRun :: FilePath -> ServiceRun -> Either Text ServiceResult -> UTCTime -> IO ()
finishServiceRun path run outcome now = withRegistry path $ \connection -> immediate connection $ do
  let checked = case outcome of
        Right result | resultTimestamp result /= serviceRunTimestamp run -> Left "result timestamp does not match invocation"
        Right result | resultStatusCode result < 0 || resultStatusCode result > 255 -> Left "invalid result status code"
        _ -> outcome
      (state, code, details) = case checked of
        Left reason -> ("outcome_unknown" :: Text, Nothing, object ["reason" .= reason])
        Right result -> (if resultStatusCode result == 0 then "succeeded" else "failed", Just (resultStatusCode result), resultInfo result)
  execute connection "UPDATE service_runs SET state = ?, status_code = ?, info_json = ?, finished_at = ? WHERE id = ? AND state = 'running'"
    (state, code, jsonText details, epoch now, serviceRunId run)
  changed <- query_ connection "SELECT changes()" :: IO [Only Int]
  when (changed == [Only 1] && state == "outcome_unknown") $
    execute connection "UPDATE service_jobs SET enabled = 0 WHERE name = ?" (Only (serviceJobName (serviceRunJob run)))

recoverServiceRuns :: FilePath -> UTCTime -> IO Int
recoverServiceRuns path now = withRegistry path $ \connection -> immediate connection $ do
  execute connection "UPDATE service_jobs SET enabled = 0 WHERE name IN (SELECT job FROM service_runs WHERE state = 'running' AND lease_until <= ?)" (Only (epoch now))
  execute connection "UPDATE service_runs SET state = 'outcome_unknown', finished_at = ?, info_json = ? WHERE state = 'running' AND lease_until <= ?"
    (epoch now, jsonText (object ["reason" .= ("invocation lease expired; external effects may have occurred; inspect before resume" :: Text)]), epoch now)
  changed <- query_ connection "SELECT changes()" :: IO [Only Int]
  pure (sum [count | Only count <- changed])

data HistoryRow = HistoryRow Int64 Text Double Double Text (Maybe Int) (Maybe Text) (Maybe Double) Double
instance FromRow HistoryRow where
  fromRow = HistoryRow <$> field <*> field <*> field <*> field <*> field <*> field <*> field <*> field <*> field

serviceRunHistory :: FilePath -> Int -> IO [Value]
serviceRunHistory path limit = withRegistry path $ \connection -> do
  rows <- query connection "SELECT id, job, logical_at, started_at, state, status_code, info_json, finished_at, lease_until FROM service_runs ORDER BY id DESC LIMIT ?" (Only (max 1 (min 1000 limit)))
  pure [object
    [ "id" .= identity, "job" .= job, "timestamp" .= fromEpoch logical,
      "started_at" .= fromEpoch started, "state" .= state, "status_code" .= code,
      "info" .= maybe Null (maybe Null id . decodeStrict . encodeUtf8) details,
      "finished_at" .= fmap fromEpoch finished, "lease_until" .= fromEpoch lease
    ] | HistoryRow identity job logical started state code details finished lease <- rows]

withRegistry :: FilePath -> (Connection -> IO a) -> IO a
withRegistry path action = bracket (open path) close $ \connection -> do
  -- PRAGMA preparation can itself need a database lock, so install the busy
  -- handler before journal mode or any other database-dependent setup.
  execute_ connection "PRAGMA busy_timeout = 5000"
  execute_ connection "PRAGMA journal_mode = WAL"
  execute_ connection "PRAGMA foreign_keys = ON"
  execute_ connection "PRAGMA synchronous = FULL"
  action connection

immediate :: Connection -> IO a -> IO a
immediate connection action = do
  execute_ connection "BEGIN IMMEDIATE"
  result <- action `onException` execute_ connection "ROLLBACK"
  execute_ connection "COMMIT"
  pure result

epoch :: UTCTime -> Double
epoch = realToFrac . utcTimeToPOSIXSeconds

fromEpoch :: Double -> UTCTime
fromEpoch = posixSecondsToUTCTime . realToFrac

jsonText :: Value -> Text
jsonText = decodeUtf8 . LBS.toStrict . encode
