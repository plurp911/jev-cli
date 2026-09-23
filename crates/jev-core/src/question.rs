//! Questions: the three System One primitives, validated before they can exist.
//!
//! Shapes are transcribed from the official HTTP API reference
//! (<https://docs.typesafe.ai/api>) and cross-checked against the primitive pages for
//! [Noul](https://docs.typesafe.ai/primitives/noul),
//! [Choice](https://docs.typesafe.ai/primitives/choice), and
//! [Score](https://docs.typesafe.ai/primitives/score).
//!
//! # Why the types are shaped this way
//!
//! Each variant owns exactly the fields its wire form has, so it is not possible to
//! build a Choice with score levels or a Score with named options. Validation happens
//! in the constructors, so a `Question` value that exists is one the API's documented
//! rules accept — and every rejection happens locally, before any tokens are spent.

use std::collections::BTreeSet;
use std::fmt;

use serde::ser::{SerializeMap, SerializeStruct};
use serde::{Serialize, Serializer};

use crate::content::Content;
use crate::limits::{CHOICE_MAX_OPTIONS, CHOICE_MIN_OPTIONS, SCORE_MAX_LEVELS, SCORE_MIN_LEVELS};

/// Reasons a question definition is not valid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum QuestionError {
    /// A Choice question defined too few options.
    #[error(
        "a choice question needs at least {CHOICE_MIN_OPTIONS} options, found {found}{}",
        // The explanation fits one option and not zero, where it reads as nonsense.
        if *found == 1 { "; with one option there is only one possible answer" } else { "" }
    )]
    TooFewOptions {
        /// How many were supplied.
        found: usize,
    },
    /// A Choice question defined more options than the API accepts.
    #[error("a choice question accepts at most {CHOICE_MAX_OPTIONS} options, found {found}")]
    TooManyOptions {
        /// How many were supplied.
        found: usize,
    },
    /// Two options in one Choice question shared a name.
    #[error("duplicate choice option {name:?}")]
    DuplicateOption {
        /// The repeated name.
        name: String,
    },
    /// An option name was empty or whitespace only.
    #[error("a choice option name must not be empty")]
    EmptyOptionName,
    /// An option name contained a control character.
    ///
    /// The same rule `QuestionId` applies, and for the same reason: an option name is
    /// echoed in output, becomes a JSON key in `probabilities`, comes back as the
    /// answer's `choice`, and is what a `--require` gate compares against. `QuestionId`
    /// rejected control characters from the beginning; option names did not, so the two
    /// halves of the same document were held to different standards.
    #[error("a choice option name must not contain control characters")]
    ControlCharacterInOptionName,
    /// A Score question defined too few levels.
    #[error("a score question needs at least {SCORE_MIN_LEVELS} levels, found {found}")]
    TooFewLevels {
        /// How many were supplied.
        found: usize,
    },
    /// A Score question defined more levels than the API accepts.
    #[error("a score question accepts at most {SCORE_MAX_LEVELS} levels, found {found}")]
    TooManyLevels {
        /// How many were supplied.
        found: usize,
    },
    /// A Noul question supplied a `criteria` object with neither outcome described.
    #[error("noul criteria must describe the true outcome, the false outcome, or both")]
    EmptyNoulCriteria,
}

/// Optional descriptions of what a Noul's yes and no outcomes mean.
///
/// At least one side must be present; an empty object carries no information and is
/// rejected rather than sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoulCriteria {
    /// What a yes (a value near 1) means.
    yes: Option<Content>,
    /// What a no (a value near 0) means.
    no: Option<Content>,
}

impl NoulCriteria {
    /// Builds criteria from either or both outcome descriptions.
    ///
    /// # Errors
    ///
    /// Returns [`QuestionError::EmptyNoulCriteria`] when both are `None`.
    pub fn new(yes: Option<Content>, no: Option<Content>) -> Result<Self, QuestionError> {
        if yes.is_none() && no.is_none() {
            return Err(QuestionError::EmptyNoulCriteria);
        }
        Ok(Self { yes, no })
    }

    /// The description of the yes outcome, if any.
    #[must_use]
    pub const fn yes(&self) -> Option<&Content> {
        self.yes.as_ref()
    }

    /// The description of the no outcome, if any.
    #[must_use]
    pub const fn no(&self) -> Option<&Content> {
        self.no.as_ref()
    }
}

