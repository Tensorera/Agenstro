{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE ScopedTypeVariables #-}
{-# LANGUAGE CPP #-}

module ServiceTests (runServiceTests) where

import Control.Concurrent (newEmptyMVar, putMVar, takeMVar)
import Control.Concurrent.Async (concurrently, wait, waitCatch, withAsync)
import Control.Exception (IOException, bracket, catch, finally, try)
import Control.Monad (forM_, unless, void)
import Data.Aeson (Value (..), encode, object, (.=))
import qualified Data.Aeson.KeyMap as KeyMap
import Data.Aeson.Key (Key)
import qualified Data.ByteString as BS
import qualified Data.ByteString.Char8 as Char8
import qualified Data.ByteString.Lazy as LBS
import Data.IORef (modifyIORef', newIORef, readIORef, writeIORef)
import Data.Maybe (catMaybes, isNothing)
import Data.Time (UTCTime, addUTCTime, getCurrentTime)
import qualified Database.SQLite.Simple as SQLite
import Network.Socket
  ( Family (AF_INET), SocketType (Stream), SockAddr (SockAddrInet), bind, close,
    connect, defaultProtocol, getSocketName, socket, tupleToHostAddress )
import Network.Socket.ByteString (recv, sendAll)
import Segno.Driver (Clock (..))
import Segno.Service (ServiceEnvironment (..), serviceOnceWith)
import Segno.Service.Protocol (ServiceResult (..), decodeServiceResult, readServiceResultFile)
import Segno.Service.Registry
import Segno.Service.Web (withServiceWeb)
import qualified Segno.Store.SQLite as Store
import System.Directory (createDirectory, createDirectoryIfMissing, getTemporaryDirectory, removeDirectoryRecursive)
import System.FilePath ((</>))
import System.Timeout (timeout)
#ifndef mingw32_HOST_OS
import System.Posix.Files (createNamedPipe, createSymbolicLink)
#endif

runServiceTests :: IO ()
runServiceTests = do
  test "strict timestamp/result contract" testResultContract
  test "result handoff rejects directories and special files" testResultFile
  test "machine registry rejects duplicate workspace and invalid interval" testRegistration
  test "database connection setup waits for a competing lock" testBusyConnectionSetup
  test "concurrent claims exclude overlapping workspace invocation" testConcurrentClaims
  test "coalesced due timestamp and durable success/failure" testScheduling
  test "expired work pauses without replay; stale completion cannot overwrite" testRecovery
  test "malformed outcome is isolated from other workspace tasks" testIsolation
  test "slow prior task does not consume the next task lease" testFreshClaimLease
  test "result timestamp mismatch pauses schedule" testMismatchedResult
  test "status HTTP remains responsive while task is running" testWebDuringTask
  where
    test label action = putStr ("[test] service " <> label <> " ... ") >> action >> putStrLn "ok"

assert :: Bool -> String -> IO ()
assert condition message = unless condition (fail message)

now :: UTCTime
now = read "2026-09-12 12:00:00 UTC"

testResultContract :: IO ()
testResultContract = do
  let good = ServiceResult now 0 (object ["message" .= ("finished" :: String)])
      bytes = LBS.toStrict (encode good)
  assert (decodeServiceResult now bytes == Right good) "valid result failed"
  assert (isLeft (decodeServiceResult (addUTCTime 1 now) bytes)) "mismatched timestamp accepted"
  assert (isLeft (decodeServiceResult now (BS.replicate (1024 * 1024 + 1) 32))) "oversized result accepted"
  assert (isLeft (decodeServiceResult now (LBS.toStrict (encode (ServiceResult now 256 Null))))) "out-of-range status accepted"
  assert (isLeft (decodeServiceResult now "{\"api\":\"agenstro.segno-service-result/v1\",\"timestamp\":\"2026-09-12T12:00:00Z\",\"status_code\":0,\"status_code\":1,\"info\":null}")) "duplicate status accepted"
  assert (isLeft (decodeServiceResult now "{}")) "missing fields accepted"

testRegistration :: IO ()
testRegistration = withFixture "registration" $ \registry root -> do
  job <- registerServiceJob registry "alpha" root 60 30 now
  jobs <- listServiceJobs registry
  assert (jobs == [job]) "registered workspace did not persist"
  duplicate <- try (registerServiceJob registry "another" root 60 30 now) :: IO (Either IOException ServiceJob)
  assert (isLeft duplicate) "duplicate workspace was accepted"
  invalid <- try (registerServiceJob registry "bad" root 0 30 now) :: IO (Either IOException ServiceJob)
  assert (isLeft invalid) "zero schedule interval accepted"

testResultFile :: IO ()
testResultFile = withFixture "result-file" $ \_ root -> do
  let file = root </> "result.json"
      result = ServiceResult now 0 Null
  LBS.writeFile file (encode result)
  decoded <- readServiceResultFile now file
  assert (decoded == Right result) "regular result file was rejected"
  directory <- readServiceResultFile now root
  assert (isLeft directory) "directory result accepted"
#ifndef mingw32_HOST_OS
  let fifo = root </> "fifo.json"
      link = root </> "link.json"
  createNamedPipe fifo 0o600
  pipe <- timeout 1000000 (readServiceResultFile now fifo)
  assert (maybe False isLeft pipe) "FIFO result was accepted or blocked"
  createSymbolicLink file link
  symbolic <- readServiceResultFile now link
  assert (isLeft symbolic) "symlink result was accepted"
#endif

testConcurrentClaims :: IO ()
testConcurrentClaims = withFixture "claims" $ \registry root -> do
  void (registerServiceJob registry "alpha" root 60 30 now)
  (first, second) <- concurrently (claimServiceRun registry now) (claimServiceRun registry now)
  assert (length (catMaybes [first, second]) == 1) "two processes claimed the same workspace"
  blocked <- claimServiceRun registry (addUTCTime 60 now)
  assert (blocked == Nothing) "active workspace overlapped at next interval"

testBusyConnectionSetup :: IO ()
testBusyConnectionSetup = withFixture "busy-setup" $ \registry root -> do
  initialiseRegistry registry
  let paths = Store.segnoPaths root
  Store.initialiseStore paths
  forM_
    [ (registry, void (listServiceJobs registry)),
      (Store.pathsBusinessDatabase paths, void (Store.businessHistory paths Nothing 1)),
      (Store.pathsLifecycleDatabase paths, void (Store.lifecycleStatus paths Nothing))
    ] $ \(path, inspect) -> bracket (SQLite.open path) SQLite.close $ \blocker -> do
      -- A rollback-journal exclusive writer also blocks schema reads during
      -- PRAGMA preparation, deterministically exercising connection setup.
      SQLite.execute_ blocker "PRAGMA journal_mode = DELETE"
      SQLite.execute_ blocker "BEGIN EXCLUSIVE"
      started <- newEmptyMVar
      withAsync (putMVar started () >> inspect) $ \reader -> do
        early <- (takeMVar started >> timeout 100000 (waitCatch reader))
          `finally` SQLite.execute_ blocker "ROLLBACK"
        assert (isNothing early) ("connection setup did not wait for the competing lock: " <> show early <> ": " <> path)
        completed <- timeout 2000000 (wait reader)
        assert (completed == Just ()) ("connection setup did not recover after lock release: " <> path)

testScheduling :: IO ()
testScheduling = withFixture "schedule" $ \registry root -> do
  void (registerServiceJob registry "alpha" root 60 30 now)
  run <- requireClaim registry (addUTCTime 185 now)
  assert (serviceRunTimestamp run == addUTCTime 180 now) "missed ticks were not coalesced to latest due timestamp"
  finishServiceRun registry run (Right (ServiceResult (serviceRunTimestamp run) 0 Null)) (addUTCTime 190 now)
  jobs <- listServiceJobs registry
  assert (map serviceJobNextAt jobs == [addUTCTime 240 now]) "schedule drifted from initial anchor"
  later <- requireClaim registry (addUTCTime 240 now)
  finishServiceRun registry later (Right (ServiceResult (serviceRunTimestamp later) 7 (String "known failure"))) (addUTCTime 241 now)
  history <- serviceRunHistory registry 10
  assert (map (lookupValue "state") history == [Just (String "failed"), Just (String "succeeded")]) "terminal results were not durable"
  assert (map (lookupValue "status_code") history == [Just (Number 7), Just (Number 0)]) "status codes were lost"
  initialised <- listServiceJobs registry
  assert (all serviceJobEnabled initialised) "known failure unexpectedly disabled future intervals"

testRecovery :: IO ()
testRecovery = withFixture "recovery" $ \registry root -> do
  void (registerServiceJob registry "alpha" root 60 30 now)
  old <- requireClaim registry now
  early <- recoverServiceRuns registry (addUTCTime 179 now)
  assert (early == 0) "live lease was recovered early"
  expired <- recoverServiceRuns registry (addUTCTime 180 now)
  assert (expired == 1) "expired running occurrence did not recover"
  finishServiceRun registry old (Right (ServiceResult now 0 Null)) (addUTCTime 181 now)
  history <- serviceRunHistory registry 10
  assert (map (lookupValue "state") history == [Just (String "outcome_unknown")]) "late process overwrote recovered unknown outcome"
  nothing <- claimServiceRun registry (addUTCTime 240 now)
  assert (nothing == Nothing) "unknown outcome retried automatically"
  setServiceJobEnabled registry "alpha" True (addUTCTime 240 now)
  before <- claimServiceRun registry (addUTCTime 299 now)
  assert (before == Nothing) "resume immediately replayed uncertain work"
  fresh <- requireClaim registry (addUTCTime 300 now)
  assert (serviceRunTimestamp fresh == addUTCTime 300 now) "resume did not use fresh timestamp"

testIsolation :: IO ()
testIsolation = withFixture "isolation" $ \registry root -> do
  secondRoot <- prepareWorkspace (root </> "other")
  void (registerServiceJob registry "alpha" root 60 30 now)
  void (registerServiceJob registry "beta" secondRoot 60 30 now)
  calls <- newIORef (0 :: Int)
  let environment = ServiceEnvironment (Clock (pure now) (const (pure ()))) $ \run -> do
        modifyIORef' calls (+ 1)
        if serviceJobName (serviceRunJob run) == "alpha" then ioError (userError "missing result")
          else pure (Right (ServiceResult (serviceRunTimestamp run) 0 Null))
  void (serviceOnceWith environment registry)
  count <- readIORef calls
  jobs <- listServiceJobs registry
  assert (count == 2 && map serviceJobEnabled jobs == [False, True]) "one failed workspace starved or paused another"

testMismatchedResult :: IO ()
testMismatchedResult = withFixture "correlation" $ \registry root -> do
  void (registerServiceJob registry "alpha" root 60 30 now)
  run <- requireClaim registry now
  finishServiceRun registry run (Right (ServiceResult (addUTCTime 1 now) 0 Null)) now
  jobs <- listServiceJobs registry
  assert (not (any serviceJobEnabled jobs)) "uncorrelated result was accepted"

testFreshClaimLease :: IO ()
testFreshClaimLease = withFixture "fresh-lease" $ \registry root -> do
  secondRoot <- prepareWorkspace (root </> "other")
  void (registerServiceJob registry "alpha" root 60 300 now)
  void (registerServiceJob registry "beta" secondRoot 60 30 now)
  current <- newIORef now
  let environment = ServiceEnvironment (Clock (readIORef current) (writeIORef current)) $ \run -> do
        if serviceJobName (serviceRunJob run) == "alpha"
          then writeIORef current (addUTCTime 300 now)
          else do
            recovered <- recoverServiceRuns registry =<< readIORef current
            assert (recovered == 0) "later job inherited an already-expired lease from batch start"
        pure (Right (ServiceResult (serviceRunTimestamp run) 0 Null))
  void (serviceOnceWith environment registry)
  history <- serviceRunHistory registry 10
  assert (map (lookupValue "state") history == replicate 2 (Just (String "succeeded"))) "running second task was recovered prematurely"

testWebDuringTask :: IO ()
testWebDuringTask = withFixture "web" $ \registry root -> do
  void (registerServiceJob registry "alpha" root 60 30 now)
  port <- unusedPort
  started <- newEmptyMVar
  release <- newEmptyMVar
  let environment = ServiceEnvironment (Clock (pure now) (const (pure ()))) $ \run -> do
        putMVar started ()
        takeMVar release
        pure (Right (ServiceResult (serviceRunTimestamp run) 0 Null))
      scheduler = void (serviceOnceWith environment registry)
  withAsync (withServiceWeb registry port scheduler) $ \_ -> do
    ready <- timeout (5 * 1000000) (takeMVar started)
    assert (ready == Just ()) "scheduler did not start behind listener"
    response <- timeout (5 * 1000000) (http port ("127.0.0.1:" <> show port))
    case response of
      Just bytes -> assert ("200 OK" `BS.isInfixOf` bytes && "running" `BS.isInfixOf` bytes) "status did not show running task"
      Nothing -> fail "web status blocked behind running task"
    forbidden <- http port "malicious.example"
    assert ("403 Forbidden" `BS.isInfixOf` forbidden) "foreign Host was accepted"
  jobs <- listServiceJobs registry
  assert (not (any serviceJobEnabled jobs)) "service cancellation failed to pause uncertain invocation"

http :: Int -> String -> IO BS.ByteString
http port host = bracket (socket AF_INET Stream defaultProtocol) close $ \client -> do
  connect client (SockAddrInet (fromIntegral port) (tupleToHostAddress (127, 0, 0, 1)))
  sendAll client (Char8.pack ("GET /api/status HTTP/1.1\r\nHost: " <> host <> "\r\nConnection: close\r\n\r\n"))
  let collect acc = do
        chunk <- recv client 65536
        if BS.null chunk then pure acc else collect (acc <> chunk)
  collect BS.empty

unusedPort :: IO Int
unusedPort = bracket (socket AF_INET Stream defaultProtocol) close $ \listener -> do
  bind listener (SockAddrInet 0 (tupleToHostAddress (127, 0, 0, 1)))
  address <- getSocketName listener
  case address of
    SockAddrInet port _ -> pure (fromIntegral port)
    _ -> fail "expected IPv4 socket"

requireClaim :: FilePath -> UTCTime -> IO ServiceRun
requireClaim registry timestamp = claimServiceRun registry timestamp >>= maybe (fail "expected a due task") pure

lookupValue :: Key -> Value -> Maybe Value
lookupValue key (Object fields) = KeyMap.lookup key fields
lookupValue _ _ = Nothing

isLeft :: Either a b -> Bool
isLeft (Left _) = True
isLeft _ = False

withFixture :: String -> (FilePath -> FilePath -> IO a) -> IO a
withFixture label action = bracket create cleanup $ \root -> action (root </> "registry.sqlite3") root
  where
    create = do
      temporary <- getTemporaryDirectory
      timestamp <- getCurrentTime
      let root = temporary </> ("segno-service-test-" <> label <> "-" <> filter (/= ':') (show timestamp))
      createDirectory root
      prepareWorkspace root
    cleanup root = removeDirectoryRecursive root `catch` (\(_ :: IOException) -> pure ())

prepareWorkspace :: FilePath -> IO FilePath
prepareWorkspace root = do
  createDirectoryIfMissing True (root </> ".tactus")
  writeFile (root </> ".tactus" </> "tactus.toml") ""
  writeFile (root </> ".tactus" </> "segno.hs") "main = pure ()\n"
  pure root
