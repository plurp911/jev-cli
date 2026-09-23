//! End-to-end tests of the real `jev` binary.
//!
//! These complement the in-process unit tests: they are the only place that exercises
//! actual process exit codes, real stream separation, and the real HTTP transport
//! against a real socket.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "a panicking assertion is the correct failure mode inside a test binary"
)]

mod support;

use predicates::prelude::*;
use serde_json::json;
use support::{
    CANARY_KEY, MockApi, Reply, assert_no_canary, jev, jev_authed, jev_spawnable, json_stdout,
    noul_body, wait_until,
};

// --- Basics ------------------------------------------------------------------------

#[test]
fn bare_invocation_shows_usage_and_exits_two() {
    jev()
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Usage:"));
}

#[test]
fn version_is_printed_to_stdout() {
    jev()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("jev"));
}

#[test]
fn help_goes_to_stdout_and_exits_zero() {
    let assert = jev().arg("--help").assert().success();
    let output = assert.get_output();
    assert!(output.stderr.is_empty(), "help contaminated stderr");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("not affiliated"));
}

#[test]
fn unknown_flag_exits_two_on_stderr_only() {
    let assert = jev().arg("--nope").assert().code(2);
    assert!(assert.get_output().stdout.is_empty());
}

// --- Credentials -------------------------------------------------------------------

#[test]
fn a_missing_credential_exits_three_and_names_every_source() {
    let assert = jev()
        .args(["noul", "urgent?", "--state", "x"])
        .assert()
        .code(3);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    for name in [
        "JEV_API_KEY",
        "JEV_API_KEY_FILE",
        "TYPESAFE_API_KEY",
        "jev auth login",
    ] {
        assert!(stderr.contains(name), "missing {name} from:\n{stderr}");
    }
}

#[test]
fn the_official_sdk_environment_variable_is_honoured() {
    // Against the official endpoint the resolver consults TYPESAFE_API_KEY; the
    // unit tests in `jev-config` cover the order exhaustively. Here we only need to
    // know that an SDK-configured environment is enough to authenticate.
    jev()
        .env("TYPESAFE_API_KEY", CANARY_KEY)
        .args(["auth", "status", "-o", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("typesafe-environment"));
}

#[test]
fn the_jev_variable_takes_precedence_over_the_typesafe_one() {
    let assert = jev()
        .env("JEV_API_KEY", "sk-from-jev-variable")
        .env("TYPESAFE_API_KEY", "sk-from-typesafe-variable")
        .args(["auth", "status", "-o", "json"])
        .assert()
        .success();
    assert_eq!(
        json_stdout(assert.get_output())["effective_source"],
        json!("environment")
    );
}

#[test]
fn a_key_file_is_read_and_its_trailing_newline_stripped() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), format!("{CANARY_KEY}\n")).unwrap();

    jev()
        .env("JEV_CUSTOM_API_KEY_FILE", file.path())
        .args([
            "noul",
            "urgent?",
            "--state",
            "x",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();
    assert_eq!(
        api.requests()[0].headers["authorization"],
        format!("Bearer {CANARY_KEY}")
    );
}

#[test]
fn an_empty_credential_variable_is_reported_rather_than_ignored() {
    jev()
        .env("JEV_API_KEY", "   ")
        .args(["noul", "urgent?", "--state", "x"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("empty"));
}

/// The part of a text report that describes one variable, for asserting its state.
fn line_naming<'a>(text: &'a str, name: &str) -> &'a str {
    text.lines()
        .find(|line| line.trim_start().starts_with(name))
        .unwrap_or_else(|| panic!("no line for {name} in:\n{text}"))
}

#[test]
fn auth_status_and_doctor_report_a_blank_variable_as_set_but_unusable() {
    // A set-but-blank `JEV_API_KEY` stops resolution: the resolver says it is empty
    // and does not fall through. The report used to call it "not set", disagreeing
    // with the resolver about the same environment.
    let status = jev()
        .env("JEV_API_KEY", "")
        .args(["auth", "status", "-o", "json"])
        .assert()
        .code(3);
    let doctor = jev()
        .env("JEV_API_KEY", "")
        .args(["doctor", "-o", "json"])
        .assert()
        .success();
    let status = json_stdout(status.get_output());
    let doctor = json_stdout(doctor.get_output());
    for sources in [&status["sources"], &doctor["credentials"]["sources"]] {
        let sources = sources.as_array().expect("sources is an array");
        let blank = sources
            .iter()
            .find(|source| source["source"] == "environment")
            .expect("the environment source is listed");
        // `present` keeps its documented meaning, a usable value was found; `set` says
        // the variable exists, which is what makes a blank one stop resolution.
        assert_eq!(blank["present"], json!(false), "{blank}");
        assert_eq!(blank["set"], json!(true), "{blank}");
        let unset = sources
            .iter()
            .find(|source| source["source"] == "environment-file")
            .expect("the file source is listed");
        assert_eq!(unset["present"], json!(false), "{unset}");
        assert_eq!(unset["set"], json!(false), "{unset}");
    }

    for args in [&["auth", "status"][..], &["doctor"][..]] {
        let assert = jev().env("JEV_API_KEY", "").args(args).assert();
        let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
        let line = line_naming(&stdout, "JEV_API_KEY ");
        assert!(line.contains("unusable"), "{args:?}: {line}");
        assert!(!line.contains("not set"), "{args:?}: {line}");
    }
}

#[test]
fn a_two_line_key_file_is_refused_before_any_request_is_sent() {
    // A key file with a second line cannot become an HTTP header. It used to reach the
    // transport and fail there as "could not reach the API endpoint", after retries
    // that could never succeed. The mock is loopback, so the custom-endpoint file
    // variable is the one read.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        format!("{CANARY_KEY}\nsk-second-line-of-the-file\n"),
    )
    .unwrap();

    let assert = jev()
        .env("JEV_CUSTOM_API_KEY_FILE", file.path())
        .args([
            "noul",
            "urgent?",
            "--state",
            "x",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .code(3);
    assert_no_canary(assert.get_output());
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("$JEV_CUSTOM_API_KEY_FILE"), "{stderr}");
    assert!(stderr.contains("line break"), "{stderr}");
    assert!(!stderr.contains("sk-second-line"), "{stderr}");
    assert_eq!(api.hits(), 0, "a request was sent with an unsendable key");

    // The official variable, through the one command that resolves without sending.
    // A sending command here would reach the real API if the check regressed.
    let assert = jev()
        .env("JEV_API_KEY_FILE", file.path())
        .args(["auth", "status"])
        .assert()
        .code(3);
    assert_no_canary(assert.get_output());
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("$JEV_API_KEY_FILE"), "{stdout}");
    assert!(stdout.contains("line break"), "{stdout}");
    assert!(!stdout.contains("sk-second-line"), "{stdout}");
}

#[test]
fn auth_status_exits_three_when_there_is_no_credential() {
    jev().args(["auth", "status"]).assert().code(3);
}

#[test]
fn auth_status_never_prints_the_credential() {
    let assert = jev_authed().args(["auth", "status"]).assert().success();
    assert_no_canary(assert.get_output());
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("environment"));
}

#[test]
fn auth_status_json_reports_sources_without_values() {
    let assert = jev_authed()
        .args(["auth", "status", "-o", "json"])
        .assert()
        .success();
    assert_no_canary(assert.get_output());
    let document = json_stdout(assert.get_output());
    assert_eq!(document["schema"], json!("jev.auth/v1"));
    assert_eq!(document["effective_source"], json!("environment"));
}

#[test]
fn auth_login_refuses_a_credential_on_the_command_line() {
    // There is no such flag, and clap rejects it. The structural guarantee is tested in
    // `cli.rs`; this is the user-visible half.
    jev()
        .args(["auth", "login", "--api-key", CANARY_KEY])
        .assert()
        .code(2);
}

#[test]
fn auth_login_refuses_to_prompt_when_stdin_is_not_a_terminal() {
    jev()
        .args(["auth", "login"])
        .write_stdin("")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--stdin"));
}

// --- Endpoint safety ---------------------------------------------------------------

