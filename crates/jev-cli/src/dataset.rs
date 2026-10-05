//! The labelled dataset `jev eval` measures a question against.
//!
//! # The format
//!
//! JSONL, one labelled example per line:
//!
//! ```json
//! {"schema":"jev.eval.row/v1","id":"1","state":"payouts have failed for three days",
//!  "labels":{"urgent":true,"team":"billing"}}
//! ```
//!
//! It composes with the reusable request file rather than replacing it: the questions
//! live in the same `-r` document `jev ask` and `jev map` already take, and a row here
//! supplies only the state and the ground truth. That split is what lets the same
//! committed question set be run, batched, and *evaluated* without being written three
//! times — and it is why `labels` is keyed by question id.
//!
//! A row may label a subset of the questions. Each question is scored over whichever
//! rows carry a label for it, and the report says how many that was.
//!
//! # Why a `schema` field, when a request file has none
//!
//! A request file *is* the official API request body, so inventing a version field for
//! it would make a document that no longer pastes into `curl`. A dataset row is this
//! CLI's own invention, so it follows this project's own rule instead: every document
//! `jev` defines carries a version a consumer can branch on.
//!
//! # Labels never leave the machine
//!
//! A [`LabeledRow`] keeps its ground truth in `labels`, and the request is built from
//! `state` alone. The types are what enforce that: there is no path from a `Label` into
//! an `EvaluationRequest`, so sending the answer along with the question is not a
//! mistake a call site can make. See `docs/threat-model.md` T11.

use std::collections::BTreeMap;

use jev_core::{EmbeddedImage, Question, QuestionId, State};
use serde_json::Value;

use crate::digest::{self, fnv1a};
use crate::errors::{CliError, Result};

/// Schema identifier every dataset row must carry.
pub const ROW_SCHEMA: &str = "jev.eval.row/v1";

/// Most labelled rows `jev eval` will read.
///
/// The same ceiling `jev map` puts on input records, for the same reason: rows are held
/// in memory so that the split is reproducible and the metrics see the whole set.
pub const MAX_ROWS: usize = 1_000_000;

/// The ground truth for one question, in the shape that question's answers take.
///
/// An enum rather than a raw `Value` so that "a Noul labelled with an option name" is a
/// state that cannot be constructed rather than one every metric has to re-check.
#[derive(Debug, Clone, PartialEq)]
pub enum Label {
    /// The expected answer to a yes/no question.
    Noul(bool),
    /// The expected option of a Choice, which must be one the question declared.
    Choice(String),
    /// The expected level of a Score, as a zero-based index into its legend.
    Score(u32),
}

impl Label {
    /// The wire name of the question kind this label belongs to.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Noul(_) => "noul",
            Self::Choice(_) => "choice",
            Self::Score(_) => "score",
        }
    }
}

/// One labelled example.
#[derive(Debug, Clone)]
pub struct LabeledRow {
    /// The row's stable identifier. Unique within the file, and the key the split is
    /// derived from, so the same row lands on the same side however the file is
    /// reordered or extended.
    pub id: String,
    /// The state that is sent. The only field that ever reaches the API.
    pub state: State,
    /// Embedded media and request options; labels are never included.
    pub(crate) features: crate::media::Features,
    /// The ground truth, keyed by question id. Never sent.
    pub labels: BTreeMap<String, Label>,
}

/// A parsed dataset, with the fingerprint that identifies it in a report.
#[derive(Debug, Clone)]
pub struct Dataset {
    /// The rows, in file order.
    pub rows: Vec<LabeledRow>,
    /// A digest of every row's id, state, and labels, in file order.
    pub fingerprint: String,
}

impl Dataset {
    /// Keeps only the first `limit` rows, and re-fingerprints.
    ///
    /// The fingerprint must describe the rows that were actually evaluated, not the
    /// file they came from: a report whose fingerprint names a thousand rows when sixty
    /// were measured is a report that cannot be compared with anything.
    #[must_use]
    pub fn truncated(mut self, limit: usize) -> Self {
        if self.rows.len() > limit {
            self.rows.truncate(limit);
            self.fingerprint = fingerprint_of(&self.rows);
        }
        self
    }
}

