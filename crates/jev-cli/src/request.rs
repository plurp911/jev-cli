//! Building requests from command-line arguments and request files.
//!
//! # Request-file format
//!
//! The document `jev ask --request` reads is the **official API request body**, not an
//! invention of this CLI:
//!
//! ```json
//! {
//!   "state": "…",
//!   "model": "jev-latest",
//!   "questions": {
//!     "is_urgent": {"type": "noul", "instructions": "Does this convey urgency?"}
//!   }
//! }
//! ```
//!
//! `model` is optional and `--model` overrides it. That choice is deliberate: a user
//! can paste an example from <https://docs.typesafe.ai/api> straight into a file and
//! run it, and a file written for `jev` is a valid body for `curl` or an SDK. Several
//! community CLIs invented their own question vocabulary; the result is that the
//! official documentation stops applying to their tool.
//!
//! Every question is validated locally — cardinality, duplicate ids, empty
//! instructions — before anything is sent, so a malformed request costs nothing.

use jev_core::{
    ChoiceOption, Content, EvaluationRequest, ModelId, NoulCriteria, Question, QuestionId, State,
};
use serde_json::Value;

use crate::errors::{CliError, Result};
use crate::ordered::{self, Field, OrderedMap, OrderedObject};

/// A parsed request document.
#[derive(Debug)]
pub struct RequestDocument {
    /// The state, when the document carried one.
    pub state: Option<State>,
    /// The model, when the document named one.
    pub model: Option<ModelId>,
    /// The questions, in document order.
    pub questions: Vec<(QuestionId, Question)>,
}

/// Parses a full request document, or a bare questions map.
///
/// A bare map is accepted so that `--questions` and `jev map --request` can share one
/// parser: if the top level has a `questions` key it is a full document, otherwise it
/// is read as the questions map itself.
///
/// # Errors
///
/// Returns a usage-class [`CliError`] naming the offending question and field.
pub fn parse_document(text: &str, origin: &str) -> Result<RequestDocument> {
    // Parsed order-preserving and duplicate-aware: a `serde_json::Map` would sort the
    // questions alphabetically and silently drop a repeated id. See `crate::ordered`.
    let object = ordered::parse_object(text)
        .map_err(|error| CliError::usage(format!("{origin} is not valid JSON object: {error}")))?;

    if let Some(questions) = ordered::get(&object, "questions") {
        // Guard against the most likely mistake: a file that is *only* a questions map
        // but happens to contain a question called "questions". A question always has a
        // `type`, so a `questions` value that is itself a question belongs to the bare
        // form; anything else means the user wrote a full document.
        //
        // The test used to be that *every* entry under `questions` has a `type`, which
        // made one malformed question fall through to the bare branch and report the
        // first key it tripped over: "question `state` must be an object". The real
        // fault was a question with no `type`, and the message pointed at a key that was
        // completely correct.
        //
        // The test is that `type` holds one of the three type *names*, not merely that
        // the key exists: a full document with a question legitimately called `type`
        // has `questions.type` present as a question *object*, and a presence test took
        // the bare branch for it and then reported ``question `questions`: `type` is
        // missing or not a string`` -- blaming a key the user did not write.
        let is_itself_a_question = questions.as_object().is_some_and(|map| {
            map.get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| ["noul", "choice", "score"].contains(&kind))
        });
        if !is_itself_a_question {
            // Only meaningful for a full document: in a bare questions map a repeated
            // top-level key *is* a repeated question id, and gets the better message.
            if let Some(duplicate) = object.first_duplicate() {
                return Err(CliError::usage(format!(
                    "{origin}: duplicate top-level field {duplicate:?}"
                )));
            }
            let state = match ordered::get(&object, "state") {
                Some(raw) => Some(state_from_value(raw.clone(), origin)?),
                None => None,
            };
            let model = match ordered::get(&object, "model") {
                Some(Value::String(name)) => Some(
                    ModelId::new(name.clone())
                        .map_err(|error| CliError::usage(format!("{origin}: `model`: {error}")))?,
                ),
                Some(_) => {
                    return Err(CliError::usage(format!(
                        "{origin}: `model` must be a string"
                    )));
                }
                None => None,
            };
            reject_unknown_top_level_keys(&object, origin)?;
            let questions = parse_questions_map(text, "questions", origin)?;
            return Ok(RequestDocument {
                state,
                model,
                questions,
            });
        }
    }

    Ok(RequestDocument {
        state: None,
        model: None,
        questions: parse_bare_questions(text, origin)?,
    })
}

