//! Provider selection through the real process, including offline request previews.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "assertion failures belong in test binaries"
)]

mod support;

use predicates::prelude::*;
use serde_json::json;
use support::{CANARY_KEY, MockApi, Reply, assert_no_canary, jev, json_stdout};

const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn cloudflare_preview_routes_both_models_without_reading_a_credential() {
    for model in ["clef", "clef-flash"] {
        let assert = jev()
            .env("JEV_API_KEY", CANARY_KEY)
            .env("JEV_CUSTOM_API_KEY_FILE", "/nonexistent/must-not-read")
            .args([
                "--provider",
                "cloudflare",
                "--cloudflare-account-id",
                ACCOUNT,
                "--model",
                model,
                "noul",
                "Visible?",
                "--state",
                "context",
                "--dry-run",
                "-o",
                "json",
            ])
            .assert()
            .success();
        let output = assert.get_output();
        assert_no_canary(output);
        let doc = json_stdout(output);
        assert_eq!(
            doc["url"],
            json!(format!(
                "https://api.cloudflare.com/client/v4/accounts/{ACCOUNT}/ai/run/@cf/cloudflare/{model}"
            ))
        );
        assert_eq!(doc["body"]["model"], model);
        assert_eq!(doc["sent"], false);
    }
}

#[test]
fn local_provider_defaults_select_a_local_route_and_no_authentication() {
    for (provider, url) in [
        ("ollama", "http://127.0.0.1:11434/v1/systemone"),
        ("llamacpp", "http://127.0.0.1:8080/v1/systemone"),
    ] {
        let assert = jev()
            .env("JEV_CUSTOM_API_KEY", CANARY_KEY)
            .args([
                "--provider",
                provider,
                "noul",
                "Urgent?",
                "--state",
                "x",
                "--dry-run",
                "-o",
                "json",
            ])
            .assert()
            .success();
        assert_no_canary(assert.get_output());
        let doc = json_stdout(assert.get_output());
        assert_eq!(doc["url"], url);
        assert_eq!(doc["body"]["model"], "clef");
        assert_eq!(doc["credential"]["source"], "anonymous");
        assert!(
            !doc["headers"]
                .as_array()
                .unwrap()
                .contains(&json!("authorization"))
        );
    }
}

#[test]
fn provider_specific_flags_are_refused_on_the_default_provider() {
    for flags in [
        vec!["--cloudflare-account-id", ACCOUNT],
        vec!["--reject-if-busy"],
        vec!["--keep-alive", "5m"],
    ] {
        jev()
            .args(flags)
            .args(["noul", "?", "--state", "x", "--dry-run"])
            .assert()
            .code(2)
            .stderr(predicate::str::contains("requires"));
    }
}

#[test]
fn cloudflare_account_is_required_and_untrusted_identifiers_are_refused() {
    jev()
        .args([
            "--provider",
            "cloudflare",
            "noul",
            "?",
            "--state",
            "x",
            "--dry-run",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("account"));
    jev()
        .args([
            "--provider",
            "cloudflare",
            "--cloudflare-account-id",
            "../account",
            "noul",
            "?",
            "--state",
            "x",
            "--dry-run",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("32"));
}

#[test]
fn cloudflare_account_environment_is_scoped_to_explicit_provider_selection() {
    let assert = jev()
        .env("CLOUDFLARE_ACCOUNT_ID", ACCOUNT)
        .args([
            "--provider",
            "cloudflare",
            "noul",
            "?",
            "--state",
            "x",
            "--dry-run",
            "-o",
            "json",
        ])
        .assert()
        .success();
    assert!(
        json_stdout(assert.get_output())["url"]
            .as_str()
            .unwrap()
            .contains(ACCOUNT)
    );
    let unchanged = jev()
        .env("CLOUDFLARE_ACCOUNT_ID", ACCOUNT)
        .args(["noul", "?", "--state", "x", "--dry-run", "-o", "json"])
        .assert()
        .success();
    assert_eq!(
        json_stdout(unchanged.get_output())["url"],
        "https://api.typesafe.ai/v1/systemone"
    );
}

#[test]
fn local_inference_does_not_read_key_files_or_send_authorization() {
    let reply = json!({"model":"clef-flash","answers":{"answer":{"type":"noul","noul":0.75}},
        "usage":{"input_tokens":12,"output_tokens":0}});
    let api = MockApi::start(vec![Reply::ok(reply.to_string())]);
    let assert = jev()
        .env("JEV_API_KEY", CANARY_KEY)
        .env("JEV_CUSTOM_API_KEY_FILE", "/nonexistent/must-not-read")
        .env("JEV_API_KEY_FILE", "/nonexistent/must-not-read")
        .args([
            "--provider",
            "ollama",
            "--endpoint",
            &api.endpoint(),
            "--model",
            "clef-flash",
            "noul",
            "Visible?",
            "--state",
            "receipt",
            "--output",
            "json",
        ])
        .assert()
        .success();
    assert_no_canary(assert.get_output());
    assert_eq!(
        json_stdout(assert.get_output())["answers"]["answer"]["noul"],
        0.75
    );
    let doc = json_stdout(assert.get_output());
    assert_eq!(doc["provider"], "ollama");
    assert_eq!(doc["cloudflare_account_id"], json!(null));
    let received = api.requests();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].path, "/v1/systemone");
    assert!(!received[0].headers.contains_key("authorization"));
}

