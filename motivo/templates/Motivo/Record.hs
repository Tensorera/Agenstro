{-# LANGUAGE OverloadedStrings #-}

-- | Shared bounded input and safe, atomic report publication.
module Motivo.Record
  ( atomicWriteJson, atomicWriteText, ensureDirectory, findWorkspace, isLinked,
    jsonText, newRunId, nowText, readBoundedText, refreshWorkspaceIndex, validToken,
    valueText
  ) where

import Control.Exception (IOException, bracketOnError, onException, try)
import Control.Monad (forM, unless, when)
import Data.Aeson (Value (..), eitherDecodeStrict', encode)
import qualified Data.Aeson.Key as Key
import qualified Data.Aeson.KeyMap as KeyMap
import qualified Data.ByteString as Bytes
import qualified Data.ByteString.Lazy as LazyBytes
import Data.Char (isAlphaNum, isAscii)
import Data.List (sortOn)
import Data.Ord (Down (..))
import Data.Text (Text)
import qualified Data.Text as Text
import qualified Data.Text.Encoding as Encoding
import qualified Data.Text.IO as TextIO
import Data.Time (UTCTime, defaultTimeLocale, formatTime, getCurrentTime, parseTimeM)
import Motivo.Html (escapeHtml, renderPage)
import System.Directory
  ( canonicalizePath, createDirectoryIfMissing, doesFileExist, listDirectory,
    pathIsSymbolicLink, removeFile, renameFile
  )
import System.FilePath ((</>), takeDirectory)
import System.IO
  ( IOMode (ReadMode), hClose, hFlush, hSetEncoding, hSetNewlineMode,
    noNewlineTranslation, openTempFile, utf8, withBinaryFile
  )
import System.IO.Error (catchIOError, isDoesNotExistError)

validToken :: Text -> Bool
validToken value = not (Text.null value) && Text.length value <= 80 && Text.all valid value
  where valid character = isAscii character && (isAlphaNum character || character == '_' || character == '-')

jsonText :: Value -> Text
jsonText = Encoding.decodeUtf8 . LazyBytes.toStrict . encode

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

-- | Refuse links in every output ancestor. The effect owns OS isolation.
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

-- | New test entries must not depend on an export added to a preserved old Run
-- module. Both report types use this newly installed helper instead.
refreshWorkspaceIndex :: FilePath -> Text -> Text -> IO ()
refreshWorkspaceIndex root template generated = do
  let runs = root </> ".tactus/motivo/runs"
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
  page <- either fail pure (renderPage template body)
  atomicWriteText (root </> ".tactus/motivo/index.html") page

-- Fractional precision varies; parse times instead of sorting their text.
recordTimestamp :: Value -> Maybe UTCTime
recordTimestamp = parseTimeM True defaultTimeLocale "%Y-%m-%dT%H:%M:%S%QZ"
  . Text.unpack . valueText "started_at"

valueText :: Text -> Value -> Text
valueText key (Object values) = case KeyMap.lookup (Key.fromText key) values of
  Just (String value) -> value
  Just Null -> "not reported"
  Just value -> jsonText value
  Nothing -> "not reported"
valueText _ _ = "not reported"
