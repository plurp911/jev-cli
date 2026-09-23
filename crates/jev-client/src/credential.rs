//! The carrier that moves a credential from the configuration layer to the wire.
//!
//! # Why this type exists in `jev-client`
//!
//! `jev-client` is forbidden from *reading* credentials (ADR-0007): it never touches
//! the environment, a keychain, or a file. But it has to *send* one, and the type it
//! receives matters. `docs/threat-model.md` T2 records the hazard: if the key becomes
//! an ordinary `String` inside a `Clone`-derived request struct, a retry path can
//! multiply copies that are never cleared.
//!
//! [`Credential`] closes that gap. It is not `Clone`, not `Serialize`, and never
//! becomes part of a [`Request`](crate::Request); the concrete transport receives it as
//! a separate argument and builds the `Authorization` header at the last moment.

use std::fmt;

use zeroize::Zeroizing;

/// An API key on its way to the wire.
///
/// # Examples
///
/// ```
/// use jev_client::Credential;
///
/// let credential = Credential::new("sk-not-a-real-key".to_owned());
/// assert_eq!(format!("{credential:?}"), "Credential(<redacted>)");
/// ```
pub struct Credential {
    inner: Zeroizing<String>,
}

impl Credential {
    /// Wraps a key.
    #[must_use]
    pub fn new(value: String) -> Self {
        Self {
            inner: Zeroizing::new(value),
        }
    }

    /// Returns the plaintext.
    ///
    /// Every call site is an auditable disclosure point; there should be exactly one,
    /// in the concrete transport's header construction.
    #[must_use]
    pub fn expose(&self) -> &str {
        self.inner.as_str()
    }

    /// Builds the `Authorization` header value in a buffer that is cleared on drop.
    ///
    /// Returning `Zeroizing<String>` rather than `String` keeps the concatenated
    /// `Bearer …` form out of ordinary heap memory once the request has been sent.
    /// What the HTTP library then copies internally is outside our control; that
    /// residual risk is recorded in `docs/threat-model.md`.
    #[must_use]
    pub fn bearer_header(&self) -> Zeroizing<String> {
        Zeroizing::new(format!("Bearer {}", self.inner.as_str()))
    }
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credential(<redacted>)")
    }
}

impl fmt::Display for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANARY: &str = "sk-canary-client-0123456789";

    #[test]
    fn debug_and_display_are_redacted() {
        let credential = Credential::new(CANARY.to_owned());
        assert!(!format!("{credential:?}").contains(CANARY));
        assert!(!credential.to_string().contains(CANARY));
    }

    #[test]
    fn nested_debug_is_redacted() {
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Holder {
            credential: Credential,
        }
        let rendered = format!(
            "{:?}",
            Holder {
                credential: Credential::new(CANARY.to_owned())
            }
        );
        assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
    }

    #[test]
    fn bearer_header_has_the_documented_scheme() {
        let credential = Credential::new("abc".to_owned());
        assert_eq!(credential.bearer_header().as_str(), "Bearer abc");
    }
}
