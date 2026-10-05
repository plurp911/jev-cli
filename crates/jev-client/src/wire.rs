//! Decoding API responses into validated domain types.
//!
//! # Posture
//!
//! Every byte here is treated as hostile (`docs/threat-model.md` T5, T6). The decoder:
//!
//! * rejects invalid UTF-8 explicitly rather than lossily converting it, so a response
//!   cannot smuggle replacement characters into output that is then trusted;
//! * bounds nesting depth, on top of `serde_json`'s own limit;
//! * refuses out-of-range probabilities and confidences through
//!   [`jev_core::Probability`], so an impossible value cannot exist downstream;
//! * checks that a Score's value lies inside the range its own legend spans;
//! * preserves — rather than drops — an answer whose `type` it does not know, so a
//!   future primitive surfaces instead of vanishing.
//!
//! Decoding is written against `serde_json::Value` rather than `#[derive(Deserialize)]`
//! structs because the failure messages matter: a user needs to know *which* field of
//! *which* answer was wrong, and a derived error says considerably less.

use std::collections::BTreeMap;

use jev_core::limits::MAX_JSON_DEPTH;
use jev_core::{
    Answer, Confidence, Content, EvaluationResponse, ModelCard, ModelId, Probability, QuestionId,
    Usage, Weighted, check_json_depth,
};
use serde_json::Value;

use crate::error::{ClientError, truncate};

/// Decodes a System One response body.
///
/// # Errors
///
/// Returns [`ClientError::MalformedResponse`] naming the offending field.
pub fn decode_evaluation(body: &[u8]) -> Result<EvaluationResponse, ClientError> {
    let root = parse_json(body)?;
    decode_evaluation_root(&root, false)
}

/// Preserves the publisher's scalar and blank Score descriptions.
pub(crate) fn decode_publisher_evaluation(body: &[u8]) -> Result<EvaluationResponse, ClientError> {
    let root = parse_json(body)?;
    decode_evaluation_root(&root, true)
}

fn decode_evaluation_root(
    root: &Value,
    publisher: bool,
) -> Result<EvaluationResponse, ClientError> {
    let object = root
        .as_object()
        .ok_or_else(|| malformed("the response body is not a JSON object"))?;

    let model = object
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed("`model` is missing or not a string"))?;
    let model = ModelId::new(model).map_err(|error| malformed(&format!("`model`: {error}")))?;

    let answers_object = object
        .get("answers")
        .and_then(Value::as_object)
        .ok_or_else(|| malformed("`answers` is missing or not an object"))?;

    let mut answers = Vec::with_capacity(answers_object.len());
    for (id, raw) in answers_object {
        let id = QuestionId::new(id.clone())
            .map_err(|error| malformed(&format!("answer key {id:?}: {error}")))?;
        let answer = decode_answer(id.as_str(), raw, publisher)?;
        answers.push((id, answer));
    }

    let usage = match object.get("usage") {
        Some(Value::Object(map)) => Usage {
            input_tokens: map.get("input_tokens").and_then(Value::as_u64),
            output_tokens: map.get("output_tokens").and_then(Value::as_u64),
        },
        // The official SDK models both counts as optional, so an absent or null
        // `usage` decodes rather than failing the whole response.
        _ => Usage::default(),
    };

    Ok(EvaluationResponse {
        model,
        answers,
        usage,
    })
}

/// Decodes the Workers AI envelope and its required System One result fields.
pub(crate) fn decode_cloudflare_evaluation(body: &[u8]) -> Result<EvaluationResponse, ClientError> {
    let root = parse_json(body)?;
    let result = cloudflare_result(&root)?;
    let answers = result
        .get("answers")
        .and_then(Value::as_object)
        .ok_or_else(|| malformed("Cloudflare result.answers is missing or invalid"))?;
    if answers.is_empty() {
        return Err(malformed("Cloudflare result.answers is empty"));
    }
    let usage = result
        .get("usage")
        .and_then(Value::as_object)
        .ok_or_else(|| malformed("Cloudflare result.usage is missing or invalid"))?;
    if ["input_tokens", "output_tokens"]
        .iter()
        .any(|name| usage.get(*name).and_then(Value::as_u64).is_none())
    {
        return Err(malformed("Cloudflare usage counts are missing or invalid"));
    }
    decode_evaluation_root(result, false)
}

