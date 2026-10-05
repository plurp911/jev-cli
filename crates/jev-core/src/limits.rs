//! Documented provider limits and the client-side bounds `jev` applies on top.
//!
//! # Provenance
//!
//! The TypeSafe constants marked *API* are transcribed from its documentation, which
//! was re-fetched and compared byte-for-byte against
//! `references/05-typesafe-docs/pages/` on 2026-09-19:
//!
//! * [Score levels](https://docs.typesafe.ai/primitives/score) — "Needs at least two
//!   levels and takes up to 10."
//! * [Choice options](https://docs.typesafe.ai/primitives/choice) — "A Choice question
//!   accepts up to 255 options."
//! * [API reference](https://docs.typesafe.ai/api) — Score "must include at least two
//!   levels"; `questions` must be non-empty.
//!
//! Constants marked *client* are `jev`'s own; the API does not document them. They
//! exist to fail fast, locally and for free, instead of spending tokens on a request
//! that cannot succeed — and to bound memory against hostile input. Where a client
//! bound is adjustable, the flag that adjusts it is named.
//!
//! A limit that moves in the official documentation is a compatibility change: update
//! it here, in `docs/api-compatibility.md`, and in `CHANGELOG.md` together.

/// *Client.* Fewest levels a Score question may define.
///
/// The prose says a Score *should* have at least two levels, and the generated OpenAPI
/// schema sets `min_length: 1`, so the API itself may well accept one. A one-level
/// Score has exactly one possible answer and a `score` that is always 0, so it is a
/// mistake that costs tokens rather than a usable question. `jev` rejects it locally
/// and says so, exactly as it does for a one-option [`CHOICE_MIN_OPTIONS`] Choice.
pub const SCORE_MIN_LEVELS: usize = 2;

/// *API.* Default Score maximum for TypeSafe, Cloudflare, and llama.cpp.
pub const SCORE_MAX_LEVELS: usize = 10;

/// *Client.* Absolute supported Score level bound across providers.
///
/// The publisher head dynamically enumerates options; 255 bounds local head and
/// response work, matching the existing Choice bound. This is not a model limit.
/// <https://huggingface.co/Cloudflare/clef/blob/main/joint_schema_model.py>.
pub const SCORE_ABSOLUTE_MAX_LEVELS: usize = 255;

/// *API.* Ollama's independent documented Score limit.
/// <https://docs.ollama.com/capabilities/decision>.
pub const OLLAMA_SCORE_MAX_LEVELS: usize = 26;

/// *API / client.* Most options a Choice question may define.
///
/// TypeSafe documents 255; the publisher head dynamically enumerates options and
/// uses this as a deliberate client resource bound. Ollama separately permits 26.
pub const CHOICE_MAX_OPTIONS: usize = 255;

/// *Client.* Fewest options a Choice question may define.
///
/// The API documents no minimum. A one-option Choice has exactly one possible answer,
/// always at probability 1, so it is a mistake that costs tokens rather than a usable
/// question. `jev` rejects it locally and says so.
pub const CHOICE_MIN_OPTIONS: usize = 2;

/// *API.* Fewest questions a request may carry.
pub const MIN_QUESTIONS: usize = 1;

/// *Client.* Deepest JSON nesting accepted in state, instructions, or criteria.
///
/// Deeply nested input is a stack-exhaustion vector in any recursive walk. `serde_json`
/// applies its own 128-level limit while parsing; this bound is applied again after
/// parsing so that values built by other means are covered too.
pub const MAX_JSON_DEPTH: usize = 64;

/// *Client.* Default ceiling on the bytes `jev` will read from one input source.
///
/// Adjustable with `--max-input-bytes`. The real constraint is the model's token
/// budget — 64k tokens per request, 32k for `state` plus the longest question — which
/// cannot be measured locally, so this bound only stops accidents such as piping a
/// disk image into `jev`. Exceeding it is always reported, never silently truncated.
pub const DEFAULT_MAX_INPUT_BYTES: u64 = 1 << 20;

/// *Client.* Ceiling on an API response body, in bytes.
///
/// A response larger than this is refused rather than buffered, so a hostile or
/// malfunctioning endpoint cannot exhaust memory.
pub const MAX_RESPONSE_BYTES: u64 = 16 << 20;

/// *Client.* Longest error text `jev` will echo from an API error body.
///
/// Mirrors the official SDK's `MAX_ERROR_BODY_LENGTH`, so that a hostile endpoint
/// cannot use an error message as an unbounded output channel.
pub const MAX_ERROR_BODY_CHARS: usize = 200;