impl Serialize for NoulCriteria {
    /// Serializes with the wire's `true` and `false` keys, omitting an absent side.
    ///
    /// The Rust field names are `yes`/`no` because `true` and `false` are keywords; the
    /// wire names are what the API documents.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let len = usize::from(self.yes.is_some()) + usize::from(self.no.is_some());
        let mut map = serializer.serialize_map(Some(len))?;
        if let Some(yes) = &self.yes {
            map.serialize_entry("true", yes)?;
        }
        if let Some(no) = &self.no {
            map.serialize_entry("false", no)?;
        }
        map.end()
    }
}

/// One named option of a Choice question, with an optional description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceOption {
    name: String,
    description: Option<Content>,
}

impl ChoiceOption {
    /// Builds an option.
    ///
    /// A `None` description is meaningful: the API documents it as "interpreted by its
    /// name alone", and it is encoded as a JSON `null` rather than omitted.
    ///
    /// # Errors
    ///
    /// Returns [`QuestionError::EmptyOptionName`] for a blank name, and
    /// [`QuestionError::ControlCharacterInOptionName`] for one carrying a control
    /// character.
    pub fn new(
        name: impl Into<String>,
        description: Option<Content>,
    ) -> Result<Self, QuestionError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(QuestionError::EmptyOptionName);
        }
        if name.chars().any(char::is_control) {
            return Err(QuestionError::ControlCharacterInOptionName);
        }
        Ok(Self { name, description })
    }

    /// The option name, which is also the key the answer's `choice` field will match.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The option's description, if it has one.
    #[must_use]
    pub const fn description(&self) -> Option<&Content> {
        self.description.as_ref()
    }
}

/// A System One question.
///
/// Construct one through [`Question::noul`], [`Question::choice`], or
/// [`Question::score`]; the fields are private so that an invalid question cannot be
/// assembled field by field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Question {
    /// A yes/no question. The answer is the probability that the answer is yes.
    Noul {
        /// The yes/no question or statement to evaluate.
        instructions: Content,
        /// Optional descriptions of the two outcomes.
        criteria: Option<NoulCriteria>,
    },
    /// A selection from a defined set of named options.
    Choice {
        /// What the model should decide.
        instructions: Content,
        /// The options, in the order the user gave them. Order is preserved because it
        /// is the order human output renders in; the API treats the set as unordered.
        options: Vec<ChoiceOption>,
    },
    /// A rating against ordered, described levels.
    Score {
        /// What the model should rate.
        instructions: Content,
        /// Level descriptions from the low end of the scale to the high end. A level's
        /// number is its index, starting at zero.
        levels: Vec<Content>,
    },
}

impl Question {
    /// Builds a Noul question.
    ///
    /// # Errors
    ///
    /// Returns [`QuestionError::EmptyNoulCriteria`] if `criteria` is present but
    /// describes neither outcome.
    pub fn noul(
        instructions: Content,
        criteria: Option<NoulCriteria>,
    ) -> Result<Self, QuestionError> {
        Ok(Self::Noul {
            instructions,
            criteria,
        })
    }

    /// Builds a Choice question.
    ///
    /// # Errors
    ///
    /// Returns [`QuestionError::TooFewOptions`], [`QuestionError::TooManyOptions`], or
    /// [`QuestionError::DuplicateOption`].
    pub fn choice(
        instructions: Content,
        options: Vec<ChoiceOption>,
    ) -> Result<Self, QuestionError> {
        if options.len() < CHOICE_MIN_OPTIONS {
            return Err(QuestionError::TooFewOptions {
                found: options.len(),
            });
        }
        if options.len() > CHOICE_MAX_OPTIONS {
            return Err(QuestionError::TooManyOptions {
                found: options.len(),
            });
        }
        let mut seen = BTreeSet::new();
        for option in &options {
            if !seen.insert(option.name()) {
                return Err(QuestionError::DuplicateOption {
                    name: option.name().to_owned(),
                });
            }
        }
        Ok(Self::Choice {
            instructions,
            options,
        })
    }

    /// Builds a Score question.
    ///
    /// # Errors
    ///
    /// Returns [`QuestionError::TooFewLevels`] or [`QuestionError::TooManyLevels`].
    pub fn score(instructions: Content, levels: Vec<Content>) -> Result<Self, QuestionError> {
        if levels.len() < SCORE_MIN_LEVELS {
            return Err(QuestionError::TooFewLevels {
                found: levels.len(),
            });
        }
        if levels.len() > SCORE_MAX_LEVELS {
            return Err(QuestionError::TooManyLevels {
                found: levels.len(),
            });
        }
        Ok(Self::Score {
            instructions,
            levels,
        })
    }