/// Parses a JSONL dataset and validates every label against `questions`.
///
/// Validation is total and happens before anything is sent: a label naming a question
/// that is not in the request file, or a Choice label that is not one of the declared
/// options, is a mistake worth catching for free rather than after two hundred billed
/// rows.
///
/// # Errors
///
/// Returns a usage-class [`CliError`] naming the origin, the line, and the field.
pub fn parse(
    text: &str,
    origin: &str,
    questions: &[(QuestionId, Question)],
    request_origin: &str,
) -> Result<Dataset> {
    parse_for_provider(text, origin, questions, request_origin, "typesafe", &[])
}

/// Parses optional images using the explicitly selected provider's input format.
pub(crate) fn parse_for_provider(
    text: &str,
    origin: &str,
    questions: &[(QuestionId, Question)],
    request_origin: &str,
    provider: &str,
    template_images: &[EmbeddedImage],
) -> Result<Dataset> {
    let known: BTreeMap<&str, &Question> = questions
        .iter()
        .map(|(id, question)| (id.as_str(), question))
        .collect();

    let mut rows: Vec<LabeledRow> = Vec::new();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();

    for (offset, line) in text.lines().enumerate() {
        let number = offset + 1;
        if line.trim().is_empty() {
            continue;
        }
        if rows.len() >= MAX_ROWS {
            return Err(CliError::usage(format!(
                "{origin}: more than {MAX_ROWS} labelled rows; split the file, or cap \
                 the run with --limit"
            )));
        }
        let row = parse_row(
            line,
            origin,
            number,
            &known,
            request_origin,
            provider,
            template_images,
        )?;
        if let Some(first) = seen.insert(row.id.clone(), number) {
            return Err(CliError::usage(format!(
                "{origin}: duplicate row id `{}` (line {first}, and again at line \
                 {number}).\n\nIds are the split key, so a repeated one would put the \
                 same example on both sides of the holdout.",
                crate::output::Safe::new(&row.id),
            )));
        }
        rows.push(row);
    }

    if rows.is_empty() {
        return Err(CliError::usage(format!(
            "{origin}: no labelled rows; a dataset needs at least one"
        )));
    }

    let fingerprint = fingerprint_of(&rows);
    Ok(Dataset { rows, fingerprint })
}