#[test]
fn plain_http_to_a_public_host_is_refused() {
    jev_authed()
        .args([
            "noul",
            "urgent?",
            "--state",
            "x",
            "--endpoint",
            "http://example.com",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("plain HTTP"));
}

#[test]
fn a_custom_endpoint_cannot_use_the_typesafe_credential() {
    // The T4 control, end to end: a stored or exported TypeSafe key is structurally
    // unreachable when `jev` is pointed at another host.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let assert = jev()
        .env("JEV_API_KEY", CANARY_KEY)
        .env("TYPESAFE_API_KEY", CANARY_KEY)
        .args([
            "noul",
            "urgent?",
            "--state",
            "x",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .code(3);
    assert_no_canary(assert.get_output());
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("JEV_CUSTOM_API_KEY"), "{stderr}");
    assert_eq!(api.hits(), 0, "a request was sent without a credential");
}

#[test]
fn a_custom_endpoint_uses_its_own_credential_and_always_warns() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let assert = jev()
        .env("JEV_CUSTOM_API_KEY", "sk-custom-endpoint-key")
        .args([
            "noul",
            "urgent?",
            "--state",
            "x",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("non-official endpoint"), "{stderr}");
    assert_eq!(
        api.requests()[0].headers["authorization"],
        "Bearer sk-custom-endpoint-key"
    );
}

#[test]
fn auth_status_and_doctor_point_a_custom_endpoint_at_its_own_credential() {
    // `auth login` refuses a custom endpoint and `JEV_API_KEY` is never read for one
    // (ADR-0008), so advising either sends the user somewhere that cannot work.
    let endpoint = "http://127.0.0.1:9";
    let status = jev()
        .args(["auth", "status", "--endpoint", endpoint])
        .assert()
        .code(3);
    let doctor = jev()
        .args(["doctor", "--endpoint", endpoint])
        .assert()
        .success();
    for output in [status.get_output(), doctor.get_output()] {
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("JEV_CUSTOM_API_KEY_FILE"), "{stdout}");
        assert!(!stdout.contains("auth login"), "{stdout}");
        assert!(!stdout.contains("set JEV_API_KEY"), "{stdout}");
    }
}

#[test]
fn the_endpoint_warning_survives_quiet() {
    // Where a credential and the user's data are going is not a routine progress note.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let assert = jev()
        .env("JEV_CUSTOM_API_KEY", "sk-custom")
        .args([
            "noul",
            "urgent?",
            "--state",
            "x",
            "--quiet",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();
    assert!(String::from_utf8_lossy(&assert.get_output().stderr).contains("non-official endpoint"));
}

// --- The evaluation path -----------------------------------------------------------

#[test]
fn a_noul_request_matches_the_documented_wire_form() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    jev_authed()
        .args([
            "noul",
            "Does this convey urgency?",
            "--state",
            "Help! My payouts have been failing for 3 days.",
            "--true",
            "Explicitly time-sensitive",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();

    let seen = &api.requests()[0];
    assert_eq!(seen.method, "POST");
    assert_eq!(seen.path, "/v1/systemone");
    assert_eq!(seen.headers["content-type"], "application/json");
    assert!(seen.headers["user-agent"].starts_with("jev-cli/"));

    let body: serde_json::Value = serde_json::from_str(&seen.body).unwrap();
    assert_eq!(
        body,
        json!({
            "state": "Help! My payouts have been failing for 3 days.",
            "model": "jev-latest",
            "questions": {
                "answer": {
                    "type": "noul",
                    "instructions": "Does this convey urgency?",
                    "criteria": {"true": "Explicitly time-sensitive"}
                }
            }
        })
    );
}

#[test]
fn the_json_document_is_one_line_and_carries_its_schema() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let assert = jev_authed()
        .args([
            "noul",
            "urgent?",
            "--state",
            "x",
            "-o",
            "json",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert_eq!(stdout.lines().count(), 1);
    assert!(stdout.ends_with('\n'));

    let document = json_stdout(assert.get_output());
    assert_eq!(document["schema"], json!("jev.evaluation/v1"));
    assert_eq!(document["model"], json!("jev-1.13.0"));
    assert_eq!(document["model_requested"], json!("jev-latest"));
    assert_eq!(document["answers"]["answer"]["noul"], json!(0.92));
    assert_eq!(document["usage"]["input_tokens"], json!(312));
}

#[test]
fn value_mode_prints_the_bare_scalar() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    jev_authed()
        .args([
            "noul",
            "urgent?",
            "--state",
            "x",
            "--value",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success()
        .stdout("0.92\n");
}

#[test]
fn a_choice_round_trips_with_its_full_distribution() {
    let body = json!({
        "model": "jev-1.13.0",
        "answers": {"answer": {
            "type": "choice",
            "choice": "returns",
            "confidence": 0.6,
            "probabilities": {"returns": 0.6, "shipping": 0.3, "billing": 0.1}
        }},
        "usage": {"input_tokens": 10, "output_tokens": 2}
    })
    .to_string();
    let api = MockApi::start(vec![Reply::ok(body)]);

    let assert = jev_authed()
        .args([
            "choice",
            "Which team?",
            "-O",
            "returns=Exchanges",
            "-O",
            "shipping",
            "-O",
            "billing",
            "--state",
            "x",
            "-o",
            "json",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();
    let document = json_stdout(assert.get_output());
    assert_eq!(document["answers"]["answer"]["choice"], json!("returns"));
    assert_eq!(document["answers"]["answer"]["confidence"], json!(0.6));
    assert_eq!(
        document["answers"]["answer"]["probabilities"],
        json!({"returns": 0.6, "shipping": 0.3, "billing": 0.1})
    );
}

#[test]
fn a_score_round_trips_with_its_legend() {
    let body = json!({
        "model": "jev-1.13.0",
        "answers": {"answer": {
            "type": "score",
            "score": 1.3,
            "confidence": 0.54,
            "legend": {"0": "Cosmetic", "1": "Workaround exists", "2": "Blocking"},
            "probabilities": {"0": 0.0, "1": 0.7, "2": 0.3}
        }},
        "usage": {"input_tokens": 10, "output_tokens": 2}
    })
    .to_string();
    let api = MockApi::start(vec![Reply::ok(body)]);

    let assert = jev_authed()
        .args([
            "score",
            "How severe?",
            "-L",
            "Cosmetic",
            "-L",
            "Workaround exists",
            "-L",
            "Blocking",
            "--state",
            "x",
            "-o",
            "json",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();
    let document = json_stdout(assert.get_output());
    assert_eq!(document["answers"]["answer"]["score"], json!(1.3));
    assert_eq!(
        document["answers"]["answer"]["legend"]["1"],
        json!("Workaround exists")
    );
}

#[test]
fn a_mixed_request_asks_every_question_in_one_call() {
    let body = json!({
        "model": "jev-1.13.0",
        "answers": {
            "urgent": {"type": "noul", "noul": 0.9},
            "team": {"type": "choice", "choice": "billing", "confidence": 0.8,
                     "probabilities": {"billing": 0.8, "tech": 0.2}},
            "severity": {"type": "score", "score": 1.0, "confidence": 0.7,
                         "legend": {"0": "low", "1": "high"},
                         "probabilities": {"0": 0.0, "1": 1.0}}
        },
        "usage": {"input_tokens": 50, "output_tokens": 9}
    })
    .to_string();
    let api = MockApi::start(vec![Reply::ok(body)]);

    let request = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        request.path(),
        json!({
            "state": "a support ticket",
            "questions": {
                "urgent": {"type": "noul", "instructions": "Urgent?"},
                "team": {"type": "choice", "instructions": "Which team?",
                         "criteria": {"billing": null, "tech": null}},
                "severity": {"type": "score", "instructions": "How severe?",
                             "criteria": ["low", "high"]}
            }
        })
        .to_string(),
    )
    .unwrap();

    let assert = jev_authed()
        .args([
            "ask",
            "-r",
            request.path().to_str().unwrap(),
            "-o",
            "json",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();

    assert_eq!(api.hits(), 1, "questions were not batched into one request");
    let document = json_stdout(assert.get_output());
    assert_eq!(document["answers"].as_object().unwrap().len(), 3);
}

#[test]
fn a_documented_api_example_can_be_pasted_straight_in() {
    // Copied from https://docs.typesafe.ai/api. If a user cannot run a documentation
    // example unchanged, the request format is an invention rather than the protocol.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let document = r#"{
      "state": "Help! My payouts have been failing for 3 days.",
      "model": "jev-latest",
      "questions": {
        "is_urgent": {
          "type": "noul",
          "instructions": "Does this convey urgency?",
          "criteria": {"true": "Explicitly time-sensitive", "false": "No urgency expressed"}
        }
      }
    }"#;
    jev_authed()
        .args(["ask", "--endpoint", &api.endpoint()])
        .write_stdin(document)
        .assert()
        .success();
    let body: serde_json::Value = serde_json::from_str(&api.requests()[0].body).unwrap();
    assert_eq!(
        body,
        serde_json::from_str::<serde_json::Value>(document).unwrap()
    );
}

// --- Errors and exit codes ---------------------------------------------------------

#[test]
fn an_unauthorized_response_exits_three_and_is_not_retried() {
    let api = MockApi::start(vec![Reply::status(401, r#"{"detail":"Invalid API key"}"#)]);
    let assert = jev_authed()
        .args(["noul", "q", "--state", "x", "--endpoint", &api.endpoint()])
        .assert()
        .code(3);
    assert_no_canary(assert.get_output());
    assert_eq!(api.hits(), 1, "an authentication failure was retried");
    assert!(String::from_utf8_lossy(&assert.get_output().stderr).contains("Invalid API key"));
}

#[test]
fn a_validation_failure_exits_two_and_names_the_field() {
    let body = json!({
        "detail": [{"loc": ["body", "questions", "answer", "criteria"], "msg": "Field required"}]
    })
    .to_string();
    let api = MockApi::start(vec![Reply::status(422, body)]);
    let assert = jev_authed()
        .args(["noul", "q", "--state", "x", "--endpoint", &api.endpoint()])
        .assert()
        .code(2);
    assert!(
        String::from_utf8_lossy(&assert.get_output().stderr)
            .contains("questions.answer.criteria: Field required")
    );
}

#[test]
fn an_overloaded_api_is_retried_and_then_exits_four() {
    let api = MockApi::start(vec![Reply::status(529, "overloaded")]);
    jev_authed()
        .args([
            "noul",
            "q",
            "--state",
            "x",
            "--retries",
            "2",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .code(4);
    assert_eq!(
        api.hits(),
        3,
        "expected the initial attempt plus two retries"
    );
}

#[test]
fn a_rate_limit_honours_the_retry_after_ms_header_and_then_succeeds() {
    let api = MockApi::start(vec![
        Reply::status(429, "slow down").header("retry-after-ms", "10"),
        Reply::ok(noul_body()),
    ]);
    jev_authed()
        .args(["noul", "q", "--state", "x", "--endpoint", &api.endpoint()])
        .assert()
        .success();
    assert_eq!(api.hits(), 2);
}

#[test]
fn retries_can_be_disabled() {
    let api = MockApi::start(vec![Reply::status(503, "down")]);
    jev_authed()
        .args([
            "noul",
            "q",
            "--state",
            "x",
            "--retries",
            "0",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .code(4);
    assert_eq!(api.hits(), 1);
}

#[test]
fn a_connection_failure_exits_four() {
    let dead = MockApi::dead_endpoint();
    jev_authed()
        .args([
            "noul",
            "q",
            "--state",
            "x",
            "--retries",
            "0",
            "--endpoint",
            &dead,
        ])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("could not reach"));
}

#[test]
fn every_sending_command_sends_nothing_under_dry_run() {
    // `--dry-run` was covered for `noul` only. `ask` and `map` each have their own
    // dry-run implementation, and `map` -- the command with by far the largest blast
    // radius, N records by M questions -- had none at all.
    let questions = one_noul_question();

    for args in [
        vec!["choice", "q", "-O", "a", "-O", "b", "--state", "x"],
        vec!["score", "q", "-L", "low", "-L", "high", "--state", "x"],
        vec!["ask", "-r", "@QUESTIONS@", "--state", "x"],
    ] {
        let api = MockApi::start(vec![Reply::ok(noul_body())]);
        let resolved: Vec<String> = args
            .iter()
            .map(|argument| {
                if *argument == "@QUESTIONS@" {
                    questions.path().to_string_lossy().into_owned()
                } else {
                    (*argument).to_owned()
                }
            })
            .collect();

        let assert = jev_authed()
            .args(&resolved)
            .args([
                "--dry-run",
                "--output",
                "json",
                "--endpoint",
                &api.endpoint(),
            ])
            .assert()
            .success();

        assert_eq!(
            api.hits(),
            0,
            "{resolved:?} sent a request during a dry run"
        );
        let document = json_stdout(assert.get_output());
        assert_eq!(document["schema"], json!("jev.dry-run/v1"));
        assert_eq!(document["sent"], json!(false));
        assert_no_canary(assert.get_output());
    }
}

#[test]
fn map_dry_run_sends_nothing_and_writes_no_output_file() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let questions = one_noul_question();
    let directory = tempfile::tempdir().unwrap();
    // Deliberately a path that does not exist: a dry run must not bring it into being.
    let out = directory.path().join("results.jsonl");

    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--output-file",
            out.to_str().unwrap(),
            "--dry-run",
            "--endpoint",
            &api.endpoint(),
        ])
        .write_stdin("\"a\"\n\"b\"\n\"c\"\n\"d\"\n")
        .assert()
        .success();

    assert_eq!(api.hits(), 0, "a dry run sent a request");
    assert!(!out.exists(), "a dry run created the output file");

    let document = json_stdout(assert.get_output());
    assert_eq!(document["schema"], json!("jev.dry-run/v1"));
    assert_eq!(document["sent"], json!(false));
    assert_eq!(document["records"], json!(4));
    assert_eq!(document["sample"].as_array().unwrap().len(), 3);
    assert_eq!(document["sample_truncated"], json!(true));
    assert_no_canary(assert.get_output());
}

#[test]
fn a_dry_run_shows_the_body_that_would_really_be_sent() {
    // The dry-run document used to be built by a second, parallel construction of the
    // URL, the header list, and the body. A rehearsal assembled separately from the
    // performance can diverge from it, and the whole value of `--dry-run` is that it
    // cannot. This pins them together: the same request, once previewed and once sent.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let questions = one_noul_question();

    let preview = jev_authed()
        .args([
            "ask",
            "-r",
            questions.path().to_str().unwrap(),
            "--state",
            "some state",
            "--dry-run",
            "--output",
            "json",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();
    let document = json_stdout(preview.get_output());

    jev_authed()
        .args([
            "ask",
            "-r",
            questions.path().to_str().unwrap(),
            "--state",
            "some state",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();

    let sent = &api.requests()[0];
    let sent_body: serde_json::Value = serde_json::from_str(&sent.body).expect("the real body");
    assert_eq!(
        document["body"], sent_body,
        "the dry run previewed a different body from the one that was sent"
    );
    assert_eq!(
        document["body_bytes"],
        json!(sent.body.len()),
        "body_bytes did not match the bytes that went on the wire"
    );
    assert!(
        document["url"].as_str().unwrap().ends_with("/v1/systemone"),
        "the previewed URL is not the one that was used: {}",
        document["url"]
    );
}

#[test]
fn value_output_is_always_exactly_one_line() {
    // `--value` promises "one scalar and a newline, nothing else". A Choice's selected
    // option is API-supplied text, so it can contain a newline or a tab -- and
    // `sanitize` deliberately preserves both, because it is written for prose
    // diagnostics. `x=$(jev … --value)` then held an embedded newline and `read x`
    // truncated it.
    let api = MockApi::start(vec![Reply::ok(
        json!({
            "model": "jev-1.13.0",
            "answers": {"answer": {
                "type": "choice",
                "choice": "bill\ning\tX",
                "confidence": 0.9,
                "probabilities": {"bill\ning\tX": 1.0}
            }},
            "usage": {"input_tokens": 1, "output_tokens": 1}
        })
        .to_string(),
    )]);

    let assert = jev_authed()
        .args([
            "choice",
            "q",
            "-O",
            "a",
            "-O",
            "b",
            "--state",
            "x",
            "--value",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert_eq!(
        stdout.lines().count(),
        1,
        "--value emitted more than one line: {stdout:?}"
    );
    assert!(stdout.ends_with('\n'));
    assert!(
        !stdout.trim_end().contains('\t'),
        "a raw tab survived into a scalar: {stdout:?}"
    );
}

#[test]
fn a_proxy_in_the_environment_does_not_reroute_the_request() {
    // `ureq`'s default configuration reads `HTTP_PROXY`, `HTTPS_PROXY`, and
    // `ALL_PROXY`, so before `HttpTransport` disabled it a variable `jev` neither
    // documents nor reports decided where every request went. For the one case where
    // `jev` permits cleartext -- loopback, justified by "there is no network to
    // observe" -- that meant a CONNECT tunnel to the proxy carrying the `Authorization`
    // header in the clear, to exactly the observer the justification assumed away.
    //
    // A dead proxy port is not a test: the agent falls back to a direct connection and
    // the run succeeds either way. The proxy here therefore *listens* and counts, so a
    // reintroduction shows up as a connection that should not exist.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);

    let proxy = std::net::TcpListener::bind("127.0.0.1:0").expect("bind proxy");
    let proxy_url = format!("http://127.0.0.1:{}", proxy.local_addr().unwrap().port());
    let contacted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = std::sync::Arc::clone(&contacted);
    let proxy_thread = std::thread::spawn(move || {
        for stream in proxy.incoming() {
            let Ok(stream) = stream else { break };
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            drop(stream);
        }
    });

    jev_authed()
        .env("HTTP_PROXY", &proxy_url)
        .env("HTTPS_PROXY", &proxy_url)
        .env("ALL_PROXY", &proxy_url)
        .env("http_proxy", &proxy_url)
        .env("https_proxy", &proxy_url)
        .env("all_proxy", &proxy_url)
        .args([
            "noul",
            "q",
            "--state",
            "x",
            "--retries",
            "0",
            "--endpoint",
            &api.endpoint(),
        ])
        .timeout(std::time::Duration::from_secs(20))
        .assert()
        .success();

    assert_eq!(
        contacted.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the request was routed through a proxy named only by the environment"
    );
    assert_eq!(
        api.hits(),
        1,
        "the request did not reach the endpoint the user named"
    );

    // Unblock `incoming()` so the thread can end with the test.
    let _ = std::net::TcpStream::connect(proxy_url.trim_start_matches("http://"));
    drop(proxy_thread);
}

#[test]
fn a_timeout_exits_four_without_hanging() {
    let api = MockApi::hanging();
    jev_authed()
        .args([
            "noul",
            "q",
            "--state",
            "x",
            "--timeout",
            "1",
            "--retries",
            "0",
            "--endpoint",
            &api.endpoint(),
        ])
        .timeout(std::time::Duration::from_secs(20))
        .assert()
        .code(4)
        .stderr(predicate::str::contains("timed out"));
}

#[test]
fn a_malformed_response_exits_four_rather_than_panicking() {
    for body in [
        "not json at all".to_owned(),
        "{}".to_owned(),
        json!({"model": "jev-1.13.0", "answers": {"answer": {"type": "noul", "noul": 5}}})
            .to_string(),
        json!({"model": "jev-1.13.0", "answers": {"answer": {"type": "noul"}}}).to_string(),
    ] {
        let api = MockApi::start(vec![Reply::ok(body.clone())]);
        let assert = jev_authed()
            .args(["noul", "q", "--state", "x", "--endpoint", &api.endpoint()])
            .assert()
            .code(4);
        let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
        assert!(
            !stderr.contains("panicked"),
            "panicked on {body}:\n{stderr}"
        );
    }
}

#[test]
fn an_invalid_utf8_response_is_an_error_not_a_lossy_success() {
    let api = MockApi::start(vec![Reply::ok("").raw(vec![0xff, 0xfe, 0x00, 0x01])]);
    jev_authed()
        .args(["noul", "q", "--state", "x", "--endpoint", &api.endpoint()])
        .assert()
        .code(4);
}

#[test]
fn a_deeply_nested_response_is_refused() {
    // A hostile endpoint must not be able to exhaust the stack or the heap.
    let mut body =
        String::from(r#"{"model":"jev-1.13.0","answers":{"answer":{"type":"noul","noul":"#);
    body.push_str(&"[".repeat(5000));
    body.push_str(&"]".repeat(5000));
    body.push_str("}},\"usage\":{}}");
    let api = MockApi::start(vec![Reply::ok(body)]);
    let assert = jev_authed()
        .args(["noul", "q", "--state", "x", "--endpoint", &api.endpoint()])
        .assert()
        .code(4);
    assert!(!String::from_utf8_lossy(&assert.get_output().stderr).contains("panicked"));
}

// --- Gating ------------------------------------------------------------------------

#[test]
fn a_satisfied_gate_exits_zero_and_still_prints_the_answer() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let assert = jev_authed()
        .args([
            "noul",
            "q",
            "--state",
            "x",
            "-o",
            "json",
            "--require",
            "answer.noul > 0.9",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();
    let document = json_stdout(assert.get_output());
    assert_eq!(document["gate"]["passed"], json!(true));
}

#[test]
fn an_unsatisfied_gate_exits_one_and_still_prints_the_answer() {
    // The data is still produced, so it still belongs on stdout. Only the status
    // changes.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let assert = jev_authed()
        .args([
            "noul",
            "q",
            "--state",
            "x",
            "-o",
            "json",
            "--require",
            "answer.noul > 0.99",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .code(1);
    let document = json_stdout(assert.get_output());
    assert_eq!(document["gate"]["result"], json!("failed"));
    assert_eq!(document["answers"]["answer"]["noul"], json!(0.92));
}

#[test]
fn an_unevaluable_gate_exits_six_and_never_zero() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let assert = jev_authed()
        .args([
            "noul",
            "q",
            "--state",
            "x",
            "-o",
            "json",
            "--require",
            "typo.noul > 0.5",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .code(6);
    let document = json_stdout(assert.get_output());
    assert_eq!(document["gate"]["result"], json!("unevaluable"));
    assert_eq!(document["gate"]["passed"], json!(false));
}

#[test]
fn a_malformed_gate_expression_costs_nothing() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let assert = jev_authed()
        .args([
            "noul",
            "q",
            "--state",
            "x",
            "--require",
            "answer.noul = 0.5",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .code(2);
    assert_eq!(
        api.hits(),
        0,
        "a bad expression should be caught before sending"
    );
    assert!(String::from_utf8_lossy(&assert.get_output().stderr).contains("comparison operator"));
}

// --- Input handling ----------------------------------------------------------------

#[test]
fn state_arrives_from_stdin_a_file_or_a_flag_identically() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), "the state\n").unwrap();

    for (label, mut command) in [
        ("flag", {
            let mut c = jev_authed();
            c.args(["noul", "q", "--state", "the state"]);
            c
        }),
        ("file", {
            let mut c = jev_authed();
            c.args(["noul", "q", "--state-file", file.path().to_str().unwrap()]);
            c
        }),
        ("stdin", {
            let mut c = jev_authed();
            c.args(["noul", "q"]).write_stdin("the state\n");
            c
        }),
        ("dash", {
            let mut c = jev_authed();
            c.args(["noul", "q", "--state-file", "-"])
                .write_stdin("the state\n");
            c
        }),
    ] {
        let api = MockApi::start(vec![Reply::ok(noul_body())]);
        command
            .args(["--endpoint", &api.endpoint()])
            .assert()
            .success();
        let body: serde_json::Value = serde_json::from_str(&api.requests()[0].body).unwrap();
        assert_eq!(body["state"], json!("the state"), "via {label}");
    }
}

#[test]
fn json_state_keeps_its_structure() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    jev_authed()
        .args([
            "noul",
            "q",
            "--state-json",
            r#"{"subject":"x","lines":["a","b"]}"#,
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();
    let body: serde_json::Value = serde_json::from_str(&api.requests()[0].body).unwrap();
    assert_eq!(body["state"], json!({"subject": "x", "lines": ["a", "b"]}));
}

#[test]
fn conflicting_state_flags_are_a_usage_error() {
    jev_authed()
        .args(["noul", "q", "--state", "a", "--state-json", "\"b\""])
        .assert()
        .code(2);
}

#[test]
fn binary_input_is_refused_before_anything_is_sent() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), [0x7f, 0x45, 0x4c, 0x46, 0x00]).unwrap();
    jev_authed()
        .args([
            "noul",
            "q",
            "--state-file",
            file.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("binary"));
    assert_eq!(api.hits(), 0);
}

#[test]
fn oversized_input_is_refused_and_explains_the_limit() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    jev_authed()
        .args([
            "noul",
            "q",
            "--max-input-bytes",
            "16",
            "--endpoint",
            &api.endpoint(),
        ])
        .write_stdin("x".repeat(100))
        .assert()
        .code(2)
        .stderr(predicate::str::contains("never silently truncates"));
    assert_eq!(api.hits(), 0);
}

#[test]
fn empty_input_is_refused_before_anything_is_sent() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    jev_authed()
        .args(["noul", "q", "--endpoint", &api.endpoint()])
        .write_stdin("")
        .assert()
        .code(2);
    assert_eq!(api.hits(), 0);
}

// --- Output hygiene ----------------------------------------------------------------

#[test]
fn api_supplied_escape_sequences_never_reach_the_terminal_raw() {
    let body = json!({
        "model": "jev-1.13.0",
        "answers": {"answer": {
            "type": "choice",
            "choice": "bill\u{1b}[2Jing",
            "confidence": 1.0,
            "probabilities": {"bill\u{1b}[2Jing": 1.0}
        }},
        "usage": {}
    })
    .to_string();
    let api = MockApi::start(vec![Reply::ok(body)]);
    let assert = jev_authed()
        .args([
            "choice",
            "q",
            "-O",
            "a",
            "-O",
            "b",
            "--state",
            "x",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();
    assert!(
        !assert.get_output().stdout.contains(&0x1b),
        "a raw escape byte reached stdout"
    );
}

#[test]
fn diagnostics_never_contaminate_stdout() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let assert = jev_authed()
        .args([
            "noul",
            "q",
            "--state",
            "x",
            "-o",
            "json",
            "--verbose",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();
    // Verbose notes went somewhere, and it was not stdout.
    assert!(!assert.get_output().stderr.is_empty());
    let _ = json_stdout(assert.get_output());
}

#[test]
fn verbose_output_never_contains_the_credential() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let assert = jev_authed()
        .args([
            "noul",
            "q",
            "--state",
            "x",
            "--verbose",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();
    assert_no_canary(assert.get_output());
}

#[test]
fn a_dry_run_sends_nothing_and_shows_no_credential() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let assert = jev_authed()
        .args([
            "noul",
            "q",
            "--state",
            "x",
            "--dry-run",
            "-o",
            "json",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();
    assert_eq!(api.hits(), 0);
    assert_no_canary(assert.get_output());
    let document = json_stdout(assert.get_output());
    assert_eq!(document["sent"], json!(false));
    assert_eq!(document["body"]["state"], json!("x"));
}

#[test]
fn a_broken_pipe_is_success() {
    // `jev … | head -c1` is normal Unix behaviour.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "{} noul q --state x -o json --endpoint {} | head -c 1 >/dev/null",
            assert_cmd::cargo::cargo_bin("jev").display(),
            api.endpoint()
        ))
        .env("JEV_API_KEY", CANARY_KEY)
        .env("JEV_CUSTOM_API_KEY", CANARY_KEY)
        .env("JEV_CONFIG_DIR", "/nonexistent/jev-integration-test")
        .status()
        .unwrap();
    assert!(status.success());
}

// --- models, doctor, config, completions --------------------------------------------

#[test]
fn models_lists_what_the_api_returns_and_nothing_hard_coded() {
    let body = json!({
        "models": [
            {"name": "jev-latest", "description": "flagship", "release_date": "2026-09-15"},
            {"name": "some-future-model", "description": "new", "release_date": "2027-01-01"}
        ]
    })
    .to_string();
    let api = MockApi::start(vec![Reply::ok(body)]);
    let assert = jev_authed()
        .args(["models", "-o", "json", "--endpoint", &api.endpoint()])
        .assert()
        .success();
    assert_eq!(api.requests()[0].method, "GET");
    assert_eq!(api.requests()[0].path, "/v1/models");
    let document = json_stdout(assert.get_output());
    assert_eq!(document["schema"], json!("jev.models/v1"));
    assert_eq!(document["models"].as_array().unwrap().len(), 2);
}

#[test]
fn doctor_makes_no_request_by_default() {
    let api = MockApi::start(vec![Reply::ok("{}")]);
    jev_authed()
        .args(["doctor", "--endpoint", &api.endpoint()])
        .assert()
        .success();
    assert_eq!(api.hits(), 0, "doctor contacted the network without --live");
}

#[test]
fn doctor_live_makes_exactly_one_cheap_call() {
    let api = MockApi::start(vec![Reply::ok(r#"{"models":[]}"#)]);
    let assert = jev_authed()
        .args([
            "doctor",
            "--live",
            "-o",
            "json",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();
    assert_eq!(api.hits(), 1);
    assert_eq!(api.requests()[0].path, "/v1/models");
    let document = json_stdout(assert.get_output());
    assert_eq!(document["live"]["checked"], json!(true));
    assert_eq!(document["live"]["ok"], json!(true));
}

#[test]
fn doctor_never_prints_the_credential() {
    let assert = jev_authed()
        .args(["doctor", "--verbose"])
        .assert()
        .success();
    assert_no_canary(assert.get_output());
}

#[test]
fn config_refuses_to_store_a_secret() {
    let dir = tempfile::tempdir().unwrap();
    jev()
        .env("JEV_CONFIG_DIR", dir.path())
        .args(["config", "set", "api_key", CANARY_KEY])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("credential"));
    // And nothing was written.
    assert!(!dir.path().join("config.toml").exists());
}

#[test]
fn config_set_get_and_unset_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let with_dir = |args: &[&str]| {
        let mut command = jev();
        command.env("JEV_CONFIG_DIR", dir.path()).args(args);
        command
    };

    with_dir(&["config", "set", "model", "jev-1.13.0"])
        .assert()
        .success();
    with_dir(&["config", "get", "model"])
        .assert()
        .success()
        .stdout("jev-1.13.0\n");
    with_dir(&["config", "unset", "model"]).assert().success();
    with_dir(&["config", "get", "model"]).assert().code(1);
}

#[test]
fn a_configured_model_is_used_and_a_flag_overrides_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "model = \"jev-1.13.0\"\n").unwrap();

    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    jev_authed()
        .env("JEV_CONFIG_DIR", dir.path())
        .args(["noul", "q", "--state", "x", "--endpoint", &api.endpoint()])
        .assert()
        .success();
    let body: serde_json::Value = serde_json::from_str(&api.requests()[0].body).unwrap();
    assert_eq!(body["model"], json!("jev-1.13.0"));

    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    jev_authed()
        .env("JEV_CONFIG_DIR", dir.path())
        .args([
            "noul",
            "q",
            "--state",
            "x",
            "--model",
            "jev-preview",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();
    let body: serde_json::Value = serde_json::from_str(&api.requests()[0].body).unwrap();
    assert_eq!(body["model"], json!("jev-preview"));
}

#[test]
fn completions_are_generated_for_every_supported_shell() {
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let assert = jev().args(["completions", shell]).assert().success();
        assert!(
            !assert.get_output().stdout.is_empty(),
            "no completion script for {shell}"
        );
    }
}

// --- map ----------------------------------------------------------------------------

#[test]
fn map_evaluates_every_record_in_input_order() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let questions = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        questions.path(),
        json!({"urgent": {"type": "noul", "instructions": "Urgent?"}}).to_string(),
    )
    .unwrap();

    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "-j",
            "4",
        ])
        .write_stdin("\"first\"\n\"second\"\n\"third\"\n")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let lines: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 4, "three rows plus a summary");
    for (index, line) in lines.iter().take(3).enumerate() {
        assert_eq!(line["schema"], json!("jev.map.row/v1"));
        assert_eq!(line["index"], json!(index));
        assert_eq!(line["ok"], json!(true));
    }
    assert_eq!(lines[3]["schema"], json!("jev.map.summary/v1"));
    assert_eq!(lines[3]["succeeded"], json!(3));
}

#[test]
fn map_reports_partial_failure_with_exit_five_and_keeps_the_good_rows() {
    let api = MockApi::start(vec![
        Reply::ok(noul_body()),
        Reply::status(400, r#"{"detail":"bad"}"#),
        Reply::ok(noul_body()),
    ]);
    let questions = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        questions.path(),
        json!({"urgent": {"type": "noul", "instructions": "Urgent?"}}).to_string(),
    )
    .unwrap();

    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "-j",
            "1",
            "--retries",
            "0",
        ])
        .write_stdin("\"a\"\n\"b\"\n\"c\"\n")
        .assert()
        .code(5);

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let lines: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines[0]["ok"], json!(true));
    assert_eq!(lines[1]["ok"], json!(false));
    assert_eq!(lines[2]["ok"], json!(true));
    assert_eq!(lines[3]["failed"], json!(1));
}

#[test]
fn map_resumes_without_re_evaluating_completed_records() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let questions = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        questions.path(),
        json!({"urgent": {"type": "noul", "instructions": "Urgent?"}}).to_string(),
    )
    .unwrap();
    let out = tempfile::NamedTempFile::new().unwrap();
    // Pretend index 0 and 1 already completed in an earlier run.
    std::fs::write(
        out.path(),
        "{\"schema\":\"jev.map.row/v1\",\"index\":0,\"ok\":true}\n\
         {\"schema\":\"jev.map.row/v1\",\"index\":1,\"ok\":true}\n",
    )
    .unwrap();

    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--output-file",
            out.path().to_str().unwrap(),
            "--resume",
            "--endpoint",
            &api.endpoint(),
        ])
        .write_stdin("\"a\"\n\"b\"\n\"c\"\n")
        .assert()
        .success();

    assert_eq!(api.hits(), 1, "a completed record was re-evaluated");
    let summary = json_stdout(assert.get_output());
    assert_eq!(summary["resumed"], json!(2));
    assert_eq!(summary["evaluated"], json!(1));

    // The file now holds three rows and no summary line.
    let written = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(written.lines().count(), 3);
    assert!(!written.contains("jev.map.summary"));
}

#[test]
fn map_resume_does_not_append_onto_a_truncated_line() {
    // The failure `--resume` exists to handle: a `SIGKILL` or a power cut leaves the
    // last row half-written. `read_completed` already tolerated that and re-ran the
    // record — but the append-mode writer then concatenated the new row onto the
    // partial one, so the re-run record was billed, answered, and written into a line
    // that parses as neither row, while the summary reported the batch complete and
    // exited 0.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let questions = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        questions.path(),
        json!({"urgent": {"type": "noul", "instructions": "Urgent?"}}).to_string(),
    )
    .unwrap();

    let out = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        out.path(),
        "{\"schema\":\"jev.map.row/v1\",\"index\":0,\"ok\":true}\n{\"schema\":\"jev.map.row/v1\",\"index\":1,\"ok\":tr",
    )
    .unwrap();

    jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--output-file",
            out.path().to_str().unwrap(),
            "--resume",
            "--endpoint",
            &api.endpoint(),
        ])
        .write_stdin("\"a\"\n\"b\"\n")
        .assert()
        .success();

    // Every line the file now holds must be a document on its own. The truncated one
    // stays truncated -- it is skipped again on the next resume -- but nothing may be
    // welded onto it.
    let written = std::fs::read_to_string(out.path()).unwrap();
    let mut parsed = 0;
    for line in written.lines() {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
            assert_eq!(value["schema"], json!("jev.map.row/v1"));
            parsed += 1;
        } else {
            assert!(
                line == "{\"schema\":\"jev.map.row/v1\",\"index\":1,\"ok\":tr",
                "a line was corrupted rather than left truncated: {line}"
            );
        }
    }
    assert_eq!(
        parsed, 2,
        "the re-evaluated row was not written as a parseable line of its own"
    );
}

#[test]
fn map_output_file_rows_are_escaped_like_stdout() {
    // The escaping was applied only on the stdout path, so `--output-file` -- the
    // artifact most likely to be `cat`-ed, grepped, or diffed days later -- was the
    // one place a bidirectional override or a zero-width character from an API
    // response survived raw. An option name that reads as `approve` while containing
    // `deny` reached a reviewer's terminal from a file nobody thought to distrust.
    let api = MockApi::start(vec![Reply::ok(
        json!({
            "model": "jev-1.13.0",
            "answers": {"urgent": {
                "type": "choice",
                "choice": "approve\u{202e}yned\u{200b}\u{7f}",
                "confidence": 0.9,
                "probabilities": {"approve\u{202e}yned\u{200b}\u{7f}": 1.0}
            }},
            "usage": {}
        })
        .to_string(),
    )]);
    let questions = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        questions.path(),
        json!({"urgent": {"type": "choice", "instructions": "Which?",
               "criteria": {"a": "first", "b": "second"}}})
        .to_string(),
    )
    .unwrap();
    let out = tempfile::NamedTempFile::new().unwrap();

    jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--output-file",
            out.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .success();

    let written = std::fs::read_to_string(out.path()).unwrap();
    for (raw, escaped) in [
        ('\u{202e}', "\\u202e"),
        ('\u{200b}', "\\u200b"),
        ('\u{7f}', "\\u007f"),
    ] {
        assert!(
            !written.contains(raw),
            "a raw {raw:?} survived into the output file"
        );
        assert!(
            written.contains(escaped),
            "{escaped} is missing from the output file: {written}"
        );
    }

    // And the value a consumer parses is still exactly what the API sent.
    let row: serde_json::Value = serde_json::from_str(written.trim()).unwrap();
    assert_eq!(
        row["answers"]["urgent"]["choice"],
        json!("approve\u{202e}yned\u{200b}\u{7f}")
    );
}
#[cfg(unix)]
#[test]
fn a_map_output_file_this_run_creates_is_private() {
    use std::os::unix::fs::PermissionsExt as _;

    // The rows hold the model's answers about the user's state, which the threat model
    // lists among the assets worth protecting -- the same reasoning that makes the
    // configuration file 0600. Left to the umask this came out 0644 or 0664, readable
    // by anyone else on a shared machine or a CI runner.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let questions = one_noul_question();
    let directory = tempfile::tempdir().unwrap();
    let out = directory.path().join("rows.jsonl");

    jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--output-file",
            out.to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .success();

    let mode = std::fs::metadata(&out).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o600,
        "the output file is readable by others: {mode:o}"
    );
}

#[cfg(unix)]
#[test]
fn a_map_output_file_the_user_already_made_keeps_their_permissions() {
    use std::os::unix::fs::PermissionsExt as _;

    // The other half of the rule: the mode of a file the user created is their
    // decision. `OpenOptions::mode` applies only on creation, and this pins that -- a
    // run that silently tightened an existing file would break a deliberate setup.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let questions = one_noul_question();
    let directory = tempfile::tempdir().unwrap();
    let out = directory.path().join("rows.jsonl");
    std::fs::write(&out, "").unwrap();
    std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o644)).unwrap();

    jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--output-file",
            out.to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .success();

    let mode = std::fs::metadata(&out).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o644, "an existing file's permissions were changed");
}

#[test]
fn map_fail_fast_stops_at_the_first_failure_and_keeps_what_succeeded() {
    // `--fail-fast` is a user-facing flag with no coverage at all. The behaviour it
    // promises is three things at once, and each could regress independently: stop
    // early, still write the rows already paid for, and report the run as incomplete
    // rather than as a clean pass.
    let api = MockApi::start(vec![
        Reply::ok(noul_body()),
        Reply::status(422, r#"{"detail":"nope"}"#),
        Reply::ok(noul_body()),
    ]);
    let questions = one_noul_question();
    let out = tempfile::NamedTempFile::new().unwrap();

    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--output-file",
            out.path().to_str().unwrap(),
            "-j",
            "1",
            "--fail-fast",
            "--retries",
            "0",
            "--endpoint",
            &api.endpoint(),
        ])
        .write_stdin("\"a\"\n\"b\"\n\"c\"\n\"d\"\n")
        .assert()
        .code(5);

    let summary = json_stdout(assert.get_output());
    assert_eq!(summary["total"], json!(4));
    assert_eq!(summary["failed"], json!(1));
    assert_eq!(
        summary["stopped_early"],
        json!(true),
        "the run did not stop"
    );
    assert_eq!(
        summary["interrupted"],
        json!(false),
        "stopping for --fail-fast is not an interrupt"
    );
    assert!(
        summary["evaluated"].as_u64().unwrap() < 4,
        "every record was evaluated, so --fail-fast did nothing: {summary}"
    );

    // The successful row was still written: the user paid for it.
    let written = std::fs::read_to_string(out.path()).unwrap();
    assert!(
        written.lines().any(|line| line.contains("\"ok\":true")),
        "a successful row was discarded when the batch stopped"
    );
}

#[test]
fn map_uses_the_named_state_and_id_fields() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let questions = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        questions.path(),
        json!({"urgent": {"type": "noul", "instructions": "Urgent?"}}).to_string(),
    )
    .unwrap();

    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--state-field",
            "text",
            "--id-field",
            "ticket",
            "--endpoint",
            &api.endpoint(),
        ])
        .write_stdin("{\"ticket\":\"T-1\",\"text\":\"the body\"}\n")
        .assert()
        .success();

    let body: serde_json::Value = serde_json::from_str(&api.requests()[0].body).unwrap();
    assert_eq!(body["state"], json!("the body"));
    let first: serde_json::Value = serde_json::from_str(
        String::from_utf8_lossy(&assert.get_output().stdout)
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(first["id"], json!("T-1"));
}

