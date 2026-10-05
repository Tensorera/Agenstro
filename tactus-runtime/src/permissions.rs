//! Explicit invocation requests and evidence of the fields a provider applies.
use std::{fmt, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationPermissions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_skills: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<SandboxRequest>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxRequest {
    pub mode: InvocationSandboxMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InvocationSandboxMode {
    ReadOnly,
    WorkspaceWrite,
    Unrestricted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionField {
    AllowedTools,
    AllowedSkills,
    Sandbox,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionValidationError {
    pub field: PermissionField,
    pub message: String,
}

impl fmt::Display for PermissionValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for PermissionValidationError {}

impl InvocationPermissions {
    pub fn validate(&self) -> Result<(), PermissionValidationError> {
        for (field, names) in [
            (PermissionField::AllowedTools, &self.allowed_tools),
            (PermissionField::AllowedSkills, &self.allowed_skills),
        ] {
            if names.as_ref().is_some_and(|names| {
                names
                    .iter()
                    .any(|name| name.trim().is_empty() || name.contains('\0'))
            }) {
                return Err(PermissionValidationError {
                    field,
                    message: "permission entries must be non-empty names or paths without NUL"
                        .into(),
                });
            }
        }
        if self
            .sandbox
            .as_ref()
            .and_then(|s| s.workspace.as_ref())
            .is_some_and(|path| !path.is_absolute())
        {
            return Err(PermissionValidationError {
                field: PermissionField::Sandbox,
                message: "sandbox.workspace must be an absolute path".into(),
            });
        }
        if self
            .sandbox
            .as_ref()
            .and_then(|s| s.workspace.as_ref())
            .is_some_and(|path| path.as_os_str().to_string_lossy().contains('\0'))
        {
            return Err(PermissionValidationError {
                field: PermissionField::Sandbox,
                message: "sandbox.workspace must not contain NUL".into(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionSupport {
    Supported,
    Partial,
    #[default]
    Unsupported,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionCapability {
    #[serde(default)]
    pub support: PermissionSupport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modes: Option<Vec<InvocationSandboxMode>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<bool>,
}
impl PermissionCapability {
    pub fn supported(scope: &str) -> Self {
        Self {
            support: PermissionSupport::Supported,
            scope: Some(scope.into()),
            ..Self::default()
        }
    }
    pub fn partial(scope: &str, reason: &str) -> Self {
        Self {
            support: PermissionSupport::Partial,
            scope: Some(scope.into()),
            reason: Some(reason.into()),
            ..Self::default()
        }
    }
    pub fn unsupported(reason: &str) -> Self {
        Self {
            reason: Some(reason.into()),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionCapabilities {
    #[serde(default)]
    pub allowed_tools: PermissionCapability,
    #[serde(default)]
    pub allowed_skills: PermissionCapability,
    #[serde(default)]
    pub sandbox: PermissionCapability,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionApplicationStatus {
    Applied,
    Partial,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionApplication {
    pub field: PermissionField,
    pub status: PermissionApplicationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionWarning {
    pub code: String,
    pub field: PermissionField,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionReport {
    pub requested: InvocationPermissions,
    pub effective: InvocationPermissions,
    pub applications: Vec<PermissionApplication>,
    pub warnings: Vec<PermissionWarning>,
}
impl PermissionReport {
    pub fn new(requested: InvocationPermissions) -> Self {
        Self {
            requested,
            effective: InvocationPermissions::default(),
            applications: Vec::new(),
            warnings: Vec::new(),
        }
    }
    pub fn record(
        &mut self,
        field: PermissionField,
        status: PermissionApplicationStatus,
        effective: &InvocationPermissions,
        scope: Option<&str>,
        reason: Option<&str>,
    ) {
        match field {
            PermissionField::AllowedTools => self
                .effective
                .allowed_tools
                .clone_from(&effective.allowed_tools),
            PermissionField::AllowedSkills => self
                .effective
                .allowed_skills
                .clone_from(&effective.allowed_skills),
            PermissionField::Sandbox => self.effective.sandbox.clone_from(&effective.sandbox),
        }
        self.applications
            .retain(|application| application.field != field);
        self.applications.push(PermissionApplication {
            field,
            status,
            scope: scope.map(str::to_owned),
            reason: reason.map(str::to_owned),
        });
    }
    pub fn warn(&mut self, field: PermissionField, code: &str, message: impl Into<String>) {
        self.warnings.push(PermissionWarning {
            code: code.into(),
            field,
            message: message.into(),
        });
    }
    pub fn unsupported(&mut self, field: PermissionField, reason: impl Into<String>) {
        let reason = reason.into();
        self.record(
            field,
            PermissionApplicationStatus::Unsupported,
            &InvocationPermissions::default(),
            None,
            Some(&reason),
        );
        self.warn(field, "permission_unsupported", reason);
    }
    pub fn partial(
        &mut self,
        field: PermissionField,
        effective: &InvocationPermissions,
        scope: &str,
        reason: impl Into<String>,
    ) {
        let reason = reason.into();
        self.record(
            field,
            PermissionApplicationStatus::Partial,
            effective,
            Some(scope),
            Some(&reason),
        );
        self.warn(field, "permission_partial", reason);
    }
}