fn parse_row(
    line: &str,
    origin: &str,
    number: usize,
    known: &BTreeMap<&str, &Question>,
    request_origin: &str,
    provider: &str,
    template_images: &[EmbeddedImage],
) -> Result<LabeledRow> {
    let where_ = format!("{origin} line {number}");
    let value = crate::ordered::parse_unambiguous_value(line)
        .map_err(|error| CliError::usage(format!("{where_} is not valid JSON: {error}")))?;
    let Some(object) = value.as_object() else {
        return Err(CliError::usage(format!(
            "{where_}: a dataset row must be a JSON object"
        )));
    };

    // Checked first: a file of `jev map` rows, or of bare records, is the likeliest
    // wrong thing to point at this flag, and every other message would describe a
    // missing field rather than the actual mistake.
    match object.get("schema").and_then(Value::as_str) {
        Some(ROW_SCHEMA) => {}
        found => {
            return Err(CliError::usage(format!(
                "{where_}: `schema` is {}, not `{ROW_SCHEMA}`.\n\nA labelled row looks \
                 like:\n  {{\"schema\":\"{ROW_SCHEMA}\",\"id\":\"1\",\"state\":\"…\",\
                 \"labels\":{{\"<question id>\":<ground truth>}}}}",
                found.map_or_else(
                    || "missing".to_owned(),
                    |found| format!("`{}`", crate::output::Safe::new(found))
                )
            )));
        }
    }

    for key in object.keys() {
        if !["schema", "id", "state", "labels", "images", "videos"].contains(&key.as_str()) {
            return Err(CliError::usage(format!(
                "{where_}: unknown field `{}`; a dataset row has `schema`, `id`, \
                 `state`, `labels`, `images`, and `videos`",
                crate::output::Safe::new(key)
            )));
        }
    }

    let id = match object.get("id") {
        Some(Value::String(id)) if !id.trim().is_empty() => {
            if id.chars().any(char::is_control) {
                return Err(CliError::usage(format!(
                    "{where_}: `id` contains a control character"
                )));
            }
            id.clone()
        }
        _ => {
            return Err(CliError::usage(format!(
                "{where_}: `id` is required and must be a non-empty string"
            )));
        }
    };

    let Some(raw_state) = object.get("state") else {
        return Err(CliError::usage(format!("{where_}: `state` is required")));
    };
    // The same bounded, depth-checked conversion a request file's `state` goes through.
    // A dataset file is untrusted input like any other.
    let images = crate::media::parse_image_field(line, "images", &where_, provider == "ollama")?;
    let videos = crate::media::parse_video_field(line, "videos", &where_)?;
    let state_images = if provider == "cloudflare" {
        if images.is_empty() {
            template_images
        } else {
            &images
        }
    } else {
        &[][..]
    };
    let state = crate::request::ContentMode::for_provider(provider).state(
        raw_state.clone(),
        &where_,
        state_images,
    )?;

    let Some(Value::Object(raw_labels)) = object.get("labels") else {
        return Err(CliError::usage(format!(
            "{where_}: `labels` must be an object mapping a question id to its ground \
             truth"
        )));
    };
    if raw_labels.is_empty() {
        return Err(CliError::usage(format!(
            "{where_}: `labels` is empty; a row must label at least one question"
        )));
    }

    let mut labels = BTreeMap::new();
    for (question_id, raw) in raw_labels {
        let Some(question) = known.get(question_id.as_str()) else {
            // Refused rather than ignored, for the same reason a request file refuses an
            // unknown top-level key: a silently dropped label looks scored and was not.
            return Err(CliError::usage(format!(
                "{where_}: `labels` names question `{}`, which is not in \
                 {request_origin}",
                crate::output::Safe::new(question_id)
            )));
        };
        labels.insert(
            question_id.clone(),
            label_for(question, raw, &where_, question_id)?,
        );
    }

    Ok(LabeledRow {
        id,
        state,
        labels,
        features: crate::media::Features {
            images,
            videos,
            ..crate::media::Features::default()
        },
    })
}

/// Validates one ground-truth value against the question it labels.
fn label_for(question: &Question, raw: &Value, where_: &str, question_id: &str) -> Result<Label> {
    let refuse = |detail: String| {
        CliError::usage(format!(
            "{where_}: labels.{}: {detail}",
            crate::output::Safe::new(question_id)
        ))
    };
    match question {
        Question::Noul { .. } => match raw {
            Value::Bool(value) => Ok(Label::Noul(*value)),
            // `0`/`1` accepted because a dataset exported from a spreadsheet or a SQL
            // query almost always has them, and refusing would make the common path a
            // transformation step.
            Value::Number(number) => match number.as_i64() {
                Some(0) => Ok(Label::Noul(false)),
                Some(1) => Ok(Label::Noul(true)),
                _ => Err(refuse(format!(
                    "a noul label must be true, false, 0, or 1; found {number}"
                ))),
            },
            other => Err(refuse(format!(
                "a noul label must be true, false, 0, or 1; found {}",
                describe(other)
            ))),
        },
        Question::Choice { options, .. } => {
            let Some(name) = raw.as_str() else {
                return Err(refuse(format!(
                    "a choice label must be one of the question's option names, as a \
                     string; found {}",
                    describe(raw)
                )));
            };
            if options.iter().any(|option| option.name() == name) {
                Ok(Label::Choice(name.to_owned()))
            } else {
                // The option list is quoted so the user can see the spelling they missed;
                // it came from their own request file, and it is sanitized like any other
                // echoed input.
                let declared: Vec<String> = options
                    .iter()
                    .map(|option| format!("`{}`", crate::output::Safe::new(option.name())))
                    .collect();
                Err(refuse(format!(
                    "`{}` is not one of this question's options ({})",
                    crate::output::Safe::new(name),
                    declared.join(", ")
                )))
            }
        }
        Question::Score { levels, .. } => {
            // The level *index*, not the level's description. Matching against the prose
            // a user wrote to prompt a model would mean fuzzy-matching free text to
            // decide what a ground truth was, which is exactly the kind of guess a
            // calibration tool must not make.
            let count = u64::try_from(levels.len()).unwrap_or(u64::MAX);
            match raw.as_u64() {
                Some(index) if index < count => u32::try_from(index)
                    .map(Label::Score)
                    .map_err(|_| refuse("a score label must fit in 32 bits".to_owned())),
                _ => Err(refuse(format!(
                    "a score label must be a whole level index from 0 to {}; found {}",
                    count.saturating_sub(1),
                    describe(raw)
                ))),
            }
        }
    }
}

