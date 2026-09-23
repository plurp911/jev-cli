//! Opt-in tests against the **real** TypeSafe API.
//!
//! # These do not run by default
//!
//! Every test here is `#[ignore]`d, so a default run reports them as **ignored** rather
//! than as passing. An earlier version used a runtime early-return, which reported ten
//! green tests that had executed nothing — strictly more misleading than an ignored
//! count, because the suite's headline number silently included them.
//!
//! ```sh
//! JEV_LIVE_TESTS=1 JEV_API_KEY=… cargo test -p jev-cli --test live -- --ignored --nocapture
//! ```
//!
//! They are *also* gated on `JEV_LIVE_TESTS=1` and a credential, and say why they
//! stopped when either is missing, so `--ignored` on a machine with no key reports a
//! clear reason instead of a confusing connection error.
//!
//! # What they cost
//!
//! Deliberately small. Every request uses a short state and the fewest questions that
//! prove the point. The whole matrix is a handful of requests and a few thousand input
//! tokens. `usage.input_tokens` is asserted to be present, so the cost is visible in the
//! output rather than assumed.
//!
//! # What they must never do
//!
//! Print the key, write it anywhere, or leave state on the account. They send synthetic
//! text only — no real data, nothing from the developer's machine.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "a panicking assertion is the correct failure mode inside a test binary"
)]
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "these tests report what the live API actually said and what it cost; \
              the workspace ban on print macros exists to keep rendering inside \
              jev-cli's output module, which does not apply to a test binary"
)]

mod support;

use serde_json::{Value, json};
use support::json_stdout;

/// Synthetic state. Short, unambiguous, and not anybody's data.
const STATE: &str = "Help! My payouts have been failing for 3 days and nobody has replied.";

/// Returns a command configured for the live API, or `None` when the tests are off.
///
/// Prints why it skipped, so a run that does nothing does not look like a run that
/// passed.
fn live() -> Option<assert_cmd::Command> {
    if std::env::var("JEV_LIVE_TESTS").as_deref() != Ok("1") {
        eprintln!("skipped: JEV_LIVE_TESTS is not set to 1");
        return None;
    }
    let has_credential = ["JEV_API_KEY", "JEV_API_KEY_FILE", "TYPESAFE_API_KEY"]
        .iter()
        .any(|name| std::env::var(name).is_ok_and(|value| !value.trim().is_empty()));
    if !has_credential {
        eprintln!("skipped: JEV_LIVE_TESTS is set but no credential is available");
        return None;
    }

    let mut command = assert_cmd::Command::cargo_bin("jev").expect("the `jev` binary should build");
    // The key is inherited from the environment on purpose: it is never written to a
    // file, never passed as an argument, and never printed.
    command
        .env("NO_COLOR", "1")
        .env("JEV_CONFIG_DIR", "/nonexistent/jev-live-test")
        .timeout(std::time::Duration::from_secs(60));
    Some(command)
}

/// Asserts the invariants every live response must satisfy, and prints the cost.
fn check_envelope(document: &Value, label: &str) {
    assert_eq!(document["schema"], json!("jev.evaluation/v1"));

    let model = document["model"]
        .as_str()
        .expect("`model` must be a string");
    assert!(!model.is_empty());
    // The response reports the version that answered, which is the whole point of
    // recording it separately from what was requested.
    println!("[{label}] answered by {model}");

    let input = document["usage"]["input_tokens"]
        .as_u64()
        .expect("the API must report input_tokens");
    let output = document["usage"]["output_tokens"]
        .as_u64()
        .expect("the API must report output_tokens");
    assert!(input > 0, "input_tokens should be positive");
    println!("[{label}] {input} input tokens, {output} output");
}

// --- The matrix ----------------------------------------------------------------------

#[test]
#[ignore = "live API test: run with JEV_LIVE_TESTS=1 and a credential, via --ignored"]
fn authentication_works_and_doctor_agrees() {
    let Some(mut command) = live() else { return };
    let assert = command
        .args(["doctor", "--live", "--output", "json"])
        .assert()
        .success();
    support::assert_no_canary(assert.get_output());

    let document = json_stdout(assert.get_output());
    assert_eq!(document["live"]["checked"], json!(true));
    assert_eq!(
        document["live"]["ok"],
        json!(true),
        "the live check failed: {}",
        document["live"]["detail"]
    );
    assert_eq!(document["endpoint"]["official"], json!(true));
    println!(
        "[doctor] {} in {} ms",
        document["live"]["detail"], document["live"]["milliseconds"]
    );
}

