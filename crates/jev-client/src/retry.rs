//! Retry policy.
//!
//! # Design
//!
//! The policy is a **pure function** of (attempt number, outcome, response headers).
//! Deciding and waiting are separate: [`RetryPolicy::decide`] returns how long to wait
//! and the caller does the waiting. Nothing here sleeps, reads a clock, or generates
//! randomness on its own, so the entire policy — including backoff growth, the
//! `Retry-After` path, and budget exhaustion — is unit-tested without a single
//! `thread::sleep`, which is what `AGENTS.md` §9 requires of a deterministic suite.
//!
//! # Defaults
//!
//! Taken from the official Python SDK, so that `jev` behaves like the SDKs a user may
//! already have calibrated against: two retries, 0.5 s initial backoff doubling to a
//! 5 s ceiling, 25 % jitter, and a 30 s total budget, all from its `RetryPolicy`
//! (<https://docs.typesafe.ai/sdk/python/api/retries.md>). Retryable statuses are 408,
//! 429, and 5xx; `Retry-After` and `retry-after-ms` are honoured when present.
//!
//! The 10 s **per-attempt** timeout is not part of `RetryPolicy` and is set elsewhere
//! in `jev` (`jev_cli::context`); its source is the SDK's `DEFAULT_TIMEOUT`
//! (<https://docs.typesafe.ai/sdk/python/api/constants.md>), "the default timeout in
//! seconds for each HTTP operation".
//!
//! Permanent failures — 400, 401, 403, 404, 422 — are never retried. Retrying an
//! authentication failure is how a rate limit becomes an account lockout.

use std::time::Duration;

use crate::error::TransportError;
use crate::transport::Response;

/// Header carrying a retry delay in seconds, or an HTTP date.
const RETRY_AFTER: &str = "retry-after";
/// Header carrying a retry delay in milliseconds. Used by the official SDKs.
const RETRY_AFTER_MS: &str = "retry-after-ms";

/// Most retries `jev` will perform after the first attempt.
///
/// Without a cap, raising the retry count also raises the total budget that is supposed
/// to bound it — `--retries 4294967295` produced a budget of roughly 1,361 years and an
/// attempt counter that saturated below the limit, so the loop never terminated. Ten
/// retries with the default backoff is already well over a minute of waiting; beyond
/// that the right answer is to run the command again, not to wait longer.
pub const MAX_RETRIES: u32 = 10;

/// Upper bound on any honoured `Retry-After`.
///
/// A hostile or misconfigured endpoint can ask a client to sleep for a year. `jev`
/// clamps to its own maximum backoff instead of obeying, and gives up rather than
/// waiting beyond the total budget.
const MAX_HONOURED_RETRY_AFTER: Duration = Duration::from_secs(60);

/// What the caller should do after an attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    /// Stop: the outcome is final, whether success or a permanent failure.
    Stop,
    /// Wait this long, then try again.
    RetryAfter(Duration),
}

/// Why a retryable attempt failed, as far as the policy needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome<'a> {
    /// An HTTP response arrived.
    Status(&'a Response),
    /// No response arrived.
    Transport(&'a TransportError),
}

/// How `jev` retries.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetryPolicy {
    /// Attempts after the first. `0` disables retrying.
    pub max_retries: u32,
    /// The first backoff delay, doubled on each subsequent attempt.
    pub initial_backoff: Duration,
    /// Ceiling on a single backoff delay.
    pub max_backoff: Duration,
    /// Fraction of each backoff randomly subtracted, in `[0, 1]`.
    ///
    /// Jitter prevents a fleet of clients that were throttled together from retrying
    /// in lockstep and re-creating the overload.
    pub jitter: f64,
    /// Whether to honour `Retry-After` and `retry-after-ms`.
    pub respect_retry_after: bool,
    /// Total wall-clock budget for one logical call, including waits.
    ///
    /// Bounds the worst case absolutely: `jev` never retries indefinitely.
    pub total_budget: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(5),
            jitter: 0.25,
            respect_retry_after: true,
            total_budget: Duration::from_secs(30),
        }
    }
}

