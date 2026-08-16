use std::{collections::BTreeMap, fmt, str::FromStr};

use thiserror::Error;

/// Maximum byte length of a capability name.
pub const MAX_CAPABILITY_NAME_BYTES: usize = 128;
/// Maximum number of capabilities reported by one daemon instance.
pub const MAX_CAPABILITIES: u32 = 128;

/// A validated lower-case, dot-separated capability name.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CapabilityName(Box<str>);

impl CapabilityName {
    /// Parses a namespaced capability such as `system.health`.
    ///
    /// Each segment starts with an ASCII lower-case letter and may continue
    /// with lower-case letters, digits, or internal hyphens.
    ///
    /// # Errors
    ///
    /// Returns [`CapabilityError`] if the name is empty, oversized, lacks a
    /// namespace, or violates the segment grammar.
    pub fn parse(value: &str) -> Result<Self, CapabilityError> {
        if value.is_empty() {
            return Err(CapabilityError::InvalidName);
        }
        if value.len() > MAX_CAPABILITY_NAME_BYTES {
            return Err(CapabilityError::NameTooLong);
        }

        let segments: Vec<&str> = value.split('.').collect();
        if segments.len() < 2 || segments.iter().any(|segment| !valid_segment(segment)) {
            return Err(CapabilityError::InvalidName);
        }

        Ok(Self(value.into()))
    }

    /// Returns the canonical wire representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn valid_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_lowercase)
        && bytes
            .last()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

impl fmt::Display for CapabilityName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for CapabilityName {
    type Err = CapabilityError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

/// Stability level negotiated for one capability.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CapabilityStability {
    /// The capability is part of the supported stable contract.
    Stable,
    /// The namespaced capability may change outside the stable surface.
    Experimental,
}

/// One named capability and its negotiated stability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Capability {
    name: CapabilityName,
    stability: CapabilityStability,
}

impl Capability {
    /// Creates a capability from a validated name and stability.
    #[must_use]
    pub const fn new(name: CapabilityName, stability: CapabilityStability) -> Self {
        Self { name, stability }
    }

    /// Returns the validated capability name.
    #[must_use]
    pub const fn name(&self) -> &CapabilityName {
        &self.name
    }

    /// Returns the negotiated stability level.
    #[must_use]
    pub const fn stability(&self) -> CapabilityStability {
        self.stability
    }
}

/// A deterministic, bounded set of negotiated capabilities.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapabilitySet {
    entries: BTreeMap<CapabilityName, Capability>,
}

impl CapabilitySet {
    /// Builds a bounded set and rejects duplicate names.
    ///
    /// # Errors
    ///
    /// Returns [`CapabilityError::Duplicate`] or
    /// [`CapabilityError::TooManyCapabilities`] when an invariant is violated.
    pub fn from_capabilities(
        capabilities: impl IntoIterator<Item = Capability>,
    ) -> Result<Self, CapabilityError> {
        let mut set = Self::default();
        for capability in capabilities {
            set.insert(capability)?;
        }
        Ok(set)
    }

    /// Inserts one capability while preserving uniqueness and the hard count limit.
    ///
    /// # Errors
    ///
    /// Returns [`CapabilityError::Duplicate`] when the name already exists or
    /// [`CapabilityError::TooManyCapabilities`] when the set is full.
    pub fn insert(&mut self, capability: Capability) -> Result<(), CapabilityError> {
        if self.entries.contains_key(capability.name()) {
            return Err(CapabilityError::Duplicate {
                name: capability.name().to_string(),
            });
        }
        if self.entries.len() >= MAX_CAPABILITIES as usize {
            return Err(CapabilityError::TooManyCapabilities);
        }

        self.entries.insert(capability.name.clone(), capability);
        Ok(())
    }

    /// Returns capabilities in canonical name order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Capability> {
        self.entries.values()
    }

    /// Returns whether a capability name is present.
    #[must_use]
    pub fn contains(&self, name: &CapabilityName) -> bool {
        self.entries.contains_key(name)
    }

    /// Returns the number of capabilities.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the set is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A capability name or set invariant violation.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CapabilityError {
    /// The capability name violates the documented grammar.
    #[error("capability name is invalid")]
    InvalidName,
    /// The capability name exceeds [`MAX_CAPABILITY_NAME_BYTES`].
    #[error("capability name exceeds its byte limit")]
    NameTooLong,
    /// The set already contains the given capability name.
    #[error("duplicate capability: {name}")]
    Duplicate {
        /// The duplicate, already validated capability name.
        name: String,
    },
    /// The set exceeds [`MAX_CAPABILITIES`].
    #[error("capability set exceeds its item limit")]
    TooManyCapabilities,
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{
        Capability, CapabilityError, CapabilityName, CapabilitySet, CapabilityStability,
        MAX_CAPABILITIES,
    };

    proptest! {
        #[test]
        fn valid_capability_segments_round_trip(
            namespace in "[a-z][a-z0-9]{0,10}",
            feature in "[a-z][a-z0-9-]{0,10}[a-z0-9]"
        ) {
            let value = format!("{namespace}.{feature}");
            let parsed = CapabilityName::parse(&value);

            prop_assert_eq!(parsed.as_ref().map(CapabilityName::as_str), Ok(value.as_str()));
        }
    }

    #[test]
    fn capability_set_has_stable_order_and_rejects_duplicates() -> Result<(), CapabilityError> {
        let first = Capability::new(
            CapabilityName::parse("system.server-info")?,
            CapabilityStability::Stable,
        );
        let second = Capability::new(
            CapabilityName::parse("system.health")?,
            CapabilityStability::Stable,
        );
        let mut set = CapabilitySet::from_capabilities([first, second])?;
        let names: Vec<&str> = set.iter().map(|item| item.name().as_str()).collect();

        assert_eq!(names, ["system.health", "system.server-info"]);
        assert_eq!(
            set.insert(Capability::new(
                CapabilityName::parse("system.health")?,
                CapabilityStability::Experimental,
            )),
            Err(CapabilityError::Duplicate {
                name: "system.health".to_owned(),
            })
        );
        Ok(())
    }

    #[test]
    fn capability_set_enforces_hard_item_limit() -> Result<(), CapabilityError> {
        let mut set = CapabilitySet::default();
        for index in 0..MAX_CAPABILITIES {
            set.insert(Capability::new(
                CapabilityName::parse(&format!("test.capability{index}"))?,
                CapabilityStability::Stable,
            ))?;
        }

        let result = set.insert(Capability::new(
            CapabilityName::parse("test.overflow")?,
            CapabilityStability::Stable,
        ));
        assert_eq!(result, Err(CapabilityError::TooManyCapabilities));
        Ok(())
    }
}
