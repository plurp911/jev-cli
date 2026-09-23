//! `jev mcp serve`, driven as a real child process.
//!
//! Two kinds of client drive it:
//!
//! * the official Rust SDK's client (`rmcp`), for the protocol flows a host performs --
//!   negotiation at each supported revision, `tools/list`, `tools/call`;
//! * a raw line client over the child's pipes, for what an SDK would hide: bytes on
//!   stdout that are not protocol messages, cancellation, and hostile framing.
//!
//! The backend is `support::MockApi`, a real HTTP server on loopback, reached through
//! `--endpoint` exactly as a user would configure one. Nothing here touches the
//! internet or needs a TypeSafe key.
#![allow(
    unreachable_pub,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout,
    reason = "a panicking assertion is the correct failure mode inside a test binary"
)]

mod support;

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, Implementation,
    ProtocolVersion,
};
use rmcp::service::{ClientLifecycleMode, ClientServiceExt as _, RunningService};
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt as _};
use serde_json::{Value, json};
use support::{CANARY_KEY, MockApi, Reply, jev_spawnable, noul_body, wait_until};

/// Generous, because CI runners are slow; a passing test never waits this long.
const PATIENCE: Duration = Duration::from_secs(20);

const TOOLS: [&str; 5] = ["noul", "choice", "score", "ask", "map"];

// --- Harness ---------------------------------------------------------------------------

/// The hermetic command, pointed at `endpoint`, running `mcp serve` from a directory that
/// is not this repository: the server must not depend on its working directory.
fn serve_command(endpoint: &str, extra: &[&str]) -> (std::process::Command, tempfile::TempDir) {
    let cwd = tempfile::tempdir().unwrap();
    let mut command = jev_spawnable();
    command.current_dir(cwd.path());
    command.args(["--endpoint", endpoint]);
    command.args(extra);
    command.args(["mcp", "serve"]);
    (command, cwd)
}

type Client = RunningService<RoleClient, ClientConfig>;

fn client_config(version: ProtocolVersion) -> ClientConfig {
    ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("jev-tests", "0"),
    )
    .with_protocol_version(version)
}

/// Connects the official client over the legacy `initialize` handshake.
async fn connect(command: std::process::Command, version: ProtocolVersion) -> Client {
    let (transport, _stderr) = TokioChildProcess::builder(tokio::process::Command::from(command))
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    client_config(version).serve(transport).await.unwrap()
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

async fn call(client: &Client, tool: &str, arguments: Value) -> CallToolResult {
    let Value::Object(arguments) = arguments else {
        panic!("arguments must be an object")
    };
    client
        .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments))
        .await
        .unwrap()
}

/// The structured result, after checking the text block carries the same document.
fn structured(result: &CallToolResult) -> Value {
    assert_eq!(result.is_error, Some(false), "not a success: {result:?}");
    let document = result
        .structured_content
        .clone()
        .expect("structuredContent");
    let text = result
        .content
        .first()
        .and_then(|block| block.as_text())
        .map(|text| text.text.clone())
        .expect("a text block");
    let from_text: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(from_text, document, "the two encodings diverge");
    document
}

/// The `error` object of a tool-level failure.
fn tool_error(result: &CallToolResult) -> Value {
    assert_eq!(result.is_error, Some(true), "not a tool error: {result:?}");
    assert!(
        result.structured_content.is_none(),
        "an error must not look like a result"
    );
    let text = result
        .content
        .first()
        .and_then(|block| block.as_text())
        .map(|text| text.text.clone())
        .expect("a text block");
    let document: Value = serde_json::from_str(&text).unwrap();
    document["error"].clone()
}

fn choice_body() -> String {
    json!({
        "model": "jev-1.13.0",
        "answers": {"team": {"type": "choice", "choice": "billing", "confidence": 0.81,
            "probabilities": {"billing": 0.88, "technical": 0.12, "sales": 0.0}}},
        "usage": {"input_tokens": 318, "output_tokens": 34}
    })
    .to_string()
}

fn score_body() -> String {
    json!({
        "model": "jev-1.13.0",
        "answers": {"risk": {"type": "score", "score": 1.05, "confidence": 0.92,
            "legend": {"0": "Calm", "1": "Frustrated", "2": "Very angry"},
            "probabilities": {"0": 0.0, "1": 0.95, "2": 0.05}}},
        "usage": {"input_tokens": 304, "output_tokens": 18}
    })
    .to_string()
}

fn mixed_body() -> String {
    json!({
        "model": "jev-1.13.0",
        "answers": {
            "api_change": {"type": "noul", "noul": 0.9},
            "team": {"type": "choice", "choice": "billing", "confidence": 0.8,
                     "probabilities": {"billing": 0.8, "tech": 0.2}},
            "risk": {"type": "score", "score": 1.0, "confidence": 0.7,
                     "legend": {"0": "low", "1": "high"},
                     "probabilities": {"0": 0.0, "1": 1.0}}
        },
        "usage": {"input_tokens": 50, "output_tokens": 9}
    })
    .to_string()
}

/// A line client over the child's pipes. Reads stdout on a thread so a test can wait
/// with a deadline instead of blocking forever on a server that went quiet.
struct Raw {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: mpsc::Receiver<String>,
    stderr: std::thread::JoinHandle<String>,
    _cwd: tempfile::TempDir,
}

