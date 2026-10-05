//! Regression coverage for batch templates and ambiguous JSON rows.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "a panicking assertion is the correct failure mode inside a test binary"
)]
mod support;

use serde_json::{Value, json};
use support::{jev, json_stdout};

const PIXEL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6cS8AAAAASUVORK5CYII=";

fn file(text: &str) -> tempfile::NamedTempFile {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), text).unwrap();
    file
}

fn template(state: Option<Value>) -> tempfile::NamedTempFile {
    let mut value = json!({"questions":{"visible":{"type":"noul","instructions":"Visible?"}}});
    if let Some(state) = state {
        value["state"] = state;
    }
    file(&value.to_string())
}

fn batch(
    primitive: &str,
    request: &std::path::Path,
    input: &std::path::Path,
) -> assert_cmd::Command {
    let mut command = jev();
    command
        .args(["--dry-run", "-o", "json", primitive, "--request"])
        .arg(request);
    if primitive == "map" {
        command
            .arg("--input")
            .arg(input)
            .args(["--state-field", "state", "--id-field", "id"]);
    } else {
        command.arg("--dataset").arg(input);
    }
    command
}

fn row(primitive: &str, state: Value) -> Value {
    let mut row = json!({"id":"one"});
    row["state"] = state;
    if primitive == "eval" {
        row["schema"] = json!("jev.eval.row/v1");
        row["labels"] = json!({"visible":true});
    }
    row
}

#[test]
fn batch_templates_ignore_semantically_invalid_state_and_warn() {
    for primitive in ["map", "eval"] {
        for ignored in [json!(""), json!(" \n"), Value::Null, json!(42)] {
            let request = template(Some(ignored));
            let input = file(&row(primitive, json!("row text")).to_string());
            let output = batch(primitive, request.path(), input.path())
                .assert()
                .success();
            assert_eq!(
                json_stdout(output.get_output())["sample"][0]["body"]["state"],
                "row text"
            );
            let stderr = String::from_utf8_lossy(&output.get_output().stderr);
            assert!(stderr.contains("`state` is ignored"), "{stderr}");
        }
    }
}

#[test]
fn blank_template_state_allows_valid_cloudflare_row_images() {
    for primitive in ["map", "eval"] {
        let request = template(Some(json!("")));
        let mut input_row = row(primitive, json!(""));
        input_row["images"] = json!([PIXEL]);
        let input = file(&input_row.to_string());
        let mut command = batch(primitive, request.path(), input.path());
        command.args([
            "--provider",
            "cloudflare",
            "--cloudflare-account-id",
            "0123456789abcdef0123456789abcdef",
        ]);
        if primitive == "map" {
            command.args(["--images-field", "images"]);
        }
        let output = command.assert().success();
        let body = &json_stdout(output.get_output())["sample"][0]["body"];
        assert_eq!(body["state"], "");
        assert_eq!(body["images"][0]["content_type"], "image/png");
    }
}

