//! Compatibility tests against documents recorded from official provider sources.
//!
//! # Why these exist separately from the unit tests
//!
//! A unit test asserts that the code does what its author intended. These assert that
//! what the author intended matches what the provider published. When the API changes, a
//! unit test written against the old shape keeps passing; one of these fails.
//!
//! Provenance for every fixture is in `tests/fixtures/README.md`, and each file names
//! its source in a `_source` field. A failure here is a compatibility signal, not a
//! test to adjust.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "a panicking assertion is the correct failure mode inside a test binary"
)]

use jev_core::{
    Answer, ChoiceOption, Content, EvaluationRequest, ModelId, NoulCriteria, Question, QuestionId,
    State,
};
use serde_json::{Value, json};

#[test]
fn documented_ollama_clef_response_preserves_each_primitive_and_confidence() {
    let value = fixture("response-ollama-clef.json");
    let response = jev_client::decode_evaluation(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(response.model.as_str(), "clef");
    assert_eq!(response.usage.input_tokens, Some(1204));
    assert_eq!(response.usage.output_tokens, Some(3));
    let team = response
        .answers
        .iter()
        .find(|(id, _)| id.as_str() == "team")
        .unwrap();
    match &team.1 {
        Answer::Choice {
            choice,
            confidence,
            probabilities,
        } => {
            assert_eq!(choice, "billing");
            assert_eq!(confidence.get().to_bits(), 0.924_f64.to_bits());
            assert_eq!(
                probabilities
                    .iter()
                    .find(|p| p.key == "billing")
                    .unwrap()
                    .probability
                    .get()
                    .to_bits(),
                0.981_f64.to_bits()
            );
        }
        other => panic!("unexpected answer {other:?}"),
    }
    assert!(
        response
            .answers
            .iter()
            .any(|(id, answer)| id.as_str() == "refund"
                && matches!(answer,Answer::Noul { noul } if noul.get().to_bits()==0.996_f64.to_bits()))
    );
    let urgency = response
        .answers
        .iter()
        .find(|(id, _)| id.as_str() == "urgency")
        .unwrap();
    match &urgency.1 {
        Answer::Score {
            score,
            confidence,
            legend,
            ..
        } => {
            assert_eq!(score.to_bits(), 0.704_f64.to_bits());
            assert_eq!(confidence.get().to_bits(), 0.071_f64.to_bits());
            assert_eq!(legend.get(&0), Some(&text("Routine")));
        }
        other => panic!("unexpected answer {other:?}"),
    }
}

/// Loads a fixture and strips the provenance field, which is ours and not the API's.
fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let mut value: Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is not valid JSON: {error}", path.display()));
    if let Some(object) = value.as_object_mut() {
        assert!(
            object.remove("_source").is_some(),
            "{} has no `_source`; every fixture must record where it came from",
            path.display()
        );
    }
    value
}

fn text(value: &str) -> Content {
    Content::text(value).unwrap()
}

fn id(value: &str) -> QuestionId {
    QuestionId::new(value).unwrap()
}

// --- Requests: what `jev` builds must equal what the documentation shows -------------

#[test]
fn a_noul_request_encodes_to_the_documented_body() {
    let request = EvaluationRequest::new(
        State::text("Help! My payouts have been failing for 3 days.").unwrap(),
        ModelId::default(),
        vec![(
            id("is_urgent"),
            Question::noul(
                text("Does this convey urgency?"),
                Some(
                    NoulCriteria::new(
                        Some(text("Explicitly time-sensitive")),
                        Some(text("No urgency expressed")),
                    )
                    .unwrap(),
                ),
            )
            .unwrap(),
        )],
    )
    .unwrap();

    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        fixture("request-noul.json")
    );
}

#[test]
fn a_choice_request_encodes_to_the_documented_body() {
    let request = EvaluationRequest::new(
        State::text("Help! My payouts have been failing for 3 days.").unwrap(),
        ModelId::default(),
        vec![(
            id("department"),
            Question::choice(
                text("Which team should handle this?"),
                vec![
                    ChoiceOption::new("billing", Some(text("Payments, invoicing, refunds")))
                        .unwrap(),
                    ChoiceOption::new("technical", Some(text("Bugs, outages, integrations")))
                        .unwrap(),
                    ChoiceOption::new("sales", Some(text("Pricing, upgrades, new accounts")))
                        .unwrap(),
                ],
            )
            .unwrap(),
        )],
    )
    .unwrap();

    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        fixture("request-choice.json")
    );
}

