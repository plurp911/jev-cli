//! The evaluation request: state, model, and a named set of questions.
//!
//! Shape from <https://docs.typesafe.ai/api>, "Request body".

use std::collections::BTreeSet;

use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};

use crate::content::{Content, ContentError};
use crate::limits::MIN_QUESTIONS;
use crate::model::ModelId;
use crate::question::Question;

/// Reasons a question identifier is not usable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum QuestionIdError {
    /// The identifier was empty or whitespace only.
    #[error("a question id must not be empty")]
    Empty,
    /// The identifier contained a control character.
    ///
    /// Question ids are echoed in output and used as JSON keys, so a control character
    /// in one is both a rendering hazard and almost certainly a mistake. The API places
    /// no restriction on the key; this one is `jev`'s.
    #[error("a question id must not contain control characters")]
    ControlCharacter,
}

/// A key the caller chooses, under which the matching answer comes back.
///
/// The API states the key "is not sent to the underlying model and is not used in
/// inference", so it is purely a handle for code. Complete meaning belongs in the
/// question itself.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QuestionId(String);

impl QuestionId {
    /// Validates and wraps an identifier.
    ///
    /// # Errors
    ///
    /// Returns [`QuestionIdError`] for a blank or control-bearing identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, QuestionIdError> {
        let value = value.into();
        // Trimmed, like `ModelId`. Otherwise `" urgent"` and `"urgent"` are distinct
        // ids that duplicate detection cannot see, so a stray space in a request file
        // silently doubles a billed question.
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(QuestionIdError::Empty);
        }
        if trimmed.chars().any(char::is_control) {
            return Err(QuestionIdError::ControlCharacter);
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// Returns the identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for QuestionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The content every question in a request refers to.
///
/// The API accepts `string | object | array` here. A structured state with named
/// fields is the documented preference when the context has several parts; see
/// <https://docs.typesafe.ai/concepts/state>.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct State(Content);

impl State {
    /// Wraps validated content as request state.
    #[must_use]
    pub const fn new(content: Content) -> Self {
        Self(content)
    }

    /// Builds text state.
    ///
    /// # Errors
    ///
    /// Returns [`ContentError::Empty`] for blank text. An empty state is rejected
    /// locally rather than sent: there is nothing for the model to evaluate.
    pub fn text(text: impl Into<String>) -> Result<Self, ContentError> {
        Content::text(text).map(Self)
    }

    /// Borrows the underlying content.
    #[must_use]
    pub const fn content(&self) -> &Content {
        &self.0
    }
}

/// Reasons a request cannot be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RequestError {
    /// No questions were supplied.
    #[error("a request needs at least {MIN_QUESTIONS} question")]
    NoQuestions,
    /// Two questions shared an identifier.
    #[error("duplicate question id {id:?}")]
    DuplicateQuestionId {
        /// The repeated identifier.
        id: String,
    },
}

/// A complete System One evaluation request.
///
/// Question order is preserved so that human output reads in the order the user wrote,
/// and so that `--dry-run` shows a body a person can compare with their input. The API
/// treats `questions` as a map, where order carries no meaning.
#[derive(Debug, Clone, PartialEq)]
pub struct EvaluationRequest {
    state: State,
    model: ModelId,
    questions: Vec<(QuestionId, Question)>,
}

impl EvaluationRequest {
    /// Assembles a request.
    ///
    /// # Errors
    ///
    /// Returns [`RequestError::NoQuestions`] for an empty question list, and
    /// [`RequestError::DuplicateQuestionId`] if two questions share an id — which would
    /// otherwise collapse silently into one JSON key and drop a question the user paid
    /// to ask.
    pub fn new(
        state: State,
        model: ModelId,
        questions: Vec<(QuestionId, Question)>,
    ) -> Result<Self, RequestError> {
        if questions.len() < MIN_QUESTIONS {
            return Err(RequestError::NoQuestions);
        }
        let mut seen = BTreeSet::new();
        for (id, _) in &questions {
            if !seen.insert(id.as_str()) {
                return Err(RequestError::DuplicateQuestionId {
                    id: id.as_str().to_owned(),
                });
            }
        }
        Ok(Self {
            state,
            model,
            questions,
        })
    }

    /// The state all questions refer to.
    #[must_use]
    pub const fn state(&self) -> &State {
        &self.state
    }

    /// The model the request asks for.
    #[must_use]
    pub const fn model(&self) -> &ModelId {
        &self.model
    }

    /// The questions, in the order they were supplied.
    #[must_use]
    pub fn questions(&self) -> &[(QuestionId, Question)] {
        &self.questions
    }

    /// How many questions the request carries.
    #[must_use]
    pub fn question_count(&self) -> usize {
        self.questions.len()
    }
}

