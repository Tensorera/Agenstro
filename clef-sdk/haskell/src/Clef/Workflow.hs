{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE ScopedTypeVariables #-}
{-# LANGUAGE TupleSections #-}

module Clef.Workflow
  ( Workflow,
    Task,
    Operation,
    Plugin,
    ProviderRef (..),
    providerRef,
    task,
    textTask,
    jsonTask,
    operation,
    jsonPlugin,
    rawPlugin,
    decodeTaskResult,
    invoke,
    invokeWith,
    perform,
    call,
    parallel,
    parallelAll,
    parallelAllBounded,
    require,
    requireBecause,
    attempt,
    liftIO,
    runWorkflow,
    runTactus,
    runTactusWithRecords,
  )
where

import Control.Applicative ((<|>))
import Control.Concurrent.Async (concurrently, mapConcurrently)
import Control.Concurrent.QSem (newQSem, signalQSem, waitQSem)
import Control.Exception
  ( SomeException,
    bracket_,
    displayException,
    finally,
    fromException,
    mask,
    throwIO,
    try,
  )
import Control.Monad (unless)
import Control.Monad.IO.Class (MonadIO (liftIO))
import Data.Aeson
  ( FromJSON (parseJSON),
    Object,
    ToJSON (toJSON),
    Value (Object),
    object,
    (.:),
    (.=),
  )
import qualified Data.Aeson.Key as Key
import qualified Data.Aeson.KeyMap as KeyMap
import Data.Aeson.Types (parseEither)
import qualified Data.Map.Strict as Map
import Data.Maybe (catMaybes)
import Data.Text (Text)
import qualified Data.Text as Text
import Data.Text.Encoding (encodeUtf8)
import System.Exit (exitFailure)
import Clef.Diagnostic
  ( PresentationLevel (..),
    RuntimeMessage (..),
    RuntimeStateTransition (..),
    TransitionGuard (..),
    TransitionTrigger (..),
    TriggerKind (..),
    renderPresentationLine,
  )
import Clef.Error
  ( WorkflowError (..),
    renderWorkflowError,
    workflowErrorDiagnostic,
  )
import Clef.Internal.Exception (isAsynchronousException)
import Clef.Plugin.Protocol (decodeStrictJSON)
import Clef.Runtime
  ( PluginCallResult (..),
    Runtime,
    RuntimeRecord (..),
    callPlugin,
    closeRuntime,
    flushRuntimeSink,
    freshRuntimeId,
    newRuntime,
    readRuntimeRecords,
    recordRuntime,
    runtimeConfig,
    writeRuntimePresentation,
  )
import qualified Clef.Runtime.Config as Config

-- | A deliberately small dynamic workflow.  It is abstract so its runtime can
-- evolve without changing scripts, but it has no indexed effect row, typestate,
-- or hidden DAG.
newtype Workflow a = Workflow
  { executeWorkflow :: Runtime -> IO a
  }

instance Functor Workflow where
  fmap transform workflow = Workflow $ \runtime -> transform <$> executeWorkflow workflow runtime

instance Applicative Workflow where
  pure value = Workflow $ \_ -> pure value
  functionWorkflow <*> valueWorkflow = Workflow $ \runtime ->
    executeWorkflow functionWorkflow runtime <*> executeWorkflow valueWorkflow runtime

instance Monad Workflow where
  workflow >>= next = Workflow $ \runtime -> do
    value <- executeWorkflow workflow runtime
    executeWorkflow (next value) runtime

instance MonadIO Workflow where
  liftIO action = Workflow $ \_ -> action

data Task input output = Task
  { internalTaskName :: Text,
    internalRenderPrompt :: input -> Text,
    internalDecodeTask :: Text -> Either Text output
  }

data Operation output = Operation
  { internalEffectName :: Text,
    internalEffectMethod :: Text,
    internalEffectParams :: Value,
    internalDecodeOperation :: Value -> Either Text output
  }

-- | An open plugin method with a statically typed request and result.  Plugin
-- availability remains dynamic and is resolved from the runtime's independent
-- @plugins@ registry.
data Plugin input output = Plugin
  { internalPluginName :: Text,
    internalPluginMethod :: Text,
    internalEncodePluginInput :: input -> Value,
    internalDecodePluginOutput :: Value -> Either Text output
  }

-- | A provider name plus open-ended per-invocation overrides.  Model, effort,
-- options, and extra arguments are intentionally not enums.
data ProviderRef = ProviderRef
  { providerRefName :: Text,
    providerRefModel :: Maybe Text,
    providerRefEffort :: Maybe Text,
    providerRefOptions :: Object,
    providerRefExtraArgs :: [Text]
  }
  deriving (Eq, Show)

providerRef :: Text -> ProviderRef
providerRef name =
  ProviderRef
    { providerRefName = name,
      providerRefModel = Nothing,
      providerRefEffort = Nothing,
      providerRefOptions = mempty,
      providerRefExtraArgs = []
    }

textTask :: Text -> (input -> Text) -> Task input Text
textTask name renderPrompt = task name renderPrompt Right

-- | Build a task with an arbitrary statically typed text decoder.
task :: Text -> (input -> Text) -> (Text -> Either Text output) -> Task input output
task = Task

jsonTask :: FromJSON output => Text -> (input -> Text) -> Task input output
jsonTask name renderPrompt = task name renderPrompt decodeJson
  where
    decodeJson text = case decodeStrictJSON (encodeUtf8 text) of
      Left message -> Left (Text.pack message)
      Right value -> Right value

operation :: (ToJSON input, FromJSON output) => Text -> Text -> input -> Operation output
operation effectName method params =
  Operation
    { internalEffectName = effectName,
      internalEffectMethod = method,
      internalEffectParams = toJSON params,
      internalDecodeOperation = \value ->
        case parseEither parseJSON value of
          Left message -> Left (Text.pack message)
          Right result -> Right result
    }

-- | Define a typed JSON plugin method.  GHC checks the input/output wiring;
-- Aeson validates the dynamic result at the plugin boundary.
jsonPlugin :: (ToJSON input, FromJSON output) => Text -> Text -> Plugin input output
jsonPlugin pluginName method =
  Plugin
    { internalPluginName = pluginName,
      internalPluginMethod = method,
      internalEncodePluginInput = toJSON,
      internalDecodePluginOutput = \value ->
        case parseEither parseJSON value of
          Left message -> Left (Text.pack message)
          Right result -> Right result
    }

-- | Fully open JSON escape hatch for plugins whose schema is intentionally
-- dynamic.
rawPlugin :: Text -> Text -> Plugin Value Value
rawPlugin pluginName method =
  Plugin
    { internalPluginName = pluginName,
      internalPluginMethod = method,
      internalEncodePluginInput = id,
      internalDecodePluginOutput = Right
    }

decodeTaskResult :: Task input output -> Text -> Either WorkflowError output
decodeTaskResult selectedTask text =
  case internalDecodeTask selectedTask text of
    Left message -> Left (TaskDecodeFailed (internalTaskName selectedTask) message)
    Right value -> Right value

invoke :: Task input output -> input -> Workflow output
invoke selectedTask input = Workflow $ \runtime -> do
  let config = runtimeConfig runtime
  invokeTask runtime (providerRef (Config.runtimeDefaultProvider config)) selectedTask input

invokeWith :: ProviderRef -> Task input output -> input -> Workflow output
invokeWith selectedProvider selectedTask input = Workflow $ \runtime ->
  invokeTask runtime selectedProvider selectedTask input

perform :: Operation output -> Workflow output
perform selectedOperation = Workflow $ \runtime -> do
  let config = runtimeConfig runtime
      effectName = internalEffectName selectedOperation
      method = internalEffectMethod selectedOperation
  effectConfig <- case Map.lookup effectName (Config.runtimeEffects config) of
    Nothing -> throwIO (UnknownEffect effectName)
    Just found -> pure found
  pluginParams <-
    either
      (throwIO . PluginParameterConflict ("effect:" <> effectName) method)
      pure
      ( augmentPluginParams
          (Config.runtimeWorkspace config)
          (Config.effectOptions effectConfig)
          (internalEffectParams selectedOperation)
      )
  pluginResult <-
    callPlugin
      runtime
      ("effect:" <> effectName)
      (Config.effectCommand effectConfig)
      method
      pluginParams
  recordRuntime
    runtime
    (EffectEvidenceRecord (pluginCallId pluginResult) effectName method (pluginCallValue pluginResult))
  case internalDecodeOperation selectedOperation (pluginCallValue pluginResult) of
    Left message -> throwIO (OperationDecodeFailed effectName method message)
    Right value -> pure value

-- | Invoke an arbitrary configured plugin.  Streaming events are delivered to
-- the runtime sink as a side channel; only the typed terminal result enters
-- the workflow value graph.
call :: Plugin input output -> input -> Workflow output
call selectedPlugin input = Workflow $ \runtime -> do
  let config = runtimeConfig runtime
      pluginName = internalPluginName selectedPlugin
      method = internalPluginMethod selectedPlugin
  pluginConfig <- case Map.lookup pluginName (Config.runtimePlugins config) of
    Nothing -> throwIO (UnknownPlugin pluginName)
    Just found -> pure found
  params <-
    either
      (throwIO . PluginParameterConflict ("plugin:" <> pluginName) method)
      pure
      ( augmentPluginParams
          (Config.runtimeWorkspace config)
          (Config.pluginOptions pluginConfig)
          (internalEncodePluginInput selectedPlugin input)
      )
  result <-
    callPlugin
      runtime
      ("plugin:" <> pluginName)
      (Config.pluginCommand pluginConfig)
      method
      params
  recordRuntime
    runtime
    (PluginValueRecord (pluginCallId result) pluginName method (pluginCallValue result))
  case internalDecodePluginOutput selectedPlugin (pluginCallValue result) of
    Left message -> throwIO (PluginDecodeFailed pluginName method message)
    Right value -> pure value

augmentPluginParams :: FilePath -> Object -> Value -> Either [Text] Object
augmentPluginParams workspace options input =
  let commonParams =
        KeyMap.fromList
          [ ("workspace", toJSON workspace),
            ("options", Object options)
          ]
   in case input of
        Object inputFields ->
          case [Key.toText field | field <- ["workspace", "options"], KeyMap.member field inputFields] of
            [] -> Right (KeyMap.union commonParams inputFields)
            collisions -> Left collisions
        other -> Right (KeyMap.insert "input" other commonParams)

parallel :: Workflow left -> Workflow right -> Workflow (left, right)
parallel leftWorkflow rightWorkflow = Workflow $ \runtime ->
  concurrently
    (executeWorkflow leftWorkflow runtime)
    (executeWorkflow rightWorkflow runtime)

parallelAll :: Traversable collection => collection (Workflow value) -> Workflow (collection value)
parallelAll workflows = Workflow $ \runtime ->
  mapConcurrently (\workflow -> executeWorkflow workflow runtime) workflows

-- | Run a homogeneous collection with at most the requested number of active
-- branches.  'mapConcurrently' preserves traversal order and cancels sibling
-- branches when one throws; the semaphore only bounds active work and is
-- released safely on synchronous failure or asynchronous cancellation.
parallelAllBounded :: Traversable collection => Int -> collection (Workflow value) -> Workflow (collection value)
parallelAllBounded maximumConcurrency workflows
  | maximumConcurrency <= 0 = Workflow $ \_ -> throwIO (RequirementFailed "parallelAllBounded concurrency must be positive")
  | otherwise = Workflow $ \runtime -> do
      permits <- newQSem maximumConcurrency
      mapConcurrently
        ( \workflow ->
            bracket_
              (waitQSem permits)
              (signalQSem permits)
              (executeWorkflow workflow runtime)
        )
        workflows

require :: (value -> Bool) -> value -> Workflow value
require = requireBecause "predicate returned false"

requireBecause :: Text -> (value -> Bool) -> value -> Workflow value
requireBecause message predicate value =
  if predicate value
    then pure value
    else Workflow $ \_ -> throwIO (RequirementFailed message)

-- | Capture ordinary workflow failures as values without swallowing
-- asynchronous cancellation.
attempt :: Workflow value -> Workflow (Either WorkflowError value)
attempt workflow = Workflow $ \runtime -> try (executeWorkflow workflow runtime)

-- | Run a workflow and boundedly flush its orthogonal record projection before
-- returning.  Presentation and diagnostic projection are observations: a sink
-- failure never replaces either a successful value or a known workflow/plugin
-- failure.  The sink failure remains available as an internal runtime record.
-- Asynchronous cancellation is rethrown immediately.
runWorkflow :: forall value. Runtime -> Workflow value -> IO value
runWorkflow runtime workflow = do
  workflowId <- freshRuntimeId runtime
  recordWorkflowTransition
    runtime
    workflowId
    "ready"
    "running"
    RequestTrigger
    "workflow.request.accepted"
    "clef.runWorkflow"
    "Workflow started."
    "The runtime is initialized and the workflow value is ready."
    "Clef accepted the workflow execution request."
    mempty
  outcome <- try (executeWorkflow workflow runtime) :: IO (Either SomeException value)
  case outcome of
    Left exception
      | isAsynchronousException exception -> do
        recordWorkflowTransition
          runtime
          workflowId
          "running"
          "cancelled"
          ControlTrigger
          "workflow.control.cancelled"
          "runtime"
          "Workflow entered the cancelled state."
          "An asynchronous cancellation was received."
          "Cancellation has priority over continued workflow execution."
          mempty
        recordWorkflowMessage
          runtime
          workflowId
          "workflow.cancelled"
          WarningLevel
          "Workflow execution was cancelled before it produced a terminal value."
          mempty
        throwIO exception
      | otherwise -> do
        case fromException exception :: Maybe WorkflowError of
          Just workflowError ->
            let (stateAfter, message) = case workflowError of
                  PluginOutcomeUnknown {} ->
                    ( "outcome_unknown",
                      "Workflow entered the outcome-unknown state."
                    )
                  _ -> ("failed", "Workflow entered the failed state.")
             in recordWorkflowTransition
                  runtime
                  workflowId
                  "running"
                  stateAfter
                  InternalResultTrigger
                  "workflow.result.error"
                  "clef.workflow"
                  message
                  "Workflow execution returned a typed failure."
                  "The failure determines the workflow terminal state."
                  (KeyMap.singleton "error" (workflowErrorDiagnostic workflowError))
          Nothing ->
            recordWorkflowTransition
              runtime
              workflowId
              "running"
              "failed"
              InternalResultTrigger
              "workflow.result.exception"
              "haskell.runtime"
              "Workflow entered the failed state after an unexpected runtime error."
              "Workflow execution raised an untyped exception."
              "Clef cannot produce a successful value after the exception."
              (KeyMap.singleton "exception" (toJSON (displayException exception)))
        _ <- flushRuntimeSink runtime
        throwIO exception
    Right value -> do
      recordWorkflowTransition
        runtime
        workflowId
        "running"
        "succeeded"
        InternalResultTrigger
        "workflow.result.success"
        "clef.workflow"
        "Workflow completed successfully."
        "Workflow execution returned a typed value."
        "The typed value is the authoritative terminal result."
        mempty
      _ <- flushRuntimeSink runtime
      pure value

recordWorkflowTransition :: Runtime -> Text -> Text -> Text -> TriggerKind -> Text -> Text -> Text -> Text -> Text -> Object -> IO ()
recordWorkflowTransition runtime workflowId stateBefore stateAfter triggerKind triggerCode triggerSource message condition reason context =
  recordRuntime
    runtime
    ( RuntimeTransitionRecord
        ( RuntimeStateTransition
            { stateTransitionCode = triggerCode,
              stateTransitionMessage = message,
              stateTransitionSubject = workflowId,
              stateTransitionStateBefore = stateBefore,
              stateTransitionTrigger =
                TransitionTrigger
                  { transitionTriggerKind = triggerKind,
                    transitionTriggerSource = triggerSource,
                    transitionTriggerCode = triggerCode,
                    transitionTriggerDetails = Nothing
                  },
              stateTransitionGuard =
                TransitionGuard
                  { transitionGuardCondition = condition,
                    transitionGuardPassed = True,
                    transitionGuardReason = reason
                  },
              stateTransitionStateAfter = stateAfter,
              stateTransitionContext = context
            }
        )
    )

recordWorkflowMessage :: Runtime -> Text -> Text -> PresentationLevel -> Text -> Object -> IO ()
recordWorkflowMessage runtime workflowId messageCode level message context =
  recordRuntime
    runtime
    ( RuntimeMessageRecord
        RuntimeMessage
          { runtimeMessageCode = messageCode,
            runtimeMessageLevel = level,
            runtimeMessageText = message,
            runtimeMessageContext = KeyMap.insert "workflow_id" (toJSON workflowId) context
          }
    )

-- | Standard executable entry point.  Synchronous failures are rendered once
-- as concise tagged natural language and terminate without exposing JSON or
-- GHC's uncaught-exception call stack.  Asynchronous exceptions are always
-- rethrown.  Library users that need exception semantics should use
-- 'runWorkflow' or 'runTactusWithRecords'.
runTactus :: forall value. Workflow value -> IO value
runTactus workflow = do
  runtimeOutcome <-
    try (Config.loadRuntimeConfigFromEnv >>= newRuntime) :: IO (Either SomeException Runtime)
  case runtimeOutcome of
    Left exception -> handleTactusException Nothing exception
    Right runtime -> do
      let execute = do
            outcome <- try (runWorkflow runtime workflow) :: IO (Either SomeException value)
            case outcome of
              Right value -> pure value
              Left exception -> handleTactusException (Just runtime) exception
      finally execute (closeRuntime runtime)

handleTactusException :: Maybe Runtime -> SomeException -> IO value
handleTactusException maybeRuntime exception =
  if isAsynchronousException exception
    then throwIO exception
    else case fromException exception :: Maybe WorkflowError of
      Just workflowError -> do
        alreadyPresented <- case maybeRuntime of
          Nothing -> pure False
          Just runtime -> workflowErrorWasPresented workflowError <$> readRuntimeRecords runtime
        unless alreadyPresented (presentWorkflowError workflowError)
        exitFailure
      Nothing -> do
        writeRuntimePresentation
          ( renderPresentationLine
              ErrorLevel
              "Workflow execution stopped because of an unexpected Haskell runtime error. Inspect the diagnostic record for technical details."
          )
        exitFailure

presentWorkflowError :: WorkflowError -> IO ()
presentWorkflowError workflowError =
  writeRuntimePresentation
    (renderPresentationLine level (renderWorkflowError workflowError))
  where
    level = case workflowError of
      PluginOutcomeUnknown {} -> WarningLevel
      _ -> ErrorLevel

workflowErrorWasPresented :: WorkflowError -> [RuntimeRecord] -> Bool
workflowErrorWasPresented workflowError records =
  not sinkFailed
    && case workflowError of
      PluginOutcomeUnknown {} -> hasMessageCode "plugin.outcome_unknown"
      PluginReportedFailure {} -> hasMessageCode "plugin.failure.reported"
      _ -> False
  where
    sinkFailed = hasInternalDiagnostic "runtime.sink_failed"
    hasMessageCode expected =
      any
        (\record -> case record of
          RuntimeMessageRecord message -> runtimeMessageCode message == expected
          _ -> False
        )
        records
    hasInternalDiagnostic expected =
      any
        (\record -> case record of
          RuntimeInternalDiagnosticRecord message -> runtimeMessageCode message == expected
          _ -> False
        )
        records

-- | Retain provider events and effect evidence on both successful and failed
-- workflow outcomes.  Invalid runtime configuration still fails before a
-- runtime (and therefore a record store) exists.
runTactusWithRecords :: Workflow value -> IO (Either WorkflowError value, [RuntimeRecord])
runTactusWithRecords workflow = do
  config <- Config.loadRuntimeConfigFromEnv
  runtime <- newRuntime config
  let execute = do
        outcome <- try (runWorkflow runtime workflow)
        records <- readRuntimeRecords runtime
        pure (outcome, records)
  finally execute (closeRuntime runtime)

invokeTask :: Runtime -> ProviderRef -> Task input output -> input -> IO output
invokeTask runtime selectedProvider selectedTask input = do
  let config = runtimeConfig runtime
      providerName = providerRefName selectedProvider
  providerConfig <- case Map.lookup providerName (Config.runtimeProviders config) of
    Nothing -> throwIO (UnknownProvider providerName)
    Just found -> pure found
  let renderedPrompt = internalRenderPrompt selectedTask input
      prompt = prependInstructions (Config.runtimeInstructions config) renderedPrompt
      model = providerRefModel selectedProvider <|> Config.providerModel providerConfig
      effort = providerRefEffort selectedProvider <|> Config.providerEffort providerConfig
      options = KeyMap.union (providerRefOptions selectedProvider) (Config.providerOptions providerConfig)
      params =
        KeyMap.fromList . catMaybes $
          [ Just ("task", toJSON (internalTaskName selectedTask)),
            Just ("prompt", toJSON prompt),
            Just ("workspace", toJSON (Config.runtimeWorkspace config)),
            ("model",) . toJSON <$> model,
            ("effort",) . toJSON <$> effort,
            Just ("options", Object options),
            if null (providerRefExtraArgs selectedProvider)
              then Nothing
              else Just ("extra_args", toJSON (providerRefExtraArgs selectedProvider))
          ]
      context =
        object
          [ "provider" .= providerName,
            "task" .= internalTaskName selectedTask
          ]
  pluginResult <-
    withInvocationObservers runtime context $ do
      result <-
        callPlugin
          runtime
          ("provider:" <> providerName)
          (Config.providerCommand providerConfig)
          "invoke"
          params
      recordRuntime
        runtime
        (ProviderValueRecord (pluginCallId result) providerName (pluginCallValue result))
      pure result
  text <- case parseEither (.: "text") =<< asObject (pluginCallValue pluginResult) of
    Left message -> throwIO (TaskDecodeFailed (internalTaskName selectedTask) (Text.pack message))
    Right value -> pure value
  either throwIO pure (decodeTaskResult selectedTask text)

prependInstructions :: Text -> Text -> Text
prependInstructions instructions prompt
  | Text.null instructions = prompt
  | Text.null prompt = instructions
  | otherwise = instructions <> "\n\n" <> prompt

asObject :: Value -> Either String Object
asObject (Object objectValue) = Right objectValue
asObject _ = Left "provider result.value must be an object containing a text field"

data ActiveObserver = ActiveObserver
  { activeObserverName :: Text,
    activeObserverConfig :: Config.EffectConfig,
    activeObserverBegin :: PluginCallResult
  }

withInvocationObservers :: forall value. Runtime -> Value -> IO value -> IO value
withInvocationObservers runtime invocationContext action = mask $ \restore -> do
  invocationId <- freshRuntimeId runtime
  activeObservers <- beginObservers runtime invocationId invocationContext
  actionResult <- try (restore action) :: IO (Either SomeException value)
  let outcome = case actionResult of
        Left _ -> "error"
        Right _ -> "ok"
  endErrors <- endObservers runtime invocationId invocationContext outcome activeObservers
  case actionResult of
    Left exception -> throwIO exception
    Right value -> case endErrors of
      [] -> pure value
      firstError : _ -> throwIO firstError

beginObservers :: Runtime -> Text -> Value -> IO [ActiveObserver]
beginObservers runtime invocationId invocationContext = go [] configuredObservers
  where
    config = runtimeConfig runtime
    configuredObservers =
      filter (Config.effectObserveInvocations . snd) . Map.toAscList $ Config.runtimeEffects config

    go active [] = pure (reverse active)
    go active ((effectName, effectConfig) : remaining) = do
      beginResult <-
        try
          ( callPlugin
              runtime
              ("effect:" <> effectName)
              (Config.effectCommand effectConfig)
              "observe.begin"
              ( KeyMap.fromList
                  [ "workspace" .= Config.runtimeWorkspace config,
                    "options" .= Object (Config.effectOptions effectConfig),
                    "invocation" .= invocationId,
                    "context" .= invocationContext
                  ]
              )
          )
      case beginResult of
        Left (exception :: SomeException) -> do
          _ <- endObservers runtime invocationId invocationContext "begin_error" (reverse active)
          throwIO exception
        Right pluginResult -> do
          let newlyActive = ActiveObserver effectName effectConfig pluginResult
          recordResult <-
            try
              ( recordRuntime
                  runtime
                  ( EffectEvidenceRecord
                      (pluginCallId pluginResult)
                      effectName
                      "observe.begin"
                      (pluginCallValue pluginResult)
                  )
              )
          case recordResult of
            Left (exception :: SomeException) -> do
              _ <- endObservers runtime invocationId invocationContext "begin_error" (reverse (newlyActive : active))
              throwIO exception
            Right () -> go (newlyActive : active) remaining

endObservers :: Runtime -> Text -> Value -> Text -> [ActiveObserver] -> IO [SomeException]
endObservers runtime invocationId invocationContext outcome activeObservers = do
  results <- mapM endOne (reverse activeObservers)
  pure [exception | Left exception <- results]
  where
    config = runtimeConfig runtime
    endOne activeObserver = do
      let effectName = activeObserverName activeObserver
          effectConfig = activeObserverConfig activeObserver
      try $ do
        pluginResult <-
          callPlugin
            runtime
            ("effect:" <> effectName)
            (Config.effectCommand effectConfig)
            "observe.end"
            ( KeyMap.fromList
                [ "workspace" .= Config.runtimeWorkspace config,
                  "options" .= Object (Config.effectOptions effectConfig),
                  "invocation" .= invocationId,
                  "context" .= invocationContext,
                  "outcome" .= outcome,
                  "begin" .= pluginCallValue (activeObserverBegin activeObserver)
                ]
            )
        recordRuntime
          runtime
          ( EffectEvidenceRecord
              (pluginCallId pluginResult)
              effectName
              "observe.end"
              (pluginCallValue pluginResult)
          )
