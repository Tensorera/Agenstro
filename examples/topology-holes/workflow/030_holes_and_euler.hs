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

holesAndEuler :: Task () StageReport
holesAndEuler = jsonTask "topology-holes-and-euler" $ \() ->
  "Inspect the tested parser and foreground component code already under solution/. Add the next "
    <> "atomic topology layer. Traverse . background using 8-neighbour connectivity. A background "
    <> "component is a hole exactly when it cannot reach the grid border. Report sorted hole areas "
    <> "and Euler characteristic = foreground component count - hole count. Test two holes, a "
    <> "diagonal background opening, and a foreground island inside a cavity. Keep the domain "
    <> "logic independent from any CLI. Run focused tests. Return JSON only with keys summary "
    <> "(string), files (string array), and testsRun (string array)."

workflow :: Workflow StageReport
workflow = do
  report <- invoke holesAndEuler ()
  requireBecause "the topology stage reported neither files nor tests" valid report
  where
    valid value = not (null (files value)) && not (null (testsRun value))

main :: IO ()
main = runTactus workflow >>= print