#[test]
fn map_rejects_a_non_json_record_and_suggests_lines() {
    let questions = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        questions.path(),
        json!({"urgent": {"type": "noul", "instructions": "Urgent?"}}).to_string(),
    )
    .unwrap();
    jev_authed()
        .args(["map", "-r", questions.path().to_str().unwrap()])
        .write_stdin("not json\n")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--lines"));
}

// --- Interruption, and the exit codes that are hard to reach ------------------------

/// A questions file with one Noul, for the tests below.
/// A successful response answering `one_noul_question`'s `urgent`.
fn urgent_body() -> String {
    json!({"model": "jev-1.13.0", "answers": {"urgent": {"type": "noul", "noul": 0.92}},
           "usage": {"input_tokens": 312, "output_tokens": 48}})
    .to_string()
}

#[test]
fn a_row_whose_answer_the_api_skipped_names_it() {
    // `noul_body` answers `answer`, not the `urgent` that was asked. A row that simply
    // lacked `urgent` would read, to a script, like a question that was never asked.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let questions = one_noul_question();
    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    let row: serde_json::Value = serde_json::from_str(stdout.lines().next().unwrap()).unwrap();
    assert_eq!(row["missing_answers"], json!(["urgent"]), "{row}");

    let assert = jev_authed()
        .args([
            "ask",
            "-r",
            questions.path().to_str().unwrap(),
            "--state",
            "x",
            "--endpoint",
            &api.endpoint(),
            "--output",
            "json",
        ])
        .assert()
        .success();
    assert_eq!(
        json_stdout(assert.get_output())["missing_answers"],
        json!(["urgent"])
    );
}

fn one_noul_question() -> tempfile::NamedTempFile {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        json!({"urgent": {"type": "noul", "instructions": "Urgent?"}}).to_string(),
    )
    .unwrap();
    file
}

/// Ctrl-C during a batch must stop at a record boundary, leave a readable file, and
/// exit `130`.
///
/// This is the invariant `src/interrupt.rs` exists for, and it is not observable from a
/// unit test: it needs a real process, a real signal, and a real partially written
/// file. Without it, the handler could be removed entirely and every other test would
/// still pass.
#[cfg(unix)]
#[test]
fn an_interrupted_batch_exits_130_and_leaves_a_readable_file() {
    use std::io::Write as _;

    // Slow enough that the batch is certainly still running when the signal lands, and
    // serial so that rows complete one at a time.
    let api = MockApi::slow(
        vec![Reply::ok(noul_body())],
        std::time::Duration::from_millis(150),
    );
    let questions = one_noul_question();
    let out = tempfile::NamedTempFile::new().unwrap();
    let out_path = out.path().to_owned();

    let mut child = jev_spawnable()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--output-file",
            out_path.to_str().unwrap(),
            "-j",
            "1",
            "--endpoint",
            &api.endpoint(),
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn jev map");

    let mut stdin = child.stdin.take().expect("stdin is piped");
    let records: String = (0..40).fold(String::new(), |mut acc, i| {
        use std::fmt::Write as _;
        let _ = writeln!(acc, "\"record {i}\"");
        acc
    });
    std::thread::spawn(move || {
        let _ = stdin.write_all(records.as_bytes());
    });

    // Wait for at least one row to be written, so the interrupt lands mid-batch rather
    // than before any work has happened.
    let progressed = wait_until(std::time::Duration::from_secs(20), || {
        std::fs::read_to_string(&out_path)
            .map(|text| text.lines().count() >= 2)
            .unwrap_or(false)
    });
    assert!(progressed, "the batch never wrote a second row");

    // SIGINT, exactly as Ctrl-C would. Sent with `kill(1)` rather than through `libc`
    // because `unsafe_code` is forbidden workspace-wide, tests included, and a
    // `#[allow]` cannot lift a `forbid`.
    let signalled = std::process::Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("run kill(1)");
    assert!(signalled.success(), "could not signal the child");

    let output = child.wait_with_output().expect("reap the child");
    assert_no_canary(&output);
    assert_eq!(
        output.status.code(),
        Some(130),
        "interrupt must exit 130 (128 + SIGINT); stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Every line that reached the file is a complete JSON document. A partially written
    // line here would make `--resume` skip a record that was never evaluated.
    let written = std::fs::read_to_string(&out_path).unwrap();
    let rows: Vec<&str> = written.lines().collect();
    assert!(!rows.is_empty(), "nothing was written before the interrupt");
    for row in &rows {
        serde_json::from_str::<serde_json::Value>(row)
            .unwrap_or_else(|error| panic!("a truncated row survived ({error}): {row}"));
    }
    assert!(
        rows.len() < 40,
        "the batch ran to completion; the signal did not interrupt anything"
    );
}

/// Ctrl-C during a retry backoff must stop the run, not be swallowed by the wait.
///
/// `main.rs` says the handler is installed "so that an interrupt during a long retry
/// wait or a batch stops cleanly and reports 130". It did not: `SystemClock::sleep` is
/// a bare `thread::sleep`, so a `Retry-After: 5` made the process deaf for five seconds
/// per attempt, and the retry loop then simply started the next attempt. The run
/// completed every remaining attempt and exited `4` -- "the API is unavailable" -- when
/// what happened was that the user asked it to stop.
///
/// Like the batch test above, this needs a real process and a real signal.
#[cfg(unix)]
#[test]
fn an_interrupt_during_a_retry_backoff_exits_130() {
    // A 429 with a long `Retry-After` puts the client into a wait it cannot finish
    // within the test, so anything other than "the signal ended it" shows up as a
    // timeout or as exit 4.
    let api = MockApi::start(vec![
        Reply::status(429, "{}").header("retry-after", "30"),
        Reply::ok(noul_body()),
    ]);

    let child = jev_spawnable()
        .args([
            "noul",
            "q",
            "--state",
            "x",
            "--retries",
            "3",
            "--endpoint",
            &api.endpoint(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn jev noul");

    // Wait until the first attempt has been served, so the process is inside the
    // backoff when the signal lands rather than still starting up.
    let started = wait_until(std::time::Duration::from_secs(20), || api.hits() >= 1);
    assert!(started, "the request never reached the mock API");

    let signalled = std::process::Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("run kill(1)");
    assert!(signalled.success(), "could not signal the child");

    let signalled_at = std::time::Instant::now();
    let output = child.wait_with_output().expect("reap the child");
    let took = signalled_at.elapsed();
    assert_no_canary(&output);
    assert_eq!(
        output.status.code(),
        Some(130),
        "an interrupt during a backoff must exit 130, not report the API's status; \
         stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The exit code alone does not prove the wait was interrupted -- sleeping through
    // the full 30 seconds and *then* noticing would produce the same code.
    //
    // `AGENTS.md` §9 says tests must not depend on wall-clock time, and this one does,
    // so it needs the same written justification `live.rs` carries: responsiveness is
    // the property under test and there is no other way to observe it. The bound is
    // half the `Retry-After`, which is not a measurement -- it separates "ended the
    // wait" (observed: ~0.1s) from "slept through it" (observed: ~30s) with two orders
    // of magnitude of headroom, so a slow or loaded machine cannot flip it.
    assert!(
        took < std::time::Duration::from_secs(15),
        "the process took {took:?} to stop, so it slept through the backoff instead of \
         ending it"
    );
}

/// A write failure is the environment's problem, not a bug report.
///
/// `EX_IOERR` exists so that a full disk does not print "this is a bug in jev, please
/// report it" and send the user to the issue tracker. `/dev/full` is a real device that
/// accepts an `open` and fails every `write` with `ENOSPC`, which is exactly the shape
/// of a disk filling up mid-batch — and the reason this is Linux-only.
#[cfg(target_os = "linux")]
#[test]
fn a_full_disk_exits_74_and_does_not_claim_a_bug() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let questions = one_noul_question();

    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--output-file",
            "/dev/full",
            "--endpoint",
            &api.endpoint(),
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .code(74);

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        !stderr.contains("bug in jev"),
        "a write failure told the user to file a bug:\n{stderr}"
    );
    assert!(
        stderr.contains("cannot write the output file"),
        "the message does not say what failed:\n{stderr}"
    );
    assert_no_canary(assert.get_output());
}

/// And a path that cannot be opened at all is the user's mistake, reported in English.
///
/// `ErrorKind`'s own wording is "entity not found", which tells a user nothing. This is
/// exit `2`, not `74`: nothing was written, and the fix is to type a different path.
#[test]
fn an_unopenable_output_path_is_a_usage_error_in_plain_english() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let questions = one_noul_question();
    let directory = tempfile::tempdir().unwrap();
    let unopenable = directory.path().join("no-such-directory").join("out.jsonl");

    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--output-file",
            unopenable.to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .code(2);

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        stderr.contains("no such file or directory"),
        "Rust jargon reached the user:\n{stderr}"
    );
    assert!(!stderr.contains("entity not found"), "{stderr}");
}

// --- Regressions ---------------------------------------------------------------------

/// Colour follows *stdout*, not stderr.
///
/// `jev … > file` used to write ANSI escapes into the file, because the decision was
/// made from stderr's terminal-ness. Neither stream is a terminal under a test harness,
/// so this asserts the positive form: `--color always` colours stderr but must still
/// leave a redirected stdout clean of escapes it did not ask for.
#[test]
fn a_redirected_stdout_never_receives_ansi_escapes() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let assert = jev_authed()
        .args([
            "noul",
            "Is this urgent?",
            "--endpoint",
            &api.endpoint(),
            "--output",
            "json",
        ])
        .write_stdin("the server is down\n")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(
        !stdout.contains('\u{1b}'),
        "an ANSI escape reached a non-terminal stdout:\n{stdout:?}"
    );
    // And what landed there is still exactly one JSON document.
    let _ = json_stdout(assert.get_output());
}

/// `--value` prints one scalar, so a response carrying several answers has no single
/// value to print. Returning the first one silently would be worse than refusing.
#[test]
fn value_refuses_a_response_with_more_than_one_answer() {
    let body = r#"{"model":"jev-1.13.0","answers":{
        "a":{"type":"noul","noul":0.9},
        "b":{"type":"noul","noul":0.1}
    },"usage":{}}"#;
    let api = MockApi::start(vec![Reply::ok(body)]);
    let questions = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        questions.path(),
        json!({
            "a": {"type": "noul", "instructions": "A?"},
            "b": {"type": "noul", "instructions": "B?"}
        })
        .to_string(),
    )
    .unwrap();

    let assert = jev_authed()
        .args([
            "ask",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "--value",
        ])
        .write_stdin("some state\n")
        .assert()
        .failure();

    assert_ne!(
        assert.get_output().status.code(),
        Some(0),
        "two answers were reduced to one scalar"
    );
    assert_no_canary(assert.get_output());
}

/// `--dry-run` means "show me the request you would send". A command that sends no
/// request has no such thing to show, and accepting the flag there would imply it does.
#[test]
fn dry_run_is_refused_by_the_commands_that_send_nothing() {
    for args in [
        vec!["config", "path"],
        vec!["auth", "status"],
        vec!["doctor"],
        vec!["completions", "bash"],
    ] {
        let assert = jev().args(&args).arg("--dry-run").assert().code(2);
        let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
        assert!(
            stderr.contains("--dry-run"),
            "`jev {}` did not explain why --dry-run is refused:\n{stderr}",
            args.join(" ")
        );
    }
}

