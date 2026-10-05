//! The five tools, as plain functions from arguments to a result document.
//!
//! Nothing in this file knows about MCP. Each tool deserializes its arguments, builds a
//! [`jev_core::EvaluationRequest`] with the same constructors the CLI uses, sends it
//! through the same [`crate::commands::evaluate::send`] or [`crate::commands::map`]
//! path, and returns the same `jev.evaluation/v1` or `jev.map.*` documents the CLI
//! prints. The protocol adapter in `mcp/server.rs` only moves bytes.
//!
//! # Why the question shape is an array here
//!
//! A request file on the CLI is the official API body, whose `questions` and Choice
//! `criteria` are JSON objects. The CLI parses those order-preserving (`crate::ordered`)
//! because option order is part of what a model sees. MCP arguments reach this crate
//! already decoded into a `serde_json::Map`, which sorts its keys, so an object-shaped
//! question set would be silently reordered. Arrays keep the order the agent wrote, and
//! they are also easier for a model to produce correctly than an object keyed by id.

use std::sync::Arc;

use jev_client::{Credential, Transport};
use jev_config::{Environment, SecretStore};
use jev_core::{
    ChoiceOption, Content, EvaluationRequest, ModelId, NoulCriteria, Question, QuestionId, State,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::commands::{self, evaluate, map};
use crate::context::Context;
use crate::errors::CliError;
use crate::interrupt::InterruptibleClock;
use crate::render::json as render_json;

/// Schema identifier of the MCP `map` result: the CLI's rows and summary, together.
pub(crate) const MCP_MAP_SCHEMA: &str = "jev.mcp.map/v1";

/// Most records one `map` call accepts.
///
/// A CLI batch streams rows to a file or a pipe; an MCP result lands in the agent's
/// context window, all at once. A hundred rows of a small question set is a few
/// thousand tokens, and is already more than an agent can usefully reason over in one
/// turn. Larger jobs belong in `jev map`, whose output never enters the context.
pub(crate) const MAX_MAP_RECORDS: usize = 100;

/// Ceiling on the estimated size of one `map` result, in bytes.
///
/// Checked before anything is sent, from the question set and the record count, so an
/// oversized batch costs nothing. JSON of this kind runs at roughly three to four bytes
/// a token, so 80 KiB is about 20k to 27k tokens: near Claude Code's default ceiling of
/// 25k tokens for one tool result, above which it saves the result to disk instead.
pub(crate) const MAX_MAP_OUTPUT_BYTES: usize = 80 * 1024;

/// Most requests in flight for one `map` call.
///
/// Lower than the CLI's 64: an MCP server takes several calls at once, and the budget
/// below is shared by all of them. See [`crate::mcp`]'s call limit.
pub(crate) const MAX_MAP_CONCURRENCY: usize = 16;

/// Default requests in flight for `map`, the same as `jev map --concurrency`.
const DEFAULT_MAP_CONCURRENCY: usize = 4;

/// Id a single-question tool uses when the caller does not name one, the same as the
/// CLI's `--id` default.
const DEFAULT_ID: &str = "answer";

/// Everything a tool needs, owned, so a call can run on a blocking thread.
#[derive(Clone)]
pub(crate) struct Deps {
    /// The resolved configuration: endpoint, default model, retry, timeout, limits.
    pub(crate) context: Arc<Context>,
    /// Where credentials are resolved from, read on every call.
    pub(crate) environment: Arc<dyn Environment + Send + Sync>,
    /// The OS credential store.
    pub(crate) store: Arc<dyn SecretStore + Send + Sync>,
    /// One transport for the life of the server, so calls share warm connections.
    pub(crate) transport: Arc<dyn Transport + Send + Sync>,
}

impl std::fmt::Debug for Deps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Deps")
            .field("context", &self.context)
            .finish_non_exhaustive()
    }
}