/// Names a JSON value's type for an error message, without quoting its content.
fn describe(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(_) => "a string".to_owned(),
        Value::Array(_) => "an array".to_owned(),
        Value::Object(_) => "an object".to_owned(),
    }
}

/// A digest of the rows, in file order.
///
/// Recorded in the report so that two runs can be compared, and so that a threshold can
/// be traced back to the exact examples it was measured on. It is a change detector,
/// not a commitment: it carries none of the content it summarizes.
#[must_use]
pub fn fingerprint_of(rows: &[LabeledRow]) -> String {
    let mut rendered = String::new();
    for row in rows {
        rendered.push(digest::RECORD);
        rendered.push_str(&row.id);
        rendered.push(digest::UNIT);
        rendered.push_str(&row.state.content().to_value().to_string());
        if !row.features.images.is_empty() || !row.features.videos.is_empty() {
            rendered.push(digest::UNIT);
            rendered.push_str(
                &serde_json::to_string(&(&row.features.images, &row.features.videos))
                    .unwrap_or_default(),
            );
        }
        for (question, label) in &row.labels {
            rendered.push(digest::UNIT);
            rendered.push_str(question);
            rendered.push(digest::UNIT);
            match label {
                Label::Noul(value) => rendered.push_str(if *value { "true" } else { "false" }),
                Label::Choice(name) => rendered.push_str(name),
                Label::Score(level) => rendered.push_str(&level.to_string()),
            }
        }
    }
    fnv1a(&rendered)
}

/// Which side of the holdout a row is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Rows a threshold is chosen on.
    Calibration,
    /// Rows the chosen threshold is reported on.
    Test,
}

/// Assigns a row to a side, deterministically, from its id and the seed.
///
/// Keyed on the **id**, not on the row's position, so the assignment does not move when
/// the file is sorted, filtered, or appended to. That is the property that makes a
/// seeded split reproducible in the way a reader of the report expects: rerunning after
/// adding twenty examples re-uses the same split for the original rows instead of
/// reshuffling everything and quietly changing what "test" means.
///
/// FNV-1a rather than the standard library's hasher for the same reason `jev map`
/// uses it: `DefaultHasher` is explicitly not stable across releases, and a split that
/// moved when the user upgraded their toolchain would make two reports incomparable for
/// no visible reason.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    reason = "the digest is deliberately projected onto the unit interval; the low bits \
              lost to the cast do not change which side of a fraction it falls on in any \
              way a reader could depend on"
)]
pub fn side(id: &str, seed: u64, test_fraction: f64) -> Side {
    // Mixed, not raw: FNV-1a's high bits avalanche weakly for short, similar ids, and
    // it is precisely the high bits this projection reads. See `digest::mix`.
    let bits = digest::mix(digest::fnv1a_u64(&format!("{seed}{}{id}", digest::UNIT)));
    let position = bits as f64 / (u64::MAX as f64);
    if position < test_fraction {
        Side::Test
    } else {
        Side::Calibration
    }
}

#[cfg(test)]
mod tests {
    use jev_core::ChoiceOption;
    use jev_core::Content;

    use super::*;

    fn text(value: &str) -> Content {
        Content::text(value).unwrap()
    }

    fn questions() -> Vec<(QuestionId, Question)> {
        vec![
            (
                QuestionId::new("urgent").unwrap(),
                Question::noul(text("Is it urgent?"), None).unwrap(),
            ),
            (
                QuestionId::new("team").unwrap(),
                Question::choice(
                    text("Which team?"),
                    vec![
                        ChoiceOption::new("billing", None).unwrap(),
                        ChoiceOption::new("support", None).unwrap(),
                    ],
                )
                .unwrap(),
            ),
            (
                QuestionId::new("severity").unwrap(),
                Question::score(text("How bad?"), vec![text("low"), text("high")]).unwrap(),
            ),
        ]
    }

