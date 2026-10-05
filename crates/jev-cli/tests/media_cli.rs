//! Media and provider options across the inference surfaces.
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

fn request_file(value: &Value) -> tempfile::NamedTempFile {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), value.to_string()).unwrap();
    file
}

#[test]
fn map_preserves_template_capacity_options_in_its_exact_preview() {
    let request = request_file(&json!({"options":{"rejectIfBusy":true},
        "questions":{"visible":{"type":"noul","instructions":"Visible?"}}}));
    let input = request_file(&json!({"state":"a receipt"}));
    let output = jev()
        .args([
            "--provider",
            "cloudflare",
            "--cloudflare-account-id",
            "0123456789abcdef0123456789abcdef",
            "--dry-run",
            "-o",
            "json",
            "map",
            "--request",
        ])
        .arg(request.path())
        .arg("--input")
        .arg(input.path())
        .args(["--state-field", "state"])
        .assert()
        .success();
    assert_eq!(
        json_stdout(output.get_output())["sample"][0]["body"]["options"]["rejectIfBusy"],
        true
    );
}

const PIXEL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6cS8AAAAASUVORK5CYII=";

#[test]
fn publisher_score_supports_large_bounded_scales_and_negative_numeric_state() {
    for count in [26, 255] {
        let levels = vec![json!("level"); count];
        let file =
            request_file(&json!({"state":-1,"questions":{"q":{"type":"score","criteria":levels}}}));
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
            .arg(file.path())
            .assert()
            .success();
        assert_eq!(
            json_stdout(output.get_output())["body"]["questions"]["q"]["criteria"]
                .as_array()
                .unwrap()
                .len(),
            count
        );
        let levels_file = request_file(&json!(levels));
        let output = jev()
            .args([
                "--provider",
                "huggingface",
                "--dry-run",
                "-o",
                "json",
                "score",
                "?",
                "--state-json=-1",
                "--levels-file",
            ])
            .arg(levels_file.path())
            .assert()
            .success();
        assert_eq!(json_stdout(output.get_output())["body"]["state"], -1);
    }
    let file = request_file(
        &json!({"state":-1,"questions":{"q":{"type":"score","criteria":vec![json!("level");256]}}}),
    );
    let output = jev()
        .args(["--provider", "huggingface", "--dry-run", "ask", "--request"])
        .arg(file.path())
        .assert()
        .code(2);
    assert!(String::from_utf8_lossy(&output.get_output().stderr).contains("at most 255"));
}

#[test]
fn publisher_json_content_preserves_scalars_null_criteria_and_instruction_fallback() {
    for state in [
        json!(0),
        json!(1.5),
        json!(true),
        json!(false),
        json!(null),
        json!(""),
        json!("   "),
    ] {
        for instructions in [
            json!(null),
            json!(""),
            json!(false),
            json!(42),
            json!("   "),
        ] {
            let request = request_file(&json!({"state":state,"questions":{
                "truth":{"type":"noul","instructions":instructions,"criteria":{"true":null,"false":false}},
                "rating":{"type":"score","instructions":true,"criteria":[null,0,false,"","   "]},
                "selection":{"type":"choice","instructions":0,"criteria":{"a":false,"b":42}}
            }}));
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
            assert_eq!(preview["body"]["state"], state);
            let expected = if instructions.is_null() || instructions == json!("") {
                json!("truth")
            } else {
                instructions
            };
            assert_eq!(
                preview["body"]["questions"]["truth"]["instructions"],
                expected
            );
            assert_eq!(
                preview["body"]["questions"]["truth"]["criteria"],
                json!({"true":null,"false":false})
            );
            assert_eq!(
                preview["body"]["questions"]["rating"]["criteria"],
                json!([null, 0, false, "", "   "])
            );
            assert_eq!(
                preview["body"]["questions"]["selection"]["criteria"],
                json!({"a":false,"b":42})
            );
        }
    }
}

