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
use serde::de::{Deserializer, MapAccess, SeqAccess, Visitor};
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
/// duplicates. Arrays recurse through [`Field`] so objects inside them keep duplicate
/// keys too; only scalar values use an ordinary [`Value`].
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Field {
    /// A nested object, order and duplicates intact.
    Object(OrderedMap<Field>),
    /// An array whose nested objects keep duplicates intact.
    Array(Vec<Field>),
    /// A scalar.
    Other(Value),
}

impl Field {
    /// The plain JSON value, rejecting duplicates before collecting object entries.
    ///
    /// Used where the value is handed to `jev-core`, which takes `serde_json::Value`.
    /// # Errors
    /// Returns an error for a duplicate object key at any nesting level.
    pub fn to_value(&self) -> Result<Value, serde_json::Error> {
        match self {
            Self::Object(map) => {
                if let Some(key) = map.first_duplicate() {
                    return Err(duplicate_error(key));
                }
                map.entries()
                    .iter()
                    .map(|(key, value)| value.to_value().map(|value| (key.clone(), value)))
                    .collect::<Result<serde_json::Map<_, _>, _>>()
                    .map(Value::Object)
            }
            Self::Array(values) => values
                .iter()
                .map(Self::to_value)
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            Self::Other(value) => Ok(value.clone()),
        }
    }

    /// The entries, when this is an object.
    #[must_use]
    pub const fn as_object(&self) -> Option<&OrderedMap<Self>> {
        match self {
            Self::Object(map) => Some(map),
            Self::Array(_) | Self::Other(_) => None,
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

/// Parses JSON without silently overwriting duplicate keys at any nesting level.
///
/// Values have the same representation as ordinary `serde_json::Value` decoding,
/// and the JSON deserializer keeps its normal nesting limit. Use [`Field`] when
/// object order must also be preserved until validation.
pub(crate) fn parse_unambiguous_value(text: &str) -> Result<Value, serde_json::Error> {
    serde_json::from_str::<UnambiguousValue>(text).map(|value| value.0)
}

/// Checks a selected optional JSON field without changing how other fields decode.
pub(crate) fn deserialize_optional_unambiguous_value<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Option::<UnambiguousValue>::deserialize(deserializer).map(|value| value.map(|value| value.0))
}

struct UnambiguousValue(Value);

fn duplicate_error<E: serde::de::Error>(key: &str) -> E {
    E::custom(format!(
        "duplicate field `{}`; use each object key only once",
        crate::output::Safe::new(key)
    ))
}

impl<'de> Deserialize<'de> for UnambiguousValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Unambiguous;

        impl<'de> Visitor<'de> for Unambiguous {
            type Value = UnambiguousValue;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("JSON with no duplicate object keys")
            }

            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(UnambiguousValue(Value::Bool(value)))
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(UnambiguousValue(Value::Number(value.into())))
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(UnambiguousValue(Value::Number(value.into())))
            }

            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(value)
                    .map(|number| UnambiguousValue(Value::Number(number)))
                    .ok_or_else(|| E::custom("non-finite JSON number"))
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                self.visit_string(value.to_owned())
            }

            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(UnambiguousValue(Value::String(value)))
            }

            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(UnambiguousValue(Value::Null))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::with_capacity(access.size_hint().unwrap_or(0).min(64));
                while let Some(value) = access.next_element::<UnambiguousValue>()? {
                    values.push(value.0);
                }
                Ok(UnambiguousValue(Value::Array(values)))
            }

            fn visit_map<M: MapAccess<'de>>(self, mut access: M) -> Result<Self::Value, M::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = access.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(duplicate_error(&key));
                    }
                    let value = access.next_value::<UnambiguousValue>()?;
                    values.insert(key, value.0);
                }
                Ok(UnambiguousValue(Value::Object(values)))
            }
        }

        deserializer.deserialize_any(Unambiguous)
    }
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

    #[test]
    fn fields_reject_duplicates_inside_nested_arrays_before_conversion() {
        for text in [
            r#"{"flag":null,"flag":false}"#,
            r#"[{"flag":null,"flag":false}]"#,
            r#"{"nested":[[{"flag":null,"flag":false}]]}"#,
        ] {
            let field: Field = serde_json::from_str(text).unwrap();
            let error = field.to_value().unwrap_err().to_string();
            assert!(error.contains("duplicate field `flag`"), "{error}");
        }
        let text = r#"{"nested":[[null,false,-1,"",{"flag":42}]]}"#;
        let field: Field = serde_json::from_str(text).unwrap();
        assert_eq!(
            field.to_value().unwrap(),
            serde_json::from_str::<Value>(text).unwrap()
        );
    }
}