#[test]
#[ignore = "live API test: run with JEV_LIVE_TESTS=1 and a credential, via --ignored"]
fn models_returns_at_least_the_default_alias() {
    let Some(mut command) = live() else { return };
    let assert = command
        .args(["models", "--output", "json"])
        .assert()
        .success();
    let document = json_stdout(assert.get_output());

    let models = document["models"].as_array().expect("`models` is an array");
    assert!(!models.is_empty(), "the account can use no models");
    for model in models {
        assert!(model["name"].is_string());
        assert!(model["description"].is_string());
        assert!(model["release_date"].is_string());
    }
    let names: Vec<&str> = models
        .iter()
        .filter_map(|model| model["name"].as_str())
        .collect();
    println!("[models] {}", names.join(", "));
    assert!(
        names.contains(&"jev-latest"),
        "the documented default alias is missing from the listing: {names:?}"
    );
}

#[test]
#[ignore = "live API test: run with JEV_LIVE_TESTS=1 and a credential, via --ignored"]
fn a_noul_returns_a_probability_and_no_confidence() {
    let Some(mut command) = live() else { return };
    let assert = command
        .args([
            "noul",
            "Does this message convey urgency?",
            "--state",
            STATE,
            "--id",
            "urgent",
            "--output",
            "json",
        ])
        .assert()
        .success();
    let document = json_stdout(assert.get_output());
    check_envelope(&document, "noul");

    let answer = &document["answers"]["urgent"];
    assert_eq!(answer["type"], json!("noul"));
    let noul = answer["noul"].as_f64().expect("`noul` is a number");
    assert!((0.0..=1.0).contains(&noul), "noul out of range: {noul}");
    // The API does not return a confidence for a Noul. If this ever fails, the API
    // changed and `jev` should be updated to surface it rather than keep hiding it.
    assert!(
        answer.get("confidence").is_none(),
        "the API returned a confidence on a Noul: {answer}"
    );
    println!("[noul] {noul}");
    // The state is unambiguously urgent, so a value below 0.5 would mean either the
    // question or this test is wrong. Deliberately loose: this is a smoke test, not a
    // calibration claim.
    assert!(noul > 0.5, "expected an urgent reading, got {noul}");
}

#[test]
#[ignore = "live API test: run with JEV_LIVE_TESTS=1 and a credential, via --ignored"]
fn a_choice_returns_a_selection_a_distribution_and_a_confidence() {
    let Some(mut command) = live() else { return };
    let assert = command
        .args([
            "choice",
            "Which team should handle this?",
            "-O",
            "billing=Payments, invoicing, refunds, payouts",
            "-O",
            "technical=Bugs, outages, integrations",
            "-O",
            "sales=Pricing, upgrades, new accounts",
            "--state",
            STATE,
            "--id",
            "team",
            "--output",
            "json",
        ])
        .assert()
        .success();
    let document = json_stdout(assert.get_output());
    check_envelope(&document, "choice");

    let answer = &document["answers"]["team"];
    assert_eq!(answer["type"], json!("choice"));
    let choice = answer["choice"].as_str().expect("`choice` is a string");
    let probabilities = answer["probabilities"]
        .as_object()
        .expect("`probabilities` is an object");

    // Every option comes back, and only the options that were sent.
    assert_eq!(probabilities.len(), 3, "not every option was returned");
    for option in ["billing", "technical", "sales"] {
        assert!(probabilities.contains_key(option), "missing {option}");
    }
    // The selection is in its own distribution, and the distribution sums to one.
    assert!(probabilities.contains_key(choice));
    let total: f64 = probabilities.values().filter_map(Value::as_f64).sum();
    assert!((total - 1.0).abs() < 1e-6, "probabilities sum to {total}");

    let confidence = answer["confidence"].as_f64().expect("a confidence");
    assert!((0.0..=1.0).contains(&confidence));
    println!("[choice] {choice} at {confidence} confidence, {probabilities:?}");
    assert_eq!(
        choice, "billing",
        "expected the payouts ticket to route to billing"
    );
}