#[test]
fn publisher_scalar_state_flows_through_flags_files_map_and_eval() {
    for state in [
        json!(null),
        json!(false),
        json!(1.5),
        json!(""),
        json!("   "),
    ] {
        let state_file = request_file(&state);
        for source in ["--state-json", "--state-json-file"] {
            let mut command = jev();
            command.args([
                "--provider",
                "huggingface",
                "--dry-run",
                "-o",
                "json",
                "noul",
                "?",
            ]);
            if source == "--state-json" {
                command.args([source, &state.to_string()]);
            } else {
                command.arg(source).arg(state_file.path());
            }
            let output = command.assert().success();
            assert_eq!(json_stdout(output.get_output())["body"]["state"], state);
        }
        let template =
            request_file(&json!({"questions":{"q":{"type":"noul","instructions":null}}}));
        let rows = request_file(
            &json!({"schema":"jev.eval.row/v1","id":"row","state":state,"labels":{"q":false}}),
        );
        for primitive in ["map", "eval"] {
            let mut command = jev();
            command
                .args([
                    "--provider",
                    "huggingface",
                    "--dry-run",
                    "-o",
                    "json",
                    primitive,
                    "--request",
                ])
                .arg(template.path());
            if primitive == "map" {
                command
                    .arg("--input")
                    .arg(rows.path())
                    .args(["--state-field", "state"]);
            } else {
                command.arg("--dataset").arg(rows.path());
            }
            let output = command.assert().success();
            assert_eq!(
                json_stdout(output.get_output())["sample"][0]["body"]["state"],
                state
            );
        }
    }
}

#[test]
fn publisher_text_flags_and_json_criteria_files_keep_empty_and_whitespace_values() {
    let options = request_file(&json!({"a":null,"b":false,"c":42,"d":"","e":"   "}));
    let levels = request_file(&json!([null, false, 42, "", "   "]));
    for primitive in ["noul", "choice", "score"] {
        let mut command = jev();
        command.args([
            "--provider",
            "huggingface",
            "--dry-run",
            "-o",
            "json",
            primitive,
            "",
            "--id",
            " q ",
            "--state",
            "",
        ]);
        if primitive == "choice" {
            command.arg("--options-file").arg(options.path());
        }
        if primitive == "score" {
            command.arg("--levels-file").arg(levels.path());
        }
        let output = command.assert().success();
        let body = &json_stdout(output.get_output())["body"];
        assert_eq!(body["state"], "");
        assert_eq!(body["questions"]["q"]["instructions"], "q");
        if primitive == "choice" {
            assert_eq!(
                body["questions"]["q"]["criteria"],
                json!({"a":null,"b":false,"c":42,"d":"","e":"   "})
            );
        }
        if primitive == "score" {
            assert_eq!(
                body["questions"]["q"]["criteria"],
                json!([null, false, 42, "", "   "])
            );
        }
    }
    for args in [
        vec!["choice", "   ", "--option", "a=", "--option", "b=   "],
        vec!["score", "   ", "--level", "", "--level", "   "],
        vec!["noul", "   ", "--true", "", "--false", "   "],
    ] {
        let output = jev()
            .args(["--provider", "huggingface", "--dry-run", "-o", "json"])
            .args(args)
            .args(["--state", "   "])
            .assert()
            .success();
        let body = &json_stdout(output.get_output())["body"];
        assert_eq!(body["state"], "   ");
        assert_eq!(body["questions"]["answer"]["instructions"], "   ");
    }
}

#[test]
fn publisher_noul_defaults_distinguish_empty_criteria_from_an_explicit_null_side() {
    for criteria in [
        Value::Null,
        json!({}),
        json!({"true":null}),
        json!({"false":null}),
    ] {
        let file = request_file(
            &json!({"state":null,"questions":{"q":{"type":"noul","criteria":criteria}}}),
        );
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
            .arg(file.path())
            .assert()
            .success();
        let question = &json_stdout(output.get_output())["body"]["questions"]["q"];
        assert_eq!(question["instructions"], "q");
        if criteria.is_null() || criteria == json!({}) {
            assert!(question.get("criteria").is_none());
        } else {
            assert_eq!(question["criteria"], criteria);
        }
    }
    let output = jev()
        .args(["--provider", "huggingface", "--dry-run", "noul", "?"])
        .write_stdin("")
        .assert()
        .code(2);
    assert!(String::from_utf8_lossy(&output.get_output().stderr).contains("empty"));
}