#[test]
fn cloudflare_credentials_never_fall_back_to_typesafe_sources() {
    let api = MockApi::start(vec![Reply::ok("{}")]);
    let assert = jev()
        .env("JEV_API_KEY", CANARY_KEY)
        .env("TYPESAFE_API_KEY", CANARY_KEY)
        .args([
            "--provider",
            "cloudflare",
            "--cloudflare-account-id",
            ACCOUNT,
            "--endpoint",
            &api.endpoint(),
            "noul",
            "?",
            "--state",
            "x",
        ])
        .assert()
        .code(3);
    assert_no_canary(assert.get_output());
    assert_eq!(api.hits(), 0);
}

#[test]
fn provider_settings_select_defaults_and_explicit_flags_override_them() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "provider = \"ollama\"\n").unwrap();
    let output = jev()
        .env("JEV_CONFIG_DIR", dir.path())
        .args(["noul", "?", "--state", "x", "--dry-run", "-o", "json"])
        .assert()
        .success();
    assert_eq!(
        json_stdout(output.get_output())["url"],
        "http://127.0.0.1:11434/v1/systemone"
    );
    let overridden = jev()
        .env("JEV_CONFIG_DIR", dir.path())
        .args([
            "--provider",
            "typesafe",
            "noul",
            "?",
            "--state",
            "x",
            "--dry-run",
            "-o",
            "json",
        ])
        .assert()
        .success();
    assert_eq!(
        json_stdout(overridden.get_output())["body"]["model"],
        "jev-latest"
    );
}

#[test]
fn huggingface_preview_carries_local_context_controls_without_authentication() {
    let output = jev()
        .env("JEV_API_KEY_FILE", "/nonexistent/must-not-read")
        .args([
            "--provider",
            "huggingface",
            "--max-length",
            "8192",
            "--media-kwargs",
            "{}",
            "--dry-run",
            "-o",
            "json",
            "noul",
            "Visible?",
            "--state",
            "frames",
        ])
        .assert()
        .success();
    let doc = json_stdout(output.get_output());
    assert_eq!(doc["url"], "http://127.0.0.1:8787/v1/systemone");
    assert_eq!(doc["body"]["max_length"], 8192);
    assert_eq!(doc["body"]["media_kwargs"], json!({}));
    assert_eq!(doc["credential"]["source"], "anonymous");
}

#[test]
fn diagnostics_identify_protocol_and_cloudflare_account_without_secrets() {
    for provider in [
        "typesafe",
        "cloudflare",
        "ollama",
        "huggingface",
        "llamacpp",
    ] {
        let mut command = jev();
        command
            .env("JEV_API_KEY", CANARY_KEY)
            .args(["--provider", provider]);
        if provider == "cloudflare" {
            command.args(["--cloudflare-account-id", ACCOUNT]);
        }
        let output = command.args(["doctor", "-o", "json"]).assert().success();
        assert_no_canary(output.get_output());
        let doc = json_stdout(output.get_output());
        assert_eq!(doc["endpoint"]["provider"], provider);
        assert_eq!(
            doc["endpoint"]["cloudflare_account_id"],
            if provider == "cloudflare" {
                json!(ACCOUNT)
            } else {
                json!(null)
            }
        );
    }
}

