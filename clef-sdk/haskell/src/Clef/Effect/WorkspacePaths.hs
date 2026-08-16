{-# LANGUAGE OverloadedStrings #-}

module Clef.Effect.WorkspacePaths
  ( WorkspaceSnapshot (..),
    WorkspacePathDiff (..),
    ForgetResult (..),
    snapshot,
    diff,
    forget,
  )
where

import Data.Aeson
  ( FromJSON (parseJSON),
    ToJSON (toJSON),
    object,
    withObject,
    (.:),
    (.=),
  )
import Data.Text (Text)
import Clef.Workflow (Operation, operation)

-- | An opaque snapshot allocated by the @workspace.paths@ effect plugin.
newtype WorkspaceSnapshot = WorkspaceSnapshot
  { workspaceSnapshotId :: Text
  }
  deriving (Eq, Show)

-- | A path-only diff.  Content storage and artifact publication are outside
-- this effect's contract.
data WorkspacePathDiff = WorkspacePathDiff
  { workspaceAddedPaths :: [FilePath],
    workspaceModifiedPaths :: [FilePath],
    workspaceDeletedPaths :: [FilePath],
    workspaceTypeChangedPaths :: [FilePath]
  }
  deriving (Eq, Show)

newtype ForgetResult = ForgetResult
  { workspaceSnapshotForgotten :: Bool
  }
  deriving (Eq, Show)

instance FromJSON WorkspaceSnapshot where
  parseJSON = withObject "workspace snapshot" $ \objectValue ->
    WorkspaceSnapshot <$> objectValue .: "snapshot_id"

instance ToJSON WorkspaceSnapshot where
  toJSON workspaceSnapshot =
    object ["snapshot_id" .= workspaceSnapshotId workspaceSnapshot]

instance FromJSON WorkspacePathDiff where
  parseJSON = withObject "workspace path diff" $ \objectValue ->
    WorkspacePathDiff
      <$> objectValue .: "added"
      <*> objectValue .: "modified"
      <*> objectValue .: "deleted"
      <*> objectValue .: "type_changed"

instance FromJSON ForgetResult where
  parseJSON = withObject "workspace snapshot forget result" $ \objectValue ->
    ForgetResult <$> objectValue .: "forgotten"

snapshot :: Operation WorkspaceSnapshot
snapshot = operation "workspace.paths" "snapshot" (object [])

diff :: WorkspaceSnapshot -> WorkspaceSnapshot -> Operation WorkspacePathDiff
diff before after =
  operation
    "workspace.paths"
    "diff"
    ( object
        [ "before" .= before,
          "after" .= after
        ]
    )

forget :: WorkspaceSnapshot -> Operation ForgetResult
forget workspaceSnapshot =
  operation "workspace.paths" "forget" workspaceSnapshot