/// A redirect is the endpoint's misconfiguration, not the user's mistake.
///
/// Redirects are never followed (`docs/threat-model.md`), so a 3xx has to be classified
/// somewhere. It belongs with the server's problems — exit `4` — and must not be
/// reported as a usage error the user could fix by typing something different.
#[test]
fn a_redirect_is_classified_as_the_servers_problem() {
    let api = MockApi::start(vec![
        Reply::status(302, "{}").header("Location", "https://elsewhere.example/v1/systemone"),
    ]);

    let assert = jev_authed()
        .args([
            "noul",
            "Is this urgent?",
            "--endpoint",
            &api.endpoint(),
            "--retries",
            "0",
        ])
        .write_stdin("state\n")
        .assert()
        .code(4);

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        !stderr.contains("elsewhere.example"),
        "the redirect target was followed or echoed:\n{stderr}"
    );
    assert_no_canary(assert.get_output());
}

// --- The credential canary -----------------------------------------------------------

/// The canary in `scripts/credential-canary.sh` exercises CLI commands with fake keys
/// and fails if a key appears in a stream or output file. Its command list is
/// hand-maintained and can silently stop covering a newly added subcommand.
///
/// This test closes that gap: it derives the real command list from `jev --help`,
/// **including nested subcommands**, and asserts the script lists every one.
///
/// Matching is by set membership, not by substring. A substring test passes for the
/// wrong reason all the time — `"conf"` is contained in `"config"`, and `"auth"` is
/// contained in `"auth login"` — so it would report full coverage for a list that
/// exercises none of the leaves where the credential is actually handled.
#[test]
fn credential_canary_script_covers_every_subcommand() {
    let script_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/credential-canary.sh");
    let script = std::fs::read_to_string(&script_path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", script_path.display()));

    let covered = canary_command_list(&script);
    assert!(
        !covered.is_empty(),
        "parsed no commands out of the canary's `for args in` list"
    );

    let mut missing = Vec::new();
    for command in every_command() {
        if !covered.contains(&command) {
            missing.push(command);
        }
    }

    assert!(
        missing.is_empty(),
        "these commands are not covered by the credential canary in \
         scripts/credential-canary.sh; add them to the `for args in` list:\n  {}\n\
         the list currently covers:\n  {}",
        missing.join("\n  "),
        covered.iter().cloned().collect::<Vec<_>>().join("\n  ")
    );
}

/// The quoted words of the canary's `for args in ...; do` loop, as a set.
///
/// The loop is written across several lines with trailing backslashes, so the list
/// runs from `for args in` to the `; do` that closes it rather than to end of line.
fn canary_command_list(workflow: &str) -> std::collections::BTreeSet<String> {
    let after = workflow
        .split_once("for args in")
        .expect("the canary step's `for args in` loop has been renamed or removed")
        .1;
    let list = after
        .split_once("; do")
        .expect("the `for args in` loop has no `; do`")
        .0;

    list.split('"')
        .skip(1)
        .step_by(2)
        .map(|word| word.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|word| !word.is_empty() && !word.starts_with('-'))
        .collect()
}

/// Every subcommand path `jev` accepts, one level of nesting deep, derived from
/// `--help` so that adding a command to `cli.rs` is enough to make this fail.
fn every_command() -> Vec<String> {
    let mut commands = Vec::new();
    for top in subcommands_of(&[]) {
        let nested = subcommands_of(&[top.as_str()]);
        commands.push(top.clone());
        for leaf in nested {
            commands.push(format!("{top} {leaf}"));
        }
    }
    assert!(
        commands.iter().any(|c| c == "auth login"),
        "no nested subcommands were found; the --help format changed and this test \
         would now pass vacuously"
    );
    commands
}

fn subcommands_of(path: &[&str]) -> Vec<String> {
    let mut command = jev();
    for segment in path {
        command.arg(segment);
    }
    let assertion = command.arg("--help").assert().success();
    let help = String::from_utf8_lossy(&assertion.get_output().stdout).into_owned();

    help.split_once("Commands:")
        .map(|(_, rest)| rest)
        .unwrap_or_default()
        .lines()
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .filter(|word| !word.starts_with('-'))
        // `clap` lists `help` under every command; the canary does not need it, and it
        // takes no credential.
        .filter(|word| *word != "help")
        .map(str::to_owned)
        .collect()
}

// --- Golden key sets ----------------------------------------------------------------
//
// `docs/cli-contract.md` promises that a field may be added but never removed or
// renamed. Nothing enforced that: the schema-identifier tests pin the identifiers, not
// the fields they name, so a rename of `credentials.effective_source` or
// `summary.complete` passed the entire suite. These assert the exact top-level key set
// of each document.
//
// Adding a field is meant to be easy -- it is a compatible change -- so a failure here
// is a one-line edit *and* a prompt to update `docs/output-schema.md`. Removing or
// renaming one is meant to be hard, which is the point.

/// Asserts a document's top-level keys are exactly `expected`.
#[track_caller]
fn assert_keys(document: &serde_json::Value, expected: &[&str]) {
    let mut found: Vec<&str> = document
        .as_object()
        .unwrap_or_else(|| panic!("not an object: {document}"))
        .keys()
        .map(String::as_str)
        .collect();
    found.sort_unstable();
    let mut want = expected.to_vec();
    want.sort_unstable();
    assert_eq!(
        found, want,
        "the document's field set changed; removing or renaming a field is a breaking \
         change (docs/cli-contract.md), and adding one needs a docs update"
    );
}

#[test]
fn the_evaluation_document_has_its_documented_fields() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let assert = jev_authed()
        .args([
            "noul",
            "q",
            "--state",
            "x",
            "--output",
            "json",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();
    assert_keys(
        &json_stdout(assert.get_output()),
        &[
            "schema",
            "model",
            "model_requested",
            "endpoint",
            "answers",
            "usage",
            "request_id",
        ],
    );
}

#[test]
fn the_doctor_document_has_its_documented_fields() {
    let assert = jev_authed()
        .args(["doctor", "--output", "json"])
        .assert()
        .success();
    let document = json_stdout(assert.get_output());
    assert_keys(
        &document,
        &[
            "schema",
            "version",
            "platform",
            "endpoint",
            "model",
            "config",
            "credentials",
            "limits",
            "live",
        ],
    );
    assert_keys(
        &document["credentials"],
        &[
            "effective_source",
            "sources",
            "store",
            "store_error",
            "error",
        ],
    );
}

#[test]
fn the_auth_status_document_has_its_documented_fields() {
    let assert = jev_authed()
        .args(["auth", "status", "--output", "json"])
        .assert()
        .success();
    assert_keys(
        &json_stdout(assert.get_output()),
        &[
            "schema",
            "action",
            "endpoint",
            "official_endpoint",
            "store",
            "store_error",
            "error",
            "effective_source",
            "sources",
        ],
    );
}

#[test]
fn the_dry_run_document_has_its_documented_fields() {
    let assert = jev_authed()
        .args(["noul", "q", "--state", "x", "--dry-run", "--output", "json"])
        .assert()
        .success();
    assert_keys(
        &json_stdout(assert.get_output()),
        &[
            "schema",
            "method",
            "url",
            "headers",
            "body",
            "body_bytes",
            "credential",
            "gate",
            "sent",
        ],
    );
}

#[test]
fn a_dry_run_shows_the_gate_and_says_value_has_nothing_to_print() {
    // `--require` is parsed during a dry run but was not shown, so a user could not
    // confirm it had been read the way they meant. And `--value` -- "one scalar and a
    // newline, nothing else" -- silently produced a whole JSON document instead.
    let assert = jev_authed()
        .args([
            "noul",
            "q",
            "--state",
            "x",
            "--require",
            "answer.noul > 0.9",
            "--value",
            "--dry-run",
        ])
        .assert()
        .success();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(
        stderr.contains("--value has nothing to print"),
        "the user was not told --value does not apply: {stderr}"
    );

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    let document: serde_json::Value = serde_json::from_str(&stdout).expect("a dry-run document");
    assert_eq!(document["gate"]["expression"], json!("answer.noul > 0.9"));
}

#[test]
fn the_map_documents_have_their_documented_fields() {
    // The reply answers the question the request asks, so the row is a complete one;
    // `a_row_whose_answer_the_api_skipped_names_it` covers the other shape.
    let api = MockApi::start(vec![Reply::ok(urgent_body())]);
    let questions = one_noul_question();
    let out = tempfile::NamedTempFile::new().unwrap();

    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--output-file",
            out.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .success();

    assert_keys(
        &json_stdout(assert.get_output()),
        &[
            "schema",
            "total",
            "resumed",
            "evaluated",
            "succeeded",
            "failed",
            "complete",
            "stopped_early",
            "interrupted",
            "gate",
        ],
    );

    let row: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(out.path()).unwrap().trim()).unwrap();
    assert_keys(
        &row,
        &[
            "schema",
            "index",
            "id",
            "state_digest",
            "request_digest",
            "ok",
            "model",
            "answers",
            "usage",
            "attempts",
            "request_id",
            "gate",
        ],
    );
}

/// The canary must exercise every credential source, not just the two most obvious.
///
/// This is a grep, and therefore weaker than it looks: it catches a source being
/// dropped or renamed out of the script, not a source being set but never resolved.
/// The script itself guards the second case, by unsetting the sources that shadow
/// each one under test -- resolution stops at the first populated source, so without
/// that the extra canaries would sit unused and the check could not fail for them.
///
/// `TYPESAFE_API_KEY` and both `*_KEY_FILE` variables were never set, so three of the
/// six sources -- including the whole key-file reading path and its error variants --
/// were outside the check that exists to prove no credential reaches an output stream.
#[test]
fn credential_canary_script_covers_every_credential_source() {
    let script_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/credential-canary.sh");
    let script = std::fs::read_to_string(&script_path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", script_path.display()));

    // The environment variables named in `docs/cli-contract.md` as credential sources.
    // The OS keychain is omitted: a local canary must not change a real secure store.
    for variable in [
        "JEV_API_KEY",
        "JEV_API_KEY_FILE",
        "TYPESAFE_API_KEY",
        "JEV_CUSTOM_API_KEY",
        "JEV_CUSTOM_API_KEY_FILE",
    ] {
        assert!(
            script.contains(variable),
            "{variable} is a credential source but the canary in \
             scripts/credential-canary.sh never sets it, so nothing proves it \
             leak-free"
        );
    }

    // And the files `jev` writes are scanned, not only the two streams.
    assert!(
        script.contains("check_files"),
        "the canary greps stdout and stderr only; a credential written into \
         `jev map --output-file` or the configuration file would pass unnoticed"
    );

    let verify_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/verify.sh");
    let verify = std::fs::read_to_string(&verify_path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", verify_path.display()));
    assert!(
        verify.contains("run \"credential canary\" \"\" scripts/credential-canary.sh"),
        "the local push gate must run the credential canary"
    );
}

#[test]
fn the_api_request_id_reaches_the_output() {
    // `x-typesafe-request-id` is documented at
    // https://docs.typesafe.ai/sdk/python/api/exceptions.md, and the official SDK
    // appends it to every API error message. `jev` discarded it, so a user whose batch
    // row failed had nothing to hand TypeSafe support -- and it cannot be recovered
    // after the process exits.
    let api = MockApi::start(vec![
        Reply::ok(noul_body()).header("x-typesafe-request-id", "req_abc123"),
    ]);

    let assert = jev_authed()
        .args([
            "noul",
            "q",
            "--state",
            "x",
            "--output",
            "json",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();
    assert_eq!(
        json_stdout(assert.get_output())["request_id"],
        json!("req_abc123")
    );

    // And on the path that needs it most: a row that failed.
    let failing = MockApi::start(vec![
        Reply::status(422, r#"{"detail":"nope"}"#).header("x-typesafe-request-id", "req_fail99"),
    ]);
    let questions = one_noul_question();
    let out = tempfile::NamedTempFile::new().unwrap();
    jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--output-file",
            out.path().to_str().unwrap(),
            "--retries",
            "0",
            "--endpoint",
            &failing.endpoint(),
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .code(5);
    let row: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(out.path()).unwrap().trim()).unwrap();
    assert_eq!(row["ok"], json!(false));
    assert_eq!(row["request_id"], json!("req_fail99"));
}

#[test]
fn a_response_without_a_request_id_reports_null_rather_than_failing() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let assert = jev_authed()
        .args([
            "noul",
            "q",
            "--state",
            "x",
            "--output",
            "json",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();
    assert_eq!(
        json_stdout(assert.get_output())["request_id"],
        serde_json::Value::Null
    );
}

#[test]
fn the_models_document_has_its_documented_fields() {
    let api = MockApi::start(vec![Reply::ok(
        json!({"models": [
            {"name": "jev-latest", "description": "…", "release_date": "2026-09-15"}
        ]})
        .to_string(),
    )]);
    let assert = jev_authed()
        .args(["models", "--output", "json", "--endpoint", &api.endpoint()])
        .assert()
        .success();
    assert_keys(
        &json_stdout(assert.get_output()),
        &["schema", "endpoint", "models"],
    );
}

#[test]
fn the_config_documents_have_their_documented_fields() {
    // One document per action, and the action decides the shape. `docs/output-schema.md`
    // tables all five; nothing asserted any of them.
    let directory = tempfile::tempdir().unwrap();
    let config_dir = directory.path().to_str().unwrap();

    let expected: [(&[&str], &[&str]); 5] = [
        (&["config", "path"], &["schema", "action", "path"]),
        (
            &["config", "list"],
            &["schema", "action", "path", "exists", "settings"],
        ),
        (
            &["config", "set", "model", "jev-latest"],
            &["schema", "action", "key", "value", "path"],
        ),
        (
            &["config", "get", "model"],
            &["schema", "action", "key", "set", "value"],
        ),
        (
            &["config", "unset", "model"],
            &["schema", "action", "key", "path"],
        ),
    ];

    for (args, keys) in expected {
        let assert = jev()
            .env("JEV_CONFIG_DIR", config_dir)
            .args(args)
            .args(["--output", "json"])
            .assert()
            .success();
        assert_keys(&json_stdout(assert.get_output()), keys);
    }
}

#[test]
fn a_failed_map_row_has_its_documented_fields() {
    // The failure row is a different shape from the success row -- no `model`, no
    // `answers`, no `usage`, an `error` object instead -- and only the success shape was
    // pinned. A consumer branching on `ok` depends on both.
    let api = MockApi::start(vec![Reply::status(422, r#"{"detail":"nope"}"#)]);
    let questions = one_noul_question();
    let out = tempfile::NamedTempFile::new().unwrap();

    jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--output-file",
            out.path().to_str().unwrap(),
            "--retries",
            "0",
            "--endpoint",
            &api.endpoint(),
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .code(5);

    let row: serde_json::Value =
        serde_json::from_str(std::fs::read_to_string(out.path()).unwrap().trim()).unwrap();
    assert_keys(
        &row,
        &[
            "schema",
            "index",
            "id",
            "state_digest",
            "request_digest",
            "ok",
            "attempts",
            "request_id",
            "error",
            "gate",
        ],
    );
    assert_keys(&row["error"], &["kind", "message"]);
    assert_eq!(row["ok"], json!(false));
}

// --- map --require and --review-file -----------------------------------------------
//
// `--require` in `jev map` *routes*; it does not gate. The exit code keeps reporting
// whether the API answered, which is the distinction `docs/cli-contract.md` calls the
// one that matters most. These tests pin that, and pin that no row is ever dropped.

/// A Noul response body with a chosen probability, under the question id `answer`.
fn noul_body_with(probability: f64) -> String {
    json!({
        "model": "jev-1.13.0",
        "answers": {"answer": {"type": "noul", "noul": probability}},
        "usage": {"input_tokens": 1, "output_tokens": 1},
    })
    .to_string()
}

/// A questions file asking one Noul, written to a temporary path.
fn urgent_questions() -> tempfile::NamedTempFile {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        json!({"answer": {"type": "noul", "instructions": "Urgent?"}}).to_string(),
    )
    .unwrap();
    file
}

#[test]
fn map_require_classifies_every_row_and_leaves_the_exit_code_alone() {
    // 0.92 passes, 0.10 does not. Both rows stay in the stream, and the run still
    // exits 0: the API answered both times, which is what the exit code reports.
    let api = MockApi::start(vec![
        Reply::ok(noul_body_with(0.92)),
        Reply::ok(noul_body_with(0.10)),
    ]);
    let questions = urgent_questions();

    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "-j",
            "1",
            "--require",
            "answer.noul > 0.9",
        ])
        .write_stdin("\"a\"\n\"b\"\n")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let lines: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 3, "two rows plus a summary; nothing dropped");

    assert_eq!(lines[0]["gate"]["outcome"], json!("passed"));
    assert_eq!(lines[0]["gate"]["expression"], json!("answer.noul > 0.9"));
    assert_eq!(lines[0]["gate"]["reason"], json!(null));
    assert_eq!(lines[1]["gate"]["outcome"], json!("failed"));

    let summary = &lines[2];
    assert_eq!(summary["gate"]["passed"], json!(1));
    assert_eq!(summary["gate"]["failed"], json!(1));
    assert_eq!(summary["gate"]["unevaluable"], json!(0));
    assert_eq!(summary["gate"]["expression"], json!("answer.noul > 0.9"));
}

#[test]
fn map_gate_is_null_when_no_require_was_given() {
    // Present as a key, null as a value: a consumer filtering on `.gate.outcome` gets a
    // missing verdict rather than a missing field.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let questions = urgent_questions();

    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let lines: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines[0]["gate"], json!(null));
    assert_eq!(lines[1]["gate"], json!(null));
}

#[test]
fn map_review_file_diverts_the_rows_that_did_not_pass() {
    let api = MockApi::start(vec![
        Reply::ok(noul_body_with(0.92)),
        Reply::ok(noul_body_with(0.10)),
        Reply::ok(noul_body_with(0.99)),
    ]);
    let questions = urgent_questions();
    let out = tempfile::NamedTempFile::new().unwrap();
    let review = tempfile::NamedTempFile::new().unwrap();

    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "-j",
            "1",
            "--require",
            "answer.noul > 0.9",
            "--output-file",
            out.path().to_str().unwrap(),
            "--review-file",
            review.path().to_str().unwrap(),
        ])
        .write_stdin("\"a\"\n\"b\"\n\"c\"\n")
        .assert()
        .success();

    let read = |path: &std::path::Path| -> Vec<serde_json::Value> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    };

    let passed = read(out.path());
    let diverted = read(review.path());
    assert_eq!(passed.len(), 2, "the two rows above the threshold");
    assert_eq!(diverted.len(), 1, "the one below it");
    assert_eq!(diverted[0]["index"], json!(1));
    assert_eq!(diverted[0]["gate"]["outcome"], json!("failed"));
    for row in &passed {
        assert_eq!(row["gate"]["outcome"], json!("passed"));
    }

    // Every record is accounted for in exactly one of the two files.
    assert_eq!(passed.len() + diverted.len(), 3);

    // The summary is on stdout, in neither file.
    let summary = json_stdout(assert.get_output());
    assert_eq!(summary["schema"], json!("jev.map.summary/v1"));
    assert_eq!(summary["succeeded"], json!(3));
    assert_eq!(summary["gate"]["passed"], json!(2));
    assert_eq!(summary["gate"]["failed"], json!(1));
}

#[test]
fn map_an_unevaluable_expression_is_diverted_not_treated_as_a_pass() {
    // A Noul carries no confidence: the API returns none and `jev` invents none. The
    // gate cannot be evaluated, and an unevaluable gate must never read as a pass --
    // the same rule `exit.rs` encodes by keeping exit 6 apart from exit 1.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let questions = urgent_questions();
    let review = tempfile::NamedTempFile::new().unwrap();

    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "--require",
            "answer.confidence > 0.5",
            "--review-file",
            review.path().to_str().unwrap(),
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .success();

    let diverted: Vec<serde_json::Value> = std::fs::read_to_string(review.path())
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(diverted.len(), 1);
    assert_eq!(diverted[0]["gate"]["outcome"], json!("unevaluable"));
    assert!(
        diverted[0]["gate"]["reason"].as_str().is_some(),
        "an unevaluable verdict says why: {}",
        diverted[0]["gate"]
    );

    let summary = json_stdout(assert.get_output());
    assert_eq!(summary["gate"]["unevaluable"], json!(1));
    assert_eq!(summary["gate"]["passed"], json!(0));
}

#[test]
fn map_without_a_review_file_a_failing_row_still_reaches_stdout() {
    // Nothing is ever discarded. semdecide drops the records inside its uncertainty
    // margin and reports the fact only through an exit code; a row the user paid for
    // and cannot see is worse than no feature at all.
    let api = MockApi::start(vec![Reply::ok(noul_body_with(0.10))]);
    let questions = urgent_questions();

    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "--require",
            "answer.noul > 0.9",
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let lines: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2, "the row is still there, plus the summary");
    assert_eq!(lines[0]["gate"]["outcome"], json!("failed"));
}

