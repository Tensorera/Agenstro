use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf},
};

use thiserror::Error;

/// Hard maximum path components accepted by a project path policy.
pub const MAX_PATH_COMPONENTS: usize = 256;
/// Hard maximum encoded bytes in one native path component.
pub const MAX_PATH_COMPONENT_BYTES: u64 = 1_024;
/// Hard maximum encoded bytes in one project-relative path.
pub const MAX_PATH_BYTES: u64 = 32_768;

/// Invalid project path policy or relative path.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[non_exhaustive]
pub enum ProjectPathError {
    /// A policy limit was zero or above its hard maximum.
    #[error("invalid project path policy: {field}")]
    InvalidPolicy {
        /// Name of the invalid field.
        field: &'static str,
    },
    /// Empty, absolute, prefixed, current, or parent-relative path.
    #[error("path is not a project-relative normal-component path")]
    NotProjectRelative,
    /// A component contained a NUL or control code point.
    #[error("path component contains a forbidden control code point")]
    ForbiddenControl,
    /// A component violated native Windows device, ADS, or trailing-name rules.
    #[error("path component violates Windows native path policy")]
    InvalidWindowsComponent,
    /// Component count exceeded the selected policy.
    #[error("path component budget {maximum} exceeded")]
    ComponentBudgetExceeded {
        /// Maximum accepted components.
        maximum: usize,
    },
    /// One component exceeded its selected byte budget.
    #[error("path component byte budget {maximum} exceeded")]
    ComponentBytesExceeded {
        /// Maximum accepted encoded component bytes.
        maximum: u64,
    },
    /// Total path encoding exceeded its selected byte budget.
    #[error("path byte budget {maximum} exceeded")]
    PathBytesExceeded {
        /// Maximum accepted encoded path bytes.
        maximum: u64,
    },
}

/// Caller-selected native path limits under hard maxima.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PathPolicy {
    max_components: usize,
    max_component_bytes: u64,
    max_path_bytes: u64,
}

impl PathPolicy {
    /// Constructs a validated native project path policy.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectPathError::InvalidPolicy`] for zero or excessive limits.
    pub fn new(
        max_components: usize,
        max_component_bytes: u64,
        max_path_bytes: u64,
    ) -> Result<Self, ProjectPathError> {
        if max_components == 0 || max_components > MAX_PATH_COMPONENTS {
            return Err(ProjectPathError::InvalidPolicy {
                field: "component count",
            });
        }
        if max_component_bytes == 0 || max_component_bytes > MAX_PATH_COMPONENT_BYTES {
            return Err(ProjectPathError::InvalidPolicy {
                field: "component bytes",
            });
        }
        if max_path_bytes == 0 || max_path_bytes > MAX_PATH_BYTES {
            return Err(ProjectPathError::InvalidPolicy {
                field: "path bytes",
            });
        }
        Ok(Self {
            max_components,
            max_component_bytes,
            max_path_bytes,
        })
    }
}

/// A validated, opaque native path relative to a project root.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ProjectRelativePath(PathBuf);

impl ProjectRelativePath {
    /// Validates a native relative path without converting it to UTF-8.
    ///
    /// # Errors
    ///
    /// Rejects root escape, prefixes, NUL/control characters, policy excess,
    /// and Windows reserved-device/ADS/trailing-dot names on Windows.
    pub fn parse(path: impl AsRef<Path>, policy: PathPolicy) -> Result<Self, ProjectPathError> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() || path.is_absolute() {
            return Err(ProjectPathError::NotProjectRelative);
        }
        let mut components = 0_usize;
        let mut total_bytes = 0_u64;
        for component in path.components() {
            let Component::Normal(value) = component else {
                return Err(ProjectPathError::NotProjectRelative);
            };
            components =
                components
                    .checked_add(1)
                    .ok_or(ProjectPathError::ComponentBudgetExceeded {
                        maximum: policy.max_components,
                    })?;
            if components > policy.max_components {
                return Err(ProjectPathError::ComponentBudgetExceeded {
                    maximum: policy.max_components,
                });
            }
            validate_component(value)?;
            let bytes = os_str_bytes(value);
            if bytes > policy.max_component_bytes {
                return Err(ProjectPathError::ComponentBytesExceeded {
                    maximum: policy.max_component_bytes,
                });
            }
            total_bytes = total_bytes
                .checked_add(bytes)
                .and_then(|current| {
                    current.checked_add(if components > 1 { separator_bytes() } else { 0 })
                })
                .ok_or(ProjectPathError::PathBytesExceeded {
                    maximum: policy.max_path_bytes,
                })?;
            if total_bytes > policy.max_path_bytes {
                return Err(ProjectPathError::PathBytesExceeded {
                    maximum: policy.max_path_bytes,
                });
            }
        }
        if components == 0 {
            return Err(ProjectPathError::NotProjectRelative);
        }
        Ok(Self(path.to_path_buf()))
    }

    /// Borrows the native relative path.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

