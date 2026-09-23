//! `Content` — the JSON shapes TypeSafe accepts wherever free-form text may also be
//! structured.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::limits::MAX_JSON_DEPTH;

/// Reasons a JSON value cannot be [`Content`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ContentError {
    /// The value was a number, boolean, or null.
    ///
    /// The API accepts `string | object | array` in these positions; a bare scalar is
    /// rejected by the server, so rejecting it locally saves a round trip.
    #[error("expected a string, object, or array, found {found}")]
    WrongKind {
        /// The JSON kind that was supplied.
        found: &'static str,
    },
    /// The value nested deeper than [`MAX_JSON_DEPTH`].
    #[error("JSON nesting is deeper than the supported limit of {MAX_JSON_DEPTH}")]
    TooDeep,
    /// A string was empty, or contained only whitespace.
    #[error("expected non-empty text")]
    Empty,
}

/// Text, or JSON structure standing in for text.
///
/// The official API accepts `string | object | array` for `state`, for `instructions`,
/// and for every entry of `criteria`. Modelling that as an enum rather than a bare
/// [`Value`] means a number or a `null` cannot reach the wire encoder at all.
///
/// See <https://docs.typesafe.ai/primitives/advanced>.
///
/// # Examples
///
/// ```
/// use jev_core::Content;
///
/// let text = Content::text("Is this urgent?")?;
/// assert!(text.is_text());
///
/// // A bare scalar is not valid here, and cannot be constructed.
/// assert!(Content::try_from(serde_json::json!(42)).is_err());
/// # Ok::<(), jev_core::ContentError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// Plain text.
    Text(String),
    /// A JSON object, used when a description has several named parts.
    Object(Map<String, Value>),
    /// A JSON array, used for lists of examples or contrasts.
    Array(Vec<Value>),
}

impl Content {
    /// Wraps `text`, rejecting an empty or whitespace-only string.
    ///
    /// # Errors
    ///
    /// Returns [`ContentError::Empty`] when `text` has no non-whitespace characters.
    pub fn text(text: impl Into<String>) -> Result<Self, ContentError> {
        let text = text.into();
        if text.trim().is_empty() {
            return Err(ContentError::Empty);
        }
        Ok(Self::Text(text))
    }

    /// Returns `true` for the [`Content::Text`] variant.
    #[must_use]
    pub const fn is_text(&self) -> bool {
        matches!(self, Self::Text(_))
    }

    /// Returns the text of a [`Content::Text`], or `None` for a structured value.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Object(_) | Self::Array(_) => None,
        }
    }

    /// Borrows this value as a [`Value`]-compatible view for rendering.
    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Text(text) => Value::String(text.clone()),
            Self::Object(map) => Value::Object(map.clone()),
            Self::Array(items) => Value::Array(items.clone()),
        }
    }

    /// The JSON kind name, for diagnostics.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Text(_) => "string",
            Self::Object(_) => "object",
            Self::Array(_) => "array",
        }
    }
}

impl TryFrom<Value> for Content {
    type Error = ContentError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        check_json_depth(&value, MAX_JSON_DEPTH)?;
        match value {
            Value::String(text) => Self::text(text),
            Value::Object(map) => Ok(Self::Object(map)),
            Value::Array(items) => Ok(Self::Array(items)),
            Value::Null => Err(ContentError::WrongKind { found: "null" }),
            Value::Bool(_) => Err(ContentError::WrongKind { found: "boolean" }),
            Value::Number(_) => Err(ContentError::WrongKind { found: "number" }),
        }
    }
}

impl fmt::Display for Content {
    /// Renders text verbatim and structure as compact JSON.
    ///
    /// Callers displaying this to a terminal must sanitize the result first: the
    /// content can come from a file or an API response.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => f.write_str(text),
            Self::Object(_) | Self::Array(_) => {
                // Serializing a `Value` cannot fail for any value that exists.
                let rendered =
                    serde_json::to_string(&self.to_value()).unwrap_or_else(|_| "{}".to_owned());
                f.write_str(&rendered)
            }
        }
    }
}