#[test]
#[ignore = "live API test: run with JEV_LIVE_TESTS=1 and a credential, via --ignored"]
fn a_score_returns_a_value_a_legend_and_a_distribution() {
    let Some(mut command) = live() else { return };
    let assert = command
        .args([
            "score",
            "How frustrated is the customer?",
            "-L",
            "Calm",
            "-L",
            "Frustrated",
            "-L",
            "Very angry",
            "--state",
            STATE,
            "--id",
            "frustration",
            "--output",
            "json",
        ])
        .assert()
        .success();
    let document = json_stdout(assert.get_output());
    check_envelope(&document, "score");

    let answer = &document["answers"]["frustration"];
    assert_eq!(answer["type"], json!("score"));
    let score = answer["score"].as_f64().expect("`score` is a number");
    assert!((0.0..=2.0).contains(&score), "score off the scale: {score}");

    let legend = answer["legend"].as_object().expect("a legend");
    assert_eq!(legend.len(), 3);
    assert_eq!(legend["0"], json!("Calm"));
    assert_eq!(legend["2"], json!("Very angry"));

    // The documented arithmetic: the score is the probability-weighted mean of the
    // level numbers. Checking it against the real API is the point of this test.
    let probabilities = answer["probabilities"].as_object().expect("probabilities");
    let weighted: f64 = probabilities
        .iter()
        .filter_map(|(level, probability)| Some(level.parse::<f64>().ok()? * probability.as_f64()?))
        .sum();
    assert!(
        (weighted - score).abs() < 1e-3,
        "score {score} is not the weighted mean {weighted} of {probabilities:?}"
    );
    println!("[score] {score} from {probabilities:?}");
}

#[test]
#[ignore = "live API test: run with JEV_LIVE_TESTS=1 and a credential, via --ignored"]
fn a_mixed_request_answers_every_question_in_one_call() {
    let Some(mut command) = live() else { return };
    let request = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        request.path(),
        json!({
            "state": STATE,
            "questions": {
                "urgent": {"type": "noul", "instructions": "Does this convey urgency?"},
                "repeat": {
                    "type": "noul",
                    "instructions": "Has the customer contacted support about this before?",
                    "criteria": {
                        "true": "Mentions a prior attempt, ticket, or that they have asked before",
                        "false": "No sign of any previous contact"
                    }
                },
                "team": {
                    "type": "choice",
                    "instructions": "Which team should handle this?",
                    "criteria": {
                        "billing": "Payments, invoicing, refunds, payouts",
                        "technical": "Bugs, outages, integrations",
                        "sales": null
                    }
                },
                "frustration": {
                    "type": "score",
                    "instructions": "How frustrated is the customer?",
                    "criteria": ["Calm", "Frustrated", "Very angry"]
                }
            }
        })
        .to_string(),
    )
    .unwrap();

    let assert = command
        .args([
            "ask",
            "-r",
            request.path().to_str().unwrap(),
            "--output",
            "json",
        ])
        .assert()
        .success();
    let document = json_stdout(assert.get_output());
    check_envelope(&document, "ask");

    let answers = document["answers"].as_object().expect("answers");
    assert_eq!(answers.len(), 4, "not every question was answered");
    assert_eq!(answers["urgent"]["type"], json!("noul"));
    assert_eq!(answers["team"]["type"], json!("choice"));
    assert_eq!(answers["frustration"]["type"], json!("score"));
    // A `null` option description is documented as "interpreted by its name alone", so
    // the option must still come back in the distribution.
    assert!(
        answers["team"]["probabilities"]
            .as_object()
            .is_some_and(|map| map.contains_key("sales")),
        "an option with a null description was dropped"
    );
}

#[test]
#[ignore = "live API test: run with JEV_LIVE_TESTS=1 and a credential, via --ignored"]
fn json_state_and_text_state_both_work() {
    let Some(mut command) = live() else { return };
    let assert = command
        .args([
            "noul",
            "Is the customer reporting a payment problem?",
            "--state-json",
            r#"{"subject":"Payouts failing","body":"Three days, no reply.","channel":"email"}"#,
            "--id",
            "payment",
            "--output",
            "json",
        ])
        .assert()
        .success();
    let document = json_stdout(assert.get_output());
    check_envelope(&document, "json-state");
    let noul = document["answers"]["payment"]["noul"]
        .as_f64()
        .expect("a noul");
    println!("[json-state] {noul}");
    assert!(noul > 0.5, "structured state was not understood: {noul}");
}