fn cloudflare_result(root: &Value) -> Result<&Value, ClientError> {
    match root.get("success").and_then(Value::as_bool) {
        Some(true) => root
            .get("result")
            .ok_or_else(|| malformed("Cloudflare result is missing")),
        Some(false) => Err(ClientError::from_status(
            400,
            Some("Cloudflare rejected the request".to_owned()),
        )),
        None => Err(malformed("Cloudflare success is missing or not a boolean")),
    }
}

/// The search API intentionally leaves model entries untyped. Expose only the two
/// supported models documented by Cloudflare, without inventing release dates.
pub(crate) fn decode_cloudflare_models(body: &[u8]) -> Result<Vec<ModelCard>, ClientError> {
    let root = parse_json(body)?;
    cloudflare_result(&root)?
        .as_array()
        .ok_or_else(|| malformed("Cloudflare model result is not an array"))?;
    Ok(vec![
        ModelCard {
            name: "clef".to_owned(),
            description: "Cloudflare Clef 27B multimodal decision model".to_owned(),
            release_date: String::new(),
        },
        ModelCard {
            name: "clef-flash".to_owned(),
            description: "Cloudflare Clef Flash 9B multimodal decision model".to_owned(),
            release_date: String::new(),
        },
    ])
}

/// Normalizes the documented local model list without treating modification or
/// creation timestamps as release dates.
pub(crate) fn decode_local_models(
    body: &[u8],
    ollama: bool,
) -> Result<Vec<ModelCard>, ClientError> {
    let root = parse_json(body)?;
    let (list_key, id_key, description) = if ollama {
        ("models", "name", "Installed Ollama model")
    } else {
        ("data", "id", "Loaded llama.cpp model")
    };
    let list = root
        .get(list_key)
        .and_then(Value::as_array)
        .ok_or_else(|| malformed("local model list is missing or invalid"))?;
    list.iter()
        .map(|entry| {
            let name = entry
                .get(id_key)
                .and_then(Value::as_str)
                .ok_or_else(|| malformed("local model id is missing or invalid"))?;
            Ok(ModelCard {
                name: clip(name),
                description: description.to_owned(),
                release_date: String::new(),
            })
        })
        .collect()
}

pub(crate) fn cloudflare_error_code(body: &[u8]) -> Option<u64> {
    let root = parse_json(body).ok()?;
    root.get("errors")?
        .as_array()?
        .first()?
        .get("code")?
        .as_u64()
}

/// Decodes a `GET /v1/models` body.
///
/// # Errors
///
/// Returns [`ClientError::MalformedResponse`] if the document is not the documented
/// `{"models": [...]}` shape.
pub fn decode_models(body: &[u8]) -> Result<Vec<ModelCard>, ClientError> {
    let root = parse_json(body)?;
    let list = root
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| malformed("`models` is missing or not an array"))?;

    list.iter()
        .enumerate()
        .map(|(index, entry)| {
            let field = |name: &str| {
                entry
                    .get(name)
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        malformed(&format!(
                            "models[{index}].{name} is missing or not a string"
                        ))
                    })
                    .map(str::to_owned)
            };
            // Bounded on the way in. These strings are rendered in a column layout
            // whose width is the longest name, so an unbounded name from a hostile or
            // malfunctioning endpoint would size an allocation.
            Ok(ModelCard {
                name: clip(&field("name")?),
                description: clip(&field("description")?),
                release_date: clip(&field("release_date")?),
            })
        })
        .collect()
}

