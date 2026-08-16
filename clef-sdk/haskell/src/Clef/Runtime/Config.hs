{-# LANGUAGE OverloadedStrings #-}

module Clef.Runtime.Config
  ( RuntimeConfig (..),
    ProviderConfig (..),
    EffectConfig (..),
    PluginConfig (..),
    decodeRuntimeConfig,
    loadRuntimeConfig,
    loadRuntimeConfigFromEnv,
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
    runtimeInstructions :: Text
  }
  deriving (Eq, Show)

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
          runtimeInstructions = instructions
        }

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
    Just path -> loadRuntimeConfig path
