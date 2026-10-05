{-# LANGUAGE OverloadedStrings #-}

-- | Shared bounded input and safe, atomic report publication.
module Motivo.Record
  ( atomicWriteJson, atomicWriteText, ensureDirectory, findWorkspace, isLinked,
    jsonText, newRunId, nowText, readBoundedText, validToken
  ) where

import Control.Exception (bracketOnError, onException)
import Control.Monad (unless, when)
import Data.Aeson (Value, encode)
import qualified Data.ByteString as Bytes
import qualified Data.ByteString.Lazy as LazyBytes
import Data.Char (isAlphaNum, isAscii)
import Data.Text (Text)
import qualified Data.Text as Text
import qualified Data.Text.Encoding as Encoding
import qualified Data.Text.IO as TextIO
import Data.Time (defaultTimeLocale, formatTime, getCurrentTime)
import System.Directory
  ( canonicalizePath, createDirectoryIfMissing, doesFileExist,
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