/// Rejects a top-level key the API does not define.
///
/// A typo such as `"question"` or `"State"` would otherwise be silently ignored, and
/// the user would be billed for a request that asked nothing they intended.
fn reject_unknown_top_level_keys(object: &OrderedObject, origin: &str) -> Result<()> {
    const KNOWN: &[&str] = &["state", "model", "questions"];
    for (key, _) in object.entries() {
        if !KNOWN.contains(&key.as_str()) {
            return Err(CliError::usage(format!(
                "{origin}: unknown field `{key}`; a request document has `state`, \
                 `model`, and `questions`"
            )));
        }
    }
    Ok(())
}

/// Parses the `questions` map of a full document, preserving order and rejecting
/// duplicates.
///
/// Re-parses `text` rather than walking the already-decoded value: the decoded form has
/// lost both the order and the duplicates by then.
fn parse_questions_map(
    text: &str,
    field: &str,
    origin: &str,
) -> Result<Vec<(QuestionId, Question)>> {
    #[derive(serde::Deserialize)]
    struct Wrapper {
        questions: OrderedMap<Field>,
    }

    let wrapper: Wrapper = serde_json::from_str(text).map_err(|error| {
        CliError::usage(format!("{origin}: `{field}` is not an object: {error}"))
    })?;
    entries_to_questions(&wrapper.questions, origin)
}

/// Parses a document that is itself the questions map.
fn parse_bare_questions(text: &str, origin: &str) -> Result<Vec<(QuestionId, Question)>> {
    // Re-parsed for the same reason `parse_questions_map` re-parses: the already-decoded
    // form has lost the order and the duplicates *inside* each question body.
    let object: OrderedMap<Field> = serde_json::from_str(text)
        .map_err(|error| CliError::usage(format!("{origin} is not valid JSON object: {error}")))?;
    entries_to_questions(&object, origin)
}

/// Shared validation for an ordered set of question entries.
fn entries_to_questions(
    object: &OrderedMap<Field>,
    origin: &str,
) -> Result<Vec<(QuestionId, Question)>> {
    if object.is_empty() {
        return Err(CliError::usage(format!(
            "{origin}: `questions` is empty; a request needs at least one question"
        )));
    }
    // This is the check `serde_json::Map` made unreachable: without an order-preserving
    // parse, the second `"dup"` would have overwritten the first before ever getting
    // here, and the user would pay for a request missing a question they wrote.
    if let Some(duplicate) = object.first_duplicate() {
        return Err(CliError::usage(format!(
            "{origin}: duplicate question id {duplicate:?}"
        )));
    }

    object
        .entries()
        .iter()
        .map(|(key, raw)| {
            let id = QuestionId::new(key.clone()).map_err(|error| {
                CliError::usage(format!("{origin}: question id {key:?}: {error}"))
            })?;
            let question = parse_question(raw, key, origin)?;
            Ok((id, question))
        })
        .collect()
}

fn parse_question(raw: &Field, id: &str, origin: &str) -> Result<Question> {
    let where_ = format!("{origin}: question `{id}`");
    let object = raw
        .as_object()
        .ok_or_else(|| CliError::usage(format!("{where_} must be an object")))?;

    // The same rule the two levels above already enforce. Without it a question with
    // `"type"` twice was last-one-wins: the user wrote a Noul, `jev` sent a Choice, and
    // nothing said so.
    if let Some(duplicate) = object.first_duplicate() {
        return Err(CliError::usage(format!(
            "{where_}: duplicate field {duplicate:?}"
        )));
    }

    for (key, _) in object.entries() {
        if !["type", "instructions", "criteria"].contains(&key.as_str()) {
            return Err(CliError::usage(format!(
                "{where_}: unknown field `{key}`; a question has `type`, `instructions`, \
                 and `criteria`"
            )));
        }
    }

    let kind = ordered::get(object, "type")
        .map(Field::to_value)
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or_else(|| CliError::usage(format!("{where_}: `type` is missing or not a string")))?;

    let instructions = ordered::get(object, "instructions")
        .map(Field::to_value)
        .ok_or_else(|| CliError::usage(format!("{where_}: `instructions` is required")))?;
    let instructions = Content::try_from(instructions)
        .map_err(|error| CliError::usage(format!("{where_}: `instructions`: {error}")))?;

    let criteria = ordered::get(object, "criteria");

    match kind.as_str() {
        "noul" => parse_noul(
            instructions,
            criteria.map(Field::to_value).as_ref(),
            &where_,
        ),
        "choice" => parse_choice(instructions, criteria, &where_),
        "score" => parse_score(
            instructions,
            criteria.map(Field::to_value).as_ref(),
            &where_,
        ),
        other => Err(CliError::usage(format!(
            "{where_}: unknown question type {other:?}; the API defines `noul`, `choice`, \
             and `score`"
        ))),
    }
}