/// Extracts a human-readable message from an API error body.
///
/// Mirrors the official SDK's `extract_message`: it tries `error`, `error.message`,
/// `message`, `detail` as a string or an object with a `message` or an `error_type`,
/// and finally `FastAPI`'s
/// `detail: [{loc, msg}]` validation-error array, which is what a 422 from the
/// System One endpoint actually carries. Falls back to the raw text. Always truncated.
#[must_use]
pub fn extract_error_message(body: &[u8]) -> Option<String> {
    extract_error_text(body).map(|text| truncate(&text))
}

/// [`extract_error_message`] before truncation, and with JSON escapes already decoded.
///
/// The client redacts the credential from this and only then clips it: redacting the
/// clipped message could miss a key cut in half at the limit, and redacting the raw body
/// could miss one the endpoint wrote with JSON escapes.
pub(crate) fn extract_error_text(body: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(body).ok()?.trim();
    if text.is_empty() {
        return None;
    }
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Some(text.to_owned());
    };
    let extracted = match &value {
        Value::String(message) => Some(message.clone()),
        Value::Object(map) => object_message(map),
        _ => None,
    };
    Some(extracted.unwrap_or_else(|| text.to_owned()))
}

fn object_message(map: &serde_json::Map<String, Value>) -> Option<String> {
    if let Some(Value::String(error)) = map.get("error") {
        return Some(error.clone());
    }
    if let Some(Value::String(message)) = map.get("error").and_then(|e| e.get("message")) {
        return Some(message.clone());
    }
    if let Some(Value::String(message)) = map.get("message") {
        return Some(message.clone());
    }
    match map.get("detail") {
        Some(Value::String(detail)) => Some(detail.clone()),
        Some(Value::Object(detail)) => detail
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                detail
                    .get("error_type")
                    .and_then(Value::as_str)
                    .map(describe_error_type)
            }),
        Some(Value::Array(entries)) => {
            let parts: Vec<String> = entries
                .iter()
                .filter_map(|entry| {
                    let message = entry.get("msg").and_then(Value::as_str)?;
                    let path = entry
                        .get("loc")
                        .and_then(Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter(|item| item.as_str() != Some("body"))
                                .map(render_loc_segment)
                                .collect::<Vec<_>>()
                                .join(".")
                        })
                        .unwrap_or_default();
                    Some(if path.is_empty() {
                        message.to_owned()
                    } else {
                        format!("{path}: {message}")
                    })
                })
                .collect();
            (!parts.is_empty()).then(|| parts.join("; "))
        }
        _ => None,
    }
}

/// Renders a `detail.error_type` code, adding an explanation for codes that have been
/// seen from the official API.
///
/// Not from the documentation: `max_tokens_exceeded` was observed live on 2026-09-23 as
/// HTTP 400 `{"detail":{"error_type":"max_tokens_exceeded"}}`, with no `message`. An
/// unrecognised code is reported as itself, which is still better than raw JSON.
fn describe_error_type(error_type: &str) -> String {
    match error_type {
        "max_tokens_exceeded" => {
            "max_tokens_exceeded: the request is over the model's token limit".to_owned()
        }
        other => other.to_owned(),
    }
}

