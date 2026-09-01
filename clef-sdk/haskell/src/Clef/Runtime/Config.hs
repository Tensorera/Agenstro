{-# LANGUAGE OverloadedStrings #-}

module Clef.Runtime.Config
  ( RuntimeConfig (..),
    RuntimeLimits (..),
    defaultRuntimeLimits,
    validateRuntimeLimits,
    ProviderConfig (..),
    EffectConfig (..),
    PluginConfig (..),
    decodeRuntimeConfig,
    loadRuntimeConfig,
    loadRuntimeConfigFromEnv,
    providerDispatchDeadlineSeconds,
    providerDispatchDeadlineSecondsFor,
  )
where

import Control.Exception (IOException, throwIO, try)
import Control.Monad (unless, when)
import Data.Aeson
  ( FromJSON (parseJSON),
    Object,
    withObject,
    (.:),
    (.:?),
    (.!=),
  )
import qualified Data.ByteString as ByteString
import Data.Map.Strict (Map)
import qualified Data.Map.Strict as Map
import Data.Text (Text)
import qualified Data.Text as Text
import System.Environment (lookupEnv)
import System.FilePath (isAbsolute)
import Clef.Error (WorkflowError (..))
import Clef.Plugin.Protocol (decodeStrictJSON)

data RuntimeConfig = RuntimeConfig
  { runtimeApi :: Text,
    runtimeWorkspace :: FilePath,
    runtimeDefaultProvider :: Text,
    runtimeProviders :: Map Text ProviderConfig,
    runtimeEffects :: Map Text EffectConfig,
    runtimePlugins :: Map Text PluginConfig,
    runtimeInstructions :: Text,
    runtimeLimits :: RuntimeLimits
  }
  deriving (Eq, Show)

-- | Workspace-owned process and concurrency policy.  The JSON names are
-- deliberately shared with Tactus' optional @limits@ object.  Missing fields
-- retain the pre-limits Clef defaults, so older runtime documents remain
-- valid.
data RuntimeLimits = RuntimeLimits
  { limitMaxConcurrentProviderCalls :: Int,
    limitPluginTimeoutSeconds :: Int,
    -- | Inner Tactus provider-dispatch deadline.
    limitProviderTimeoutSeconds :: Int,
    -- | Outer Clef provider transport deadline, including cleanup headroom.
    limitProviderOuterTimeoutSeconds :: Int,
    limitMaxRequestBytes :: Int,
    limitMaxFrameBytes :: Int,
    limitMaxStdoutBytes :: Int,
    limitMaxEventFrames :: Int,
    limitMaxStderrBytes :: Int
  }
  deriving (Eq, Show)

defaultRuntimeLimits :: RuntimeLimits
defaultRuntimeLimits =
  RuntimeLimits
    { limitMaxConcurrentProviderCalls = 4,
      limitPluginTimeoutSeconds = 60 * 60,
      limitProviderTimeoutSeconds = providerDispatchDeadlineSeconds,
      limitProviderOuterTimeoutSeconds = providerDispatchDeadlineSeconds + 15 * 60,
      limitMaxRequestBytes = 1024 * 1024,
      limitMaxFrameBytes = 1024 * 1024,
      limitMaxStdoutBytes = 64 * 1024 * 1024,
      limitMaxEventFrames = 10000,
      limitMaxStderrBytes = 1024 * 1024
    }

data ProviderConfig = ProviderConfig
  { providerCommand :: [Text],
    providerModel :: Maybe Text,
    providerEffort :: Maybe Text,
    providerOptions :: Object
  }
  deriving (Eq, Show)

data EffectConfig = EffectConfig
  { effectCommand :: [Text],
    effectOptions :: Object,
    effectObserveInvocations :: Bool
  }
  deriving (Eq, Show)

-- | Configuration for an open, application-defined plugin.  Clef assigns no
-- provider or effect semantics to entries in this registry.
data PluginConfig = PluginConfig
  { pluginCommand :: [Text],
    pluginOptions :: Object
  }
  deriving (Eq, Show)

instance FromJSON RuntimeConfig where
  parseJSON = withObject "Clef runtime configuration" $ \objectValue -> do
    api <- objectValue .: "api"
    unless (api == "clef.runtime/v1") $ fail "api must be exactly clef.runtime/v1"
    workspace <- objectValue .: "workspace"
    unless (isAbsolute workspace) $ fail "workspace must be an absolute path"
    defaultProvider <- objectValue .: "default_provider"
    providers <- objectValue .: "providers"
    effects <- objectValue .: "effects"
    plugins <- objectValue .:? "plugins" .!= Map.empty
    instructions <- objectValue .: "instructions"
    limits <- objectValue .:? "limits" .!= defaultRuntimeLimits
    when (Text.null defaultProvider) $ fail "default_provider must not be empty"
    unless (Map.member defaultProvider providers) $
      fail "default_provider must name an entry in providers"
    pure
      RuntimeConfig
        { runtimeApi = api,
          runtimeWorkspace = workspace,
          runtimeDefaultProvider = defaultProvider,
          runtimeProviders = providers,
          runtimeEffects = effects,
          runtimePlugins = plugins,
          runtimeInstructions = instructions,
          runtimeLimits = limits
        }

instance FromJSON RuntimeLimits where
  parseJSON = withObject "runtime limits" $ \objectValue -> do
    maxConcurrentProviderCalls <-
      objectValue .:? "max_concurrent_provider_calls" .!= limitMaxConcurrentProviderCalls defaultRuntimeLimits
    pluginTimeoutSeconds <-
      objectValue .:? "plugin_timeout_seconds" .!= limitPluginTimeoutSeconds defaultRuntimeLimits
    providerTimeoutSeconds <-
      objectValue .:? "provider_timeout_seconds" .!= limitProviderTimeoutSeconds defaultRuntimeLimits
    configuredProviderOuterTimeout <- objectValue .:? "provider_outer_timeout_seconds"
    let providerOuterTimeoutSeconds =
          maybe
            (providerTimeoutSeconds + providerCleanupHeadroomSeconds providerTimeoutSeconds)
            id
            configuredProviderOuterTimeout
    maxRequestBytes <- objectValue .:? "max_request_bytes" .!= limitMaxRequestBytes defaultRuntimeLimits
    maxFrameBytes <- objectValue .:? "max_frame_bytes" .!= limitMaxFrameBytes defaultRuntimeLimits
    maxStdoutBytes <- objectValue .:? "max_stdout_bytes" .!= limitMaxStdoutBytes defaultRuntimeLimits
    maxEventFrames <- objectValue .:? "max_event_frames" .!= limitMaxEventFrames defaultRuntimeLimits
    maxStderrBytes <- objectValue .:? "max_stderr_bytes" .!= limitMaxStderrBytes defaultRuntimeLimits
    let limits =
          RuntimeLimits
            { limitMaxConcurrentProviderCalls = maxConcurrentProviderCalls,
              limitPluginTimeoutSeconds = pluginTimeoutSeconds,
              limitProviderTimeoutSeconds = providerTimeoutSeconds,
              limitProviderOuterTimeoutSeconds = providerOuterTimeoutSeconds,
              limitMaxRequestBytes = maxRequestBytes,
              limitMaxFrameBytes = maxFrameBytes,
              limitMaxStdoutBytes = maxStdoutBytes,
              limitMaxEventFrames = maxEventFrames,
              limitMaxStderrBytes = maxStderrBytes
            }
    either (fail . Text.unpack) pure (validateRuntimeLimits limits)

instance FromJSON ProviderConfig where
  parseJSON = withObject "provider configuration" $ \objectValue -> do
    command <- objectValue .: "command"
    when (null command) $ fail "provider command must contain an executable"
    ProviderConfig
      <$> pure command
      <*> objectValue .:? "model"
      <*> objectValue .:? "effort"
      <*> objectValue .:? "options" .!= mempty

instance FromJSON EffectConfig where
  parseJSON = withObject "effect configuration" $ \objectValue -> do
    command <- objectValue .: "command"
    when (null command) $ fail "effect command must contain an executable"
    EffectConfig
      <$> pure command
      <*> objectValue .:? "options" .!= mempty
      <*> objectValue .:? "observe_invocations" .!= False

instance FromJSON PluginConfig where
  parseJSON = withObject "plugin configuration" $ \objectValue -> do
    command <- objectValue .: "command"
    when (null command) $ fail "plugin command must contain an executable"
    PluginConfig
      <$> pure command
      <*> objectValue .:? "options" .!= mempty

decodeRuntimeConfig :: ByteString.ByteString -> Either WorkflowError RuntimeConfig
decodeRuntimeConfig bytes =
  case decodeStrictJSON bytes of
    Left message -> Left (RuntimeConfigError (Text.pack message))
    Right config -> Right config

loadRuntimeConfig :: FilePath -> IO RuntimeConfig
loadRuntimeConfig path = do
  result <- try (ByteString.readFile path) :: IO (Either IOException ByteString.ByteString)
  bytes <- case result of
    Left exception ->
      failWith . RuntimeConfigError $
        "cannot read '" <> Text.pack path <> "': " <> Text.pack (show exception)
    Right contents -> pure contents
  either failWith pure (decodeRuntimeConfig bytes)
  where
    failWith :: WorkflowError -> IO a
    failWith = throwIO

loadRuntimeConfigFromEnv :: IO RuntimeConfig
loadRuntimeConfigFromEnv = do
  configuredPath <- lookupEnv "TACTUS_RUNTIME_CONFIG"
  case configuredPath of
    Nothing -> throwIO $ RuntimeConfigError "TACTUS_RUNTIME_CONFIG is not set"
    Just path -> ensureProviderDispatchDeadline <$> loadRuntimeConfig path

-- | The inner Tactus dispatch deadline.  Clef's provider transport supervisor
-- allows a further fifteen minutes for dispatch to terminate its process tree,
-- flush a terminal frame, and exit before the outer boundary is reached.
providerDispatchDeadlineSeconds :: Int
providerDispatchDeadlineSeconds = 3 * 60 * 60 + 45 * 60

-- | Resolve the inner dispatch deadline from one parsed runtime policy.
providerDispatchDeadlineSecondsFor :: RuntimeLimits -> Int
providerDispatchDeadlineSecondsFor = limitProviderTimeoutSeconds

providerCleanupHeadroomSeconds :: Int -> Int
providerCleanupHeadroomSeconds providerSeconds =
  min (15 * 60) (max 1 (providerSeconds `div` 4))

validateRuntimeLimits :: RuntimeLimits -> Either Text RuntimeLimits
validateRuntimeLimits limits = do
  requireRange "max_concurrent_provider_calls" 1 32 (limitMaxConcurrentProviderCalls limits)
  requireRange "plugin_timeout_seconds" 1 maxTimeoutSeconds (limitPluginTimeoutSeconds limits)
  requireRange "provider_timeout_seconds" 1 maxTimeoutSeconds (limitProviderTimeoutSeconds limits)
  requireRange "provider_outer_timeout_seconds" 1 maxTimeoutSeconds (limitProviderOuterTimeoutSeconds limits)
  unlessEither
    (limitProviderOuterTimeoutSeconds limits >= limitProviderTimeoutSeconds limits + 60)
    "limits.provider_outer_timeout_seconds must leave at least 60 seconds after provider_timeout_seconds"
  requireRange "max_request_bytes" 1 (16 * 1024 * 1024) (limitMaxRequestBytes limits)
  requireRange "max_frame_bytes" 1 (32 * 1024 * 1024) (limitMaxFrameBytes limits)
  requireRange "max_stdout_bytes" (limitMaxFrameBytes limits) (512 * 1024 * 1024) (limitMaxStdoutBytes limits)
  requireRange "max_event_frames" 1 1000000 (limitMaxEventFrames limits)
  requireRange "max_stderr_bytes" 1 (16 * 1024 * 1024) (limitMaxStderrBytes limits)
  pure limits
  where
    maxTimeoutSeconds = 7 * 24 * 60 * 60

requireRange :: Text -> Int -> Int -> Int -> Either Text ()
requireRange name minimumValue maximumValue actual
  | actual < minimumValue || actual > maximumValue =
      Left
        ( "limits."
            <> name
            <> " must be between "
            <> Text.pack (show minimumValue)
            <> " and "
            <> Text.pack (show maximumValue)
            <> ", received "
            <> Text.pack (show actual)
        )
  | otherwise = Right ()

unlessEither :: Bool -> Text -> Either Text ()
unlessEither True _ = Right ()
unlessEither False message = Left message

ensureProviderDispatchDeadline :: RuntimeConfig -> RuntimeConfig
ensureProviderDispatchDeadline config =
  config
    { runtimeProviders = fmap addDispatchDeadline (runtimeProviders config)
    }
  where
    addDispatchDeadline provider =
      provider
        { providerCommand = addDeadlineArgument (providerCommand provider)
        }
    addDeadlineArgument command
      | "dispatch" `elem` command && not (any isDeadlineArgument command) =
          command
            <> [ "--timeout-seconds",
                 Text.pack (show (providerDispatchDeadlineSecondsFor (runtimeLimits config)))
               ]
      | otherwise = command
    isDeadlineArgument argument =
      argument == "--timeout-seconds"
        || "--timeout-seconds=" `Text.isPrefixOf` argument