fn parse_noul(instructions: Content, criteria: Option<&Value>, where_: &str) -> Result<Question> {
    {
        {
            let criteria = match criteria {
                None | Some(Value::Null) => None,
                Some(Value::Object(map)) => {
                    for key in map.keys() {
                        if !["true", "false"].contains(&key.as_str()) {
                            return Err(CliError::usage(format!(
                                "{where_}: noul criteria may only have `true` and `false`, \
                                 found `{key}`"
                            )));
                        }
                    }
                    let side = |name: &str| -> Result<Option<Content>> {
                        match map.get(name) {
                            None | Some(Value::Null) => Ok(None),
                            Some(value) => {
                                Content::try_from(value.clone()).map(Some).map_err(|error| {
                                    CliError::usage(format!("{where_}: `criteria.{name}`: {error}"))
                                })
                            }
                        }
                    };
                    Some(
                        NoulCriteria::new(side("true")?, side("false")?).map_err(|error| {
                            CliError::usage(format!("{where_}: `criteria`: {error}"))
                        })?,
                    )
                }
                Some(_) => {
                    return Err(CliError::usage(format!(
                        "{where_}: noul `criteria` must be an object with `true` and `false`"
                    )));
                }
            };
            Question::noul(instructions, criteria)
                .map_err(|error| CliError::usage(format!("{where_}: {error}")))
        }
    }
}

/// Parses Choice `criteria`, **in the order the user wrote them**.
///
/// Taken as a [`Field`] rather than a [`Value`] because option order is the part of a
/// question a model can be sensitive to, and a `serde_json::Map` is a `BTreeMap`: options
/// written zebra, apple, mango went on the wire as apple, mango, zebra, which also broke
/// `--dry-run`'s promise that the body can be compared against the file. A duplicate
/// option name was silently last-one-wins for the same reason.
fn parse_choice(instructions: Content, criteria: Option<&Field>, where_: &str) -> Result<Question> {
    let map = criteria.and_then(Field::as_object).ok_or_else(|| {
        CliError::usage(format!(
            "{where_}: a choice question needs `criteria`, an object mapping each \
                 option name to a description or null"
        ))
    })?;
    if let Some(duplicate) = map.first_duplicate() {
        return Err(CliError::usage(format!(
            "{where_}: duplicate option name {duplicate:?} in `criteria`"
        )));
    }
    let options = map
        .entries()
        .iter()
        .map(|(name, description)| {
            let description = match description.to_value() {
                Value::Null => None,
                other => Some(Content::try_from(other).map_err(|error| {
                    CliError::usage(format!("{where_}: `criteria.{name}`: {error}"))
                })?),
            };
            ChoiceOption::new(name.clone(), description)
                .map_err(|error| CliError::usage(format!("{where_}: {error}")))
        })
        .collect::<Result<Vec<_>>>()?;
    Question::choice(instructions, options)
        .map_err(|error| CliError::usage(format!("{where_}: {error}")))
}