#[test]
fn map_a_failed_row_is_never_diverted_to_the_review_file() {
    // "The API did not answer" and "the API answered and the answer needs a look" are
    // different problems for different people.
    let api = MockApi::start(vec![Reply::status(400, r#"{"detail":"bad"}"#)]);
    let questions = urgent_questions();
    let review = tempfile::NamedTempFile::new().unwrap();

    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "--retries",
            "0",
            "--require",
            "answer.noul > 0.9",
            "--review-file",
            review.path().to_str().unwrap(),
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .code(5);

    assert_eq!(
        std::fs::read_to_string(review.path()).unwrap().trim(),
        "",
        "a transport failure is not a semantic review item"
    );
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let first: serde_json::Value = serde_json::from_str(stdout.lines().next().unwrap()).unwrap();
    assert_eq!(first["ok"], json!(false));
    assert_eq!(first["gate"]["outcome"], json!("not-evaluated"));
}

#[test]
fn map_refuses_a_review_file_that_is_also_the_output_file() {
    let questions = urgent_questions();
    let out = tempfile::NamedTempFile::new().unwrap();

    jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--require",
            "answer.noul > 0.9",
            "--output-file",
            out.path().to_str().unwrap(),
            "--review-file",
            out.path().to_str().unwrap(),
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("same path"));
}

#[test]
fn map_rejects_a_malformed_require_before_sending_anything() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let questions = urgent_questions();

    jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "--require",
            "answer.noul >",
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .code(2);

    assert_eq!(api.hits(), 0, "a bad expression costs nothing");
}

#[test]
fn map_review_file_requires_require() {
    let questions = urgent_questions();
    let review = tempfile::NamedTempFile::new().unwrap();

    jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--review-file",
            review.path().to_str().unwrap(),
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .code(2);
}

#[test]
fn map_resume_counts_a_reviewed_row_as_done() {
    // A diverted row is as done as one that passed. Reading only the output file would
    // re-evaluate -- and re-bill -- every reviewed record on every resume.
    let api = MockApi::start(vec![
        Reply::ok(noul_body_with(0.92)),
        Reply::ok(noul_body_with(0.10)),
    ]);
    let questions = urgent_questions();
    let out = tempfile::NamedTempFile::new().unwrap();
    let review = tempfile::NamedTempFile::new().unwrap();

    let args = [
        "map".to_owned(),
        "-r".to_owned(),
        questions.path().to_str().unwrap().to_owned(),
        "--endpoint".to_owned(),
        api.endpoint(),
        "-j".to_owned(),
        "1".to_owned(),
        "--require".to_owned(),
        "answer.noul > 0.9".to_owned(),
        "--output-file".to_owned(),
        out.path().to_str().unwrap().to_owned(),
        "--review-file".to_owned(),
        review.path().to_str().unwrap().to_owned(),
    ];

    jev_authed()
        .args(&args)
        .write_stdin("\"a\"\n\"b\"\n")
        .assert()
        .success();
    let after_first = api.hits();
    assert_eq!(after_first, 2);

    let mut resumed = args.to_vec();
    resumed.push("--resume".to_owned());
    let assert = jev_authed()
        .args(&resumed)
        .write_stdin("\"a\"\n\"b\"\n")
        .assert()
        .success();

    assert_eq!(api.hits(), after_first, "nothing was re-sent, or re-billed");
    let summary = json_stdout(assert.get_output());
    assert_eq!(summary["resumed"], json!(2));
    assert_eq!(summary["evaluated"], json!(0));
    assert_eq!(summary["complete"], json!(2));
}

#[cfg(unix)]
#[test]
fn map_creates_the_review_file_privately() {
    // The review file holds the same answers about the user's state as the output file,
    // and gets the same 0600 the output file does.
    use std::os::unix::fs::PermissionsExt as _;

    let api = MockApi::start(vec![Reply::ok(noul_body_with(0.10))]);
    let questions = urgent_questions();
    let directory = tempfile::tempdir().unwrap();
    let review = directory.path().join("review.jsonl");

    jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "--require",
            "answer.noul > 0.9",
            "--review-file",
            review.to_str().unwrap(),
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .success();

    let mode = std::fs::metadata(&review).unwrap().permissions().mode();
    assert_eq!(
        mode & 0o777,
        0o600,
        "review file mode was {:o}",
        mode & 0o777
    );
}

// --- map: defects found by the adversarial audit -----------------------------------

#[test]
fn map_resume_refuses_a_changed_question_set() {
    // `state_digest` catches a changed input. Nothing caught a changed *question set*,
    // so resuming after editing the prompt produced a file whose early rows answered one
    // question and whose later rows answered another, reported the batch complete, and
    // exited 0.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let first = urgent_questions();
    let second = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        second.path(),
        json!({"answer": {"type": "noul", "instructions": "Something else entirely?"}}).to_string(),
    )
    .unwrap();
    let out = tempfile::NamedTempFile::new().unwrap();

    let run = |questions: &std::path::Path, resume: bool| {
        let mut command = jev_authed();
        command.args([
            "map",
            "-r",
            questions.to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "-j",
            "1",
            "--output-file",
            out.path().to_str().unwrap(),
        ]);
        if resume {
            command.arg("--resume");
        }
        command.write_stdin("\"a\"\n\"b\"\n").assert()
    };

    run(first.path(), false).success();

    run(second.path(), true)
        .code(2)
        .stderr(predicate::str::contains("different request"));

    // The same question set still resumes.
    run(first.path(), true).success();
}

#[test]
fn map_resume_refuses_a_changed_model() {
    // The same question answered by a different model is a different result.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let questions = urgent_questions();
    let out = tempfile::NamedTempFile::new().unwrap();

    let run = |model: &str, resume: bool| {
        let mut command = jev_authed();
        command.args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "--model",
            model,
            "--output-file",
            out.path().to_str().unwrap(),
        ]);
        if resume {
            command.arg("--resume");
        }
        command.write_stdin("\"a\"\n").assert()
    };

    run("jev-latest", false).success();
    run("jev-1.13", true)
        .code(2)
        .stderr(predicate::str::contains("different request"));
}

#[test]
fn map_stops_and_exits_three_when_the_credential_is_rejected() {
    // A rejected credential is per-credential, not per-record. Carrying on sent one
    // doomed request per remaining row and then reported a partial batch, so a CI job
    // branching on exit 3 to re-authenticate never saw it.
    let api = MockApi::start(vec![Reply::status(401, r#"{"detail":"nope"}"#)]);
    let questions = urgent_questions();

    jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "-j",
            "1",
            "--retries",
            "0",
        ])
        .write_stdin("\"a\"\n\"b\"\n\"c\"\n\"d\"\n\"e\"\n")
        .assert()
        .code(3);

    assert_eq!(
        api.hits(),
        1,
        "the batch stopped at the first rejection instead of sending one per record"
    );
}

#[test]
fn map_refuses_an_output_file_that_already_has_rows() {
    // Appending without being asked to resume silently doubled the file: running the
    // same command twice left every record in it twice, with no warning.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let questions = urgent_questions();
    let out = tempfile::NamedTempFile::new().unwrap();

    let run = || {
        jev_authed()
            .args([
                "map",
                "-r",
                questions.path().to_str().unwrap(),
                "--endpoint",
                &api.endpoint(),
                "--output-file",
                out.path().to_str().unwrap(),
            ])
            .write_stdin("\"a\"\n")
            .assert()
    };

    run().success();
    run().code(2).stderr(predicate::str::contains("--resume"));

    let rows = std::fs::read_to_string(out.path()).unwrap();
    assert_eq!(rows.lines().filter(|l| !l.is_empty()).count(), 1);
}

#[test]
fn map_refuses_to_write_its_output_over_its_input() {
    // The input is read fully before the sink opens, so this appended result rows onto
    // the input file and left it silently no longer valid as input.
    let questions = urgent_questions();
    let directory = tempfile::tempdir().unwrap();
    let both = directory.path().join("records.jsonl");
    std::fs::write(&both, "\"a\"\n").unwrap();

    jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "-i",
            both.to_str().unwrap(),
            "--output-file",
            both.to_str().unwrap(),
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("same path"));

    assert_eq!(std::fs::read_to_string(&both).unwrap(), "\"a\"\n");
}

// --- request files: defects found by the adversarial audit --------------------------

/// Runs `--dry-run` over a request file and returns the body that would be sent.
fn dry_run_body(document: &str) -> std::process::Output {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), document).unwrap();
    jev_authed()
        .args([
            "ask",
            "-r",
            file.path().to_str().unwrap(),
            "--state",
            "x",
            "--dry-run",
            "--output",
            "json",
        ])
        .assert()
        .get_output()
        .clone()
}

#[test]
fn a_duplicate_field_inside_a_question_is_refused() {
    // The top level and the question-id level both reject duplicates; one level deeper
    // the policy silently reversed, so this sent a Choice for a document that says
    // `noul` first, with no diagnostic.
    let output = dry_run_body(
        r#"{"questions":{"a":{"type":"noul","type":"choice","instructions":"?",
            "criteria":{"x":null,"y":null}}}}"#,
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("duplicate field"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_duplicate_choice_option_name_is_refused() {
    let output = dry_run_body(
        r#"{"questions":{"a":{"type":"choice","instructions":"?",
            "criteria":{"x":"first","x":"second","y":null}}}}"#,
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("duplicate option name"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_choice_reaches_the_api_in_the_order_it_was_written() {
    // Option order is the part of a question a model can be sensitive to, so it is
    // asserted on the *bytes that reach the socket* rather than on any rendering of
    // them: both `serde_json::Value` and `serde_json::Map` are sorted, so a test that
    // decodes the body before looking at it cannot tell the difference.
    let api = MockApi::start(vec![Reply::ok(
        json!({
            "model": "jev-1.13.0",
            "answers": {"team": {"type": "choice", "choice": "zebra", "confidence": 0.9,
                "probabilities": {"zebra": 0.9, "apple": 0.05, "mango": 0.05}}},
            "usage": {"input_tokens": 1, "output_tokens": 1},
        })
        .to_string(),
    )]);
    let questions = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        questions.path(),
        r#"{"team":{"type":"choice","instructions":"?",
            "criteria":{"zebra":null,"apple":null,"mango":null}}}"#,
    )
    .unwrap();

    jev_authed()
        .args([
            "ask",
            "-r",
            questions.path().to_str().unwrap(),
            "--state",
            "x",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .success();

    let sent = &api.requests()[0].body;
    let zebra = sent.find("zebra").expect("zebra is in the body");
    let apple = sent.find("apple").expect("apple is in the body");
    let mango = sent.find("mango").expect("mango is in the body");
    assert!(
        zebra < apple && apple < mango,
        "options were reordered on the wire: {sent}"
    );
}

#[test]
fn a_malformed_question_is_reported_instead_of_a_nonsense_error_about_state() {
    // The "is this a full document?" test required *every* question to have a `type`,
    // so one malformed question fell through to the bare-questions branch and the first
    // key it tripped over was reported: "question `state` must be an object". The real
    // fault is question `b`, and `state` was completely correct.
    let output = dry_run_body(
        r#"{"state":"x","questions":{"a":{"type":"noul","instructions":"?"},
            "b":{"instructions":"?"}}}"#,
    );
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains('b'), "{stderr}");
    assert!(
        !stderr.contains("question `state`"),
        "still blaming the wrong key: {stderr}"
    );
}

#[test]
fn a_bare_questions_map_with_a_question_called_questions_still_parses() {
    // The guard the heuristic exists for: a file that is only a questions map, one of
    // whose questions happens to be called `questions`.
    let output = dry_run_body(
        r#"{"questions":{"type":"noul","instructions":"Is it urgent?"},
            "other":{"type":"noul","instructions":"Is it new?"}}"#,
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(document["body"]["questions"]["questions"].is_object());
    assert!(document["body"]["questions"]["other"].is_object());
}

#[test]
fn gate_equality_on_a_number_is_exact_and_is_the_complement_of_not_equal() {
    // `f64::EPSILON` as an absolute tolerance inverted across the range: near zero it
    // was enormous in relative terms, so `x == 0` and `x > 0` both held for 1e-17.
    let api = MockApi::start(vec![Reply::ok(
        json!({
            "model": "jev-1.13.0",
            "answers": {"answer": {"type": "noul", "noul": 1e-17}},
            "usage": {"input_tokens": 1, "output_tokens": 1},
        })
        .to_string(),
    )]);

    let check = |expression: &str| {
        jev_authed()
            .args([
                "noul",
                "Urgent?",
                "--state",
                "x",
                "--endpoint",
                &api.endpoint(),
                "--require",
                expression,
            ])
            .assert()
            .get_output()
            .status
            .code()
    };

    assert_eq!(check("answer.noul == 0"), Some(1), "it is not exactly zero");
    assert_eq!(check("answer.noul > 0"), Some(0));
    assert_eq!(check("answer.noul != 0"), Some(0), "the complement of ==");
}

// --- the committed examples ---------------------------------------------------------

/// The request files in `examples/requests/`.
fn example_requests() -> Vec<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("examples/requests");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&root)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", root.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no examples found in {}", root.display());
    files
}

#[test]
fn every_committed_example_request_is_accepted() {
    // A cookbook whose files do not parse is worse than no cookbook. `--dry-run` is the
    // whole check: it parses and validates the document and builds the exact body,
    // without a network or a credential.
    for path in example_requests() {
        let output = jev_authed()
            .args([
                "ask",
                "-r",
                path.to_str().unwrap(),
                "--state",
                "example state",
                "--dry-run",
                "--output",
                "json",
            ])
            .assert()
            .get_output()
            .clone();
        assert_eq!(
            output.status.code(),
            Some(0),
            "{} was refused: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
        let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            document["body"]["questions"]
                .as_object()
                .is_some_and(|questions| !questions.is_empty()),
            "{} built a request with no questions",
            path.display()
        );
    }
}

#[test]
fn every_committed_example_request_works_with_map_too() {
    // The architectural promise: one request-file format across `ask`, `map`, and
    // `--dry-run`. A file that only works with one of them would break it silently.
    for path in example_requests() {
        let output = jev_authed()
            .args([
                "map",
                "-r",
                path.to_str().unwrap(),
                "--state-field",
                "text",
                "--dry-run",
                "--output",
                "json",
            ])
            .write_stdin("{\"text\":\"one\"}\n{\"text\":\"two\"}\n")
            .assert()
            .get_output()
            .clone();
        assert_eq!(
            output.status.code(),
            Some(0),
            "{} was refused by map: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn a_deeply_nested_request_document_is_refused_not_a_stack_overflow() {
    // `ordered::Field` is recursive -- `Field::Object(OrderedMap<Field>)` -- so that the
    // order- and duplicate-preserving parse reaches inside a question body. A recursive
    // type over attacker-influenced bytes is a stack-exhaustion question, and the answer
    // has to be a bounded error rather than a crash.
    //
    // Two bounds apply: `serde_json`'s own recursion limit, and `jev-core`'s
    // MAX_JSON_DEPTH of 64 on the content itself.
    for (depth, expected) in [(60_usize, 0), (100, 2), (1000, 2), (100_000, 2)] {
        let document = format!(
            r#"{{"questions":{{"a":{{"type":"noul","instructions":{}1{}}}}}}}"#,
            r#"{"x":"#.repeat(depth),
            "}".repeat(depth)
        );
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), &document).unwrap();

        let output = jev_authed()
            .args([
                "ask",
                "-r",
                file.path().to_str().unwrap(),
                "--state",
                "x",
                "--dry-run",
            ])
            .assert()
            .get_output()
            .clone();
        assert_eq!(
            output.status.code(),
            Some(expected),
            "depth {depth}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        // Not a signal, and not exit 101 from a panic: a bounded refusal.
        assert_ne!(
            output.status.code(),
            None,
            "depth {depth} killed the process"
        );
    }
}

#[test]
fn map_refuses_colliding_paths_written_differently() {
    // A whole-path `==` misses `./out.jsonl` against `out.jsonl`, which is the likely
    // way to name one file twice by accident rather than an adversarial one.
    let questions = urgent_questions();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("in.jsonl"), "\"a\"\n").unwrap();

    let assert = jev_authed()
        .current_dir(directory.path())
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "-i",
            "in.jsonl",
            "--output-file",
            "./in.jsonl",
        ])
        .assert()
        .code(2);
    assert.stderr(predicate::str::contains("same path"));

    // And two genuinely different files are still accepted -- refusing those would be a
    // worse bug than the one this guards against.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    jev_authed()
        .current_dir(directory.path())
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "-i",
            "in.jsonl",
            "--endpoint",
            &api.endpoint(),
            "--output-file",
            "./out.jsonl",
        ])
        .assert()
        .success();
}

// --- fixes from the independent review passes ---------------------------------------

#[test]
fn map_honours_the_request_documents_model_like_ask_does() {
    // `map` read the session model directly and discarded the document's, so a request
    // file naming a model was billed on a different one -- while the documentation
    // promised the same file works with `ask` and `map` alike. It also made
    // `request_digest` blind to the one edit it exists to catch.
    let questions = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        questions.path(),
        json!({
            "model": "jev-1.13.0",
            "questions": {"answer": {"type": "noul", "instructions": "Urgent?"}},
        })
        .to_string(),
    )
    .unwrap();

    let sent = |extra: &[&str]| -> serde_json::Value {
        let api = MockApi::start(vec![Reply::ok(noul_body())]);
        let mut command = jev_authed();
        command.args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
        ]);
        command.args(extra);
        command.write_stdin("\"a\"\n").assert().success();
        serde_json::from_str(&api.requests()[0].body).unwrap()
    };

    assert_eq!(
        sent(&[])["model"],
        json!("jev-1.13.0"),
        "the document's model"
    );
    assert_eq!(
        sent(&["--model", "jev-latest"])["model"],
        json!("jev-latest"),
        "an explicit --model still wins"
    );
}

#[test]
fn map_resume_refuses_a_model_changed_inside_the_request_file() {
    // The consequence of the bug above: the fingerprint was computed from the model the
    // run was going to send anyway, so editing `model` in the file moved nothing.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let out = tempfile::NamedTempFile::new().unwrap();
    let questions = tempfile::NamedTempFile::new().unwrap();

    let run = |model: &str, resume: bool| {
        std::fs::write(
            questions.path(),
            json!({
                "model": model,
                "questions": {"answer": {"type": "noul", "instructions": "Urgent?"}},
            })
            .to_string(),
        )
        .unwrap();
        let mut command = jev_authed();
        command.args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "--output-file",
            out.path().to_str().unwrap(),
        ]);
        if resume {
            command.arg("--resume");
        }
        command.write_stdin("\"a\"\n").assert()
    };

    run("jev-1.13.0", false).success();
    run("jev-1.14.0", true)
        .code(2)
        .stderr(predicate::str::contains("different request"));
}

#[test]
fn map_refuses_a_review_file_that_already_has_rows() {
    // The same doubling the output file is protected from. This is the file a person
    // opens while iterating on a threshold, so accumulating every previous run's rows in
    // it is exactly as bad.
    let api = MockApi::start(vec![Reply::ok(noul_body_with(0.10))]);
    let questions = urgent_questions();
    let review = tempfile::NamedTempFile::new().unwrap();

    let run = || {
        jev_authed()
            .args([
                "map",
                "-r",
                questions.path().to_str().unwrap(),
                "--endpoint",
                &api.endpoint(),
                "--require",
                "answer.noul > 0.9",
                "--review-file",
                review.path().to_str().unwrap(),
            ])
            .write_stdin("\"a\"\n")
            .assert()
    };

    run().success();
    run().code(2).stderr(predicate::str::contains("remove it"));
    assert_eq!(
        std::fs::read_to_string(review.path())
            .unwrap()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count(),
        1
    );
}

