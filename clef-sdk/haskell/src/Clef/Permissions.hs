{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE TupleSections #-}

-- | Explicit requests replace a provider's startup request as a whole. An
-- absent request inherits startup defaults; 'emptyPermissions' selects ordinary
-- provider behavior. Empty tool/skill lists are explicit empty selections.
module Clef.Permissions
  ( InvocationPermissions (..),
    emptyPermissions,
    SandboxRequest (..),
    InvocationSandboxMode (..),
    PermissionField (..),
    PermissionSupport (..),
    PermissionCapability (..),
    PermissionCapabilities (..),
    PermissionApplicationStatus (..),
    PermissionApplication (..),
    PermissionWarning (..),
    PermissionReport (..),
    validatePermissions,
    negotiatePermissions,
  )
where

import Control.Monad (unless)
import Data.Aeson
import qualified Data.Aeson.Key as Key
import qualified Data.Aeson.KeyMap as KeyMap
import Data.Aeson.Types (Parser)
import Data.Maybe (catMaybes)
import Data.Text (Text)
import qualified Data.Text as Text
import System.FilePath (isAbsolute)

data InvocationPermissions = InvocationPermissions
  { allowedTools :: Maybe [Text],
    allowedSkills :: Maybe [Text],
    invocationSandbox :: Maybe SandboxRequest
  }
  deriving (Eq, Show)

emptyPermissions :: InvocationPermissions
emptyPermissions = InvocationPermissions Nothing Nothing Nothing

data SandboxRequest = SandboxRequest
  { sandboxMode :: InvocationSandboxMode,
    sandboxWorkspace :: Maybe FilePath
  }
  deriving (Eq, Show)

data InvocationSandboxMode = ReadOnly | WorkspaceWrite | Unrestricted
  deriving (Eq, Show)

data PermissionField = AllowedTools | AllowedSkills | Sandbox
  deriving (Eq, Show)

data PermissionSupport = Supported | PartiallySupported | Unsupported
  deriving (Eq, Show)

data PermissionCapability = PermissionCapability
  { capabilitySupport :: PermissionSupport,
    capabilityScope :: Maybe Text,
    capabilityReason :: Maybe Text,
    capabilityModes :: Maybe [InvocationSandboxMode],
    capabilityWorkspace :: Maybe Bool
  }
  deriving (Eq, Show)

data PermissionCapabilities = PermissionCapabilities
  { toolsCapability :: PermissionCapability,
    skillsCapability :: PermissionCapability,
    sandboxCapability :: PermissionCapability
  }
  deriving (Eq, Show)

data PermissionApplicationStatus = Applied | PartiallyApplied | NotApplied
  deriving (Eq, Show)

data PermissionApplication = PermissionApplication
  { applicationField :: PermissionField,
    applicationStatus :: PermissionApplicationStatus,
    applicationScope :: Maybe Text,
    applicationReason :: Maybe Text
  }
  deriving (Eq, Show)

data PermissionWarning = PermissionWarning
  { permissionWarningCode :: Text,
    permissionWarningField :: PermissionField,
    permissionWarningMessage :: Text
  }
  deriving (Eq, Show)

data PermissionReport = PermissionReport
  { requestedPermissions :: InvocationPermissions,
    effectivePermissions :: InvocationPermissions,
    permissionApplications :: [PermissionApplication],
    permissionWarnings :: [PermissionWarning]
  }
  deriving (Eq, Show)

validatePermissions :: InvocationPermissions -> Either Text ()
validatePermissions request
  | any invalidName (maybe [] id (allowedTools request) <> maybe [] id (allowedSkills request)) =
      Left "permission entries must be non-empty names or paths without NUL"
  | Just path <- invocationSandbox request >>= sandboxWorkspace,
    not (isAbsolute path) || '\0' `elem` path = Left "sandbox.workspace must be an absolute path without NUL"
  | otherwise = Right ()
  where
    invalidName name = Text.null (Text.strip name) || Text.any (== '\0') name

instance ToJSON InvocationPermissions where
  toJSON request = object . catMaybes $
    [ ("allowed_tools" .=) <$> allowedTools request,
      ("allowed_skills" .=) <$> allowedSkills request,
      ("sandbox" .=) <$> invocationSandbox request
    ]

instance FromJSON InvocationPermissions where
  parseJSON = withObject "invocation permissions" $ \fields -> do
    rejectUnknown ["allowed_tools", "allowed_skills", "sandbox"] fields
    request <- InvocationPermissions <$> fields .:? "allowed_tools" <*> fields .:? "allowed_skills" <*> fields .:? "sandbox"
    either (fail . Text.unpack) (const (pure request)) (validatePermissions request)

instance ToJSON SandboxRequest where
  toJSON request = object (["mode" .= sandboxMode request] <> maybe [] (\path -> ["workspace" .= path]) (sandboxWorkspace request))

instance FromJSON SandboxRequest where
  parseJSON = withObject "sandbox request" $ \fields -> do
    rejectUnknown ["mode", "workspace"] fields
    SandboxRequest <$> fields .: "mode" <*> fields .:? "workspace"

instance ToJSON InvocationSandboxMode where
  toJSON mode = String $ case mode of ReadOnly -> "read-only"; WorkspaceWrite -> "workspace-write"; Unrestricted -> "unrestricted"

instance FromJSON InvocationSandboxMode where
  parseJSON = withText "sandbox mode" $ \mode -> case mode of
    "read-only" -> pure ReadOnly
    "workspace-write" -> pure WorkspaceWrite
    "unrestricted" -> pure Unrestricted
    _ -> fail "sandbox mode must be read-only, workspace-write, or unrestricted"

instance ToJSON PermissionField where
  toJSON field = String $ case field of AllowedTools -> "allowed_tools"; AllowedSkills -> "allowed_skills"; Sandbox -> "sandbox"

instance FromJSON PermissionField where
  parseJSON = withText "permission field" $ \field -> case field of
    "allowed_tools" -> pure AllowedTools
    "allowed_skills" -> pure AllowedSkills
    "sandbox" -> pure Sandbox
    _ -> fail "unknown permission field"

instance ToJSON PermissionSupport where
  toJSON support = String $ case support of Supported -> "supported"; PartiallySupported -> "partial"; Unsupported -> "unsupported"

instance FromJSON PermissionSupport where
  parseJSON = withText "permission support" $ \support -> case support of
    "supported" -> pure Supported
    "partial" -> pure PartiallySupported
    "unsupported" -> pure Unsupported
    _ -> fail "unknown permission support"

instance ToJSON PermissionCapability where
  toJSON capability = object (["support" .= capabilitySupport capability] <> catMaybes
    [ ("scope" .=) <$> capabilityScope capability, ("reason" .=) <$> capabilityReason capability,
      ("modes" .=) <$> capabilityModes capability, ("workspace" .=) <$> capabilityWorkspace capability ])

instance FromJSON PermissionCapability where
  parseJSON = withObject "permission capability" $ \fields ->
    PermissionCapability <$> fields .:? "support" .!= Unsupported <*> fields .:? "scope" <*> fields .:? "reason" <*> fields .:? "modes" <*> fields .:? "workspace"

unsupportedCapability :: PermissionCapability
unsupportedCapability = PermissionCapability Unsupported Nothing Nothing Nothing Nothing

instance ToJSON PermissionCapabilities where
  toJSON capabilities = object ["allowed_tools" .= toolsCapability capabilities, "allowed_skills" .= skillsCapability capabilities, "sandbox" .= sandboxCapability capabilities]

instance FromJSON PermissionCapabilities where
  parseJSON = withObject "permission capabilities" $ \fields ->
    PermissionCapabilities <$> fields .:? "allowed_tools" .!= unsupportedCapability <*> fields .:? "allowed_skills" .!= unsupportedCapability <*> fields .:? "sandbox" .!= unsupportedCapability

instance ToJSON PermissionApplicationStatus where
  toJSON status = String $ case status of Applied -> "applied"; PartiallyApplied -> "partial"; NotApplied -> "unsupported"

instance FromJSON PermissionApplicationStatus where
  parseJSON = withText "permission application status" $ \status -> case status of
    "applied" -> pure Applied
    "partial" -> pure PartiallyApplied
    "unsupported" -> pure NotApplied
    _ -> fail "unknown permission application status"

instance ToJSON PermissionApplication where
  toJSON application = object (["field" .= applicationField application, "status" .= applicationStatus application] <> catMaybes
    [ ("scope" .=) <$> applicationScope application, ("reason" .=) <$> applicationReason application ])

instance FromJSON PermissionApplication where
  parseJSON = withObject "permission application" $ \fields ->
    PermissionApplication <$> fields .: "field" <*> fields .: "status" <*> fields .:? "scope" <*> fields .:? "reason"

instance ToJSON PermissionWarning where
  toJSON warning = object ["code" .= permissionWarningCode warning, "field" .= permissionWarningField warning, "message" .= permissionWarningMessage warning]

instance FromJSON PermissionWarning where
  parseJSON = withObject "permission warning" $ \fields -> PermissionWarning <$> fields .: "code" <*> fields .: "field" <*> fields .: "message"

instance ToJSON PermissionReport where
  toJSON report = object ["requested" .= requestedPermissions report, "effective" .= effectivePermissions report, "applications" .= permissionApplications report, "warnings" .= permissionWarnings report]

instance FromJSON PermissionReport where
  parseJSON = withObject "permission report" $ \fields -> PermissionReport <$> fields .: "requested" <*> fields .: "effective" <*> fields .: "applications" <*> fields .: "warnings"

rejectUnknown :: [Text] -> Object -> Parser ()
rejectUnknown known fields = unless (all ((`elem` known) . Key.toText) (KeyMap.keys fields)) (fail "unknown permission request field")

-- | Negotiate only fields the plugin advertises. This records declared support;
-- the plugin owns native enforcement and may report narrower application itself.
negotiatePermissions :: InvocationPermissions -> PermissionCapabilities -> PermissionReport
negotiatePermissions requested capabilities = foldl select initial selections
  where
    initial = PermissionReport requested emptyPermissions [] []
    selections = catMaybes
      [ (AllowedTools, toolsCapability capabilities,) . (\names -> emptyPermissions {allowedTools = Just names}) <$> allowedTools requested,
        (AllowedSkills, skillsCapability capabilities,) . (\names -> emptyPermissions {allowedSkills = Just names}) <$> allowedSkills requested,
        (Sandbox, sandboxCapability capabilities,) . (\sandbox -> emptyPermissions {invocationSandbox = Just sandbox}) <$> invocationSandbox requested ]
    select report (field, capability, effective) =
      let sandboxSupported = case invocationSandbox effective of
            Nothing -> True
            Just sandbox -> maybe True (sandboxMode sandbox `elem`) (capabilityModes capability)
          workspacePartial = case invocationSandbox effective of
            Just sandbox -> sandboxWorkspace sandbox /= Nothing && capabilityWorkspace capability /= Just True
            Nothing -> False
          narrowed = if workspacePartial then effective {invocationSandbox = (\sandbox -> sandbox {sandboxWorkspace = Nothing}) <$> invocationSandbox effective} else effective
          supported = capabilitySupport capability /= Unsupported && sandboxSupported
          status = if not supported then NotApplied else if capabilitySupport capability == PartiallySupported || workspacePartial then PartiallyApplied else Applied
          reason = if not sandboxSupported then Just "The plugin does not advertise the requested sandbox mode."
            else if supported && workspacePartial then Just "The plugin supports the sandbox mode but not a custom sandbox workspace; its normal workspace is retained."
            else capabilityReason capability
          message = maybe "The plugin does not advertise support for this requested permission field; its ordinary configuration is retained." id reason
          warning = if status == Applied then [] else [PermissionWarning (if status == PartiallyApplied then "permission_partial" else "permission_unsupported") field message]
          merged = if supported then mergeField field narrowed (effectivePermissions report) else effectivePermissions report
       in report { effectivePermissions = merged,
            permissionApplications = permissionApplications report <> [PermissionApplication field status (capabilityScope capability) reason],
            permissionWarnings = permissionWarnings report <> warning }
    mergeField field source target = case field of
      AllowedTools -> target {allowedTools = allowedTools source}
      AllowedSkills -> target {allowedSkills = allowedSkills source}
      Sandbox -> target {invocationSandbox = invocationSandbox source}
