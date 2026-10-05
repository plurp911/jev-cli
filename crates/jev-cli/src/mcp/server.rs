//! The protocol adapter: rmcp's `ServerHandler`, over the tool functions.
//!
//! Everything protocol-shaped lives here and nothing else does. Negotiation, framing,
//! JSON-RPC, cancellation bookkeeping, and the version matrix are the official SDK's;
//! what a tool *does* is `tools.rs`, which is the CLI's code.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context as TaskContext, Poll};
use std::time::Instant;

use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
    Implementation, JsonObject, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
    ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::Value;
use tokio::io::{AsyncRead, ReadBuf};

use crate::interrupt::InterruptibleClock;
use crate::mcp::tools::{self, Deps};

/// The server-level instructions, surfaced by hosts that read them (Claude Code and
/// Codex both do). Deliberately short: the `jev-cli` Agent Skill carries the rest.
pub(crate) const INSTRUCTIONS: &str = "Bounded semantic judgements: Noul (yes/no), Choice \
    (named options), and Score (ordered levels), with probabilities. State and explicitly \
    supplied images and videos are sent to the configured inference endpoint: TypeSafe, Cloudflare, \
    Ollama, llama.cpp, or the Python Hugging Face bridge. A local provider uses the configured local server. No generated \
    text. Prefer deterministic code for deterministic rules; preserve returned uncertainty.";

/// One tool's static definition.
struct Definition {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    input: &'static str,
    output: &'static str,
}

const EVALUATION_OUTPUT: &str = include_str!("schema/evaluation.output.json");

/// The tool surface, in presentation order. Names, schemas, and descriptions are a
/// public integration API: `tests/mcp.rs` snapshots the whole `tools/list` result.
const DEFINITIONS: [Definition; 5] = [
    Definition {
        name: "noul",
        title: "Jev Noul (yes/no)",
        description: "Yes/no judgement: the probability (0-1) that the instructions hold \
            for the given state. Returns answers.<id>.noul; there is no confidence field, \
            and 0.5 means genuinely unsure. For semantic judgement of text or JSON -- not \
            for generating text, and not for anything code can check exactly (arithmetic, \
            string matching, parsing). Use `choice` to pick one of several named options, \
            `score` to rate on ordered levels, `ask` for several questions about the same \
            state. The state is sent to the configured inference endpoint.",
        input: include_str!("schema/noul.input.json"),
        output: EVALUATION_OUTPUT,
    },
    Definition {
        name: "choice",
        title: "Jev Choice (one of named options)",
        description: "Pick one of 2-255 named options (Ollama: 2-26), with the \
            probability of every option and a confidence value. Returns \
            answers.<id>.choice, .probabilities, and .confidence. For routing, \
            categorising, and triage. Not for generating text or for rules code can check \
            exactly. Use `noul` for a single yes/no, `score` when the options are ordered \
            levels, `ask` to combine with other questions about the same state. The state \
            is sent to the configured inference endpoint.",
        input: include_str!("schema/choice.input.json"),
        output: EVALUATION_OUTPUT,
    },
    Definition {
        name: "score",
        title: "Jev Score (ordered levels)",
        description: "Rate the state on 2-10 ordered levels (Ollama: 2-26; Python Hugging Face bridge: 2-255), \
            lowest first. Returns answers.<id>.score (the probability-weighted mean level, \
            0-based, which can fall between levels), the probability of every level, and \
            a confidence value. For severity, risk, quality, or \
            priority on your own rubric. Not for generating text or for quantities code \
            can compute. Use `choice` when the options are not ordered, `noul` for yes/no. \
            The state is sent to the configured inference endpoint.",
        input: include_str!("schema/score.input.json"),
        output: EVALUATION_OUTPUT,
    },
    Definition {
        name: "ask",
        title: "Jev Ask (several questions, one state)",
        description: "Several independent noul, choice, and score questions about ONE \
            state, in a single request. Questions share state and explicit images. Returns one answer per question id, each with its full \
            probabilities. Use instead of calling noul, choice, or score repeatedly on the \
            same state; use `map` to ask the same questions of many different states. The \
            state is sent to the configured inference endpoint.",
        input: include_str!("schema/ask.input.json"),
        output: EVALUATION_OUTPUT,
    },
    Definition {
        name: "map",
        title: "Jev Map (same questions, many records)",
        description: "Many records, each its own state: the same question set over up to \
            100 inline records, one request per record, run concurrently, results in input \
            order. Several questions about ONE state -- even one with several parts, such \
            as a multi-file diff -- are a single `ask` call, not a map. Returns one row per \
            record sent -- its answers, or an `error` for a record that failed, which is \
            not an answer -- and a summary. For larger or offline jobs, files on disk, CI gates, or \
            calibrated evaluation, run the `jev map` or `jev eval` CLI instead. Every \
            record's state is sent to the configured inference endpoint.",
        input: include_str!("schema/map.input.json"),
        output: include_str!("schema/map.output.json"),
    },
];