#[test]
fn a_score_request_encodes_to_the_documented_body() {
    let request = EvaluationRequest::new(
        State::text("Help! My payouts have been failing for 3 days.").unwrap(),
        ModelId::default(),
        vec![(
            id("frustration"),
            Question::score(
                text("How frustrated is the customer?"),
                vec![text("Calm"), text("Frustrated"), text("Very angry")],
            )
            .unwrap(),
        )],
    )
    .unwrap();

    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        fixture("request-score.json")
    );
}

// --- Responses: every documented body must decode, with nothing lost ------------------

#[test]
fn the_documented_noul_response_decodes() {
    let body = fixture("response-noul.json").to_string();
    let response = jev_client::decode_evaluation(body.as_bytes()).unwrap();

    assert_eq!(response.model.as_str(), "jev-1.13.0");
    assert_eq!(response.usage.input_tokens, Some(307));
    assert_eq!(response.usage.output_tokens, Some(20));
    match response.answer("is_urgent").unwrap() {
        Answer::Noul { noul } => assert!((noul.get() - 0.95).abs() < 1e-12),
        other => panic!("expected a noul answer, got {other:?}"),
    }
    // And no confidence was invented for it.
    assert_eq!(response.answer("is_urgent").unwrap().confidence(), None);
}

#[test]
fn the_documented_choice_response_decodes_with_its_whole_distribution() {
    let body = fixture("response-choice.json").to_string();
    let response = jev_client::decode_evaluation(body.as_bytes()).unwrap();

    match response.answer("department").unwrap() {
        Answer::Choice {
            choice,
            probabilities,
            confidence,
        } => {
            assert_eq!(choice, "billing");
            assert!((confidence.get() - 0.81).abs() < 1e-12);
            assert_eq!(probabilities.len(), 3);
            let selected = probabilities
                .iter()
                .find(|entry| entry.key == "billing")
                .unwrap();
            assert!((selected.probability.get() - 0.88).abs() < 1e-12);
            // The distribution sums to one, which is what makes confidence meaningful.
            let total: f64 = probabilities
                .iter()
                .map(|entry| entry.probability.get())
                .sum();
            assert!((total - 1.0).abs() < 1e-9, "probabilities sum to {total}");
        }
        other => panic!("expected a choice answer, got {other:?}"),
    }
}

#[test]
fn the_documented_score_response_decodes_with_its_legend() {
    let body = fixture("response-score.json").to_string();
    let response = jev_client::decode_evaluation(body.as_bytes()).unwrap();

    match response.answer("frustration").unwrap() {
        Answer::Score {
            score,
            legend,
            probabilities,
            confidence,
        } => {
            assert!((score - 1.05).abs() < 1e-12);
            assert!((confidence.get() - 0.92).abs() < 1e-12);
            assert_eq!(legend.len(), 3);
            assert_eq!(legend[&2].as_text(), Some("Very angry"));
            // Levels come back in scale order, whatever order the JSON object used.
            let levels: Vec<u32> = probabilities.iter().map(|entry| entry.key).collect();
            assert_eq!(levels, vec![0, 1, 2]);
            // And the documented arithmetic holds: score is the weighted mean.
            let weighted: f64 = probabilities
                .iter()
                .map(|entry| f64::from(entry.key) * entry.probability.get())
                .sum();
            assert!(
                (weighted - score).abs() < 1e-9,
                "score {score} is not the probability-weighted mean {weighted}"
            );
        }
        other => panic!("expected a score answer, got {other:?}"),
    }
}