fn duplicate_content_documents(duplicate: &str) -> [(String, String); 5] {
    [
        (
            duplicate.to_owned(),
            r#"{"q":{"type":"noul","instructions":"?"}}"#.to_owned(),
        ),
        (
            "\"text\"".to_owned(),
            format!(r#"{{"q":{{"type":"noul","instructions":{duplicate}}}}}"#),
        ),
        (
            "\"text\"".to_owned(),
            format!(
                r#"{{"q":{{"type":"noul","instructions":"?","criteria":{{"true":{duplicate}}}}}}}"#
            ),
        ),
        (
            "\"text\"".to_owned(),
            format!(
                r#"{{"q":{{"type":"choice","instructions":"?","criteria":{{"a":{duplicate},"b":null}}}}}}"#
            ),
        ),
        (
            "\"text\"".to_owned(),
            format!(
                r#"{{"q":{{"type":"score","instructions":"?","criteria":[{duplicate},"other"]}}}}"#
            ),
        ),
    ]
}

#[test]
fn explicit_json_content_rejects_nested_duplicate_keys_before_preview() {
    const DUPLICATE: &str = r#"{"flag":null,"flag":false}"#;
    const IN_ARRAY: &str = r#"[{"flag":null,"flag":false}]"#;
    for provider in [
        "typesafe",
        "huggingface",
        "ollama",
        "llamacpp",
        "cloudflare",
    ] {
        let configure = |command: &mut assert_cmd::Command| {
            command.args(["--provider", provider, "--dry-run"]);
            if provider == "cloudflare" {
                command.args([
                    "--cloudflare-account-id",
                    "0123456789abcdef0123456789abcdef",
                ]);
            }
        };
        for duplicate in [DUPLICATE, IN_ARRAY] {
            for source in ["--state-json", "--state-json-file"] {
                let state = file(duplicate);
                let mut command = jev();
                configure(&mut command);
                command.args(["noul", "?"]);
                if source == "--state-json" {
                    command.args([source, duplicate]);
                } else {
                    command.arg(source).arg(state.path());
                }
                let output = command.assert().code(2);
                let error = String::from_utf8_lossy(&output.get_output().stderr);
                assert!(
                    error.contains("duplicate") && error.contains("flag"),
                    "{provider}/{source}: {error}"
                );
                assert!(output.get_output().stdout.is_empty());
            }
            let levels = file(&format!("[{duplicate},\"other\"]"));
            let options = file(&format!("{{\"a\":{duplicate},\"b\":null}}"));
            for (primitive, flag, path) in [
                ("score", "--levels-file", levels.path()),
                ("choice", "--options-file", options.path()),
            ] {
                let mut command = jev();
                configure(&mut command);
                let output = command
                    .args([primitive, "?", "--state", "text", flag])
                    .arg(path)
                    .assert()
                    .code(2);
                let error = String::from_utf8_lossy(&output.get_output().stderr);
                assert!(
                    error.contains("duplicate") && error.contains("flag"),
                    "{provider}/{flag}: {error}"
                );
                assert!(output.get_output().stdout.is_empty());
            }
            for (state, questions) in duplicate_content_documents(duplicate) {
                let request = file(&format!(r#"{{"state":{state},"questions":{questions}}}"#));
                let mut command = jev();
                configure(&mut command);
                let output = command
                    .args(["ask", "--request"])
                    .arg(request.path())
                    .assert()
                    .code(2);
                let error = String::from_utf8_lossy(&output.get_output().stderr);
                assert!(
                    error.contains("duplicate") && error.contains("flag"),
                    "{provider}: {error}"
                );
                assert!(output.get_output().stdout.is_empty());

                if state == "\"text\"" {
                    let bare = file(&questions);
                    let mut command = jev();
                    configure(&mut command);
                    let output = command
                        .args(["ask", "--state", "text", "--request"])
                        .arg(bare.path())
                        .assert()
                        .code(2);
                    let error = String::from_utf8_lossy(&output.get_output().stderr);
                    assert!(
                        error.contains("duplicate") && error.contains("flag"),
                        "{provider}/bare: {error}"
                    );
                    assert!(output.get_output().stdout.is_empty());
                }
            }
        }
    }
}

#[test]
fn explicit_json_duplicate_diagnostics_sanitize_keys_and_omit_values() {
    let duplicate = r#"{"\u001b[2J\u202ehidden":"request-value-canary-do-not-echo","\u001b[2J\u202ehidden":false}"#;
    let levels = file(&format!("[{duplicate},\"other\"]"));
    let request = file(&format!(
        r#"{{"q":{{"type":"noul","instructions":{duplicate}}}}}"#
    ));
    for args in [
        vec!["noul", "?", "--state-json", duplicate],
        vec![
            "score",
            "?",
            "--state",
            "text",
            "--levels-file",
            levels.path().to_str().unwrap(),
        ],
        vec![
            "ask",
            "--state",
            "text",
            "--request",
            request.path().to_str().unwrap(),
        ],
    ] {
        let output = jev().args(["--dry-run"]).args(args).assert().code(2);
        let error = String::from_utf8_lossy(&output.get_output().stderr);
        assert!(error.contains("duplicate"), "{error}");
        assert!(
            !error.contains('\u{1b}') && !error.contains('\u{202e}'),
            "{error}"
        );
        assert!(
            !error.contains("request-value-canary-do-not-echo"),
            "{error}"
        );
        assert!(output.get_output().stdout.is_empty());
    }
}

#[test]
fn batch_rows_reject_duplicate_keys_before_sending() {
    let request = template(None);
    for primitive in ["map", "eval"] {
        for (key, fields) in [
            (
                "schema",
                "\"schema\":\"bad\",\"schema\":\"jev.eval.row/v1\",\"id\":\"one\",\"state\":\"text\",\"labels\":{\"visible\":true}",
            ),
            (
                "id",
                "\"schema\":\"jev.eval.row/v1\",\"id\":\"other\",\"id\":\"one\",\"state\":\"text\",\"labels\":{\"visible\":true}",
            ),
            (
                "state",
                "\"schema\":\"jev.eval.row/v1\",\"id\":\"one\",\"state\":\"other\",\"state\":\"text\",\"labels\":{\"visible\":true}",
            ),
            (
                "visible",
                "\"schema\":\"jev.eval.row/v1\",\"id\":\"one\",\"state\":\"text\",\"labels\":{\"visible\":false,\"visible\":true}",
            ),
            (
                "message",
                "\"schema\":\"jev.eval.row/v1\",\"id\":\"one\",\"state\":[{\"message\":\"other\",\"message\":\"text\"}],\"labels\":{\"visible\":true}",
            ),
        ] {
            let input = file(&format!("{{{fields}}}"));
            let output = batch(primitive, request.path(), input.path())
                .assert()
                .code(2);
            let stderr = String::from_utf8_lossy(&output.get_output().stderr);
            assert!(
                stderr.contains("duplicate") && stderr.contains(key),
                "{stderr}"
            );
            assert!(output.get_output().stdout.is_empty());
        }
    }
}

#[test]
fn duplicate_row_diagnostics_sanitize_keys_and_do_not_echo_rows() {
    let request = template(None);
    for primitive in ["map", "eval"] {
        let input = file(
            "{\"schema\":\"jev.eval.row/v1\",\"id\":\"one\",\"state\":{\"x\\u001b[2J\":\"private-row-content\",\"x\\u001b[2J\":\"other\"},\"labels\":{\"visible\":true}}",
        );
        let output = batch(primitive, request.path(), input.path())
            .assert()
            .code(2);
        let stderr = String::from_utf8_lossy(&output.get_output().stderr);
        assert!(stderr.contains("duplicate"), "{stderr}");
        assert!(!stderr.contains('\u{1b}'));
        assert!(!stderr.contains("private-row-content"));
    }
}

#[test]
fn ignored_template_state_does_not_relax_ask_or_row_state_validation() {
    let request = template(Some(json!("")));
    jev()
        .args(["--dry-run", "ask", "--request"])
        .arg(request.path())
        .assert()
        .code(2);
    for primitive in ["map", "eval"] {
        let input = file(&row(primitive, json!("")).to_string());
        let output = batch(primitive, request.path(), input.path())
            .assert()
            .code(2);
        let stderr = String::from_utf8_lossy(&output.get_output().stderr);
        assert!(stderr.contains("line 1"), "{stderr}");
    }
}

#[test]
fn ignored_template_state_preserves_selected_row_media_and_provider_validation() {
    let request = template(Some(json!("")));
    for primitive in ["map", "eval"] {
        for (provider, state, images, expected) in [
            (
                "cloudflare",
                "row text",
                json!(["invalid image"]),
                "invalid embedded images",
            ),
            ("cloudflare", "", json!([]), "line 1"),
            ("typesafe", "row text", json!([PIXEL]), "image"),
            ("ollama", "", json!([PIXEL]), "line 1"),
        ] {
            let mut input_row = row(primitive, json!(state));
            input_row["images"] = images;
            let input = file(&input_row.to_string());
            let mut command = batch(primitive, request.path(), input.path());
            command.args(["--provider", provider]);
            if provider == "cloudflare" {
                command.args([
                    "--cloudflare-account-id",
                    "0123456789abcdef0123456789abcdef",
                ]);
            }
            if primitive == "map" {
                command.args(["--images-field", "images"]);
            }
            let output = command.assert().code(2);
            let stderr = String::from_utf8_lossy(&output.get_output().stderr);
            assert!(stderr.contains(expected), "{stderr}");
            assert!(stderr.contains("`state` is ignored"), "{stderr}");
        }
    }
}

#[test]
fn batch_row_order_and_map_unknown_fields_remain_compatible() {
    let request = template(None);
    for primitive in ["map", "eval"] {
        let mut first = row(
            primitive,
            json!({"messages":[null, true, 1, -2, 0.5, "text"]}),
        );
        first["id"] = json!("zebra");
        let mut second = row(primitive, json!("another row"));
        second["id"] = json!("apple");
        if primitive == "map" {
            first["unknown"] = json!({"future":true});
        }
        let input = file(&format!("{first}\n{second}"));
        let output = batch(primitive, request.path(), input.path())
            .assert()
            .success();
        let preview = json_stdout(output.get_output());
        assert_eq!(preview["sample"][0]["id"], "zebra");
        assert_eq!(preview["sample"][1]["id"], "apple");
        assert_eq!(preview["sample"][0]["body"]["state"], first["state"]);
    }
}

#[test]
fn batch_json_still_enforces_depth_and_input_size_bounds() {
    let request = template(None);
    for primitive in ["map", "eval"] {
        for depth in [65, 140] {
            let deep = format!("{}\"text\"{}", "[".repeat(depth), "]".repeat(depth));
            let input = file(&format!(
                "{{\"schema\":\"jev.eval.row/v1\",\"id\":\"one\",\"state\":{deep},\"labels\":{{\"visible\":true}}}}"
            ));
            batch(primitive, request.path(), input.path())
                .assert()
                .code(2);
        }
        let input = file(&row(primitive, json!("x".repeat(2000))).to_string());
        batch(primitive, request.path(), input.path())
            .args(["--max-input-bytes", "1024"])
            .assert()
            .code(2);
    }
}

#[test]
fn media_kwargs_flags_reject_duplicate_keys_at_every_nesting_level() {
    for (raw, key) in [
        (r#"{"max_pixels":65536,"max_pixels":32768}"#, "max_pixels"),
        (r#"{"max_pixels":{"bound":1,"bound":2}}"#, "bound"),
        (r#"{"max_pixels":[{"bound":1,"bound":2}]}"#, "bound"),
    ] {
        let output = jev()
            .args([
                "--provider",
                "huggingface",
                "--dry-run",
                "--media-kwargs",
                raw,
                "noul",
                "Visible?",
                "--state",
                "row text",
            ])
            .assert()
            .code(2);
        let stderr = String::from_utf8_lossy(&output.get_output().stderr);
        assert!(
            stderr.contains("duplicate") && stderr.contains(key),
            "{stderr}"
        );
        assert!(output.get_output().stdout.is_empty());
    }
}

#[test]
fn media_kwargs_documents_reject_duplicates_across_ask_map_and_eval() {
    for primitive in ["ask", "map", "eval"] {
        for (raw, key) in [
            (r#"{"max_pixels":65536,"max_pixels":32768}"#, "max_pixels"),
            (r#"{"max_pixels":{"bound":1,"bound":2}}"#, "bound"),
            (r#"{"max_pixels":[{"bound":1,"bound":2}]}"#, "bound"),
        ] {
            let request = file(&format!(
                r#"{{"state":"row text","media_kwargs":{raw},"questions":{{"visible":{{"type":"noul","instructions":"Visible?"}}}}}}"#
            ));
            let input = file(&row(primitive, json!("row text")).to_string());
            let mut command = if primitive == "ask" {
                let mut command = jev();
                command
                    .args(["--dry-run", "ask", "--request"])
                    .arg(request.path());
                command
            } else {
                batch(primitive, request.path(), input.path())
            };
            let output = command.args(["--provider", "huggingface"]).assert().code(2);
            let stderr = String::from_utf8_lossy(&output.get_output().stderr);
            assert!(
                stderr.contains("duplicate") && stderr.contains(key),
                "{stderr}"
            );
            assert!(output.get_output().stdout.is_empty());
        }
    }
}

#[test]
fn media_kwargs_duplicate_diagnostics_sanitize_keys_and_omit_values() {
    let raw = r#"{"x\u001b[2J":"private-processor-value","x\u001b[2J":false}"#;
    let request = file(&format!(
        r#"{{"state":"row text","media_kwargs":{raw},"questions":{{"visible":{{"type":"noul","instructions":"Visible?"}}}}}}"#
    ));
    for document in [false, true] {
        let mut command = jev();
        command.args(["--provider", "huggingface", "--dry-run"]);
        if document {
            command.args(["ask", "--request"]).arg(request.path());
        } else {
            command.args([
                "--media-kwargs",
                raw,
                "noul",
                "Visible?",
                "--state",
                "row text",
            ]);
        }
        let output = command.assert().code(2);
        let stderr = String::from_utf8_lossy(&output.get_output().stderr);
        assert!(stderr.contains("duplicate"), "{stderr}");
        assert!(!stderr.contains('\u{1b}'));
        assert!(!stderr.contains("private-processor-value"));
    }
}

#[test]
fn media_kwargs_unique_values_and_optional_document_fields_keep_their_meaning() {
    let unique = json!({"max_pixels":65536,"do_sample_frames":false});
    let output = jev()
        .args([
            "--provider",
            "huggingface",
            "--dry-run",
            "-o",
            "json",
            "--media-kwargs",
        ])
        .arg(unique.to_string())
        .args(["noul", "Visible?", "--state", "row text"])
        .assert()
        .success();
    assert_eq!(
        json_stdout(output.get_output())["body"]["media_kwargs"],
        unique
    );
    for kwargs in [None, Some(Value::Null), Some(unique)] {
        let mut document = json!({"state":"row text", "questions":{
            "visible":{"type":"noul","instructions":"Visible?"}}});
        if let Some(value) = &kwargs {
            document["media_kwargs"] = value.clone();
        }
        let request = file(&document.to_string());
        let output = jev()
            .args([
                "--provider",
                "huggingface",
                "--dry-run",
                "-o",
                "json",
                "ask",
                "--request",
            ])
            .arg(request.path())
            .assert()
            .success();
        let preview = json_stdout(output.get_output());
        if let Some(value) = kwargs.filter(|value| !value.is_null()) {
            assert_eq!(preview["body"]["media_kwargs"], value);
        } else {
            assert!(preview["body"].get("media_kwargs").is_none());
        }
    }
}
