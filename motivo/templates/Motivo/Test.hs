{-# LANGUAGE CPP #-}
{-# LANGUAGE OverloadedStrings #-}

-- | Command acceptance tests through the enforcing motivo.test effect.
-- The host checks facts, never a provider's claim that it ran tests.
module Motivo.Test
  ( Suite, TestCase, Command, Assertion, SuiteResult,
    suite, testCase, command, withTimeout,
    assertExit, assertStdout, assertStdoutContains, assertStderr, assertStderrContains,
    assertFileExists, assertFileEquals, assertFileContains,
    runSuite, suitePassed, suiteStatus, resultJSON, runTestSuite, runTestSuiteFile
  ) where

import Clef
  ( Workflow, WorkflowError, attempt, liftIO, loadRuntimeConfigFromEnv, operation, perform,
    renderWorkflowError, requireBecause, runWorkflow, withRuntime
  )
import Clef.Plugin.Protocol (decodeStrictJSON)
import Control.Concurrent.MVar (MVar, modifyMVar_, newMVar, readMVar)
import Control.Exception (IOException, try)
import Control.Monad (forM, forM_, unless, when)
import Data.Aeson
  ( FromJSON (parseJSON), Object, ToJSON (toJSON), Value (..),
    encode, object, withObject, (.:), (.:?), (.!=), (.=)
  )
import qualified Data.Aeson.Key as Key
import qualified Data.Aeson.KeyMap as KeyMap
import Data.Aeson.Types (Parser, parseEither)
import qualified Data.ByteString as Bytes
import qualified Data.ByteString.Lazy as LazyBytes
import Data.List (nub)
import Data.Text (Text)
import qualified Data.Text as Text
import qualified Data.Text.Encoding as Encoding
import qualified Data.Text.IO as TextIO
import Motivo.Html (escapeHtml, renderPage)
import Motivo.Record
import System.Directory (canonicalizePath, createDirectory, doesFileExist, getCurrentDirectory)
import System.Environment (getArgs, lookupEnv)
import System.Exit (ExitCode (..), exitWith)
import System.FilePath ((</>), isAbsolute, splitDirectories)
import System.IO (IOMode (AppendMode, ReadMode), hFlush, stderr, withBinaryFile, withFile)
import System.IO.Error (catchIOError, isAlreadyExistsError, isDoesNotExistError)
#ifndef mingw32_HOST_OS
import qualified System.Posix.Files as Posix
#endif

data Suite = Suite Text [TestCase] deriving (Eq, Show)
data TestCase = TestCase Text Command [Assertion] deriving (Eq, Show)
data Command = Command [String] Int deriving (Eq, Show)
data Assertion
  = ExitCodeIs Int | StdoutIs Text | StdoutHas Text | StderrIs Text | StderrHas Text
  | FileExists FilePath | FileIs FilePath Text | FileHas FilePath Text
  deriving (Eq, Show)

suite :: Text -> [TestCase] -> Suite
suite = Suite

testCase :: Text -> Command -> [Assertion] -> TestCase
testCase = TestCase

-- | Literal argv; an explicit shell is required for shell syntax.
command :: [String] -> Command
command argv = Command argv 600

withTimeout :: Int -> Command -> Command
withTimeout seconds (Command argv _) = Command argv seconds

assertExit :: Int -> Assertion
assertExit = ExitCodeIs
assertStdout, assertStdoutContains, assertStderr, assertStderrContains :: Text -> Assertion
assertStdout = StdoutIs
assertStdoutContains = StdoutHas
assertStderr = StderrIs
assertStderrContains = StderrHas
assertFileExists :: FilePath -> Assertion
assertFileExists = FileExists
assertFileEquals, assertFileContains :: FilePath -> Text -> Assertion
assertFileEquals = FileIs
assertFileContains = FileHas

instance ToJSON Suite where
  toJSON (Suite identity cases) = object
    ["api" .= ("motivo.test-suite/v1" :: Text), "id" .= identity, "cases" .= cases]
instance ToJSON TestCase where
  toJSON (TestCase identity action assertions) = object
    ["id" .= identity, "command" .= action, "assertions" .= assertions]
instance ToJSON Command where
  toJSON (Command argv seconds) = object ["argv" .= argv, "timeout_seconds" .= seconds]
instance ToJSON Assertion where
  toJSON assertion = object ("kind" .= assertionKind assertion : fields)
    where fields = case assertion of
            ExitCodeIs value -> ["expected" .= value]
            StdoutIs value -> ["expected" .= value]
            StdoutHas value -> ["expected" .= value]
            StderrIs value -> ["expected" .= value]
            StderrHas value -> ["expected" .= value]
            FileExists path -> ["path" .= path]
            FileIs path value -> ["path" .= path, "expected" .= value]
            FileHas path value -> ["path" .= path, "expected" .= value]

assertionKind :: Assertion -> Text
assertionKind assertion = case assertion of
  ExitCodeIs _ -> "exit_code"
  StdoutIs _ -> "stdout_equals"
  StdoutHas _ -> "stdout_contains"
  StderrIs _ -> "stderr_equals"
  StderrHas _ -> "stderr_contains"
  FileExists _ -> "file_exists"
  FileIs _ _ -> "file_equals"
  FileHas _ _ -> "file_contains"

onlyFields :: [Text] -> Object -> Parser ()
onlyFields allowed values = unless (all ((`elem` allowed) . Key.toText) (KeyMap.keys values))
  (fail "Unknown suite, case, command or assertion field")

instance FromJSON Suite where
  parseJSON = withObject "Motivo suite" $ \values -> do
    onlyFields ["api", "id", "cases"] values
    api <- values .: "api"
    unless (api == ("motivo.test-suite/v1" :: Text)) (fail "Expected motivo.test-suite/v1")
    Suite <$> values .: "id" <*> values .: "cases"
instance FromJSON TestCase where
  parseJSON = withObject "Motivo case" $ \values -> do
    onlyFields ["id", "command", "assertions"] values
    TestCase <$> values .: "id" <*> values .: "command" <*> values .: "assertions"
instance FromJSON Command where
  parseJSON = withObject "Motivo command" $ \values -> do
    onlyFields ["argv", "timeout_seconds"] values
    Command <$> values .: "argv" <*> (values .:? "timeout_seconds" .!= 600)
instance FromJSON Assertion where
  parseJSON = withObject "Motivo assertion" $ \values -> do
    kind <- values .: "kind" :: Parser Text
    case kind of
      "exit_code" -> expected values ExitCodeIs
      "stdout_equals" -> expected values StdoutIs
      "stdout_contains" -> expected values StdoutHas
      "stderr_equals" -> expected values StderrIs
      "stderr_contains" -> expected values StderrHas
      "file_exists" -> onlyFields ["kind", "path"] values >> FileExists <$> values .: "path"
      "file_equals" -> fileExpected values FileIs
      "file_contains" -> fileExpected values FileHas
      _ -> fail "Unknown assertion kind"
    where
      expected values constructor = onlyFields ["kind", "expected"] values >> constructor <$> values .: "expected"
      fileExpected values constructor = do
        onlyFields ["kind", "path", "expected"] values
        constructor <$> values .: "path" <*> values .: "expected"

validateSuite :: Suite -> Either Text ()
validateSuite value@(Suite identity cases) = do
  unless (validToken identity) (Left "Invalid suite id; use 1–80 ASCII letters, digits, underscores or hyphens")
  unless (not (null cases) && length cases <= 64) (Left "A suite needs 1–64 cases")
  let identities = [name | TestCase name _ _ <- cases]
  unless (length (nub identities) == length identities) (Left "Case ids must be unique")
  unless (LazyBytes.length (encode value) <= 512 * 1024) (Left "A normalized suite exceeds 512 KiB")
  forM_ cases $ \(TestCase name (Command argv seconds) assertions) -> do
    unless (validToken name) (Left "Invalid case id")
    let hasProgram = case argv of [] -> False; program : _ -> not (null program)
    unless (hasProgram && length argv <= 256 && all (notElem '\0') argv)
      (Left "A command needs 1–256 literal arguments and a nonempty program, without NUL")
    unless (seconds >= 1 && seconds <= 600) (Left "A case timeout must be 1–600 seconds")
    unless (not (null assertions) && length assertions <= 16) (Left "Each case needs 1–16 explicit assertions")
    forM_ assertions $ \assertion -> case assertion of
      ExitCodeIs number -> unless (number >= 0 && number <= 255) (Left "Expected exit code must be 0–255")
      StdoutIs text -> boundedExpected text
      StdoutHas text -> nonemptyExpected text
      StderrIs text -> boundedExpected text
      StderrHas text -> nonemptyExpected text
      FileExists path -> checkedRelative path
      FileIs path text -> checkedRelative path >> boundedExpected text
      FileHas path text -> checkedRelative path >> nonemptyExpected text
  where
    boundedExpected text = unless (Bytes.length (Encoding.encodeUtf8 text) <= 64 * 1024) (Left "An expected text exceeds 64 KiB")
    nonemptyExpected text = unless (not (Text.null text)) (Left "A contains assertion must have nonempty expected text") >> boundedExpected text

checkedRelative :: FilePath -> Either Text ()
checkedRelative path = unless
  (not (null path) && not (isAbsolute path) && all (`notElem` ['\\', '\0']) path
    && all (`notElem` [".", "..", "", ".motivo-logs"]) (splitDirectories path))
  (Left "An artifact path must be relative to /work, without traversal, links or .motivo-logs")

data CaseResult = CaseResult Text Text (Maybe Text) (Maybe Value) [Value] deriving (Eq, Show)
data SuiteResult = SuiteResult Suite [CaseResult] (Maybe Text) deriving (Eq, Show)

instance ToJSON CaseResult where
  toJSON (CaseResult identity status failure outcome assertions) = object
    ["id" .= identity, "status" .= status, "error" .= failure, "outcome" .= outcome, "assertions" .= assertions]
instance ToJSON SuiteResult where
  toJSON result@(SuiteResult value@(Suite identity _) cases failure) = object
    ["api" .= ("motivo.test-suite-result/v1" :: Text), "suite_id" .= identity,
      "manifest" .= value, "status" .= suiteStatus result, "counts" .= counts result,
      "suite_error" .= failure, "cases" .= cases]

resultJSON :: SuiteResult -> Value
resultJSON = toJSON

suitePassed :: SuiteResult -> Bool
suitePassed result = suiteStatus result == "passed"

suiteStatus :: SuiteResult -> Text
suiteStatus (SuiteResult _ cases failure)
  | failure /= Nothing || any ((== "error") . caseState) cases = "error"
  | any ((== "failed") . caseState) cases = "failed"
  | null cases = "error"
  | otherwise = "passed"

caseState :: CaseResult -> Text
caseState (CaseResult _ status _ _ _) = status

counts :: SuiteResult -> Value
counts (SuiteResult _ cases _) = object
  ["total" .= length cases, "passed" .= count "passed", "failed" .= count "failed", "errors" .= count "error"]
  where count status = length (filter ((== status) . caseState) cases)

-- | Compose real command acceptance with ordinary Clef do-notation. This core
-- returns evidence; the CLI wrapper below adds immutable reports and red/green.
runSuite :: FilePath -> Text -> Suite -> Workflow SuiteResult
runSuite = runSuiteObserved (\_ _ -> pure ())

runSuiteObserved :: (Text -> Value -> IO ()) -> FilePath -> Text -> Suite -> Workflow SuiteResult
runSuiteObserved observer givenRoot runId value@(Suite _ cases) = do
  _ <- requireBecause (either id (const "Invalid suite") (validateSuite value))
    (either (const False) (const True) . validateSuite) value
  _ <- requireBecause "Invalid run id" validToken runId
  root <- liftIO (canonicalizePath givenRoot)
  initialized <- liftIO (doesFileExist (root </> ".tactus/tactus.toml"))
  _ <- requireBecause "runSuite needs an initialized workspace root" id initialized
  results <- forM cases $ \entry@(TestCase identity (Command argv seconds) assertions) -> do
    let params = object ["run_id" .= runId, "sample_id" .= identity, "argv" .= argv, "timeout_seconds" .= seconds]
    liftIO (observer "case.started" (object ["case_id" .= identity, "command" .= entry]))
    attempted <- attempt (perform (operation "motivo.test" "run" params) :: Workflow Value)
    result <- case attempted of
      Left failure -> pure (CaseResult identity "error" (Just (renderWorkflowError failure)) Nothing [])
      Right outcome -> case parseEither (parseFacts runId identity seconds) outcome of
        Left message -> pure (CaseResult identity "error" (Just (Text.pack message)) (Just outcome) [])
        Right facts -> do
          checked <- liftIO (try (checkAssertions root runId identity facts assertions) :: IO (Either IOException [Value]))
          case checked of
            Left failure -> pure (CaseResult identity "error" (Just (Text.pack (show failure))) (Just outcome) [])
            Right observations -> do
              forM_ observations (liftIO . observer "assertion.checked")
              pure (CaseResult identity (if all assertionPassed observations then "passed" else "failed") Nothing (Just outcome) observations)
    liftIO (observer "case.finished" (toJSON result))
    pure result
  pure (SuiteResult value results Nothing)

data Facts = Facts Int Text Text

-- Required facts have exact identities; extra provider-like claims are ignored.
parseFacts :: Text -> Text -> Int -> Value -> Parser Facts
parseFacts runId caseId seconds = withObject "motivo.test facts" $ \values -> do
  status <- values .: "status" :: Parser Text
  unless (status == "exited") (fail "No complete exited result: timeout, cancellation or unknown status is not a red test")
  code <- values .: "exit_code" :: Parser (Maybe Int)
  signal <- values .: "signal" :: Parser (Maybe Int)
  exitCode <- case (code, signal) of
    (Just number, Nothing) | number >= 0 && number <= 255 -> pure number
    _ -> fail "Missing exit code or signal termination is incomplete execution evidence"
  duration <- values .: "duration_ms" :: Parser Integer
  unless (duration >= 0) (fail "Invalid duration")
  recordedSeconds <- values .: "timeout_seconds" :: Parser Int
  unless (recordedSeconds == seconds) (fail "Outcome timeout does not match this case")
  workspace <- values .: "workspace_dir" :: Parser Text
  stdoutPath <- values .: "stdout_path" :: Parser Text
  stderrPath <- values .: "stderr_path" :: Parser Text
  let expected = ".tactus/motivotest/" <> runId <> "/" <> caseId
  unless (workspace == expected && stdoutPath == expected <> "/.motivo-logs/stdout.log"
    && stderrPath == expected <> "/.motivo-logs/stderr.log") (fail "Outcome paths do not identify this case's protected logs")
  partial <- values .: "logs_may_be_partial" :: Parser Bool
  stdoutTruncated <- values .: "stdout_truncated" :: Parser Bool
  stderrTruncated <- values .: "stderr_truncated" :: Parser Bool
  when (partial || stdoutTruncated || stderrTruncated) (fail "Partial or truncated logs cannot establish a red or green result")
  pure (Facts exitCode stdoutPath stderrPath)

checkAssertions :: FilePath -> Text -> Text -> Facts -> [Assertion] -> IO [Value]
checkAssertions root runId identity (Facts code stdoutPath stderrPath) assertions = do
  stdoutText <- requiredText root (Text.unpack stdoutPath)
  stderrText <- requiredText root (Text.unpack stderrPath)
  forM assertions $ \assertion -> do
    let observation actual passed = object
          ["case_id" .= identity, "assertion" .= assertion,
            "status" .= (if passed then "passed" else "failed" :: Text), "observed" .= actual]
        textObservation expected actual predicate = observation
          (object ["text" .= Text.take 4096 actual, "display_truncated" .= (Text.length actual > 4096)]) (predicate expected actual)
        artifact path = optionalText root (".tactus/motivotest/" ++ Text.unpack runId ++ "/" ++ Text.unpack identity </> path)
    case assertion of
      ExitCodeIs expected -> pure (observation (toJSON code) (code == expected))
      StdoutIs expected -> pure (textObservation expected stdoutText (==))
      StdoutHas expected -> pure (textObservation expected stdoutText Text.isInfixOf)
      StderrIs expected -> pure (textObservation expected stderrText (==))
      StderrHas expected -> pure (textObservation expected stderrText Text.isInfixOf)
      FileExists path -> do
        present <- evidenceFile root (".tactus/motivotest/" ++ Text.unpack runId ++ "/" ++ Text.unpack identity </> path)
        pure (observation (toJSON present) present)
      FileIs path expected -> do
        actual <- artifact path
        pure $ maybe (observation Null False) (\text -> textObservation expected text (==)) actual
      FileHas path expected -> do
        actual <- artifact path
        pure $ maybe (observation Null False) (\text -> textObservation expected text Text.isInfixOf) actual

assertionPassed :: Value -> Bool
assertionPassed (Object values) = KeyMap.lookup "status" values == Just (String "passed")
assertionPassed _ = False

-- Never open a FIFO/device, follow a link, or read outside this sample.
evidenceFile :: FilePath -> FilePath -> IO Bool
evidenceFile root relative = do
  when (isAbsolute relative || any (`elem` ["..", "."]) (splitDirectories relative)) (fail "Unsafe evidence path")
  forM_ (drop 1 (scanl (</>) root (splitDirectories relative))) $ \path -> do
    linked <- isLinked path
    when linked (fail ("Evidence is a symbolic link: " ++ path))
#ifdef mingw32_HOST_OS
  fail "Motivo test evidence verification currently requires Linux"
#else
  status <- (Just <$> Posix.getSymbolicLinkStatus (root </> relative)) `catchIOError` \failure ->
    if isDoesNotExistError failure then pure Nothing else ioError failure
  case status of
    Nothing -> pure False
    Just facts -> do
      unless (Posix.isRegularFile facts && Posix.linkCount facts == 1) (fail "Evidence must be a regular file with one hard link")
      pure True
#endif

optionalText :: FilePath -> FilePath -> IO (Maybe Text)
optionalText root relative = do
  exists <- evidenceFile root relative
  if not exists then pure Nothing else do
    bytes <- withBinaryFile (root </> relative) ReadMode (\handle -> Bytes.hGet handle (1024 * 1024 + 1))
    when (Bytes.length bytes > 1024 * 1024) (fail "Evidence text exceeds 1 MiB")
    Just <$> either (fail . show) pure (Encoding.decodeUtf8' bytes)

requiredText :: FilePath -> FilePath -> IO Text
requiredText root relative = optionalText root relative >>= maybe (fail "A protected log is missing") pure

data Options = Options
  { optionSuite :: Maybe FilePath, optionRunId :: Maybe Text, optionPhase :: Text,
    optionBaseline :: Maybe Text, optionAgent :: Text, optionModel :: Text
  }

emptyOptions :: Options
emptyOptions = Options Nothing Nothing "normal" Nothing "not reported" "not reported"

runTestSuite :: Suite -> IO ()
runTestSuite = runEntry . Just

runTestSuiteFile :: IO ()
runTestSuiteFile = runEntry Nothing

runEntry :: Maybe Suite -> IO ()
runEntry authored = do
  arguments <- getArgs
  if arguments == ["--help"] then TextIO.putStrLn usage else do
    options <- either usageError pure (parseOptions emptyOptions arguments)
    value <- case (authored, optionSuite options) of
      (Just provided, Nothing) -> pure provided
      (Nothing, Just path) -> do
        loaded <- try (readBoundedText path) :: IO (Either IOException Text)
        input <- either (usageError . show) pure loaded
        either usageError pure (decodeStrictJSON (Encoding.encodeUtf8 input))
      (Just _, Just _) -> usageError "A Haskell-authored suite does not accept --suite"
      _ -> usageError "--suite FILE is required"
    either (usageError . Text.unpack) pure (validateSuite value)
    when (optionPhase options == "green" && optionBaseline options == Nothing) (usageError "--phase green requires --baseline RED_RUN_ID")
    when (optionPhase options /= "green" && optionBaseline options /= Nothing) (usageError "Only --phase green accepts --baseline")
    root <- getCurrentDirectory >>= findWorkspace >>= canonicalizePath
    ensureDirectory root [".tactus", "motivo", "runs"]
    template <- readBoundedText (root </> ".tactus/skills/motivo/assets/report-template.html")
    _ <- either usageError pure (renderPage template "")
    runId <- maybe newRunId pure (optionRunId options)
    let directory = root </> ".tactus/motivo/runs" </> Text.unpack runId
    createDirectory directory `catchIOError` \failure ->
      if isAlreadyExistsError failure then usageError "This Motivo run already exists; preserve it and choose a new --run-id" else ioError failure
    createDirectory (directory </> "artifacts")
    started <- nowText
    tactusRun <- lookupEnv "TACTUS_RUN_ID"
    sequenceNumber <- newMVar (0 :: Int)
    let report matched result = publish root directory runId started tactusRun template options matched result sequenceNumber
        event :: Text -> Value -> IO ()
        event kind details = do
          at <- nowText
          modifyMVar_ sequenceNumber $ \number -> do
            let sample = object ["api" .= ("motivo.sample/v1" :: Text), "run_id" .= runId,
                  "seq" .= (number + 1), "at" .= at, "kind" .= kind,
                  "summary" .= kind, "details" .= details]
            withFile (directory </> "samples.jsonl") AppendMode $ \handle -> LazyBytes.hPut handle (encode sample <> "\n") >> hFlush handle
            pure (number + 1)
        observer = event
    atomicWriteText (directory </> "samples.jsonl") ""
    atomicWriteJson (directory </> "artifacts/suite.json") (toJSON value)
    atomicWriteText (directory </> "request.md") ("# Test suite\n\nPhase: " <> optionPhase options <> "\n\n```json\n" <> jsonText (toJSON value) <> "\n```\n")
    event "suite.started" (object ["phase" .= optionPhase options, "manifest" .= value])
    report Nothing (SuiteResult value [] (Just "Execution has not produced a terminal suite result"))
    baseline <- checkBaseline root options value
    let matched = case optionBaseline options of
          Nothing -> Nothing
          Just _ -> Just (either (const False) (const True) baseline)
    result <- case baseline of
      Left failure -> pure (SuiteResult value [] (Just failure))
      Right () -> do
        -- Catch only expected IO/configuration failures, never cancellation.
        executed <- try (try (do
          config <- loadRuntimeConfigFromEnv
          withRuntime config $ \runtime -> runWorkflow runtime (runSuiteObserved observer root runId value)) :: IO (Either WorkflowError SuiteResult)) :: IO (Either IOException (Either WorkflowError SuiteResult))
        case executed of
          Left failure -> pure (SuiteResult value [] (Just (Text.pack (show failure))))
          Right (Left failure) -> pure (SuiteResult value [] (Just (renderWorkflowError failure)))
          Right (Right actual) -> pure actual
    let terminal = if optionPhase options == "red" && suitePassed result
          then case result of SuiteResult spec cases _ -> SuiteResult spec cases (Just "Red was not observed: every case passed")
          else result
    event "suite.finished" (resultJSON terminal)
    report matched terminal
    TextIO.putStrLn ("Motivo test suite: " <> suiteStatus terminal <> " · phase " <> optionPhase options)
    TextIO.putStrLn ("Motivo report: " <> Text.pack (directory </> "report.html"))
    TextIO.putStrLn ("Workspace overview: " <> Text.pack (root </> ".tactus/motivo/index.html"))
    exitWith $ case suiteStatus terminal of
      "passed" -> ExitSuccess
      "failed" -> ExitFailure 1
      _ -> ExitFailure 2

usageError :: String -> IO value
usageError message = TextIO.hPutStrLn stderr (Text.pack message) >> exitWith (ExitFailure 2)

usage :: Text
usage = Text.unlines
  ["Motivo command acceptance suite (no model calls).",
    "090_test.hs --suite FILE [--run-id TOKEN] [--phase normal|red|green]",
    "[--baseline RED_RUN_ID] [--agent NAME] [--model NAME]",
    "Haskell runTestSuite uses the same flags except --suite.",
    "Exit 0: passed; 1: actual assertion failure (including observed red); 2: unusable evidence or invalid suite/phase.",
    "Green requires the exact normalized command/assertion manifest recorded by a valid red run."]

parseOptions :: Options -> [String] -> Either String Options
parseOptions options [] = Right options
parseOptions options ("--suite" : path : rest) = parseOptions options {optionSuite = Just path} rest
parseOptions options ("--run-id" : value : rest) = token value >>= \identity -> parseOptions options {optionRunId = Just identity} rest
parseOptions options ("--baseline" : value : rest) = token value >>= \identity -> parseOptions options {optionBaseline = Just identity} rest
parseOptions options ("--phase" : value : rest)
  | value `elem` ["normal", "red", "green"] = parseOptions options {optionPhase = Text.pack value} rest
  | otherwise = Left "--phase must be normal, red or green"
parseOptions options ("--agent" : value : rest) = label value >>= \name -> parseOptions options {optionAgent = name} rest
parseOptions options ("--model" : value : rest) = label value >>= \name -> parseOptions options {optionModel = name} rest
parseOptions _ (argument : _) = Left ("Unknown or incomplete argument: " ++ argument)

token :: String -> Either String Text
token value = if validToken (Text.pack value) then Right (Text.pack value) else Left "Invalid identity"

label :: String -> Either String Text
label value = if null value || length value > 200 || any (`elem` ['\0', '\r', '\n']) value
  then Left "Identity labels need one nonempty line of at most 200 characters" else Right (Text.pack value)

checkBaseline :: FilePath -> Options -> Suite -> IO (Either Text ())
checkBaseline root options value = case optionBaseline options of
  Nothing -> pure (Right ())
  Just identity -> do
    let relative = ".tactus/motivo/runs/" ++ Text.unpack identity ++ "/run.json"
    loaded <- try (requiredText root relative) :: IO (Either IOException Text)
    pure $ case loaded of
      Left failure -> Left ("Cannot read red baseline: " <> Text.pack (show failure))
      Right content -> case decodeStrictJSON (Encoding.encodeUtf8 content) of
        Left failure -> Left ("Invalid red baseline JSON: " <> Text.pack failure)
        Right metadata -> case parseEither (baselineParser identity) metadata of
          Left failure -> Left (Text.pack failure)
          Right manifest -> if manifest == toJSON value then Right () else Left "Oracle changed: green commands, timeouts, case ids and assertions must match the red manifest"
  where
    baselineParser identity = withObject "Motivo red baseline" $ \metadata -> do
      api <- metadata .: "api" :: Parser Text
      method <- metadata .: "method" :: Parser Text
      runIdentity <- metadata .: "run_id" :: Parser Text
      status <- metadata .: "status" :: Parser Text
      unless (api == "motivo.run/v1" && method == "test" && runIdentity == identity && status == "test-failed") (fail "Baseline must be a failed Motivo test run with the requested identity")
      details <- metadata .: "test_suite"
      withObject "Red suite evidence" (\fields -> do
        phase <- fields .: "phase" :: Parser Text
        observed <- fields .: "red_observed" :: Parser Bool
        failure <- fields .: "suite_error" :: Parser (Maybe Text)
        recordedCounts <- fields .: "counts"
        withObject "Red case counts" (\numbers -> do
          failed <- numbers .: "failed" :: Parser Int
          errors <- numbers .: "errors" :: Parser Int
          unless (phase == "red" && observed && failed > 0 && errors == 0 && failure == Nothing)
            (fail "Baseline did not establish an assertion failure without execution/evidence errors")) recordedCounts
        fields .: "manifest") details

publish :: FilePath -> FilePath -> Text -> Text -> Maybe String -> Text -> Options -> Maybe Bool -> SuiteResult -> MVar Int -> IO ()
publish root directory runId started tactusRun template options matched result@(SuiteResult value@(Suite identity _) cases failure) sequenceNumber = do
  generated <- nowText
  count <- readMVar sequenceNumber
  let observedRed = optionPhase options == "red" && suiteStatus result == "failed"
      status = case suiteStatus result of "passed" -> "test-passed"; "failed" -> "test-failed"; _ -> "needs-check"
      details = object ["api" .= ("motivo.test-suite-result/v1" :: Text), "suite_id" .= identity,
        "phase" .= optionPhase options, "baseline_run_id" .= optionBaseline options,
        "oracle_matched" .= matched,
        "manifest" .= value, "counts" .= counts result, "suite_error" .= failure, "red_observed" .= observedRed]
      artifact = case resultJSON result of Object fields -> Object (KeyMap.insert "phase_evidence" details fields); other -> other
      metadata = object ["api" .= ("motivo.run/v1" :: Text), "run_id" .= runId, "method" .= ("test" :: Text),
        "title" .= ("Test suite · " <> identity), "started_at" .= started, "generated_at" .= generated,
        "status" .= status, "parent_run_id" .= optionBaseline options,
        "agent" .= optionAgent options, "model" .= optionModel options,
        "identity_source" .= ("caller-reported; not inferred from a default provider" :: Text),
        "requested_provider" .= Null, "requested_model" .= Null, "actual_provider_model" .= Null,
        "tactus_run_id" .= tactusRun, "sample_count" .= count, "input_path" .= optionSuite options,
        "request_path" .= Null, "missing_sections" .= ([] :: [Text]),
        "artifacts" .= (["artifacts/suite.json", "artifacts/test-suite.json"] :: [Text]),
        "experiment" .= Null, "provider_error" .= Null, "test_suite" .= details]
      body = "<header class=\"motivo-header\"><a href=\"../../index.html\">← All runs</a><p class=\"eyebrow\">MOTIVO · COMMAND ACCEPTANCE</p><h1>"
        <> escapeHtml identity <> "</h1><p class=\"status\">" <> escapeHtml status <> " · phase " <> escapeHtml (optionPhase options)
        <> "</p><p>Run " <> escapeHtml runId <> " · generated " <> escapeHtml generated <> "</p></header><main>"
        <> "<aside class=\"note\">The host checked the declared command assertions. Provider claims are not test evidence. A passing suite covers these assertions; it does not certify every part of the user's task. Interrupted or incomplete execution cannot establish red or green.</aside>"
        <> (if observedRed then "<p><strong>Red observed:</strong> at least one assertion failed, with no execution or evidence errors.</p>" else "")
        <> maybe "" (\message -> "<section><h2>Suite error</h2><pre>" <> escapeHtml message <> "</pre></section>") failure
        <> "<nav class=\"report-links\"><a href=\"artifacts/suite.json\">Frozen suite manifest</a><a href=\"artifacts/test-suite.json\">Full case evidence</a></nav>"
        <> maybe "" (\identity' -> "<p>Red baseline: <a href=\"../" <> identity' <> "/report.html\">" <> identity' <> "</a></p>") (optionBaseline options)
        <> Text.concat (map caseHtml cases) <> "</main>"
      caseHtml (CaseResult caseId caseStatus caseFailure outcome assertions) =
        "<section><h2>" <> escapeHtml caseId <> " · " <> escapeHtml caseStatus <> "</h2>"
        <> maybe "" (\message -> "<pre>" <> escapeHtml message <> "</pre>") caseFailure
        <> (if outcome == Nothing then "" else "<nav class=\"report-links\"><a href=\"../../../motivotest/" <> runId <> "/" <> caseId <> "/.motivo-logs/stdout.log\">stdout</a><a href=\"../../../motivotest/" <> runId <> "/" <> caseId <> "/.motivo-logs/stderr.log\">stderr</a></nav>")
        <> "<details open><summary>Assertions and observed facts</summary><pre>" <> escapeHtml (jsonText (toJSON assertions)) <> "</pre></details></section>"
  page <- either fail pure (renderPage template body)
  atomicWriteJson (directory </> "artifacts/test-suite.json") artifact
  atomicWriteJson (directory </> "run.json") metadata
  atomicWriteText (directory </> "report.html") page
  atomicWriteText (directory </> "report.md") ("# Test suite · " <> identity <> "\n\nState: " <> status <> " · phase " <> optionPhase options <> "\n\n```json\n" <> jsonText artifact <> "\n```\n")
  refreshWorkspaceIndex root template generated
