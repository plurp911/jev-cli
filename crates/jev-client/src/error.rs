//! Error types.
//!
//! No variant in this module may carry credential material, an `Authorization` header,
//! or an unbounded copy of a response body. Text taken from an API error body is
//! truncated to [`MAX_ERROR_BODY_CHARS`], mirroring the official SDK, so a hostile
//! endpoint cannot use an error message as an unbounded output channel.

use jev_core::limits::MAX_ERROR_BODY_CHARS;

/// A failure at the transport layer: the request never produced an HTTP response.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TransportError {
    /// The request could not be delivered (DNS, TCP, TLS, or proxy failure).
    #[error("could not reach the API endpoint: {reason}")]
    Unreachable {
        /// Human-readable cause, never containing header values.
        reason: String,
    },
    /// The request was delivered but no response arrived within the deadline.
    #[error("the request timed out after {seconds}s")]
    Timeout {
        /// The deadline that elapsed.
        seconds: u64,
    },
    /// The response body exceeded the client's hard cap.
    #[error("the API response exceeded the {limit}-byte limit this client will buffer")]
    ResponseTooLarge {
        /// The cap that was exceeded.
        limit: u64,
    },
    /// The credential cannot be sent as an HTTP header value, because it contains a
    /// line break or control character. Nothing was sent.
    ///
    /// `jev-config` refuses such a key at load time; this is the defence in depth for
    /// any other path to a [`Credential`](crate::Credential). It is a credential
    /// problem, not an unreachable endpoint, and retrying it cannot help.
    #[error(
        "the API key cannot be sent: it contains a line break or control character, \
         so it is not a valid HTTP header value; nothing was sent"
    )]
    InvalidCredential,
}

impl TransportError {
    /// Whether retrying this failure could plausibly succeed.
    ///
    /// A connection or timeout failure is transient. An over-large response is not: the
    /// same endpoint will send the same thing again.
    #[must_use]
    pub const fn is_transient(&self) -> bool {
        match self {
            Self::Unreachable { .. } | Self::Timeout { .. } => true,
            Self::ResponseTooLarge { .. } | Self::InvalidCredential => false,
        }
    }
}

/// A failure of an API call as a whole.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ClientError {
    /// The request never reached the API, or the response could not be read.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// The API returned a response this client could not decode.
    #[error("the API returned a response this version cannot decode: {reason}")]
    MalformedResponse {
        /// What specifically failed to decode. Never the full body.
        reason: String,
    },
    /// The API rejected the credential (HTTP 401), or denied access (HTTP 403).
    #[error("the API rejected the credential (HTTP {status}){}", suffix(.message.as_ref()))]
    Unauthorized {
        /// The status observed: 401 or 403.
        status: u16,
        /// The API's stated reason, truncated.
        message: Option<String>,
    },
    /// The API rejected the request as invalid (HTTP 400 or 422).
    #[error("the API rejected the request as invalid (HTTP {status}){}", suffix(.message.as_ref()))]
    InvalidRequest {
        /// The status observed.
        status: u16,
        /// The API's stated reason, truncated.
        message: Option<String>,
    },
    /// The requested resource does not exist (HTTP 404).
    ///
    /// Against the official endpoint this usually means a base URL that points at the
    /// wrong service. An unknown model identifier is not a 404: the official API
    /// answers it with HTTP 400 "Unknown model: …", which is [`Self::InvalidRequest`]
    /// (observed live 2026-09-23).
    #[error("the API reported that the resource does not exist (HTTP 404){}", suffix(.message.as_ref()))]
    NotFound {
        /// The API's stated reason, truncated.
        message: Option<String>,
    },
    /// The API applied a rate limit (HTTP 429) or reported overload (HTTP 529), and the
    /// retry budget was exhausted.
    #[error("the API is rate limiting or overloaded (HTTP {status}){}", suffix(.message.as_ref()))]
    Throttled {
        /// The status observed.
        status: u16,
        /// The API's stated reason, truncated.
        message: Option<String>,
    },
    /// The API failed (HTTP 5xx) and the retry budget was exhausted.
    #[error("the API failed (HTTP {status}){}", suffix(.message.as_ref()))]
    ServerError {
        /// The status observed.
        status: u16,
        /// The API's stated reason, truncated.
        message: Option<String>,
    },
    /// A status this client has no specific handling for.
    #[error("the API returned an unexpected HTTP {status}{}", suffix(.message.as_ref()))]
    Unexpected {
        /// The status observed.
        status: u16,
        /// The API's stated reason, truncated.
        message: Option<String>,
    },
}