impl RetryPolicy {
    /// A policy that never retries, for `--retries 0` and for tests that assert a
    /// single attempt.
    #[must_use]
    pub fn none() -> Self {
        Self {
            max_retries: 0,
            ..Self::default()
        }
    }

    /// Clamps the retry count to [`MAX_RETRIES`].
    ///
    /// Applied on construction from user input, so the invariant that the budget bounds
    /// the loop cannot be broken by a flag.
    #[must_use]
    pub const fn clamp_retries(requested: u32) -> u32 {
        if requested > MAX_RETRIES {
            MAX_RETRIES
        } else {
            requested
        }
    }

    /// Decides what to do after attempt number `attempt` (1-based).
    ///
    /// `elapsed` is how long the logical call has taken so far, and `jitter_sample` is
    /// a value in `[0, 1)` supplied by the caller — injected rather than generated so
    /// that the decision is a pure function and the tests are deterministic.
    #[must_use]
    pub fn decide(
        &self,
        attempt: u32,
        outcome: Outcome<'_>,
        elapsed: Duration,
        jitter_sample: f64,
    ) -> RetryDecision {
        if attempt > self.max_retries {
            return RetryDecision::Stop;
        }
        if !self.is_retryable(outcome) {
            return RetryDecision::Stop;
        }

        let delay = self.delay_for(attempt, outcome, jitter_sample);

        // Stop before a wait that would reach or exceed the budget, mirroring the
        // official SDK's `stop_before_delay`. Waiting past the budget and then failing
        // wastes the user's time for no chance of success.
        if elapsed.saturating_add(delay) >= self.total_budget {
            return RetryDecision::Stop;
        }
        RetryDecision::RetryAfter(delay)
    }

    /// Whether this outcome is worth another attempt.
    #[must_use]
    pub fn is_retryable(&self, outcome: Outcome<'_>) -> bool {
        match outcome {
            Outcome::Transport(error) => error.is_transient(),
            Outcome::Status(response) => is_retryable_status(response.status),
        }
    }

    fn delay_for(&self, attempt: u32, outcome: Outcome<'_>, jitter_sample: f64) -> Duration {
        if self.respect_retry_after
            && let Outcome::Status(response) = outcome
            && let Some(hint) = parse_retry_after(response)
        {
            return hint.min(MAX_HONOURED_RETRY_AFTER);
        }
        self.backoff(attempt, jitter_sample)
    }

    /// Exponential backoff with subtractive jitter, capped at `max_backoff`.
    #[must_use]
    pub fn backoff(&self, attempt: u32, jitter_sample: f64) -> Duration {
        if self.initial_backoff.is_zero() || self.max_backoff.is_zero() {
            return Duration::ZERO;
        }
        let exponent = attempt.saturating_sub(1).min(32);
        let scaled = self
            .initial_backoff
            .saturating_mul(1_u32.checked_shl(exponent).unwrap_or(u32::MAX))
            .min(self.max_backoff);

        let sample = if jitter_sample.is_finite() {
            jitter_sample.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let factor = 1.0 - sample * self.jitter.clamp(0.0, 1.0);
        scaled.mul_f64(factor.clamp(0.0, 1.0))
    }
}

/// Statuses worth retrying.
///
/// 408 Request Timeout, 429 Too Many Requests, and every 5xx — which includes the
/// 529 Overloaded the HTTP API reference documents. Everything else is the server
/// telling us the request itself is wrong, and repeating it cannot help.
#[must_use]
pub fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 429) || (500..600).contains(&status)
}