#[test]
fn cloudflare_configuration_can_be_edited_before_an_account_is_configured() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["config", "set", "provider", "cloudflare"],
        vec!["config", "get", "provider"],
        vec!["config", "set", "cloudflare_account_id", ACCOUNT],
        vec!["config", "unset", "cloudflare_account_id"],
        vec!["config", "unset", "provider"],
    ] {
        jev()
            .env("JEV_CONFIG_DIR", dir.path())
            .args(args)
            .assert()
            .success();
    }
}

#[test]
fn provider_switch_does_not_reuse_another_providers_configured_endpoint() {
    let dir = tempfile::tempdir().unwrap();
    for configured in [None, Some("typesafe"), Some("ollama")] {
        let prefix = configured.map_or(String::new(), |provider| {
            format!("provider = \"{provider}\"\n")
        });
        std::fs::write(
            dir.path().join("config.toml"),
            format!("{prefix}endpoint = \"https://proxy.example.com\"\n"),
        )
        .unwrap();
        for (provider, expected) in [
            ("ollama", "http://127.0.0.1:11434/v1/systemone"),
            ("huggingface", "http://127.0.0.1:8787/v1/systemone"),
            ("llamacpp", "http://127.0.0.1:8080/v1/systemone"),
            ("cloudflare", "https://api.cloudflare.com"),
        ] {
            if configured == Some(provider) {
                continue;
            }
            let mut cmd = jev();
            cmd.env("JEV_CONFIG_DIR", dir.path())
                .args(["--provider", provider]);
            if provider == "cloudflare" {
                cmd.args(["--cloudflare-account-id", ACCOUNT]);
            }
            let output = cmd
                .args(["noul", "?", "--state", "x", "--dry-run", "-o", "json"])
                .assert()
                .success();
            let doc = json_stdout(output.get_output());
            assert!(doc["url"].as_str().unwrap().starts_with(expected), "{doc}");
            if provider != "cloudflare" {
                assert_eq!(doc["credential"]["source"], "anonymous");
            }
        }
    }
}

#[test]
fn explicit_same_provider_retains_its_configured_endpoint() {
    let dir = tempfile::tempdir().unwrap();
    for provider in [
        "typesafe",
        "ollama",
        "huggingface",
        "llamacpp",
        "cloudflare",
    ] {
        std::fs::write(
            dir.path().join("config.toml"),
            format!("provider = \"{provider}\"\nendpoint = \"https://proxy.example.com\"\n"),
        )
        .unwrap();
        let mut cmd = jev();
        cmd.env("JEV_CONFIG_DIR", dir.path())
            .args(["--provider", provider]);
        if provider == "cloudflare" {
            cmd.args(["--cloudflare-account-id", ACCOUNT]);
        }
        let output = cmd
            .args(["noul", "?", "--state", "x", "--dry-run", "-o", "json"])
            .assert()
            .success();
        assert!(
            json_stdout(output.get_output())["url"]
                .as_str()
                .unwrap()
                .starts_with("https://proxy.example.com/")
        );
    }
}

#[test]
fn inference_only_flags_are_rejected_by_administrative_commands() {
    for command in [
        vec!["models"],
        vec!["doctor"],
        vec!["auth", "status"],
        vec!["config", "get", "provider"],
    ] {
        for flags in [
            vec![
                "--provider",
                "cloudflare",
                "--cloudflare-account-id",
                ACCOUNT,
                "--image",
                "missing.png",
            ],
            vec![
                "--provider",
                "cloudflare",
                "--cloudflare-account-id",
                ACCOUNT,
                "--reject-if-busy",
            ],
            vec!["--provider", "ollama", "--keep-alive", "5m"],
            vec!["--provider", "huggingface", "--max-length", "8192"],
        ] {
            jev()
                .args(flags)
                .args(&command)
                .assert()
                .code(2)
                .stderr(predicate::str::contains("inference"));
        }
    }
}