impl Serialize for Content {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Text(text) => serializer.serialize_str(text),
            Self::Object(map) => map.serialize(serializer),
            Self::Array(items) => items.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Content {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::try_from(value).map_err(serde::de::Error::custom)
    }
}

/// Returns an error if `value` nests deeper than `budget` levels.
///
/// Written iteratively with an explicit stack: a recursive walk over attacker-supplied
/// JSON is itself the stack-exhaustion bug this function exists to prevent. Exported so
/// that the client layer can apply the same bound to a decoded API response.
///
/// # Errors
///
/// Returns [`ContentError::TooDeep`] when `value` nests deeper than `budget`.
///
/// # Examples
///
/// ```
/// use jev_core::{check_json_depth, limits::MAX_JSON_DEPTH};
///
/// assert!(check_json_depth(&serde_json::json!({"a": [1]}), MAX_JSON_DEPTH).is_ok());
/// assert!(check_json_depth(&serde_json::json!({"a": [1]}), 1).is_err());
/// ```
pub fn check_json_depth(value: &Value, budget: usize) -> Result<(), ContentError> {
    let mut stack: Vec<(&Value, usize)> = vec![(value, 1)];
    while let Some((current, depth)) = stack.pop() {
        if depth > budget {
            return Err(ContentError::TooDeep);
        }
        match current {
            Value::Object(map) => stack.extend(map.values().map(|child| (child, depth + 1))),
            Value::Array(items) => stack.extend(items.iter().map(|child| (child, depth + 1))),
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn accepts_the_three_documented_shapes() {
        assert!(Content::try_from(json!("text")).is_ok());
        assert!(Content::try_from(json!({"a": 1})).is_ok());
        assert!(Content::try_from(json!([1, 2])).is_ok());
    }

    #[test]
    fn rejects_bare_scalars() {
        for (value, found) in [
            (json!(null), "null"),
            (json!(true), "boolean"),
            (json!(1.5), "number"),
        ] {
            assert_eq!(
                Content::try_from(value),
                Err(ContentError::WrongKind { found })
            );
        }
    }

    #[test]
    fn rejects_blank_text() {
        assert_eq!(Content::try_from(json!("")), Err(ContentError::Empty));
        assert_eq!(Content::try_from(json!("  \n ")), Err(ContentError::Empty));
    }

    #[test]
    fn rejects_input_nested_past_the_limit() {
        // Built iteratively: constructing the hostile value recursively would risk the
        // very overflow this test is about.
        let mut value = json!("leaf");
        for _ in 0..MAX_JSON_DEPTH + 5 {
            value = Value::Array(vec![value]);
        }
        assert_eq!(Content::try_from(value), Err(ContentError::TooDeep));
    }

    #[test]
    fn accepts_input_at_exactly_the_limit() {
        let mut value = json!("leaf");
        for _ in 0..MAX_JSON_DEPTH - 1 {
            value = Value::Array(vec![value]);
        }
        assert!(Content::try_from(value).is_ok());
    }

    #[test]
    fn a_very_deep_document_does_not_overflow_the_stack() {
        // 100_000 levels: a recursive checker would abort the process here.
        let mut value = json!("leaf");
        for _ in 0..100_000 {
            value = Value::Array(vec![value]);
        }
        assert_eq!(
            check_json_depth(&value, MAX_JSON_DEPTH),
            Err(ContentError::TooDeep)
        );
        // Dropping a deeply nested `Value` is also recursive, so unwind it by hand.
        while let Value::Array(mut items) = value {
            value = items.pop().unwrap_or(Value::Null);
        }
    }

    #[test]
    fn round_trips_through_json() {
        for original in [json!("text"), json!({"a": "b"}), json!(["x"])] {
            let content = Content::try_from(original.clone()).unwrap();
            let encoded = serde_json::to_value(&content).unwrap();
            assert_eq!(encoded, original);
        }
    }
}