#[test]
fn every_documented_response_asserts_the_weighted_mean_where_it_applies() {
    // The documentation defines `score` as the probability-weighted mean of the level
    // numbers. Checking that against every Score fixture is what would catch the API
    // changing the definition, as opposed to only changing the example values.
    for name in ["response-score.json", "response-score-bug.json"] {
        let body = fixture(name).to_string();
        let response = jev_client::decode_evaluation(body.as_bytes()).unwrap();
        for (_, answer) in &response.answers {
            if let Answer::Score {
                score,
                probabilities,
                ..
            } = answer
            {
                let weighted: f64 = probabilities
                    .iter()
                    .map(|entry| f64::from(entry.key) * entry.probability.get())
                    .sum();
                assert!(
                    (weighted - score).abs() < 1e-9,
                    "{name}: score {score} is not the weighted mean {weighted}"
                );
            }
        }
    }
}

#[test]
fn a_documented_multi_question_response_decodes_every_answer() {
    let body = fixture("response-mixed.json").to_string();
    let response = jev_client::decode_evaluation(body.as_bytes()).unwrap();
    assert_eq!(response.answers.len(), 5);
    for name in [
        "department",
        "return_reason",
        "shipping_issue",
        "requested_resolution",
        "tone",
    ] {
        assert!(response.answer(name).is_some(), "missing {name}");
    }
    // Confidence of exactly 1.0 and probabilities of exactly 0.0 both appear in the
    // documented example; the bounds must accept the closed interval.
    match response.answer("return_reason").unwrap() {
        Answer::Choice { confidence, .. } => assert!((confidence.get() - 1.0).abs() < f64::EPSILON),
        other => panic!("expected a choice answer, got {other:?}"),
    }
}

#[test]
fn a_documented_two_noul_response_decodes() {
    let body = fixture("response-noul-pair.json").to_string();
    let response = jev_client::decode_evaluation(body.as_bytes()).unwrap();
    assert_eq!(response.answers.len(), 2);
}

#[test]
fn the_score_cookbook_response_decodes() {
    let body = fixture("response-score-bug.json").to_string();
    let response = jev_client::decode_evaluation(body.as_bytes()).unwrap();
    match response.answer("bug_severity").unwrap() {
        Answer::Score { score, legend, .. } => {
            assert!((score - 1.43).abs() < 1e-12);
            assert_eq!(
                legend[&1].as_text(),
                Some("Broken or degraded feature, but workaround exists")
            );
        }
        other => panic!("expected a score answer, got {other:?}"),
    }
}

#[test]
fn the_documented_models_response_decodes() {
    let body = fixture("response-models.json").to_string();
    let models = jev_client::decode_models(body.as_bytes()).unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].name, "jev-latest");
    assert_eq!(models[0].release_date, "2026-09-15");
}

/// The live `/v1/models` response, recorded 2026-09-20.
///
/// It differs from the Python SDK's `ModelMetadata` example in two ways a consumer
/// would notice, and neither is written down anywhere in the documentation:
///
///  * `release_date` is a full RFC-3339 **timestamp** with microseconds, not the plain
///    `YYYY-MM-DD` the SDK example shows. Anything parsing it as a date breaks. `jev`
///    treats it as an opaque string and prints it verbatim, which is why it does not.
///  * The list contains only the **aliases**. `jev-1.13.0` is a valid `model` value and
///    is not listed, so this endpoint is not an enumeration of what may be sent.
#[test]
fn the_live_models_response_decodes_with_a_timestamp_release_date() {
    let body = fixture("response-models-live.json").to_string();
    let models = jev_client::decode_models(body.as_bytes()).unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].name, "jev-latest");
    assert_eq!(models[1].name, "jev-preview");
    assert!(
        models[0].release_date.starts_with("2026-09-10T"),
        "release_date is a timestamp, not a date: {:?}",
        models[0].release_date
    );
    // Held as given. Reformatting it here would hide the divergence above from the one
    // place a reader would look for it.
    assert_eq!(models[0].release_date, "2026-09-10T18:38:01.391457+00:00");
}

/// Both shapes decode, because `jev` never parses this field.
#[test]
fn a_release_date_is_an_opaque_string_in_either_shape() {
    for (name, expected) in [
        ("response-models.json", "2026-09-15"),
        (
            "response-models-live.json",
            "2026-09-10T18:38:01.391457+00:00",
        ),
    ] {
        let body = fixture(name).to_string();
        let models = jev_client::decode_models(body.as_bytes()).unwrap();
        assert_eq!(models[0].release_date, expected, "{name}");
    }
}

