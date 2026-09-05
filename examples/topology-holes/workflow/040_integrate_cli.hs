{-# LANGUAGE DeriveAnyClass #-}
{-# LANGUAGE DeriveGeneric #-}
{-# LANGUAGE DerivingStrategies #-}
{-# LANGUAGE OverloadedStrings #-}

module Main (main) where

import Clef
import Data.Aeson (FromJSON)
import Data.Text (Text)
import qualified Data.Text as Text
import GHC.Generics (Generic)

data Review = Review
  { approved :: Bool,
    findings :: [Text]
  }
  deriving stock (Eq, Show, Generic)
  deriving anyclass (FromJSON)

data StageReport = StageReport
  { summary :: Text,
    files :: [FilePath],
    testsRun :: [Text]
  }
  deriving stock (Eq, Show, Generic)
  deriving anyclass (FromJSON)

algorithmReview :: Task () Review
algorithmReview = jsonTask "topology-algorithm-review" $ \() ->
  "Read solution/ without editing it. Review parser validation, 4-connected foreground, "
    <> "8-connected background, border reachability, hole areas, and Euler characteristic. "
    <> "Look for diagonal-connectivity and island-in-cavity mistakes. Return JSON only with "
    <> "approved (boolean) and findings (string array)."

interfaceReview :: Task () Review
interfaceReview = jsonTask "topology-interface-review" $ \() ->
  "Read solution/ without editing it. Review its planned command-line contract and testability. "
    <> "The final executable must accept exactly one grid path, write deterministic compact JSON "
    <> "to stdout, diagnostics to stderr, and a nonzero status on invalid input. Return JSON only "
    <> "with approved (boolean) and findings (string array)."

integrate :: Task (Review, Review) StageReport
integrate = jsonTask "topology-integrate-cli" $ \(algorithm, interface) ->
  "Finish the existing solution/ as a complete strongly typed CLI. Address both reviews, add an "
    <> "end-to-end fixture equivalent to a 9x5 connected frame containing two 3x3 holes, and run "
    <> "the complete test suite. Expected values are one foreground component, two holes with "
    <> "areas [9,9], and Euler characteristic -1. Keep deterministic JSON field names and avoid "
    <> "a GUI. Algorithm findings: "
    <> renderFindings (findings algorithm)
    <> ". Algorithm approved: "
    <> Text.pack (show (approved algorithm))
    <> ". Interface findings: "
    <> renderFindings (findings interface)
    <> ". Interface approved: "
    <> Text.pack (show (approved interface))
    <> ". Return JSON only with keys summary (string), files (string array), and testsRun "
    <> "(string array)."

workflow :: Workflow StageReport
workflow = do
  reviews <- parallel (invoke algorithmReview ()) (invoke interfaceReview ())
  -- A rejection supplies repair input to this one integration pass.  It must
  -- not prevent the task that addresses the reviewers' findings from running.
  report <- invoke integrate reviews
  -- These fields describe provider-reported work.  Independently verifying
  -- the generated program remains the responsibility of the caller's harness.
  requireBecause "the integration report omitted files or test commands" valid report
  where
    valid value = not (null (files value)) && not (null (testsRun value))

renderFindings :: [Text] -> Text
renderFindings [] = "none"
renderFindings values = Text.intercalate " | " values

main :: IO ()
main = runTactus workflow >>= print
