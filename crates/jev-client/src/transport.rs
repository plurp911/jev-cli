//! The seam between this crate's logic and the outside world.

use std::collections::BTreeMap;
use std::fmt;

use crate::credential::Credential;
use crate::error::TransportError;

/// A single outbound HTTP request, expressed without reference to any HTTP library.
///
/// The credential is deliberately **not** here. It travels separately to
/// [`Transport::execute`] as a [`Credential`], so the plaintext never enters a
/// `Clone`-derived structure that a retry loop could duplicate
/// (`docs/threat-model.md` T2).
///
/// The `Debug` implementation still redacts header values and prints only the body's
/// length: headers may carry other sensitive material in future, and the body carries
/// the user's state.
#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    /// Absolute request URL.
    pub url: String,
    /// HTTP method, uppercase.
    pub method: &'static str,
    /// Non-credential headers, by lowercase name.
    pub headers: BTreeMap<String, String>,
    /// Raw request body, empty for a GET.
    pub body: Vec<u8>,
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("url", &self.url)
            .field("method", &self.method)
            .field("headers", &RedactedHeaderNames(&self.headers))
            .field("body_len", &self.body.len())
            .finish()
    }
}

struct RedactedHeaderNames<'a>(&'a BTreeMap<String, String>);

impl fmt::Debug for RedactedHeaderNames<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.0.keys().map(|name| (name, "<redacted>")))
            .finish()
    }
}

/// A single inbound HTTP response.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Response {
    /// HTTP status code.
    pub status: u16,
    /// Response header names, lowercased, mapped to values.
    pub headers: BTreeMap<String, String>,
    /// Raw response body, already bounded by the transport.
    pub body: Vec<u8>,
}

impl Response {
    /// Looks a header up by lowercase name.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

/// Performs HTTP requests.
///
/// Implementations must not log request or response bodies, must not include header
/// values in error messages, and must bound the number of bytes they read from a
/// response.
pub trait Transport: fmt::Debug + Send + Sync {
    /// Executes `request`, authenticating with `credential`.
    ///
    /// The credential is passed per call rather than held by the transport so that its
    /// lifetime is as short as the request, and so that a transport value can be
    /// inspected in a test without holding a secret.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] when the request could not be completed. A non-2xx
    /// HTTP status is *not* an error at this layer; it is returned as a [`Response`] so
    /// that the caller can decode the API's structured error body.
    fn execute(
        &self,
        request: &Request,
        credential: &Credential,
    ) -> Result<Response, TransportError>;
}

/// Lets a borrowed transport be used wherever an owned one is expected.
///
/// `Client` takes its transport by value, which keeps ownership simple in production.
/// Tests and `jev map` workers need to share one transport across several clients, and
/// this impl makes `&MockTransport` or `&dyn Transport` satisfy the bound without
/// cloning anything or introducing an `Arc`.
impl<T: Transport + ?Sized> Transport for &T {
    fn execute(
        &self,
        request: &Request,
        credential: &Credential,
    ) -> Result<Response, TransportError> {
        (**self).execute(request, credential)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_borrowed_transport_satisfies_the_trait() {
        // Exercises the blanket impl, which `jev map` relies on to share one transport
        // across worker threads.
        fn accepts(_: impl Transport) {}
        #[derive(Debug)]
        struct Null;
        impl Transport for Null {
            fn execute(&self, _: &Request, _: &Credential) -> Result<Response, TransportError> {
                Ok(Response::default())
            }
        }
        accepts(&Null);
    }

    #[test]
    fn request_debug_redacts_header_values() {
        let mut headers = BTreeMap::new();
        headers.insert("x-example".to_owned(), "sk-canary-secret".to_owned());

        let request = Request {
            url: "https://api.typesafe.ai/v1/systemone".to_owned(),
            method: "POST",
            headers,
            body: b"{}".to_vec(),
        };

        let rendered = format!("{request:?}");
        assert!(
            !rendered.contains("sk-canary-secret"),
            "header value leaked: {rendered}"
        );
        assert!(rendered.contains("x-example"));
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn request_debug_omits_body_contents() {
        // Request bodies contain user state, which may itself be sensitive.
        let request = Request {
            url: "https://api.typesafe.ai/v1/systemone".to_owned(),
            method: "POST",
            headers: BTreeMap::new(),
            body: b"patient record: confidential".to_vec(),
        };

        let rendered = format!("{request:?}");
        assert!(
            !rendered.contains("confidential"),
            "user state leaked: {rendered}"
        );
        assert!(rendered.contains("body_len"));
    }

    #[test]
    fn a_request_cannot_carry_a_credential() {
        // Structural, not behavioural: if someone adds an `Authorization` header to a
        // `Request`, this test will not catch it — but `Request` has no field for a
        // `Credential`, and `Credential` is not `Clone`, so the T2 hazard (a retry loop
        // duplicating plaintext) cannot arise through this type.
        let request = Request {
            url: String::new(),
            method: "GET",
            headers: BTreeMap::new(),
            body: Vec::new(),
        };
        let clone = request.clone();
        assert_eq!(request, clone);
    }
}