impl Serialize for EvaluationRequest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct as _;
        let mut object = serializer.serialize_struct("EvaluationRequest", 3)?;
        object.serialize_field("state", &self.state)?;
        object.serialize_field("model", &self.model)?;
        object.serialize_field("questions", &QuestionMap(&self.questions))?;
        object.end()
    }
}

struct QuestionMap<'a>(&'a [(QuestionId, Question)]);

impl Serialize for QuestionMap<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (id, question) in self.0 {
            map.serialize_entry(id.as_str(), question)?;
        }
        map.end()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::question::ChoiceOption;

    fn text(value: &str) -> Content {
        Content::text(value).unwrap()
    }

    fn id(value: &str) -> QuestionId {
        QuestionId::new(value).unwrap()
    }

    #[test]
    fn wire_form_matches_the_api_reference_example() {
        // Transcribed from https://docs.typesafe.ai/api, "Example request".
        let request = EvaluationRequest::new(
            State::text("Help! My payouts have been failing for 3 days.").unwrap(),
            ModelId::default(),
            vec![(
                id("is_urgent"),
                Question::noul(text("Does this convey urgency?"), None).unwrap(),
            )],
        )
        .unwrap();

        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            json!({
                "state": "Help! My payouts have been failing for 3 days.",
                "model": "jev-latest",
                "questions": {
                    "is_urgent": {"type": "noul", "instructions": "Does this convey urgency?"}
                }
            })
        );
    }

    #[test]
    fn a_mixed_request_carries_every_primitive() {
        let request = EvaluationRequest::new(
            State::text("ticket text").unwrap(),
            ModelId::new("jev-1.13.0").unwrap(),
            vec![
                (id("urgent"), Question::noul(text("Urgent?"), None).unwrap()),
                (
                    id("team"),
                    Question::choice(
                        text("Which team?"),
                        vec![
                            ChoiceOption::new("billing", None).unwrap(),
                            ChoiceOption::new("technical", None).unwrap(),
                        ],
                    )
                    .unwrap(),
                ),
                (
                    id("severity"),
                    Question::score(text("How severe?"), vec![text("low"), text("high")]).unwrap(),
                ),
            ],
        )
        .unwrap();

        let encoded = serde_json::to_value(&request).unwrap();
        assert_eq!(encoded["model"], json!("jev-1.13.0"));
        assert_eq!(encoded["questions"]["urgent"]["type"], json!("noul"));
        assert_eq!(encoded["questions"]["team"]["type"], json!("choice"));
        assert_eq!(encoded["questions"]["severity"]["type"], json!("score"));
        assert_eq!(request.question_count(), 3);
    }

    #[test]
    fn structured_state_is_preserved_exactly() {
        let state = State::new(Content::try_from(json!({"subject": "x", "body": ["y"]})).unwrap());
        let request = EvaluationRequest::new(
            state,
            ModelId::default(),
            vec![(id("q"), Question::noul(text("?"), None).unwrap())],
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&request).unwrap()["state"],
            json!({"subject": "x", "body": ["y"]})
        );
    }

    #[test]
    fn rejects_no_questions() {
        assert_eq!(
            EvaluationRequest::new(State::text("s").unwrap(), ModelId::default(), Vec::new()),
            Err(RequestError::NoQuestions)
        );
    }

    #[test]
    fn rejects_duplicate_question_ids() {
        // Without this, the second question silently overwrites the first in the JSON
        // object and the user is billed for a question they never get an answer to.
        let questions = vec![
            (id("same"), Question::noul(text("a"), None).unwrap()),
            (id("same"), Question::noul(text("b"), None).unwrap()),
        ];
        assert_eq!(
            EvaluationRequest::new(State::text("s").unwrap(), ModelId::default(), questions),
            Err(RequestError::DuplicateQuestionId {
                id: "same".to_owned()
            })
        );
    }

    #[test]
    fn question_ids_are_trimmed_so_whitespace_cannot_hide_a_duplicate() {
        assert_eq!(QuestionId::new(" urgent ").unwrap().as_str(), "urgent");
        // And the trimmed forms collide, so the duplicate check sees them.
        let questions = vec![
            (
                QuestionId::new(" same").unwrap(),
                Question::noul(text("a"), None).unwrap(),
            ),
            (
                QuestionId::new("same ").unwrap(),
                Question::noul(text("b"), None).unwrap(),
            ),
        ];
        assert!(
            EvaluationRequest::new(State::text("s").unwrap(), ModelId::default(), questions)
                .is_err()
        );
    }

    #[test]
    fn question_ids_reject_blank_and_control_characters() {
        assert_eq!(QuestionId::new(" "), Err(QuestionIdError::Empty));
        assert_eq!(
            QuestionId::new("a\u{1b}b"),
            Err(QuestionIdError::ControlCharacter)
        );
        assert!(QuestionId::new("is_urgent").is_ok());
    }

    #[test]
    fn empty_state_is_refused_before_it_costs_anything() {
        assert!(State::text("").is_err());
        assert!(State::text("   \t\n").is_err());
    }
}