impl Raw {
    fn spawn(endpoint: &str, extra: &[&str]) -> Self {
        let (mut command, cwd) = serve_command(endpoint, extra);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let (send, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        let stderr = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });
        let stdin = child.stdin.take();
        Self {
            child,
            stdin,
            lines,
            stderr,
            _cwd: cwd,
        }
    }

    fn send(&mut self, message: &Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    fn send_bytes(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let stdin = self.stdin.as_mut().unwrap();
        stdin.write_all(bytes)?;
        stdin.flush()
    }

    /// The next stdout line, which must be one JSON-RPC 2.0 message.
    fn recv(&self) -> Value {
        let line = self
            .lines
            .recv_timeout(PATIENCE)
            .expect("a reply on stdout");
        let message: Value = serde_json::from_str(&line)
            .unwrap_or_else(|error| panic!("stdout carried a non-JSON line ({error}): {line:?}"));
        assert_eq!(message["jsonrpc"], "2.0", "not JSON-RPC: {line}");
        message
    }

    /// The reply to request `id`, skipping any notification in between.
    fn reply(&self, id: u64) -> Value {
        loop {
            let message = self.recv();
            if message.get("id") == Some(&json!(id)) {
                return message;
            }
            assert!(
                message.get("id").is_none(),
                "an unexpected reply arrived: {message}"
            );
        }
    }

    fn initialize(&mut self, version: &str) -> Value {
        self.send(
            &json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": version, "capabilities": {},
            "clientInfo": {"name": "raw", "version": "0"}}}),
        );
        let reply = self.reply(0);
        self.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        reply
    }

    fn call(&mut self, id: u64, tool: &str, arguments: &Value) {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": tool, "arguments": arguments}}));
    }

    /// Closes stdin and waits for the process to exit on its own.
    fn close(mut self) -> (std::process::ExitStatus, String, Vec<String>) {
        drop(self.stdin.take());
        let exited = wait_until(PATIENCE, || matches!(self.child.try_wait(), Ok(Some(_))));
        if !exited {
            let _ = self.child.kill();
            panic!("the server did not exit after stdin closed");
        }
        let status = self.child.wait().unwrap();
        let rest: Vec<String> = self.lines.try_iter().collect();
        (status, self.stderr.join().unwrap(), rest)
    }
}

// --- Lifecycle and surface ---------------------------------------------------------------

#[test]
fn the_server_negotiates_every_supported_revision_and_lists_the_same_five_tools() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    runtime().block_on(async {
        for version in [
            ProtocolVersion::V_2024_11_05,
            ProtocolVersion::V_2025_03_26,
            ProtocolVersion::V_2025_06_18,
            ProtocolVersion::V_2025_11_25,
        ] {
            let (command, _cwd) = serve_command(&api.endpoint(), &[]);
            let client = connect(command, version.clone()).await;
            let info = client.peer_info().unwrap();
            assert_eq!(info.protocol_version, version, "did not echo {version}");
            let server = info.server_info.clone().unwrap();
            assert_eq!(server.name, "jev");
            assert_eq!(server.version, env!("CARGO_PKG_VERSION"));
            assert!(info.capabilities.tools.is_some());
            // Tools only: nothing else is advertised.
            assert!(info.capabilities.resources.is_none());
            assert!(info.capabilities.prompts.is_none());
            assert!(
                info.instructions
                    .as_deref()
                    .unwrap()
                    .contains("sent to TypeSafe")
            );

            let names: Vec<String> = client
                .list_all_tools()
                .await
                .unwrap()
                .into_iter()
                .map(|tool| tool.name.to_string())
                .collect();
            assert_eq!(names, TOOLS, "at {version}");
            client.cancel().await.unwrap();
        }
    });
}

#[test]
fn the_stateless_2026_revision_is_served_through_discover() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    runtime().block_on(async {
        let (command, _cwd) = serve_command(&api.endpoint(), &[]);
        let (transport, _stderr) =
            TokioChildProcess::builder(tokio::process::Command::from(command))
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
        let client = client_config(ProtocolVersion::V_2026_07_28)
            .serve_with_lifecycle(
                transport,
                ClientLifecycleMode::Discover {
                    preferred_versions: vec![ProtocolVersion::V_2026_07_28],
                },
            )
            .await
            .unwrap();
        let list = client.list_tools(None).await.unwrap();
        let names: Vec<String> = list
            .tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect();
        assert_eq!(names, TOOLS);
        // Required by 2026-07-28 on a list result; the Inspector rejects a list without
        // them, and the Rust client alone would never have noticed.
        assert_eq!(list.ttl_ms, Some(0));
        assert!(list.cache_scope.is_some());
        let result = call(
            &client,
            "noul",
            json!({"state": "payouts failing for 3 days", "instructions": "Urgent?"}),
        )
        .await;
        assert_eq!(structured(&result)["answers"]["answer"]["noul"], 0.92);
        client.cancel().await.unwrap();
    });
}

#[test]
fn every_tool_is_annotated_read_only_non_idempotent_and_open_world() {
    let api = MockApi::start(vec![]);
    runtime().block_on(async {
        let (command, _cwd) = serve_command(&api.endpoint(), &[]);
        let client = connect(command, ProtocolVersion::V_2025_11_25).await;
        for tool in client.list_all_tools().await.unwrap() {
            let annotations = tool.annotations.clone().expect("annotations");
            assert_eq!(annotations.read_only_hint, Some(true), "{}", tool.name);
            assert_eq!(annotations.destructive_hint, Some(false), "{}", tool.name);
            // A repeat is another billed, probabilistic evaluation.
            assert_eq!(annotations.idempotent_hint, Some(false), "{}", tool.name);
            assert_eq!(annotations.open_world_hint, Some(true), "{}", tool.name);
            // Object-rooted, which 2025-11-25 and earlier require, and which the
            // Inspector's legacy mode silently drops a tool for violating.
            assert_eq!(tool.input_schema.get("type"), Some(&json!("object")));
            let output = tool.output_schema.clone().expect("outputSchema");
            assert_eq!(output.get("type"), Some(&json!("object")));
            let description = tool.description.clone().unwrap_or_default();
            assert!(
                description.contains("sent to the configured TypeSafe endpoint"),
                "{} does not say where the state goes",
                tool.name
            );
        }
        // No tool touches the terminal, the filesystem, or configuration.
        assert_eq!(api.hits(), 0, "listing tools sent a request");
        client.cancel().await.unwrap();
    });
}

#[test]
fn the_tool_list_matches_the_committed_snapshot() {
    // The tool surface is a public integration API. Changing a name, a schema, or a
    // description is a release decision; this makes it a visible one.
    let api = MockApi::start(vec![]);
    let mut raw = Raw::spawn(&api.endpoint(), &[]);
    raw.initialize("2025-11-25");
    raw.send(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}));
    let tools = raw.reply(1)["result"]["tools"].clone();
    let rendered = serde_json::to_string_pretty(&tools).unwrap() + "\n";
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/mcp-tools.json");
    if std::env::var_os("JEV_UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, &rendered).unwrap();
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == rendered,
        "tools/list no longer matches {}.\nIf the change is deliberate, rerun with \
         JEV_UPDATE_SNAPSHOTS=1 and note it in CHANGELOG.md.",
        path.display()
    );
    let (status, _, _) = raw.close();
    assert!(status.success());
}

// --- Calls -----------------------------------------------------------------------------