#[test]
fn publisher_content_is_refused_by_other_providers_before_any_credentials() {
    for provider in ["typesafe", "cloudflare", "ollama", "llamacpp"] {
        for request in [
            json!({"state":false,"questions":{"q":{"type":"noul","instructions":"?"}}}),
            json!({"state":"x","questions":{"q":{"type":"noul","instructions":false}}}),
            json!({"state":"x","questions":{"q":{"type":"score","instructions":"?","criteria":[null,false]}}}),
        ] {
            let file = request_file(&request);
            let mut command = jev();
            command.args(["--provider", provider]);
            if provider == "cloudflare" {
                command.args([
                    "--cloudflare-account-id",
                    "0123456789abcdef0123456789abcdef",
                ]);
            }
            let output = command
                .args(["--dry-run", "ask", "--request"])
                .arg(file.path())
                .assert()
                .code(2);
            assert!(output.get_output().stdout.is_empty());
            let error = String::from_utf8_lossy(&output.get_output().stderr);
            assert!(
                error.contains("expected a string, object, or array"),
                "{provider}: {error}"
            );
        }
    }
}

#[test]
fn cloudflare_scalar_commands_allow_explicit_blank_state_with_valid_images() {
    let image = jev_core::EmbeddedImage::from_data_url(PIXEL).unwrap();
    let image_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(image_file.path(), image.bytes()).unwrap();
    let empty_text = tempfile::NamedTempFile::new().unwrap();
    let empty_json = request_file(&json!(""));
    let commands: &[&[&str]] = &[
        &["noul", "Visible?"],
        &["choice", "Color?", "--option", "red", "--option", "blue"],
        &[
            "score",
            "Readable?",
            "--level",
            "unclear",
            "--level",
            "clear",
        ],
    ];
    let sources = [
        ("--state", "", "", ""),
        ("--state", "  \n", "", "  "),
        ("--state-file", empty_text.path().to_str().unwrap(), "", ""),
        ("--state-file", "-", "\n", ""),
        ("--state-json", "\"\"", "", ""),
        ("--state-json", "\"  \"", "", "  "),
        (
            "--state-json-file",
            empty_json.path().to_str().unwrap(),
            "",
            "",
        ),
        ("--state-json-file", "-", "\"\"", ""),
    ];
    for command in commands {
        for (flag, value, stdin, expected_state) in sources {
            let output = jev()
                .args([
                    "--provider",
                    "cloudflare",
                    "--cloudflare-account-id",
                    "0123456789abcdef0123456789abcdef",
                    "--model",
                    "clef",
                    "--dry-run",
                    "-o",
                    "json",
                    "--image",
                ])
                .arg(image_file.path())
                .args(*command)
                .args([flag, value])
                .write_stdin(stdin)
                .assert()
                .success();
            let preview = json_stdout(output.get_output());
            assert_eq!(preview["body"]["state"], expected_state);
            assert_eq!(
                preview["body"]["images"][0]["base64"],
                PIXEL.split_once(',').unwrap().1
            );
        }
    }
}

#[test]
fn cloudflare_ask_allows_blank_state_with_document_or_flag_images() {
    let image = jev_core::EmbeddedImage::from_data_url(PIXEL).unwrap();
    let image_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(image_file.path(), image.bytes()).unwrap();
    let full_request = request_file(&json!({"state":"", "images":[PIXEL],
        "questions":{"visible":{"type":"noul","instructions":"Visible?"}}}));
    let empty_request = request_file(&json!({"state":"",
        "questions":{"visible":{"type":"noul","instructions":"Visible?"}}}));
    let questions = request_file(&json!({
        "questions":{"visible":{"type":"noul","instructions":"Visible?"}}}));
    for (flag, file, image_flag, state_flag) in [
        ("--request", &full_request, false, false),
        ("--request", &empty_request, true, false),
        ("--questions", &questions, true, true),
    ] {
        let mut command = jev();
        command.args([
            "--provider",
            "cloudflare",
            "--cloudflare-account-id",
            "0123456789abcdef0123456789abcdef",
            "--dry-run",
            "-o",
            "json",
        ]);
        if image_flag {
            command.arg("--image").arg(image_file.path());
        }
        command.args(["ask", flag]).arg(file.path());
        if state_flag {
            command.args(["--state", ""]);
        }
        let output = command.assert().success();
        let preview = json_stdout(output.get_output());
        assert_eq!(preview["body"]["state"], "");
        assert_eq!(preview["body"]["images"][0]["content_type"], "image/png");
    }
}

