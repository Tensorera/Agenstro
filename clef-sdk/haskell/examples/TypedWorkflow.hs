{-# LANGUAGE DeriveAnyClass #-}
{-# LANGUAGE DeriveGeneric #-}
{-# LANGUAGE DerivingStrategies #-}
{-# LANGUAGE OverloadedStrings #-}

module Main (main) where

import Clef
import qualified Clef.Effect.WorkspacePaths as WorkspacePaths
import Data.Aeson (FromJSON)
import Data.Text (Text)
import qualified Data.Text as Text
import GHC.Generics (Generic)

data Plan = Plan
  { summary :: Text,
    files :: [FilePath]
  }
  deriving stock (Eq, Show, Generic)
  deriving anyclass (FromJSON)

data Review = Review
  { approved :: Bool,
    notes :: Text
  }
  deriving stock (Eq, Show, Generic)
  deriving anyclass (FromJSON)

planTask :: Task Text Plan
planTask = jsonTask "plan" $ \request ->
  "Plan this coding request and return JSON with summary and files: " <> request

reviewTask :: Task Plan Review
reviewTask = jsonTask "review" $ \plan ->
  "Review this plan and return JSON with approved and notes: " <> summary plan

alternateReviewTask :: Task Plan Review
alternateReviewTask = jsonTask "alternate-review" $ \plan ->
  "Independently review these files and return JSON with approved and notes: "
    <> Text.pack (show (files plan))

workflow :: Workflow ([Review], WorkspacePaths.WorkspacePathDiff)
workflow = do
  before <- perform WorkspacePaths.snapshot
  plan <- invoke planTask "Add typed workflow support"
  (primaryReview, alternateReview) <-
    parallel (invoke reviewTask plan) (invoke alternateReviewTask plan)
  let reviews = [primaryReview, alternateReview]
  _ <- requireBecause "at least one reviewer rejected the plan" (all approved) reviews
  after <- perform WorkspacePaths.snapshot
  changes <- perform (WorkspacePaths.diff before after)
  _ <- perform (WorkspacePaths.forget before)
  _ <- perform (WorkspacePaths.forget after)
  pure (reviews, changes)

main :: IO ()
main = do
  result <- runTactus workflow
  print result