#[cfg(windows)]
fn validate_component(component: &OsStr) -> Result<(), ProjectPathError> {
    use std::os::windows::ffi::OsStrExt;

    if component
        .encode_wide()
        .any(|unit| unit == 0 || unit < 32 || unit == u16::from(b':'))
    {
        return Err(ProjectPathError::ForbiddenControl);
    }
    let display = component.to_string_lossy();
    if display.ends_with(['.', ' ']) || is_windows_reserved(&display) {
        return Err(ProjectPathError::InvalidWindowsComponent);
    }
    Ok(())
}

#[cfg(unix)]
fn validate_component(component: &OsStr) -> Result<(), ProjectPathError> {
    use std::os::unix::ffi::OsStrExt;

    if component
        .as_bytes()
        .iter()
        .any(|byte| *byte == 0 || *byte < 32)
    {
        return Err(ProjectPathError::ForbiddenControl);
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn validate_component(component: &OsStr) -> Result<(), ProjectPathError> {
    if component.to_string_lossy().chars().any(char::is_control) {
        return Err(ProjectPathError::ForbiddenControl);
    }
    Ok(())
}

#[cfg(windows)]
fn is_windows_reserved(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

#[cfg(windows)]
fn os_str_bytes(value: &OsStr) -> u64 {
    use std::os::windows::ffi::OsStrExt;
    value
        .encode_wide()
        .fold(0_u64, |bytes, _| bytes.saturating_add(2))
}

#[cfg(windows)]
const fn separator_bytes() -> u64 {
    2
}

#[cfg(not(windows))]
const fn separator_bytes() -> u64 {
    1
}

#[cfg(unix)]
fn os_str_bytes(value: &OsStr) -> u64 {
    use std::os::unix::ffi::OsStrExt;
    u64::try_from(value.as_bytes().len()).unwrap_or(u64::MAX)
}

#[cfg(not(any(unix, windows)))]
fn os_str_bytes(value: &OsStr) -> u64 {
    u64::try_from(value.to_string_lossy().len()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn policy() -> Result<PathPolicy, ProjectPathError> {
        PathPolicy::new(8, 255, 1_024)
    }

    #[test]
    fn normal_relative_components_are_preserved() -> Result<(), ProjectPathError> {
        let path = ProjectRelativePath::parse(Path::new("src").join("lib.rs"), policy()?)?;
        assert_eq!(path.as_path(), Path::new("src").join("lib.rs"));
        Ok(())
    }

    #[test]
    fn traversal_absolute_and_component_excess_are_rejected() -> Result<(), ProjectPathError> {
        assert!(matches!(
            ProjectRelativePath::parse("../escape", policy()?),
            Err(ProjectPathError::NotProjectRelative)
        ));
        let absolute = std::env::current_dir().map_err(|_| ProjectPathError::NotProjectRelative)?;
        assert!(matches!(
            ProjectRelativePath::parse(absolute, policy()?),
            Err(ProjectPathError::NotProjectRelative)
        ));
        let strict = PathPolicy::new(1, 255, 1_024)?;
        assert!(matches!(
            ProjectRelativePath::parse(Path::new("a").join("b"), strict),
            Err(ProjectPathError::ComponentBudgetExceeded { maximum: 1 })
        ));
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn windows_drive_ads_and_devices_are_rejected() -> Result<(), ProjectPathError> {
        assert!(ProjectRelativePath::parse("C:relative", policy()?).is_err());
        assert!(ProjectRelativePath::parse("file:stream", policy()?).is_err());
        assert!(ProjectRelativePath::parse("NUL.txt", policy()?).is_err());
        assert!(ProjectRelativePath::parse("trailing.", policy()?).is_err());
        Ok(())
    }
}