#[test]
fn incomplete_saved_cloudflare_configuration_can_be_diagnosed_without_authentication() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        "provider = \"cloudflare\"\n",
    )
    .unwrap();
    for command in [
        vec!["doctor"],
        vec!["doctor", "--live"],
        vec!["auth", "status"],
    ] {
        let mut cmd = jev();
        cmd.env("JEV_CONFIG_DIR", dir.path())
            .env("JEV_CUSTOM_API_KEY_FILE", "/nonexistent/must-not-read")
            .env("JEV_API_KEY_FILE", "/nonexistent/must-not-read")
            .args(&command)
            .args(["-o", "json"]);
        let output = cmd
            .assert()
            .code(if command[0] == "doctor" { 0 } else { 3 });
        let doc = json_stdout(output.get_output());
        assert!(
            doc["configuration_error"]
                .as_str()
                .unwrap()
                .contains("account")
        );
        if command[0] == "doctor" {
            assert_eq!(doc["live"]["checked"], false);
        } else {
            assert_eq!(doc["effective_source"], json!(null));
        }
    }
    jev()
        .env("JEV_CONFIG_DIR", dir.path())
        .args(["doctor", "--timeout", "0"])
        .assert()
        .code(2);
}

#[test]
fn changing_provider_reports_ignored_saved_endpoint_without_disclosing_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        "endpoint = \"https://saved-provider-only.example\"\n",
    )
    .unwrap();
    let output = jev()
        .env("JEV_CONFIG_DIR", dir.path())
        .args([
            "--provider",
            "ollama",
            "noul",
            "?",
            "--state",
            "x",
            "--dry-run",
            "-o",
            "json",
        ])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr);
    assert!(stderr.contains("ignoring the saved endpoint"), "{stderr}");
    assert!(!stderr.contains("saved-provider-only.example"), "{stderr}");
    assert_eq!(
        json_stdout(output.get_output())["url"],
        "http://127.0.0.1:11434/v1/systemone"
    );
    for flags in [
        vec!["--quiet"],
        vec!["--endpoint", "http://127.0.0.1:11500"],
    ] {
        let output = jev()
            .env("JEV_CONFIG_DIR", dir.path())
            .args(["--provider", "ollama"])
            .args(flags)
            .args(["noul", "?", "--state", "x", "--dry-run", "-o", "json"])
            .assert()
            .success();
        assert!(
            !String::from_utf8_lossy(&output.get_output().stderr)
                .contains("ignoring the saved endpoint")
        );
    }
    std::fs::write(
        dir.path().join("config.toml"),
        "provider = \"ollama\"\nendpoint = \"https://saved-provider-only.example\"\n",
    )
    .unwrap();
    let output = jev()
        .env("JEV_CONFIG_DIR", dir.path())
        .args([
            "--provider",
            "typesafe",
            "noul",
            "?",
            "--state",
            "x",
            "--dry-run",
            "-o",
            "json",
        ])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&output.get_output().stderr);
    assert!(stderr.contains("ignoring the saved endpoint"), "{stderr}");
    assert!(!stderr.contains("saved-provider-only.example"), "{stderr}");
    assert_eq!(
        json_stdout(output.get_output())["url"],
        "https://api.typesafe.ai/v1/systemone"
    );
}

#[test]
fn same_provider_configuration_does_not_report_an_ignored_endpoint() {
    let dir = tempfile::tempdir().unwrap();
    for provider in ["typesafe", "ollama"] {
        std::fs::write(
            dir.path().join("config.toml"),
            format!("provider = \"{provider}\"\nendpoint = \"http://127.0.0.1:11500\"\n"),
        )
        .unwrap();
        let output = jev()
            .env("JEV_CONFIG_DIR", dir.path())
            .args([
                "--provider",
                provider,
                "noul",
                "?",
                "--state",
                "x",
                "--dry-run",
                "-o",
                "json",
            ])
            .assert()
            .success();
        assert!(
            !String::from_utf8_lossy(&output.get_output().stderr)
                .contains("ignoring the saved endpoint")
        );
        assert_eq!(
            json_stdout(output.get_output())["url"],
            "http://127.0.0.1:11500/v1/systemone"
        );
    }
}

#[test]
fn llama_cpp_provider_alias_works_in_configuration_and_retains_its_endpoint() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        "provider = \"llama-cpp\"\nendpoint = \"http://127.0.0.1:11500\"\n",
    )
    .unwrap();
    let output = jev()
        .env("JEV_CONFIG_DIR", dir.path())
        .args([
            "--provider",
            "llamacpp",
            "noul",
            "?",
            "--state",
            "x",
            "--dry-run",
            "-o",
            "json",
        ])
        .assert()
        .success();
    assert_eq!(
        json_stdout(output.get_output())["url"],
        "http://127.0.0.1:11500/v1/systemone"
    );
    assert!(
        !String::from_utf8_lossy(&output.get_output().stderr)
            .contains("ignoring the saved endpoint")
    );
}
