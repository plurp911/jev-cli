//! The machine contract.
//!
//! # Rules this module exists to enforce
//!
//! * **One document, one line, newline-terminated.** A consumer can read a whole result
//!   with a single `read_line`, and `jev map` can stream JSONL through the same path.
//! * **A `schema` field on everything.** A consumer can branch on it, which turns the
//!   compatibility promise into something detectable rather than something believed.
//! * **Answer objects mirror the API.** A Choice answer here has `choice`,
//!   `probabilities`, and `confidence`, exactly as
//!   <https://docs.typesafe.ai/api> defines them. Users should not have to learn a
//!   second vocabulary, and a CLI that renames the primitives makes the official
//!   documentation actively misleading.
//! * **Uncertainty is never dropped.** The full distribution is always present. A Noul
//!   never grows a `confidence` field, because the API does not return one.
//! * **No colour, ever.** Escape codes in a document a script parses are a bug.

use std::fmt::Write as _;
use std::io::Write;

use jev_core::{Answer, EvaluationResponse, Usage};
use serde_json::{Map, Value, json};

use crate::errors::{CliError, Result};
use crate::gate::GateOutcome;
use crate::render::EVALUATION_SCHEMA;

/// Writes one JSON document, on one line, newline-terminated.
///
/// # Errors
///
/// Returns an internal-class [`CliError`] when the value cannot be serialized, and
/// propagates a write failure other than a broken pipe.
pub(crate) fn write_document(out: &mut dyn Write, value: &Value) -> Result<()> {
    write_all(out, &encode_line(value)?)
}

/// Encodes one document as the bytes of a single newline-terminated line.
///
/// Shared with the `jev map` file sink, which must not write its rows through
/// [`write_document`]: that treats a broken pipe as success, which is right for stdout
/// and wrong for a file, where a write failure has to reach the summary. Sharing the
/// *encoding* rather than the *writing* is what keeps the escaping below applied to
/// both — the file sink previously wrote raw bytes, so a persisted `--output-file`, the
/// artifact most likely to be `cat`-ed or diffed days later, was the one place the
/// hazards survived.
///
/// # Errors
///
/// Returns an internal-class [`CliError`] when the value cannot be serialized.
pub(crate) fn encode_line(value: &Value) -> Result<Vec<u8>> {
    let rendered = serde_json::to_string(value)
        .map_err(|error| CliError::internal(format!("could not encode output: {error}")))?;
    let mut bytes = escape_terminal_hazards(&rendered).into_bytes();
    bytes.push(b'\n');
    Ok(bytes)
}

/// Writes raw bytes, treating a broken pipe as success.
///
/// `jev … | head` closing the pipe is normal Unix behaviour, not a failure
/// (`docs/cli-contract.md`).
pub(crate) fn write_all(out: &mut dyn Write, bytes: &[u8]) -> Result<()> {
    match out.write_all(bytes) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        // A full disk or a read-only filesystem is the user's environment, not a bug
        // in `jev`.
        Err(error) => Err(CliError::io(format!(
            "could not write output: {}",
            error.kind()
        ))),
    }
}

/// Escapes the characters JSON permits raw but a terminal reads as instructions.
///
/// `serde_json` escapes `"`, `\`, and everything below `U+0020` — which is correct
/// JSON and not sufficient here. Every string in these documents can carry text the
/// API returned or the user's own input echoed back, and the characters JSON is happy
/// to pass through are not inert when the line is read: `U+007F`, the bidirectional
/// overrides and isolates that reorder how a line *reads* without changing what it
/// *contains* (Trojan Source), the zero-width characters that hide text outright, and
/// the separators some consumers treat as line terminators — which would split one
/// JSONL record into two.
///
/// The set is [`crate::output::is_terminal_actionable`], the same one the human
/// renderer neutralizes, so there is one definition of the hazard rather than two that
/// can drift.
///
/// This is not a change to the contract. `\uXXXX` parses back to exactly the same
/// string, so a consumer sees the identical value; only the bytes on the wire differ,
/// and `docs/cli-contract.md` already tells consumers not to depend on those. Nothing
/// is dropped or rewritten, which is the difference between this and `sanitize`: a
/// script still receives the character the API sent.
///
/// A scan of the whole document is safe because none of these characters can appear
/// outside a string literal in `serde_json` output.
pub(crate) fn escape_terminal_hazards(rendered: &str) -> String {
    escape_hazards(rendered, crate::output::is_terminal_actionable)
}

