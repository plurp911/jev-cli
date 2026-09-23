//! Transport-agnostic client layer for the TypeSafe System One API.
//!
//! # Why a transport trait
//!
//! Every request this crate makes goes through [`Transport`], an object-safe trait with
//! no HTTP-library types in its signature. That keeps request shaping, retry policy,
//! and response decoding exercisable in unit tests with zero network access, which is
//! the core testability requirement in `docs/adr/0007-workspace-architecture.md`.
//!
//! A blocking signature is intentional: `jev` is a short-lived process that issues a
//! small number of requests, so an async runtime would be a large dependency with no
//! user-visible benefit. See the same ADR for the trade-off.
//!
//! # Credentials
//!
//! This crate never *reads* a credential — it has no access to the environment, a
//! keychain, or the filesystem. It receives one per call as a [`Credential`], which is
//! not `Clone` and not `Serialize`, and which never becomes part of a [`Request`]. The
//! plaintext exists as an `Authorization` header only inside the concrete transport,
//! in a buffer that zeroizes. See `docs/threat-model.md` T2.
//!
//! # Wire compatibility
//!
//! Request and response shapes follow the official HTTP API reference at
//! <https://docs.typesafe.ai/api>, cross-checked against the official Python SDK.
//! Fixtures recorded from the documented examples are in
//! `crates/jev-client/tests/compatibility.rs`; they fail if decoding drifts.

mod client;
mod credential;
mod endpoint;
mod error;
#[cfg(feature = "http")]
mod http;
mod retry;
mod transport;
mod wire;

#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use client::{
    CallStats, Client, Clock, REQUEST_ID_HEADER, SystemClock, build_evaluation_request, user_agent,
};
pub use credential::Credential;
pub use endpoint::{DEFAULT_BASE_URL, Endpoint, EndpointError, MODELS_PATH, SYSTEM_ONE_PATH};
pub use error::{ClientError, TransportError};
#[cfg(feature = "http")]
pub use http::{HttpTransport, MAX_POOLED_CONNECTIONS};
pub use retry::{
    MAX_RETRIES, Outcome, RetryDecision, RetryPolicy, is_retryable_status, parse_retry_after,
};
pub use transport::{Request, Response, Transport};
pub use wire::{decode_evaluation, decode_models, extract_error_message};

/// Re-export so callers do not need a direct `jev-core` dependency for the default.
pub use jev_core::DEFAULT_MODEL;
