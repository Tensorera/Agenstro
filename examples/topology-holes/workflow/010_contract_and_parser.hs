{-# LANGUAGE DeriveAnyClass #-}
{-# LANGUAGE DeriveGeneric #-}
{-# LANGUAGE DerivingStrategies #-}
{-# LANGUAGE OverloadedStrings #-}

module Main (main) where

import Clef
import Data.Aeson (FromJSON)
import Data.Text (Text)
import GHC.Generics (Generic)

data StageReport = StageReport
  { summary :: Text,
    files :: [FilePath],
    testsRun :: [Text]
  }
  deriving stock (Eq, Show, Generic)
  deriving anyclass (FromJSON)

contractAndParser :: Task Text StageReport
contractAndParser = jsonTask "topology-contract-and-parser" $ \goal ->
  "Work only on the first atomic stage of this goal:\n"
    <> goal
    <> "\nCreate a strongly typed, dependency-light project under solution/. "
    <> "Define a rectangular ASCII grid contract where # is foreground and . is background. "
    <> "Implement strict parsing with useful errors for empty, ragged, or unknown input. "
    <> "Add focused parser tests. Do not implement connectivity, holes, Euler characteristic, "
    <> "or the final CLI yet. Run the smallest relevant tests. Return JSON only with keys "
    <> "summary (string), files (string array), and testsRun (string array)."

workflow :: Workflow StageReport
workflow = do
  report <- invoke contractAndParser "Build a planar grid topology and hole-counting CLI."
  requireBecause "the atomic parser stage did not report any files" (not . null . files) report

main :: IO ()
main = runTactus workflow >>= print