    fn parse_all(text: &str) -> Result<Dataset> {
        parse(text, "data.jsonl", &questions(), "q.json")
    }

    fn row(labels: &str) -> String {
        format!("{{\"schema\":\"{ROW_SCHEMA}\",\"id\":\"1\",\"state\":\"s\",\"labels\":{labels}}}")
    }

    fn message(text: &str) -> String {
        parse_all(text)
            .expect_err("this row should be refused")
            .to_string()
    }

    #[test]
    fn labelled_rows_accept_explicit_embedded_media() {
        let text = r#"{"schema":"jev.eval.row/v1","id":"one","state":"receipt","images":[],"labels":{"urgent":true}}"#;
        assert!(parse_all(text).is_ok());
    }

    #[test]
    fn a_well_formed_row_parses_every_label_type() {
        let dataset = parse_all(&row(
            "{\"urgent\":true,\"team\":\"billing\",\"severity\":1}",
        ))
        .unwrap();
        assert_eq!(dataset.rows.len(), 1);
        let labels = &dataset.rows.first().unwrap().labels;
        assert_eq!(labels.get("urgent"), Some(&Label::Noul(true)));
        assert_eq!(
            labels.get("team"),
            Some(&Label::Choice("billing".to_owned()))
        );
        assert_eq!(labels.get("severity"), Some(&Label::Score(1)));
    }

    #[test]
    fn a_row_may_label_a_subset_of_the_questions() {
        // This is how one dataset carries ground truth for several questions: each is
        // scored over whichever rows label it, rather than demanding every row answer
        // every question a user happened to put in the same file.
        let dataset = parse_all(&row("{\"urgent\":false}")).unwrap();
        assert_eq!(dataset.rows.first().unwrap().labels.len(), 1);
    }

    #[test]
    fn blank_lines_are_skipped() {
        let text = format!("\n{}\n\n", row("{\"urgent\":true}"));
        assert_eq!(parse_all(&text).unwrap().rows.len(), 1);
    }

    #[test]
    fn a_zero_or_one_is_accepted_as_a_noul_label() {
        // A dataset exported from a spreadsheet or a SQL query almost always has these;
        // refusing would make the common path a transformation step.
        assert_eq!(
            parse_all(&row("{\"urgent\":1}"))
                .unwrap()
                .rows
                .first()
                .unwrap()
                .labels
                .get("urgent"),
            Some(&Label::Noul(true))
        );
        assert_eq!(
            parse_all(&row("{\"urgent\":0}"))
                .unwrap()
                .rows
                .first()
                .unwrap()
                .labels
                .get("urgent"),
            Some(&Label::Noul(false))
        );
    }

    #[test]
    fn a_noul_label_that_is_neither_boolean_nor_zero_or_one_is_refused() {
        assert!(message(&row("{\"urgent\":0.5}")).contains("true, false, 0, or 1"));
        assert!(message(&row("{\"urgent\":\"yes\"}")).contains("true, false, 0, or 1"));
        assert!(message(&row("{\"urgent\":2}")).contains("true, false, 0, or 1"));
    }

    #[test]
    fn a_choice_label_must_name_a_declared_option_and_the_message_lists_them() {
        // A misspelled option is the mistake this catches, so the message has to show
        // the spelling the user missed rather than only saying it was wrong.
        let error = message(&row("{\"team\":\"biling\"}"));
        assert!(error.contains("`biling` is not one of"), "{error}");
        assert!(
            error.contains("`billing`") && error.contains("`support`"),
            "{error}"
        );
    }

    #[test]
    fn a_choice_label_is_matched_exactly_and_not_case_folded() {
        // The same exact comparison `--require 'team.choice == billing'` makes. Folding
        // here and not there would mean a dataset validated against one rule and a gate
        // ran under another.
        assert!(message(&row("{\"team\":\"Billing\"}")).contains("is not one of"));
    }

