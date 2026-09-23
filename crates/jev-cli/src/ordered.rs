//! Order-preserving, duplicate-detecting JSON object parsing.
//!
//! # Why this exists
//!
//! `serde_json::Map` is a `BTreeMap` unless the `preserve_order` feature is on, and it
//! collapses duplicate keys with last-one-wins in either case. Both behaviours are
//! wrong here, and both were wrong *silently*:
//!
//! * **Order.** `jev --dry-run` promises a body the user can compare against the file
//!   they wrote, and human output reads in the order the questions were asked. With a
//!   `BTreeMap` both come out alphabetical.
//! * **Duplicates.** A request document with the same question id twice would lose one
//!   of them before any validation ran, so the duplicate check in `jev-core` could
//!   never fire for file input. The user would be billed for a request that asked fewer
//!   questions than they wrote, with no diagnostic.
//!
//! Enabling `preserve_order` would fix the first and not the second, and it would add a
//! dependency. A small `Visitor` fixes both and adds none.

use std::fmt;

use serde::Deserialize;
use serde::de::{Deserializer, MapAccess, Visitor};
use serde_json::Value;

/// A JSON object as an ordered list of entries, duplicates included.
///
/// Nothing is discarded and nothing is reordered, so the caller can decide what a
/// duplicate means rather than discovering that one was dropped.
///
/// Generic in the value type so the guarantee can be carried one level deeper. It was
/// applied only to the top level and the question-id level, and one level further in --
/// inside a question body -- the policy silently reversed: a question with `"type"`
/// twice was last-one-wins, so `{"type": "noul", "type": "choice"}` sent a Choice
/// without a word of complaint, and a Choice's option names came back alphabetized.
pub type OrderedObject = OrderedMap<Value>;

/// A question body, or any object whose own keys must keep their order.
///
/// [`Field::Object`] is tried first, so a nested object keeps its order and its
/// duplicates; anything else is an ordinary [`Value`], which is enough because the only
/// other ordered shape JSON has is an array, and arrays already keep their order.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Field {
    /// A nested object, order and duplicates intact.
    Object(OrderedMap<Field>),
    /// A scalar or an array.
    Other(Value),
}

impl Field {
    /// The plain JSON value, losing the order of any nested object's keys.
    ///
    /// Used where the value is handed to `jev-core`, which takes `serde_json::Value`.
    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Object(map) => Value::Object(
                map.entries()
                    .iter()
                    .map(|(key, value)| (key.clone(), value.to_value()))
                    .collect(),
            ),
            Self::Other(value) => value.clone(),
        }
    }

    /// The entries, when this is an object.
    #[must_use]
    pub const fn as_object(&self) -> Option<&OrderedMap<Self>> {
        match self {
            Self::Object(map) => Some(map),
            Self::Other(_) => None,
        }
    }
}

/// See [`OrderedObject`].
#[derive(Debug, Clone, PartialEq)]
pub struct OrderedMap<V>(Vec<(String, V)>);

impl<V> Default for OrderedMap<V> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<V> OrderedMap<V> {
    /// The entries, in document order.
    #[must_use]
    pub fn entries(&self) -> &[(String, V)] {
        &self.0
    }

    /// The first key that appears more than once, if any.
    #[must_use]
    pub fn first_duplicate(&self) -> Option<&str> {
        let mut seen = std::collections::BTreeSet::new();
        self.0
            .iter()
            .find(|(key, _)| !seen.insert(key.as_str()))
            .map(|(key, _)| key.as_str())
    }

    /// How many entries there are, counting duplicates separately.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the object has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'de, V: Deserialize<'de>> Deserialize<'de> for OrderedMap<V> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entries<V>(std::marker::PhantomData<V>);

        impl<'de, V: Deserialize<'de>> Visitor<'de> for Entries<V> {
            type Value = OrderedMap<V>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<M: MapAccess<'de>>(self, mut access: M) -> Result<Self::Value, M::Error> {
                // `size_hint` is a hint from attacker-controlled input, so it only sizes
                // a bounded pre-allocation, never an unbounded one.
                let mut entries = Vec::with_capacity(access.size_hint().unwrap_or(0).min(64));
                while let Some((key, value)) = access.next_entry::<String, V>()? {
                    entries.push((key, value));
                }
                Ok(OrderedMap(entries))
            }
        }

        deserializer.deserialize_map(Entries(std::marker::PhantomData))
    }
}

/// Parses `text` as a JSON object, preserving order and keeping duplicates.
///
/// # Errors
///
/// Returns the `serde_json` error when `text` is not valid JSON, or is not an object.
pub fn parse_object(text: &str) -> Result<OrderedObject, serde_json::Error> {
    serde_json::from_str(text)
}

/// Looks a key up in an ordered object.
#[must_use]
pub fn get<'a, V>(object: &'a OrderedMap<V>, key: &str) -> Option<&'a V> {
    object
        .entries()
        .iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, value)| value)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn document_order_is_preserved() {
        // A `serde_json::Map` would return these alphabetically, which would make
        // `--dry-run` output impossible to compare against the file the user wrote.
        let object = parse_object(r#"{"zebra": 1, "apple": 2, "mango": 3}"#).unwrap();
        let keys: Vec<&str> = object
            .entries()
            .iter()
            .map(|(key, _)| key.as_str())
            .collect();
        assert_eq!(keys, vec!["zebra", "apple", "mango"]);
    }

    #[test]
    fn duplicates_are_kept_so_the_caller_can_reject_them() {
        // `serde_json::Map` would silently keep only the last, and the user would be
        // billed for a request that asked fewer questions than they wrote.
        let object = parse_object(r#"{"dup": "first", "other": 1, "dup": "second"}"#).unwrap();
        assert_eq!(object.len(), 3);
        assert_eq!(object.first_duplicate(), Some("dup"));
    }

    #[test]
    fn a_document_without_duplicates_reports_none() {
        let object = parse_object(r#"{"a": 1, "b": 2}"#).unwrap();
        assert_eq!(object.first_duplicate(), None);
    }

    #[test]
    fn nested_objects_are_ordinary_values() {
        // Only the top level needs ordering and duplicate detection; nested values are
        // content, and the API treats them as data.
        let object = parse_object(r#"{"q": {"type": "noul", "instructions": "?"}}"#).unwrap();
        assert_eq!(
            get(&object, "q"),
            Some(&json!({"type": "noul", "instructions": "?"}))
        );
    }

    #[test]
    fn a_non_object_is_an_error() {
        assert!(parse_object("[1, 2]").is_err());
        assert!(parse_object("\"text\"").is_err());
        assert!(parse_object("not json").is_err());
    }

    #[test]
    fn an_empty_object_parses() {
        let object = parse_object("{}").unwrap();
        assert!(object.is_empty());
        assert_eq!(object.first_duplicate(), None);
    }
}
