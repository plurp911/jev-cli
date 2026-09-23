//! A wrapper that makes accidental disclosure of credential material a compile-time or
//! redaction-time event rather than a production incident.

use std::fmt;

use zeroize::Zeroizing;

/// The text substituted for a secret in every human- or machine-readable rendering.
const REDACTED: &str = "<redacted>";

/// Holds credential material.
///
/// Guarantees:
///
/// * [`fmt::Debug`] and [`fmt::Display`] render `<redacted>`, so a secret cannot leak
///   through `{:?}` in a log line, an error chain, or a panic payload.
/// * The value is zeroized on drop, by `Zeroizing`.
/// * The type deliberately does **not** implement `Serialize`, `Clone`, or `Deref`;
///   reading the plaintext requires the explicit, greppable [`Secret::expose`].
///
/// # Examples
///
/// ```
/// use jev_config::Secret;
///
/// let key = Secret::new("sk-live-not-a-real-key".to_owned());
/// assert_eq!(format!("{key:?}"), "Secret(<redacted>)");
/// assert_eq!(key.to_string(), "<redacted>");
/// assert_eq!(key.expose(), "sk-live-not-a-real-key");
/// ```
pub struct Secret {
    inner: Zeroizing<String>,
}

impl Secret {
    /// Wraps `value` as a secret.
    #[must_use]
    pub fn new(value: String) -> Self {
        Self {
            inner: Zeroizing::new(value),
        }
    }

    /// Returns the plaintext.
    ///
    /// Every call site is an auditable disclosure point. Reviewers should be able to
    /// enumerate them with `rg 'expose\(\)'` and justify each one.
    #[must_use]
    pub fn expose(&self) -> &str {
        self.inner.as_str()
    }

    /// Returns `true` when the secret contains no characters.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns `true` when the secret contains a line break or other control character.
    ///
    /// Such a value cannot be an HTTP header, so it cannot be a usable API key. Asked
    /// here, rather than through [`Self::expose`], so the check is not a disclosure
    /// point.
    #[must_use]
    pub fn has_control_character(&self) -> bool {
        self.inner.chars().any(char::is_control)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret({REDACTED})")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANARY: &str = "sk-canary-0123456789abcdef";

    #[test]
    fn debug_is_redacted() {
        let secret = Secret::new(CANARY.to_owned());
        let rendered = format!("{secret:?}");
        assert_eq!(rendered, "Secret(<redacted>)");
        assert!(!rendered.contains(CANARY));
    }

    #[test]
    fn display_is_redacted() {
        let secret = Secret::new(CANARY.to_owned());
        assert_eq!(secret.to_string(), REDACTED);
    }

    #[test]
    fn nested_debug_is_redacted() {
        // The realistic leak is a secret buried inside a larger `{:?}` of a config
        // struct or an error variant, not a direct `println!` of the key itself.
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Config {
            endpoint: &'static str,
            api_key: Secret,
        }

        let rendered = format!(
            "{:?}",
            Config {
                endpoint: "https://api.typesafe.ai",
                api_key: Secret::new(CANARY.to_owned())
            }
        );
        assert!(
            !rendered.contains(CANARY),
            "secret leaked through nested Debug: {rendered}"
        );
    }

    #[test]
    fn expose_returns_plaintext() {
        assert_eq!(Secret::new(CANARY.to_owned()).expose(), CANARY);
    }
}
