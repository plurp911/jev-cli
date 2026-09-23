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
            CliError::Usage(_) => "usage",
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
    serde_json::from_value(arguments)
        .map_err(|error| ToolFailure::usage(format!("invalid arguments: {error}")))
}

// --- Argument shapes. Mirrored by the input schemas in `mcp/schema/`. ------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoulArgs {
    state: Value,
    instructions: String,
    #[serde(default)]
    criteria: Option<NoulCriteriaArgs>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoulCriteriaArgs {
    #[serde(default, rename = "true")]
    yes: Option<String>,
    #[serde(default, rename = "false")]
    no: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChoiceArgs {
    state: Value,
    instructions: String,
    options: Vec<OptionArgs>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionArgs {
    name: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScoreArgs {
    state: Value,
    instructions: String,
    levels: Vec<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AskArgs {
    state: Value,
    questions: Vec<QuestionArgs>,
    #[serde(default)]
    model: Option<String>,
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
    instructions: String,
    #[serde(default)]
    criteria: Option<NoulCriteriaArgs>,
    #[serde(default)]
    options: Option<Vec<OptionArgs>>,
    #[serde(default)]
    levels: Option<Vec<String>>,
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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordArgs {
    #[serde(default)]
    id: Option<Value>,
    state: Value,
}

// --- Conversion into the core model. --------------------------------------------------

fn text(value: String, what: &str) -> Result<Content, ToolFailure> {
    Content::text(value).map_err(|_| ToolFailure::usage(format!("{what} must not be empty")))
}

fn noul_criteria(
    criteria: Option<NoulCriteriaArgs>,
    what: &str,
) -> Result<Option<NoulCriteria>, ToolFailure> {
    let Some(criteria) = criteria else {
        return Ok(None);
    };
    let side = |value: Option<String>, name: &str| {
        value
            .map(|text| self::text(text, &format!("{what}.criteria.{name}")))
            .transpose()
    };
    NoulCriteria::new(side(criteria.yes, "true")?, side(criteria.no, "false")?)
        .map(Some)
        .map_err(|error| ToolFailure::usage(format!("{what}: {error}")))
}

fn options(options: Vec<OptionArgs>, what: &str) -> Result<Vec<ChoiceOption>, ToolFailure> {
    // Duplicate names are refused by `Question::choice`, as they are for the CLI.
    options
        .into_iter()
        .map(|option| {
            let description = option
                .description
                .map(|text| self::text(text, &format!("{what}: an option description")))
                .transpose()?;
            ChoiceOption::new(option.name, description)
                .map_err(|error| ToolFailure::usage(format!("{what}: {error}")))
        })
        .collect()
}

fn levels(levels: Vec<String>, what: &str) -> Result<Vec<Content>, ToolFailure> {
    levels
        .into_iter()
        .map(|level| text(level, &format!("{what}: a level")))
        .collect()
}

fn question(args: QuestionArgs) -> Result<(QuestionId, Question), ToolFailure> {
    let what = format!("question {:?}", crate::output::sanitize(&args.id));
    let id = QuestionId::new(args.id)
        .map_err(|error| ToolFailure::usage(format!("{what}: `id`: {error}")))?;
    let instructions = text(args.instructions, &format!("{what}: `instructions`"))?;
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
            Question::noul(instructions, noul_criteria(args.criteria, &what)?)
        }
        Kind::Choice => {
            stray(args.criteria.is_some(), "criteria")?;
            stray(args.levels.is_some(), "levels")?;
            let list = args.options.ok_or_else(|| {
                ToolFailure::usage(format!("{what}: a choice question needs `options`"))
            })?;
            Question::choice(instructions, options(list, &what)?)
        }
        Kind::Score => {
            stray(args.criteria.is_some(), "criteria")?;
            stray(args.options.is_some(), "options")?;
            let list = args.levels.ok_or_else(|| {
                ToolFailure::usage(format!("{what}: a score question needs `levels`"))
            })?;
            Question::score(instructions, levels(list, &what)?)
        }
    }
    .map_err(|error| ToolFailure::usage(format!("{what}: {error}")))?;
    Ok((id, question))
}

/// Converts a question list. Duplicate ids are refused by
/// [`EvaluationRequest::new`], exactly as for a CLI request file.
fn questions(list: Vec<QuestionArgs>) -> Result<Vec<(QuestionId, Question)>, ToolFailure> {
    list.into_iter().map(question).collect()
}

/// Converts a state argument, applying the CLI's per-source byte ceiling.
fn state(deps: &Deps, value: Value, what: &str) -> Result<State, ToolFailure> {
    check_size(deps, &value, what)?;
    Content::try_from(value)
        .map(State::new)
        .map_err(|error| ToolFailure::usage(format!("{what}: {error}")))
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
) -> Outcome {
    let question = question.map_err(|error| ToolFailure::usage(error.to_string()))?;
    let id = QuestionId::new(id.unwrap_or_else(|| DEFAULT_ID.to_owned()))
        .map_err(|error| ToolFailure::usage(format!("`id`: {error}")))?;
    let state = state(deps, state_value, "`state`")?;
    let model = model(deps, model_name)?;
    let request = EvaluationRequest::new(state, model, vec![(id, question)])
        .map_err(|error| ToolFailure::usage(error.to_string()))?;
    execute(deps, clock, &request)
}

fn noul(deps: &Deps, clock: &InterruptibleClock, args: NoulArgs) -> Outcome {
    let instructions = text(args.instructions, "`instructions`")?;
    let criteria = noul_criteria(args.criteria, "`criteria`")?;
    single(
        deps,
        clock,
        args.state,
        args.id,
        Question::noul(instructions, criteria),
        args.model,
    )
}

fn choice(deps: &Deps, clock: &InterruptibleClock, args: ChoiceArgs) -> Outcome {
    let instructions = text(args.instructions, "`instructions`")?;
    let options = options(args.options, "`options`")?;
    single(
        deps,
        clock,
        args.state,
        args.id,
        Question::choice(instructions, options),
        args.model,
    )
}

fn score(deps: &Deps, clock: &InterruptibleClock, args: ScoreArgs) -> Outcome {
    let instructions = text(args.instructions, "`instructions`")?;
    let levels = levels(args.levels, "`levels`")?;
    single(
        deps,
        clock,
        args.state,
        args.id,
        Question::score(instructions, levels),
        args.model,
    )
}

fn ask(deps: &Deps, clock: &InterruptibleClock, args: AskArgs) -> Outcome {
    let questions = questions(args.questions)?;
    let state = state(deps, args.state, "`state`")?;
    let model = model(deps, args.model)?;
    let request = EvaluationRequest::new(state, model, questions)
        .map_err(|error| ToolFailure::usage(error.to_string()))?;
    execute(deps, clock, &request)
}

/// Sends one request and renders it as `jev.evaluation/v1`, exactly as the CLI does.
fn execute(deps: &Deps, clock: &InterruptibleClock, request: &EvaluationRequest) -> Outcome {
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
    Ok(render_json::evaluation(
        &response,
        request.model().as_str(),
        &deps.context.endpoint.value.to_string(),
        None,
        stats.request_id.as_deref(),
        &response.missing(request.questions().iter().map(|(id, _)| id)),
    ))
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
    let questions = questions(args.questions)?;
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

    let records = records(deps, args.records)?;

    // Every record gets the same question set, so validate it once, against the first
    // record, before anything is sent. Left to `evaluate_all`, a duplicate id would
    // come back as one failed row per record.
    if let Some(first) = records.first() {
        EvaluationRequest::new(first.state.clone(), model.clone(), questions.clone())
            .map_err(|error| ToolFailure::usage(error.to_string()))?;
    }

    let credential = credential(deps)?;
    let fingerprint = map::request_fingerprint(&questions, &model);
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

/// Converts inline records, applying the CLI's whole-input byte ceiling to all of them.
fn records(deps: &Deps, list: Vec<RecordArgs>) -> Result<Vec<map::Record>, ToolFailure> {
    // The CLI applies `--max-input-bytes` to a whole `jev map` input; so does this.
    let mut total = 0usize;
    let mut records = Vec::with_capacity(list.len());
    for (index, record) in list.into_iter().enumerate() {
        total = total.saturating_add(check_size(deps, &record.state, &format!("record {index}"))?);
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
        let state = Content::try_from(record.state)
            .map(State::new)
            .map_err(|error| ToolFailure::usage(format!("record {index}: {error}")))?;
        records.push(map::Record::new(index, id, state));
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