    #[test]
    fn a_score_label_is_a_level_index_inside_the_legend() {
        assert!(parse_all(&row("{\"severity\":0}")).is_ok());
        assert!(parse_all(&row("{\"severity\":1}")).is_ok());
        let error = message(&row("{\"severity\":2}"));
        assert!(error.contains("level index from 0 to 1"), "{error}");
    }

    #[test]
    fn a_score_label_is_not_matched_against_the_level_text() {
        // Fuzzy-matching a ground truth against the prose a user wrote to prompt a model
        // is exactly the kind of guess a calibration tool must not make.
        assert!(message(&row("{\"severity\":\"high\"}")).contains("level index"));
    }

    #[test]
    fn a_label_naming_an_unknown_question_is_refused_not_ignored() {
        // A silently dropped label looks scored and was not, which is the worst possible
        // failure for a tool whose output is a number someone will trust.
        let error = message(&row("{\"urgnet\":true}"));
        assert!(error.contains("`urgnet`"), "{error}");
        assert!(error.contains("q.json"), "{error}");
    }

    #[test]
    fn a_row_without_the_schema_field_is_refused_and_shown_the_shape() {
        // Pointing `--dataset` at a `jev map` output file, or at bare records, is the
        // likeliest wrong thing to do; every other message would describe a missing
        // field rather than the actual mistake.
        let error = message("{\"id\":\"1\",\"state\":\"s\",\"labels\":{\"urgent\":true}}");
        assert!(error.contains("`schema` is missing"), "{error}");
        assert!(error.contains(ROW_SCHEMA), "{error}");
    }

    #[test]
    fn a_row_with_the_wrong_schema_is_refused() {
        let error = message(
            "{\"schema\":\"jev.map.row/v1\",\"id\":\"1\",\"state\":\"s\",\"labels\":{\"urgent\":true}}",
        );
        assert!(error.contains("jev.map.row/v1"), "{error}");
    }

    #[test]
    fn a_typo_in_a_row_field_is_refused_rather_than_ignored() {
        let error = message(&format!(
            "{{\"schema\":\"{ROW_SCHEMA}\",\"id\":\"1\",\"stat\":\"s\",\"labels\":{{\"urgent\":true}}}}"
        ));
        assert!(error.contains("unknown field `stat`"), "{error}");
    }

    #[test]
    fn the_required_fields_are_required() {
        let without = |field: &str| {
            let mut fields = vec![
                format!("\"schema\":\"{ROW_SCHEMA}\""),
                "\"id\":\"1\"".to_owned(),
                "\"state\":\"s\"".to_owned(),
                "\"labels\":{\"urgent\":true}".to_owned(),
            ];
            fields.retain(|entry| !entry.starts_with(&format!("\"{field}\"")));
            message(&format!("{{{}}}", fields.join(",")))
        };
        assert!(without("id").contains("`id` is required"));
        assert!(without("state").contains("`state` is required"));
        assert!(without("labels").contains("`labels` must be an object"));
    }

    #[test]
    fn an_empty_or_control_laden_id_is_refused() {
        assert!(
            message(&format!(
                "{{\"schema\":\"{ROW_SCHEMA}\",\"id\":\"  \",\"state\":\"s\",\"labels\":{{\"urgent\":true}}}}"
            ))
            .contains("non-empty string")
        );
        assert!(
            message(&format!(
                "{{\"schema\":\"{ROW_SCHEMA}\",\"id\":\"a\\u0007b\",\"state\":\"s\",\"labels\":{{\"urgent\":true}}}}"
            ))
            .contains("control character")
        );
    }

    #[test]
    fn an_empty_labels_object_is_refused() {
        assert!(message(&row("{}")).contains("at least one question"));
    }

    #[test]
    fn a_duplicate_row_id_is_refused_because_it_would_split_both_ways() {
        let one = row("{\"urgent\":true}");
        let error = message(&format!("{one}\n{one}"));
        assert!(error.contains("duplicate row id `1`"), "{error}");
        assert!(error.contains("line 1"), "{error}");
    }

    #[test]
    fn an_empty_dataset_is_refused() {
        assert!(
            parse_all("\n\n")
                .unwrap_err()
                .to_string()
                .contains("no labelled rows")
        );
    }

    #[test]
    fn a_line_that_is_not_json_names_the_line() {
        let error = message("not json at all");
        assert!(error.contains("data.jsonl line 1"), "{error}");
    }

