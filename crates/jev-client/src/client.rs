//! Request construction, retry orchestration, and response dispatch.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use jev_core::{EvaluationRequest, EvaluationResponse, ModelCard};

use crate::credential::Credential;
use crate::endpoint::{Endpoint, MODELS_PATH, SYSTEM_ONE_PATH};
use crate::error::{ClientError, TransportError};
use crate::retry::{Outcome, RetryDecision, RetryPolicy};
use crate::transport::{Request, Response, Transport};
use crate::wire;

/// The `User-Agent` this CLI identifies itself with.
///
/// A distinct agent string lets TypeSafe distinguish this community tool from their own
/// SDKs in their logs, which matters if `jev` ever misbehaves at scale. It carries the
/// version and nothing about the user or the machine.
#[must_use]
pub fn user_agent() -> String {
    format!("jev-cli/{}", env!("CARGO_PKG_VERSION"))
}

/// Something that can wait and tell the time.
///
/// Injected so the retry loop can be tested at full speed with no sleeping, which is
/// what keeps the suite deterministic (`AGENTS.md` §9).
pub trait Clock: std::fmt::Debug + Send + Sync {
    /// The current instant.
    fn now(&self) -> Instant;
    /// Blocks for `duration`.
    fn sleep(&self, duration: Duration);
    /// A jitter sample in `[0, 1)`.
    fn jitter_sample(&self) -> f64;

    /// Whether the caller has been asked to stop.
    ///
    /// Consulted after every wait, so an interrupt that arrives during a backoff ends
    /// the loop instead of being noticed only after the remaining attempts have run.
    /// `jev-client` has no signal handling of its own — it cannot, without becoming the
    /// process — so the answer comes from the caller, which does.
    ///
    /// The default is `false`, which is exactly the old behaviour for every clock that
    /// has nothing to report.
    fn cancelled(&self) -> bool {
        false
    }
}

/// The real clock: `std::time` and `std::thread::sleep`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }

    /// A cheap, non-cryptographic jitter source.
    ///
    /// Derived from the current nanosecond, which is enough to decorrelate retries
    /// between processes. It is deliberately not a random-number-generator dependency:
    /// jitter is not a security property, and the official SDK uses an ordinary PRNG
    /// for the same purpose.
    fn jitter_sample(&self) -> f64 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.subsec_nanos());
        f64::from(nanos) / 1_000_000_000.0
    }
}

/// What happened during one API call, for diagnostics.
///
/// Carries no request or response content — only shape, timing, and the API's own
/// identifier for the call — so that printing it cannot disclose the user's state or
/// the API's reply.
///
/// Not `Copy`: `request_id` is an owned `String`. It is small and cloned once per call.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CallStats {
    /// How many HTTP attempts were made, including the first.
    pub attempts: u32,
    /// Wall-clock time for the whole call, including waits between attempts.
    pub elapsed: Duration,
    /// The final HTTP status, when a response arrived.
    pub status: Option<u16>,
    /// The API's own identifier for the final attempt, from `x-typesafe-request-id`.
    ///
    /// Documented at <https://docs.typesafe.ai/sdk/python/api/exceptions.md>, where the
    /// official SDK exposes it as `TypeSafeAPIError.request_id` and appends it to every
    /// API error message. It is an opaque support identifier, not credential material,
    /// and it is the one thing TypeSafe can use to find a specific call — without it a
    /// user whose batch row failed has nothing to hand support.
    ///
    /// `None` when no response arrived, or when the API did not send the header.
    pub request_id: Option<String>,
}

/// A System One API client.
///
/// The client holds no credential: one is supplied per call. That keeps a secret's
/// lifetime as short as the request that needs it, and means a `Client` can be built,
/// inspected, and passed around in tests without holding one.
#[derive(Debug)]
pub struct Client<T: Transport, C: Clock = SystemClock> {
    transport: T,
    clock: C,
    endpoint: Endpoint,
    retry: RetryPolicy,
}

impl<T: Transport> Client<T> {
    /// Builds a client against `endpoint` with the default retry policy and the real
    /// clock.
    pub fn new(transport: T, endpoint: Endpoint) -> Self {
        Self {
            transport,
            clock: SystemClock,
            endpoint,
            retry: RetryPolicy::default(),
        }
    }
}

