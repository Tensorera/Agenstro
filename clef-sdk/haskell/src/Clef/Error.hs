{-# LANGUAGE OverloadedStrings #-}

module Clef.Error
  ( WorkflowCause (..),
    WorkflowError (..),
    renderWorkflowError,
    workflowErrorCode,
    workflowErrorCause,
    workflowErrorDiagnostic,
  )
where

import Control.Exception (Exception (displayException))
import Data.Aeson
  ( ToJSON (toJSON),
    Value,
    object,
    (.=),
  )
import Data.Text (Text)
import qualified Data.Text as Text

-- | A stable machine-readable cause retained alongside the natural-language
-- presentation of a workflow error.  Plugin-specific details stay in the
-- open JSON value and are never rendered directly to a user's terminal.
data WorkflowCause = WorkflowCause
  { workflowCauseCode :: Text,
    workflowCauseMessage :: Text,
    workflowCauseDetails :: Maybe Value
  }
  deriving (Eq, Show)

instance ToJSON WorkflowCause where
  toJSON cause =
    object
      [ "code" .= workflowCauseCode cause,
        "message" .= workflowCauseMessage cause,
        "details" .= workflowCauseDetails cause
      ]

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
  | PluginParameterConflict Text Text [Text]
  | PluginProtocolFailed Text Text
  | PluginProcessFailed Text Text
  | PluginOutcomeUnknown Text Text WorkflowCause
  | PluginReportedFailure Text Text WorkflowCause
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
  PluginParameterConflict pluginName method fields ->
    "plugin '"
      <> pluginName
      <> "' method '"
      <> method
      <> "' input conflicts with runtime-owned parameter(s): "
      <> Text.intercalate ", " fields
  PluginProtocolFailed pluginName message ->
    "plugin '" <> pluginName <> "' violated agenstro.plugin/v1: " <> message
  PluginProcessFailed pluginName message ->
    "plugin '" <> pluginName <> "' process failed: " <> message
  PluginOutcomeUnknown pluginName method cause ->
    "The result of plugin '"
      <> pluginName
      <> "' operation '"
      <> method
      <> "' is unknown. The external operation may have completed, so Clef did not retry it automatically. Cause: "
      <> workflowCauseMessage cause
      <> ". Inspect the workspace and diagnostic record before retrying."
  PluginReportedFailure pluginName method cause ->
    "Plugin '"
      <> pluginName
      <> "' operation '"
      <> method
      <> "' failed: "
      <> workflowCauseMessage cause

workflowErrorCode :: WorkflowError -> Text
workflowErrorCode workflowError = case workflowError of
  RuntimeConfigError _ -> "runtime.configuration_invalid"
  UnknownProvider _ -> "runtime.provider_unknown"
  UnknownEffect _ -> "runtime.effect_unknown"
  UnknownPlugin _ -> "runtime.plugin_unknown"
  RequirementFailed _ -> "workflow.requirement_failed"
  TaskDecodeFailed _ _ -> "workflow.task_decode_failed"
  OperationDecodeFailed _ _ _ -> "workflow.operation_decode_failed"
  PluginDecodeFailed _ _ _ -> "workflow.plugin_decode_failed"
  PluginParameterConflict _ _ _ -> "plugin.parameter_conflict"
  PluginProtocolFailed _ _ -> "plugin.protocol_failed"
  PluginProcessFailed _ _ -> "plugin.process_failed"
  PluginOutcomeUnknown _ _ _ -> "plugin.outcome_unknown"
  PluginReportedFailure _ _ _ -> "plugin.reported_failure"

workflowErrorCause :: WorkflowError -> Maybe WorkflowCause
workflowErrorCause workflowError = case workflowError of
  PluginOutcomeUnknown _ _ cause -> Just cause
  PluginReportedFailure _ _ cause -> Just cause
  _ -> Nothing

-- | Structured evidence for journals and diagnostic tooling.  The
-- human-facing renderer above deliberately does not expose the JSON details.
workflowErrorDiagnostic :: WorkflowError -> Value
workflowErrorDiagnostic workflowError =
  object
    [ "code" .= workflowErrorCode workflowError,
      "message" .= renderWorkflowError workflowError,
      "cause" .= workflowErrorCause workflowError
    ]
