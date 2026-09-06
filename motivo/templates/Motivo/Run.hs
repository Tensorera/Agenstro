{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE ScopedTypeVariables #-}

-- | File-backed method reports for a coding agent. No model selection or loop.
module Motivo.Run (runMethod, parseSections, validateNotes) where

import Clef
  ( EventSink (..), operation, perform, renderRuntimeRecord, runWorkflow,
    withRuntimeWithSink, loadRuntimeConfigFromEnv, invokeWith, providerRef,
    ProviderRef (..), textTask, writeRuntimePresentation
  )
import Control.Concurrent.MVar (MVar, modifyMVar, newMVar, readMVar)
import Control.Exception (IOException, SomeException, bracketOnError, displayException, onException, throwIO, try)
import Control.Monad (filterM, forM, forM_, unless, when)
import Data.Aeson (Value (..), eitherDecodeStrict', encode, object, (.=))
import qualified Data.Aeson.Key as Key
import qualified Data.Aeson.KeyMap as KeyMap
import qualified Data.ByteString as Bytes
import qualified Data.ByteString.Lazy as LazyBytes
import Data.Char (isAscii, isAlphaNum)
import Data.List (sortOn)
import Data.Maybe (fromMaybe)
import Data.Ord (Down (..))
import Data.Text (Text)
import qualified Data.Text as Text
import qualified Data.Text.Encoding as Encoding
import qualified Data.Text.IO as TextIO
import Data.Time (UTCTime, defaultTimeLocale, formatTime, getCurrentTime, parseTimeM)
import Motivo.Html (escapeHtml, renderMarkdown, renderPage)
import Motivo.Method
import System.Directory
  ( canonicalizePath, createDirectory, createDirectoryIfMissing,
    doesFileExist, getCurrentDirectory, listDirectory, pathIsSymbolicLink,
    removeFile, renameFile
  )
import System.Environment (getArgs, lookupEnv)
import System.FilePath ((</>), takeDirectory, splitDirectories)
import System.IO (IOMode (AppendMode, ReadMode), hClose, hFlush, hSetEncoding, hSetNewlineMode, noNewlineTranslation, openTempFile, utf8, withBinaryFile, withFile)
import System.IO.Error (catchIOError, isAlreadyExistsError, isDoesNotExistError)
import Text.Read (readMaybe)

data Options = Options
  { inputPath :: FilePath, requestPath :: Maybe FilePath, requestedRunId :: Maybe Text,
    parentRunId :: Maybe Text, agentName :: Text, modelName :: Text,
    sampleId :: Text, timeoutSeconds :: Int, probeArgv :: [String], selectedProvider :: Maybe Text
  }

emptyOptions :: Options
emptyOptions = Options "" Nothing Nothing Nothing "not reported" "not reported" "sample-001" 600 [] Nothing

data Recorder = Recorder
  { recordRoot :: FilePath, recordDirectory :: FilePath, recordRunId :: Text,
    recordMethod :: Method, recordOptions :: Options, recordNotes :: Text,
    recordStartedAt :: Text, recordTactusRunId :: Maybe String,
    recordTemplate :: Text, recordSamples :: MVar (Int, [Value])
  }

-- | Method entrypoints accept data authored in the user's existing agent session.
-- Probe invokes only an effect. Other methods call a provider only if --provider is explicit.
runMethod :: Method -> IO ()
runMethod method = do
  arguments <- getArgs
  if arguments == ["--help"] then TextIO.putStrLn (usage method) else do
    options <- either fail pure (parseOptions emptyOptions arguments)
    when (null (inputPath options)) (fail "--input FILE is required; use --help")
    when (method /= Probe && not (null (probeArgv options))) (fail "Only probe accepts an experiment command after --")
    when (method == Probe && null (probeArgv options)) (fail "Probe requires an explicit command after --")
    when (method == Probe && selectedProvider options /= Nothing) (fail "Probe does not accept --provider; prepare its hypothesis in the calling agent")
    notes <- readBoundedText (inputPath options)
    either fail pure (validateNotes method notes)
    request <- maybe (pure notes) readBoundedText (requestPath options)
    root <- getCurrentDirectory >>= findWorkspace >>= canonicalizePath
    ensureDirectory root [".tactus", "motivo", "runs"]
    template <- readBoundedText (root </> ".tactus/skills/motivo/assets/report-template.html")
    _ <- either fail pure (renderPage template "")
    runId <- maybe newRunId pure (requestedRunId options)
    forM_ (parentRunId options) (\value -> unless (validToken value) (fail "Invalid parent run id"))
    unless (validToken (sampleId options)) (fail "Invalid sample id")
    let directory = root </> ".tactus/motivo/runs" </> Text.unpack runId
    createDirectory directory `catchIOError` \errorValue ->
      if isAlreadyExistsError errorValue
      then fail "This Motivo run already exists. Keep it immutable and use a new --run-id."
      else ioError errorValue
    createDirectory (directory </> "artifacts")
    started <- nowText
    tactusRunId <- lookupEnv "TACTUS_RUN_ID"
    samples <- newMVar (0, [])
    let recorder = Recorder root directory runId method options notes started tactusRunId template samples
    atomicWriteText (directory </> "request.md") request
    atomicWriteText (directory </> "artifacts/submitted-notes.md") notes
    atomicWriteText (directory </> "samples.jsonl") ""
    writeMethodArtifacts recorder
    appendSample recorder "method.started" "The calling agent's method notes were recorded; this is not a claim that the user's task is complete." Nothing
    publishReport recorder (if method == Probe || selectedProvider options /= Nothing then "running" else "recorded") Nothing
    if method == Probe then do
      appendSample recorder "experiment.started" "Starting one bounded experiment through motivo.test." (Just (probeParams recorder))
      outcome <- try (runProbe recorder) :: IO (Either SomeException Value)
      case outcome of
        Right result -> do
          atomicWriteJson (directory </> "artifacts/experiment.json") result
          let status = probeStatus result
          appendSample recorder "experiment.finished" ("Experiment state: " <> status <> ". A nonzero exit is an observation, not an automatic retry request.") (Just result)
          publishReport recorder status (Just result)
        Left exception -> failedInvocation recorder "experiment" exception
    else case selectedProvider options of
      Nothing -> do
        appendSample recorder "method.recorded" "Method notes were recorded without calling a model. Missing sections remain unknown." Nothing
        publishReport recorder "recorded" Nothing
      Just provider -> do
        appendSample recorder "provider.started" ("A separate context was explicitly requested from provider " <> provider <> ".") Nothing
        outcome <- try (runProvider recorder provider) :: IO (Either SomeException Text)
        case outcome of
          Right response -> do
            atomicWriteText (directory </> "artifacts/provider-response.md") response
            let displayed = Text.take (512 * 1024) response
                completed = recorder {recordNotes = displayed}
            writeMethodArtifacts completed
            appendSample completed "provider.responded" "The explicitly selected provider returned. Its response is preserved without an automatic formatting retry." Nothing
            publishReport completed "responded" Nothing
          Left exception -> failedInvocation recorder "provider" exception
    TextIO.putStrLn ("Motivo report: " <> Text.pack (directory </> "report.html"))
    TextIO.putStrLn ("Workspace overview: " <> Text.pack (root </> ".tactus/motivo/index.html"))

usage :: Method -> Text
usage method = Text.unlines
  [ "Motivo " <> methodSlug method <> " — record a method chosen by your coding agent.",
    "--input FILE [--request FILE] [--run-id TOKEN] [--parent-run-id TOKEN]",
    "[--agent NAME] [--provider CONFIGURED_NAME] [--model NAME]",
    if method == Probe then "[--sample-id TOKEN] [--timeout-seconds 1..600] -- PROGRAM ARG..." else "",
    "Suggested Markdown ## headings: " <> Text.intercalate ", " (methodSections method ++ ["Local reflection"]),
    "Suggested local reflection questions (missing answers remain unknown):",
    Text.unlines (map ("- " <>) (reflectionQuestions method)),
    "Agent/model are caller-reported metadata; omitted values remain unknown."
  ]

parseOptions :: Options -> [String] -> Either String Options
parseOptions options [] = Right options
parseOptions options ("--" : rest) = Right options {probeArgv = rest}
parseOptions options ("--input" : value : rest) = parseOptions options {inputPath = value} rest
parseOptions options ("--request" : value : rest) = parseOptions options {requestPath = Just value} rest
parseOptions options ("--run-id" : value : rest) = checkedToken "run id" value >>= \token -> parseOptions options {requestedRunId = Just token} rest
parseOptions options ("--parent-run-id" : value : rest) = checkedToken "parent run id" value >>= \token -> parseOptions options {parentRunId = Just token} rest
parseOptions options ("--provider" : value : rest) = checkedLabel value >>= \label -> parseOptions options {selectedProvider = Just label} rest
parseOptions options ("--agent" : value : rest) = checkedLabel value >>= \label -> parseOptions options {agentName = label} rest
parseOptions options ("--model" : value : rest) = checkedLabel value >>= \label -> parseOptions options {modelName = label} rest
parseOptions options ("--sample-id" : value : rest) = checkedToken "sample id" value >>= \token -> parseOptions options {sampleId = token} rest
parseOptions options ("--timeout-seconds" : value : rest) = case readMaybe value of
  Just seconds | seconds >= 1 && seconds <= 600 -> parseOptions options {timeoutSeconds = seconds} rest
  _ -> Left "--timeout-seconds must be an integer from 1 through 600; zero does not disable it"
parseOptions _ (argument : _) = Left ("Unknown or incomplete argument: " ++ argument)

checkedLabel :: String -> Either String Text
checkedLabel value = if null value || length value > 200 || any (`elem` ['\r', '\n', '\0']) value
  then Left "Agent/model labels must be one nonempty line of at most 200 characters"
  else Right (Text.pack value)

checkedToken :: String -> String -> Either String Text
checkedToken label value = let token = Text.pack value in if validToken token
  then Right token else Left ("Invalid " ++ label ++ ": use 1–80 ASCII letters, digits, underscores or hyphens")

validToken :: Text -> Bool
validToken value = not (Text.null value) && Text.length value <= 80 && Text.all valid value
  where valid character = isAscii character && (isAlphaNum character || character == '_' || character == '-')

-- | Markdown H2 fields. Duplicate headings are rejected by validateNotes.
parseSections :: Text -> [(Text, Text)]
parseSections = finish . foldl step (Nothing, [], [], False) . Text.lines
  where
    step (current, body, finished, inCode) line
      | "```" `Text.isPrefixOf` Text.stripStart line = (current, line : body, finished, not inCode)
      | not inCode && "## " `Text.isPrefixOf` line =
          (Just (Text.strip (Text.drop 3 line)), [], flush current body finished, inCode)
      | otherwise = (current, line : body, finished, inCode)
    flush Nothing _ finished = finished
    flush (Just heading) body finished = finished ++ [(heading, Text.strip (Text.unlines (reverse body)))]
    finish (current, body, finished, _) = flush current body finished

lookupSection :: Text -> [(Text, Text)] -> Maybe Text
lookupSection heading = lookup (Text.toCaseFold heading) . map (\(name, value) -> (Text.toCaseFold name, value))

validateNotes :: Method -> Text -> Either String ()
validateNotes _ notes = do
  when (Text.null (Text.strip notes)) (Left "Motivo notes must not be empty")
  let names = map (Text.toCaseFold . fst) (parseSections notes)
      duplicates = [name | (index, name) <- zip [0 :: Int ..] names, name `elem` take index names]
  unless (null duplicates) (Left ("Duplicate Markdown headings are ambiguous: " ++ Text.unpack (Text.intercalate ", " duplicates)))

missingSections :: Recorder -> [Text]
missingSections recorder =
  [heading | heading <- methodSections (recordMethod recorder) ++ ["Local reflection"],
    maybe True (Text.null . Text.strip) (lookupSection heading (parseSections (recordNotes recorder)))]

-- A retrospective has concrete review/handoff/history artifacts. History entries
-- preserve source headings; no chronological or causal ordering is invented.
writeMethodArtifacts :: Recorder -> IO ()
writeMethodArtifacts recorder = when (recordMethod recorder == Retrospect) $ do
  let directory = recordDirectory recorder </> "artifacts"
      sections = parseSections (recordNotes recorder)
      section heading = fromMaybe "Unknown: not supplied in the recorded materials." (lookupSection heading sections)
      headings = ["Expected", "Observed", "Causes", "Keep", "Change", "Next step"]
  atomicWriteText (directory </> "retrospective.md") (recordNotes recorder)
  atomicWriteText (directory </> "handoff.md") (Text.unlines
    ["# Handoff from retrospective", "## Current observation", section "Observed",
      "## Evidence", section "Evidence", "## Next step", section "Next step",
      "## Limits", "This file extracts submitted review material. It does not restore a session, prove a cause, or certify completion."])
  atomicWriteJson (directory </> "decision-history.json") (object
    ["api" .= ("motivo.decision-history/v1" :: Text), "run_id" .= recordRunId recorder,
      "temporal_order_known" .= False,
      "entries" .= [object ["source_heading" .= heading, "text" .= section heading, "decided_at" .= Null] | heading <- headings]])

failedInvocation :: Recorder -> Text -> SomeException -> IO ()
failedInvocation recorder kind exception = do
  let failure = object ["error" .= Text.take 4000 (Text.pack (displayException exception)), "result_known" .= False]
  atomicWriteJson (recordDirectory recorder </> "artifacts" </> Text.unpack kind ++ "-error.json") failure
  appendSample recorder (kind <> ".interrupted") "Execution did not produce a usable terminal result. Inspect recorded and external outcomes before deciding whether anything can be retried." (Just failure)
  publishReport recorder "needs-check" (Just failure)
  throwIO exception

runProvider :: Recorder -> Text -> IO Text
runProvider recorder provider = do
  config <- loadRuntimeConfigFromEnv
  let options = recordOptions recorder
      reference = (providerRef provider) {providerRefModel = if modelName options == "not reported" then Nothing else Just (modelName options)}
      method = recordMethod recorder
      prompt notes = Text.unlines
        ["Use one focused Motivo method: " <> methodSlug method <> ".",
          methodPurpose method,
          "Investigate and reason within the supplied scope. Read relevant sources and cite concrete locations. Do not modify business files or perform experiments; experiments belong to the enforcing motivo.test effect invoked separately by the main agent.",
          "Separate observations, interpretations, and unknowns. Do not invent evidence, dates, model identity, or successful checks. Do not create a task loop or delegate additional agents.",
          "Useful report headings, when applicable: " <> Text.intercalate ", " (methodSections method),
          "Brief local reflection should address: " <> Text.intercalate " / " (reflectionQuestions method),
          "Return readable Markdown. These headings are guidance, not a schema to repair through retries.",
          "Submitted task and material:", notes]
  withRuntimeWithSink config (progressSink recorder) $ \runtime ->
    runWorkflow runtime (invokeWith reference (textTask ("motivo." <> methodSlug method) prompt) (recordNotes recorder))

progressSink :: Recorder -> EventSink
progressSink recorder = EventSink $ \record -> do
  -- Keep Tactus's existing sidecar import intact when using a custom sink.
  -- The full runtime record stays in runtime diagnostics, not the HTML page.
  diagnosticPath <- lookupEnv "TACTUS_DIAGNOSTIC_PATH"
  forM_ diagnosticPath $ \path -> do
    persisted <- try (LazyBytes.appendFile path (encode record <> "\n")) :: IO (Either IOException ())
    case persisted of
      Right () -> pure ()
      Left _ -> appendSample recorder "observation.degraded" "Tactus diagnostics could not be appended; the Motivo observation remains available." Nothing
  case renderRuntimeRecord record of
    Nothing -> pure ()
    Just line -> do
      writeRuntimePresentation line
      appendSample recorder "runtime.progress" (Text.take 4000 line) Nothing
      publishReport recorder "running" Nothing

probeParams :: Recorder -> Value
probeParams recorder = object
  [ "run_id" .= recordRunId recorder, "sample_id" .= sampleId options,
    "argv" .= probeArgv options, "timeout_seconds" .= timeoutSeconds options
  ]
  where options = recordOptions recorder

runProbe :: Recorder -> IO Value
runProbe recorder = do
  config <- loadRuntimeConfigFromEnv
  withRuntimeWithSink config (progressSink recorder) $ \runtime ->
    runWorkflow runtime (perform (operation "motivo.test" "run" (probeParams recorder)))

probeStatus :: Value -> Text
probeStatus (Object result) = case KeyMap.lookup "status" result of
  Just (String "timed_out") -> "timed-out"
  Just (String "cancelled") -> "cancelled"
  Just (String "exited") -> case KeyMap.lookup "exit_code" result of
    Just (Number 0) -> "experiment-passed"
    Just (Number _) -> "experiment-failed"
    _ -> "needs-check"
  _ -> "needs-check"
probeStatus _ = "needs-check"

appendSample :: Recorder -> Text -> Text -> Maybe Value -> IO ()
appendSample recorder kind summary details = modifyMVar (recordSamples recorder) $ \(count, recent) -> do
  at <- nowText
  let sample = object
        [ "api" .= ("motivo.sample/v1" :: Text), "run_id" .= recordRunId recorder,
          "seq" .= (count + 1), "at" .= at, "kind" .= kind, "summary" .= summary,
          "sample_id" .= (if recordMethod recorder == Probe then Just (sampleId (recordOptions recorder)) else Nothing),
          "details" .= details
        ]
      next = drop (max 0 (length recent + 1 - 200)) (recent ++ [sample])
  withFile (recordDirectory recorder </> "samples.jsonl") AppendMode $ \handle -> do
    LazyBytes.hPut handle (encode sample)
    Bytes.hPut handle "\n"
    hFlush handle
  pure ((count + 1, next), ())

publishReport :: Recorder -> Text -> Maybe Value -> IO ()
publishReport recorder status result = do
  generated <- nowText
  (count, samples) <- readMVar (recordSamples recorder)
  artifacts <- filterM (doesFileExist . (recordDirectory recorder </>) . Text.unpack)
    (["artifacts/submitted-notes.md", "artifacts/retrospective.md", "artifacts/handoff.md",
      "artifacts/decision-history.json", "artifacts/provider-response.md", "artifacts/provider-error.json",
      "artifacts/experiment.json", "artifacts/experiment-error.json"] :: [Text])
  let method = recordMethod recorder
      options = recordOptions recorder
      metadata = object
        [ "api" .= ("motivo.run/v1" :: Text), "run_id" .= recordRunId recorder,
          "method" .= methodSlug method, "title" .= methodTitle method,
          "started_at" .= recordStartedAt recorder, "generated_at" .= generated,
          "status" .= status, "parent_run_id" .= parentRunId options,
          "agent" .= agentName options, "model" .= displayedModel options,
          "requested_provider" .= selectedProvider options, "requested_model" .= (if selectedProvider options == Nothing || modelName options == "not reported" then Nothing else Just (modelName options)),
          "actual_provider_model" .= Null, "missing_sections" .= missingSections recorder,
          "identity_source" .= ("caller-reported; not inferred from a default provider" :: Text),
          "tactus_run_id" .= recordTactusRunId recorder, "sample_count" .= count,
          "input_path" .= inputPath options, "request_path" .= requestPath options,
          "artifacts" .= artifacts,
          "experiment" .= result
        ]
      report = reportMarkdown recorder status generated result
      body = reportHtml recorder status generated count samples result
  page <- either fail pure (renderPage (recordTemplate recorder) body)
  atomicWriteText (recordDirectory recorder </> "report.md") report
  atomicWriteText (recordDirectory recorder </> "report.html") page
  atomicWriteJson (recordDirectory recorder </> "run.json") metadata
  refreshIndex recorder generated

reportMarkdown :: Recorder -> Text -> Text -> Maybe Value -> Text
reportMarkdown recorder status generated result = Text.unlines
  [ "# " <> methodTitle (recordMethod recorder),
    "Run: " <> recordRunId recorder <> " · State: " <> status,
    "Agent: " <> agentName options <> " · Model: " <> displayedModel options <> " (caller-reported/requested; actual remote model is not inferred)",
    "Snapshot generated: " <> generated,
    "This is a record of one method invocation. It does not certify completion of the user's task.",
    recordNotes recorder,
    maybe "" (\value -> "\n## Experiment observation\n\n```json\n" <> jsonText value <> "\n```\n\nInterpret the observation in the calling agent before changing the next step. The submitted reflection is preserved.") result
  ]
  where options = recordOptions recorder

reportHtml :: Recorder -> Text -> Text -> Int -> [Value] -> Maybe Value -> Text
reportHtml recorder status generated count samples result =
  "<header class=\"motivo-header\"><a href=\"../../index.html\">← All method runs</a>"
  <> "<p class=\"eyebrow\">MOTIVO · METHOD RECORD</p><h1>" <> escapeHtml (methodTitle method) <> "</h1>"
  <> "<p class=\"lede\">" <> escapeHtml (methodPurpose method) <> "</p>"
  <> "<div class=\"meta-grid\">" <> Text.concat (map metadataItem
       [("Run", recordRunId recorder), ("Started", recordStartedAt recorder), ("Agent · caller-reported", agentName options), ("Model identity", displayedModel options), ("Snapshot generated", generated)])
  <> "</div><p class=\"status status-" <> status <> "\">" <> escapeHtml status <> "</p></header>"
  <> "<main data-method=\"" <> methodSlug method <> "\" data-run-id=\"" <> recordRunId recorder <> "\">"
  <> "<aside class=\"note\">This page is a file snapshot. Reload to read a newer generated snapshot. A recorded method or a passing experiment does not certify the user's task. Main-agent work is visible only when the agent records it.</aside>"
  <> "<nav class=\"report-links\"><a href=\"request.md\">Submitted request</a><a href=\"report.md\">Markdown report</a><a href=\"run.json\">Run data</a><a href=\"samples.jsonl\">Ordered samples</a></nav>"
  <> missingNotice
  <> "<section class=\"submitted-material\"><details open><summary>Complete recorded material</summary>" <> renderMarkdown (recordNotes recorder) <> "</details></section>"
  <> "<div class=\"section-grid\">" <> Text.concat (map artifactSection [heading | heading <- methodSections method, lookupSection heading sections /= Nothing]) <> "</div>"
  <> specificArtifacts
  <> reflectionSection
  <> maybe "" experimentSection result
  <> "<section class=\"samples\"><h2>Observed sequence</h2><p>" <> Text.pack (show count) <> " records; at most the latest 200 are shown. Sequence expresses append order, not causality across parallel work.</p>"
  <> "<ol class=\"timeline\">" <> Text.concat (map sampleHtml samples) <> "</ol></section></main>"
  where
    method = recordMethod recorder
    options = recordOptions recorder
    sections = parseSections (recordNotes recorder)
    metadataItem (label, value) = "<div><span>" <> escapeHtml label <> "</span><strong>" <> escapeHtml value <> "</strong></div>"
    specificArtifacts = if method == Retrospect then
      "<section class=\"method-artifacts\"><h2>Review artifacts</h2><nav class=\"report-links\"><a href=\"artifacts/retrospective.md\">Retrospective</a><a href=\"artifacts/handoff.md\">Handoff</a><a href=\"artifacts/decision-history.json\">Decision history</a></nav><p>History retains the source material; decision times and causal order are not inferred.</p></section>"
      else if status == "responded" then "<nav class=\"report-links\"><a href=\"artifacts/provider-response.md\">Original independent response</a></nav>" else ""
    missingNotice = if null (missingSections recorder) then "" else "<aside class=\"note\">Optional fields not supplied: " <> escapeHtml (Text.intercalate ", " (missingSections recorder)) <> ". The complete material is preserved; no model was called to repair its format.</aside>"
    artifactSection heading = "<article class=\"artifact\"><h2>" <> escapeHtml heading <> "</h2>"
      <> renderMarkdown (fromMaybe "Unknown: not supplied." (lookupSection heading sections)) <> "</article>"
    reflectionSection = "<section class=\"reflection\"><h2>Local reflection</h2><p>Recorded reflection is preserved below. The prompts are separate guidance, not inferred labels for individual lines.</p>"
      <> renderMarkdown (fromMaybe "Unknown: no reflection supplied." (lookupSection "Local reflection" sections))
      <> "<details class=\"reflection-prompts\"><summary>Questions for this method</summary><ul>"
      <> Text.concat ["<li>" <> escapeHtml question <> "</li>" | question <- reflectionQuestions method]
      <> "</ul></details></section>"
    experimentSection value = "<section class=\"experiment\"><h2>Experiment observation</h2><p>Interpret this observation in the calling agent. The submitted hypothesis and reflection remain as written before the run.</p>"
      <> "<nav class=\"report-links\">" <> logLink "stdout_path" "Standard output" value <> logLink "stderr_path" "Standard error" value <> "</nav>"
      <> "<details open><summary>Structured result</summary><pre>" <> escapeHtml (jsonText value) <> "</pre></details></section>"
    logLink field label value =
      let path = valueText field value
          prefix = ".tactus/motivotest/" <> recordRunId recorder <> "/" <> sampleId options <> "/"
      in if prefix `Text.isPrefixOf` path && all (`notElem` ["..", "."]) (splitDirectories (Text.unpack path)) && not (Text.any (`elem` ['\\', '\0']) path)
      then "<a href=\"../../../" <> escapeHtml (Text.drop (Text.length (".tactus/" :: Text)) path) <> "\">" <> label <> "</a>" else ""
    sampleHtml value = "<li class=\"sample\"><span class=\"sample-order\">" <> escapeHtml (valueText "seq" value)
      <> "</span><div><time>" <> escapeHtml (valueText "at" value) <> "</time><strong>" <> escapeHtml (valueText "kind" value)
      <> "</strong><p>" <> escapeHtml (valueText "summary" value) <> "</p></div></li>"

refreshIndex :: Recorder -> Text -> IO ()
refreshIndex recorder generated = do
  let runs = recordRoot recorder </> ".tactus/motivo/runs"
  names <- listDirectory runs
  reports <- forM (filter (validToken . Text.pack) names) $ \name -> do
    let directory = runs </> name
    linked <- isLinked directory
    metadataLinked <- isLinked (directory </> "run.json")
    if linked || metadataLinked then pure Nothing else do
      metadata <- try (withBinaryFile (directory </> "run.json") ReadMode (\handle -> Bytes.hGet handle (1024 * 1024 + 1))) :: IO (Either IOException Bytes.ByteString)
      pure $ case metadata of
        Right bytes | Bytes.length bytes <= 1024 * 1024 -> case eitherDecodeStrict' bytes of
          Right value@(Object _) -> Just (name, value)
          _ -> Nothing
        _ -> Nothing
  let sorted = take 200 (sortOn (\(name, value) -> (Down (recordTimestamp value), name)) [row | Just row <- reports])
      body = "<header class=\"motivo-header\"><p class=\"eyebrow\">MOTIVO · WORKSPACE OBSERVATIONS</p><h1>Method runs</h1><p class=\"lede\">Your coding agent leads the work. These records preserve questions, evidence, experiments and decisions.</p></header>"
        <> "<main><aside class=\"note\">Snapshot generated " <> escapeHtml generated <> ". Concurrent writes may make this overview lag behind an individual run report; keep the report path printed by the method. Reload to read newly generated records. No server, model call or execution control lives on this page.</aside>"
        <> "<section class=\"run-list\">" <> (if null sorted then "<p class=\"empty\">No recorded runs.</p>" else Text.concat (map runCard sorted)) <> "</section></main>"
      runCard (name, value) = "<article class=\"run-card\" data-method=\"" <> escapeHtml (valueText "method" value) <> "\"><div><p class=\"eyebrow\">" <> escapeHtml (valueText "started_at" value)
        <> "</p><h2><a href=\"runs/" <> Text.pack name <> "/report.html\">" <> escapeHtml (valueText "title" value) <> "</a></h2><p>" <> escapeHtml (Text.pack name) <> "</p></div><div><p class=\"status\">"
        <> escapeHtml (valueText "status" value) <> "</p><p>" <> escapeHtml (valueText "agent" value) <> " · " <> escapeHtml (valueText "model" value) <> "</p></div></article>"
  page <- either fail pure (renderPage (recordTemplate recorder) body)
  atomicWriteText (recordRoot recorder </> ".tactus/motivo/index.html") page

-- %Q has variable fractional precision, so lexical timestamp order is not time
-- order. Unknown timestamps sort last; the run name breaks equal-time ties.
recordTimestamp :: Value -> Maybe UTCTime
recordTimestamp = parseTimeM True defaultTimeLocale "%Y-%m-%dT%H:%M:%S%QZ"
  . Text.unpack . valueText "started_at"

valueText :: Text -> Value -> Text
valueText key (Object values) = case KeyMap.lookup (fromStringKey key) values of
  Just (String value) -> value
  Just Null -> "not reported"
  Just value -> jsonText value
  Nothing -> "not reported"
  where fromStringKey = Key.fromText
valueText _ _ = "not reported"

jsonText :: Value -> Text
jsonText = Encoding.decodeUtf8 . LazyBytes.toStrict . encode

displayedModel :: Options -> Text
displayedModel options = case selectedProvider options of
  Nothing -> modelName options
  Just provider -> provider <> " / requested: " <> modelName options <> "; actual model: not reported"

nowText :: IO Text
nowText = Text.pack . formatTime defaultTimeLocale "%Y-%m-%dT%H:%M:%S%QZ" <$> getCurrentTime

newRunId :: IO Text
newRunId = Text.pack . ("run-" ++) . formatTime defaultTimeLocale "%Y%m%dT%H%M%S%qZ" <$> getCurrentTime

readBoundedText :: FilePath -> IO Text
readBoundedText path = do
  bytes <- withBinaryFile path ReadMode (\handle -> Bytes.hGet handle (512 * 1024 + 1))
  when (Bytes.length bytes > 512 * 1024) (fail ("Motivo input exceeds 512 KiB: " ++ path))
  either (fail . show) pure (Encoding.decodeUtf8' bytes)

findWorkspace :: FilePath -> IO FilePath
findWorkspace directory = do
  exists <- doesFileExist (directory </> ".tactus/tactus.toml")
  if exists then pure directory else
    let parent = takeDirectory directory in
    if parent == directory then fail "No initialized .tactus workspace found" else findWorkspace parent

isLinked :: FilePath -> IO Bool
isLinked path = pathIsSymbolicLink path `catchIOError` \errorValue ->
  if isDoesNotExistError errorValue then pure False else ioError errorValue

-- Check each output ancestor before creating it; this prevents accidental links
-- from redirecting reports. The experiment plugin owns OS write isolation.
ensureDirectory :: FilePath -> [FilePath] -> IO ()
ensureDirectory _ [] = pure ()
ensureDirectory parent (part : rest) = do
  let path = parent </> part
  linked <- isLinked path
  when linked (fail ("Motivo output directory must not be a symbolic link: " ++ path))
  createDirectoryIfMissing False path
  resolved <- canonicalizePath path
  unless (takeDirectory resolved == parent) (fail "Motivo output directory escaped its parent")
  ensureDirectory resolved rest

atomicWriteText :: FilePath -> Text -> IO ()
atomicWriteText path content = bracketOnError
  (openTempFile (takeDirectory path) ".motivo-write-")
  (\(temporary, handle) -> do
      hClose handle `catchIOError` (\_ -> pure ())
      removeFile temporary `catchIOError` (\_ -> pure ()))
  (\(temporary, handle) -> do
      hSetEncoding handle utf8
      hSetNewlineMode handle noNewlineTranslation
      TextIO.hPutStr handle content
      hFlush handle
      hClose handle
      renameFile temporary path `onException` (removeFile temporary `catchIOError` (\_ -> pure ())))

atomicWriteJson :: FilePath -> Value -> IO ()
atomicWriteJson path = atomicWriteText path . (<> "\n") . jsonText