#[test]
fn cloudflare_ask_state_flags_use_images_from_the_request_document() {
    let document = json!({"images":[PIXEL], "questions":{
        "visible":{"type":"noul","instructions":"Visible?"}}});
    let request = request_file(&document);
    let empty_text = tempfile::NamedTempFile::new().unwrap();
    let empty_json = request_file(&json!(""));
    let sources = [
        ("--state", "", ""),
        ("--state-file", empty_text.path().to_str().unwrap(), ""),
        ("--state-file", "-", ""),
        ("--state-json", "\"\"", ""),
        ("--state-json-file", empty_json.path().to_str().unwrap(), ""),
        ("--state-json-file", "-", "\"\""),
    ];
    for request_on_stdin in [false, true] {
        for (flag, value, state_stdin) in sources {
            // One stdin cannot carry both a request document and separate state.
            if request_on_stdin && value == "-" {
                continue;
            }
            let mut command = jev();
            command.args([
                "--provider",
                "cloudflare",
                "--cloudflare-account-id",
                "0123456789abcdef0123456789abcdef",
                "--dry-run",
                "-o",
                "json",
                "ask",
                "--request",
            ]);
            if request_on_stdin {
                command.arg("-").write_stdin(document.to_string());
            } else {
                command.arg(request.path()).write_stdin(state_stdin);
            }
            let output = command.args([flag, value]).assert().success();
            let preview = json_stdout(output.get_output());
            assert_eq!(preview["body"]["state"], "");
            assert_eq!(preview["body"]["images"][0]["content_type"], "image/png");
        }
    }
}

#[test]
fn cloudflare_batches_validate_flag_images_before_ignored_blank_template_state() {
    assert_flag_images_with_blank_template("map");
}

#[test]
fn cloudflare_eval_validates_flag_images_before_ignored_blank_template_state() {
    assert_flag_images_with_blank_template("eval");
}

fn assert_flag_images_with_blank_template(primitive: &str) {
    let image = jev_core::EmbeddedImage::from_data_url(PIXEL).unwrap();
    let image_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(image_file.path(), image.bytes()).unwrap();
    let request = request_file(&json!({"state":"", "questions":{
        "visible":{"type":"noul","instructions":"Visible?"}}}));
    let row = if primitive == "eval" {
        json!({"schema":"jev.eval.row/v1", "id":"receipt", "state":"row state", "labels":{"visible":true}})
    } else {
        json!({"state":"row state"})
    };
    let input = request_file(&row);
    let mut command = jev();
    command
        .args([
            "--provider",
            "cloudflare",
            "--cloudflare-account-id",
            "0123456789abcdef0123456789abcdef",
            "--dry-run",
            "-o",
            "json",
            "--image",
        ])
        .arg(image_file.path())
        .args([primitive, "--request"])
        .arg(request.path());
    if primitive == "map" {
        command
            .arg("--input")
            .arg(input.path())
            .args(["--state-field", "state"]);
    } else {
        command.arg("--dataset").arg(input.path());
    }
    let output = command.assert().success();
    let preview = json_stdout(output.get_output());
    assert_eq!(preview["sample"][0]["body"]["state"], "row state");
    assert_eq!(
        preview["sample"][0]["body"]["images"][0]["content_type"],
        "image/png"
    );
}

#[test]
fn cloudflare_batches_allow_blank_state_with_row_template_or_flag_images() {
    assert_cloudflare_image_only_batch("map");
}

#[test]
fn cloudflare_eval_allows_blank_state_with_row_template_or_flag_images() {
    assert_cloudflare_image_only_batch("eval");
}

fn assert_cloudflare_image_only_batch(primitive: &str) {
    let image = jev_core::EmbeddedImage::from_data_url(PIXEL).unwrap();
    let image_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(image_file.path(), image.bytes()).unwrap();
    let questions = json!({"visible":{"type":"noul","instructions":"Visible?"}});
    for (template_images, row_images, flag_images) in [
        (false, true, false),
        (true, false, false),
        (false, false, true),
    ] {
        let mut request = json!({"questions":questions});
        if template_images {
            request["images"] = json!([PIXEL]);
        }
        let request = request_file(&request);
        let mut row = json!({"state":""});
        if primitive == "eval" {
            row["schema"] = json!("jev.eval.row/v1");
            row["id"] = json!("receipt");
            row["labels"] = json!({"visible":true});
        }
        if row_images {
            row["images"] = json!([PIXEL]);
        }
        let input = request_file(&row);
        let mut command = jev();
        command.args([
            "--provider",
            "cloudflare",
            "--cloudflare-account-id",
            "0123456789abcdef0123456789abcdef",
            "--dry-run",
            "-o",
            "json",
        ]);
        if flag_images {
            command.arg("--image").arg(image_file.path());
        }
        command.args([primitive, "--request"]).arg(request.path());
        if primitive == "map" {
            command
                .arg("--input")
                .arg(input.path())
                .args(["--state-field", "state"]);
            if row_images {
                command.args(["--images-field", "images"]);
            }
        } else {
            command.arg("--dataset").arg(input.path());
        }
        let output = command.assert().success();
        let preview = json_stdout(output.get_output());
        assert_eq!(preview["sample"][0]["body"]["state"], "");
        assert_eq!(
            preview["sample"][0]["body"]["images"][0]["content_type"],
            "image/png"
        );
    }
}