/// [`escape_terminal_hazards`], for `serde_json`'s **pretty** rendering.
///
/// Identical but for one exemption: the line feeds `to_string_pretty` puts between
/// fields are left alone, because there they are structure rather than content, and
/// escaping them collapses the document onto one line of `\u000a` — which is exactly
/// what a reader of the pretty form asked not to have.
///
/// The exemption is safe because `serde_json` has already turned every line feed
/// *inside* a string into the two characters `\n` by the time this runs, so a raw
/// `U+000A` in the rendered text cannot have come from the user's data. Every other
/// hazard — DEL, the bidirectional overrides, the zero-width characters, the line and
/// paragraph separators — is escaped exactly as in the one-line form.
pub(crate) fn escape_terminal_hazards_pretty(rendered: &str) -> String {
    escape_hazards(rendered, |character| {
        character != '\n' && crate::output::is_terminal_actionable(character)
    })
}

/// The shared body of the two escapers above.
fn escape_hazards(rendered: &str, hazardous: impl Fn(char) -> bool) -> String {
    if !rendered.chars().any(&hazardous) {
        return rendered.to_owned();
    }
    let mut escaped = String::with_capacity(rendered.len());
    for character in rendered.chars() {
        if !hazardous(character) {
            escaped.push(character);
            continue;
        }
        let code = character as u32;
        if let Some(above_bmp) = code.checked_sub(0x1_0000) {
            // JSON has no escape for a character outside the Basic Multilingual Plane;
            // it is written as the UTF-16 surrogate pair, which every parser rejoins.
            let high = 0xd800 + (above_bmp >> 10);
            let low = 0xdc00 + (above_bmp & 0x3ff);
            let _ = write!(escaped, "\\u{high:04x}\\u{low:04x}");
        } else {
            let _ = write!(escaped, "\\u{code:04x}");
        }
    }
    escaped
}

/// Builds the evaluation document.
///
/// `requested_model` is reported alongside the model that answered: when the request
/// used a moving alias, the two differ, and only the resolved one is meaningful later
/// for a calibrated threshold.
pub(crate) fn evaluation(
    response: &EvaluationResponse,
    requested_model: &str,
    endpoint: &str,
    gate: Option<(&str, &GateOutcome)>,
    request_id: Option<&str>,
    missing: &[String],
) -> Value {
    let mut document = Map::new();
    document.insert("schema".to_owned(), json!(EVALUATION_SCHEMA));
    document.insert("model".to_owned(), json!(response.model.as_str()));
    document.insert("model_requested".to_owned(), json!(requested_model));
    document.insert("endpoint".to_owned(), json!(endpoint));

    let mut answers = Map::new();
    for (id, answer) in &response.answers {
        answers.insert(id.as_str().to_owned(), answer_value(answer));
    }
    document.insert("answers".to_owned(), Value::Object(answers));
    document.insert("usage".to_owned(), usage(&response.usage));
    // The API's own identifier for this call, from `x-typesafe-request-id`. Log it
    // alongside the answer: it is what TypeSafe support can use to find the call, and
    // it cannot be reconstructed afterwards. `null` when the API sent no header.
    document.insert("request_id".to_owned(), json!(request_id));
    // Present only when the API skipped a question. A missing answer read as a negative
    // one is the error this field exists to prevent; stderr says it too, but a program
    // or an agent reading this document never sees stderr.
    if !missing.is_empty() {
        document.insert("missing_answers".to_owned(), json!(missing));
    }

    if let Some((expression, outcome)) = gate {
        document.insert("gate".to_owned(), gate_value(expression, outcome));
    }
    Value::Object(document)
}

/// Renders one answer in the API's own shape.
pub(crate) fn answer_value(answer: &Answer) -> Value {
    match answer {
        Answer::Noul { noul } => json!({"type": "noul", "noul": noul.get()}),
        Answer::Choice {
            choice,
            probabilities,
            confidence,
        } => {
            let mut map = Map::new();
            for entry in probabilities {
                map.insert(entry.key.clone(), json!(entry.probability.get()));
            }
            json!({
                "type": "choice",
                "choice": choice,
                "confidence": confidence.get(),
                "probabilities": Value::Object(map),
            })
        }
        Answer::Score {
            score,
            legend,
            probabilities,
            confidence,
        } => {
            let mut distribution = Map::new();
            for entry in probabilities {
                distribution.insert(entry.key.to_string(), json!(entry.probability.get()));
            }
            let mut legend_map = Map::new();
            for (level, description) in legend {
                legend_map.insert(level.to_string(), description.to_value());
            }
            json!({
                "type": "score",
                "score": score,
                "confidence": confidence.get(),
                "legend": Value::Object(legend_map),
                "probabilities": Value::Object(distribution),
            })
        }
        // A primitive this version does not model. The payload is passed through
        // verbatim so that a script can still see what the API said.
        Answer::Unrecognized { kind, raw } => {
            let mut object = raw.as_object().cloned().unwrap_or_default();
            object.insert("type".to_owned(), json!(kind));
            object.insert("unrecognized".to_owned(), json!(true));
            Value::Object(object)
        }
    }
}