fn render_loc_segment(item: &Value) -> String {
    match item {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn parse_json(body: &[u8]) -> Result<Value, ClientError> {
    // Explicit rather than lossy: a response that is not valid UTF-8 is not a response
    // this client understands, and replacing the bad bytes would hide that.
    let text =
        std::str::from_utf8(body).map_err(|_| malformed("the response body is not valid UTF-8"))?;
    let value: Value = serde_json::from_str(text)
        .map_err(|error| malformed(&format!("the response body is not valid JSON: {error}")))?;
    check_json_depth(&value, MAX_JSON_DEPTH)
        .map_err(|_| malformed("the response body nests deeper than this client will decode"))?;
    Ok(value)
}

/// Bounds an API-supplied display string.
///
/// `ModelId` is length-checked by its own constructor; these three fields are not, and
/// a 16 MiB `name` would become the padding width of every row `jev models` prints.
fn clip(text: &str) -> String {
    const MAX_MODEL_FIELD_CHARS: usize = 512;
    let mut out: String = text.chars().take(MAX_MODEL_FIELD_CHARS).collect();
    if out.chars().count() < text.chars().count() {
        out.push('…');
    }
    out
}

fn malformed(reason: &str) -> ClientError {
    ClientError::MalformedResponse {
        reason: truncate(reason),
    }
}

fn decode_answer(id: &str, raw: &Value, publisher: bool) -> Result<Answer, ClientError> {
    let object = raw
        .as_object()
        .ok_or_else(|| malformed(&format!("answers.{id} is not an object")))?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed(&format!("answers.{id}.type is missing or not a string")))?;

    match kind {
        "noul" => {
            let noul = probability_field(id, object, "noul")?;
            Ok(Answer::Noul { noul })
        }
        "choice" => {
            let choice = object
                .get("choice")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    malformed(&format!("answers.{id}.choice is missing or not a string"))
                })?
                .to_owned();
            let confidence = confidence_field(id, object)?;
            let probabilities = decode_string_distribution(id, object)?;
            // The API documents `choice` as the highest-probability option, so it must
            // be one of the options in the distribution. A response where it is not is
            // internally inconsistent and must not be reported as an answer.
            if !probabilities.is_empty() && !probabilities.iter().any(|entry| entry.key == choice) {
                return Err(malformed(&format!(
                    "answers.{id}.choice is not present in its own probabilities"
                )));
            }
            Ok(Answer::Choice {
                choice,
                probabilities,
                confidence,
            })
        }
        "score" => {
            let score = object.get("score").and_then(Value::as_f64).ok_or_else(|| {
                malformed(&format!("answers.{id}.score is missing or not a number"))
            })?;
            if !score.is_finite() {
                return Err(malformed(&format!("answers.{id}.score is not finite")));
            }
            let legend = decode_legend(id, object, publisher)?;
            let probabilities = decode_level_distribution(id, object)?;
            // A score is the probability-weighted mean of the level numbers, so it
            // cannot fall outside the range those numbers span. Both ends matter, and
            // both the legend and the distribution contribute levels: taking only the
            // legend's maximum rejected a self-consistent response whose distribution
            // mentioned a higher level, and assuming a lower bound of zero accepted a
            // score below a scale that does not start at zero.
            let levels = legend
                .keys()
                .copied()
                .chain(probabilities.iter().map(|entry| entry.key));
            let (lowest, highest) = levels.fold((u32::MAX, 0_u32), |(low, high), level| {
                (low.min(level), high.max(level))
            });
            let (lowest, highest) = if lowest == u32::MAX {
                (0, 0)
            } else {
                (lowest, highest)
            };
            if score < f64::from(lowest) || score > f64::from(highest) {
                return Err(malformed(&format!(
                    "answers.{id}.score is outside the range its levels span"
                )));
            }
            let confidence = confidence_field(id, object)?;
            Ok(Answer::Score {
                score,
                legend,
                probabilities,
                confidence,
            })
        }
        // Forward compatibility: report the unknown type rather than failing the whole
        // response or silently dropping the answer.
        other => Ok(Answer::Unrecognized {
            kind: truncate(other),
            raw: raw.clone(),
        }),
    }
}