#[test]
fn blank_state_stays_invalid_without_images_and_for_ollama() {
    let image = jev_core::EmbeddedImage::from_data_url(PIXEL).unwrap();
    let image_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(image_file.path(), image.bytes()).unwrap();
    for (provider, with_images) in [("cloudflare", false), ("ollama", true)] {
        for flag in ["--state", "--state-json"] {
            let mut command = jev();
            command.args(["--provider", provider, "--dry-run"]);
            if provider == "cloudflare" {
                command.args([
                    "--cloudflare-account-id",
                    "0123456789abcdef0123456789abcdef",
                ]);
            }
            if with_images {
                command.arg("--image").arg(image_file.path());
            }
            command.args([
                "noul",
                "Visible?",
                flag,
                if flag == "--state" { "" } else { "\"\"" },
            ]);
            command.assert().code(2);
        }
    }
}

#[test]
fn cloudflare_images_preserve_state_validation_and_implicit_stdin() {
    let image = jev_core::EmbeddedImage::from_data_url(PIXEL).unwrap();
    let image_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(image_file.path(), image.bytes()).unwrap();
    let command = || {
        let mut command = jev();
        command
            .args([
                "--provider",
                "cloudflare",
                "--cloudflare-account-id",
                "0123456789abcdef0123456789abcdef",
                "--dry-run",
                "-o",
                "json",
                "--image",
            ])
            .arg(image_file.path())
            .args(["noul", "Visible?"]);
        command
    };
    let output = command().write_stdin("piped text\n").assert().success();
    assert_eq!(
        json_stdout(output.get_output())["body"]["state"],
        "piped text"
    );
    command().write_stdin("").assert().code(2);
    for invalid_json in ["", "null", "false", "42", "{", "\""] {
        command()
            .args(["--state-json", invalid_json])
            .assert()
            .code(2);
    }
    command()
        .args(["--max-input-bytes", "1", "--state", "too long"])
        .assert()
        .code(2);
    for hostile in [&[0xff][..], &[0][..]] {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), hostile).unwrap();
        command()
            .arg("--state-file")
            .arg(file.path())
            .assert()
            .code(2);
    }
    let bad_image = tempfile::NamedTempFile::new().unwrap();
    let output = jev()
        .args([
            "--provider",
            "cloudflare",
            "--cloudflare-account-id",
            "0123456789abcdef0123456789abcdef",
            "--dry-run",
            "--image",
        ])
        .arg(bad_image.path())
        .args(["noul", "Visible?", "--state", ""])
        .assert()
        .code(2);
    assert!(String::from_utf8_lossy(&output.get_output().stderr).contains("image"));
}

#[test]
fn mcp_refuses_file_images_before_starting_the_protocol() {
    jev()
        .args([
            "--provider",
            "ollama",
            "--image",
            "/does/not/exist",
            "mcp",
            "serve",
        ])
        .assert()
        .code(2);
}

#[cfg(unix)]
#[test]
fn file_images_refuse_a_symlinked_parent_directory() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let real = directory.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let image = jev_core::EmbeddedImage::from_data_url(PIXEL).unwrap();
    std::fs::write(real.join("pixel.png"), image.bytes()).unwrap();
    let link = directory.path().join("link");
    symlink(&real, &link).unwrap();
    jev()
        .args(["--provider", "ollama", "--dry-run", "--image"])
        .arg(link.join("pixel.png"))
        .args(["noul", "Visible?", "--state", "receipt"])
        .assert()
        .code(2);
}

#[cfg(unix)]
#[test]
fn file_images_refuse_a_symlinked_final_component() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("pixel.png");
    let image = jev_core::EmbeddedImage::from_data_url(PIXEL).unwrap();
    std::fs::write(&target, image.bytes()).unwrap();
    let link = directory.path().join("link.png");
    symlink(&target, &link).unwrap();
    jev()
        .args(["--provider", "ollama", "--dry-run", "--image"])
        .arg(link)
        .args(["noul", "Visible?", "--state", "receipt"])
        .assert()
        .code(2);
}