fn usage(usage: &Usage) -> Value {
    json!({
        "input_tokens": usage.input_tokens,
        "output_tokens": usage.output_tokens,
    })
}

fn gate_value(expression: &str, outcome: &GateOutcome) -> Value {
    match outcome {
        GateOutcome::Passed => json!({
            "expression": expression,
            "result": "passed",
            "passed": true,
        }),
        GateOutcome::Failed => json!({
            "expression": expression,
            "result": "failed",
            "passed": false,
        }),
        GateOutcome::Unevaluable { reason } => json!({
            "expression": expression,
            "result": "unevaluable",
            "passed": false,
            "reason": reason,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use jev_core::{Confidence, Content, ModelId, Probability, QuestionId, Weighted};
    use proptest::prelude::*;

    use super::*;

    fn probability(value: f64) -> Probability {
        Probability::new(value).unwrap()
    }

    fn response() -> EvaluationResponse {
        EvaluationResponse {
            model: ModelId::new("jev-1.13.0").unwrap(),
            answers: vec![
                (
                    QuestionId::new("urgent").unwrap(),
                    Answer::Noul {
                        noul: probability(0.92),
                    },
                ),
                (
                    QuestionId::new("team").unwrap(),
                    Answer::Choice {
                        choice: "billing".to_owned(),
                        probabilities: vec![
                            Weighted {
                                key: "billing".to_owned(),
                                probability: probability(0.8),
                            },
                            Weighted {
                                key: "technical".to_owned(),
                                probability: probability(0.2),
                            },
                        ],
                        confidence: Confidence::new(0.75).unwrap(),
                    },
                ),
                (
                    QuestionId::new("severity").unwrap(),
                    Answer::Score {
                        score: 1.3,
                        legend: BTreeMap::from([
                            (0, Content::text("Calm").unwrap()),
                            (1, Content::text("Frustrated").unwrap()),
                        ]),
                        probabilities: vec![
                            Weighted {
                                key: 0,
                                probability: probability(0.3),
                            },
                            Weighted {
                                key: 1,
                                probability: probability(0.7),
                            },
                        ],
                        confidence: Confidence::new(0.54).unwrap(),
                    },
                ),
            ],
            usage: Usage {
                input_tokens: Some(312),
                output_tokens: Some(48),
            },
        }
    }

    #[test]
    fn the_document_matches_the_apis_own_vocabulary() {
        // A user who reads https://docs.typesafe.ai/api must find the same field names
        // here. Renaming a primitive makes the official docs misleading.
        let document = evaluation(
            &response(),
            "jev-latest",
            "https://api.typesafe.ai",
            None,
            None,
            &[],
        );
        assert_eq!(document["schema"], json!("jev.evaluation/v1"));
        assert_eq!(document["model"], json!("jev-1.13.0"));
        assert_eq!(document["model_requested"], json!("jev-latest"));

        assert_eq!(
            document["answers"]["urgent"],
            json!({"type": "noul", "noul": 0.92})
        );
        assert_eq!(
            document["answers"]["team"],
            json!({
                "type": "choice",
                "choice": "billing",
                "confidence": 0.75,
                "probabilities": {"billing": 0.8, "technical": 0.2}
            })
        );
        assert_eq!(document["answers"]["severity"]["type"], json!("score"));
        assert_eq!(document["answers"]["severity"]["score"], json!(1.3));
        assert_eq!(
            document["answers"]["severity"]["legend"],
            json!({"0": "Calm", "1": "Frustrated"})
        );
        assert_eq!(document["usage"]["input_tokens"], json!(312));
    }

    #[test]
    fn a_noul_answer_never_grows_a_confidence_field() {
        // Several community CLIs synthesize one. The API does not return it, and
        // inventing one would give a script a number with no defined meaning.
        let document = evaluation(
            &response(),
            "jev-latest",
            "https://api.typesafe.ai",
            None,
            None,
            &[],
        );
        assert!(document["answers"]["urgent"].get("confidence").is_none());
    }

    #[test]
    fn the_full_distribution_is_always_present() {
        let document = evaluation(
            &response(),
            "jev-latest",
            "https://api.typesafe.ai",
            None,
            None,
            &[],
        );
        assert_eq!(
            document["answers"]["team"]["probabilities"]
                .as_object()
                .map(Map::len),
            Some(2)
        );
        assert_eq!(
            document["answers"]["severity"]["probabilities"],
            json!({"0": 0.3, "1": 0.7})
        );
    }

    #[test]
    fn an_unrecognized_answer_is_passed_through_and_flagged() {
        let response = EvaluationResponse {
            model: ModelId::default(),
            answers: vec![(
                QuestionId::new("future").unwrap(),
                Answer::Unrecognized {
                    kind: "ranking".to_owned(),
                    raw: json!({"type": "ranking", "order": ["a", "b"]}),
                },
            )],
            usage: Usage::default(),
        };
        let document = evaluation(
            &response,
            "jev-latest",
            "https://api.typesafe.ai",
            None,
            None,
            &[],
        );
        assert_eq!(document["answers"]["future"]["type"], json!("ranking"));
        assert_eq!(document["answers"]["future"]["unrecognized"], json!(true));
        assert_eq!(document["answers"]["future"]["order"], json!(["a", "b"]));
    }

    #[test]
    fn a_gate_result_is_reported_with_its_expression() {
        let document = evaluation(
            &response(),
            "jev-latest",
            "https://api.typesafe.ai",
            Some(("urgent.noul > 0.9", &GateOutcome::Passed)),
            None,
            &[],
        );
        assert_eq!(document["gate"]["passed"], json!(true));
        assert_eq!(document["gate"]["result"], json!("passed"));
        assert_eq!(document["gate"]["expression"], json!("urgent.noul > 0.9"));
    }

    #[test]
    fn an_unevaluable_gate_reports_passed_false_and_why() {
        let outcome = GateOutcome::Unevaluable {
            reason: "`typo.noul` does not name anything in the response".to_owned(),
        };
        let document = evaluation(
            &response(),
            "jev-latest",
            "https://api.typesafe.ai",
            Some(("typo.noul > 0.9", &outcome)),
            None,
            &[],
        );
        assert_eq!(document["gate"]["result"], json!("unevaluable"));
        assert_eq!(document["gate"]["passed"], json!(false));
        assert!(document["gate"]["reason"].is_string());
    }

    #[test]
    fn output_is_one_newline_terminated_line() {
        let mut out = Vec::new();
        write_document(&mut out, &json!({"a": 1, "b": "two"})).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        assert_eq!(rendered.lines().count(), 1);
        assert!(rendered.ends_with('\n'));
        assert!(serde_json::from_str::<Value>(&rendered).is_ok());
    }

    #[test]
    fn output_carries_no_escape_sequences() {
        // JSON string escaping already neutralizes a control character, but the
        // property is worth asserting because it is what lets `jq` consumers be safe.
        let response = EvaluationResponse {
            model: ModelId::default(),
            answers: vec![(
                QuestionId::new("team").unwrap(),
                Answer::Choice {
                    choice: "bill\u{1b}[2Jing".to_owned(),
                    probabilities: Vec::new(),
                    confidence: Confidence::new(1.0).unwrap(),
                },
            )],
            usage: Usage::default(),
        };
        let mut out = Vec::new();
        write_document(
            &mut out,
            &evaluation(
                &response,
                "jev-latest",
                "https://api.typesafe.ai",
                None,
                None,
                &[],
            ),
        )
        .unwrap();
        assert!(!out.contains(&0x1b), "a raw escape byte reached stdout");
    }

    proptest! {
        /// The JSONL invariant, over hostile content rather than over one fixture.
        ///
        /// `jev map` writes one row per line, and a consumer reads it with
        /// `read_line`. That only works if no value a row carries -- an option name, a
        /// selected choice, a level description, or an error message straight out of
        /// an API response body -- can ever emit a raw newline, a raw control
        /// character, or an escape byte. A single unescaped newline in one row does not
        /// corrupt that row: it corrupts the *surrounding* rows, by splitting one
        /// record into two lines that neither parse nor align with their neighbours.
        ///
        /// Asserting it at the shared encoder covers every command at once -- both
        /// stdout and the `jev map --output-file` sink, which go through
        /// [`encode_line`] for exactly this reason.
        #[test]
        fn a_document_is_always_exactly_one_line(
            choice in ".{0,40}",
            option in ".{0,40}",
            message in ".{0,40}",
            kind in ".{0,20}",
        ) {
            let answer = Answer::Choice {
                choice: choice.clone(),
                probabilities: vec![Weighted {
                    key: option.clone(),
                    probability: probability(1.0),
                }],
                confidence: Confidence::new(1.0).unwrap(),
            };
            let document = json!({
                "schema": "jev.map.row/v1",
                "answer": answer_value(&answer),
                "error": {"kind": kind, "message": message},
            });

            let mut out = Vec::new();
            write_document(&mut out, &document).unwrap();

            let newlines = out.iter().fold(0_usize, |total, byte| {
                total + usize::from(*byte == b'\n')
            });
            prop_assert_eq!(
                newlines, 1,
                "a row carried a newline of its own, which splits it across lines"
            );
            prop_assert_eq!(out.last(), Some(&b'\n'), "the row is not newline-terminated");

            let body = &out[..out.len() - 1];
            prop_assert!(
                !body.iter().any(|byte| *byte < 0x20 || *byte == 0x7f),
                "a raw control byte reached the row"
            );
            let text = std::str::from_utf8(body).unwrap();
            prop_assert!(
                !text.chars().any(|character| matches!(character,
                    '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}'
                    | '\u{2028}' | '\u{2029}' | '\u{2060}'..='\u{2064}'
                    | '\u{2066}'..='\u{2069}' | '\u{feff}')),
                "a raw bidirectional or zero-width character reached the row"
            );

            // And the row is still a document, not just well-shaped bytes: a consumer
            // that reads the line must be able to parse it and get its values back.
            let parsed: Value = serde_json::from_slice(body).unwrap();
            prop_assert_eq!(&parsed["answer"]["choice"], &json!(choice));
            prop_assert_eq!(&parsed["error"]["message"], &json!(message));
        }
    }

    #[test]
    fn a_hidden_character_is_escaped_without_changing_the_value() {
        // Trojan Source, in a machine document. An option name that *reads* as
        // "approve" while *containing* "deny" must not be able to reach a reviewer's
        // terminal, a log viewer, or a `git diff` looking like the former -- and the
        // script parsing the line must still receive exactly what the API sent, which
        // is why this escapes rather than strips.
        let hostile = "approve\u{202e}yned\u{200b}\u{7f}";
        let mut out = Vec::new();
        write_document(&mut out, &json!({"choice": hostile})).unwrap();

        let text = std::str::from_utf8(&out).unwrap();
        assert!(
            text.contains("\\u202e"),
            "the override was not escaped: {text}"
        );
        assert!(
            text.contains("\\u200b"),
            "the zero-width space was not escaped"
        );
        assert!(text.contains("\\u007f"), "DEL was not escaped");
        assert!(!text.contains('\u{202e}'), "a raw override survived");

        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(
            parsed["choice"],
            json!(hostile),
            "escaping changed the value a consumer parses; it must not"
        );
    }

    #[test]
    fn a_hazard_outside_the_bmp_is_escaped_as_a_surrogate_pair() {
        // The Unicode tag block (U+E0020..U+E007F) is invisible text and is in the
        // hazard set, and it is above the BMP -- so it needs the two-escape form. A
        // single `\u{e0020}` would be a different character entirely, and a parser
        // would hand the consumer the wrong string.
        let hostile = "ok\u{e0041}\u{13430}";
        let mut out = Vec::new();
        write_document(&mut out, &json!({"text": hostile})).unwrap();

        let text = std::str::from_utf8(&out).unwrap();
        assert!(
            text.contains("\\udb40\\udc41"),
            "the tag character was not written as a surrogate pair: {text}"
        );
        assert!(!text.contains('\u{e0041}'), "a raw tag character survived");

        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(
            parsed["text"],
            json!(hostile),
            "the surrogate pair did not round-trip to the original character"
        );
    }

    #[test]
    fn a_broken_pipe_is_not_an_error() {
        /// A writer that behaves like `head` closing the pipe.
        struct ClosedPipe;
        impl Write for ClosedPipe {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert!(write_document(&mut ClosedPipe, &json!({})).is_ok());
    }
}