fn parse_score(instructions: Content, criteria: Option<&Value>, where_: &str) -> Result<Question> {
    {
        {
            let list = criteria.and_then(Value::as_array).ok_or_else(|| {
                CliError::usage(format!(
                    "{where_}: a score question needs `criteria`, an ordered array of level \
                     descriptions from lowest to highest"
                ))
            })?;
            let levels = list
                .iter()
                .enumerate()
                .map(|(index, level)| {
                    Content::try_from(level.clone()).map_err(|error| {
                        CliError::usage(format!("{where_}: `criteria[{index}]`: {error}"))
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Question::score(instructions, levels)
                .map_err(|error| CliError::usage(format!("{where_}: {error}")))
        }
    }
}

fn state_from_value(value: Value, origin: &str) -> Result<State> {
    Content::try_from(value)
        .map(State::new)
        .map_err(|error| CliError::usage(format!("{origin}: `state`: {error}")))
}

/// Parses a `name` or `name=description` option from the command line.
///
/// Split on the **first** `=`, so a description may contain one.
///
/// # Errors
///
/// Returns a usage-class [`CliError`] for a blank name or a blank description.
pub fn parse_option(raw: &str) -> Result<ChoiceOption> {
    let (name, description) = match raw.split_once('=') {
        Some((name, description)) => {
            let description = Content::text(description).map_err(|_| {
                CliError::usage(format!(
                    "--option {raw:?}: the description after `=` is empty; write just \
                     `{name}` for an option with no description"
                ))
            })?;
            (name, Some(description))
        }
        None => (raw, None),
    };
    ChoiceOption::new(name, description)
        .map_err(|error| CliError::usage(format!("--option {raw:?}: {error}")))
}

/// Parses a JSON object of options, as written by `--options-file`.
///
/// # Errors
///
/// Returns a usage-class [`CliError`] when the document is not an object of
/// name-to-description entries.
pub fn parse_options_file(text: &str, origin: &str) -> Result<Vec<ChoiceOption>> {
    // Parsed order-preserving and duplicate-aware, for the reason `parse_choice` is: a
    // `serde_json::Map` sent `{"zeta": …, "alpha": …}` as alpha, zeta and kept only the
    // last of a repeated name. The syntax check comes first so that malformed JSON and
    // a well-formed non-object get their own messages.
    let value: Value = serde_json::from_str(text)
        .map_err(|error| CliError::usage(format!("{origin} is not valid JSON: {error}")))?;
    if !value.is_object() {
        return Err(CliError::usage(format!(
            "{origin} must be a JSON object mapping each option name to a description or null"
        )));
    }
    let map: OrderedMap<Field> = serde_json::from_str(text)
        .map_err(|error| CliError::usage(format!("{origin} is not valid JSON: {error}")))?;
    if let Some(duplicate) = map.first_duplicate() {
        return Err(CliError::usage(format!(
            "{origin}: duplicate option name {duplicate:?}"
        )));
    }
    map.entries()
        .iter()
        .map(|(name, description)| {
            let description = match description.to_value() {
                Value::Null => None,
                other => Some(
                    Content::try_from(other)
                        .map_err(|error| CliError::usage(format!("{origin}: {name}: {error}")))?,
                ),
            };
            ChoiceOption::new(name.clone(), description)
                .map_err(|error| CliError::usage(format!("{origin}: {error}")))
        })
        .collect()
}

/// Parses a JSON array of Score levels, as written by `--levels-file`.
///
/// # Errors
///
/// Returns a usage-class [`CliError`] when the document is not an array of level
/// descriptions.
pub fn parse_levels_file(text: &str, origin: &str) -> Result<Vec<Content>> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| CliError::usage(format!("{origin} is not valid JSON: {error}")))?;
    let list = value.as_array().ok_or_else(|| {
        CliError::usage(format!(
            "{origin} must be a JSON array of level descriptions, lowest first"
        ))
    })?;
    list.iter()
        .enumerate()
        .map(|(index, level)| {
            Content::try_from(level.clone())
                .map_err(|error| CliError::usage(format!("{origin}: level {index}: {error}")))
        })
        .collect()
}

/// Assembles a validated request.
///
/// # Errors
///
/// Returns a usage-class [`CliError`] for an empty or duplicate-bearing question set.
pub fn build(
    state: State,
    model: ModelId,
    questions: Vec<(QuestionId, Question)>,
) -> Result<EvaluationRequest> {
    EvaluationRequest::new(state, model, questions)
        .map_err(|error| CliError::usage(error.to_string()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const ORIGIN: &str = "request.json";

    #[test]
    fn the_documented_api_request_body_parses_unchanged() {
        // Pasted from https://docs.typesafe.ai/api, "Example request". A user must be
        // able to copy an example out of the docs and run it.
        let text = json!({
            "state": "Help! My payouts have been failing for 3 days.",
            "model": "jev-latest",
            "questions": {
                "is_urgent": {
                    "type": "noul",
                    "instructions": "Does this convey urgency?",
                    "criteria": {
                        "true": "Explicitly time-sensitive",
                        "false": "No urgency expressed"
                    }
                }
            }
        })
        .to_string();

        let document = parse_document(&text, ORIGIN).unwrap();
        assert!(document.state.is_some());
        assert_eq!(
            document.model.as_ref().map(ModelId::as_str),
            Some("jev-latest")
        );
        assert_eq!(document.questions.len(), 1);

        // And what we would send back is byte-identical in meaning to what came in.
        let request = build(
            document.state.unwrap(),
            document.model.unwrap(),
            document.questions,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::from_str::<Value>(&text).unwrap()
        );
    }

    #[test]
    fn a_mixed_document_parses_every_primitive() {
        let text = json!({
            "state": {"subject": "late shoes"},
            "questions": {
                "urgent": {"type": "noul", "instructions": "Urgent?"},
                "team": {
                    "type": "choice",
                    "instructions": "Which team?",
                    "criteria": {"returns": "Exchanges", "shipping": null}
                },
                "severity": {
                    "type": "score",
                    "instructions": "How severe?",
                    "criteria": ["Cosmetic", "Degraded", "Blocking"]
                }
            }
        })
        .to_string();
        let document = parse_document(&text, ORIGIN).unwrap();
        assert_eq!(document.questions.len(), 3);
        assert!(document.model.is_none());
    }

    #[test]
    fn a_bare_questions_map_parses() {
        let text = json!({"urgent": {"type": "noul", "instructions": "Urgent?"}}).to_string();
        let document = parse_document(&text, ORIGIN).unwrap();
        assert!(document.state.is_none());
        assert_eq!(document.questions.len(), 1);
    }

    #[test]
    fn a_typo_in_a_top_level_key_is_refused_not_ignored() {
        // Silently ignoring `questons` would bill the user for a request that asked
        // nothing they intended.
        let text = json!({
            "state": "x",
            "questions": {"a": {"type": "noul", "instructions": "?"}},
            "modle": "jev-latest"
        })
        .to_string();
        let error = parse_document(&text, ORIGIN).unwrap_err();
        assert!(
            error.to_string().contains("unknown field `modle`"),
            "{error}"
        );
    }

    #[test]
    fn a_typo_in_a_question_field_is_refused() {
        let text = json!({"a": {"type": "noul", "instruction": "?"}}).to_string();
        assert!(parse_document(&text, ORIGIN).is_err());
    }

    #[test]
    fn an_unknown_question_type_names_the_three_that_exist() {
        // The `boolean` type is a gateway abstraction some community tools expose; it
        // is not the API's vocabulary, and a user who tries it deserves to be told.
        let text = json!({"a": {"type": "boolean", "instructions": "?"}}).to_string();
        let error = parse_document(&text, ORIGIN).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("noul"));
        assert!(message.contains("choice"));
        assert!(message.contains("score"));
    }

    #[test]
    fn cardinality_violations_are_caught_before_the_request_is_sent() {
        let one_option =
            json!({"a": {"type": "choice", "instructions": "?", "criteria": {"x": null}}})
                .to_string();
        assert!(parse_document(&one_option, ORIGIN).is_err());

        let one_level =
            json!({"a": {"type": "score", "instructions": "?", "criteria": ["only"]}}).to_string();
        assert!(parse_document(&one_level, ORIGIN).is_err());

        let eleven: Vec<String> = (0..11).map(|i| format!("level {i}")).collect();
        let too_many =
            json!({"a": {"type": "score", "instructions": "?", "criteria": eleven}}).to_string();
        assert!(parse_document(&too_many, ORIGIN).is_err());
    }

    #[test]
    fn missing_criteria_is_explained_rather_than_sent() {
        let error = parse_document(
            &json!({"a": {"type": "choice", "instructions": "?"}}).to_string(),
            ORIGIN,
        )
        .unwrap_err();
        assert!(error.to_string().contains("needs `criteria`"));
    }

    #[test]
    fn an_empty_questions_map_is_refused() {
        assert!(
            parse_document(&json!({"state": "x", "questions": {}}).to_string(), ORIGIN).is_err()
        );
        assert!(parse_document("{}", ORIGIN).is_err());
    }

    #[test]
    fn noul_criteria_accepts_only_true_and_false() {
        let text = json!({
            "a": {"type": "noul", "instructions": "?", "criteria": {"maybe": "x"}}
        })
        .to_string();
        assert!(parse_document(&text, ORIGIN).is_err());
    }

    #[test]
    fn options_split_on_the_first_equals_only() {
        let option = parse_option("billing=payments, invoices = refunds").unwrap();
        assert_eq!(option.name(), "billing");
        assert_eq!(
            option.description().and_then(Content::as_text),
            Some("payments, invoices = refunds")
        );
    }

    #[test]
    fn an_option_without_a_description_has_none() {
        let option = parse_option("calm").unwrap();
        assert_eq!(option.name(), "calm");
        assert!(option.description().is_none());
    }

    #[test]
    fn a_blank_option_description_is_a_usage_error_with_a_suggestion() {
        let error = parse_option("calm=").unwrap_err();
        assert!(error.to_string().contains("write just `calm`"));
    }

    #[test]
    fn an_options_file_parses_names_and_nulls() {
        let options = parse_options_file(r#"{"a": "first", "b": null}"#, "options.json").unwrap();
        assert_eq!(options.len(), 2);
        assert!(
            options
                .iter()
                .any(|o| o.name() == "b" && o.description().is_none())
        );
    }

    #[test]
    fn an_options_file_keeps_the_order_it_was_written_in() {
        // `docs/commands.md` promises options go out in written order. Parsed through a
        // `serde_json::Map`, these were sent as alpha, zeta.
        let options =
            parse_options_file(r#"{"zeta": null, "alpha": null}"#, "options.json").unwrap();
        let names: Vec<&str> = options.iter().map(ChoiceOption::name).collect();
        assert_eq!(names, vec!["zeta", "alpha"]);
    }

    #[test]
    fn an_options_file_rejects_a_repeated_name_like_a_request_file_does() {
        // Last-one-wins used to accept the first silently and misreport the second as
        // "found 1". Both must be the same error `ask -r` gives for the same mistake.
        for text in [
            r#"{"a": null, "a": "x", "b": null}"#,
            r#"{"a": null, "a": null}"#,
        ] {
            let error = parse_options_file(text, "options.json")
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("duplicate option name \"a\""),
                "{text}: {error}"
            );
            assert!(error.contains("options.json"), "{error}");
        }
    }

    #[test]
    fn a_levels_file_parses_in_order() {
        let levels = parse_levels_file(r#"["low", "high"]"#, "levels.json").unwrap();
        assert_eq!(levels.len(), 2);
        assert_eq!(levels.first().and_then(Content::as_text), Some("low"));
    }

    #[test]
    fn malformed_json_names_the_file() {
        let error = parse_document("{oops", "my-request.json").unwrap_err();
        assert!(error.to_string().contains("my-request.json"));
    }

    #[test]
    fn duplicate_question_ids_are_refused_rather_than_silently_collapsed() {
        // `serde_json::Map` keeps only the last entry, so without an order-preserving
        // parse this document would be accepted with one question instead of two and
        // the user would pay for a request they did not write.
        let text = r#"{"state":"x","questions":{
            "dup":{"type":"noul","instructions":"first"},
            "dup":{"type":"noul","instructions":"second"}}}"#;
        let error = parse_document(text, ORIGIN).unwrap_err();
        assert!(
            error.to_string().contains("duplicate question id"),
            "{error}"
        );

        // Same for a bare questions map.
        let bare = r#"{"dup":{"type":"noul","instructions":"a"},
                       "dup":{"type":"noul","instructions":"b"}}"#;
        assert!(
            parse_document(bare, ORIGIN)
                .unwrap_err()
                .to_string()
                .contains("duplicate question id")
        );
    }

    #[test]
    fn a_duplicate_top_level_field_is_refused() {
        let text =
            r#"{"state":"x","state":"y","questions":{"a":{"type":"noul","instructions":"?"}}}"#;
        let error = parse_document(text, ORIGIN).unwrap_err();
        assert!(
            error.to_string().contains("duplicate top-level field"),
            "{error}"
        );
    }

    #[test]
    fn question_order_follows_the_document_not_the_alphabet() {
        // `--dry-run` promises a body the user can compare against their file, and
        // human output reads in the order the questions were asked.
        let text = r#"{"state":"x","questions":{
            "zebra":{"type":"noul","instructions":"z"},
            "apple":{"type":"noul","instructions":"a"},
            "mango":{"type":"noul","instructions":"m"}}}"#;
        let document = parse_document(text, ORIGIN).unwrap();
        let ids: Vec<&str> = document
            .questions
            .iter()
            .map(|(id, _)| id.as_str())
            .collect();
        assert_eq!(ids, vec!["zebra", "apple", "mango"]);
    }

    #[test]
    fn a_hostile_question_id_cannot_reach_the_terminal_raw() {
        let text = json!({"a\u{1b}[2J": {"type": "noul", "instructions": "?"}}).to_string();
        let error = parse_document(&text, ORIGIN).unwrap_err();
        assert!(!error.to_string().contains('\u{1b}'));
    }
}