#[test]
fn keep_alive_integer_flags_are_sent_as_seconds() {
    for value in ["90", "-1", "0"] {
        let output = jev()
            .args([
                "--provider",
                "ollama",
                "--keep-alive",
                value,
                "--dry-run",
                "-o",
                "json",
                "noul",
                "Visible?",
                "--state",
                "receipt",
            ])
            .assert()
            .success();
        assert_eq!(
            json_stdout(output.get_output())["body"]["keep_alive"],
            json!(value.parse::<i64>().unwrap())
        );
    }
}

#[test]
fn named_frames_form_one_explicit_video_in_the_bridge_preview() {
    let image = jev_core::EmbeddedImage::from_data_url(PIXEL).unwrap();
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), image.bytes()).unwrap();
    let output = jev()
        .args([
            "--provider",
            "huggingface",
            "--max-length",
            "4096",
            "--video-frame",
        ])
        .arg(file.path())
        .arg("--video-frame")
        .arg(file.path())
        .args([
            "--dry-run",
            "-o",
            "json",
            "noul",
            "Motion?",
            "--state",
            "clip",
        ])
        .assert()
        .success();
    let document = json_stdout(output.get_output());
    assert_eq!(
        document["body"]["videos"][0]["frames"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(document["body"]["max_length"], 4096);
    assert_eq!(document["credential"]["source"], "anonymous");
}

#[test]
fn map_uses_explicit_embedded_row_media_without_reading_file_paths() {
    let request =
        request_file(&json!({"questions":{"visible":{"type":"noul","instructions":"Visible?"}}}));
    let input = request_file(&json!({"state":"receipt", "images":[PIXEL]}));
    let output = jev()
        .args([
            "--provider",
            "ollama",
            "--dry-run",
            "-o",
            "json",
            "map",
            "--request",
        ])
        .arg(request.path())
        .arg("--input")
        .arg(input.path())
        .args(["--state-field", "state", "--images-field", "images"])
        .assert()
        .success();
    assert_eq!(
        json_stdout(output.get_output())["sample"][0]["body"]["images"][0],
        PIXEL.split_once(',').unwrap().1
    );
    std::fs::write(
        input.path(),
        json!({"state":"receipt","images":["/tmp/secret.png"]}).to_string(),
    )
    .unwrap();
    jev()
        .args(["--provider", "ollama", "map", "--request"])
        .arg(request.path())
        .arg("--input")
        .arg(input.path())
        .args(["--state-field", "state", "--images-field", "images"])
        .assert()
        .code(2);
}

#[test]
fn request_file_media_on_typesafe_is_refused_before_credentials() {
    let request = request_file(&json!({"state":"receipt","images":[PIXEL],
        "questions":{"visible":{"type":"noul","instructions":"Visible?"}}}));
    jev()
        .args(["ask", "--request"])
        .arg(request.path())
        .assert()
        .code(2);
}

#[test]
fn map_removes_selected_media_fields_from_whole_record_state() {
    let request =
        request_file(&json!({"questions":{"visible":{"type":"noul","instructions":"Visible?"}}}));
    let input = request_file(&json!({"caption":"receipt", "images":[PIXEL]}));
    let output = jev()
        .args([
            "--provider",
            "ollama",
            "--dry-run",
            "-o",
            "json",
            "map",
            "--request",
        ])
        .arg(request.path())
        .arg("--input")
        .arg(input.path())
        .args(["--images-field", "images"])
        .assert()
        .success();
    assert_eq!(
        json_stdout(output.get_output())["sample"][0]["body"]["state"],
        json!({"caption":"receipt"})
    );
}

#[test]
fn map_refuses_using_one_field_for_both_state_and_media() {
    let request =
        request_file(&json!({"questions":{"visible":{"type":"noul","instructions":"Visible?"}}}));
    let input = request_file(&json!({"images":[PIXEL]}));
    jev()
        .args(["--provider", "ollama", "--dry-run", "map", "--request"])
        .arg(request.path())
        .arg("--input")
        .arg(input.path())
        .args(["--images-field", "images", "--state-field", "images"])
        .assert()
        .code(2);
}

#[test]
fn fully_resumed_cloudflare_dry_run_keeps_the_model_route() {
    let request =
        request_file(&json!({"questions":{"visible":{"type":"noul","instructions":"Visible?"}}}));
    let input = request_file(&json!({"caption":"receipt"}));
    let completed = request_file(&json!({"schema":"jev.map.row/v1","index":0,"ok":true}));
    let output = jev()
        .args([
            "--provider",
            "cloudflare",
            "--cloudflare-account-id",
            "0123456789abcdef0123456789abcdef",
            "--model",
            "clef-flash",
            "--dry-run",
            "-o",
            "json",
            "map",
            "--request",
        ])
        .arg(request.path())
        .arg("--input")
        .arg(input.path())
        .arg("--output-file")
        .arg(completed.path())
        .arg("--resume")
        .assert()
        .success();
    let preview = json_stdout(output.get_output());
    assert_eq!(preview["records"], 0);
    assert!(
        preview["url"]
            .as_str()
            .unwrap()
            .ends_with("/ai/run/@cf/cloudflare/clef-flash"),
        "{preview}"
    );
    assert!(
        preview["headers"]
            .as_array()
            .unwrap()
            .contains(&json!("authorization"))
    );
}

#[test]
fn local_request_instructions_default_to_the_normalized_question_id() {
    let request =
        request_file(&json!({"state":"receipt","questions":{"  readable  ":{"type":"noul"}}}));
    for provider in ["ollama", "huggingface"] {
        let output = jev()
            .args([
                "--provider",
                provider,
                "--dry-run",
                "-o",
                "json",
                "ask",
                "--request",
            ])
            .arg(request.path())
            .assert()
            .success();
        assert_eq!(
            json_stdout(output.get_output())["body"]["questions"]["readable"]["instructions"],
            "readable"
        );
    }
    for provider in ["typesafe", "llamacpp"] {
        jev()
            .args(["--provider", provider, "--dry-run", "ask", "--request"])
            .arg(request.path())
            .assert()
            .code(2);
    }
}

#[test]
fn questions_only_file_refusal_names_provider_options_without_claiming_state() {
    let request = request_file(
        &json!({"keep_alive":"5m", "questions":{"readable":{"type":"noul","instructions":"Readable?"}}}),
    );
    let output = jev()
        .args(["--provider", "ollama", "--dry-run", "ask", "--questions"])
        .arg(request.path())
        .args(["--state", "receipt"])
        .assert()
        .code(2);
    let message = String::from_utf8_lossy(&output.get_output().stderr);
    assert!(message.contains("media or provider options"), "{message}");
    assert!(message.contains("--request"), "{message}");
}

#[test]
fn cli_capacity_flag_keeps_precedence_over_the_request_document() {
    let request = request_file(&json!({"state":"receipt","options":{"rejectIfBusy":false},
        "questions":{"readable":{"type":"noul","instructions":"Readable?"}}}));
    let input = request_file(&json!({"schema":"jev.eval.row/v1", "id":"receipt",
        "state":"receipt", "labels":{"readable":true}}));
    for primitive in ["ask", "map", "eval"] {
        let mut command = jev();
        command
            .args([
                "--provider",
                "cloudflare",
                "--cloudflare-account-id",
                "0123456789abcdef0123456789abcdef",
                "--reject-if-busy",
                "--dry-run",
                "-o",
                "json",
                primitive,
                "--request",
            ])
            .arg(request.path());
        if primitive == "map" {
            command
                .arg("--input")
                .arg(input.path())
                .args(["--state-field", "state"]);
        } else if primitive == "eval" {
            command.arg("--dataset").arg(input.path());
        }
        let output = command.assert().success();
        let preview = json_stdout(output.get_output());
        let body = if primitive == "ask" {
            &preview["body"]
        } else {
            &preview["sample"][0]["body"]
        };
        assert_eq!(body["options"], json!({"rejectIfBusy":true}));
    }
}

#[test]
fn cli_image_document_collisions_name_both_explicit_sources() {
    let image = jev_core::EmbeddedImage::from_data_url(PIXEL).unwrap();
    let image_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(image_file.path(), image.bytes()).unwrap();
    let request = request_file(&json!({"state":"receipt", "images":[PIXEL],
        "questions":{"readable":{"type":"noul","instructions":"Readable?"}}}));
    let input = request_file(&json!({"schema":"jev.eval.row/v1", "id":"receipt",
        "state":"receipt", "labels":{"readable":true}}));
    for primitive in ["ask", "map", "eval"] {
        let mut command = jev();
        command
            .args([
                "--provider",
                "cloudflare",
                "--cloudflare-account-id",
                "0123456789abcdef0123456789abcdef",
                "--dry-run",
                "--image",
            ])
            .arg(image_file.path())
            .args([primitive, "--request"])
            .arg(request.path());
        if primitive == "map" {
            command.arg("--input").arg(input.path());
        } else if primitive == "eval" {
            command.arg("--dataset").arg(input.path());
        }
        let output = command.assert().code(2);
        let stderr = String::from_utf8_lossy(&output.get_output().stderr);
        assert!(stderr.contains("--image"), "{stderr}");
        assert!(stderr.contains("request document"), "{stderr}");
        assert!(output.get_output().stdout.is_empty());
    }
}

#[test]
fn local_cli_preserves_independent_state_budget_and_source_video_fps() {
    let image = jev_core::EmbeddedImage::from_data_url(PIXEL).unwrap();
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), image.bytes()).unwrap();
    let output = jev()
        .args([
            "--provider",
            "huggingface",
            "--max-state-tokens",
            "128",
            "--video-fps",
            "30",
            "--video-frame",
        ])
        .arg(file.path())
        .arg("--video-frame")
        .arg(file.path())
        .args([
            "--dry-run",
            "-o",
            "json",
            "noul",
            "Motion?",
            "--state",
            "clip",
        ])
        .assert()
        .success();
    let document = json_stdout(output.get_output());
    assert_eq!(document["body"]["max_state_tokens"], 128);
    assert_eq!(document["body"]["videos"][0]["metadata"]["fps"], 30.0);
}