/// Reads a retry hint from a response, preferring the millisecond form.
///
/// Mirrors the official SDK: `retry-after-ms` wins, then `retry-after` as a decimal
/// number of seconds. A negative or non-finite value is ignored. The HTTP-date form of
/// `Retry-After` is deliberately **not** honoured: parsing it requires a clock, which
/// would make retry behaviour non-deterministic and untestable, and the API's own SDKs
/// send the numeric form.
#[must_use]
pub fn parse_retry_after(response: &Response) -> Option<Duration> {
    if let Some(raw) = response.header(RETRY_AFTER_MS)
        && let Some(delay) = parse_delay(raw, 0.001)
    {
        return Some(delay);
    }
    response
        .header(RETRY_AFTER)
        .and_then(|raw| parse_delay(raw, 1.0))
}

fn parse_delay(raw: &str, seconds_per_unit: f64) -> Option<Duration> {
    let value: f64 = raw.trim().parse().ok()?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    Duration::try_from_secs_f64(value * seconds_per_unit).ok()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use proptest::prelude::*;

    use super::*;

    fn response(status: u16, headers: &[(&str, &str)]) -> Response {
        Response {
            status,
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            body: Vec::new(),
        }
    }

    #[test]
    fn permanent_failures_are_never_retried() {
        // Retrying a 401 turns a credential mistake into a lockout, and retrying a 422
        // sends the same invalid body again at the user's expense.
        let policy = RetryPolicy::default();
        for status in [400_u16, 401, 403, 404, 422] {
            let attempted = response(status, &[]);
            assert_eq!(
                policy.decide(1, Outcome::Status(&attempted), Duration::ZERO, 0.0),
                RetryDecision::Stop,
                "HTTP {status} was retried"
            );
        }
    }

    #[test]
    fn transient_failures_are_retried() {
        let policy = RetryPolicy::default();
        for status in [408_u16, 429, 500, 502, 503, 529] {
            let attempted = response(status, &[]);
            assert!(
                matches!(
                    policy.decide(1, Outcome::Status(&attempted), Duration::ZERO, 0.0),
                    RetryDecision::RetryAfter(_)
                ),
                "HTTP {status} was not retried"
            );
        }
    }

    #[test]
    fn a_success_stops() {
        let policy = RetryPolicy::default();
        assert_eq!(
            policy.decide(1, Outcome::Status(&response(200, &[])), Duration::ZERO, 0.0),
            RetryDecision::Stop
        );
    }

    #[test]
    fn retries_stop_at_the_configured_count() {
        let policy = RetryPolicy {
            max_retries: 2,
            ..RetryPolicy::default()
        };
        let attempted = response(503, &[]);
        for attempt in 1..=2 {
            assert!(matches!(
                policy.decide(attempt, Outcome::Status(&attempted), Duration::ZERO, 0.0),
                RetryDecision::RetryAfter(_)
            ));
        }
        assert_eq!(
            policy.decide(3, Outcome::Status(&attempted), Duration::ZERO, 0.0),
            RetryDecision::Stop
        );
    }

    #[test]
    fn zero_retries_means_one_attempt() {
        assert_eq!(
            RetryPolicy::none().decide(
                1,
                Outcome::Status(&response(503, &[])),
                Duration::ZERO,
                0.0
            ),
            RetryDecision::Stop
        );
    }

    #[test]
    fn backoff_doubles_and_then_caps() {
        let policy = RetryPolicy {
            jitter: 0.0,
            ..RetryPolicy::default()
        };
        assert_eq!(policy.backoff(1, 0.0), Duration::from_millis(500));
        assert_eq!(policy.backoff(2, 0.0), Duration::from_millis(1000));
        assert_eq!(policy.backoff(3, 0.0), Duration::from_millis(2000));
        assert_eq!(policy.backoff(4, 0.0), Duration::from_millis(4000));
        // Capped at max_backoff from here on.
        assert_eq!(policy.backoff(5, 0.0), Duration::from_secs(5));
        assert_eq!(policy.backoff(50, 0.0), Duration::from_secs(5));
        assert_eq!(policy.backoff(u32::MAX, 0.0), Duration::from_secs(5));
    }

    #[test]
    fn jitter_only_ever_subtracts() {
        let policy = RetryPolicy::default();
        let full = policy.backoff(1, 0.0);
        let jittered = policy.backoff(1, 1.0);
        assert_eq!(full, Duration::from_millis(500));
        // 25 % jitter fully applied.
        assert_eq!(jittered, Duration::from_millis(375));
        assert!(jittered <= full);
    }

    #[test]
    fn retry_after_ms_is_preferred_and_honoured() {
        let policy = RetryPolicy::default();
        let attempted = response(429, &[("retry-after-ms", "1500"), ("retry-after", "30")]);
        assert_eq!(
            policy.decide(1, Outcome::Status(&attempted), Duration::ZERO, 0.0),
            RetryDecision::RetryAfter(Duration::from_millis(1500))
        );
    }

    #[test]
    fn retry_after_seconds_is_honoured() {
        let policy = RetryPolicy::default();
        let attempted = response(429, &[("retry-after", "2")]);
        assert_eq!(
            policy.decide(1, Outcome::Status(&attempted), Duration::ZERO, 0.0),
            RetryDecision::RetryAfter(Duration::from_secs(2))
        );
    }

    /// `Retry-After: 0` means "try again now", not "there is no hint".
    ///
    /// A zero that falls through to backoff would make the client wait half a second
    /// when the server explicitly said it did not have to. It is a real value the
    /// documented parser accepts, and the easiest one to lose to an `is_zero()` guard.
    #[test]
    fn a_zero_retry_after_is_honoured_as_zero() {
        let policy = RetryPolicy::default();
        for header in [("retry-after", "0"), ("retry-after-ms", "0")] {
            let attempted = response(429, &[header]);
            assert_eq!(
                policy.decide(1, Outcome::Status(&attempted), Duration::ZERO, 0.0),
                RetryDecision::RetryAfter(Duration::ZERO),
                "{header:?} did not produce a zero wait"
            );
        }
    }

    /// A negative or non-numeric hint is not a hint. Falling back to backoff is right;
    /// treating `-1` as zero, or panicking on `soon`, is not.
    #[test]
    fn an_unusable_retry_after_falls_back_to_backoff() {
        let policy = RetryPolicy::default();
        for raw in ["-1", "soon", "", "NaN", "inf", "1e400"] {
            let attempted = response(429, &[("retry-after", raw)]);
            let decision = policy.decide(1, Outcome::Status(&attempted), Duration::ZERO, 0.0);
            let expected = RetryDecision::RetryAfter(policy.backoff(1, 0.0));
            assert_eq!(
                decision, expected,
                "`Retry-After: {raw}` was treated as a hint instead of falling back to backoff"
            );
        }
    }

    #[test]
    fn an_absurd_retry_after_is_clamped_not_obeyed() {
        // A hostile endpoint asking for a year must not hang the process.
        let policy = RetryPolicy {
            total_budget: Duration::from_secs(3600),
            ..RetryPolicy::default()
        };
        let attempted = response(429, &[("retry-after", "31536000")]);
        assert_eq!(
            policy.decide(1, Outcome::Status(&attempted), Duration::ZERO, 0.0),
            RetryDecision::RetryAfter(MAX_HONOURED_RETRY_AFTER)
        );
    }

    #[test]
    fn a_malformed_retry_after_falls_back_to_backoff() {
        let policy = RetryPolicy {
            jitter: 0.0,
            ..RetryPolicy::default()
        };
        for value in [
            "not-a-number",
            "-5",
            "NaN",
            "inf",
            "",
            "Wed, 21 Oct 2026 07:28:00 GMT",
        ] {
            let attempted = response(429, &[("retry-after", value)]);
            assert_eq!(
                policy.decide(1, Outcome::Status(&attempted), Duration::ZERO, 0.0),
                RetryDecision::RetryAfter(Duration::from_millis(500)),
                "unexpected handling of retry-after: {value:?}"
            );
        }
    }

    #[test]
    fn retry_after_can_be_disabled() {
        let policy = RetryPolicy {
            respect_retry_after: false,
            jitter: 0.0,
            ..RetryPolicy::default()
        };
        let attempted = response(429, &[("retry-after", "30")]);
        assert_eq!(
            policy.decide(1, Outcome::Status(&attempted), Duration::ZERO, 0.0),
            RetryDecision::RetryAfter(Duration::from_millis(500))
        );
    }

    #[test]
    fn the_total_budget_stops_a_retry_that_would_overrun_it() {
        let policy = RetryPolicy {
            total_budget: Duration::from_secs(2),
            jitter: 0.0,
            ..RetryPolicy::default()
        };
        let attempted = response(503, &[]);
        // 1.8 s elapsed plus a 0.5 s wait exceeds the 2 s budget.
        assert_eq!(
            policy.decide(
                1,
                Outcome::Status(&attempted),
                Duration::from_millis(1800),
                0.0
            ),
            RetryDecision::Stop
        );
        assert!(matches!(
            policy.decide(
                1,
                Outcome::Status(&attempted),
                Duration::from_millis(100),
                0.0
            ),
            RetryDecision::RetryAfter(_)
        ));
    }

    #[test]
    fn a_timeout_is_retried_but_an_oversized_response_is_not() {
        let policy = RetryPolicy::default();
        let timeout = TransportError::Timeout { seconds: 10 };
        assert!(matches!(
            policy.decide(1, Outcome::Transport(&timeout), Duration::ZERO, 0.0),
            RetryDecision::RetryAfter(_)
        ));
        // Re-requesting will produce the same oversized body.
        let too_large = TransportError::ResponseTooLarge { limit: 16 };
        assert_eq!(
            policy.decide(1, Outcome::Transport(&too_large), Duration::ZERO, 0.0),
            RetryDecision::Stop
        );
    }

    #[test]
    fn header_lookup_is_case_normalized_by_the_transport_not_here() {
        // Transports lowercase header names; this asserts the contract explicitly so a
        // new transport cannot quietly break `Retry-After` handling.
        let mut headers = BTreeMap::new();
        headers.insert("retry-after".to_owned(), "1".to_owned());
        let response = Response {
            status: 429,
            headers,
            body: Vec::new(),
        };
        assert_eq!(parse_retry_after(&response), Some(Duration::from_secs(1)));
    }

    proptest! {
        /// The absolute bound: no combination of inputs produces an unbounded wait.
        #[test]
        fn a_retry_wait_never_exceeds_the_budget(
            attempt in 1_u32..10,
            status in 400_u16..600,
            hint in "[-0-9eE.]{0,20}",
            elapsed_ms in 0_u64..60_000,
            jitter in 0.0_f64..1.0,
        ) {
            let policy = RetryPolicy::default();
            let attempted = response(status, &[("retry-after", &hint)]);
            let decision = policy.decide(
                attempt,
                Outcome::Status(&attempted),
                Duration::from_millis(elapsed_ms),
                jitter,
            );
            if let RetryDecision::RetryAfter(delay) = decision {
                prop_assert!(delay <= MAX_HONOURED_RETRY_AFTER);
                prop_assert!(
                    Duration::from_millis(elapsed_ms) + delay < policy.total_budget
                );
            }
        }

        /// Backoff is total and monotonically bounded for every attempt number.
        #[test]
        fn backoff_is_total_and_bounded(attempt in 0_u32..u32::MAX, jitter in -10.0_f64..10.0) {
            let policy = RetryPolicy::default();
            let delay = policy.backoff(attempt, jitter);
            prop_assert!(delay <= policy.max_backoff);
        }
    }
}