#[test]
fn map_resume_warns_when_a_review_file_may_be_missing() {
    // Drop --review-file from an otherwise identical resumed command and the diverted
    // records look unevaluated: they are sent again, billed again, and end up in both
    // files. There is no honest way to refuse it -- a run that used --require without a
    // review file has nothing missing -- so the rows say whether the earlier run was
    // classifying, which is the precondition for the mistake.
    let api = MockApi::start(vec![Reply::ok(noul_body_with(0.99))]);
    let questions = urgent_questions();
    let out = tempfile::NamedTempFile::new().unwrap();

    jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "--require",
            "answer.noul > 0.9",
            "--output-file",
            out.path().to_str().unwrap(),
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .success();

    jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "--require",
            "answer.noul > 0.9",
            "--output-file",
            out.path().to_str().unwrap(),
            "--resume",
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .success()
        .stderr(predicate::str::contains("--review-file"));
}

#[test]
fn a_full_document_may_contain_a_question_called_type() {
    // The full-vs-bare heuristic tested that `questions.type` exists, not that it holds
    // a type name -- so a full document with a question legitimately called `type` took
    // the bare branch and blamed a key the user did not write.
    let output = dry_run_body(r#"{"questions":{"type":{"type":"noul","instructions":"?"}}}"#);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["body"]["questions"]["type"]["type"], json!("noul"));
}

#[test]
fn a_gate_error_is_one_readable_line_naming_the_offending_value() {
    // A missing line continuation left a 32-space run in the middle of the message, and
    // the number that caused it had been replaced by the word "a number".
    let output = jev()
        .env("JEV_API_KEY", "x")
        .args(["noul", "x", "--state", "y", "--require", "0.5 > 0.2"])
        .assert()
        .code(2)
        .get_output()
        .clone();
    let stderr = String::from_utf8_lossy(&output.stderr);
    // The message itself, not the aligned table of addressable paths that follows it.
    let message = stderr.lines().next().unwrap_or_default();
    assert!(!message.contains("   "), "run of spaces in: {message}");
    assert!(message.contains("the number 0.5"), "{message}");
}

// --- fixes from the Codex review pass ----------------------------------------------

#[test]
fn map_resume_refuses_reordered_choice_options() {
    // The digest was built by rendering each question into a `serde_json::Value`, whose
    // object type is a `BTreeMap` -- so it sorted the option names before hashing the
    // very thing `jev-core`'s hand-written serializers preserve on the wire. Reordering
    // the options changed what was sent and did not change the digest.
    let api = MockApi::start(vec![Reply::ok(
        json!({
            "model": "jev-1.13.0",
            "answers": {"t": {"type": "choice", "choice": "zebra", "confidence": 0.9,
                "probabilities": {"zebra": 0.9, "apple": 0.1}}},
            "usage": {"input_tokens": 1, "output_tokens": 1},
        })
        .to_string(),
    )]);
    let out = tempfile::NamedTempFile::new().unwrap();
    let questions = tempfile::NamedTempFile::new().unwrap();

    let run = |order: [&str; 2], resume: bool| {
        std::fs::write(
            questions.path(),
            format!(
                r#"{{"questions":{{"t":{{"type":"choice","instructions":"?","criteria":{{"{}":null,"{}":null}}}}}}}}"#,
                order[0], order[1]
            ),
        )
        .unwrap();
        let mut command = jev_authed();
        command.args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "--output-file",
            out.path().to_str().unwrap(),
        ]);
        if resume {
            command.arg("--resume");
        }
        command.write_stdin("\"x\"\n").assert()
    };

    run(["zebra", "apple"], false).success();
    run(["apple", "zebra"], true)
        .code(2)
        .stderr(predicate::str::contains("different request"));
    run(["zebra", "apple"], true).success();
}

#[test]
fn map_resume_validates_each_file_before_merging_them() {
    // `already.insert(index, row)` overwrote, so a good review-file row masked an
    // output-file row written from a different request: the run exited 0 reporting the
    // batch complete while the incompatible row sat on disk untouched.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let directory = tempfile::tempdir().unwrap();
    let out = directory.path().join("out.jsonl");
    let review = directory.path().join("review.jsonl");
    let questions = urgent_questions();

    let stale = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        stale.path(),
        json!({"answer": {"type": "noul", "instructions": "Something else?"}}).to_string(),
    )
    .unwrap();
    jev_authed()
        .args([
            "map",
            "-r",
            stale.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "--output-file",
            out.to_str().unwrap(),
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .success();

    jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "--require",
            "answer.noul > 0.99",
            "--review-file",
            review.to_str().unwrap(),
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .success();

    // Supplying the matching review file must not excuse the conflicting output row.
    jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "--require",
            "answer.noul > 0.99",
            "--output-file",
            out.to_str().unwrap(),
            "--review-file",
            review.to_str().unwrap(),
            "--resume",
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("different request"));
}

#[test]
fn map_reports_what_it_is_about_to_send_without_verbose() {
    // ADR-0010 and the threat model both name this count as one of the controls that
    // stands in for a confirmation prompt on a bulk-send command. It was written with
    // the verbose-only helper, so nobody saw it.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let questions = urgent_questions();

    jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
        ])
        .write_stdin("\"a\"\n\"b\"\n")
        .assert()
        .success()
        .stderr(predicate::str::contains("sending 2 record(s)"));

    // `--quiet` still silences it: that is an explicit instruction, not a default.
    jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "--quiet",
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .success()
        // The full phrase: the non-official-endpoint warning also contains the word
        // "sending" and deliberately survives --quiet.
        .stderr(predicate::str::contains("record(s),").not());
}

#[test]
fn a_choice_option_name_may_not_contain_a_control_character() {
    // `QuestionId` rejected control characters from the beginning; option names did not,
    // so the two halves of one document were held to different standards -- and an
    // option name becomes a JSON key, comes back as the answer's `choice`, and is what a
    // `--require` gate compares against.
    let output = dry_run_body(
        "{\"questions\":{\"a\":{\"type\":\"choice\",\"instructions\":\"?\",\"criteria\":{\"a\\u0007b\":null,\"b\":null}}}}",
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("control character"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_text_mode_dry_run_escapes_terminal_hazards_like_every_other_output() {
    // `--dry-run` exists so a request built from untrusted state -- a ticket body, a log
    // line, a diff -- can be read before it is sent. The JSON branch escaped the
    // characters that rewrite a terminal; the text branch, which is by definition the
    // one read on a terminal, pretty-printed straight to stdout. A bidirectional
    // override in the state therefore reordered the preview on the reviewer's own
    // screen, so the request they approved was not the request they read.
    use std::io::Write as _;
    let mut state = tempfile::NamedTempFile::new().unwrap();
    state
        .write_all("approve\u{202e}yned\u{200b}\u{7f}".as_bytes())
        .unwrap();
    state.flush().unwrap();

    for format in [None, Some("json")] {
        let mut command = jev_authed();
        command.args([
            "noul",
            "does it?",
            "--state-file",
            state.path().to_str().unwrap(),
            "--dry-run",
        ]);
        if let Some(format) = format {
            command.args(["--output", format]);
        }
        let assert = command.assert().success();
        let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

        for (hazard, name) in [
            ('\u{202e}', "RIGHT-TO-LEFT OVERRIDE"),
            ('\u{200b}', "ZERO WIDTH SPACE"),
            ('\u{7f}', "DELETE"),
        ] {
            assert!(
                !stdout.contains(hazard),
                "a raw {name} reached stdout in {} mode",
                format.unwrap_or("text")
            );
        }
        // Escaped, not dropped: the reviewer still sees that the character is there.
        assert!(stdout.contains("\\u202e"), "{stdout}");
    }
}

#[test]
fn a_text_mode_dry_run_is_still_pretty_printed() {
    // The first fix for the hazard above escaped the structural line feeds as well,
    // collapsing the whole document onto one line of `\u000a`. The escaping and the
    // indentation have to survive together, so both are pinned.
    let assert = jev_authed()
        .args(["noul", "does it?", "--state", "plain", "--dry-run"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.lines().count() > 5, "not pretty-printed: {stdout}");
    assert!(
        !stdout.contains("\\u000a"),
        "line feeds were escaped: {stdout}"
    );
    assert!(stdout.contains("\n  \"body\""), "{stdout}");
}

// --- jev eval -----------------------------------------------------------------------

/// A response body carrying one Noul answer with the given probability.
fn noul_at(probability: f64) -> String {
    json!({
        "model": "jev-1.13.0",
        "answers": {"urgent": {"type": "noul", "noul": probability}},
        "usage": {"input_tokens": 10, "output_tokens": 2}
    })
    .to_string()
}

/// A one-Noul request file, and a labelled dataset built from `(probability, label)`
/// pairs. Returns the files and the replies to queue, in row order.
fn eval_fixture(
    points: &[(f64, bool)],
) -> (tempfile::NamedTempFile, tempfile::NamedTempFile, Vec<Reply>) {
    let request = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        request.path(),
        json!({"urgent": {"type": "noul", "instructions": "Is this urgent?"}}).to_string(),
    )
    .unwrap();

    let dataset = tempfile::NamedTempFile::new().unwrap();
    let rows: Vec<String> = points
        .iter()
        .enumerate()
        .map(|(index, (_, label))| {
            json!({
                "schema": "jev.eval.row/v1",
                "id": format!("row-{index}"),
                "state": format!("example {index}"),
                "labels": {"urgent": label},
            })
            .to_string()
        })
        .collect();
    std::fs::write(dataset.path(), rows.join("\n")).unwrap();

    let replies = points
        .iter()
        .map(|(probability, _)| Reply::ok(noul_at(*probability)))
        .collect();
    (request, dataset, replies)
}

/// `jev eval`, wired to a mock, with concurrency 1 so replies line up with rows.
fn eval_command(
    api: &MockApi,
    request: &tempfile::NamedTempFile,
    dataset: &tempfile::NamedTempFile,
) -> assert_cmd::Command {
    let mut command = jev_authed();
    command.args([
        "eval",
        "-r",
        request.path().to_str().unwrap(),
        "-d",
        dataset.path().to_str().unwrap(),
        "--endpoint",
        &api.endpoint(),
        "-j",
        "1",
        "--output",
        "json",
    ]);
    command
}

#[test]
fn eval_computes_metrics_that_match_a_hand_computed_value() {
    // Brier = ((0.9-1)^2 + (0.8-0)^2 + (0.2-0)^2 + (0.1-0)^2) / 4
    //       = (0.01 + 0.64 + 0.04 + 0.01) / 4 = 0.175
    // Asserted against arithmetic done here, not against another run of the same code.
    let points = [(0.9, true), (0.8, false), (0.2, false), (0.1, false)];
    let (request, dataset, replies) = eval_fixture(&points);
    let api = MockApi::start(replies);

    let assert = eval_command(&api, &request, &dataset).assert().success();
    let document = json_stdout(assert.get_output());
    let urgent = &document["questions"]["urgent"];

    assert_eq!(document["schema"], json!("jev.eval/v1"));
    assert_eq!(urgent["n"], json!(4));
    assert!(
        (urgent["brier_score"].as_f64().unwrap() - 0.175).abs() < 1e-9,
        "{urgent}"
    );
    // No objective was given, so nothing was selected and every row is reported.
    assert_eq!(document["split"]["mode"], json!("none"));
    assert_eq!(document["split"]["reported_rows"], json!(4));
    assert_eq!(urgent["threshold"], json!(null));
    assert_no_canary(assert.get_output());
}

#[test]
fn eval_maximizing_f1_picks_the_cut_that_actually_maximizes_it() {
    // Sweep on these four rows: F1 is 0.4 at 0.0/0.1/0.2, 0.667 at 0.8, and 1.0 at 0.9.
    let points = [(0.9, true), (0.8, false), (0.2, false), (0.1, false)];
    let (request, dataset, replies) = eval_fixture(&points);
    let api = MockApi::start(replies);

    let assert = eval_command(&api, &request, &dataset)
        .args(["--objective", "maximize-f1", "--no-split"])
        .assert()
        .success();
    let document = json_stdout(assert.get_output());
    let urgent = &document["questions"]["urgent"];
    assert!(
        (urgent["threshold"].as_f64().unwrap() - 0.9).abs() < 1e-9,
        "{urgent}"
    );
    assert_eq!(urgent["threshold_reachable"], json!(true));
    assert_eq!(document["objective"]["name"], json!("maximize-f1"));
    assert_eq!(document["objective"]["threshold_field"], json!("noul"));
    // The objective is recorded beside the threshold, always. A cut is optimal only
    // with respect to what it was optimized for.
    assert!(urgent["threshold_tie_break"].is_string(), "{urgent}");
}

#[test]
fn eval_says_a_target_is_unreachable_rather_than_relaxing_it_and_exits_one() {
    // The best precision available on this data is 0.5, so a 0.95 floor cannot be met.
    // Quietly returning the closest threshold would hand the user a gate that does not
    // do what the number beside it claims.
    let points = [(0.9, false), (0.1, true)];
    let (request, dataset, replies) = eval_fixture(&points);
    let api = MockApi::start(replies);

    let assert = eval_command(&api, &request, &dataset)
        .args([
            "--objective",
            "min-precision",
            "--target",
            "0.95",
            "--no-split",
        ])
        .assert()
        .code(1);
    let document = json_stdout(assert.get_output());
    let urgent = &document["questions"]["urgent"];
    assert_eq!(urgent["threshold"], json!(null));
    assert_eq!(urgent["threshold_reachable"], json!(false));
    assert!(
        String::from_utf8_lossy(&assert.get_output().stderr).contains("no threshold"),
        "{}",
        String::from_utf8_lossy(&assert.get_output().stderr)
    );
}

#[test]
fn eval_never_sends_a_label_to_the_api() {
    // The privacy invariant this command turns on: ground truth is compared locally,
    // and the request is built from `state` alone. A regression here would ship the
    // answers to a third party along with the questions.
    const GROUND_TRUTH: &str = "ZZ-ground-truth-must-not-be-sent-ZZ";
    let request = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        request.path(),
        json!({"team": {"type": "choice", "instructions": "Which?",
               "criteria": {GROUND_TRUTH: "the labelled one", "other": "not it"}}})
        .to_string(),
    )
    .unwrap();
    let dataset = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        dataset.path(),
        json!({"schema": "jev.eval.row/v1", "id": "1", "state": "a ticket",
               "labels": {"team": GROUND_TRUTH}})
        .to_string(),
    )
    .unwrap();
    let api = MockApi::start(vec![Reply::ok(
        json!({"model": "jev-1.13.0", "answers": {"team": {"type": "choice",
               "choice": "other", "confidence": 0.8,
               "probabilities": {GROUND_TRUTH: 0.2, "other": 0.8}}},
               "usage": {}})
        .to_string(),
    )]);

    let assert = eval_command(&api, &request, &dataset).assert().success();

    // The option name legitimately appears in the request, because it is part of the
    // question. What must not appear is a `labels` object.
    for seen in api.requests() {
        let body: serde_json::Value = serde_json::from_str(&seen.body).unwrap();
        assert!(
            body.get("labels").is_none(),
            "a request carried a labels object: {}",
            seen.body
        );
        assert_eq!(
            body.as_object().unwrap().keys().collect::<Vec<_>>(),
            vec!["model", "questions", "state"],
            "a request carried a field beyond the API's own request body"
        );
    }
    assert_no_canary(assert.get_output());
}

#[test]
fn eval_dry_run_sends_nothing_and_previews_the_split() {
    let points = [(0.9, true), (0.1, false)];
    let (request, dataset, _) = eval_fixture(&points);
    let api = MockApi::start(vec![Reply::ok(noul_at(0.9))]);

    let assert = eval_command(&api, &request, &dataset)
        .arg("--dry-run")
        .assert()
        .success();
    let document = json_stdout(assert.get_output());
    assert_eq!(document["schema"], json!("jev.dry-run/v1"));
    assert_eq!(document["sent"], json!(false));
    // `records`, the same key `jev map --dry-run` uses for the same thing: one schema,
    // one name per concept.
    assert_eq!(document["records"], json!(2));
    assert!(document["split"].is_object(), "{document}");
    assert_eq!(api.hits(), 0, "a dry run reached the network");
    assert_no_canary(assert.get_output());
}

#[test]
fn eval_records_the_model_that_answered_separately_from_the_one_requested() {
    // `jev-latest` moves. A report that recorded only the alias would let a threshold be
    // quoted without the version it was measured against, which is the whole reason the
    // report exists.
    let points = [(0.9, true), (0.1, false)];
    let (request, dataset, replies) = eval_fixture(&points);
    let api = MockApi::start(replies);

    let assert = eval_command(&api, &request, &dataset)
        .args(["--model", "jev-latest"])
        .assert()
        .success();
    let document = json_stdout(assert.get_output());
    assert_eq!(document["model_requested"], json!("jev-latest"));
    assert_eq!(document["model"], json!(["jev-1.13.0"]));
    for seen in api.requests() {
        let body: serde_json::Value = serde_json::from_str(&seen.body).unwrap();
        assert_eq!(body["model"], json!("jev-latest"), "the pin was not sent");
    }
}

#[test]
fn eval_warns_when_the_threshold_is_chosen_and_reported_on_the_same_rows() {
    // The mistake the whole split exists to prevent. It is allowed, because a user with
    // forty examples may have no better option, but it is never silent -- and the
    // warning is in the document as well as on stderr, so a report read back months
    // later still says what was wrong with it.
    let points = [(0.9, true), (0.8, false), (0.2, false), (0.1, false)];
    let (request, dataset, replies) = eval_fixture(&points);
    let api = MockApi::start(replies);

    let assert = eval_command(&api, &request, &dataset)
        .args(["--objective", "maximize-f1", "--no-split"])
        .assert()
        .success();
    let document = json_stdout(assert.get_output());
    let warnings = document["warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|warning| warning.as_str().unwrap_or("").contains("--no-split")),
        "{document}"
    );
    assert!(
        String::from_utf8_lossy(&assert.get_output().stderr).contains("optimistic"),
        "{}",
        String::from_utf8_lossy(&assert.get_output().stderr)
    );
}

#[test]
fn eval_says_the_no_split_warning_once_and_ends_its_text_lines_cleanly() {
    // The warning was printed bare before the run and again, prefixed, after it.
    let points = [(0.9, true), (0.8, false), (0.2, false), (0.1, false)];
    let (request, dataset, replies) = eval_fixture(&points);
    let api = MockApi::start(replies);

    let assert = jev_authed()
        .args([
            "eval",
            "-r",
            request.path().to_str().unwrap(),
            "-d",
            dataset.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "-j",
            "1",
            "--output",
            "text",
            "--objective",
            "maximize-f1",
            "--no-split",
        ])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert_eq!(stderr.matches("optimistic").count(), 1, "{stderr}");
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.lines().all(|line| line == line.trim_end()),
        "a line ends in whitespace:\n{stdout}"
    );
}

#[test]
fn eval_with_separate_files_reports_only_the_test_rows() {
    let (request, calibration, mut replies) =
        eval_fixture(&[(0.9, true), (0.8, false), (0.2, false)]);
    let test = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        test.path(),
        [
            json!({"schema": "jev.eval.row/v1", "id": "t-0", "state": "held out 0",
                   "labels": {"urgent": true}})
            .to_string(),
            json!({"schema": "jev.eval.row/v1", "id": "t-1", "state": "held out 1",
                   "labels": {"urgent": false}})
            .to_string(),
        ]
        .join("\n"),
    )
    .unwrap();
    replies.push(Reply::ok(noul_at(0.95)));
    replies.push(Reply::ok(noul_at(0.05)));
    let api = MockApi::start(replies);

    let assert = jev_authed()
        .args([
            "eval",
            "-r",
            request.path().to_str().unwrap(),
            "--calibration",
            calibration.path().to_str().unwrap(),
            "--test",
            test.path().to_str().unwrap(),
            "--objective",
            "maximize-f1",
            "--endpoint",
            &api.endpoint(),
            "-j",
            "1",
            "--output",
            "json",
        ])
        .assert()
        .success();
    let document = json_stdout(assert.get_output());
    assert_eq!(document["split"]["mode"], json!("files"));
    assert_eq!(document["split"]["calibration_rows"], json!(3));
    assert_eq!(document["split"]["reported_rows"], json!(2));
    assert_eq!(document["questions"]["urgent"]["n"], json!(2));
    assert_eq!(api.hits(), 5, "every row should have been evaluated once");
}