#[test]
fn the_documented_validation_error_yields_a_field_path() {
    // A 422 is the most common failure a user will hit while writing a request file,
    // so the message has to name the field the server objected to.
    let body = fixture("error-422.json").to_string();
    let message = jev_client::extract_error_message(body.as_bytes()).unwrap();
    assert_eq!(message, "questions.urgency.score.criteria: Field required");
}

// --- Forward and backward compatibility ----------------------------------------------

#[test]
fn an_unknown_answer_type_is_surfaced_rather_than_dropped_or_fatal() {
    // If TypeSafe adds a fourth primitive, an old `jev` must neither fail the whole
    // response nor silently hide the answer. The official Python SDK drops it with a
    // log warning; keeping the payload is strictly more useful to a script.
    let body = json!({
        "model": "jev-latest",
        "answers": {
            "known": {"type": "noul", "noul": 0.5},
            "future": {"type": "ranking", "order": ["a", "b"]}
        },
        "usage": {"input_tokens": 1, "output_tokens": 1}
    })
    .to_string();
    let response = jev_client::decode_evaluation(body.as_bytes()).unwrap();
    assert_eq!(response.answers.len(), 2);
    match response.answer("future").unwrap() {
        Answer::Unrecognized { kind, raw } => {
            assert_eq!(kind, "ranking");
            assert_eq!(raw["order"], json!(["a", "b"]));
        }
        other => panic!("expected an unrecognized answer, got {other:?}"),
    }
}

#[test]
fn an_unknown_field_on_a_known_answer_is_ignored() {
    // Adding a field is a compatible change on the API's side, so an older `jev` must
    // keep working when one appears.
    let body = json!({
        "model": "jev-latest",
        "answers": {"a": {"type": "noul", "noul": 0.5, "explanation": "new field"}},
        "usage": {"input_tokens": 1, "output_tokens": 1, "cached_tokens": 3}
    })
    .to_string();
    let response = jev_client::decode_evaluation(body.as_bytes()).unwrap();
    assert!(response.answer("a").is_some());
    assert_eq!(response.usage.input_tokens, Some(1));
}

#[test]
fn a_response_without_usage_still_decodes() {
    // The official Python SDK models both token counts as optional.
    let body = json!({
        "model": "jev-latest",
        "answers": {"a": {"type": "noul", "noul": 0.5}}
    })
    .to_string();
    let response = jev_client::decode_evaluation(body.as_bytes()).unwrap();
    assert_eq!(response.usage.input_tokens, None);
}

#[test]
fn an_internally_inconsistent_response_is_refused() {
    // These are not shapes the API produces. They are shapes a compromised or
    // misconfigured endpoint could produce, and accepting them would mean reporting a
    // selection that is not in its own distribution, or a score off its own scale.
    let cases = [
        (
            "choice not present in its own probabilities",
            json!({
                "model": "jev-latest",
                "answers": {"a": {"type": "choice", "choice": "ghost", "confidence": 1.0,
                                  "probabilities": {"real": 1.0}}},
                "usage": {}
            }),
        ),
        (
            "score outside the range its legend spans",
            json!({
                "model": "jev-latest",
                "answers": {"a": {"type": "score", "score": 9.0, "confidence": 1.0,
                                  "legend": {"0": "low", "1": "high"},
                                  "probabilities": {"0": 0.5, "1": 0.5}}},
                "usage": {}
            }),
        ),
        (
            "probability above one",
            json!({
                "model": "jev-latest",
                "answers": {"a": {"type": "noul", "noul": 1.5}},
                "usage": {}
            }),
        ),
        (
            "confidence below zero",
            json!({
                "model": "jev-latest",
                "answers": {"a": {"type": "choice", "choice": "x", "confidence": -0.1,
                                  "probabilities": {"x": 1.0}}},
                "usage": {}
            }),
        ),
    ];
    for (label, body) in cases {
        assert!(
            jev_client::decode_evaluation(body.to_string().as_bytes()).is_err(),
            "accepted a response with a {label}"
        );
    }
}