    /// The wire `type` discriminant: `"noul"`, `"choice"`, or `"score"`.
    #[must_use]
    pub const fn kind(&self) -> QuestionKind {
        match self {
            Self::Noul { .. } => QuestionKind::Noul,
            Self::Choice { .. } => QuestionKind::Choice,
            Self::Score { .. } => QuestionKind::Score,
        }
    }

    /// The question's instructions.
    #[must_use]
    pub const fn instructions(&self) -> &Content {
        match self {
            Self::Noul { instructions, .. }
            | Self::Choice { instructions, .. }
            | Self::Score { instructions, .. } => instructions,
        }
    }
}

impl Serialize for Question {
    /// Emits the documented wire form, omitting absent optional fields exactly as the
    /// official SDK's `None`-omitting serializer does.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Noul {
                instructions,
                criteria,
            } => {
                let len = 2 + usize::from(criteria.is_some());
                let mut object = serializer.serialize_struct("Question", len)?;
                object.serialize_field("type", "noul")?;
                object.serialize_field("instructions", instructions)?;
                if let Some(criteria) = criteria {
                    object.serialize_field("criteria", criteria)?;
                }
                object.end()
            }
            Self::Choice {
                instructions,
                options,
            } => {
                let mut object = serializer.serialize_struct("Question", 3)?;
                object.serialize_field("type", "choice")?;
                object.serialize_field("instructions", instructions)?;
                object.serialize_field("criteria", &ChoiceCriteria(options))?;
                object.end()
            }
            Self::Score {
                instructions,
                levels,
            } => {
                let mut object = serializer.serialize_struct("Question", 3)?;
                object.serialize_field("type", "score")?;
                object.serialize_field("instructions", instructions)?;
                object.serialize_field("criteria", levels)?;
                object.end()
            }
        }
    }
}

/// Serializes ordered options as the wire's `map<string, Content | null>`.
///
/// Written by hand rather than held as a `BTreeMap` so that the user's option order
/// survives into human output; JSON object order carries no meaning to the API.
struct ChoiceCriteria<'a>(&'a [ChoiceOption]);

impl Serialize for ChoiceCriteria<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for option in self.0 {
            match option.description() {
                Some(description) => map.serialize_entry(option.name(), description)?,
                None => map.serialize_entry(option.name(), &serde_json::Value::Null)?,
            }
        }
        map.end()
    }
}

/// Which primitive a question or answer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum QuestionKind {
    /// A yes/no question.
    Noul,
    /// A selection from named options.
    Choice,
    /// A rating against ordered levels.
    Score,
}

impl QuestionKind {
    /// The wire discriminant. Part of the output contract.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Noul => "noul",
            Self::Choice => "choice",
            Self::Score => "score",
        }
    }
}