#[test]
fn each_tool_returns_the_cli_document_with_its_probabilities_model_and_usage() {
    let api = MockApi::start(vec![
        Reply::ok(noul_body()).header("x-typesafe-request-id", "req_noul"),
        Reply::ok(choice_body()),
        Reply::ok(score_body()),
        Reply::ok(mixed_body()),
    ]);
    runtime().block_on(async {
        let (command, _cwd) = serve_command(&api.endpoint(), &[]);
        let client = connect(command, ProtocolVersion::V_2025_06_18).await;

        let secret_state = "ticket body 7f3a: payouts failing";
        let noul = structured(
            &call(
                &client,
                "noul",
                json!({"state": secret_state, "instructions": "Does this convey urgency?",
                       "criteria": {"true": "time-sensitive", "false": "no urgency"}}),
            )
            .await,
        );
        assert_eq!(noul["schema"], "jev.evaluation/v1");
        assert_eq!(noul["model"], "jev-1.13.0");
        assert_eq!(noul["model_requested"], "jev-latest");
        assert_eq!(noul["answers"]["answer"], json!({"type": "noul", "noul": 0.92}));
        assert_eq!(noul["usage"], json!({"input_tokens": 312, "output_tokens": 48}));
        assert_eq!(noul["request_id"], "req_noul");
        assert!(
            !noul.to_string().contains("7f3a"),
            "the state was echoed back into the result"
        );

        let choice = structured(
            &call(
                &client,
                "choice",
                json!({"state": {"subject": "refund"}, "instructions": "Which team?", "id": "team",
                       "options": [{"name": "technical"}, {"name": "billing", "description": "money"},
                                   {"name": "sales"}]}),
            )
            .await,
        );
        let answer = &choice["answers"]["team"];
        assert_eq!(answer["choice"], "billing");
        assert_eq!(answer["confidence"], 0.81);
        assert_eq!(
            answer["probabilities"],
            json!({"billing": 0.88, "technical": 0.12, "sales": 0.0})
        );

        let score = structured(
            &call(
                &client,
                "score",
                json!({"state": "x", "instructions": "How upset?", "id": "risk",
                       "levels": ["Calm", "Frustrated", "Very angry"], "model": "jev-1.13.0"}),
            )
            .await,
        );
        assert_eq!(score["answers"]["risk"]["score"], 1.05);
        assert_eq!(score["answers"]["risk"]["probabilities"]["1"], 0.95);
        assert_eq!(score["model_requested"], "jev-1.13.0");

        let ask = structured(
            &call(
                &client,
                "ask",
                json!({"state": "diff --git a/api.rs", "questions": [
                    {"id": "api_change", "type": "noul", "instructions": "Changes a public API?"},
                    {"id": "team", "type": "choice", "instructions": "Owner?",
                     "options": [{"name": "billing"}, {"name": "tech"}]},
                    {"id": "risk", "type": "score", "instructions": "Risk?", "levels": ["low", "high"]}
                ]}),
            )
            .await,
        );
        assert_eq!(ask["answers"].as_object().unwrap().len(), 3);
        client.cancel().await.unwrap();
    });

    assert_the_wire_carries_what_the_cli_would_send(&api.requests());
}

/// What went on the wire is what the CLI would send: the official body, the options in
/// the order the agent wrote them, the per-call model, and one request per call.
fn assert_the_wire_carries_what_the_cli_would_send(requests: &[support::Seen]) {
    assert_eq!(requests.len(), 4);
    let noul: Value = serde_json::from_str(&requests[0].body).unwrap();
    assert_eq!(
        noul,
        json!({"state": "ticket body 7f3a: payouts failing", "model": "jev-latest", "questions": {
            "answer": {"type": "noul", "instructions": "Does this convey urgency?",
                       "criteria": {"true": "time-sensitive", "false": "no urgency"}}}})
    );
    let order: Vec<&str> = ["\"technical\"", "\"billing\"", "\"sales\""]
        .into_iter()
        .collect();
    let positions: Vec<usize> = order
        .iter()
        .map(|name| requests[1].body.find(name).unwrap())
        .collect();
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "Choice options were reordered on the wire: {}",
        requests[1].body
    );
    assert!(requests[2].body.contains("\"model\":\"jev-1.13.0\""));
    let ask: Value = serde_json::from_str(&requests[3].body).unwrap();
    assert_eq!(ask["questions"].as_object().unwrap().len(), 3);
    for request in requests {
        assert_eq!(request.path, "/v1/systemone");
        assert_eq!(
            request.headers.get("authorization").map(String::as_str),
            Some(format!("Bearer {CANARY_KEY}").as_str())
        );
    }
}

