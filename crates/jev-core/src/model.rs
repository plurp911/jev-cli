//! Model identifiers.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The TypeSafe default model, the alias its official documentation and SDKs use.
///
/// An alias moves when TypeSafe ships a new release, so the answers behind it can
/// change without any change on the user's side. Anyone who has calibrated a threshold
/// against a specific version should pin that version instead; `jev` always reports the
/// concrete model the API says answered, in the `model` field of its JSON output.
///
/// See <https://docs.typesafe.ai/models>.
pub const DEFAULT_MODEL: &str = "jev-latest";

/// Reasons a string is not a usable model identifier.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ModelIdError {
    /// The identifier was empty or whitespace only.
    #[error("a model identifier must not be empty")]
    Empty,
    /// The identifier contained a character that cannot appear in an HTTP JSON body
    /// safely, or that would corrupt terminal output.
    #[error("a model identifier must not contain control characters")]
    ControlCharacter,
    /// The identifier was implausibly long.
    #[error("a model identifier must be at most {max} characters")]
    TooLong {
        /// The accepted maximum.
        max: usize,
    },
}

/// Longest model identifier `jev` accepts.
///
/// The API documents no limit. This is a client-side sanity bound: it is far above any
/// published identifier and exists only so that a mistyped flag cannot push an
/// unbounded string into a request body.
const MAX_MODEL_ID_LEN: usize = 128;

/// A validated model name or alias, as accepted by the request's `model` field.
///
/// The TypeSafe adapter does not keep a hard-coded catalogue of valid models. The set
/// changes without a release on our side, `GET /v1/models` is the authority, and
/// versioned identifiers are accepted by the API whether or not they appear in that
/// list (<https://docs.typesafe.ai/models>). Validation here is therefore about shape,
/// not membership. The Cloudflare adapter separately restricts selectors to its
/// two supported Clef models. Local names and aliases do not fingerprint weights.
///
/// # Examples
///
/// ```
/// use jev_core::ModelId;
///
/// assert_eq!(ModelId::new("jev-1.13.0")?.as_str(), "jev-1.13.0");
/// assert!(ModelId::new("").is_err());
/// # Ok::<(), jev_core::ModelIdError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ModelId(String);

impl ModelId {
    /// Validates and wraps a model identifier.
    ///
    /// # Errors
    ///
    /// Returns [`ModelIdError`] for an empty, over-long, or control-bearing string.
    pub fn new(value: impl Into<String>) -> Result<Self, ModelIdError> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(ModelIdError::Empty);
        }
        if trimmed.chars().count() > MAX_MODEL_ID_LEN {
            return Err(ModelIdError::TooLong {
                max: MAX_MODEL_ID_LEN,
            });
        }
        if trimmed.chars().any(char::is_control) {
            return Err(ModelIdError::ControlCharacter);
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The default alias, `jev-latest`.
    #[must_use]
    pub fn default_alias() -> Self {
        Self(DEFAULT_MODEL.to_owned())
    }

    /// Returns the identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns `true` when this identifier is one of the documented moving aliases.
    ///
    /// Used to warn a user who pins a threshold against an alias that can move beneath
    /// them. The list is documentation-derived and additive: an unknown name is simply
    /// treated as a concrete identifier.
    #[must_use]
    pub fn is_moving_alias(&self) -> bool {
        matches!(self.0.as_str(), "jev-latest" | "jev-preview")
    }
}

impl Default for ModelId {
    fn default() -> Self {
        Self::default_alias()
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ModelId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::new(raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_the_documented_alias() {
        assert_eq!(ModelId::default().as_str(), "jev-latest");
        assert!(ModelId::default().is_moving_alias());
    }

    #[test]
    fn a_pinned_version_is_not_an_alias() {
        assert!(!ModelId::new("jev-1.13.0").unwrap().is_moving_alias());
    }

    #[test]
    fn unknown_models_are_accepted() {
        // The authority is GET /v1/models, not a hard-coded list here.
        assert!(ModelId::new("some-future-model-2.0").is_ok());
    }

    #[test]
    fn rejects_empty_and_control_characters() {
        assert_eq!(ModelId::new("   "), Err(ModelIdError::Empty));
        assert_eq!(
            ModelId::new("jev\u{1b}[2J"),
            Err(ModelIdError::ControlCharacter)
        );
    }

    #[test]
    fn rejects_an_implausibly_long_identifier() {
        let long = "x".repeat(MAX_MODEL_ID_LEN + 1);
        assert!(matches!(
            ModelId::new(long),
            Err(ModelIdError::TooLong { .. })
        ));
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(
            ModelId::new("  jev-latest \n").unwrap().as_str(),
            "jev-latest"
        );
    }
}