#[test]
fn eval_refuses_the_same_row_in_both_the_calibration_and_the_test_file() {
    // Ids are unique within a file, so the one way two files can hide the leak the
    // split exists to prevent is the same example appearing in both.
    let (request, calibration, _) = eval_fixture(&[(0.9, true)]);
    let duplicate = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        duplicate.path(),
        json!({"schema": "jev.eval.row/v1", "id": "row-0", "state": "example 0",
               "labels": {"urgent": true}})
        .to_string(),
    )
    .unwrap();

    jev_authed()
        .args([
            "eval",
            "-r",
            request.path().to_str().unwrap(),
            "--calibration",
            calibration.path().to_str().unwrap(),
            "--test",
            duplicate.path().to_str().unwrap(),
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("appears in both"));
}

#[test]
fn eval_refuses_an_objective_that_applies_to_no_question_before_sending_anything() {
    // A confidence objective against a request of nothing but nouls. A noul has no
    // confidence, so there is nothing to sweep -- and finding that out after paying for
    // two hundred rows would be the worst possible time.
    let (request, dataset, _) = eval_fixture(&[(0.9, true)]);
    let api = MockApi::start(vec![Reply::ok(noul_at(0.9))]);

    jev_authed()
        .args([
            "eval",
            "-r",
            request.path().to_str().unwrap(),
            "-d",
            dataset.path().to_str().unwrap(),
            "--objective",
            "target-coverage",
            "--target",
            "0.8",
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("applies to no question"));
    assert_eq!(api.hits(), 0, "a refused run still sent a request");
}

#[test]
fn eval_refuses_an_objective_and_target_that_do_not_go_together() {
    let (request, dataset, _) = eval_fixture(&[(0.9, true)]);
    let base = |extra: &[&str]| {
        let mut command = jev_authed();
        command.args([
            "eval",
            "-r",
            request.path().to_str().unwrap(),
            "-d",
            dataset.path().to_str().unwrap(),
        ]);
        command.args(extra);
        command
    };

    base(&["--objective", "min-precision"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("needs a --target"));
    base(&["--objective", "maximize-f1", "--target", "0.9"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("takes no --target"));
    base(&["--target", "0.9"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("without --objective"));
    base(&["--objective", "min-precision", "--target", "1.5"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("between 0 and 1"));
}

#[test]
fn eval_refuses_a_dataset_that_is_not_one() {
    let (request, _, _) = eval_fixture(&[(0.9, true)]);
    let refuse = |body: &str, needle: &str| {
        let dataset = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(dataset.path(), body).unwrap();
        jev_authed()
            .args([
                "eval",
                "-r",
                request.path().to_str().unwrap(),
                "-d",
                dataset.path().to_str().unwrap(),
            ])
            .assert()
            .code(2)
            .stderr(predicate::str::contains(needle));
    };

    // A `jev map` output file is the likeliest wrong thing to point at --dataset.
    refuse(
        r#"{"schema":"jev.map.row/v1","index":0,"id":"a","ok":true}"#,
        "jev.eval.row/v1",
    );
    refuse(
        r#"{"schema":"jev.eval.row/v1","id":"a","state":"s","labels":{"urgnet":true}}"#,
        "which is not in",
    );
    refuse(
        r#"{"schema":"jev.eval.row/v1","id":"a","state":"s","labels":{"urgent":"yes"}}"#,
        "true, false, 0, or 1",
    );
    refuse("", "no labelled rows");
}

#[test]
fn eval_writes_a_report_file_that_is_not_world_readable() {
    let points = [(0.9, true), (0.1, false)];
    let (request, dataset, replies) = eval_fixture(&points);
    let api = MockApi::start(replies);
    let directory = tempfile::tempdir().unwrap();
    let report = directory.path().join("report.json");

    eval_command(&api, &request, &dataset)
        .args(["--report", report.to_str().unwrap()])
        .assert()
        .success();

    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    assert_eq!(written["schema"], json!("jev.eval/v1"));
    assert_eq!(written["questions"]["urgent"]["n"], json!(2));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        // A report holds the model's judgments about the user's own examples, so it gets
        // the same 0600 as the configuration file and `jev map`'s row files.
        let mode = std::fs::metadata(&report).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "report mode was {:o}", mode & 0o777);
    }
}

#[test]
fn eval_reports_a_partial_run_as_partial_and_names_the_reason() {
    // A run where every row failed used to report only a count. The reason is the only
    // actionable part, so it is in the document and in the stderr line.
    let (request, dataset, _) = eval_fixture(&[(0.9, true), (0.1, false)]);
    let api = MockApi::start(vec![
        Reply::ok(noul_at(0.9)),
        Reply::status(500, r#"{"error":{"message":"upstream exploded"}}"#),
    ]);

    let assert = eval_command(&api, &request, &dataset)
        .args(["--retries", "0"])
        .assert()
        .code(5);
    let document = json_stdout(assert.get_output());
    assert_eq!(document["rows"]["failed"], json!(1));
    assert!(
        document["rows"]["errors"]["unavailable"]["count"] == json!(1),
        "{}",
        document["rows"]
    );
    // The good row is still scored.
    assert_eq!(document["questions"]["urgent"]["n"], json!(1));
    assert_eq!(document["questions"]["urgent"]["labelled"], json!(2));
    assert_no_canary(assert.get_output());
}

#[test]
fn eval_never_uses_a_typesafe_credential_for_a_custom_endpoint() {
    // The same structural isolation every other sending command has, asserted for this
    // one too rather than assumed from the shared call site.
    let (request, dataset, replies) = eval_fixture(&[(0.9, true)]);
    let api = MockApi::start(replies);

    jev()
        .env("JEV_API_KEY", CANARY_KEY)
        .env("TYPESAFE_API_KEY", CANARY_KEY)
        .args([
            "eval",
            "-r",
            request.path().to_str().unwrap(),
            "-d",
            dataset.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
        ])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("JEV_CUSTOM_API_KEY"));
    assert_eq!(api.hits(), 0);
}

#[test]
fn the_eval_document_has_its_documented_fields() {
    let points = [(0.9, true), (0.1, false)];
    let (request, dataset, replies) = eval_fixture(&points);
    let api = MockApi::start(replies);

    let assert = eval_command(&api, &request, &dataset).assert().success();
    let document = json_stdout(assert.get_output());
    let mut keys: Vec<&str> = document
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "dataset",
            "endpoint",
            "evaluated_at",
            "model",
            "model_requested",
            "objective",
            "questions",
            "request",
            "rows",
            "schema",
            "split",
            "usage",
            "warnings",
        ]
    );
    // RFC 3339, in UTC, so two reports sort and compare.
    let stamp = document["evaluated_at"].as_str().unwrap();
    assert!(stamp.ends_with('Z') && stamp.len() == 20, "{stamp}");
    assert!(document["dataset"]["fingerprint"].as_str().unwrap().len() == 16);
    assert!(document["request"]["fingerprint"].as_str().unwrap().len() == 16);
}

#[test]
fn eval_splits_the_same_rows_the_same_way_for_the_same_seed() {
    // The property that makes two reports comparable: rerunning, or rerunning after
    // appending examples, must not silently change what "reported on" means.
    let points: Vec<(f64, bool)> = (0..40)
        .map(|index| (f64::from(index) / 40.0, index % 3 == 0))
        .collect();
    let (request, dataset, replies) = eval_fixture(&points);

    let split_of = |seed: &str| {
        let api = MockApi::start(replies.clone());
        let assert = eval_command(&api, &request, &dataset)
            .args(["--objective", "maximize-f1", "--seed", seed])
            .assert();
        let document = json_stdout(assert.get_output());
        (
            document["split"]["calibration_rows"].clone(),
            document["split"]["reported_rows"].clone(),
        )
    };
    assert_eq!(split_of("0"), split_of("0"));
    assert_ne!(
        split_of("0"),
        split_of("12345"),
        "two seeds produced the same split"
    );
}

#[test]
fn the_committed_labelled_dataset_validates_against_its_request_file() {
    // The cookbook's calibration recipe points these two files at each other. A label
    // naming a question that was renamed, or a Choice label that no longer matches an
    // option, would make the documented command fail for every reader -- and a dataset
    // is exactly the kind of file that drifts from the question set beside it.
    jev_authed()
        .args([
            "eval",
            "-r",
            "../../examples/requests/issue-triage.json",
            "-d",
            "../../examples/datasets/issue-triage.labelled.jsonl",
            "--dry-run",
            "--output",
            "json",
        ])
        .assert()
        .success();
}

// --- Combinations the isolated tests missed -----------------------------------------

#[test]
fn a_dry_run_never_resolves_a_credential() {
    // `--dry-run` is sold on touching nothing, and every existing dry-run test happened
    // to run with a credential already present. A regression that moved
    // `session.credential()` ahead of the dry-run check would therefore have passed the
    // whole suite while making the flag require a key it never uses.
    let questions = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        questions.path(),
        json!({"answer": {"type": "noul", "instructions": "urgent?"}}).to_string(),
    )
    .unwrap();

    // `jev()`, not `jev_authed()`: no environment credential, and the harness already
    // sets JEV_NO_KEYCHAIN, so there is no credential anywhere to find.
    jev()
        .args([
            "ask",
            "-r",
            questions.path().to_str().unwrap(),
            "--state",
            "a ticket",
            "--dry-run",
        ])
        .assert()
        .success();

    jev()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--lines",
            "--dry-run",
        ])
        .write_stdin("a ticket\n")
        .assert()
        .success();

    let dataset = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        dataset.path(),
        json!({"schema": "jev.eval.row/v1", "id": "1", "state": "a ticket",
               "labels": {"answer": true}})
        .to_string(),
    )
    .unwrap();
    jev()
        .args([
            "eval",
            "-r",
            questions.path().to_str().unwrap(),
            "-d",
            dataset.path().to_str().unwrap(),
            "--dry-run",
        ])
        .assert()
        .success();
}

#[test]
fn map_never_uses_the_typesafe_credential_for_a_custom_endpoint() {
    // The isolation is structural and shared, but it had only ever been asserted through
    // `noul`. `map` has its own `session.credential()` call site, and a batch is the
    // command most likely to be pointed at a proxy.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let questions = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        questions.path(),
        json!({"answer": {"type": "noul", "instructions": "urgent?"}}).to_string(),
    )
    .unwrap();

    jev()
        .env("JEV_API_KEY", CANARY_KEY)
        .env("TYPESAFE_API_KEY", CANARY_KEY)
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--lines",
            "--endpoint",
            &api.endpoint(),
        ])
        .write_stdin("a ticket\n")
        .assert()
        .code(3)
        .stderr(predicate::str::contains("JEV_CUSTOM_API_KEY"))
        .stderr(predicate::str::contains("non-official"));
    assert_eq!(
        api.hits(),
        0,
        "a request went out with no usable credential"
    );
}

#[test]
fn map_ignores_a_request_documents_own_state_and_says_so() {
    // `docs/commands.md` promises one request file works with `ask` and `map` alike, and
    // the same file used with `ask` legitimately carries a `state`. If `map` silently
    // preferred it, every row would be answered about the wrong thing.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let questions = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        questions.path(),
        json!({
            "state": "THE REQUEST FILE'S OWN STATE",
            "questions": {"answer": {"type": "noul", "instructions": "urgent?"}}
        })
        .to_string(),
    )
    .unwrap();

    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--state-field",
            "text",
            "--endpoint",
            &api.endpoint(),
        ])
        .write_stdin("{\"text\":\"the row's own state\"}\n")
        .assert()
        .success();
    assert!(
        String::from_utf8_lossy(&assert.get_output().stderr).contains("is ignored"),
        "{}",
        String::from_utf8_lossy(&assert.get_output().stderr)
    );

    let sent: serde_json::Value = serde_json::from_str(&api.requests()[0].body).unwrap();
    assert_eq!(sent["state"], json!("the row's own state"));
}

#[test]
fn map_carries_every_question_type_and_its_uncertainty_through_a_row_unchanged() {
    // The only place `map` executed a multi-type request file was a dry run, so the row
    // renderer had never run over a Choice and a Score at once -- and the values a
    // caller actually acts on, the confidence and the distribution, were never checked
    // for surviving the round trip at all.
    let api = MockApi::start(vec![Reply::ok(
        json!({
            "model": "jev-1.13.0",
            "answers": {
                "kind": {"type": "choice", "choice": "bug", "confidence": 0.734_56,
                         "probabilities": {"bug": 0.734_56, "feature": 0.265_44}},
                "severity": {"type": "score", "score": 1.25,
                             "legend": {"0": "low", "1": "mid", "2": "high"},
                             "probabilities": {"0": 0.1, "1": 0.55, "2": 0.35},
                             "confidence": 0.61},
                "actionable": {"type": "noul", "noul": 0.5}
            },
            "usage": {"input_tokens": 10, "output_tokens": 2}
        })
        .to_string(),
    )]);
    let questions = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        questions.path(),
        json!({
            "kind": {"type": "choice", "instructions": "Which?",
                     "criteria": {"bug": "a defect", "feature": "a request"}},
            "severity": {"type": "score", "instructions": "How bad?",
                         "criteria": ["low", "mid", "high"]},
            "actionable": {"type": "noul", "instructions": "Can anyone act?"}
        })
        .to_string(),
    )
    .unwrap();

    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--lines",
            "--endpoint",
            &api.endpoint(),
        ])
        .write_stdin("a ticket\n")
        .assert()
        .success();

    let row: serde_json::Value = serde_json::from_str(
        String::from_utf8_lossy(&assert.get_output().stdout)
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    let answers = &row["answers"];
    assert_eq!(answers["kind"]["choice"], json!("bug"));
    // Byte-for-byte, not rounded: the distribution is the reason to use this model, and
    // a renderer that reformatted it would quietly change what a threshold compares to.
    assert_eq!(answers["kind"]["confidence"], json!(0.734_56));
    assert_eq!(answers["kind"]["probabilities"]["feature"], json!(0.265_44));
    assert_eq!(answers["severity"]["score"], json!(1.25));
    assert_eq!(answers["severity"]["confidence"], json!(0.61));
    assert_eq!(answers["severity"]["legend"]["2"], json!("high"));
    assert_eq!(answers["actionable"]["noul"], json!(0.5));
    // And the invariant the whole CLI turns on: a Noul grows no confidence on the way
    // through a batch.
    assert!(answers["actionable"].get("confidence").is_none(), "{row}");
}

#[test]
fn map_retries_a_transient_row_while_a_permanent_one_is_not_retried() {
    // Every existing partial-failure test ran with `--retries 0`, so no row had ever
    // actually retried *inside* a batch. What this pins is that the two kinds of failure
    // are still told apart at row granularity: a 500 is worth another attempt, a 400 is
    // not, and one row's outcome does not decide another's.
    let api = MockApi::start(vec![
        // Row 0: transient, then good.
        Reply::status(503, r#"{"error":{"message":"overloaded"}}"#),
        Reply::ok(noul_body()),
        // Row 1: permanent. Must not be retried.
        Reply::status(400, r#"{"error":{"message":"malformed question"}}"#),
        // Row 2: good first time.
        Reply::ok(noul_body()),
    ]);
    let questions = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        questions.path(),
        json!({"answer": {"type": "noul", "instructions": "urgent?"}}).to_string(),
    )
    .unwrap();

    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--lines",
            "--endpoint",
            &api.endpoint(),
            // One worker, so the replies above line up with the rows in order.
            "-j",
            "1",
            "--retries",
            "2",
        ])
        .write_stdin("first\nsecond\nthird\n")
        .assert()
        .code(5);

    let lines: Vec<serde_json::Value> = String::from_utf8_lossy(&assert.get_output().stdout)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let rows: Vec<&serde_json::Value> = lines
        .iter()
        .filter(|line| line["schema"] == json!("jev.map.row/v1"))
        .collect();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[0]["ok"],
        json!(true),
        "the transient row was not retried"
    );
    assert_eq!(rows[0]["attempts"], json!(2));
    assert_eq!(rows[1]["ok"], json!(false));
    assert_eq!(rows[1]["error"]["kind"], json!("request"));
    assert_eq!(rows[2]["ok"], json!(true));

    // Four requests: two for row 0, one for row 1, one for row 2. A retried permanent
    // failure would make this five or more.
    assert_eq!(api.hits(), 4, "a permanent failure was retried");

    let summary = lines.last().unwrap();
    assert_eq!(summary["schema"], json!("jev.map.summary/v1"));
    assert_eq!(summary["succeeded"], json!(2));
    assert_eq!(summary["failed"], json!(1));
    assert_no_canary(assert.get_output());
}

#[test]
fn map_resume_refuses_an_output_file_that_describes_more_records_than_the_input_has() {
    // Every resume test either matched the input exactly or grew it. Shrinking it -- the
    // normal thing to do after filtering a batch down -- left a recorded row at an index
    // the input no longer contains, which used to inflate `total` and report a shrunken
    // batch as complete.
    let questions = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        questions.path(),
        json!({"answer": {"type": "noul", "instructions": "urgent?"}}).to_string(),
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("out.jsonl");
    std::fs::write(
        &output,
        (0..5)
            .map(|index| {
                json!({"schema": "jev.map.row/v1", "index": index, "ok": true}).to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .unwrap();

    jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--lines",
            "--output-file",
            output.to_str().unwrap(),
            "--resume",
        ])
        .write_stdin("first\nsecond\n")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("different input"));
}

/// A request file with one question of each type, and a matching labelled dataset.
fn eval_mixed_fixture() -> (tempfile::NamedTempFile, tempfile::NamedTempFile) {
    let request = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        request.path(),
        json!({
            "urgent": {"type": "noul", "instructions": "Is this urgent?"},
            "team": {"type": "choice", "instructions": "Which team?",
                     "criteria": {"billing": "money", "support": "help", "other": "neither"}},
            "severity": {"type": "score", "instructions": "How bad?",
                         "criteria": ["minor", "moderate", "severe"]}
        })
        .to_string(),
    )
    .unwrap();

    let dataset = tempfile::NamedTempFile::new().unwrap();
    let teams = ["billing", "support", "other"];
    let rows: Vec<String> = (0..6)
        .map(|index: usize| {
            json!({
                "schema": "jev.eval.row/v1",
                "id": format!("row-{index}"),
                "state": format!("ticket {index}"),
                "labels": {
                    "urgent": index.is_multiple_of(2),
                    "team": teams[index % 3],
                    "severity": index % 3,
                },
            })
            .to_string()
        })
        .collect();
    std::fs::write(dataset.path(), rows.join("\n")).unwrap();
    (request, dataset)
}

/// A response answering all three questions of [`eval_mixed_fixture`].
fn mixed_answer(team: &str, level: u32) -> String {
    json!({
        "model": "jev-1.13.0",
        "answers": {
            "urgent": {"type": "noul", "noul": 0.7},
            "team": {"type": "choice", "choice": team, "confidence": 0.82,
                     "probabilities": {"billing": 0.82, "support": 0.1, "other": 0.08}},
            "severity": {"type": "score", "score": f64::from(level),
                         "legend": {"0": "minor", "1": "moderate", "2": "severe"},
                         "probabilities": {"0": 0.2, "1": 0.7, "2": 0.1},
                         "confidence": 0.66}
        },
        "usage": {"input_tokens": 12, "output_tokens": 3}
    })
    .to_string()
}

