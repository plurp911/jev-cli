//! In-memory transport doubles.
//!
//! These exist so that request shaping, retry policy, and response decoding can be
//! tested exhaustively and deterministically without a network, a live API key, or a
//! recorded-cassette framework.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::credential::Credential;
use crate::error::TransportError;
use crate::transport::{Request, Response, Transport};

/// A [`Transport`] that returns queued responses and records what it was asked to send.
///
/// It deliberately does **not** record the credential. A test double that stored a
/// secret would be the easiest place in the codebase for one to end up in a failure
/// message; instead it records only *that* a credential was supplied.
#[derive(Debug, Default)]
pub struct MockTransport {
    state: Mutex<MockState>,
}

#[derive(Debug, Default)]
struct MockState {
    queued: Vec<Result<Response, TransportError>>,
    observed: Vec<Request>,
    credentials_seen: usize,
}

impl MockTransport {
    /// Creates a transport with no queued responses.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues a successful HTTP response with no headers.
    #[must_use]
    pub fn with_response(self, status: u16, body: impl Into<Vec<u8>>) -> Self {
        self.push(Ok(Response {
            status,
            body: body.into(),
            ..Response::default()
        }));
        self
    }

    /// Queues a response including headers, for behaviour that depends on one —
    /// `retry-after`, for example.
    #[must_use]
    pub fn with_response_headers(
        self,
        status: u16,
        headers: BTreeMap<String, String>,
        body: impl Into<Vec<u8>>,
    ) -> Self {
        self.push(Ok(Response {
            status,
            headers,
            body: body.into(),
        }));
        self
    }

    /// Queues a transport-level failure.
    #[must_use]
    pub fn with_failure(self, error: TransportError) -> Self {
        self.push(Err(error));
        self
    }

    /// Returns the requests this transport was asked to execute, in order.
    #[must_use]
    pub fn observed(&self) -> Vec<Request> {
        self.lock().observed.clone()
    }

    /// How many calls supplied a credential.
    #[must_use]
    pub fn credentials_seen(&self) -> usize {
        self.lock().credentials_seen
    }

    fn push(&self, outcome: Result<Response, TransportError>) {
        self.lock().queued.push(outcome);
    }

    /// A poisoned lock means another test thread panicked while holding it. The
    /// recorded state is still structurally valid, so recovering keeps the real test
    /// failure visible instead of burying it under a lock panic.
    fn lock(&self) -> std::sync::MutexGuard<'_, MockState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Transport for MockTransport {
    fn execute(
        &self,
        request: &Request,
        credential: &Credential,
    ) -> Result<Response, TransportError> {
        let mut state = self.lock();
        state.observed.push(request.clone());
        if !credential.expose().is_empty() {
            state.credentials_seen += 1;
        }
        if state.queued.is_empty() {
            return Err(TransportError::Unreachable {
                reason: "mock transport has no queued response".to_owned(),
            });
        }
        state.queued.remove(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Request {
        Request {
            url: "https://api.typesafe.ai/v1/systemone".to_owned(),
            method: "POST",
            headers: BTreeMap::new(),
            body: Vec::new(),
        }
    }

    fn credential() -> Credential {
        Credential::new("sk-test".to_owned())
    }

    #[test]
    fn returns_queued_responses_in_order() {
        let transport = MockTransport::new()
            .with_response(200, b"first".to_vec())
            .with_response(500, b"second".to_vec());
        assert_eq!(
            transport.execute(&request(), &credential()).unwrap().status,
            200
        );
        assert_eq!(
            transport.execute(&request(), &credential()).unwrap().status,
            500
        );
        assert_eq!(transport.observed().len(), 2);
        assert_eq!(transport.credentials_seen(), 2);
    }

    #[test]
    fn reports_an_empty_queue_rather_than_hanging() {
        let transport = MockTransport::new();
        assert!(transport.execute(&request(), &credential()).is_err());
    }
}
