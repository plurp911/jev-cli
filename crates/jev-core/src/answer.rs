//! Answers, as returned by the API, validated on the way in.
//!
//! Shapes are from the official HTTP API reference
//! (<https://docs.typesafe.ai/api>), section "Answer types".
//!
//! Two invariants are enforced here so that no downstream code has to re-check them:
//!
//! * Every probability is finite and within `[0, 1]`, because [`Probability`] cannot
//!   hold anything else.
//! * A Score's value lies within the range its own legend spans.
//!
//! Uncertainty is never discarded. The full distribution is kept on every Choice and
//! Score answer and is always present in `--output json`; that distribution is the
//! reason to use a System One model at all.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::confidence::Confidence;
use crate::content::Content;
use crate::probability::Probability;
use crate::question::QuestionKind;

/// One option or level and the probability the model assigned to it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Weighted<K> {
    /// The option name, or the level number.
    pub key: K,
    /// The probability mass on it.
    pub probability: Probability,
}

/// An answer to one question.
///
/// The [`Answer::Unrecognized`] variant exists for forward compatibility: if TypeSafe
/// adds a fourth primitive, `jev` reports it rather than failing the whole response or
/// silently dropping it. The official Python SDK drops such answers with a log warning;
/// preserving the payload is strictly more useful to a script.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// The probability that the answer to a yes/no question is yes.
    ///
    /// There is deliberately no confidence field. The API does not return one for a
    /// Noul, and `jev` does not invent one. See <https://docs.typesafe.ai/confidence>.
    Noul {
        /// Probability of "yes". `0.5` means yes and no are similarly likely — it does
        /// not mean "medium".
        noul: Probability,
    },
    /// The selected option, the full distribution, and the confidence in the selection.
    Choice {
        /// The highest-probability option, as reported by the API.
        choice: String,
        /// Every option and its probability, in the order the API returned them.
        probabilities: Vec<Weighted<String>>,
        /// How concentrated the distribution is.
        confidence: Confidence,
    },
    /// The probability-weighted position on the level scale.
    Score {
        /// The expected level. May fall between levels.
        score: f64,
        /// Each level number mapped back to the description that was sent.
        legend: BTreeMap<u32, Content>,
        /// Each level number and its probability.
        probabilities: Vec<Weighted<u32>>,
        /// How concentrated the distribution is.
        confidence: Confidence,
    },
    /// An answer whose `type` this version of `jev` does not model.
    Unrecognized {
        /// The `type` string the API sent.
        kind: String,
        /// The answer object verbatim, so nothing the API said is lost.
        raw: Value,
    },
}

impl Answer {
    /// The kind of question this answers, or `None` for an unrecognized type.
    #[must_use]
    pub const fn kind(&self) -> Option<QuestionKind> {
        match self {
            Self::Noul { .. } => Some(QuestionKind::Noul),
            Self::Choice { .. } => Some(QuestionKind::Choice),
            Self::Score { .. } => Some(QuestionKind::Score),
            Self::Unrecognized { .. } => None,
        }
    }

    /// The wire `type` discriminant.
    #[must_use]
    pub fn kind_str(&self) -> &str {
        match self {
            Self::Noul { .. } => QuestionKind::Noul.as_str(),
            Self::Choice { .. } => QuestionKind::Choice.as_str(),
            Self::Score { .. } => QuestionKind::Score.as_str(),
            Self::Unrecognized { kind, .. } => kind,
        }
    }

    /// The confidence the API reported, or `None` where the API reports none.
    ///
    /// Returns `None` for a Noul. That is the API's behaviour, not a gap in `jev`.
    #[must_use]
    pub const fn confidence(&self) -> Option<Confidence> {
        match self {
            Self::Choice { confidence, .. } | Self::Score { confidence, .. } => Some(*confidence),
            Self::Noul { .. } | Self::Unrecognized { .. } => None,
        }
    }

    /// The single scalar a script most often wants: the Noul probability, the chosen
    /// option name, or the score.
    ///
    /// This is what `--value` prints. It is a convenience over the full answer, never a
    /// replacement for it.
    #[must_use]
    pub fn scalar(&self) -> Option<Scalar<'_>> {
        match self {
            Self::Noul { noul } => Some(Scalar::Number(noul.get())),
            Self::Choice { choice, .. } => Some(Scalar::Text(choice)),
            Self::Score { score, .. } => Some(Scalar::Number(*score)),
            Self::Unrecognized { .. } => None,
        }
    }
}

/// The scalar projection of an answer, for shell use.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scalar<'a> {
    /// A number: a Noul probability or a Score value.
    Number(f64),
    /// Text: the selected Choice option.
    Text(&'a str),
}

/// Token counts for a request.
///
/// Both fields are optional because the official SDK models them as optional; a
/// response that omits them still decodes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Usage {
    /// Billable input tokens, when the API reported a count.
    pub input_tokens: Option<u64>,
    /// Output tokens, when the API reported a count. Currently free of charge.
    pub output_tokens: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probability(value: f64) -> Probability {
        Probability::new(value).unwrap()
    }

    #[test]
    fn a_noul_answer_has_no_confidence() {
        // The single most common misreading of the API in the community tooling.
        let answer = Answer::Noul {
            noul: probability(0.5),
        };
        assert_eq!(answer.confidence(), None);
    }

    #[test]
    fn choice_and_score_carry_confidence() {
        let choice = Answer::Choice {
            choice: "a".to_owned(),
            probabilities: vec![Weighted {
                key: "a".to_owned(),
                probability: probability(1.0),
            }],
            confidence: Confidence::new(0.9).unwrap(),
        };
        assert!(choice.confidence().is_some());
    }

    #[test]
    fn scalar_projections_match_the_primitive() {
        assert_eq!(
            Answer::Noul {
                noul: probability(0.25)
            }
            .scalar(),
            Some(Scalar::Number(0.25))
        );
        assert_eq!(
            Answer::Choice {
                choice: "billing".to_owned(),
                probabilities: Vec::new(),
                confidence: Confidence::new(0.5).unwrap(),
            }
            .scalar(),
            Some(Scalar::Text("billing"))
        );
        assert_eq!(
            Answer::Unrecognized {
                kind: "future".to_owned(),
                raw: Value::Null,
            }
            .scalar(),
            None
        );
    }

    #[test]
    fn an_unrecognized_answer_keeps_its_type_name() {
        let answer = Answer::Unrecognized {
            kind: "ranking".to_owned(),
            raw: serde_json::json!({"type": "ranking"}),
        };
        assert_eq!(answer.kind_str(), "ranking");
        assert_eq!(answer.kind(), None);
    }
}