/// Why a tool call produced no result.
#[derive(Debug)]
pub(crate) struct ToolFailure {
    /// The same classification the CLI turns into an exit code.
    pub(crate) kind: &'static str,
    /// What went wrong, sanitized, with no credential material.
    pub(crate) message: String,
}

impl ToolFailure {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            kind: "usage",
            message: message.into(),
        }
    }

    /// The document sent back as the tool's error content.
    pub(crate) fn document(&self) -> Value {
        json!({"error": {"kind": self.kind, "message": self.message}})
    }
}

impl From<CliError> for ToolFailure {
    fn from(error: CliError) -> Self {
        let kind = match &error {
            CliError::Usage(_) | CliError::IncompleteCloudflareConfiguration => "usage",
            CliError::Auth(_) => "auth",
            CliError::Unavailable(_) => "unavailable",
            CliError::Io(_) | CliError::Internal(_) => "internal",
            CliError::Interrupted => "cancelled",
        };
        // `CliError`'s `Display` sanitizes the message, and no variant carries a
        // credential (see `errors.rs`).
        Self {
            kind,
            message: error.to_string(),
        }
    }
}

type Outcome = Result<Value, ToolFailure>;

/// The five tool names, in the order `tools/list` presents them.
pub(crate) const NAMES: [&str; 5] = ["noul", "choice", "score", "ask", "map"];

#[cfg(test)]
mod media_regressions {
    use super::*;

    #[test]
    fn local_question_instructions_can_be_omitted_at_argument_boundary() {
        assert!(accepts(
            "ask",
            serde_json::json!({
                "state": "receipt", "questions": [{"id": "readable", "type": "noul"}]
            })
        ));
    }

    #[test]
    fn omitted_instructions_use_normalized_id_only_for_supported_local_providers() {
        let args = || {
            parse::<QuestionArgs>(serde_json::json!({
                "id": "  readable  ", "type": "noul"
            }))
            .unwrap()
        };
        let (id, question) =
            question(args(), 10, true, crate::request::ContentMode::Strict).unwrap();
        assert_eq!(id.as_str(), "readable");
        assert_eq!(
            serde_json::to_value(question.instructions()).unwrap(),
            "readable"
        );
        assert!(super::question(args(), 10, false, crate::request::ContentMode::Strict).is_err());
    }

    #[test]
    fn mcp_accepts_structured_content_and_explicit_images() {
        assert!(accepts(
            "noul",
            serde_json::json!({
                "state": "receipt", "instructions": {"task": "Is it readable?"},
                "criteria": {"true": ["Legible"]}, "images": []
            })
        ));
        assert!(accepts(
            "ask",
            serde_json::json!({
                "state": "receipt", "images": [], "questions": [{
                    "id": "readable", "type": "noul", "instructions": ["Is it readable?"]
                }]
            })
        ));
    }
}

/// Runs one tool by name.
///
/// Blocking: it may wait on the network and on retry backoff. `clock` carries the
/// call's cancellation flag, so a cancelled call stops at the next wait or record.
pub(crate) fn call(
    deps: &Deps,
    clock: &InterruptibleClock,
    name: &str,
    arguments: Value,
) -> Outcome {
    match name {
        "noul" => noul(deps, clock, parse(arguments)?),
        "choice" => choice(deps, clock, parse(arguments)?),
        "score" => score(deps, clock, parse(arguments)?),
        "ask" => ask(deps, clock, parse(arguments)?),
        "map" => map_tool(deps, clock, parse(arguments)?),
        other => Err(ToolFailure::usage(format!(
            "unknown tool {:?}",
            crate::output::sanitize(other)
        ))),
    }
}