#[test]
fn map_returns_one_cli_row_per_record_in_input_order_and_keeps_failures_as_failures() {
    let api = MockApi::start(vec![
        Reply::ok(noul_body()),
        Reply::status(422, r#"{"detail":"bad"}"#),
        Reply::ok(noul_body()),
    ]);
    runtime().block_on(async {
        let (command, _cwd) = serve_command(&api.endpoint(), &[]);
        let client = connect(command, ProtocolVersion::V_2025_11_25).await;
        let result = call(
            &client,
            "map",
            json!({
                "questions": [{"id": "answer", "type": "noul", "instructions": "Is this an outage?"}],
                "records": [{"id": "a-1", "state": "db down"}, {"state": "typo"},
                            {"id": 7, "state": {"msg": "latency"}}],
                "concurrency": 1
            }),
        )
        .await;
        let document = structured(&result);
        assert_eq!(document["schema"], "jev.mcp.map/v1");
        let rows = document["rows"].as_array().unwrap();
        let indexes: Vec<u64> = rows.iter().map(|row| row["index"].as_u64().unwrap()).collect();
        assert_eq!(indexes, [0, 1, 2]);
        let ids: Vec<&str> = rows.iter().map(|row| row["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["a-1", "1", "7"]);
        for row in rows {
            assert_eq!(row["schema"], "jev.map.row/v1");
            assert!(row.get("state").is_none(), "a row echoed its record");
        }
        assert_eq!(rows[0]["ok"], true);
        assert_eq!(rows[0]["answers"]["answer"]["noul"], 0.92);
        // A failed record is an error row, never an answer of `false` or `0`.
        assert_eq!(rows[1]["ok"], false);
        assert!(rows[1].get("answers").is_none());
        assert_eq!(rows[1]["error"]["kind"], "request");
        assert_eq!(document["summary"]["schema"], "jev.map.summary/v1");
        assert_eq!(document["summary"]["succeeded"], 2);
        assert_eq!(document["summary"]["failed"], 1);
        client.cancel().await.unwrap();
    });
}

#[test]
fn a_failed_map_row_reports_how_many_attempts_were_made() {
    let api = MockApi::start(vec![
        Reply::status(503, "{}").header("retry-after-ms", "1"),
        Reply::status(503, "{}").header("retry-after-ms", "1"),
        Reply::status(422, r#"{"detail":"bad"}"#),
    ]);
    runtime().block_on(async {
        let (command, _cwd) = serve_command(&api.endpoint(), &["--retries", "1"]);
        let client = connect(command, ProtocolVersion::V_2025_11_25).await;
        let result = call(
            &client,
            "map",
            json!({
                "questions": [{"id": "answer", "type": "noul", "instructions": "?"}],
                "records": [{"state": "a"}, {"state": "b"}],
                "concurrency": 1
            }),
        )
        .await;
        let rows = structured(&result)["rows"].as_array().unwrap().clone();
        assert_eq!(rows[0]["error"]["kind"], "unavailable");
        assert_eq!(rows[0]["attempts"], 2, "{}", rows[0]);
        // Not retryable, so one attempt.
        assert_eq!(rows[1]["error"]["kind"], "request");
        assert_eq!(rows[1]["attempts"], 1, "{}", rows[1]);
        client.cancel().await.unwrap();
    });
    assert_eq!(api.hits(), 3);
}

#[test]
fn concurrent_calls_are_all_answered() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    runtime().block_on(async {
        let (command, _cwd) = serve_command(&api.endpoint(), &[]);
        let client = connect(command, ProtocolVersion::V_2025_11_25).await;
        let calls = (0..8).map(|index| {
            call(
                &client,
                "noul",
                json!({"state": format!("record {index}"), "instructions": "?"}),
            )
        });
        let results = futures_join_all(calls).await;
        assert_eq!(results.len(), 8);
        for result in &results {
            assert_eq!(structured(result)["answers"]["answer"]["noul"], 0.92);
        }
        // Then sequential calls on the same connection still work.
        for _ in 0..3 {
            structured(&call(&client, "noul", json!({"state": "x", "instructions": "?"})).await);
        }
        client.cancel().await.unwrap();
    });
    assert_eq!(api.requests().len(), 11);
}

/// `futures::future::join_all` without a `futures` dev-dependency: polls every future
/// on one task, which is what makes the calls concurrent on a current-thread runtime.
async fn futures_join_all<F: Future>(futures: impl Iterator<Item = F>) -> Vec<F::Output> {
    let mut set = Vec::new();
    for future in futures {
        set.push(Box::pin(future));
    }
    let mut outputs: Vec<Option<F::Output>> = set.iter().map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut pending = false;
        for (future, output) in set.iter_mut().zip(outputs.iter_mut()) {
            if output.is_none() {
                match future.as_mut().poll(cx) {
                    std::task::Poll::Ready(value) => *output = Some(value),
                    std::task::Poll::Pending => pending = true,
                }
            }
        }
        if pending {
            std::task::Poll::Pending
        } else {
            std::task::Poll::Ready(())
        }
    })
    .await;
    outputs.into_iter().map(Option::unwrap).collect()
}

// --- Errors ----------------------------------------------------------------------------

#[test]
fn invalid_arguments_are_tool_errors_the_model_can_correct_and_nothing_is_sent() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    runtime().block_on(async {
        let (command, _cwd) = serve_command(&api.endpoint(), &[]);
        let client = connect(command, ProtocolVersion::V_2025_11_25).await;
        let cases = [
            ("noul", json!({"instructions": "?"}), "state"),
            ("noul", json!({"state": "x", "instructions": "?", "extra": 1}), "extra"),
            ("noul", json!({"state": "x", "instructions": "   "}), "must not be empty"),
            ("noul", json!({"state": null, "instructions": "?"}), "null"),
            ("choice", json!({"state": "x", "instructions": "?", "options": [{"name": "only"}]}), "option"),
            ("choice", json!({"state": "x", "instructions": "?",
                             "options": [{"name": "a"}, {"name": "a"}]}), "duplicate"),
            ("score", json!({"state": "x", "instructions": "?", "levels": ["one"]}), "level"),
            ("ask", json!({"state": "x", "questions": []}), "question"),
            ("ask", json!({"state": "x", "questions": [
                {"id": "q", "type": "noul", "instructions": "?"},
                {"id": "q", "type": "noul", "instructions": "?"}]}), "duplicate"),
            ("ask", json!({"state": "x", "questions": [
                {"id": "q", "type": "score", "instructions": "?", "options": [{"name": "a"}, {"name": "b"}]}]}),
             "does not apply"),
            ("ask", json!({"state": "x", "questions": [{"id": "q", "type": "boolean", "instructions": "?"}]}),
             "unknown variant"),
            ("map", json!({"questions": [{"id": "q", "type": "noul", "instructions": "?"}], "records": []}),
             "empty"),
            ("map", json!({"questions": [{"id": "q", "type": "noul", "instructions": "?"}],
                           "records": [{"state": "x"}], "concurrency": 0}), "between 1 and 16"),
            ("map", json!({"questions": [{"id": "q", "type": "noul", "instructions": "?"}],
                           "records": [{"state": "x"}], "concurrency": 17}), "between 1 and 16"),
            ("noul", json!({"state": {"a": {"b": {"c": nest(70)}}}, "instructions": "?"}), "deep"),
        ];
        for (tool, arguments, needle) in cases {
            let error = tool_error(&call(&client, tool, arguments.clone()).await);
            assert_eq!(error["kind"], "usage", "{tool} {arguments}: {error}");
            let message = error["message"].as_str().unwrap().to_lowercase();
            assert!(
                message.contains(needle),
                "{tool} {arguments}: {message:?} does not mention {needle:?}"
            );
        }
        client.cancel().await.unwrap();
    });
    assert_eq!(api.hits(), 0, "an invalid call was sent");
}

fn nest(depth: usize) -> Value {
    (0..depth).fold(json!("leaf"), |inner, _| json!({ "n": inner }))
}