impl fmt::Display for QuestionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn text(value: &str) -> Content {
        Content::text(value).unwrap()
    }

    fn options(names: &[&str]) -> Vec<ChoiceOption> {
        names
            .iter()
            .map(|name| ChoiceOption::new(*name, None).unwrap())
            .collect()
    }

    #[test]
    fn noul_wire_form_matches_the_api_reference() {
        let question = Question::noul(
            text("Does this convey urgency?"),
            Some(
                NoulCriteria::new(
                    Some(text("Explicitly time-sensitive")),
                    Some(text("No urgency expressed")),
                )
                .unwrap(),
            ),
        )
        .unwrap();

        assert_eq!(
            serde_json::to_value(&question).unwrap(),
            json!({
                "type": "noul",
                "instructions": "Does this convey urgency?",
                "criteria": {"true": "Explicitly time-sensitive", "false": "No urgency expressed"}
            })
        );
    }

    #[test]
    fn noul_without_criteria_omits_the_field() {
        let question = Question::noul(text("Is this spam?"), None).unwrap();
        let encoded = serde_json::to_value(&question).unwrap();
        assert_eq!(
            encoded,
            json!({"type": "noul", "instructions": "Is this spam?"})
        );
        assert!(encoded.get("criteria").is_none());
    }

    #[test]
    fn choice_wire_form_matches_the_api_reference() {
        let question = Question::choice(
            text("Which team should handle this?"),
            vec![
                ChoiceOption::new("billing", Some(text("Payments, invoicing, refunds"))).unwrap(),
                ChoiceOption::new("technical", Some(text("Bugs, outages, integrations"))).unwrap(),
            ],
        )
        .unwrap();

        assert_eq!(
            serde_json::to_value(&question).unwrap(),
            json!({
                "type": "choice",
                "instructions": "Which team should handle this?",
                "criteria": {
                    "billing": "Payments, invoicing, refunds",
                    "technical": "Bugs, outages, integrations"
                }
            })
        );
    }

    #[test]
    fn an_undescribed_choice_option_serializes_as_null() {
        // The docs use `null` for options whose names speak for themselves; omitting
        // the key entirely would change the option set the model sees.
        let question = Question::choice(text("Tone?"), options(&["calm", "angry"])).unwrap();
        assert_eq!(
            serde_json::to_value(&question).unwrap()["criteria"],
            json!({"calm": null, "angry": null})
        );
    }

    #[test]
    fn score_wire_form_matches_the_api_reference() {
        let question = Question::score(
            text("How frustrated is the customer?"),
            vec![text("Calm"), text("Frustrated"), text("Very angry")],
        )
        .unwrap();

        assert_eq!(
            serde_json::to_value(&question).unwrap(),
            json!({
                "type": "score",
                "instructions": "How frustrated is the customer?",
                "criteria": ["Calm", "Frustrated", "Very angry"]
            })
        );
    }

    #[test]
    fn choice_cardinality_is_enforced_locally() {
        assert_eq!(
            Question::choice(text("q"), options(&["only"])),
            Err(QuestionError::TooFewOptions { found: 1 })
        );
        // The "only one possible answer" explanation fits one option and not zero.
        assert!(
            QuestionError::TooFewOptions { found: 1 }
                .to_string()
                .contains("only one possible answer")
        );
        let none = QuestionError::TooFewOptions { found: 0 }.to_string();
        assert!(none.contains("found 0"), "{none}");
        assert!(!none.contains("one possible answer"), "{none}");

        let too_many: Vec<ChoiceOption> = (0..=CHOICE_MAX_OPTIONS)
            .map(|index| ChoiceOption::new(format!("option-{index}"), None).unwrap())
            .collect();
        assert_eq!(
            Question::choice(text("q"), too_many),
            Err(QuestionError::TooManyOptions {
                found: CHOICE_MAX_OPTIONS + 1
            })
        );

        let at_limit: Vec<ChoiceOption> = (0..CHOICE_MAX_OPTIONS)
            .map(|index| ChoiceOption::new(format!("option-{index}"), None).unwrap())
            .collect();
        assert!(Question::choice(text("q"), at_limit).is_ok());
    }

    #[test]
    fn duplicate_options_are_rejected() {
        // A duplicate key would silently collapse in the JSON object, changing the
        // option set from what the user wrote.
        assert_eq!(
            Question::choice(text("q"), options(&["a", "b", "a"])),
            Err(QuestionError::DuplicateOption {
                name: "a".to_owned()
            })
        );
    }

    #[test]
    fn score_cardinality_is_enforced_locally() {
        assert_eq!(
            Question::score(text("q"), vec![text("only")]),
            Err(QuestionError::TooFewLevels { found: 1 })
        );
        let too_many: Vec<Content> = (0..=SCORE_MAX_LEVELS)
            .map(|i| text(&i.to_string()))
            .collect();
        assert_eq!(
            Question::score(text("q"), too_many),
            Err(QuestionError::TooManyLevels {
                found: SCORE_MAX_LEVELS + 1
            })
        );
        let at_limit: Vec<Content> = (0..SCORE_MAX_LEVELS)
            .map(|i| text(&i.to_string()))
            .collect();
        assert!(Question::score(text("q"), at_limit).is_ok());
    }

    #[test]
    fn empty_option_names_are_rejected() {
        assert_eq!(
            ChoiceOption::new("  ", None),
            Err(QuestionError::EmptyOptionName)
        );
    }

    #[test]
    fn noul_criteria_must_describe_something() {
        assert_eq!(
            NoulCriteria::new(None, None),
            Err(QuestionError::EmptyNoulCriteria)
        );
        assert!(NoulCriteria::new(Some(text("yes")), None).is_ok());
        assert!(NoulCriteria::new(None, Some(text("no"))).is_ok());
    }

    #[test]
    fn structured_instructions_and_criteria_survive() {
        // The advanced page allows objects and arrays anywhere text is accepted.
        let instructions = Content::try_from(json!({"task": "classify", "notes": ["a"]})).unwrap();
        let question = Question::choice(
            instructions,
            vec![
                ChoiceOption::new(
                    "spam",
                    Some(
                        Content::try_from(json!({"covers": "ads", "excludes": "newsletters"}))
                            .unwrap(),
                    ),
                )
                .unwrap(),
                ChoiceOption::new("ham", None).unwrap(),
            ],
        )
        .unwrap();
        let encoded = serde_json::to_value(&question).unwrap();
        assert_eq!(encoded["instructions"]["task"], json!("classify"));
        assert_eq!(
            encoded["criteria"]["spam"]["excludes"],
            json!("newsletters")
        );
    }
}
