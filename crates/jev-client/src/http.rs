//! The concrete blocking HTTPS transport.
//!
//! # Choice of library
//!
//! `ureq` with `rustls`, and nothing else enabled. It is blocking, which ADR-0007
//! requires; it brings no async runtime, so the `tokio` ban in `deny.toml` holds; and
//! it uses `rustls` rather than the system OpenSSL, which `deny.toml` also bans.
//! Automatic decompression and charset transcoding are switched off: both are parsers
//! operating on attacker-controlled bytes, and `jev` needs neither to speak JSON.
//!
//! # What this module is responsible for
//!
//! * Building the `Authorization` header at the last possible moment, from a
//!   [`Credential`], into a buffer that zeroizes (`docs/threat-model.md` T2).
//! * Bounding the response body so a hostile endpoint cannot exhaust memory (T6).
//! * Never putting a header value, a request body, or a response body into an error.

use std::io::Read as _;
use std::time::Duration;

use jev_core::limits::MAX_RESPONSE_BYTES;

/// Idle connections the agent keeps, per host and in total.
///
/// Matches the maximum concurrency `jev map` accepts.
pub const MAX_POOLED_CONNECTIONS: usize = 64;

use crate::credential::Credential;
use crate::error::TransportError;
use crate::transport::{Request, Response, Transport};

/// A blocking HTTPS transport.
#[derive(Debug)]
pub struct HttpTransport {
    agent: ureq::Agent,
    timeout: Duration,
    max_response_bytes: u64,
}

impl HttpTransport {
    /// Builds a transport with the given per-attempt timeout.
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            // A non-2xx status is data, not a transport failure: the caller has to read
            // the API's structured error body to tell a 401 from a 422 from a 529. With
            // ureq's default, every one of those would arrive as an opaque error and
            // `jev` would report "could not reach the API" for a rejected request.
            .http_status_as_error(false)
            // A redirect from an API endpoint is not something to follow silently: it
            // would send the credential to whatever host the `Location` names.
            .max_redirects(0)
            // `ureq`'s default configuration calls `Proxy::try_from_env()`, so without
            // this line `HTTP_PROXY`, `HTTPS_PROXY`, or `ALL_PROXY` -- variables `jev`
            // does not document, does not read, and does not report -- would decide
            // where every request goes. That defeats two invariants this crate states
            // elsewhere: the endpoint module refuses redirects and refuses cleartext to
            // anything but loopback precisely so the destination cannot move, and it
            // justifies the loopback exception with "there is no network to observe".
            // An inherited proxy moves the destination and creates the observer: for a
            // loopback endpoint the agent opens a CONNECT tunnel to the proxy and sends
            // the `Authorization` header through it in the clear.
            //
            // `jev` has no proxy feature. Adding one would need an ADR, a documented
            // variable, a `jev doctor` line, and the same standing warning a custom
            // endpoint gets -- not silent inheritance.
            .proxy(None)
            .timeout_global(Some(timeout))
            // `jev map` runs up to 64 requests in flight through one agent. With the
            // default idle-connection pool of three per host, the rest reconnect on
            // every record -- a TLS handshake each time, for no reason. Sizing the pool
            // to the concurrency cap keeps a batch on warm connections.
            .max_idle_connections_per_host(MAX_POOLED_CONNECTIONS)
            .max_idle_connections(MAX_POOLED_CONNECTIONS)
            .user_agent(crate::user_agent())
            .build();
        Self {
            agent: config.into(),
            timeout,
            max_response_bytes: MAX_RESPONSE_BYTES,
        }
    }

    /// Overrides the response-size cap. Used by tests; the default is
    /// [`MAX_RESPONSE_BYTES`].
    #[must_use]
    pub const fn with_max_response_bytes(mut self, limit: u64) -> Self {
        self.max_response_bytes = limit;
        self
    }
}

impl Default for HttpTransport {
    fn default() -> Self {
        Self::new(Duration::from_secs(10))
    }
}

impl Transport for HttpTransport {
    fn execute(
        &self,
        request: &Request,
        credential: &Credential,
    ) -> Result<Response, TransportError> {
        // The one and only place the plaintext key becomes a header. The buffer is
        // zeroized when this function returns.
        let authorization = credential.bearer_header();
        // Parsed on its own, before the builder, so that a key which cannot be a header
        // value is reported as the credential problem it is. Folded into the builder,
        // it surfaced as "could not reach the API endpoint" -- a transient failure,
        // retried three times -- though nothing was ever sent. The parse error is
        // discarded: it describes the value, and the value is the key.
        let authorization = ureq::http::HeaderValue::from_str(authorization.as_str())
            .map_err(|_| TransportError::InvalidCredential)?;

        let mut builder = ureq::http::Request::builder()
            .method(request.method)
            .uri(request.url.as_str())
            .header("authorization", authorization);
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }

        let http_request =
            builder
                .body(request.body.clone())
                .map_err(|error| TransportError::Unreachable {
                    reason: format!("could not build the request: {error}"),
                })?;

        let response = match self.agent.run(http_request) {
            Ok(response) => response,
            Err(error) => return Err(classify(&error, self.timeout)),
        };

        let status = response.status().as_u16();
        let mut headers = std::collections::BTreeMap::new();
        for (name, value) in response.headers() {
            // Header values are bytes; a non-UTF-8 one is dropped rather than
            // lossily converted, because nothing `jev` reads needs it.
            if let Ok(text) = value.to_str() {
                headers.insert(name.as_str().to_ascii_lowercase(), text.to_owned());
            }
        }