#[test]
fn an_unknown_tool_is_a_protocol_error() {
    let api = MockApi::start(vec![]);
    let mut raw = Raw::spawn(&api.endpoint(), &[]);
    raw.initialize("2025-11-25");
    for tool in ["eval", "auth_login", "config_set", "read_file", "doctor"] {
        raw.call(1, tool, &json!({}));
        let reply = raw.reply(1);
        assert_eq!(reply["error"]["code"], -32602, "{tool}: {reply}");
    }
    let (status, _, _) = raw.close();
    assert!(status.success());
}

#[test]
fn limits_are_refused_before_anything_is_sent_and_point_at_the_cli() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    runtime().block_on(async {
        let (command, _cwd) = serve_command(&api.endpoint(), &["--max-input-bytes", "1000"]);
        let client = connect(command, ProtocolVersion::V_2025_11_25).await;

        let big = "x".repeat(2000);
        let error = tool_error(&call(&client, "noul", json!({"state": big, "instructions": "?"})).await);
        assert_eq!(error["kind"], "usage");
        assert!(error["message"].as_str().unwrap().contains("input limit"));

        let question = json!([{"id": "q", "type": "noul", "instructions": "?"}]);
        let records: Vec<Value> = (0..101).map(|i| json!({"state": format!("r{i}")})).collect();
        let error = tool_error(
            &call(&client, "map", json!({"questions": question, "records": records})).await,
        );
        assert!(error["message"].as_str().unwrap().contains("jev map"), "{error}");

        // Twenty records of 100 bytes each fit one by one and not together.
        let records: Vec<Value> = (0..20).map(|_| json!({"state": "y".repeat(100)})).collect();
        let error = tool_error(
            &call(&client, "map", json!({"questions": question, "records": records})).await,
        );
        assert!(error["message"].as_str().unwrap().contains("total"), "{error}");

        // A result that would flood the context: 100 records of a 255-option Choice.
        let options: Vec<Value> = (0..255).map(|i| json!({"name": format!("option-{i:03}")})).collect();
        let records: Vec<Value> = (0..100).map(|i| json!({"state": format!("{i}")})).collect();
        let error = tool_error(
            &call(
                &client,
                "map",
                json!({"questions": [{"id": "c", "type": "choice", "instructions": "?", "options": options}],
                       "records": records}),
            )
            .await,
        );
        assert!(error["message"].as_str().unwrap().contains("result would be"), "{error}");
        client.cancel().await.unwrap();
    });
    assert_eq!(api.hits(), 0);
}