    #[test]
    fn a_deeply_nested_state_is_refused_like_any_other_untrusted_input() {
        // A dataset file is untrusted input, and gets the same bounded conversion a
        // request file's `state` does rather than a second, laxer path.
        let deep = format!("{}1{}", "[".repeat(200), "]".repeat(200));
        let line = format!(
            "{{\"schema\":\"{ROW_SCHEMA}\",\"id\":\"1\",\"state\":{deep},\"labels\":{{\"urgent\":true}}}}"
        );
        assert!(parse_all(&line).is_err());
    }

    // --- Fingerprint and split -------------------------------------------------------

    #[test]
    fn the_fingerprint_moves_when_a_label_changes() {
        // The point of recording it: a report whose fingerprint matches was measured on
        // the same examples, and one whose fingerprint differs was not.
        let yes = parse_all(&row("{\"urgent\":true}")).unwrap().fingerprint;
        let no = parse_all(&row("{\"urgent\":false}")).unwrap().fingerprint;
        assert_ne!(yes, no);
    }

    #[test]
    fn the_fingerprint_moves_when_the_state_changes() {
        let one = parse_all(&row("{\"urgent\":true}")).unwrap().fingerprint;
        let other = parse_all(
            &row("{\"urgent\":true}").replace("\"state\":\"s\"", "\"state\":\"different\""),
        )
        .unwrap()
        .fingerprint;
        assert_ne!(one, other);
    }

    #[test]
    fn truncating_a_dataset_re_fingerprints_it() {
        // A fingerprint naming a thousand rows when sixty were measured is a report that
        // cannot be compared with anything.
        let lines: Vec<String> = (0..5)
            .map(|index| {
                row("{\"urgent\":true}").replace("\"id\":\"1\"", &format!("\"id\":\"{index}\""))
            })
            .collect();
        let full = parse_all(&lines.join("\n")).unwrap();
        let before = full.fingerprint.clone();
        let limited = full.truncated(2);
        assert_eq!(limited.rows.len(), 2);
        assert_ne!(limited.fingerprint, before);

        // And a limit above the row count changes nothing.
        let full = parse_all(&lines.join("\n")).unwrap();
        assert_eq!(full.clone().truncated(99).fingerprint, full.fingerprint);
    }

    #[test]
    fn the_split_is_stable_across_runs_and_independent_of_position() {
        // Keyed on the id, so adding rows or re-sorting the file does not reshuffle the
        // rows that were already assigned -- which is what makes two reports comparable.
        for id in ["a", "b", "row-0001", "row-0002", "  spaced  "] {
            assert_eq!(side(id, 0, 0.3), side(id, 0, 0.3));
        }
    }

    #[test]
    fn the_seed_actually_moves_the_split() {
        let ids: Vec<String> = (0..200).map(|index| format!("row-{index}")).collect();
        let with = |seed| {
            ids.iter()
                .filter(|id| side(id, seed, 0.3) == Side::Test)
                .count()
        };
        assert_ne!(with(0), with(1), "two seeds produced identical splits");
    }

    #[test]
    fn the_split_lands_near_the_requested_fraction() {
        // Not exact -- it is a hash, not a shuffle-and-cut -- but a fraction that came
        // out at 5% when 30% was asked for would make the holdout meaningless.
        let ids: Vec<String> = (0..2000).map(|index| format!("row-{index}")).collect();
        for fraction in [0.2, 0.3, 0.5] {
            let held = ids
                .iter()
                .filter(|id| side(id, 7, fraction) == Side::Test)
                .count();
            #[allow(
                clippy::cast_precision_loss,
                reason = "two thousand rows is far below 2^53"
            )]
            let observed = held as f64 / ids.len() as f64;
            assert!(
                (observed - fraction).abs() < 0.05,
                "asked for {fraction}, got {observed}"
            );
        }
    }

    #[test]
    fn a_fraction_of_zero_or_one_holds_out_nothing_or_everything() {
        for id in ["a", "b", "c"] {
            assert_eq!(side(id, 0, 0.0), Side::Calibration);
            assert_eq!(side(id, 0, 1.0), Side::Test);
        }
    }
}