impl ClientError {
    /// The HTTP status behind this error, when there was one.
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Unauthorized { status, .. }
            | Self::InvalidRequest { status, .. }
            | Self::Throttled { status, .. }
            | Self::ServerError { status, .. }
            | Self::Unexpected { status, .. } => Some(*status),
            Self::NotFound { .. } => Some(404),
            Self::Transport(_) | Self::MalformedResponse { .. } => None,
        }
    }

    /// Whether the failure is about credentials rather than the request or the network.
    ///
    /// Drives the exit-code split between "fix your credentials" and "fix your command".
    #[must_use]
    pub const fn is_auth(&self) -> bool {
        matches!(
            self,
            Self::Unauthorized { .. } | Self::Transport(TransportError::InvalidCredential)
        )
    }

    /// Whether the failure is about the API being unreachable or unwell, as opposed to
    /// the request being wrong.
    #[must_use]
    pub const fn is_unavailable(&self) -> bool {
        match self {
            Self::Transport(error) => !matches!(error, TransportError::InvalidCredential),
            Self::Throttled { .. } | Self::ServerError { .. } => true,
            _ => false,
        }
    }

    /// Builds the right variant for an HTTP status.
    ///
    /// The mapping follows the official SDK's `STATUS_ERROR_TYPES`, extended with 529
    /// Overloaded, which the HTTP API reference documents alongside 429.
    #[must_use]
    pub fn from_status(status: u16, message: Option<String>) -> Self {
        let message = message.map(|text| truncate(&text));
        match status {
            401 | 403 => Self::Unauthorized { status, message },
            400 | 413 | 422 => Self::InvalidRequest { status, message },
            404 => Self::NotFound { message },
            408 | 429 | 529 => Self::Throttled { status, message },
            // A redirect is a server-side routing decision, not a bad request. `jev`
            // does not follow one -- a `Location` must not be able to move a credential
            // -- but telling the user to fix their command would be wrong, so it is
            // classified with the other "the service is not behaving" statuses.
            300..=399 | 500..=528 | 530..=599 => Self::ServerError { status, message },
            _ => Self::Unexpected { status, message },
        }
    }
}

/// Formats an optional API message as a trailing clause, for the `thiserror` templates.
fn suffix(message: Option<&String>) -> String {
    match message {
        Some(text) if !text.is_empty() => format!(": {text}"),
        _ => String::new(),
    }
}

/// Clips API-supplied text to the documented bound, marking that it was clipped.
#[must_use]
pub(crate) fn truncate(text: &str) -> String {
    let mut out: String = text.chars().take(MAX_ERROR_BODY_CHARS).collect();
    if out.chars().count() < text.chars().count() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping_follows_the_official_sdk() {
        assert!(ClientError::from_status(401, None).is_auth());
        assert!(ClientError::from_status(403, None).is_auth());
        assert!(matches!(
            ClientError::from_status(422, None),
            ClientError::InvalidRequest { .. }
        ));
        assert!(matches!(
            ClientError::from_status(400, None),
            ClientError::InvalidRequest { .. }
        ));
        assert!(matches!(
            ClientError::from_status(404, None),
            ClientError::NotFound { .. }
        ));
        assert!(matches!(
            ClientError::from_status(429, None),
            ClientError::Throttled { .. }
        ));
        // 529 Overloaded is documented in the HTTP API reference next to 429.
        assert!(matches!(
            ClientError::from_status(529, None),
            ClientError::Throttled { .. }
        ));
        assert!(matches!(
            ClientError::from_status(503, None),
            ClientError::ServerError { .. }
        ));
        assert!(matches!(
            ClientError::from_status(418, None),
            ClientError::Unexpected { .. }
        ));
        // A redirect is the service's doing, not the user's command.
        for status in [301_u16, 302, 307, 308] {
            let error = ClientError::from_status(status, None);
            assert!(
                error.is_unavailable(),
                "HTTP {status} was classified as the user's mistake"
            );
        }
    }

    #[test]
    fn auth_and_availability_are_distinguishable() {
        assert!(ClientError::from_status(401, None).is_auth());
        assert!(!ClientError::from_status(401, None).is_unavailable());
        assert!(ClientError::from_status(503, None).is_unavailable());
        assert!(!ClientError::from_status(422, None).is_unavailable());
    }

    #[test]
    fn api_supplied_text_is_truncated() {
        // A hostile endpoint must not be able to write megabytes to the user's terminal
        // through an error message.
        let long = "x".repeat(MAX_ERROR_BODY_CHARS * 10);
        let error = ClientError::from_status(422, Some(long));
        let rendered = error.to_string();
        assert!(rendered.chars().count() < MAX_ERROR_BODY_CHARS + 100);
        assert!(rendered.ends_with('…'));
    }

    #[test]
    fn short_messages_are_not_marked_as_truncated() {
        let error = ClientError::from_status(422, Some("state: Field required".to_owned()));
        assert_eq!(
            error.to_string(),
            "the API rejected the request as invalid (HTTP 422): state: Field required"
        );
    }

    #[test]
    fn truncation_respects_character_boundaries() {
        // Naive byte slicing here would panic on a multi-byte character.
        let text = "é".repeat(MAX_ERROR_BODY_CHARS * 2);
        let clipped = truncate(&text);
        assert_eq!(clipped.chars().count(), MAX_ERROR_BODY_CHARS + 1);
    }

    #[test]
    fn transient_transport_failures_are_labelled() {
        assert!(TransportError::Timeout { seconds: 10 }.is_transient());
        assert!(
            TransportError::Unreachable {
                reason: "dns".to_owned()
            }
            .is_transient()
        );
        assert!(!TransportError::ResponseTooLarge { limit: 1 }.is_transient());
    }

    /// A key the HTTP library refuses to put in a header never left the machine, so
    /// it is neither an outage nor worth retrying: the same key fails the same way.
    /// Live testing found it reported as "could not reach the API endpoint" after
    /// three attempts.
    #[test]
    fn an_unsendable_credential_is_an_auth_failure_and_never_retried() {
        let transport = TransportError::InvalidCredential;
        assert!(!transport.is_transient());
        let error = ClientError::from(transport);
        assert!(error.is_auth(), "not classified as a credential problem");
        assert!(!error.is_unavailable(), "classified as an outage");
        assert!(!error.to_string().contains("could not reach"), "{error}");
    }
}