#[test]
fn provider_failures_are_classified_tool_errors_not_answers() {
    let cases: [(Vec<Reply>, &str); 4] = [
        (vec![Reply::status(401, r#"{"detail":"bad key"}"#)], "auth"),
        (vec![Reply::status(503, "{}")], "unavailable"),
        (vec![Reply::ok("{not json")], "unavailable"),
        (vec![Reply::status(422, r#"{"detail":"nope"}"#)], "usage"),
    ];
    for (replies, kind) in cases {
        let api = MockApi::start(replies);
        runtime().block_on(async {
            let (command, _cwd) = serve_command(&api.endpoint(), &["--retries", "1"]);
            let client = connect(command, ProtocolVersion::V_2025_11_25).await;
            let error = tool_error(
                &call(&client, "noul", json!({"state": "x", "instructions": "?"})).await,
            );
            assert_eq!(error["kind"], kind, "{error}");
            assert!(!error.to_string().contains(CANARY_KEY));
            client.cancel().await.unwrap();
        });
    }
}

#[test]
fn a_rejected_credential_ends_map_as_an_auth_error_not_a_table_of_failures() {
    let api = MockApi::start(vec![Reply::status(401, r#"{"detail":"bad key"}"#)]);
    runtime().block_on(async {
        let (command, _cwd) = serve_command(&api.endpoint(), &[]);
        let client = connect(command, ProtocolVersion::V_2025_11_25).await;
        let records: Vec<Value> = (0..20).map(|i| json!({"state": format!("r{i}")})).collect();
        let error = tool_error(
            &call(
                &client,
                "map",
                json!({"questions": [{"id": "q", "type": "noul", "instructions": "?"}],
                       "records": records, "concurrency": 1}),
            )
            .await,
        );
        assert_eq!(error["kind"], "auth", "{error}");
        client.cancel().await.unwrap();
    });
    // The batch stopped at the first rejection instead of sending all twenty.
    assert_eq!(api.hits(), 1);
}

#[test]
fn a_transient_failure_is_retried_through_the_shared_policy() {
    let api = MockApi::start(vec![
        Reply::status(429, "{}").header("retry-after", "0"),
        Reply::status(503, "{}"),
        Reply::ok(noul_body()),
    ]);
    runtime().block_on(async {
        let (command, _cwd) = serve_command(&api.endpoint(), &["--retries", "2"]);
        let client = connect(command, ProtocolVersion::V_2025_11_25).await;
        let document =
            structured(&call(&client, "noul", json!({"state": "x", "instructions": "?"})).await);
        assert_eq!(document["answers"]["answer"]["noul"], 0.92);
        client.cancel().await.unwrap();
    });
    assert_eq!(api.hits(), 3);
}

#[test]
fn a_timeout_and_a_refused_connection_are_unavailable() {
    let hanging = MockApi::hanging();
    for endpoint in [hanging.endpoint(), MockApi::dead_endpoint()] {
        runtime().block_on(async {
            let (command, _cwd) = serve_command(&endpoint, &["--timeout", "1", "--retries", "0"]);
            let client = connect(command, ProtocolVersion::V_2025_11_25).await;
            let error = tool_error(
                &call(&client, "noul", json!({"state": "x", "instructions": "?"})).await,
            );
            assert_eq!(error["kind"], "unavailable", "{endpoint}: {error}");
            client.cancel().await.unwrap();
        });
    }
}

// --- Credentials -----------------------------------------------------------------------

#[test]
fn a_missing_credential_is_an_auth_error_and_nothing_is_sent() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let (mut command, _cwd) = serve_command(&api.endpoint(), &[]);
    command
        .env_remove("JEV_CUSTOM_API_KEY")
        .env_remove("JEV_API_KEY");
    runtime().block_on(async {
        let client = connect(command, ProtocolVersion::V_2025_11_25).await;
        let error =
            tool_error(&call(&client, "noul", json!({"state": "x", "instructions": "?"})).await);
        assert_eq!(error["kind"], "auth");
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("JEV_CUSTOM_API_KEY")
        );
        client.cancel().await.unwrap();
    });
    assert_eq!(api.hits(), 0);
}

#[test]
fn a_typesafe_key_never_reaches_a_custom_endpoint() {
    // Only the official-endpoint sources are populated. ADR-0008: a non-official host
    // reads `JEV_CUSTOM_API_KEY` and nothing else, so there is no credential to send.
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let (mut command, _cwd) = serve_command(&api.endpoint(), &[]);
    command
        .env_remove("JEV_CUSTOM_API_KEY")
        .env("JEV_API_KEY", CANARY_KEY)
        .env("TYPESAFE_API_KEY", CANARY_KEY);
    runtime().block_on(async {
        let client = connect(command, ProtocolVersion::V_2025_11_25).await;
        let error =
            tool_error(&call(&client, "noul", json!({"state": "x", "instructions": "?"})).await);
        assert_eq!(error["kind"], "auth");
        client.cancel().await.unwrap();
    });
    assert_eq!(api.hits(), 0, "a request reached the custom endpoint");
}

#[test]
fn the_key_appears_in_no_stream_even_on_failure_and_under_verbose() {
    let api = MockApi::start(vec![
        Reply::ok(noul_body()),
        Reply::status(401, format!(r#"{{"detail":"rejected {CANARY_KEY}"}}"#)),
    ]);
    let mut raw = Raw::spawn(&api.endpoint(), &["--verbose"]);
    let init = raw.initialize("2025-11-25");
    raw.call(1, "noul", &json!({"state": "x", "instructions": "?"}));
    let ok = raw.reply(1);
    raw.call(2, "noul", &json!({"state": "x", "instructions": "?"}));
    let failed = raw.reply(2);
    raw.call(3, "noul", &json!({"state": "x"}));
    let invalid = raw.reply(3);
    let (status, stderr, rest) = raw.close();
    assert!(status.success());
    for message in [&init, &ok, &failed, &invalid] {
        assert!(
            !message.to_string().contains(CANARY_KEY),
            "leaked: {message}"
        );
    }
    assert!(!stderr.contains(CANARY_KEY), "leaked on stderr: {stderr}");
    assert!(rest.iter().all(|line| !line.contains(CANARY_KEY)));
    // `--verbose` logs the call, never its state.
    assert!(stderr.contains("jev mcp: noul: ok"), "{stderr}");
    assert!(stderr.contains("non-official endpoint"), "{stderr}");
}

// --- Streams, shutdown, and framing ----------------------------------------------------

#[test]
fn stdout_carries_only_protocol_messages_and_stderr_is_silent_by_default() {
    // The official endpoint is the default, so the only stderr line a user could see --
    // the non-official-endpoint warning -- is absent here. Calls fail at auth without a
    // key, which exercises the error path without a network.
    let mut command = jev_spawnable();
    let cwd = tempfile::tempdir().unwrap();
    command
        .current_dir(cwd.path())
        .env_remove("JEV_API_KEY")
        .env_remove("JEV_CUSTOM_API_KEY")
        .args(["mcp", "serve"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for message in [
        json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-06-18",
               "capabilities": {}, "clientInfo": {"name": "raw", "version": "0"}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "noul",
               "arguments": {"state": "x", "instructions": "?"}}}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "map",
               "arguments": {"questions": [{"id": "q", "type": "noul", "instructions": "?"}],
                             "records": [{"state": "a"}, {"state": "b"}]}}}),
        json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "nope"}}),
    ] {
        writeln!(stdin, "{message}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let mut ids = Vec::new();
    for line in stdout.lines() {
        let message: Value = serde_json::from_str(line)
            .unwrap_or_else(|error| panic!("non-protocol bytes on stdout ({error}): {line:?}"));
        assert_eq!(message["jsonrpc"], "2.0");
        ids.push(message["id"].as_u64().unwrap());
    }
    ids.sort_unstable();
    assert_eq!(ids, [0, 1, 2, 3, 4], "{stdout}");
    assert!(stdout.ends_with('\n'));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "",
        "stderr was not silent"
    );
}

#[test]
fn closing_stdin_ends_the_server_even_with_a_call_in_flight() {
    let api = MockApi::hanging();
    let mut raw = Raw::spawn(&api.endpoint(), &["--timeout", "3600", "--retries", "0"]);
    raw.initialize("2025-11-25");
    raw.call(1, "noul", &json!({"state": "x", "instructions": "?"}));
    assert!(
        wait_until(PATIENCE, || api.hits() == 1),
        "the call never started"
    );
    // `close` itself fails the test if the process has not exited within `PATIENCE`,
    // which is far below the hour-long HTTP timeout configured above.
    let (status, _, _) = raw.close();
    assert!(status.success(), "{status:?}");
}

#[test]
fn a_cancelled_call_stops_retrying_and_the_server_keeps_serving() {
    // Every attempt is a 503 with a long Retry-After, so without cancellation the call
    // would keep the backend busy for minutes.
    let api = MockApi::start(vec![Reply::status(503, "{}").header("retry-after", "30")]);
    let mut raw = Raw::spawn(&api.endpoint(), &["--retries", "10", "--verbose"]);
    raw.initialize("2025-11-25");
    raw.call(1, "noul", &json!({"state": "x", "instructions": "?"}));
    assert!(wait_until(PATIENCE, || api.hits() == 1));
    raw.send(
        &json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
                     "params": {"requestId": 1, "reason": "user pressed stop"}}),
    );
    // Still responsive, and the cancelled call gets no reply (MCP: a receiver of a
    // cancellation SHOULD NOT send a response for the cancelled request).
    raw.send(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    let reply = raw.recv();
    assert_eq!(reply["id"], 2, "{reply}");
    let (status, stderr, rest) = raw.close();
    assert!(status.success());
    assert!(
        rest.iter().all(|line| !line.contains("\"id\":1")),
        "{rest:?}"
    );
    assert_eq!(api.hits(), 1, "the cancelled call kept retrying");
    // The call observed the cancellation itself, rather than merely being outlived.
    assert!(stderr.contains("jev mcp: noul: cancelled"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn ctrl_c_stops_the_server_with_the_interrupt_status() {
    let api = MockApi::start(vec![]);
    let mut raw = Raw::spawn(&api.endpoint(), &[]);
    raw.initialize("2025-11-25");
    let pid = raw.child.id().to_string();
    let signalled = std::process::Command::new("kill")
        .args(["-INT", &pid])
        .status()
        .unwrap();
    assert!(signalled.success());
    assert!(
        wait_until(PATIENCE, || matches!(raw.child.try_wait(), Ok(Some(_)))),
        "the server ignored SIGINT"
    );
    let status = raw.child.wait().unwrap();
    assert_eq!(status.code(), Some(130));
}

#[test]
fn a_line_longer_than_the_message_limit_ends_the_session_instead_of_growing() {
    let api = MockApi::start(vec![]);
    // The floor is 16 MiB; 17 MiB of one unterminated line exceeds it.
    let mut raw = Raw::spawn(&api.endpoint(), &[]);
    raw.initialize("2025-11-25");
    let chunk = vec![b'x'; 1 << 20];
    let mut refused = false;
    for _ in 0..17 {
        if raw.send_bytes(&chunk).is_err() {
            refused = true;
            break;
        }
    }
    assert!(
        refused || wait_until(PATIENCE, || matches!(raw.child.try_wait(), Ok(Some(_)))),
        "the server kept buffering an unbounded line"
    );
    let (status, stderr, _) = raw.close();
    // Not a quiet status 0, which would be indistinguishable from a host disconnecting.
    assert_eq!(status.code(), Some(74), "{stderr}");
    assert!(stderr.contains("longer than"), "{stderr}");
}

#[test]
fn a_malformed_message_does_not_put_anything_but_protocol_on_stdout() {
    let api = MockApi::start(vec![]);
    let mut raw = Raw::spawn(&api.endpoint(), &[]);
    raw.initialize("2025-11-25");
    raw.send_bytes(b"this is not json\n").unwrap();
    raw.send_bytes("{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/list\"}\n".as_bytes())
        .unwrap();
    // Every line `recv` returns is checked to be JSON-RPC; the list reply still arrives.
    assert_eq!(raw.reply(9)["result"]["tools"].as_array().unwrap().len(), 5);
    let (_, _, rest) = raw.close();
    for line in rest {
        let _: Value = serde_json::from_str(&line).expect("stdout stayed protocol-only");
    }
}

#[test]
fn dry_run_is_refused_and_help_goes_to_stdout() {
    let output = jev_spawnable()
        .args(["--dry-run", "mcp", "serve"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());

    let output = jev_spawnable().args(["mcp", "--help"]).output().unwrap();
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    assert!(help.contains("serve"), "{help}");
    assert!(help.contains("TypeSafe"), "{help}");
}

// --- Output schemas ----------------------------------------------------------------------

/// Checks `value` against the JSON Schema subset the bundled schemas use (the unit tests
/// in `src/mcp/server.rs` pin them to that subset), returning every violation.
fn violations(schema: &Value, value: &Value, path: &str, found: &mut Vec<String>) {
    if let Some(branches) = schema.get("anyOf").and_then(Value::as_array) {
        let fits = branches.iter().any(|branch| {
            let mut inner = Vec::new();
            violations(branch, value, path, &mut inner);
            inner.is_empty()
        });
        if !fits {
            found.push(format!("{path}: {value} matches no anyOf branch"));
            return;
        }
    }
    if let Some(kind) = schema.get("type") {
        let kinds: Vec<&str> = match kind {
            Value::Array(list) => list.iter().filter_map(Value::as_str).collect(),
            other => vec![other.as_str().unwrap()],
        };
        let actual = match value {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        };
        let ok = kinds.contains(&actual) || (actual == "integer" && kinds.contains(&"number"));
        if !ok {
            found.push(format!("{path}: {actual} is not {kinds:?}"));
            return;
        }
    }
    if let Some(allowed) = schema.get("enum").and_then(Value::as_array)
        && !allowed.contains(value)
    {
        found.push(format!("{path}: {value} not in {allowed:?}"));
    }
    if let (Some(minimum), Some(number)) = (schema.get("minimum"), value.as_f64())
        && number < minimum.as_f64().unwrap()
    {
        found.push(format!("{path}: {number} < {minimum}"));
    }
    if let (Some(maximum), Some(number)) = (schema.get("maximum"), value.as_f64())
        && number > maximum.as_f64().unwrap()
    {
        found.push(format!("{path}: {number} > {maximum}"));
    }
    if let Value::Object(object) = value {
        for required in schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if !object.contains_key(required.as_str().unwrap()) {
                found.push(format!("{path}: missing {required}"));
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        for (key, child) in object {
            match properties.and_then(|map| map.get(key)) {
                Some(property) => violations(property, child, &format!("{path}.{key}"), found),
                None => match schema.get("additionalProperties") {
                    Some(Value::Bool(false)) => found.push(format!("{path}: unexpected {key}")),
                    Some(extra @ Value::Object(_)) => {
                        violations(extra, child, &format!("{path}.{key}"), found);
                    }
                    _ => {}
                },
            }
        }
    }
    if let (Some(items), Value::Array(list)) = (schema.get("items"), value) {
        for (index, item) in list.iter().enumerate() {
            violations(items, item, &format!("{path}[{index}]"), found);
        }
    }
}

#[test]
fn every_result_conforms_to_the_output_schema_its_tool_declares() {
    let api = MockApi::start(vec![
        Reply::ok(noul_body()),
        Reply::ok(choice_body()),
        Reply::ok(score_body()),
        Reply::ok(mixed_body()),
        Reply::ok(noul_body()),
        Reply::status(503, "{}"),
    ]);
    runtime().block_on(async {
        let (command, _cwd) = serve_command(&api.endpoint(), &["--retries", "0"]);
        let client = connect(command, ProtocolVersion::V_2025_11_25).await;
        let schemas: std::collections::BTreeMap<String, Value> = client
            .list_all_tools()
            .await
            .unwrap()
            .into_iter()
            .map(|tool| {
                let schema = Value::Object(tool.output_schema.unwrap().as_ref().clone());
                (tool.name.to_string(), schema)
            })
            .collect();
        let question = json!([{"id": "answer", "type": "noul", "instructions": "?"}]);
        let calls = [
            ("noul", json!({"state": "x", "instructions": "?"})),
            ("choice", json!({"state": "x", "instructions": "?", "id": "team",
                              "options": [{"name": "billing"}, {"name": "technical"}, {"name": "sales"}]})),
            ("score", json!({"state": "x", "instructions": "?", "id": "risk",
                             "levels": ["Calm", "Frustrated", "Very angry"]})),
            ("ask", json!({"state": "x", "questions": [
                {"id": "api_change", "type": "noul", "instructions": "?"},
                {"id": "team", "type": "choice", "instructions": "?", "options": [{"name": "billing"}, {"name": "tech"}]},
                {"id": "risk", "type": "score", "instructions": "?", "levels": ["low", "high"]}]})),
            // One good row and one failed row, so both row shapes are checked.
            ("map", json!({"questions": question, "records": [{"state": "a"}, {"state": "b"}],
                           "concurrency": 1})),
        ];
        for (tool, arguments) in calls {
            let document = structured(&call(&client, tool, arguments).await);
            let mut found = Vec::new();
            violations(&schemas[tool], &document, tool, &mut found);
            assert!(found.is_empty(), "{tool}: {found:#?}\n{document}");
        }
        client.cancel().await.unwrap();
    });
}

// --- Further coverage --------------------------------------------------------------------

#[test]
fn an_unknown_protocol_version_falls_back_to_the_newest_handshake_revision() {
    let api = MockApi::start(vec![]);
    let mut raw = Raw::spawn(&api.endpoint(), &[]);
    let reply = raw.initialize("1999-01-01");
    assert_eq!(reply["result"]["protocolVersion"], "2025-11-25", "{reply}");
    let (status, _, _) = raw.close();
    assert!(status.success());
}

#[test]
fn map_keeps_input_order_under_concurrency_and_uses_a_per_call_model() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    runtime().block_on(async {
        let (command, _cwd) = serve_command(&api.endpoint(), &[]);
        let client = connect(command, ProtocolVersion::V_2025_11_25).await;
        let records: Vec<Value> = (0..12)
            .map(|i| json!({"id": format!("r{i}"), "state": format!("s{i}")}))
            .collect();
        let document = structured(
            &call(
                &client,
                "map",
                json!({"questions": [{"id": "answer", "type": "noul", "instructions": "?"}],
                       "records": records, "concurrency": 4, "model": "jev-1.13.0"}),
            )
            .await,
        );
        let ids: Vec<&str> = document["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        let expected: Vec<String> = (0..12).map(|i| format!("r{i}")).collect();
        assert_eq!(ids, expected);
        client.cancel().await.unwrap();
    });
    let requests = api.requests();
    assert_eq!(requests.len(), 12);
    assert!(
        requests
            .iter()
            .all(|request| request.body.contains("\"model\":\"jev-1.13.0\""))
    );
}

#[test]
fn unusual_unicode_and_control_characters_reach_the_api_intact() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    let state = "bidi \u{202e}override, nul \u{0} bell \u{7}, emoji \u{1f980}, zero-width \u{200b}";
    runtime().block_on(async {
        let (command, _cwd) = serve_command(&api.endpoint(), &[]);
        let client = connect(command, ProtocolVersion::V_2025_11_25).await;
        structured(
            &call(
                &client,
                "noul",
                json!({"state": state, "instructions": "?"}),
            )
            .await,
        );
        client.cancel().await.unwrap();
    });
    let body: Value = serde_json::from_str(&api.requests()[0].body).unwrap();
    assert_eq!(
        body["state"], state,
        "the state was altered on the way to the API"
    );
}

#[test]
fn an_answer_the_api_skipped_is_reported_not_silently_absent() {
    let body = json!({"model": "jev-1.13.0", "answers": {"a": {"type": "noul", "noul": 0.2}},
                      "usage": {"input_tokens": 1, "output_tokens": 1}})
    .to_string();
    let api = MockApi::start(vec![Reply::ok(body)]);
    runtime().block_on(async {
        let (command, _cwd) = serve_command(&api.endpoint(), &[]);
        let client = connect(command, ProtocolVersion::V_2025_11_25).await;
        let document = structured(
            &call(
                &client,
                "ask",
                json!({"state": "x", "model": "jev-1.13.0", "questions": [
                    {"id": "a", "type": "noul", "instructions": "?"},
                    {"id": "b", "type": "noul", "instructions": "?"}]}),
            )
            .await,
        );
        assert_eq!(document["missing_answers"], json!(["b"]));
        client.cancel().await.unwrap();
    });
    assert!(api.requests()[0].body.contains("\"model\":\"jev-1.13.0\""));
}

#[test]
fn at_most_four_calls_run_at_once_and_the_rest_wait() {
    let api = MockApi::hanging();
    let mut raw = Raw::spawn(&api.endpoint(), &["--timeout", "3600", "--retries", "0"]);
    raw.initialize("2025-11-25");
    for id in 1..=6 {
        raw.call(id, "noul", &json!({"state": "x", "instructions": "?"}));
    }
    assert!(
        wait_until(PATIENCE, || api.hits() == 4),
        "hits: {}",
        api.hits()
    );
    // A round trip through the server, so a fifth call would have had time to start.
    raw.send(&json!({"jsonrpc": "2.0", "id": 99, "method": "tools/list"}));
    assert_eq!(raw.recv()["id"], 99);
    assert_eq!(api.hits(), 4, "more than four calls ran at once");
    let (status, _, _) = raw.close();
    assert!(status.success());
}

#[test]
fn a_record_id_that_is_not_a_string_or_number_is_refused_and_large_ids_count() {
    let api = MockApi::start(vec![Reply::ok(noul_body())]);
    runtime().block_on(async {
        let (command, _cwd) = serve_command(&api.endpoint(), &[]);
        let client = connect(command, ProtocolVersion::V_2025_11_25).await;
        let question = json!([{"id": "q", "type": "noul", "instructions": "?"}]);
        let error = tool_error(
            &call(
                &client,
                "map",
                json!({"questions": question,
                "records": [{"id": {"nested": true}, "state": "x"}]}),
            )
            .await,
        );
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("string or a number"),
            "{error}"
        );
        // A tiny state with a large id would flood the result through the echoed id.
        let error = tool_error(
            &call(
                &client,
                "map",
                json!({"questions": question,
                "records": [{"id": "i".repeat(200 * 1024), "state": "x"}]}),
            )
            .await,
        );
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("result would be"),
            "{error}"
        );
        client.cancel().await.unwrap();
    });
    assert_eq!(api.hits(), 0);
}
