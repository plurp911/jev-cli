//! The decoded evaluation response.

use crate::answer::{Answer, Usage};
use crate::model::ModelId;
use crate::request::QuestionId;

/// A decoded System One response.
///
/// Answers keep the request's question order rather than the JSON object's order, so
/// that human output reads the way the user wrote the request. Every answer the API
/// returned is present, including any the request did not ask for — dropping one would
/// hide a server-side surprise.
#[derive(Debug, Clone, PartialEq)]
pub struct EvaluationResponse {
    /// The model the API says answered.
    ///
    /// This may differ from the model requested: an alias such as `jev-latest` resolves
    /// to a versioned identifier. Recording it is the only way to reason later about a
    /// threshold that was calibrated against a particular version.
    pub model: ModelId,
    /// One answer per question, keyed by the id from the request.
    pub answers: Vec<(QuestionId, Answer)>,
    /// Token counts for the request.
    pub usage: Usage,
}

impl EvaluationResponse {
    /// Looks up an answer by question id.
    #[must_use]
    pub fn answer(&self, id: &str) -> Option<&Answer> {
        self.answers
            .iter()
            .find(|(key, _)| key.as_str() == id)
            .map(|(_, answer)| answer)
    }

    /// Question ids that were asked but have no answer in the response.
    ///
    /// The API returns one answer per question, so a non-empty result here means the
    /// response did not match the request and the caller should not treat missing
    /// answers as negative ones.
    #[must_use]
    pub fn missing<'a>(&self, asked: impl IntoIterator<Item = &'a QuestionId>) -> Vec<String> {
        asked
            .into_iter()
            .filter(|id| self.answer(id.as_str()).is_none())
            .map(|id| id.as_str().to_owned())
            .collect()
    }
}

/// One entry from `GET /v1/models`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCard {
    /// The identifier or alias accepted by a request's `model` field.
    pub name: String,
    /// What the model is for.
    pub description: String,
    /// Release date, formatted `YYYY-MM-DD` by the API.
    ///
    /// Kept as the string the API sent rather than parsed into a date type: `jev` only
    /// displays it, and a parser here would turn an unexpected format into a failure
    /// for no benefit.
    pub release_date: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probability::Probability;

    fn id(value: &str) -> QuestionId {
        QuestionId::new(value).unwrap()
    }

    fn response() -> EvaluationResponse {
        EvaluationResponse {
            model: ModelId::new("jev-1.13.0").unwrap(),
            answers: vec![(
                id("a"),
                Answer::Noul {
                    noul: Probability::new(0.9).unwrap(),
                },
            )],
            usage: Usage::default(),
        }
    }

    #[test]
    fn looks_answers_up_by_id() {
        assert!(response().answer("a").is_some());
        assert!(response().answer("b").is_none());
    }

    #[test]
    fn reports_questions_the_api_did_not_answer() {
        // A missing answer must never be read as a negative one.
        let asked = vec![id("a"), id("b")];
        assert_eq!(response().missing(&asked), vec!["b".to_owned()]);
    }
}