#[test]
#[ignore = "live API test: run with JEV_LIVE_TESTS=1 and a credential, via --ignored"]
fn a_pinned_model_answers_and_reports_itself() {
    let Some(mut command) = live() else { return };
    // First discover a concrete version from the API rather than hard-coding one, so
    // this test does not go stale when the alias moves.
    let listed = live()
        .expect("already checked")
        .args(["models", "--output", "json"])
        .assert()
        .success();
    let models = json_stdout(listed.get_output());

    // Ask the alias what version answers, then pin that version explicitly.
    let aliased = command
        .args([
            "noul",
            "Is this about payments?",
            "--state",
            STATE,
            "--id",
            "q",
            "--model",
            "jev-latest",
            "--output",
            "json",
        ])
        .assert()
        .success();
    let aliased = json_stdout(aliased.get_output());
    let resolved = aliased["model"].as_str().expect("a model").to_owned();
    assert_eq!(aliased["model_requested"], json!("jev-latest"));
    println!(
        "[pin] jev-latest resolved to {resolved} (listing: {})",
        models["models"]
    );

    let pinned = live()
        .expect("already checked")
        .args([
            "noul",
            "Is this about payments?",
            "--state",
            STATE,
            "--id",
            "q",
            "--model",
            &resolved,
            "--output",
            "json",
        ])
        .assert()
        .success();
    let pinned = json_stdout(pinned.get_output());
    assert_eq!(pinned["model_requested"], json!(resolved));
    assert_eq!(
        pinned["model"], aliased["model"],
        "pinning the resolved version did not reach the same model"
    );
}

#[test]
#[ignore = "live API test: run with JEV_LIVE_TESTS=1 and a credential, via --ignored"]
fn a_validation_error_is_reported_with_its_field() {
    let Some(mut command) = live() else { return };
    // A model identifier that cannot exist. The request is well-formed locally, so the
    // rejection has to come from the API — which is exactly what this checks.
    let assert = command
        .args([
            "noul",
            "Is this urgent?",
            "--state",
            STATE,
            "--model",
            "jev-does-not-exist-9999",
            "--retries",
            "0",
            "--output",
            "json",
        ])
        .assert()
        .failure();

    // The API answers an unknown model with HTTP 400, not the 404 or 422 one might
    // guess (observed 2026-09-23), and both map to exit 2.
    let code = assert.get_output().status.code();
    assert_eq!(code, Some(2), "unexpected exit code for an invalid model");
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(stderr.contains("HTTP 400"), "{stderr}");
    support::assert_no_canary(assert.get_output());
    assert!(
        !stderr.contains("panicked"),
        "the CLI panicked on an API rejection:\n{stderr}"
    );
    println!("[error] exit {code:?}: {}", stderr.trim());
}

#[test]
#[ignore = "live API test: run with JEV_LIVE_TESTS=1 and a credential, via --ignored"]
fn a_rejected_credential_exits_3_after_one_attempt() {
    let Some(mut command) = live() else { return };
    // `JEV_API_KEY` outranks every other source, and the keychain is switched off, so
    // the real key cannot be the one sent. A refused key costs no model tokens.
    let assert = command
        .env("JEV_API_KEY", "jev-live-test-not-a-real-key")
        .env("JEV_NO_KEYCHAIN", "1")
        .args(["noul", "Is this urgent?", "--state", STATE, "--verbose"])
        .assert()
        .code(3);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(stderr.contains("HTTP 401"), "{stderr}");
    assert!(
        stderr.contains("1 attempt(s)"),
        "a rejected credential must not be retried:\n{stderr}"
    );
    assert!(!stderr.contains("jev-live-test-not-a-real-key"), "{stderr}");
    println!("[credential] {}", stderr.trim());
}

#[test]
#[ignore = "live API test: run with JEV_LIVE_TESTS=1 and a credential, via --ignored"]
fn a_gate_against_a_live_answer_exits_as_documented() {
    let Some(mut command) = live() else { return };
    // A condition that cannot hold, so the gate must fail rather than error.
    let assert = command
        .args([
            "noul",
            "Does this message convey urgency?",
            "--state",
            STATE,
            "--id",
            "urgent",
            "--require",
            "urgent.noul > 1.0",
            "--output",
            "json",
        ])
        .assert()
        .code(1);
    let document = json_stdout(assert.get_output());
    assert_eq!(document["gate"]["result"], json!("failed"));
    // The answer is still on stdout, because the data was produced.
    assert!(document["answers"]["urgent"]["noul"].is_number());
}