/// Builds the `tools/list` entries.
///
/// # Errors
///
/// Fails only if a bundled schema is not a JSON object, which `tests` rules out; the
/// error is returned rather than panicked so the no-panic rule holds regardless.
pub(crate) fn definitions() -> Result<Vec<Tool>, String> {
    let object = |text: &str, what: &str| -> Result<Arc<JsonObject>, String> {
        serde_json::from_str::<JsonObject>(text)
            .map(Arc::new)
            .map_err(|error| format!("the bundled {what} schema is not a JSON object: {error}"))
    };
    DEFINITIONS
        .iter()
        .map(|definition| {
            Ok(Tool::new(
                definition.name,
                definition.description,
                object(definition.input, definition.name)?,
            )
            .with_title(definition.title)
            .with_raw_output_schema(object(definition.output, definition.name)?)
            .with_annotations(annotations(definition.title)))
        })
        .collect()
}

/// The same four hints for every tool, because every tool does the same kind of thing.
///
/// They are hints, not a control (MCP spec, "Tool annotations"): what actually keeps
/// these tools harmless is that the server has no code that writes a file, runs a
/// command, or changes configuration.
fn annotations(title: &str) -> ToolAnnotations {
    ToolAnnotations::with_title(title)
        // Nothing local is modified: no file, no configuration, no credential.
        .read_only(true)
        .destructive(false)
        // Not idempotent: a repeat is another billed request and another sample from a
        // probabilistic model. The spec makes this hint meaningful only when
        // `readOnlyHint` is false, so it is stated for accuracy, not relied on.
        .idempotent(false)
        // The state goes to an external service.
        .open_world(true)
}

/// The `ServerHandler`. Cheap to clone; every call gets its own copy of `deps`.
#[derive(Clone)]
pub(crate) struct JevServer {
    deps: Deps,
    tools: Arc<Vec<Tool>>,
    verbose: bool,
    slots: Arc<tokio::sync::Semaphore>,
}

/// Tool calls that may run at once; later ones wait their turn. With `map` capped at
/// 16 requests in flight, the server never has more than 64 outstanding, the CLI's own
/// ceiling for one batch.
const MAX_CONCURRENT_CALLS: usize = 4;

impl std::fmt::Debug for JevServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JevServer")
            .field("tools", &self.tools.len())
            .finish_non_exhaustive()
    }
}

impl JevServer {
    pub(crate) fn new(deps: Deps, verbose: bool) -> Result<Self, String> {
        Ok(Self {
            deps,
            tools: Arc::new(definitions()?),
            verbose,
            slots: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_CALLS)),
        })
    }

    /// One line on stderr per call under `--verbose`: the tool, the outcome, the time.
    /// Never the arguments, which are the user's data.
    fn log(&self, tool: &str, outcome: &str, started: Instant) {
        if self.verbose {
            use std::io::Write as _;
            let _ = writeln!(
                std::io::stderr(),
                "jev mcp: {tool}: {outcome} ({} ms)",
                started.elapsed().as_millis()
            );
        }
    }
}

