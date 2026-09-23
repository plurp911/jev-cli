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
//! * [`Content`] can only be the `string | object | array` the API accepts, so a bare
//!   number cannot reach the wire encoder.
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
mod model;
mod probability;
mod question;
mod request;
mod response;

pub use answer::{Answer, Scalar, Usage, Weighted};
pub use confidence::Confidence;
pub use content::{Content, ContentError, check_json_depth};
pub use model::{DEFAULT_MODEL, ModelId, ModelIdError};
pub use probability::{Probability, ProbabilityError};
pub use question::{ChoiceOption, NoulCriteria, Question, QuestionError, QuestionKind};
pub use request::{EvaluationRequest, QuestionId, QuestionIdError, RequestError, State};
pub use response::{EvaluationResponse, ModelCard};