impl<T: Transport, C: Clock> Client<T, C> {
    /// Builds a client with an explicit clock, for tests.
    pub const fn with_clock(transport: T, endpoint: Endpoint, clock: C) -> Self {
        Self {
            transport,
            clock,
            endpoint,
            retry: RetryPolicy {
                max_retries: 2,
                initial_backoff: Duration::from_millis(500),
                max_backoff: Duration::from_secs(5),
                jitter: 0.25,
                respect_retry_after: true,
                total_budget: Duration::from_secs(30),
            },
        }
    }

    /// Replaces the retry policy.
    #[must_use]
    pub const fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// The endpoint this client talks to.
    pub const fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// The retry policy in force.
    pub const fn retry_policy(&self) -> &RetryPolicy {
        &self.retry
    }

    /// Builds the exact HTTP request an evaluation would send.
    ///
    /// Exposed so that `--dry-run` shows the user the real bytes rather than a
    /// reconstruction. The credential is not part of a [`Request`], so this value is
    /// safe to print in full.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::MalformedResponse`] only if the request cannot be
    /// serialized, which no constructible request can trigger.
    pub fn build_evaluation_request(
        &self,
        request: &EvaluationRequest,
    ) -> Result<Request, ClientError> {
        build_evaluation_request(&self.endpoint, request)
    }

    /// Evaluates a request.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] for a transport failure, a non-2xx status the retry
    /// policy did not resolve, or a response this version cannot decode.
    pub fn evaluate(
        &self,
        request: &EvaluationRequest,
        credential: &Credential,
    ) -> (Result<EvaluationResponse, ClientError>, CallStats) {
        let http = match self.build_evaluation_request(request) {
            Ok(http) => http,
            Err(error) => return (Err(error), CallStats::default()),
        };
        let (outcome, stats) = self.send(&http, credential);
        (
            outcome.and_then(|response| wire::decode_evaluation(&response.body)),
            stats,
        )
    }

    /// Lists the models the account may use.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] as [`Client::evaluate`] does.
    pub fn models(
        &self,
        credential: &Credential,
    ) -> (Result<Vec<ModelCard>, ClientError>, CallStats) {
        let http = Request {
            url: self.endpoint.url_for(MODELS_PATH),
            method: "GET",
            headers: json_headers(false),
            body: Vec::new(),
        };
        let (outcome, stats) = self.send(&http, credential);
        (
            outcome.and_then(|response| wire::decode_models(&response.body)),
            stats,
        )
    }

    /// Runs one logical call: attempt, classify, wait, repeat within the budget.
    fn send(
        &self,
        request: &Request,
        credential: &Credential,
    ) -> (Result<Response, ClientError>, CallStats) {
        let started = self.clock.now();
        let mut attempt: u32 = 0;
        let mut last: Result<Response, TransportError>;

        loop {
            attempt = attempt.saturating_add(1);
            last = self.transport.execute(request, credential);

            let elapsed = self.clock.now().saturating_duration_since(started);
            let outcome = match &last {
                Ok(response) => Outcome::Status(response),
                Err(error) => Outcome::Transport(error),
            };

            match self
                .retry
                .decide(attempt, outcome, elapsed, self.clock.jitter_sample())
            {
                RetryDecision::Stop => break,
                RetryDecision::RetryAfter(delay) => {
                    self.clock.sleep(delay);
                    // A real clock's `sleep` returns early when the caller has been
                    // interrupted. Without this check the loop would simply start the
                    // next attempt, so Ctrl-C during a five-second `Retry-After` was
                    // swallowed: the process ran every remaining attempt and exited 4,
                    // while `main.rs` promised it would stop and report 130.
                    if self.clock.cancelled() {
                        break;
                    }
                }
            }
        }

        let elapsed = self.clock.now().saturating_duration_since(started);
        let final_status = last.as_ref().ok().map(|response| response.status);
        // From the attempt that actually ended the call, which is the one support will
        // be asked about.
        let request_id = last
            .as_ref()
            .ok()
            .and_then(|response| response.header(REQUEST_ID_HEADER))
            .map(|id| redact(id, credential));
        let stats = CallStats {
            attempts: attempt,
            elapsed,
            status: final_status,
            request_id,
        };

        let result = match last {
            Err(error) => Err(ClientError::Transport(error)),
            Ok(response) if (200..300).contains(&response.status) => Ok(response),
            Ok(response) => Err(ClientError::from_status(
                response.status,
                // Redacted in full and only then truncated (by `from_status`): redacting
                // a clipped message left the head of a key that straddled the limit.
                wire::extract_error_text(&response.body)
                    .map(|message| redact(&message, credential)),
            )),
        };
        (result, stats)
    }
}

