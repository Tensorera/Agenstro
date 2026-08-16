{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE ScopedTypeVariables #-}

module Segno.Cli (main) where

import Control.Exception (SomeException, displayException, try)
import Control.Monad (forM_, when)
import Data.Aeson (ToJSON, Value, encode, object, withObject, (.:), (.=))
import qualified Data.Aeson.KeyMap as KeyMap
import Data.Aeson.Types (parseEither)
import qualified Data.ByteString.Lazy.Char8 as LazyChar8
import Data.Text (Text)
import qualified Data.Text as Text
import Data.Time (NominalDiffTime)
import Options.Applicative
  ( Parser,
    ParserInfo,
    ReadM,
    argument,
    auto,
    command,
    eitherReader,
    execParser,
    fullDesc,
    header,
    help,
    helper,
    hsubparser,
    info,
    infoOption,
    long,
    metavar,
    option,
    optional,
    progDesc,
    short,
    strArgument,
    strOption,
    switch,
    value,
    (<**>),
  )
import Segno.Driver
  ( DriverEnvironment (driverTaskTimeoutSeconds),
    RunSummary,
    Runner (runnerPlugin),
    defaultDriverEnvironment,
    discoverWorkspaceRoot,
    initialiseWorkspaceWithSdk,
    installJob,
    listJobs,
    processRunner,
    runDriverWith,
    runOnceWith,
  )
import Segno.Lifecycle
  ( OccurrenceRecord (..),
    lifecycleText,
  )
import Segno.Plugin
  ( runActiveWindowPluginHost,
    runStatePluginHost,
    runTimePluginHost,
  )
import Segno.Protocol
  ( InstalledJob (..),
    PluginFailure (..),
    TaskManifest (..),
    TriggerOccurrence (..),
  )
import Segno.Store.SQLite
  ( SegnoPaths (pathsControl),
    initialiseStore,
    lifecycleHistory,
    lifecycleStatus,
    segnoPaths,
  )
import System.Exit (exitFailure)
import System.IO (hPutStrLn, stderr)

data Command
  = Init FilePath (Maybe FilePath)
  | Install FilePath FilePath (Maybe FilePath) Int
  | List FilePath Bool
  | Once FilePath Bool Int
  | Driver FilePath NominalDiffTime Int
  | Status FilePath (Maybe Text) Bool
  | History FilePath (Maybe Text) (Maybe Text) Int Bool
  | TimePlugin Text
  | StatePlugin
  | ActiveWindowPlugin

main :: IO ()
main = do
  selected <- execParser options
  outcome <- try (execute selected)
  case outcome of
    Left (exception :: SomeException) -> do
      hPutStrLn stderr ("segno: " <> displayException exception)
      exitFailure
    Right () -> pure ()

options :: ParserInfo Command
options =
  info
    ( commandParser
        <**> helper
        <**> infoOption "segno 0.3.0" (long "version" <> help "Show Segno version")
    )
    (fullDesc <> header "Segno - typed persistent workflows for Clef and Tactus")

commandParser :: Parser Command
commandParser =
  hsubparser $
    command "init" (info initParser (progDesc "Initialize .tactus/segno and register built-in plugins"))
      <> command "install" (info installParser (progDesc "Install a typed persistent Haskell task"))
      <> command "list" (info listParser (progDesc "List installed persistent tasks"))
      <> command "once" (info onceParser (progDesc "Poll triggers and drain runnable occurrences once"))
      <> command "driver" (info driverParser (progDesc "Run the single-node persistent driver"))
      <> command "status" (info statusParser (progDesc "Show runtime-owned occurrence lifecycle"))
      <> command "history" (info historyParser (progDesc "Show lifecycle or business-state history"))
      <> command "time-plugin" (info timePluginParser (progDesc "Run a time trigger plugin-v1 host"))
      <> command "state-plugin" (info (pure StatePlugin) (progDesc "Run the SQLite state plugin-v1 host"))
      <> command "active-window-plugin" (info (pure ActiveWindowPlugin) (progDesc "Run the Windows active-window plugin-v1 host"))

rootOption :: Parser FilePath
rootOption = strOption (long "root" <> short 'r' <> metavar "PATH" <> value "." <> help "Tactus workspace or a descendant")

sdkOption :: Parser (Maybe FilePath)
sdkOption = optional (strOption (long "sdk" <> metavar "PATH" <> help "Directory containing segno-flow.cabal"))

jsonOption :: Parser Bool
jsonOption = switch (long "json" <> help "Emit machine-readable JSON")

taskTimeoutOption :: Parser Int
taskTimeoutOption =
  option
    taskTimeoutReader
    ( long "task-timeout-seconds"
        <> metavar "SECONDS"
        <> value 1800
        <> help "Per Tactus build/run phase timeout (1..604800 seconds)"
    )

initParser :: Parser Command
initParser = Init <$> rootOption <*> sdkOption

installParser :: Parser Command
installParser =
  Install
    <$> rootOption
    <*> strArgument (metavar "SCRIPT" <> help "Haskell entry point inside the workspace")
    <*> sdkOption
    <*> taskTimeoutOption

listParser :: Parser Command
listParser = List <$> rootOption <*> jsonOption

onceParser :: Parser Command
onceParser = Once <$> rootOption <*> jsonOption <*> taskTimeoutOption

driverParser :: Parser Command
driverParser =
    Driver
    <$> rootOption
    <*> (realToFrac <$> option auto (long "poll-seconds" <> metavar "SECONDS" <> value (1 :: Double) <> help "Maximum idle wait between polls"))
    <*> taskTimeoutOption

statusParser :: Parser Command
statusParser =
  Status
    <$> rootOption
    <*> optional (Text.pack <$> strOption (long "job" <> metavar "TASK" <> help "Filter by task id"))
    <*> jsonOption

historyParser :: Parser Command
historyParser =
  History
    <$> rootOption
    <*> optional (Text.pack <$> strOption (long "state-key" <> metavar "KEY" <> help "Read business-state plugin history"))
    <*> optional (Text.pack <$> strOption (long "occurrence" <> metavar "ID" <> help "Filter lifecycle history"))
    <*> option boundedLimitReader (long "limit" <> metavar "N" <> value 100 <> help "Maximum rows (1..1000)")
    <*> jsonOption

timePluginParser :: Parser Command
timePluginParser =
  TimePlugin
    <$> argument
      (eitherReader parseTimeKind)
      (metavar "interval|cron")
  where
    parseTimeKind "interval" = Right "time.interval"
    parseTimeKind "cron" = Right "time.cron"
    parseTimeKind _ = Left "expected interval or cron"

boundedLimitReader :: ReadM Int
boundedLimitReader = eitherReader $ \encoded -> case reads encoded of
  [(number, "")] | number >= 1 && number <= (1000 :: Int) -> Right number
  _ -> Left "limit must be between 1 and 1000"

taskTimeoutReader :: ReadM Int
taskTimeoutReader = eitherReader $ \encoded -> case reads encoded of
  [(number, "")] | number >= 1 && number <= (604800 :: Int) -> Right number
  _ -> Left "task timeout must be between 1 and 604800 seconds"

execute :: Command -> IO ()
execute selected = case selected of
  Init start sdk -> do
    paths <- initialiseWorkspaceWithSdk start sdk
    putStrLn ("initialized " <> pathsControl paths)
  Install start script sdk timeoutSeconds -> do
    _ <- initialiseWorkspaceWithSdk start sdk
    let environment = defaultDriverEnvironment {driverTaskTimeoutSeconds = timeoutSeconds}
    job <- installJob environment start script
    putStrLn ("installed " <> Text.unpack (manifestTask (installedManifest job)))
  List start json -> do
    jobs <- listJobs start
    if json
      then emitJson jobs
      else forM_ jobs $ \job -> putStrLn (Text.unpack (manifestTask (installedManifest job)) <> "\t" <> installedScript job)
  Once start json timeoutSeconds -> do
    let environment = defaultDriverEnvironment {driverTaskTimeoutSeconds = timeoutSeconds}
    summary <- runOnceWith environment start
    if json then emitJson summary else printSummary summary
  Driver start seconds timeoutSeconds -> do
    let secondsAsDouble = realToFrac seconds :: Double
    when (seconds <= 0 || isNaN secondsAsDouble || isInfinite secondsAsDouble) (fail "poll-seconds must be finite and greater than zero")
    let environment = defaultDriverEnvironment {driverTaskTimeoutSeconds = timeoutSeconds}
    runDriverWith environment start seconds
  Status start selectedJob json -> do
    root <- discoverWorkspaceRoot start
    let paths = segnoPaths root
    initialiseStore paths
    records <- lifecycleStatus paths selectedJob
    if json
      then emitJson (fmap encodeOccurrenceRecord records)
      else forM_ records printOccurrenceRecord
  History start stateKey occurrence limit json -> do
    when (stateKey /= Nothing && occurrence /= Nothing) (fail "choose either --state-key or --occurrence")
    root <- discoverWorkspaceRoot start
    let paths = segnoPaths root
    initialiseStore paths
    entries <- case stateKey of
      Just key -> do
        response <-
          runnerPlugin
            processRunner
            root
            "segno.state"
            "history"
            (KeyMap.fromList ["workspace" .= root, "state_key" .= key, "limit" .= limit])
        case response of
          Left failure -> fail (Text.unpack (pluginFailureMessage failure))
          Right responseValue -> case parseEither (withObject "state history response" (.: "entries")) responseValue of
            Left message -> fail message
            Right entries -> pure entries
      Nothing -> lifecycleHistory paths occurrence limit
    if json then emitJson entries else forM_ entries (LazyChar8.putStrLn . encode)
  TimePlugin pluginName -> runTimePluginHost pluginName
  StatePlugin -> runStatePluginHost
  ActiveWindowPlugin -> runActiveWindowPluginHost

emitJson :: ToJSON value => value -> IO ()
emitJson = LazyChar8.putStrLn . encode

printSummary :: RunSummary -> IO ()
printSummary = LazyChar8.putStrLn . encode

encodeOccurrenceRecord :: OccurrenceRecord -> Value
encodeOccurrenceRecord record =
  object
    [ "job" .= recordJobId record,
      "occurrence" .= recordOccurrence record,
      "lifecycle" .= lifecycleText (recordLifecycle record),
      "attempt" .= recordAttempt record,
      "fencing_token" .= recordFencingToken record,
      "lease_until" .= recordLeaseUntil record,
      "next_attempt_at" .= recordNextAttemptAt record,
      "updated_at" .= recordUpdatedAt record,
      "error" .= recordError record,
      "output" .= recordOutput record
    ]

printOccurrenceRecord :: OccurrenceRecord -> IO ()
printOccurrenceRecord record =
  putStrLn $
    Text.unpack (recordJobId record)
      <> "\t"
      <> Text.unpack (occurrenceId (recordOccurrence record))
      <> "\t"
      <> Text.unpack (lifecycleText (recordLifecycle record))
      <> "\tattempt="
      <> show (recordAttempt record)