fn probability_field(
    id: &str,
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<Probability, ClientError> {
    let raw = object
        .get(field)
        .and_then(Value::as_f64)
        .ok_or_else(|| malformed(&format!("answers.{id}.{field} is missing or not a number")))?;
    Probability::new(raw).map_err(|error| malformed(&format!("answers.{id}.{field}: {error}")))
}

fn confidence_field(
    id: &str,
    object: &serde_json::Map<String, Value>,
) -> Result<Confidence, ClientError> {
    let raw = object
        .get("confidence")
        .and_then(Value::as_f64)
        .ok_or_else(|| {
            malformed(&format!(
                "answers.{id}.confidence is missing or not a number"
            ))
        })?;
    Confidence::new(raw).map_err(|error| malformed(&format!("answers.{id}.confidence: {error}")))
}

fn decode_string_distribution(
    id: &str,
    object: &serde_json::Map<String, Value>,
) -> Result<Vec<Weighted<String>>, ClientError> {
    let map = object
        .get("probabilities")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            malformed(&format!(
                "answers.{id}.probabilities is missing or not an object"
            ))
        })?;
    map.iter()
        .map(|(key, value)| {
            let raw = value.as_f64().ok_or_else(|| {
                malformed(&format!("answers.{id}.probabilities.{key} is not a number"))
            })?;
            Ok(Weighted {
                key: key.clone(),
                probability: Probability::new(raw).map_err(|error| {
                    malformed(&format!("answers.{id}.probabilities.{key}: {error}"))
                })?,
            })
        })
        .collect()
}

fn decode_level_distribution(
    id: &str,
    object: &serde_json::Map<String, Value>,
) -> Result<Vec<Weighted<u32>>, ClientError> {
    let map = object
        .get("probabilities")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            malformed(&format!(
                "answers.{id}.probabilities is missing or not an object"
            ))
        })?;
    let mut entries: Vec<Weighted<u32>> = map
        .iter()
        .map(|(key, value)| {
            // Score distributions are keyed by the level number as a JSON string.
            let level: u32 = key.parse().map_err(|_| {
                malformed(&format!(
                    "answers.{id}.probabilities has a non-numeric level key {key:?}"
                ))
            })?;
            let raw = value.as_f64().ok_or_else(|| {
                malformed(&format!("answers.{id}.probabilities.{key} is not a number"))
            })?;
            Ok(Weighted {
                key: level,
                probability: Probability::new(raw).map_err(|error| {
                    malformed(&format!("answers.{id}.probabilities.{key}: {error}"))
                })?,
            })
        })
        .collect::<Result<Vec<Weighted<u32>>, ClientError>>()?;
    // Level order is the scale's order, and a JSON object's key order is arbitrary.
    entries.sort_by_key(|entry| entry.key);
    Ok(entries)
}