/// Builds the exact HTTP request an evaluation would send, without a client.
///
/// [`Client::build_evaluation_request`] delegates here, and so does `--dry-run`, which
/// has no transport to construct a [`Client`] around. One function means a dry run
/// cannot describe a request that differs from the one a real run would send: the URL,
/// the method, the header set, and the body bytes all come from here in both cases.
///
/// The credential is not part of a [`Request`], so the result is safe to print in full.
///
/// # Errors
///
/// Returns [`ClientError::MalformedResponse`] only if the request cannot be
/// serialized, which no constructible request can trigger.
pub fn build_evaluation_request(
    endpoint: &Endpoint,
    request: &EvaluationRequest,
) -> Result<Request, ClientError> {
    let body = serde_json::to_vec(request).map_err(|error| ClientError::MalformedResponse {
        reason: format!("could not encode the request body: {error}"),
    })?;
    Ok(Request {
        url: endpoint.url_for(SYSTEM_ONE_PATH),
        method: "POST",
        headers: json_headers(true),
        body,
    })
}

/// The response header carrying the API's identifier for a call.
///
/// From <https://docs.typesafe.ai/sdk/python/api/exceptions.md>. Lowercased because
/// `Transport` implementations lowercase header names.
pub const REQUEST_ID_HEADER: &str = "x-typesafe-request-id";

/// The non-credential headers every request carries.
fn json_headers(has_body: bool) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    headers.insert("accept".to_owned(), "application/json".to_owned());
    headers.insert("user-agent".to_owned(), user_agent());
    if has_body {
        headers.insert("content-type".to_owned(), "application/json".to_owned());
    }
    headers
}

