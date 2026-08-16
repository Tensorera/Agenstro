use std::{fmt, str::FromStr};

const MAX_IDENTIFIER_BYTES: usize = 128;
const MAX_PATH_BYTES: usize = 4_096;
const MAX_PATH_COMPONENT_BYTES: usize = 255;

/// A portable identifier or project-relative path invariant violation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentifierError {
    /// The value is empty.
    Empty,
    /// The value exceeds its byte limit.
    TooLong,
    /// The value does not match the identifier grammar.
    InvalidFormat,
    /// A path is absolute or otherwise not project-relative.
    NotProjectRelative,
    /// A path contains an empty, current, or parent component.
    InvalidPathComponent,
    /// A component contains a non-portable character or spelling.
    NonPortableComponent,
    /// A component uses a reserved Windows device name.
    ReservedComponent,
    /// The path targets Clef/Tactus internal project state.
    ReservedNamespace,
}

impl fmt::Display for IdentifierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "value must not be empty",
            Self::TooLong => "value exceeds its byte limit",
            Self::InvalidFormat => "identifier has an invalid format",
            Self::NotProjectRelative => "path must be relative to the project root",
            Self::InvalidPathComponent => {
                "path must not contain empty, current, or parent components"
            }
            Self::NonPortableComponent => "value contains a non-portable path component",
            Self::ReservedComponent => "value uses a reserved Windows path component",
            Self::ReservedNamespace => "path uses a reserved project-state namespace",
        })
    }
}

impl std::error::Error for IdentifierError {}

fn validate_identifier(value: &str, allow_leading_underscore: bool) -> Result<(), IdentifierError> {
    if value.is_empty() {
        return Err(IdentifierError::Empty);
    }
    if value.len() > MAX_IDENTIFIER_BYTES {
        return Err(IdentifierError::TooLong);
    }
    validate_portable_component(value)?;

    let bytes = value.as_bytes();
    let valid_first =
        bytes[0].is_ascii_alphanumeric() || (allow_leading_underscore && bytes[0] == b'_');
    if !valid_first
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(IdentifierError::InvalidFormat);
    }
    Ok(())
}

fn validate_portable_component(value: &str) -> Result<(), IdentifierError> {
    if value.ends_with(' ')
        || value.ends_with('.')
        || value.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
        })
    {
        return Err(IdentifierError::NonPortableComponent);
    }
    let stem = value.split('.').next().unwrap_or_default();
    if is_windows_reserved(stem) {
        return Err(IdentifierError::ReservedComponent);
    }
    Ok(())
}

fn is_windows_reserved(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "AUX" | "CON" | "NUL" | "PRN")
        || upper
            .strip_prefix("COM")
            .or_else(|| upper.strip_prefix("LPT"))
            .is_some_and(|number| {
                matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
}

macro_rules! define_identifier {
    ($(#[$meta:meta])* $name:ident, $allow_leading_underscore:expr) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(Box<str>);

        impl $name {
            /// Parses and validates the portable identifier.
            ///
            /// # Errors
            ///
            /// Returns [`IdentifierError`] when the value is empty, oversized,
            /// malformed, or unsafe as a portable path component.
            pub fn parse(value: &str) -> Result<Self, IdentifierError> {
                validate_identifier(value, $allow_leading_underscore)?;
                Ok(Self(value.into()))
            }

            /// Returns the canonical identifier text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = IdentifierError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }
    };
}

define_identifier!(
    /// Stable identity of one versioned workflow.
    WorkflowId,
    false
);
define_identifier!(
    /// Stable identity of one task inside a workflow.
    TaskId,
    false
);
define_identifier!(
    /// Stable identity of one workflow run.
    RunId,
    false
);
define_identifier!(
    /// Registered provider-neutral domain function name.
    DomainFunctionName,
    false
);
define_identifier!(
    /// Named artifact input or output slot.
    ArtifactName,
    true
);

/// A canonical slash-separated UTF-8 path confined to a project root.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProjectPath(Box<str>);

impl ProjectPath {
    /// Parses a portable project-relative path and normalizes `\\` to `/`.
    ///
    /// Dot segments are rejected rather than resolved. Absolute, drive-relative,
    /// UNC, control-character, Windows-device, and reserved `.tactus` paths are
    /// also rejected so the stored spelling has one cross-platform meaning.
    ///
    /// # Errors
    ///
    /// Returns [`IdentifierError`] when any path invariant is violated.
    pub fn parse(value: &str) -> Result<Self, IdentifierError> {
        if value.is_empty() {
            return Err(IdentifierError::Empty);
        }
        if value.len() > MAX_PATH_BYTES {
            return Err(IdentifierError::TooLong);
        }
        let normalized = value.replace('\\', "/");
        let bytes = normalized.as_bytes();
        if normalized.starts_with('/')
            || bytes.get(1) == Some(&b':')
            || normalized.starts_with("//")
        {
            return Err(IdentifierError::NotProjectRelative);
        }

        let components: Vec<&str> = normalized.split('/').collect();
        if components
            .iter()
            .any(|component| component.is_empty() || matches!(*component, "." | ".."))
        {
            return Err(IdentifierError::InvalidPathComponent);
        }
        if components[0].eq_ignore_ascii_case(".tactus") {
            return Err(IdentifierError::ReservedNamespace);
        }
        for component in components {
            if component.len() > MAX_PATH_COMPONENT_BYTES {
                return Err(IdentifierError::TooLong);
            }
            validate_portable_component(component)?;
        }
        Ok(Self(normalized.into()))
    }

    /// Returns the canonical slash-separated path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProjectPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ProjectPath {
    type Err = IdentifierError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{ArtifactName, IdentifierError, ProjectPath, TaskId};

    #[test]
    fn rejects_nonportable_identifiers() {
        for value in ["CON", "nul.json", "task:child", "task.", "task "] {
            assert!(TaskId::parse(value).is_err(), "{value}");
        }
        assert!(TaskId::parse("COM10.result").is_ok());
        assert!(ArtifactName::parse("_result").is_ok());
    }

    #[test]
    fn path_is_canonical_and_root_confined() -> Result<(), IdentifierError> {
        assert_eq!(
            ProjectPath::parse("src\\package\\main.rs")?.as_str(),
            "src/package/main.rs"
        );
        for value in [
            "../escape",
            "src/../escape",
            "/absolute",
            "C:/absolute",
            "C:relative",
            "//server/share",
            ".tactus/state",
            "src//main.rs",
            "src/CON.txt",
        ] {
            assert!(ProjectPath::parse(value).is_err(), "{value}");
        }
        Ok(())
    }
}