#[test]
fn eval_scores_a_choice_and_a_score_question_alongside_a_noul() {
    // The Noul path had end-to-end coverage and the other two did not, so the observation
    // collector, the per-class tables, the ordinal metrics, and the coverage sweeps had
    // never run against a real response.
    let (request, dataset) = eval_mixed_fixture();
    let api = MockApi::start(vec![Reply::ok(mixed_answer("billing", 1))]);

    let assert = eval_command(&api, &request, &dataset).assert().success();
    let document = json_stdout(assert.get_output());

    let team = &document["questions"]["team"];
    assert_eq!(team["type"], json!("choice"));
    assert_eq!(team["n"], json!(6));
    // Two of six rows are labelled `billing`, and the mock always answers `billing`.
    assert!(
        (team["accuracy"].as_f64().unwrap() - 2.0 / 6.0).abs() < 1e-9,
        "{team}"
    );
    assert!(team["per_class"]["other"].is_object(), "{team}");
    assert_eq!(team["per_class"]["billing"]["support"], json!(2));
    // A class the model never predicts has no precision, and that is `null`, not zero.
    assert_eq!(team["per_class"]["support"]["precision"], json!(null));
    assert!(team["brier_score"].is_number(), "{team}");
    assert!(
        team["coverage_sweep"].as_array().unwrap().len() >= 2,
        "{team}"
    );
    // Micro-F1 is deliberately absent: in single-label multiclass it *is* accuracy.
    assert!(team.get("micro_f1").is_none(), "{team}");

    let severity = &document["questions"]["severity"];
    assert_eq!(severity["type"], json!("score"));
    // Labels cycle 0,1,2; the answer is always level 1, so two of six agree exactly and
    // every row is within one level.
    assert!(
        (severity["exact_agreement"].as_f64().unwrap() - 2.0 / 6.0).abs() < 1e-9,
        "{severity}"
    );
    assert!(
        (severity["adjacent_agreement"].as_f64().unwrap() - 1.0).abs() < 1e-9,
        "{severity}"
    );
    assert!(
        severity["quadratic_weighted_kappa"].is_number(),
        "{severity}"
    );
    // A Score gets a weighted kappa because its levels are ordered; a Choice does not.
    assert!(team.get("quadratic_weighted_kappa").is_none(), "{team}");
}

#[test]
fn eval_selects_a_confidence_cut_for_a_choice_and_refuses_one_for_a_noul() {
    let (request, dataset) = eval_mixed_fixture();
    let api = MockApi::start(vec![Reply::ok(mixed_answer("billing", 1))]);

    let assert = eval_command(&api, &request, &dataset)
        .args([
            "--objective",
            "target-coverage",
            "--target",
            "0.5",
            "--no-split",
        ])
        .assert()
        .success();
    let document = json_stdout(assert.get_output());
    assert_eq!(
        document["objective"]["threshold_field"],
        json!("confidence")
    );
    // It applies to the two that have a confidence, and not to the noul -- which is the
    // whole reason the objectives are routed by type rather than offered uniformly.
    assert_eq!(
        document["questions"]["urgent"]["objective_applies"],
        json!(false)
    );
    assert_eq!(document["questions"]["urgent"]["threshold"], json!(null));
    assert_eq!(
        document["questions"]["team"]["objective_applies"],
        json!(true)
    );
    assert!(document["questions"]["team"]["threshold"].is_number());
    assert!(document["questions"]["severity"]["threshold"].is_number());
}

#[test]
fn eval_text_output_names_the_objective_beside_every_threshold() {
    // A threshold is optimal only with respect to what it was optimized for. Printing
    // one bare is how it later gets quoted as a property of the question.
    let (request, dataset) = eval_mixed_fixture();
    let api = MockApi::start(vec![Reply::ok(mixed_answer("billing", 1))]);

    let mut command = jev_authed();
    command.args([
        "eval",
        "-r",
        request.path().to_str().unwrap(),
        "-d",
        dataset.path().to_str().unwrap(),
        "--endpoint",
        &api.endpoint(),
        "-j",
        "1",
        "--objective",
        "min-accuracy",
        "--target",
        "0.3",
        "--no-split",
    ]);
    let assert = command.assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    assert!(stdout.contains("min-accuracy"), "{stdout}");
    assert!(stdout.contains("--require"), "{stdout}");
    assert!(
        !stdout.to_lowercase().contains("optimal"),
        "the output called a threshold optimal:\n{stdout}"
    );
    // The standing caveat, on every run.
    assert!(
        stdout.contains("does not") && stdout.contains("transfer"),
        "{stdout}"
    );
    // A noul has no confidence, so a confidence objective says so rather than inventing
    // a number.
    assert!(stdout.contains("does not apply to a noul"), "{stdout}");
    assert_no_canary(assert.get_output());
}

#[test]
fn eval_text_output_says_a_noul_has_no_accuracy_without_a_threshold() {
    // Accuracy is a property of a decision, and a noul is a probability until a cut
    // makes one. Omitting the line silently, or filling it in at 0.5, would both be
    // worse than saying what is missing.
    let points = [(0.9, true), (0.1, false)];
    let (request, dataset, replies) = eval_fixture(&points);
    let api = MockApi::start(replies);

    let assert = jev_authed()
        .args([
            "eval",
            "-r",
            request.path().to_str().unwrap(),
            "-d",
            dataset.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "-j",
            "1",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("needs a threshold"), "{stdout}");
    assert!(stdout.contains("brier score"), "{stdout}");
}

#[test]
fn eval_limit_caps_the_run_and_the_fingerprint_describes_only_what_was_measured() {
    // A fingerprint naming six rows when two were measured is a report that cannot be
    // compared with anything.
    let (request, dataset) = eval_mixed_fixture();
    let api = MockApi::start(vec![Reply::ok(mixed_answer("billing", 1))]);

    let full = json_stdout(
        eval_command(&api, &request, &dataset)
            .assert()
            .success()
            .get_output(),
    );
    let api = MockApi::start(vec![Reply::ok(mixed_answer("billing", 1))]);
    let limited = json_stdout(
        eval_command(&api, &request, &dataset)
            .args(["--limit", "2"])
            .assert()
            .success()
            .get_output(),
    );

    assert_eq!(full["dataset"]["rows"], json!(6));
    assert_eq!(limited["dataset"]["rows"], json!(2));
    assert_eq!(limited["questions"]["urgent"]["n"], json!(2));
    assert_ne!(
        full["dataset"]["fingerprint"], limited["dataset"]["fingerprint"],
        "the fingerprint did not change when the measured rows did"
    );
    assert_eq!(api.hits(), 2, "--limit did not stop the requests");
}

#[test]
fn eval_show_rows_lists_the_reported_rows_and_only_those() {
    let (request, dataset) = eval_mixed_fixture();
    let api = MockApi::start(vec![Reply::ok(mixed_answer("billing", 1))]);

    let document = json_stdout(
        eval_command(&api, &request, &dataset)
            .args(["--show-rows", "--objective", "maximize-f1"])
            .assert()
            .success()
            .get_output(),
    );
    let reported = document["split"]["reported_rows"].as_u64().unwrap();
    let rows = document["questions"]["urgent"]["rows"].as_array().unwrap();
    assert_eq!(
        rows.len() as u64,
        reported,
        "--show-rows listed rows the report does not cover"
    );
    for row in rows {
        assert!(row["id"].is_string(), "{row}");
        assert!(row["label"].is_boolean(), "{row}");
    }
    // Without the flag there is no per-row detail at all.
    let api = MockApi::start(vec![Reply::ok(mixed_answer("billing", 1))]);
    let plain = json_stdout(
        eval_command(&api, &request, &dataset)
            .assert()
            .success()
            .get_output(),
    );
    assert!(
        plain["questions"]["urgent"].get("rows").is_none(),
        "{plain}"
    );
}

#[test]
fn eval_refuses_a_missing_dataset_and_a_nonsensical_split() {
    let (request, dataset) = eval_mixed_fixture();

    jev_authed()
        .args(["eval", "-r", request.path().to_str().unwrap()])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("needs labelled examples"));

    let split = |fraction: &str| {
        jev_authed()
            .args([
                "eval",
                "-r",
                request.path().to_str().unwrap(),
                "-d",
                dataset.path().to_str().unwrap(),
                "--objective",
                "maximize-f1",
                "--test-fraction",
                fraction,
            ])
            .assert()
            .code(2)
            .stderr(predicate::str::contains("strictly between 0 and 1"));
    };
    split("0");
    split("1");

    jev_authed()
        .args([
            "eval",
            "-r",
            request.path().to_str().unwrap(),
            "-d",
            dataset.path().to_str().unwrap(),
            "--limit",
            "0",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--limit must be at least 1"));
}

#[test]
fn eval_ignores_a_request_documents_own_state_and_says_so() {
    // The same promise `jev map` makes: one committed request file works with `ask`,
    // `map`, and `eval` alike, and the one it carries for `ask` must not silently become
    // every row's state here.
    let request = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        request.path(),
        json!({
            "state": "THE REQUEST FILE'S OWN STATE",
            "questions": {"urgent": {"type": "noul", "instructions": "urgent?"}}
        })
        .to_string(),
    )
    .unwrap();
    let dataset = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        dataset.path(),
        json!({"schema": "jev.eval.row/v1", "id": "1", "state": "the row's own state",
               "labels": {"urgent": true}})
        .to_string(),
    )
    .unwrap();
    let api = MockApi::start(vec![Reply::ok(noul_at(0.9))]);

    let assert = eval_command(&api, &request, &dataset).assert().success();
    assert!(
        String::from_utf8_lossy(&assert.get_output().stderr).contains("is ignored"),
        "{}",
        String::from_utf8_lossy(&assert.get_output().stderr)
    );
    let sent: serde_json::Value = serde_json::from_str(&api.requests()[0].body).unwrap();
    assert_eq!(sent["state"], json!("the row's own state"));
}

#[test]
fn eval_stops_the_run_when_the_credential_is_rejected() {
    // Per-credential, not per-row: carrying on would send one doomed request per
    // remaining row and then report the result as a partial batch, so a CI job branching
    // on exit 3 to re-authenticate would never see it.
    let points: Vec<(f64, bool)> = (0..20).map(|i| (f64::from(i) / 20.0, i % 2 == 0)).collect();
    let (request, dataset, _) = eval_fixture(&points);
    let api = MockApi::start(vec![Reply::status(
        401,
        r#"{"error":{"message":"bad key"}}"#,
    )]);

    let assert = eval_command(&api, &request, &dataset).assert().code(3);
    assert!(
        api.hits() < points.len(),
        "the batch kept going after the credential was rejected: {} requests",
        api.hits()
    );
    assert_no_canary(assert.get_output());
}

#[test]
fn eval_refuses_a_report_that_would_overwrite_one_of_its_inputs() {
    // The report is written with `truncate`, and every other path here is a file the run
    // has already read by the time it is written. Without this guard,
    // `--report data.jsonl` read the dataset, measured it, and then destroyed it --
    // silently, and after the user had paid for the requests.
    let (request, dataset) = eval_mixed_fixture();
    let refuse = |report: &std::path::Path, flag: &str| {
        jev_authed()
            .args([
                "eval",
                "-r",
                request.path().to_str().unwrap(),
                "-d",
                dataset.path().to_str().unwrap(),
                "--report",
                report.to_str().unwrap(),
            ])
            .assert()
            .code(2)
            .stderr(predicate::str::contains("name the same path"))
            .stderr(predicate::str::contains(flag));
    };
    refuse(dataset.path(), "--dataset");
    refuse(request.path(), "--request");

    // The dataset is untouched: the refusal happens before anything is opened for
    // writing, which is the property that matters.
    assert!(
        std::fs::read_to_string(dataset.path())
            .unwrap()
            .contains("jev.eval.row/v1"),
        "the dataset was damaged by a refused run"
    );
}

#[test]
fn eval_reports_the_rows_it_actually_reached_and_the_ones_it_did_not() {
    // `--fail-fast` stops the run, and the report must say so rather than presenting
    // whatever finished first as the whole measurement. The same field is what an
    // interrupted run sets.
    let points = [(0.9, true), (0.1, false), (0.5, true), (0.4, false)];
    let (request, dataset, _) = eval_fixture(&points);
    let api = MockApi::start(vec![
        Reply::ok(noul_at(0.9)),
        Reply::status(500, r#"{"error":{"message":"boom"}}"#),
    ]);

    let assert = eval_command(&api, &request, &dataset)
        .args(["--fail-fast", "--retries", "0"])
        .assert()
        .code(5);
    let document = json_stdout(assert.get_output());
    assert_eq!(document["rows"]["total"], json!(4));
    assert!(
        document["rows"]["evaluated"].as_u64().unwrap() < 4,
        "{document}"
    );
    assert_eq!(document["rows"]["stopped_early"], json!(true));
    assert_eq!(document["rows"]["interrupted"], json!(false));
    // The caveat travels with the document, not only on stderr, so a `--report` file
    // read back later still says the numbers cover only part of the dataset.
    assert!(
        document["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning
                .as_str()
                .unwrap_or("")
                .contains("did not reach every row")),
        "{document}"
    );
}

#[test]
fn eval_counts_labelled_rows_over_the_same_set_it_scores() {
    // `labelled` used to count the whole dataset while `n` counted only the reported
    // side, so a perfectly healthy 70/30 split reported `labelled: 100, n: 30` with zero
    // failures -- which the schema's own rule reads as seventy rows having failed.
    let points: Vec<(f64, bool)> = (0..40)
        .map(|index| (f64::from(index) / 40.0, index % 3 == 0))
        .collect();
    let (request, dataset, replies) = eval_fixture(&points);
    let api = MockApi::start(replies);

    let document = json_stdout(
        eval_command(&api, &request, &dataset)
            .args(["--objective", "maximize-f1"])
            .assert()
            .get_output(),
    );
    let urgent = &document["questions"]["urgent"];
    assert_eq!(document["rows"]["failed"], json!(0));
    assert_eq!(
        urgent["labelled"], urgent["n"],
        "with no failures these must agree: {urgent}"
    );
    assert_eq!(urgent["n"], document["split"]["reported_rows"]);
}

#[test]
fn eval_does_not_invent_a_target_for_an_objective_that_takes_none() {
    // No calibration row is a positive example, so recall -- and therefore F1 -- is
    // undefined at every cut and no threshold can be scored. The message used to say
    // "no threshold reaches --target 0", naming a flag `maximize-f1` explicitly
    // refuses to accept.
    let points = [(0.9, false), (0.8, false), (0.2, false), (0.1, false)];
    let (request, dataset, replies) = eval_fixture(&points);
    let api = MockApi::start(replies);

    let assert = jev_authed()
        .args([
            "eval",
            "-r",
            request.path().to_str().unwrap(),
            "-d",
            dataset.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "-j",
            "1",
            "--objective",
            "maximize-f1",
            "--no-split",
        ])
        .assert()
        .code(1);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&assert.get_output().stdout),
        String::from_utf8_lossy(&assert.get_output().stderr)
    );
    assert!(
        !combined.contains("--target"),
        "a target was named for an objective that takes none:\n{combined}"
    );
    assert!(
        combined.contains("only one of the two labels"),
        "{combined}"
    );
}

#[test]
fn eval_never_reports_a_threshold_chosen_from_no_observations() {
    // A coverage sweep always carries the synthetic cut at zero, even over an empty set.
    // Without a guard, a question whose calibration rows are all unlabelled came back
    // `threshold: 0.0, threshold_reachable: true` -- a number presented as chosen, from
    // no data at all.
    let request = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        request.path(),
        json!({"team": {"type": "choice", "instructions": "Which?",
               "criteria": {"a": "first", "b": "second"}}})
        .to_string(),
    )
    .unwrap();
    // Every row is on the reported side, so the calibration side has no observations.
    let dataset = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        dataset.path(),
        json!({"schema": "jev.eval.row/v1", "id": "1", "state": "s", "labels": {"team": "a"}})
            .to_string(),
    )
    .unwrap();
    let api = MockApi::start(vec![Reply::ok(
        json!({"model": "jev-1.13.0", "answers": {"team": {"type": "choice", "choice": "a",
               "confidence": 0.9, "probabilities": {"a": 0.9, "b": 0.1}}}, "usage": {}})
        .to_string(),
    )]);

    let assert = eval_command(&api, &request, &dataset)
        .args([
            "--objective",
            "target-coverage",
            "--target",
            "0.8",
            "--test-fraction",
            "0.99",
        ])
        .assert();
    let document = json_stdout(assert.get_output());
    let team = &document["questions"]["team"];
    assert_eq!(document["split"]["calibration_rows"], json!(0));
    assert_eq!(team["threshold"], json!(null), "{team}");
    assert_eq!(team["threshold_reachable"], json!(false), "{team}");
}

#[test]
fn eval_warns_when_the_threshold_came_from_a_handful_of_calibration_rows() {
    // The warning used to key on the *reported* rows, so a cut fitted to three examples
    // passed without a word as long as the held-out side happened to be large.
    let points: Vec<(f64, bool)> = (0..40)
        .map(|index| (f64::from(index) / 40.0, index % 2 == 0))
        .collect();
    let (request, dataset, replies) = eval_fixture(&points);
    let api = MockApi::start(replies);

    let document = json_stdout(
        eval_command(&api, &request, &dataset)
            .args(["--objective", "maximize-f1"])
            .assert()
            .get_output(),
    );
    let warnings: Vec<&str> = document["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("calibration row(s)")),
        "{warnings:?}"
    );
}

#[test]
#[cfg(unix)]
fn an_interrupted_eval_exits_130_rather_than_reporting_a_partial_measurement_as_success() {
    // The batch driver reports that it stopped; `jev eval` used to throw that away. A
    // user who hit Ctrl-C got a report built from whatever finished first, exit 0, and
    // no indication that anything was missing -- and if `--report` was given, that
    // report was persisted looking complete.
    let api = MockApi::slow(
        vec![Reply::ok(noul_at(0.9))],
        std::time::Duration::from_millis(150),
    );
    let points: Vec<(f64, bool)> = (0..40)
        .map(|index: u32| (f64::from(index) / 40.0, index.is_multiple_of(2)))
        .collect();
    let (request, dataset, _) = eval_fixture(&points);
    let directory = tempfile::tempdir().unwrap();
    let report = directory.path().join("report.json");

    let child = jev_spawnable()
        .args([
            "eval",
            "-r",
            request.path().to_str().unwrap(),
            "-d",
            dataset.path().to_str().unwrap(),
            "--report",
            report.to_str().unwrap(),
            "-j",
            "1",
            "--endpoint",
            &api.endpoint(),
            "--output",
            "json",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn jev eval");

    // Let it get properly under way, so the signal lands mid-run rather than before any
    // row has been answered.
    let progressed = wait_until(std::time::Duration::from_secs(20), || api.hits() >= 2);
    assert!(progressed, "the run never reached a second row");

    // SIGINT, exactly as Ctrl-C would. Sent with `kill(1)` rather than through `libc`
    // because `unsafe_code` is forbidden workspace-wide, tests included.
    let signalled = std::process::Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("run kill(1)");
    assert!(signalled.success(), "could not signal the child");

    let output = child.wait_with_output().expect("reap the child");
    assert_no_canary(&output);
    assert_eq!(
        output.status.code(),
        Some(130),
        "an interrupted run did not report itself as interrupted: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // The report still exists and still says, in the document itself, that it is partial.
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    assert_eq!(written["rows"]["interrupted"], json!(true));
    assert_eq!(written["rows"]["stopped_early"], json!(true));
    assert!(written["rows"]["evaluated"].as_u64().unwrap() < 40);
    assert!(
        written["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning
                .as_str()
                .unwrap_or("")
                .contains("did not reach every row")),
        "{written}"
    );
}

#[test]
fn a_failed_map_row_reports_how_many_attempts_were_made() {
    // Every attempt may have reached the server and been billed; a failed row that
    // dropped the count left a user unable to say how many.
    let api = MockApi::start(vec![
        Reply::status(500, "{}").header("retry-after-ms", "1"),
        Reply::status(500, "{}").header("retry-after-ms", "1"),
    ]);
    let questions = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        questions.path(),
        json!({"urgent": {"type": "noul", "instructions": "Urgent?"}}).to_string(),
    )
    .unwrap();

    let assert = jev_authed()
        .args([
            "map",
            "-r",
            questions.path().to_str().unwrap(),
            "--endpoint",
            &api.endpoint(),
            "--retries",
            "1",
        ])
        .write_stdin("\"a\"\n")
        .assert()
        .code(5);

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let row: serde_json::Value = serde_json::from_str(stdout.lines().next().unwrap()).unwrap();
    assert_eq!(row["ok"], json!(false));
    assert_eq!(row["error"]["kind"], json!("unavailable"));
    assert_eq!(row["attempts"], json!(2), "{row}");
    assert_eq!(api.hits(), 2);
}