        // Read through a hard cap. `Content-Length` is attacker-controlled and is
        // therefore never used to size an allocation; the limiter is the only bound.
        let mut body = Vec::new();
        let limit = self.max_response_bytes;
        let mut reader = response.into_body().into_reader().take(limit + 1);
        reader.read_to_end(&mut body).map_err(|error| {
            // A body that stalls past the deadline arrives here as an `io::Error`, and
            // `ureq` wraps a non-`Io` failure -- including its own `Timeout` -- as
            // `ErrorKind::Other`. Reporting that as "could not reach the API endpoint:
            // other error" is wrong twice over: the endpoint was reached, and the one
            // thing that would help, `--timeout`, does not occur to the reader.
            // `classify` already knows how to tell these apart, so the error is routed
            // through it rather than flattened here.
            match error
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<ureq::Error>())
            {
                Some(ureq_error) => classify(ureq_error, self.timeout),
                None if error.kind() == std::io::ErrorKind::TimedOut => TransportError::Timeout {
                    seconds: self.timeout.as_secs(),
                },
                None => TransportError::Unreachable {
                    reason: format!("could not read the response body: {}", error.kind()),
                },
            }
        })?;
        if body.len() as u64 > limit {
            return Err(TransportError::ResponseTooLarge { limit });
        }

        Ok(Response {
            status,
            headers,
            body,
        })
    }
}

/// Maps a `ureq` failure onto a transport error, without letting a header value or a
/// body fragment into the message.
fn classify(error: &ureq::Error, timeout: Duration) -> TransportError {
    match error {
        ureq::Error::Timeout(_) => TransportError::Timeout {
            seconds: timeout.as_secs(),
        },
        ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => {
            TransportError::Timeout {
                seconds: timeout.as_secs(),
            }
        }
        ureq::Error::Io(io) => TransportError::Unreachable {
            reason: io.kind().to_string(),
        },
        // Every remaining variant names a class of failure — TLS, DNS, protocol — and
        // none of them embeds request or response content.
        other => TransportError::Unreachable {
            reason: other.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_transport_can_be_built_without_touching_the_network() {
        let transport = HttpTransport::new(Duration::from_secs(1));
        assert_eq!(transport.max_response_bytes, MAX_RESPONSE_BYTES);
    }

    #[test]
    fn timeouts_are_classified_as_timeouts() {
        let error = ureq::Error::Io(std::io::Error::from(std::io::ErrorKind::TimedOut));
        assert_eq!(
            classify(&error, Duration::from_secs(7)),
            TransportError::Timeout { seconds: 7 }
        );
    }

    #[test]
    fn io_failures_report_only_their_kind() {
        // Not the OS message, which on some platforms includes a path or a hostname.
        let error = ureq::Error::Io(std::io::Error::other("connection refused to secret.host"));
        let classified = classify(&error, Duration::from_secs(1));
        assert!(!classified.to_string().contains("secret.host"));
    }

    /// A key with a line break or control character cannot be a header value. That is
    /// known before anything is sent, so it must fail as a credential problem and must
    /// not be retried. The endpoint is a loopback listener that is never accepted from:
    /// if a request were sent anyway, the test stays offline and the connection is left
    /// in the listener's queue, where the final assertion finds it.
    #[test]
    fn a_key_that_cannot_be_a_header_is_a_credential_error_not_an_outage() {
        const CANARY: &str = "sk-canary-header-0123456789";
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let transport = HttpTransport::new(Duration::from_millis(500));
        let request = Request {
            url: format!("http://{}/v1/models", listener.local_addr().expect("addr")),
            method: "GET",
            headers: std::collections::BTreeMap::new(),
            body: Vec::new(),
        };
        for key in [
            format!("{CANARY}\u{1}ctl"),
            format!("{CANARY}\nsecond-line"),
            format!("{CANARY}\r\n"),
        ] {
            let error = transport
                .execute(&request, &Credential::new(key))
                .unwrap_err();
            assert_eq!(error, TransportError::InvalidCredential);
            assert!(!error.is_transient());
            assert!(!error.to_string().contains(CANARY), "key leaked: {error}");
        }
        let accepted = listener.accept().map(|_| ()).map_err(|error| error.kind());
        assert_eq!(
            accepted,
            Err(std::io::ErrorKind::WouldBlock),
            "a connection was opened for a key that cannot be sent"
        );
    }

    /// The live network tests for this transport are in
    /// `crates/jev-cli/tests/`, driven against a local mock HTTP server on loopback.
    /// Nothing here contacts the internet.
    #[test]
    fn unreachable_hosts_fail_rather_than_hang() {
        // `.invalid` is reserved by RFC 2606 and never resolves, so this exercises the
        // DNS-failure path without depending on the outside world.
        let transport = HttpTransport::new(Duration::from_millis(500));
        let request = Request {
            url: "https://nonexistent.invalid/v1/models".to_owned(),
            method: "GET",
            headers: std::collections::BTreeMap::new(),
            body: Vec::new(),
        };
        let error = transport
            .execute(&request, &Credential::new("sk-test".to_owned()))
            .unwrap_err();
        assert!(error.is_transient());
    }
}
