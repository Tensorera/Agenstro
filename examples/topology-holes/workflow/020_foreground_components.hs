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

foregroundComponents :: Task () StageReport
foregroundComponents = jsonTask "topology-foreground-components" $ \() ->
  "Inspect the existing solution/ from stage 010. Add exactly the next atomic capability: "
    <> "count # foreground components using 4-neighbour connectivity. Keep traversal iterative "
    <> "or otherwise safe for large grids, separate it from parsing, and add tests for empty "
    <> "background, one component, multiple components, and diagonal # cells. Do not implement "
    <> "hole counting or the final CLI yet. Run focused tests. Return JSON only with keys summary "
    <> "(string), files (string array), and testsRun (string array)."

workflow :: Workflow StageReport
workflow = do
  report <- invoke foregroundComponents ()
  requireBecause "the component stage did not run tests" (not . null . testsRun) report

main :: IO ()
main = runTactus workflow >>= print
