{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE ScopedTypeVariables #-}
{-# LANGUAGE TupleSections #-}

module Clef.Workflow
  ( Workflow,
    Task,
    Operation,
    ProviderRef (..),
    providerRef,
    task,
    textTask,
    jsonTask,
    operation,
    decodeTaskResult,
    invoke,
    invokeWith,
    perform,
    parallel,
    parallelAll,
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
import Control.Exception
  ( SomeException,
    mask,
    throwIO,
    try,
  )
import Data.Aeson
  ( FromJSON (parseJSON),
    Object,
    ToJSON (toJSON),
    Value (Object),
    object,
    (.:),
    (.=),
  )
import qualified Data.Aeson.KeyMap as KeyMap
import Data.Aeson.Types (parseEither)
import qualified Data.Map.Strict as Map
import Data.Maybe (catMaybes)
import Data.Text (Text)
import qualified Data.Text as Text
import Data.Text.Encoding (encodeUtf8)
import Clef.Error (WorkflowError (..))
import Clef.Plugin.Protocol (decodeStrictJSON)
import Clef.Runtime
  ( PluginCallResult (..),
    Runtime,
    RuntimeRecord (..),
    callPlugin,
    freshRuntimeId,
    newRuntime,
    readRuntimeRecords,
    recordRuntime,
    runtimeConfig,
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
  let commonParams =
        KeyMap.fromList
          [ ("workspace", toJSON (Config.runtimeWorkspace config)),
            ("options", Object (Config.effectOptions effectConfig))
          ]
      -- Runtime-owned workspace/options win on key collisions.  Operation
      -- fields otherwise stay top-level for cross-language effect plugins.
      pluginParams = case internalEffectParams selectedOperation of
        Object operationFields -> Object (KeyMap.union commonParams operationFields)
        other -> Object (KeyMap.insert "input" other commonParams)
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

parallel :: Workflow left -> Workflow right -> Workflow (left, right)
parallel leftWorkflow rightWorkflow = Workflow $ \runtime ->
  concurrently
    (executeWorkflow leftWorkflow runtime)
    (executeWorkflow rightWorkflow runtime)

parallelAll :: Traversable collection => collection (Workflow value) -> Workflow (collection value)
parallelAll workflows = Workflow $ \runtime ->
  mapConcurrently (\workflow -> executeWorkflow workflow runtime) workflows

require :: (value -> Bool) -> value -> Workflow value
require = requireBecause "predicate returned false"

requireBecause :: Text -> (value -> Bool) -> value -> Workflow value
requireBecause message predicate value =
  if predicate value
    then pure value
    else Workflow $ \_ -> throwIO (RequirementFailed message)

-- | Embed arbitrary IO.  This is convenience, not a sandbox or a claim that
-- Clef can intercept every side effect performed by a Haskell script.
liftIO :: IO value -> Workflow value
liftIO action = Workflow $ \_ -> action

-- | Capture ordinary workflow failures as values without swallowing
-- asynchronous cancellation.
attempt :: Workflow value -> Workflow (Either WorkflowError value)
attempt workflow = Workflow $ \runtime -> try (executeWorkflow workflow runtime)

runWorkflow :: Runtime -> Workflow value -> IO value
runWorkflow runtime workflow = executeWorkflow workflow runtime

runTactus :: Workflow value -> IO value
runTactus workflow = do
  config <- Config.loadRuntimeConfigFromEnv
  runtime <- newRuntime config
  runWorkflow runtime workflow

-- | Retain provider events and effect evidence on both successful and failed
-- workflow outcomes.  Invalid runtime configuration still fails before a
-- runtime (and therefore a record store) exists.
runTactusWithRecords :: Workflow value -> IO (Either WorkflowError value, [RuntimeRecord])
runTactusWithRecords workflow = do
  config <- Config.loadRuntimeConfigFromEnv
  runtime <- newRuntime config
  outcome <- try (runWorkflow runtime workflow)
  records <- readRuntimeRecords runtime
  pure (outcome, records)

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
        Object . KeyMap.fromList . catMaybes $
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
              ( object
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
            ( object
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