fn decode_legend(
    id: &str,
    object: &serde_json::Map<String, Value>,
    publisher: bool,
) -> Result<BTreeMap<u32, Content>, ClientError> {
    let map = object
        .get("legend")
        .and_then(Value::as_object)
        .ok_or_else(|| malformed(&format!("answers.{id}.legend is missing or not an object")))?;
    map.iter()
        .map(|(key, value)| {
            let level: u32 = key.parse().map_err(|_| {
                malformed(&format!(
                    "answers.{id}.legend has a non-numeric level key {key:?}"
                ))
            })?;
            let content = if publisher {
                Content::local_json(value.clone())
            } else {
                Content::try_from(value.clone())
            }
            .map_err(|error| malformed(&format!("answers.{id}.legend.{key}: {error}")))?;
            Ok((level, content))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jev_core::limits::{MAX_ERROR_BODY_CHARS, MAX_JSON_DEPTH};

    #[test]
    fn scalar_and_blank_score_legends_are_only_decoded_for_the_publisher_bridge() {
        for value in [
            serde_json::json!(null),
            serde_json::json!(false),
            serde_json::json!(1.5),
            serde_json::json!(""),
            serde_json::json!("   "),
        ] {
            let body = serde_json::json!({"model":"clef","answers":{"q":{"type":"score","score":0.5,"legend":{"0":value,"1":"other"},"probabilities":{"0":0.5,"1":0.5},"confidence":0.5}},"usage":{}});
            let bytes = serde_json::to_vec(&body).unwrap();
            assert!(decode_evaluation(&bytes).is_err());
            let response = decode_publisher_evaluation(&bytes).unwrap();
            let Some(Answer::Score { legend, .. }) = response.answer("q") else {
                panic!("expected score");
            };
            assert_eq!(legend[&0].to_value(), value);
            let envelope =
                serde_json::to_vec(&serde_json::json!({"success":true,"result":body})).unwrap();
            assert!(decode_cloudflare_evaluation(&envelope).is_err());
        }
    }

    #[test]
    fn cloudflare_envelopes_require_success_and_valid_inner_fields() {
        for body in [br#"{"success":false,"errors":[{"code":3040}],"result":{}}"#.as_slice(),br#"{"result":{}}"#,br#"{"success":true,"result":{"model":"clef","answers":{},"usage":{"input_tokens":1,"output_tokens":0}}}"#,br#"{"success":true,"result":{"model":"clef","answers":{"q":{"type":"noul","noul":1.1}},"usage":{"input_tokens":1,"output_tokens":0}}}"#] {
            assert!(decode_cloudflare_evaluation(body).is_err());
        }
        let deep = format!(
            "{{\"success\":true,\"result\":{}0{}}}",
            "[".repeat(65),
            "]".repeat(65)
        );
        assert!(decode_cloudflare_evaluation(deep.as_bytes()).is_err());
        assert!(decode_cloudflare_evaluation(b"\xff").is_err());
    }

    fn evaluation(body: &str) -> Result<EvaluationResponse, ClientError> {
        decode_evaluation(body.as_bytes())
    }

    fn reason(error: &ClientError) -> String {
        error.to_string()
    }

    // --- The regression the `api_response` fuzz target found. ---------------------

    /// A Score whose distribution mentions a level the legend does not.
    ///
    /// The levels a score may span come from the legend *and* the distribution, not
    /// from the legend alone. A response like this one is self-consistent — 1.6 is a
    /// plausible weighted mean over levels {1, 2, 8, 880} — and must decode.
    #[test]
    fn a_score_may_span_levels_only_the_distribution_mentions() {
        let response = evaluation(
            r#"{
                "message": "ok",
                "model": "jev-1.13.0",
                "answers": {
                    "frustration": {
                        "type": "score",
                        "score": 1.6,
                        "legend": { "1": "Calm" },
                        "probabilities": { "2": 0.65, "8": 0.3, "880": 0.05 },
                        "confidence": 0.78
                    }
                },
                "usage": {}
            }"#,
        )
        .expect("a score inside the range its levels span must decode");
        let answer = response
            .answer("frustration")
            .expect("the answer is present");
        assert!(matches!(answer, Answer::Score { .. }), "{answer:?}");
    }

    /// The other end of the same rule: a scale that does not start at zero.
    #[test]
    fn a_score_below_the_lowest_level_is_refused() {
        let error = evaluation(
            r#"{
                "message": "ok", "model": "m",
                "answers": { "q": {
                    "type": "score", "score": 0.5,
                    "legend": { "1": "a", "2": "b" },
                    "probabilities": { "1": 0.5, "2": 0.5 },
                    "confidence": 0.9
                } },
                "usage": {}
            }"#,
        )
        .expect_err("0.5 is below a scale that starts at 1");
        assert!(reason(&error).contains("outside the range"), "{error}");
    }

    // --- `decode_models` error branches. ------------------------------------------

    #[test]
    fn models_must_be_an_array() {
        for body in ["{}", r#"{"models": null}"#, r#"{"models": {}}"#] {
            let error = decode_models(body.as_bytes()).expect_err(body);
            assert!(
                reason(&error).contains("`models` is missing or not an array"),
                "{body}: {error}"
            );
        }
    }

    #[test]
    fn a_model_row_names_the_field_and_index_that_are_wrong() {
        let error = decode_models(
            br#"{"models": [
                {"name": "a", "description": "d", "release_date": "2026-01-01"},
                {"name": "b", "description": "d"}
            ]}"#,
        )
        .expect_err("release_date is missing from the second row");
        let message = reason(&error);
        assert!(message.contains("models[1].release_date"), "{message}");
    }

    #[test]
    fn a_non_string_model_field_is_not_coerced() {
        let error =
            decode_models(br#"{"models": [{"name": 7, "description": "d", "release_date": "x"}]}"#)
                .expect_err("a numeric name is not a name");
        assert!(reason(&error).contains("models[0].name"), "{error}");
    }

    #[test]
    fn model_fields_are_bounded_on_the_way_in() {
        let huge = "n".repeat(10_000);
        let body = format!(
            r#"{{"models": [{{"name": "{huge}", "description": "d", "release_date": "x"}}]}}"#
        );
        let models = decode_models(body.as_bytes()).expect("a long name is not an error");
        let name = models.first().expect("one row").name.clone();
        assert!(
            name.chars().count() < 10_000,
            "a hostile endpoint sized an allocation: {} chars",
            name.chars().count()
        );
    }

    #[test]
    fn an_empty_model_list_is_a_valid_answer() {
        let models = decode_models(br#"{"models": []}"#).expect("an empty list is not malformed");
        assert!(models.is_empty());
    }

    // --- `extract_error_message`: every shape the ladder tries. -------------------

    #[test]
    fn every_documented_error_shape_yields_its_message() {
        let cases: &[(&str, &str)] = &[
            (r#""just a string""#, "just a string"),
            (r#"{"error": "flat"}"#, "flat"),
            (r#"{"error": {"message": "nested"}}"#, "nested"),
            (r#"{"message": "top level"}"#, "top level"),
            (r#"{"detail": "detail string"}"#, "detail string"),
            (
                r#"{"detail": {"message": "detail object"}}"#,
                "detail object",
            ),
        ];
        for (body, expected) in cases {
            assert_eq!(
                extract_error_message(body.as_bytes()).as_deref(),
                Some(*expected),
                "{body}"
            );
        }
    }

    /// The shape a 422 from the System One endpoint actually carries.
    #[test]
    fn a_validation_error_array_names_the_field_that_was_rejected() {
        let message = extract_error_message(
            br#"{"detail": [
                {"loc": ["body", "questions", 0, "criteria"], "msg": "too few levels"},
                {"loc": ["body", "state"], "msg": "field required"}
            ]}"#,
        )
        .expect("a detail array yields a message");
        assert_eq!(
            message,
            "questions.0.criteria: too few levels; state: field required"
        );
    }

    /// The official API's token-limit rejection: HTTP 400 with a `detail` object that
    /// carries an `error_type` and no `message`.
    ///
    /// Observed live on 2026-09-23 (with 93 KB and 312 KB of state), not documented.
    /// Until this case existed the ladder found no message and `jev` printed the raw
    /// JSON.
    #[test]
    fn a_detail_object_with_an_error_type_yields_a_readable_message() {
        let message = extract_error_message(br#"{"detail":{"error_type":"max_tokens_exceeded"}}"#)
            .expect("a detail error_type yields a message");
        assert_eq!(
            message,
            "max_tokens_exceeded: the request is over the model's token limit"
        );
    }

    #[test]
    fn an_unknown_detail_error_type_is_reported_as_itself() {
        assert_eq!(
            extract_error_message(br#"{"detail":{"error_type":"something_new"}}"#).as_deref(),
            Some("something_new")
        );
        // A message, when present, still wins: it is the API's own explanation.
        assert_eq!(
            extract_error_message(br#"{"detail":{"error_type":"x","message":"explained"}}"#)
                .as_deref(),
            Some("explained")
        );
    }

    #[test]
    fn a_long_detail_error_type_is_still_truncated() {
        let body = format!(
            r#"{{"detail":{{"error_type":"{}"}}}}"#,
            "e".repeat(MAX_ERROR_BODY_CHARS * 4)
        );
        let message = extract_error_message(body.as_bytes()).expect("a message");
        assert_eq!(message.chars().count(), MAX_ERROR_BODY_CHARS + 1);
        assert!(message.ends_with('…'));
    }

    #[test]
    fn a_validation_entry_without_a_location_still_reports_its_message() {
        let message = extract_error_message(br#"{"detail": [{"msg": "bare"}]}"#)
            .expect("a msg with no loc is still a message");
        assert_eq!(message, "bare");
    }

    #[test]
    fn an_unrecognised_body_falls_back_to_its_own_text() {
        assert_eq!(
            extract_error_message(b"<html>502 Bad Gateway</html>").as_deref(),
            Some("<html>502 Bad Gateway</html>")
        );
        // Valid JSON, but no message anywhere in the ladder.
        assert_eq!(
            extract_error_message(br#"{"code": 500}"#).as_deref(),
            Some(r#"{"code": 500}"#)
        );
    }

    #[test]
    fn an_empty_or_non_utf8_body_yields_nothing() {
        assert_eq!(extract_error_message(b""), None);
        assert_eq!(extract_error_message(b"   \n  "), None);
        assert_eq!(extract_error_message(&[0xff, 0xfe, 0x00]), None);
    }

    /// A hostile endpoint must not be able to use an error message as an unbounded
    /// output channel, whichever branch of the ladder produced it.
    #[test]
    fn every_branch_is_truncated() {
        let huge = "x".repeat(MAX_ERROR_BODY_CHARS * 20);
        let bodies = [
            format!(r#"{{"error": "{huge}"}}"#),
            format!(r#"{{"detail": "{huge}"}}"#),
            format!(r#"{{"detail": [{{"msg": "{huge}"}}]}}"#),
            huge.clone(),
        ];
        for body in &bodies {
            let message = extract_error_message(body.as_bytes()).expect("a message");
            assert!(
                message.chars().count() <= MAX_ERROR_BODY_CHARS + 1,
                "{} chars survived truncation",
                message.chars().count()
            );
        }
    }

    // --- `parse_json` rejects rather than lossily decoding. -----------------------

    #[test]
    fn a_non_utf8_body_is_refused_rather_than_replaced() {
        let error = decode_evaluation(&[0x7b, 0xff, 0x7d]).expect_err("invalid UTF-8");
        assert!(reason(&error).contains("not valid UTF-8"), "{error}");
    }

    /// Between `MAX_JSON_DEPTH` and `serde_json`'s own recursion limit, our check is
    /// the only thing that refuses the body — which is exactly the band a hostile
    /// endpoint would aim for, so it is the band worth testing.
    #[test]
    fn a_body_nested_past_our_own_limit_is_refused() {
        let depth = MAX_JSON_DEPTH + 8;
        assert!(depth < 128, "serde_json's limit would mask ours");
        let body = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
        let error = decode_evaluation(body.as_bytes()).expect_err("too deep");
        assert!(reason(&error).contains("nest"), "{error}");
    }

    /// And a body exactly at the limit passes the depth check, so the limit is a
    /// boundary rather than a blanket refusal. `n` nested arrays around a scalar nest
    /// `n + 1` levels deep, because the scalar is itself a level.
    #[test]
    fn a_body_at_our_limit_passes_the_depth_check() {
        let brackets = MAX_JSON_DEPTH - 1;
        let body = format!("{}1{}", "[".repeat(brackets), "]".repeat(brackets));
        let error = decode_evaluation(body.as_bytes()).expect_err("an array is not a response");
        assert!(
            !reason(&error).contains("nest"),
            "rejected at the limit: {error}"
        );
    }
}
