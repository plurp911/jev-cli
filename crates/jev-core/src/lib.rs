//! Pure domain model shared by every other crate in the workspace.
//!
//! # Scope
//!
//! `jev-core` contains only value types and total functions over them. It must never
//! perform I/O, read the environment, touch the filesystem, open a socket, or handle a
//! credential. That restriction is what lets the rest of the workspace be tested
//! exhaustively without network access; see `docs/adr/0007-workspace-architecture.md`.
//!
//! # What lives here
//!
//! The System One protocol's *meaning*, expressed so that invalid values cannot be
//! constructed:
//!
//! * [`Probability`] and [`Confidence`] cannot hold NaN, an infinity, or a value
//!   outside `[0, 1]` — so no downstream code checks for one.
//! * [`Content`] enforces `string | object | array` by default and admits broader
//!   publisher JSON only through an explicit constructor and provider check.
//! * [`Question`] enforces the documented cardinality of Choice options and Score
//!   levels at construction, so a request that the API would reject is rejected here
//!   first, for free.
//! * [`Answer`] keeps the full probability distribution. Uncertainty is never
//!   summarized away, and a Noul never grows a confidence value the API did not send.
//!
//! Every constant transcribed from the API is in [`limits`], with its citation.
//!
//! # Stability
//!
//! This crate is `publish = false` and carries **no** semver promise to external users.
//! The only supported interface of this project is the `jev` command line; see
//! `docs/adr/0003-cli-compatibility.md`.

mod answer;
mod confidence;
mod content;
pub mod limits;
mod media;
mod model;
mod probability;
mod question;
mod request;
mod response;

pub use answer::{Answer, Scalar, Usage, Weighted};
pub use confidence::Confidence;
pub use content::{Content, ContentError, LocalJson, check_json_depth};
pub use media::{
    EmbeddedImage, EmbeddedVideo, MAX_IMAGE_BYTES, MAX_IMAGE_PIXELS, MAX_IMAGES,
    MAX_TOTAL_IMAGE_BYTES, MAX_TOTAL_MEDIA_PIXELS, MAX_VIDEO_FRAMES, MAX_VIDEOS, MediaError,
};
pub use model::{DEFAULT_MODEL, ModelId, ModelIdError};
pub use probability::{Probability, ProbabilityError};
pub use question::{ChoiceOption, NoulCriteria, Question, QuestionError, QuestionKind};
pub use request::{
    EvaluationRequest, KeepAliveError, LocalOptionsError, QuestionId, QuestionIdError,
    RequestError, State,
};
pub use response::{EvaluationResponse, ModelCard};