/// Whether `arguments` deserialize for `name`, without running anything. For the
/// schema-agreement test.
#[cfg(test)]
pub(crate) fn accepts(name: &str, arguments: Value) -> bool {
    match name {
        "noul" => parse::<NoulArgs>(arguments).is_ok(),
        "choice" => parse::<ChoiceArgs>(arguments).is_ok(),
        "score" => parse::<ScoreArgs>(arguments).is_ok(),
        "ask" => parse::<AskArgs>(arguments).is_ok(),
        "map" => parse::<MapArgs>(arguments).is_ok(),
        _ => false,
    }
}

/// Decodes arguments, turning a type mismatch into a message the model can act on.
fn parse<T: for<'de> Deserialize<'de>>(arguments: Value) -> Result<T, ToolFailure> {
    serde_json::from_value(arguments).map_err(|error| {
        let message = error
            .to_string()
            .replace("expected non-empty text", "text must not be empty");
        ToolFailure::usage(format!("invalid arguments: {message}"))
    })
}

// --- Argument shapes. Mirrored by the input schemas in `mcp/schema/`. ------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoulArgs {
    state: Value,
    #[serde(default, deserialize_with = "present_value")]
    instructions: Option<Value>,
    #[serde(default)]
    criteria: Option<NoulCriteriaArgs>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(flatten)]
    features: crate::media::Features,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoulCriteriaArgs {
    #[serde(default, rename = "true", deserialize_with = "present_value")]
    yes: Option<Value>,
    #[serde(default, rename = "false", deserialize_with = "present_value")]
    no: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChoiceArgs {
    state: Value,
    #[serde(default, deserialize_with = "present_value")]
    instructions: Option<Value>,
    options: Vec<OptionArgs>,
    #[serde(default)]
    reject_if_busy: Option<bool>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(flatten)]
    features: crate::media::Features,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionArgs {
    name: String,
    #[serde(default)]
    description: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScoreArgs {
    state: Value,
    #[serde(default, deserialize_with = "present_value")]
    instructions: Option<Value>,
    levels: Vec<Value>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(flatten)]
    features: crate::media::Features,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AskArgs {
    state: Value,
    questions: Vec<QuestionArgs>,
    #[serde(default)]
    model: Option<String>,
    #[serde(flatten)]
    features: crate::media::Features,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Noul,
    Choice,
    Score,
}

/// One question of an `ask` or `map` call.
///
/// Flat, with the per-type fields optional, rather than a tagged union: a `oneOf` input
/// schema is the construct MCP hosts are least consistent about, and the per-type rules
/// are enforced here anyway, with a message that names the question.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuestionArgs {
    id: String,
    #[serde(rename = "type")]
    kind: Kind,
    #[serde(default, deserialize_with = "present_value")]
    instructions: Option<Value>,
    #[serde(default)]
    criteria: Option<NoulCriteriaArgs>,
    #[serde(default)]
    options: Option<Vec<OptionArgs>>,
    #[serde(default)]
    levels: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MapArgs {
    questions: Vec<QuestionArgs>,
    records: Vec<RecordArgs>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    concurrency: Option<usize>,
    #[serde(flatten)]
    features: crate::media::Features,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordArgs {
    #[serde(default)]
    id: Option<Value>,
    state: Value,
    #[serde(default)]
    images: Vec<jev_core::EmbeddedImage>,
    #[serde(default)]
    videos: Vec<jev_core::EmbeddedVideo>,
}

fn present_value<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

fn content(
    mode: crate::request::ContentMode,
    value: Value,
    what: &str,
) -> Result<Content, ToolFailure> {
    mode.content(value)
        .map_err(|error| ToolFailure::usage(format!("{what}: {error}")))
}

fn level_content(
    mode: crate::request::ContentMode,
    values: Vec<Value>,
    what: &str,
) -> Result<Vec<Content>, ToolFailure> {
    values
        .into_iter()
        .map(|value| content(mode, value, what))
        .collect()
}

fn single_instructions(
    deps: &Deps,
    value: Option<Value>,
    id: Option<&str>,
) -> Result<Content, ToolFailure> {
    let id = QuestionId::new(id.unwrap_or(DEFAULT_ID))
        .map_err(|error| ToolFailure::usage(error.to_string()))?;
    crate::request::ContentMode::for_provider(deps.context.endpoint.value.provider())
        .instructions(
            value,
            id.as_str(),
            matches!(
                deps.context.endpoint.value.provider(),
                "ollama" | "huggingface"
            ),
        )
        .map_err(Into::into)
}

// --- Conversion into the core model. --------------------------------------------------

fn noul_criteria(
    criteria: Option<NoulCriteriaArgs>,
    what: &str,
    mode: crate::request::ContentMode,
) -> Result<Option<NoulCriteria>, ToolFailure> {
    let Some(criteria) = criteria else {
        return Ok(None);
    };
    let side = |value: Option<Value>| -> Result<Option<Content>, ToolFailure> {
        match value {
            None => Ok(None),
            Some(Value::Null) if mode == crate::request::ContentMode::Strict => Ok(None),
            Some(value) => content(mode, value, what).map(Some),
        }
    };
    let yes = side(criteria.yes)?;
    let no = side(criteria.no)?;
    if yes.is_none() && no.is_none() && mode == crate::request::ContentMode::Publisher {
        return Ok(None);
    }
    NoulCriteria::new(yes, no)
        .map(Some)
        .map_err(|error| ToolFailure::usage(format!("{what}: {error}")))
}

fn options(
    options: Vec<OptionArgs>,
    what: &str,
    mode: crate::request::ContentMode,
) -> Result<Vec<ChoiceOption>, ToolFailure> {
    // Duplicate names are refused by `Question::choice`, as they are for the CLI.
    options
        .into_iter()
        .map(|option| {
            let description = option
                .description
                .map(|value| content(mode, value, what))
                .transpose()?;
            ChoiceOption::new(option.name, description)
                .map_err(|error| ToolFailure::usage(format!("{what}: {error}")))
        })
        .collect()
}

fn question(
    args: QuestionArgs,
    score_max: usize,
    optional_instructions: bool,
    mode: crate::request::ContentMode,
) -> Result<(QuestionId, Question), ToolFailure> {
    let what = format!("question {:?}", crate::output::sanitize(&args.id));
    let id = QuestionId::new(args.id)
        .map_err(|error| ToolFailure::usage(format!("{what}: `id`: {error}")))?;
    let instructions = mode
        .instructions(args.instructions, id.as_str(), optional_instructions)
        .map_err(ToolFailure::from)?;
    // A field that belongs to another type is refused rather than ignored: an agent
    // that wrote `options` on a Score meant something, and billing it for a question
    // it did not ask is worse than saying so.
    let stray = |present: bool, field: &str| {
        if present {
            Err(ToolFailure::usage(format!(
                "{what}: `{field}` does not apply to a {} question",
                match args.kind {
                    Kind::Noul => "noul",
                    Kind::Choice => "choice",
                    Kind::Score => "score",
                }
            )))
        } else {
            Ok(())
        }
    };
    let question = match args.kind {
        Kind::Noul => {
            stray(args.options.is_some(), "options")?;
            stray(args.levels.is_some(), "levels")?;
            Question::noul(instructions, noul_criteria(args.criteria, &what, mode)?)
        }
        Kind::Choice => {
            stray(args.criteria.is_some(), "criteria")?;
            stray(args.levels.is_some(), "levels")?;
            let list = args.options.ok_or_else(|| {
                ToolFailure::usage(format!("{what}: a choice question needs `options`"))
            })?;
            Question::choice(instructions, options(list, &what, mode)?)
        }
        Kind::Score => {
            stray(args.criteria.is_some(), "criteria")?;
            stray(args.options.is_some(), "options")?;
            let list = args.levels.ok_or_else(|| {
                ToolFailure::usage(format!("{what}: a score question needs `levels`"))
            })?;
            Question::score_with_max(instructions, level_content(mode, list, &what)?, score_max)
        }
    }
    .map_err(|error| ToolFailure::usage(format!("{what}: {error}")))?;
    Ok((id, question))
}

/// Converts a question list. Duplicate ids are refused by
/// [`EvaluationRequest::new`], exactly as for a CLI request file.
fn questions(
    list: Vec<QuestionArgs>,
    endpoint: &jev_client::Endpoint,
) -> Result<Vec<(QuestionId, Question)>, ToolFailure> {
    list.into_iter()
        .map(|args| {
            question(
                args,
                crate::media::score_max(endpoint),
                matches!(endpoint.provider(), "ollama" | "huggingface"),
                crate::request::ContentMode::for_provider(endpoint.provider()),
            )
        })
        .collect()
}

/// Converts a state argument, applying the CLI's per-source byte ceiling.
fn state(
    deps: &Deps,
    value: Value,
    what: &str,
    images: &[jev_core::EmbeddedImage],
) -> Result<State, ToolFailure> {
    check_size(deps, &value, what)?;
    let images = if deps.context.endpoint.value.is_cloudflare() {
        images
    } else {
        &[]
    };
    crate::request::ContentMode::for_provider(deps.context.endpoint.value.provider())
        .state(value, what, images)
        .map_err(Into::into)
}

/// Refuses a value whose encoding exceeds `--max-input-bytes`.
///
/// The same ceiling, and the same "never silently truncated" promise, as a state read
/// from a file: the limit is a property of the input, not of how it arrived.
fn check_size(deps: &Deps, value: &Value, what: &str) -> Result<usize, ToolFailure> {
    let bytes = encoded_len(value);
    let limit = deps.context.max_input_bytes.value;
    if u64::try_from(bytes).unwrap_or(u64::MAX) > limit {
        return Err(ToolFailure::usage(format!(
            "{what} is {bytes} bytes, over the {limit} byte input limit. Nothing was sent. \
             Send less state, or raise the limit with `jev --max-input-bytes N mcp serve`."
        )));
    }
    Ok(bytes)
}

fn encoded_len(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
}

fn model(deps: &Deps, requested: Option<String>) -> Result<ModelId, ToolFailure> {
    // An explicit per-call model beats the server's default, the same precedence the CLI
    // gives `--model` over a request file.
    match requested {
        Some(name) => {
            ModelId::new(name).map_err(|error| ToolFailure::usage(format!("`model`: {error}")))
        }
        None => Ok(deps.context.model.value.clone()),
    }
}

fn credential(deps: &Deps) -> Result<Credential, ToolFailure> {
    // Resolved per call, not once at startup: `jev auth login` in another terminal takes
    // effect on the next call instead of requiring the host to restart the server.
    commands::resolve_credential(
        &deps.context,
        deps.environment.as_ref(),
        deps.store.as_ref(),
    )
    .map(|(credential, _source)| credential)
    .map_err(ToolFailure::from)
}

// --- The tools. -----------------------------------------------------------------------

fn single(
    deps: &Deps,
    clock: &InterruptibleClock,
    state_value: Value,
    id: Option<String>,
    question: Result<Question, jev_core::QuestionError>,
    model_name: Option<String>,
    features: crate::media::Features,
) -> Outcome {
    let question = question.map_err(|error| ToolFailure::usage(error.to_string()))?;
    let id = QuestionId::new(id.unwrap_or_else(|| DEFAULT_ID.to_owned()))
        .map_err(|error| ToolFailure::usage(format!("`id`: {error}")))?;
    let features = features.with_mcp_defaults(&deps.context)?;
    let state = state(deps, state_value, "`state`", &features.images)?;
    let model = model(deps, model_name)?;
    let request = EvaluationRequest::new(state, model, vec![(id, question)])
        .map_err(|error| ToolFailure::usage(error.to_string()))?;
    let request = features.apply(request)?;
    execute(deps, clock, &request)
}

fn noul(deps: &Deps, clock: &InterruptibleClock, args: NoulArgs) -> Outcome {
    let mode = crate::request::ContentMode::for_provider(deps.context.endpoint.value.provider());
    let instructions = single_instructions(deps, args.instructions, args.id.as_deref())?;
    let criteria = noul_criteria(args.criteria, "`criteria`", mode)?;
    single(
        deps,
        clock,
        args.state,
        args.id,
        Question::noul(instructions, criteria),
        args.model,
        args.features,
    )
}

fn choice(deps: &Deps, clock: &InterruptibleClock, args: ChoiceArgs) -> Outcome {
    let mode = crate::request::ContentMode::for_provider(deps.context.endpoint.value.provider());
    let instructions = single_instructions(deps, args.instructions, args.id.as_deref())?;
    let options = options(args.options, "`options`", mode)?;
    let mut features = args.features;
    if let Some(reject_if_busy) = args.reject_if_busy {
        features.options = Some(crate::media::CapacityOptions { reject_if_busy });
    }
    single(
        deps,
        clock,
        args.state,
        args.id,
        Question::choice(instructions, options),
        args.model,
        features,
    )
}

fn score(deps: &Deps, clock: &InterruptibleClock, args: ScoreArgs) -> Outcome {
    let mode = crate::request::ContentMode::for_provider(deps.context.endpoint.value.provider());
    let instructions = single_instructions(deps, args.instructions, args.id.as_deref())?;
    let levels = level_content(mode, args.levels, "`levels`")?;
    single(
        deps,
        clock,
        args.state,
        args.id,
        Question::score_with_max(
            instructions,
            levels,
            crate::media::score_max(&deps.context.endpoint.value),
        ),
        args.model,
        args.features,
    )
}

fn ask(deps: &Deps, clock: &InterruptibleClock, args: AskArgs) -> Outcome {
    let questions = questions(args.questions, &deps.context.endpoint.value)?;
    let features = args.features.with_mcp_defaults(&deps.context)?;
    let state = state(deps, args.state, "`state`", &features.images)?;
    let model = model(deps, args.model)?;
    let request = EvaluationRequest::new(state, model, questions)
        .map_err(|error| ToolFailure::usage(error.to_string()))?;
    let request = features.apply(request)?;
    execute(deps, clock, &request)
}

/// Sends one request and renders it as `jev.evaluation/v1`, exactly as the CLI does.
fn execute(deps: &Deps, clock: &InterruptibleClock, request: &EvaluationRequest) -> Outcome {
    crate::media::preflight(&deps.context.endpoint.value, request)?;
    let credential = credential(deps)?;
    let (result, stats) = evaluate::send(
        &deps.context,
        clock,
        request,
        &credential,
        Some(deps.transport.as_ref()),
    );
    if clock.stop_requested() {
        return Err(CliError::Interrupted.into());
    }
    let response = result?;
    let mut document = render_json::evaluation(
        &response,
        request.model().as_str(),
        &deps.context.endpoint.value.to_string(),
        None,
        stats.request_id.as_deref(),
        &response.missing(request.questions().iter().map(|(id, _)| id)),
    );
    evaluate::add_provider_metadata(&mut document, &deps.context);
    Ok(document)
}

fn map_tool(deps: &Deps, clock: &InterruptibleClock, args: MapArgs) -> Outcome {
    let concurrency = args.concurrency.unwrap_or(DEFAULT_MAP_CONCURRENCY);
    // Checked here rather than through `batch::check_concurrency`, whose message states
    // the CLI's wider range: a host told "1 to 64" would retry with a value this rejects.
    if concurrency == 0 || concurrency > MAX_MAP_CONCURRENCY {
        return Err(ToolFailure::usage(format!(
            "`concurrency` must be between 1 and {MAX_MAP_CONCURRENCY} over MCP; \
             `jev map --concurrency` allows more"
        )));
    }

    if args.records.is_empty() {
        return Err(ToolFailure::usage(
            "`records` is empty; there is nothing to evaluate",
        ));
    }
    if args.records.len() > MAX_MAP_RECORDS {
        return Err(too_big(&format!(
            "{} records is over the {MAX_MAP_RECORDS} record limit for one MCP call",
            args.records.len()
        )));
    }
    let questions = questions(args.questions, &deps.context.endpoint.value)?;
    let model = model(deps, args.model)?;

    // Each row echoes its record's id, so the ids count against the budget too: a tiny
    // state with a megabyte id would otherwise pass the estimate and flood the result.
    let id_bytes: usize = args
        .records
        .iter()
        .filter_map(|record| record.id.as_ref())
        .map(encoded_len)
        .sum();
    let estimate = args
        .records
        .len()
        .saturating_mul(estimated_row_bytes(&questions))
        .saturating_add(id_bytes);
    if estimate > MAX_MAP_OUTPUT_BYTES {
        return Err(too_big(&format!(
            "the result would be about {estimate} bytes, over the {MAX_MAP_OUTPUT_BYTES} \
             byte limit for one MCP result"
        )));
    }

    let template = args.features.with_mcp_defaults(&deps.context)?;
    let records = records(deps, args.records, &template)?;
    let records = records
        .into_iter()
        .map(|record| {
            let features = template.merge(record.features.clone())?;
            let mut record = record;
            record.features = features;
            crate::media::preflight(
                &deps.context.endpoint.value,
                &record.request(&questions, &model)?,
            )?;
            Ok(record)
        })
        .collect::<crate::errors::Result<Vec<_>>>()?;

    let credential = credential(deps)?;
    let fingerprint =
        map::provider_fingerprint(&questions, &model, &template, &deps.context.endpoint.value);
    let (outcomes, stopped_early, write_error) = map::evaluate_all(
        &deps.context,
        clock,
        &records,
        &questions,
        &model,
        &credential,
        concurrency,
        false,
        Some(deps.transport.as_ref()),
        &map::Sink::in_memory(),
        None,
        &fingerprint,
    );
    if clock.stop_requested() {
        return Err(CliError::Interrupted.into());
    }
    // Unreachable with an in-memory sink, which writes nothing; reported rather than
    // ignored so that a future sink cannot fail silently here.
    if let Some(reason) = write_error {
        return Err(CliError::internal(reason).into());
    }
    // A rejected credential is not a property of a record: the batch stops at the first
    // one (as the CLI's does), and when nothing was answered the honest result is the
    // CLI's exit-3 condition, as a typed `auth` failure, rather than a table of rows that
    // all say the same thing. If some records were answered first, the rows are kept:
    // they were billed, and each failed row still names `auth`.
    if let Some(failure) = rejected_credential(&outcomes) {
        return Err(failure);
    }
    let summary = map::summary(
        &outcomes,
        &map::Totals {
            resumed: 0,
            pending: records.len(),
            stopped_early,
        },
        None,
        false,
    );
    let rows: Vec<Value> = outcomes
        .into_iter()
        .map(|outcome| outcome.document)
        .collect();
    Ok(json!({
        "schema": MCP_MAP_SCHEMA,
        "rows": rows,
        "summary": summary,
    }))
}

/// Converts inline records within the MCP aggregate state and media byte ceiling.
fn records(
    deps: &Deps,
    list: Vec<RecordArgs>,
    template: &crate::media::Features,
) -> Result<Vec<map::Record>, ToolFailure> {
    // MCP counts serialized states plus compressed media bytes after base64 decoding,
    // including template media once even when a row replaces it. CLI map's JSONL mode caps
    // serialized JSONL input, whose media remains base64, and excludes --image files.
    // Counting decoded file bytes avoids rebuilding base64 just to check this cap.
    let media_bytes = |images: &[jev_core::EmbeddedImage], videos: &[jev_core::EmbeddedVideo]| {
        images
            .iter()
            .map(jev_core::EmbeddedImage::byte_len)
            .chain(videos.iter().map(jev_core::EmbeddedVideo::byte_len))
            .fold(0usize, usize::saturating_add)
    };
    let mut total = media_bytes(&template.images, &template.videos);
    let mut records = Vec::with_capacity(list.len());
    for (index, record) in list.into_iter().enumerate() {
        total = total
            .saturating_add(check_size(deps, &record.state, &format!("record {index}"))?)
            .saturating_add(media_bytes(&record.images, &record.videos));
        if u64::try_from(total).unwrap_or(u64::MAX) > deps.context.max_input_bytes.value {
            return Err(too_big(&format!(
                "the records total more than the {} byte input limit",
                deps.context.max_input_bytes.value
            )));
        }
        let id = match record.id {
            None | Some(Value::Null) => index.to_string(),
            // The CLI's `--id-field` rendering, so a record has the same id either way.
            Some(other @ (Value::String(_) | Value::Number(_))) => map::render_id(&other),
            // The schema advertises a string or a number; an object or array id would
            // also be echoed into every row at whatever size it was sent.
            Some(_) => {
                return Err(ToolFailure::usage(format!(
                    "record {index}: `id` must be a string or a number"
                )));
            }
        };
        let images = if record.images.is_empty() {
            &template.images
        } else {
            &record.images
        };
        let state = state(deps, record.state, &format!("record {index}"), images)?;
        records.push(
            map::Record::new(index, id, state).with_features(crate::media::Features {
                images: record.images,
                videos: record.videos,
                ..crate::media::Features::default()
            }),
        );
    }

    Ok(records)
}

/// The batch's `auth` failure, when the credential was rejected and nothing was answered.
fn rejected_credential(outcomes: &[map::Outcome]) -> Option<ToolFailure> {
    if !outcomes.iter().any(map::Outcome::auth_failed)
        || outcomes.iter().any(map::Outcome::succeeded)
    {
        return None;
    }
    let message = outcomes
        .iter()
        .filter(|outcome| outcome.auth_failed())
        .find_map(|outcome| {
            outcome
                .document
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
        })
        .unwrap_or("the API rejected the credential")
        .to_owned();
    Some(ToolFailure {
        kind: "auth",
        message,
    })
}

fn too_big(reason: &str) -> ToolFailure {
    ToolFailure::usage(format!(
        "{reason}. Nothing was sent. Split the batch into smaller calls, or run \
         `jev map` from a shell for large or offline jobs: it streams JSONL and its \
         output does not enter the agent's context."
    ))
}

/// A deliberately generous estimate of one row's encoded size.
///
/// A row is fixed overhead plus one answer per question. A Choice answer carries every
/// option name with a probability, and a Score answer echoes every level description in
/// its `legend`, so those scale with the question rather than the record.
fn estimated_row_bytes(questions: &[(QuestionId, Question)]) -> usize {
    const ROW_OVERHEAD: usize = 320;
    const ANSWER_OVERHEAD: usize = 96;
    const PER_ENTRY: usize = 48;
    questions
        .iter()
        .fold(ROW_OVERHEAD, |total, (id, question)| {
            let body = match question {
                Question::Noul { .. } => 0,
                // Every name once in the distribution, plus the chosen one again.
                Question::Choice { options, .. } => {
                    let longest = options.iter().map(|option| option.name().len()).max();
                    options
                        .iter()
                        .map(|option| option.name().len() + PER_ENTRY)
                        .sum::<usize>()
                        + longest.unwrap_or(0)
                }
                Question::Score { levels, .. } => levels
                    .iter()
                    .map(|level| level.to_string().len() + 2 * PER_ENTRY)
                    .sum(),
            };
            total + ANSWER_OVERHEAD + id.as_str().len() + body
        })
}