/// Removes the credential from API-supplied text before it can be shown to anyone.
///
/// An error body is untrusted, and an endpoint -- a misbehaving proxy, a custom host, a
/// debugging echo server -- can quote the `Authorization` header back. That text becomes
/// an error message, which the CLI prints to a terminal and `jev mcp serve` returns into
/// an agent's context window. Neither may carry the key (`AGENTS.md` §2), so it is
/// replaced here, below both of them, in the whole decoded message before it is clipped,
/// so neither a JSON-escaped key nor one straddling the length limit survives. The
/// request-id header is untrusted text too and is redacted the same way.
fn redact(message: &str, credential: &Credential) -> String {
    let secret = credential.expose();
    if secret.is_empty() {
        return message.to_owned();
    }
    message.replace(secret, "<redacted>")
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::Mutex;

    use jev_core::{Content, ModelId, Question, QuestionId, State};
    use serde_json::json;

    use super::*;
    use crate::testing::MockTransport;

    /// A clock that never really sleeps; it advances a virtual instant instead.
    #[derive(Debug)]
    struct FakeClock {
        state: Mutex<RefCell<Duration>>,
        base: Instant,
    }

    impl FakeClock {
        fn new() -> Self {
            Self {
                state: Mutex::new(RefCell::new(Duration::ZERO)),
                base: Instant::now(),
            }
        }

        fn slept(&self) -> Duration {
            let guard = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *guard.borrow()
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            self.base + self.slept()
        }

        fn sleep(&self, duration: Duration) {
            let guard = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let current = *guard.borrow();
            *guard.borrow_mut() = current + duration;
        }

        fn jitter_sample(&self) -> f64 {
            0.0
        }
    }

    fn credential() -> Credential {
        Credential::new("sk-canary-client-must-not-appear".to_owned())
    }

    fn request() -> EvaluationRequest {
        EvaluationRequest::new(
            State::text("a ticket").unwrap(),
            ModelId::default(),
            vec![(
                QuestionId::new("urgent").unwrap(),
                Question::noul(Content::text("Urgent?").unwrap(), None).unwrap(),
            )],
        )
        .unwrap()
    }

    fn success_body() -> Vec<u8> {
        json!({
            "model": "jev-1.13.0",
            "answers": {"urgent": {"type": "noul", "noul": 0.92}},
            "usage": {"input_tokens": 312, "output_tokens": 48}
        })
        .to_string()
        .into_bytes()
    }

    fn client(transport: MockTransport) -> Client<MockTransport, FakeClock> {
        Client::with_clock(transport, Endpoint::official(), FakeClock::new())
    }

    #[test]
    fn a_successful_evaluation_decodes() {
        let transport = MockTransport::new().with_response(200, success_body());
        let (result, stats) = client(transport).evaluate(&request(), &credential());
        let response = result.unwrap();
        assert_eq!(response.model.as_str(), "jev-1.13.0");
        assert_eq!(response.usage.input_tokens, Some(312));
        assert_eq!(stats.attempts, 1);
        assert_eq!(stats.status, Some(200));
    }

    #[test]
    fn the_request_matches_the_documented_wire_form() {
        let transport = MockTransport::new().with_response(200, success_body());
        let client = client(transport);
        let _ = client.evaluate(&request(), &credential());
        let observed = client.transport_for_test().observed();
        let sent = observed.first().expect("one request");

        assert_eq!(sent.method, "POST");
        assert_eq!(sent.url, "https://api.typesafe.ai/v1/systemone");
        assert_eq!(
            sent.headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
        assert!(
            sent.headers
                .get("user-agent")
                .is_some_and(|agent| agent.starts_with("jev-cli/"))
        );
        let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
        assert_eq!(
            body,
            json!({
                "state": "a ticket",
                "model": "jev-latest",
                "questions": {"urgent": {"type": "noul", "instructions": "Urgent?"}}
            })
        );
    }

    #[test]
    fn a_credential_quoted_back_in_an_error_body_is_redacted() {
        let body = r#"{"detail":"bad key sk-canary-client-must-not-appear in header"}"#;
        let transport = MockTransport::new().with_response(401, body.as_bytes().to_vec());
        let (result, _) = client(transport).evaluate(&request(), &credential());
        let rendered = result.unwrap_err().to_string();
        assert!(
            !rendered.contains("sk-canary-client-must-not-appear"),
            "the echoed key survived: {rendered}"
        );
        assert!(
            rendered.contains("bad key <redacted> in header"),
            "{rendered}"
        );
    }

    #[test]
    fn a_credential_straddling_the_message_limit_leaves_no_prefix() {
        let secret = "sk-canary-client-must-not-appear";
        let padding = "x".repeat(jev_core::limits::MAX_ERROR_BODY_CHARS - 10);
        let body = format!(r#"{{"detail":"{padding} {secret}"}}"#);
        let transport = MockTransport::new().with_response(401, body.into_bytes());
        let (result, _) = client(transport).evaluate(&request(), &credential());
        let rendered = result.unwrap_err().to_string();
        assert!(
            !rendered.contains("sk-canary-cl"),
            "a prefix survived: {rendered}"
        );
    }

    #[test]
    fn a_json_escaped_credential_straddling_the_limit_leaves_no_prefix() {
        // `\u0073` is `s`: the raw body does not contain the key, the decoded one does.
        let padding = "x".repeat(jev_core::limits::MAX_ERROR_BODY_CHARS - 10);
        let body = format!(r#"{{"detail":"{padding} \u0073k-canary-client-must-not-appear"}}"#);
        let transport = MockTransport::new().with_response(401, body.into_bytes());
        let (result, _) = client(transport).evaluate(&request(), &credential());
        let rendered = result.unwrap_err().to_string();
        assert!(
            !rendered.contains("sk-canary-cl"),
            "a prefix survived: {rendered}"
        );
    }

    #[test]
    fn a_credential_echoed_in_the_request_id_header_is_redacted() {
        let headers = std::iter::once((
            "x-typesafe-request-id".to_owned(),
            "req sk-canary-client-must-not-appear".to_owned(),
        ))
        .collect();
        let transport = MockTransport::new().with_response_headers(200, headers, success_body());
        let (_, stats) = client(transport).evaluate(&request(), &credential());
        assert_eq!(stats.request_id.as_deref(), Some("req <redacted>"));
    }

    #[test]
    fn the_credential_never_enters_the_request_struct() {
        // The T2 invariant, asserted end to end rather than only at the type level.
        let transport = MockTransport::new().with_response(200, success_body());
        let client = client(transport);
        let _ = client.evaluate(&request(), &credential());
        let observed = client.transport_for_test().observed();
        let rendered = format!("{observed:?}");
        assert!(
            !rendered.contains("sk-canary-client-must-not-appear"),
            "credential reached the recorded request: {rendered}"
        );
        for request in &observed {
            for (name, value) in &request.headers {
                assert!(!value.contains("sk-canary"), "credential in header {name}");
            }
        }
    }

    #[test]
    fn models_uses_a_get_against_the_documented_path() {
        let body = json!({
            "models": [
                {"name": "jev-latest", "description": "flagship", "release_date": "2026-09-15"}
            ]
        })
        .to_string();
        let transport = MockTransport::new().with_response(200, body.into_bytes());
        let client = client(transport);
        let (models, _) = client.models(&credential());
        let models = models.unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "jev-latest");

        let observed = client.transport_for_test().observed();
        assert_eq!(observed[0].method, "GET");
        assert_eq!(observed[0].url, "https://api.typesafe.ai/v1/models");
        assert!(observed[0].body.is_empty());
        // No body means no Content-Type: sending one on a GET is a small but real
        // correctness wart that some proxies reject.
        assert!(!observed[0].headers.contains_key("content-type"));
    }

    #[test]
    fn a_429_is_retried_and_then_succeeds() {
        let transport = MockTransport::new()
            .with_response_headers(
                429,
                [("retry-after-ms".to_owned(), "10".to_owned())].into(),
                Vec::new(),
            )
            .with_response(200, success_body());
        let client = client(transport);
        let (result, stats) = client.evaluate(&request(), &credential());
        assert!(result.is_ok());
        assert_eq!(stats.attempts, 2);
        assert_eq!(client.clock_for_test().slept(), Duration::from_millis(10));
    }

    #[test]
    fn a_401_is_not_retried() {
        let transport = MockTransport::new()
            .with_response(401, br#"{"detail":"Invalid API key"}"#.to_vec())
            .with_response(200, success_body());
        let client = client(transport);
        let (result, stats) = client.evaluate(&request(), &credential());
        let error = result.unwrap_err();
        assert!(error.is_auth());
        assert_eq!(stats.attempts, 1, "an authentication failure was retried");
        assert!(error.to_string().contains("Invalid API key"));
    }

    #[test]
    fn a_422_reports_the_offending_field() {
        let body = json!({
            "detail": [{"loc": ["body", "questions", "urgency", "criteria"], "msg": "Field required"}]
        })
        .to_string();
        let transport = MockTransport::new().with_response(422, body.into_bytes());
        let (result, _) = client(transport).evaluate(&request(), &credential());
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("questions.urgency.criteria: Field required"),
            "{message}"
        );
    }

    #[test]
    fn retries_are_bounded_and_then_the_error_surfaces() {
        let transport = MockTransport::new()
            .with_response(503, Vec::new())
            .with_response(503, Vec::new())
            .with_response(503, Vec::new())
            .with_response(200, success_body());
        let client = client(transport);
        let (result, stats) = client.evaluate(&request(), &credential());
        assert!(result.unwrap_err().is_unavailable());
        // Two retries after the first attempt: the queued success is never reached.
        assert_eq!(stats.attempts, 3);
    }

    #[test]
    fn a_connection_failure_is_retried_then_reported() {
        let transport = MockTransport::new()
            .with_failure(TransportError::Unreachable {
                reason: "dns failure".to_owned(),
            })
            .with_response(200, success_body());
        let client = client(transport);
        let (result, stats) = client.evaluate(&request(), &credential());
        assert!(result.is_ok());
        assert_eq!(stats.attempts, 2);
    }

    #[test]
    fn a_malformed_body_is_a_decode_error_not_a_panic() {
        for body in [
            b"not json".to_vec(),
            b"{}".to_vec(),
            br#"{"model":"jev-latest","answers":{"urgent":{"type":"noul","noul":5}}}"#.to_vec(),
            vec![0xff, 0xfe, 0xfd],
        ] {
            let transport = MockTransport::new().with_response(200, body);
            let (result, _) = client(transport).evaluate(&request(), &credential());
            assert!(matches!(result, Err(ClientError::MalformedResponse { .. })));
        }
    }

    #[test]
    fn dry_run_shows_the_real_body() {
        let client = client(MockTransport::new());
        let built = client.build_evaluation_request(&request()).unwrap();
        assert_eq!(built.url, "https://api.typesafe.ai/v1/systemone");
        let rendered = String::from_utf8(built.body).unwrap();
        assert!(rendered.contains("\"model\":\"jev-latest\""));
        assert!(!rendered.contains("Bearer"));
    }

    #[test]
    fn a_custom_endpoint_is_used_verbatim() {
        let endpoint = Endpoint::parse("http://127.0.0.1:9999/base").unwrap();
        let client = Client::with_clock(
            MockTransport::new().with_response(200, success_body()),
            endpoint,
            FakeClock::new(),
        );
        let _ = client.evaluate(&request(), &credential());
        assert_eq!(
            client.transport_for_test().observed()[0].url,
            "http://127.0.0.1:9999/base/v1/systemone"
        );
    }

    impl<T: Transport, C: Clock> Client<T, C> {
        fn transport_for_test(&self) -> &T {
            &self.transport
        }

        fn clock_for_test(&self) -> &C {
            &self.clock
        }
    }
}