impl ServerHandler for JevServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            // Explicit: rmcp's default names the server "rmcp".
            .with_server_info(
                Implementation::new("jev", env!("CARGO_PKG_VERSION")).with_title("Jev"),
            )
            .with_instructions(INSTRUCTIONS)
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.iter().find(|tool| tool.name == name).cloned()
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let mut result = ListToolsResult::with_all_items(self.tools.as_ref().clone());
        // 2026-07-28 makes `ttlMs` and `cacheScope` required on a list result, and a
        // strict client rejects the list without them (the MCP Inspector does). rmcp's
        // own `#[tool_handler]` fills them the same way; a hand-written handler has to.
        // Earlier revisions do not define the fields, so they are left out there.
        if context
            .protocol_version()
            .is_some_and(|version| version >= ProtocolVersion::V_2026_07_28)
        {
            result.ttl_ms = Some(0);
            result.cache_scope = Some(CacheScope::Public);
        }
        Ok(result)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let started = Instant::now();
        let name = request.name.to_string();
        if !tools::NAMES.contains(&name.as_str()) {
            // A protocol error, per the spec: the tool does not exist, so there is no
            // tool result to report.
            return Err(ErrorData::invalid_params(
                format!("unknown tool: {}", crate::output::sanitize(&name)),
                None,
            ));
        }
        let arguments = Value::Object(request.arguments.unwrap_or_default());

        // The call runs on the blocking pool, because the core is blocking by design
        // (ADR-0007). Cancellation is cooperative: the flag reaches the retry wait and
        // the batch loop through the clock, and the in-flight HTTP attempt, if any, ends
        // at its own timeout. The result of a cancelled call is discarded by rmcp.
        // A server takes several calls at once and each `map` runs its own workers, so
        // the calls share one budget rather than each assuming the machine is theirs.
        // Waiting for a slot is itself cancellable.
        let slot = tokio::select! {
            () = context.ct.cancelled() => {
                return Err(ErrorData::internal_error("the request was cancelled", None));
            }
            slot = Arc::clone(&self.slots).acquire_owned() => slot
                .map_err(|_| ErrorData::internal_error("the server is shutting down", None))?,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let clock = InterruptibleClock::with_cancel(Arc::clone(&cancel));
        let deps = self.deps.clone();
        let tool = name.clone();
        // The slot moves into the blocking task, so it is released when the work ends,
        // not when the handler returns: a cancelled call keeps running until its next
        // wait or record, and must keep counting against the limit until then.
        let work = tokio::task::spawn_blocking(move || {
            let _slot = slot;
            tools::call(&deps, &clock, &tool, arguments)
        });

        let joined = tokio::select! {
            () = context.ct.cancelled() => {
                cancel.store(true, Ordering::SeqCst);
                self.log(&name, "cancelled", started);
                return Err(ErrorData::internal_error("the request was cancelled", None));
            }
            joined = work => joined,
        };

        let result = match joined {
            Ok(Ok(document)) => {
                self.log(&name, "ok", started);
                // `structured` sets `structuredContent` and also a text block with the
                // same JSON, which the spec asks for so that clients that predate
                // structured output still see the result. One document, two encodings.
                CallToolResult::structured(document)
            }
            Ok(Err(failure)) => {
                self.log(&name, failure.kind, started);
                CallToolResult::error(vec![ContentBlock::text(failure.document().to_string())])
            }
            Err(_panicked) => {
                // The worker cannot panic on input -- panics are denied workspace-wide --
                // but a join failure must still be an error, never a silent success.
                self.log(&name, "internal", started);
                CallToolResult::error(vec![ContentBlock::text(
                    tools::ToolFailure {
                        kind: "internal",
                        message: "the tool call failed unexpectedly; this is a bug in jev"
                            .to_owned(),
                    }
                    .document()
                    .to_string(),
                )])
            }
        };
        Ok(result.into())
    }
}

/// A reader that refuses any line longer than `limit` bytes.
///
/// rmcp's stdio transport buffers a message until its newline, with no ceiling, so a
/// host that writes one enormous line would have the server allocate all of it. A
/// message this long cannot be a valid call anyway: every tool refuses state over
/// `--max-input-bytes` before sending. The error ends the session, which fails closed.
pub(crate) struct BoundedLines<R> {
    inner: R,
    run: usize,
    limit: usize,
    /// Set when the limit was hit. rmcp reports a transport read error as an ordinary
    /// close, so without this the session would end with status 0 and no word said.
    exceeded: Arc<AtomicBool>,
}