#[test]
fn local_numeric_state_budget_cannot_accept_credential_text_and_controls_reject_other_providers() {
    jev()
        .args([
            "--provider",
            "huggingface",
            "--max-state-tokens",
            "credential-canary-text",
            "noul",
            "Motion?",
            "--state",
            "clip",
        ])
        .assert()
        .code(2);
    for provider in ["typesafe", "cloudflare", "ollama", "llamacpp"] {
        jev()
            .args([
                "--provider",
                provider,
                "--max-state-tokens",
                "128",
                "--dry-run",
                "noul",
                "Motion?",
                "--state",
                "clip",
            ])
            .assert()
            .code(2);
    }
    for args in [
        vec!["--max-state-tokens", "65537"],
        vec!["--video-fps", "0"],
        vec!["--video-fps", "NaN"],
        vec!["--video-fps", "30"],
    ] {
        jev()
            .args(["--provider", "huggingface"])
            .args(args)
            .args(["--dry-run", "noul", "Motion?", "--state", "clip"])
            .assert()
            .code(2);
    }
}

#[test]
fn native_documents_preserve_state_budget_and_video_timing_across_ask_map_and_eval() {
    let request = request_file(&json!({"state":"clip", "max_state_tokens":128,
        "videos":[{"frames":[PIXEL,PIXEL],"metadata":{"fps":30,"total_num_frames":90,"frames_indices":[0,60],"duration":3}}],
        "questions":{"visible":{"type":"noul","instructions":"Visible?"}}}));
    let input = request_file(
        &json!({"schema":"jev.eval.row/v1","id":"clip", "state":"clip", "labels":{"visible":true}}),
    );
    for primitive in ["ask", "map", "eval"] {
        for limit in [None, Some("256")] {
            let mut command = jev();
            command.args(["--provider", "huggingface", "--dry-run", "-o", "json"]);
            if let Some(limit) = limit {
                command.args(["--max-state-tokens", limit]);
            }
            command.args([primitive, "--request"]).arg(request.path());
            if primitive == "map" {
                command
                    .arg("--input")
                    .arg(input.path())
                    .args(["--state-field", "state"]);
            }
            if primitive == "eval" {
                command.arg("--dataset").arg(input.path());
            }
            let output = command.assert().success();
            let preview = json_stdout(output.get_output());
            let body = if primitive == "ask" {
                &preview["body"]
            } else {
                &preview["sample"][0]["body"]
            };
            assert_eq!(
                body["max_state_tokens"],
                if limit.is_some() { 256 } else { 128 }
            );
            assert_eq!(
                body["videos"][0]["metadata"]["frames_indices"],
                json!([0, 60])
            );
            assert_eq!(body["videos"][0]["metadata"]["fps"], 30);
        }
    }
}
