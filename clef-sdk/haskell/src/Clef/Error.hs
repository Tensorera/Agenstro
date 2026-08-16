{-# LANGUAGE OverloadedStrings #-}

module Clef.Error
  ( WorkflowError (..),
  )
where

import Control.Exception (Exception (displayException))
import Data.Aeson (Value)
import Data.Text (Text)
import qualified Data.Text as Text

-- | Failures at the dynamic boundary of a workflow.
--
-- Haskell type errors never become values of this type: GHC rejects those
-- before a script starts.  These constructors describe configuration,
-- subprocess protocol, provider, effect, and decoded-result failures.
data WorkflowError
  = RuntimeConfigError Text
  | UnknownProvider Text
  | UnknownEffect Text
  | UnknownPlugin Text
  | RequirementFailed Text
  | TaskDecodeFailed Text Text
  | OperationDecodeFailed Text Text Text
  | PluginDecodeFailed Text Text Text
  | PluginProtocolFailed Text Text
  | PluginProcessFailed Text Text
  | PluginOutcomeUnknown Text Text Text
  | PluginReportedFailure Text Text Value
  | RuntimeSinkFailed Text
  deriving (Eq, Show)

instance Exception WorkflowError where
  displayException = Text.unpack . renderWorkflowError

renderWorkflowError :: WorkflowError -> Text
renderWorkflowError workflowError = case workflowError of
  RuntimeConfigError message -> "invalid Clef runtime configuration: " <> message
  UnknownProvider name -> "unknown provider: " <> name
  UnknownEffect name -> "unknown effect: " <> name
  UnknownPlugin name -> "unknown plugin: " <> name
  RequirementFailed message -> "workflow requirement failed: " <> message
  TaskDecodeFailed name message ->
    "task '" <> name <> "' returned an invalid result: " <> message
  OperationDecodeFailed effectName method message ->
    "effect '" <> effectName <> "' method '" <> method <> "' returned an invalid result: " <> message
  PluginDecodeFailed pluginName method message ->
    "plugin '" <> pluginName <> "' method '" <> method <> "' returned an invalid result: " <> message
  PluginProtocolFailed pluginName message ->
    "plugin '" <> pluginName <> "' violated agenstro.plugin/v1: " <> message
  PluginProcessFailed pluginName message ->
    "plugin '" <> pluginName <> "' process failed: " <> message
  PluginOutcomeUnknown pluginName method message ->
    "plugin '"
      <> pluginName
      <> "' method '"
      <> method
      <> "' may have completed externally; outcome unknown: "
      <> message
  PluginReportedFailure pluginName method failure ->
    "plugin '"
      <> pluginName
      <> "' method '"
      <> method
      <> "' failed: "
      <> Text.pack (show failure)
  RuntimeSinkFailed message ->
    "runtime event sink failed: " <> message