impl<R> BoundedLines<R> {
    pub(crate) const fn new(inner: R, limit: usize, exceeded: Arc<AtomicBool>) -> Self {
        Self {
            inner,
            run: 0,
            limit,
            exceeded,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for BoundedLines<R> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        std::task::ready!(Pin::new(&mut this.inner).poll_read(cx, buf))?;
        for &byte in buf.filled().get(before..).unwrap_or_default() {
            if byte == b'\n' {
                this.run = 0;
            } else {
                this.run += 1;
                if this.run > this.limit {
                    this.exceeded.store(true, Ordering::SeqCst);
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("an MCP message is longer than {} bytes", this.limit),
                    )));
                }
            }
        }
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    /// The keywords the bundled schemas may use.
    ///
    /// A deliberately small subset of JSON Schema 2020-12: hosts differ in how much of
    /// the dialect they understand, and `oneOf`, `$ref`, and `const` are where they
    /// differ most. Staying inside this set keeps every tool usable everywhere. `type`
    /// is always a single string -- a nullable field is `anyOf` two single types, which
    /// is what the MCP Inspector's portability check asks for.
    const PORTABLE: &[&str] = &[
        "type",
        "anyOf",
        "properties",
        "required",
        "additionalProperties",
        "items",
        "enum",
        "description",
        "minimum",
        // JSON Schema 2020-12 uses a numeric bound for strictly positive inputs.
        "exclusiveMinimum",
        "maximum",
        "minItems",
        "maxItems",
        "minLength",
    ];

    fn keywords(schema: &Value, path: &str, found: &mut Vec<String>) {
        let Some(object) = schema.as_object() else {
            return;
        };
        for (key, value) in object {
            if !PORTABLE.contains(&key.as_str()) {
                found.push(format!("{path}.{key}"));
            }
            if key == "type" && !value.is_string() {
                found.push(format!("{path}.type is not a single string"));
            }
            if key == "exclusiveMinimum" && !value.is_number() {
                found.push(format!("{path}.exclusiveMinimum is not a number"));
            }
            match key.as_str() {
                "anyOf" => {
                    for branch in value.as_array().into_iter().flatten() {
                        keywords(branch, &format!("{path}|"), found);
                    }
                }
                "properties" => {
                    for (name, property) in value.as_object().into_iter().flatten() {
                        keywords(property, &format!("{path}.{name}"), found);
                    }
                }
                "items" | "additionalProperties" => keywords(value, &format!("{path}[]"), found),
                _ => {}
            }
        }
    }

    #[test]
    fn portable_keywords_require_numeric_exclusive_minimum_and_refuse_unknowns() {
        let mut found = Vec::new();
        keywords(
            &serde_json::json!({"type":"number","exclusiveMinimum":0}),
            "cadence",
            &mut found,
        );
        assert!(found.is_empty());
        keywords(
            &serde_json::json!({"type":"number","exclusiveMinimum":true}),
            "cadence",
            &mut found,
        );
        assert_eq!(found, ["cadence.exclusiveMinimum is not a number"]);
        found.clear();
        keywords(
            &serde_json::json!({"type":"number","unsupportedKeyword":0}),
            "cadence",
            &mut found,
        );
        assert_eq!(found, ["cadence.unsupportedKeyword"]);
    }

    #[test]
    fn every_schema_is_an_object_rooted_portable_subset() {
        for definition in &DEFINITIONS {
            for (kind, text) in [("input", definition.input), ("output", definition.output)] {
                let schema: Value = serde_json::from_str(text).unwrap();
                assert_eq!(schema["type"], "object", "{} {kind}", definition.name);
                let mut found = Vec::new();
                keywords(&schema, definition.name, &mut found);
                assert!(found.is_empty(), "{} {kind}: {found:?}", definition.name);
            }
        }
    }

    #[test]
    fn definitions_build_and_are_in_the_documented_order() {
        let tools = definitions().unwrap();
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
        assert_eq!(names, tools::NAMES);
    }

    #[test]
    fn discovery_describes_provider_score_limits_and_single_instruction_fallbacks() {
        let tools = definitions().unwrap();
        let score = tools.iter().find(|tool| tool.name == "score").unwrap();
        let description = score.description.as_deref().unwrap();
        for limit in ["2-10", "Ollama: 2-26", "Python Hugging Face bridge: 2-255"] {
            assert!(
                description.contains(limit),
                "missing {limit}: {description}"
            );
        }
        for tool in tools
            .iter()
            .filter(|tool| ["noul", "choice", "score"].contains(&tool.name.as_ref()))
        {
            let instructions = &tool.input_schema["properties"]["instructions"]["description"];
            let description = instructions.as_str().unwrap();
            assert!(description.contains(
                "Omission uses the validated question id for Ollama and the publisher Python bridge"
            ));
            assert!(description.contains("required for other providers"));
            assert!(description.contains(
                "Only the Python bridge uses that fallback for null or an exact empty string"
            ));
        }
    }

    /// A minimal argument set for each tool, built from the schema's `required` list.
    fn minimal(name: &str) -> Value {
        let question = json!([{"id": "q", "type": "noul", "instructions": "?"}]);
        match name {
            "noul" => json!({"state": "s", "instructions": "?"}),
            "choice" => json!({"state": "s", "instructions": "?",
                               "options": [{"name": "a"}, {"name": "b"}]}),
            "score" => json!({"state": "s", "instructions": "?", "levels": ["lo", "hi"]}),
            "ask" => json!({"state": "s", "questions": question}),
            _ => json!({"questions": question, "records": [{"state": "s"}]}),
        }
    }

    /// Checks structural schema compatibility recursively, including nested array
    /// items. Deserialization alone cannot catch an advertised array becoming an object.
    fn schema_accepts_value(schema: &Value, value: &Value) -> bool {
        if let Some(variants) = schema["anyOf"].as_array() {
            return variants
                .iter()
                .any(|schema| schema_accepts_value(schema, value));
        }
        let matches_type = match schema["type"].as_str() {
            Some("object") => value.is_object(),
            Some("array") => value.is_array(),
            Some("string") => value.is_string(),
            Some("boolean") => value.is_boolean(),
            Some("integer") => value.is_i64() || value.is_u64(),
            Some("number") => value.is_number(),
            Some("null") => value.is_null(),
            None => true,
            _ => false,
        };
        if !matches_type {
            return false;
        }
        if let Some(required) = schema["required"].as_array()
            && required
                .iter()
                .any(|key| value.get(key.as_str().unwrap()).is_none())
        {
            return false;
        }
        if let Some(object) = value.as_object() {
            for (key, value) in object {
                if let Some(property) = schema["properties"].get(key) {
                    if !schema_accepts_value(property, value) {
                        return false;
                    }
                } else if schema["additionalProperties"] == false {
                    return false;
                }
            }
        }
        if let Some(items) = value.as_array()
            && let Some(item_schema) = schema.get("items")
        {
            return items
                .iter()
                .all(|value| schema_accepts_value(item_schema, value));
        }
        true
    }

    #[test]
    fn the_input_schemas_and_the_argument_types_agree() {
        // Every property the schema declares is accepted, every required one is
        // required, and nothing the schema omits is accepted. A schema that drifted
        // from the Rust type would advertise arguments the server then refuses.
        for definition in &DEFINITIONS {
            let schema: Value = serde_json::from_str(definition.input).unwrap();
            let base = minimal(definition.name);
            assert!(
                schema_accepts_value(&schema, &base),
                "{}: the advertised schema refuses the existing minimal payload",
                definition.name
            );
            assert!(
                tools::accepts(definition.name, base.clone()),
                "{}: the minimal call is refused",
                definition.name
            );
            for required in schema["required"].as_array().unwrap() {
                let mut without = base.clone();
                without
                    .as_object_mut()
                    .unwrap()
                    .remove(required.as_str().unwrap());
                assert!(
                    !tools::accepts(definition.name, without),
                    "{}: `{required}` is required by the schema but not by the type",
                    definition.name
                );
            }
            let declared: Vec<&String> = schema["properties"].as_object().unwrap().keys().collect();
            for key in base.as_object().unwrap().keys() {
                assert!(
                    declared.contains(&key),
                    "{}: `{key}` undeclared",
                    definition.name
                );
            }
            let mut extra = base.clone();
            extra
                .as_object_mut()
                .unwrap()
                .insert("surprise".to_owned(), json!(1));
            assert!(
                !tools::accepts(definition.name, extra),
                "{}",
                definition.name
            );
            for optional in declared {
                if base.get(optional.as_str()).is_some() {
                    continue;
                }
                let mut with = base.clone();
                let value = match optional.as_str() {
                    "criteria" => json!({"true": "yes", "false": "no"}),
                    "concurrency" => json!(2),
                    "images" | "videos" => json!([]),
                    "options" => json!({"rejectIfBusy":true}),
                    "reject_if_busy" => json!(true),
                    "keep_alive" => json!("5m"),
                    "max_length" | "max_state_tokens" => json!(4096),
                    "media_kwargs" => json!({}),
                    _ => json!("x"),
                };
                with.as_object_mut()
                    .unwrap()
                    .insert(optional.clone(), value);
                assert!(
                    schema_accepts_value(&schema, &with),
                    "{}: the advertised schema refuses optional `{optional}`",
                    definition.name
                );
                assert!(
                    tools::accepts(definition.name, with),
                    "{}: optional `{optional}` is declared but refused",
                    definition.name
                );
            }
        }
    }

    #[test]
    fn the_description_of_every_tool_says_where_the_state_goes() {
        for definition in &DEFINITIONS {
            assert!(
                definition
                    .description
                    .contains("sent to the configured inference endpoint"),
                "{}",
                definition.name
            );
            // Descriptions cost context on every turn in every host; keep them short.
            assert!(definition.description.len() < 700, "{}", definition.name);
        }
        assert!(
            INSTRUCTIONS.len() < 512,
            "Codex reads the first 512 characters"
        );
    }
}
