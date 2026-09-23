//! Argument parsing.
//!
//! This module defines the command surface and nothing else: no I/O, no logic, no
//! network. Dispatch lives in [`crate::commands`].
//!
//! # What is stable here
//!
//! Command names, flag names, short forms, and their meanings are a compatibility
//! promise (`docs/cli-contract.md`). Help *wording* is not. Adding a flag or a
//! subcommand is compatible; changing a default is not.
//!
//! # What is deliberately absent
//!
//! There is no `--api-key`. Arguments are visible in `ps`, in shell history, and in CI
//! logs, so a credential may never be one (`docs/threat-model.md` T1). `jev auth login`
//! prompts without echo, or reads from stdin for scripted provisioning.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::context::{Overrides, Verbosity};

/// The `jev` command line.
#[derive(Debug, Parser)]
#[command(
    name = "jev",
    version,
    about,
    long_about = "jev is an independent, community-maintained CLI for TypeSafe's System One \
                  API and its Jev model. It is not affiliated with, endorsed by, or supported \
                  by TypeSafe AI.\n\n\
                  State you supply is transmitted to the configured API endpoint when a \
                  request runs. Use --dry-run to see exactly what would be sent.\n\n\
                  Text output is for humans and is not a stable interface. Scripts should use \
                  `--output json`, whose documents carry a versioned `schema` field.",
    propagate_version = true,
    disable_help_subcommand = true,
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(flatten)]
    pub(crate) global: GlobalArgs,

    #[command(subcommand)]
    pub(crate) command: Command,
}

/// Flags accepted by every subcommand.
///
/// The doc comment on each field *is* its `--help` text, which is why two lints are
/// relaxed here and nowhere else: backticks added to satisfy `doc_markdown` would
/// appear literally in the help a user reads, and the boolean fields are independent
/// switches that `clap` requires as separate fields.
#[derive(Debug, Args, Clone)]
#[allow(
    clippy::doc_markdown,
    clippy::struct_excessive_bools,
    reason = "these doc comments are rendered verbatim as --help text by clap"
)]
pub struct GlobalArgs {
    /// Output format. `json` is the stable, versioned machine contract [default: text]
    #[arg(long, short = 'o', value_enum, global = true, help_heading = GLOBAL_HEADING)]
    pub(crate) output: Option<OutputFormat>,

    /// When to colour human output; NO_COLOR overrides `auto` [default: auto]
    #[arg(long, value_enum, global = true, help_heading = GLOBAL_HEADING)]
    pub(crate) color: Option<ColorArg>,

    /// Model identifier or alias; pin a version when a threshold is calibrated
    /// [default: jev-latest]
    #[arg(long, short = 'm', global = true, value_name = "MODEL", help_heading = GLOBAL_HEADING)]
    pub(crate) model: Option<String>,

    /// API base URL; a non-official one uses JEV_CUSTOM_API_KEY, never a stored
    /// TypeSafe key [default: the official TypeSafe API]
    #[arg(long, global = true, value_name = "URL", help_heading = GLOBAL_HEADING)]
    pub(crate) endpoint: Option<String>,

    /// Per-attempt HTTP timeout, in seconds [default: 10, maximum: 3600]
    #[arg(long, global = true, value_name = "SECONDS", help_heading = GLOBAL_HEADING)]
    pub(crate) timeout: Option<u64>,

    /// Retries after the first attempt; permanent failures are never retried
    /// [default: 2, maximum: 10]
    #[arg(long, global = true, value_name = "N", help_heading = GLOBAL_HEADING)]
    pub(crate) retries: Option<u32>,

    /// Ceiling on bytes read from one input source; input is never silently truncated
    /// [default: 1048576]
    #[arg(long, global = true, value_name = "BYTES", help_heading = GLOBAL_HEADING)]
    pub(crate) max_input_bytes: Option<u64>,

    /// Ignore the configuration file entirely
    #[arg(long, global = true, help_heading = GLOBAL_HEADING)]
    pub(crate) no_config: bool,

    /// Print the request that would be sent, and send nothing
    #[arg(long, global = true, help_heading = GLOBAL_HEADING)]
    pub(crate) dry_run: bool,

    /// Suppress non-error diagnostics on stderr
    #[arg(long, short = 'q', global = true, conflicts_with = "verbose", help_heading = GLOBAL_HEADING)]
    pub(crate) quiet: bool,

    /// Print extra diagnostics on stderr; never prints credentials or request bodies
    #[arg(long, short = 'v', global = true, help_heading = GLOBAL_HEADING)]
    pub(crate) verbose: bool,
}

/// Heading the global flags are listed under.
///
/// Without it `clap` interleaves them with each command's own options — `--true` and
/// `--false` split by `--color`, the four state flags scattered among timeouts — which
/// reads as though the help itself is broken.
const GLOBAL_HEADING: &str = "Global options";

/// Heading the state flags are listed under.
const STATE_HEADING: &str = "State (choose one; with none, state is read from stdin)";

/// The `--require` grammar, appended to the long help of every command that accepts it.
///
/// It lives here rather than only in `docs/commands.md` because a user who installed a
/// binary and no documentation still has to be able to find it.
const REQUIRE_HELP: &str = "\
--require turns a model judgment into an exit status:

  expr       := or
  or         := and ('or' and)*
  and        := unary ('and' unary)*
  unary      := 'not' unary | '(' expr ')' | comparison
  comparison := path operator literal
  path       := ident ('.' ident)*
  operator   := '>' | '>=' | '<' | '<=' | '==' | '!='
  literal    := number | 'quoted text' | bare-word

Paths:
  <id>.noul                        a Noul's yes-probability
  <id>.choice                      a Choice's selected option (text; use == or !=)
  <id>.score                       a Score's value
  <id>.confidence                  a Choice or Score confidence -- NOT on a Noul
  <id>.probabilities.<option>      one entry of a distribution

  jev noul \"is this a security issue?\" --state-file r.md --require 'answer.noul > 0.9'

A gate that holds exits 0, one that does not exits 1, and one that cannot be
evaluated exits 6 -- never 0. Nothing is ever passed to a shell.";

impl GlobalArgs {
    /// Converts to the resolution input.
    #[must_use]
    pub fn overrides(&self) -> Overrides {
        Overrides {
            endpoint: self.endpoint.clone(),
            model: self.model.clone(),
            output: self.output,
            color: self.color,
            timeout_seconds: self.timeout,
            retries: self.retries,
            max_input_bytes: self.max_input_bytes,
            no_config: self.no_config,
            verbosity: Verbosity {
                quiet: self.quiet,
                verbose: self.verbose,
            },
            dry_run: self.dry_run,
        }
    }
}

/// Output formats. Adding a variant is compatible; removing one is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    /// Human-readable text. **Not a stable interface; do not parse it.**
    Text,
    /// One JSON document, one line, with a versioned `schema` field.
    Json,
}

/// Colour policy.
///
/// Variant doc comments become `--help` text, so backticks are suppressed here for the
/// same reason as on [`GlobalArgs`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[allow(
    clippy::doc_markdown,
    reason = "these doc comments are rendered verbatim as --help text by clap"
)]
pub enum ColorArg {
    /// Colour when stdout is a terminal and NO_COLOR is unset.
    Auto,
    /// Always colour.
    Always,
    /// Never colour.
    Never,
}

#[derive(Debug, Subcommand)]
/// The subcommands. Names are a compatibility promise; see `docs/cli-contract.md`.
pub enum Command {
    /// Ask one yes/no question and get the probability of "yes".
    Noul(NoulArgs),
    /// Ask one question that selects from named options.
    Choice(ChoiceArgs),
    /// Ask one question that rates against ordered levels.
    Score(ScoreArgs),
    /// Ask several independent questions about one state, in a single request.
    Ask(AskArgs),
    /// Evaluate one question set over many records.
    Map(MapArgs),
    /// Measure a question against your own labelled examples, and choose a threshold.
    Eval(EvalArgs),
    /// List the models this account may use.
    Models,
    /// Report what `jev` sees: configuration, credentials, endpoint, and model.
    Doctor(DoctorArgs),
    /// Manage the stored credential.
    #[command(subcommand)]
    Auth(AuthCommand),
    /// Read and write non-secret configuration.
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Print a shell completion script.
    Completions(CompletionsArgs),
    /// Serve the Jev tools to an AI agent over the Model Context Protocol.
    #[command(subcommand)]
    Mcp(McpCommand),
}

/// Where the state comes from.
///
/// The four flags conflict with each other, so "two sources at once" is a usage error
/// rather than a precedence rule the user has to learn. With none of them, state is
/// read from standard input.
#[derive(Debug, Args, Clone, Default)]
pub struct StateArgs {
    /// Literal text state.
    #[arg(long, value_name = "TEXT", help_heading = STATE_HEADING, conflicts_with_all = ["state_file", "state_json", "state_json_file"])]
    pub(crate) state: Option<String>,

    /// Read text state from a file. `-` means standard input.
    #[arg(long, value_name = "PATH", help_heading = STATE_HEADING, conflicts_with_all = ["state", "state_json", "state_json_file"])]
    pub(crate) state_file: Option<PathBuf>,

    /// Literal JSON state: an object, an array, or a string.
    #[arg(long, value_name = "JSON", help_heading = STATE_HEADING, conflicts_with_all = ["state", "state_file", "state_json_file"])]
    pub(crate) state_json: Option<String>,

    /// Read JSON state from a file. `-` means standard input.
    #[arg(long, value_name = "PATH", help_heading = STATE_HEADING, conflicts_with_all = ["state", "state_file", "state_json"])]
    pub(crate) state_json_file: Option<PathBuf>,
}

/// Flags shared by the single-question commands.
#[derive(Debug, Args, Clone, Default)]
pub struct AnswerArgs {
    /// Print only the scalar answer: the probability, the option name, or the score.
    #[arg(long, conflicts_with = "output")]
    pub(crate) value: bool,

    /// Exit non-zero unless this expression holds; the grammar is in this command's
    /// long help
    #[arg(long, value_name = "EXPR")]
    pub(crate) require: Option<String>,
}

/// `jev noul`
#[derive(Debug, Args)]
#[command(
    long_about = "Ask one yes/no question. The answer is `noul`, the probability that the \
                  answer is yes, from 0 to 1.\n\n\
                  A Noul carries no confidence value: the API does not return one, and `jev` \
                  does not invent one. A value near 0.5 means yes and no are similarly likely \
                  -- it does not mean \"medium\".\n\n\
                  For one of several options use `jev choice`; for a position on a \
                  described scale use `jev score`.",
    after_long_help = REQUIRE_HELP
)]
pub struct NoulArgs {
    /// The yes/no question or statement to evaluate.
    #[arg(value_name = "INSTRUCTIONS")]
    pub(crate) instructions: String,

    /// What a yes means, when the boundary is subtle.
    #[arg(long = "true", value_name = "TEXT")]
    pub(crate) yes: Option<String>,

    /// What a no means, when the boundary is subtle.
    #[arg(long = "false", value_name = "TEXT")]
    pub(crate) no: Option<String>,

    /// Question id, used as the key in JSON output and in --require.
    #[arg(long, default_value = "answer", value_name = "ID")]
    pub(crate) id: String,

    #[command(flatten)]
    pub(crate) state: StateArgs,

    #[command(flatten)]
    pub(crate) answer: AnswerArgs,
}

/// `jev choice`
#[derive(Debug, Args)]
#[command(
    long_about = "Ask one question that selects from a set of named options. The answer is the \
                  selected option, the probability of every option, and a confidence derived \
                  from how concentrated that distribution is.\n\n\
                  Give every option, not a shortlist, and add an `other` option when the list \
                  may not cover every input. The API accepts up to 255 options.",
    after_long_help = REQUIRE_HELP
)]
pub struct ChoiceArgs {
    /// What the model should decide.
    #[arg(value_name = "INSTRUCTIONS")]
    pub(crate) instructions: String,

    /// An option, as `name` or `name=description`. Repeat for each option.
    #[arg(long = "option", short = 'O', value_name = "NAME[=DESCRIPTION]")]
    pub(crate) options: Vec<String>,

    /// Read options from a JSON object mapping name to description or null.
    #[arg(long, value_name = "PATH", conflicts_with = "options")]
    pub(crate) options_file: Option<PathBuf>,

    /// Question id, used as the key in JSON output and in --require.
    #[arg(long, default_value = "answer", value_name = "ID")]
    pub(crate) id: String,

    #[command(flatten)]
    pub(crate) state: StateArgs,

    #[command(flatten)]
    pub(crate) answer: AnswerArgs,
}

/// `jev score`
#[derive(Debug, Args)]
#[command(
    long_about = "Ask one question that rates the state against ordered levels. The answer is \
                  the probability-weighted position on those levels, which can fall between \
                  them, plus the full distribution and a confidence.\n\n\
                  Levels are numbered from 0 in the order given. The API accepts 2 to 10 \
                  levels, and each must describe a concrete situation that stands on its own.",
    after_long_help = REQUIRE_HELP
)]
pub struct ScoreArgs {
    /// What the model should rate.
    #[arg(value_name = "INSTRUCTIONS")]
    pub(crate) instructions: String,

    /// A level description, lowest first. Repeat for each level.
    #[arg(long = "level", short = 'L', value_name = "TEXT")]
    pub(crate) levels: Vec<String>,

    /// Read levels from a JSON array of descriptions, lowest first.
    #[arg(long, value_name = "PATH", conflicts_with = "levels")]
    pub(crate) levels_file: Option<PathBuf>,

    /// Question id, used as the key in JSON output and in --require.
    #[arg(long, default_value = "answer", value_name = "ID")]
    pub(crate) id: String,

    #[command(flatten)]
    pub(crate) state: StateArgs,

    #[command(flatten)]
    pub(crate) answer: AnswerArgs,
}

/// `jev ask`
#[derive(Debug, Args)]
#[command(
    long_about = "Ask several independent questions about one state in a single request.\n\n\
                  This is how System One is meant to be used: questions are evaluated in \
                  parallel against one reading of the state, so asking ten costs far less \
                  than ten requests and answers in roughly the same time. Include \
                  speculative questions and let your code read only the relevant answers.\n\n\
                  The request document is the official API request body:\n  \
                    {\"state\": ..., \"model\": \"jev-latest\", \"questions\": {...}}\n\
                  `model` is optional and --model overrides it. With --questions, only the \
                  questions map is read and the state comes from the --state flags.",
    after_long_help = REQUIRE_HELP
)]
pub struct AskArgs {
    /// A full request document. `-` or omitted means standard input.
    #[arg(long, short = 'r', value_name = "PATH", conflicts_with = "questions")]
    pub(crate) request: Option<PathBuf>,

    /// A questions map on its own, with state supplied separately.
    #[arg(long, value_name = "PATH")]
    pub(crate) questions: Option<PathBuf>,

    #[command(flatten)]
    pub(crate) state: StateArgs,

    #[command(flatten)]
    pub(crate) answer: AnswerArgs,
}

/// `jev map`
#[derive(Debug, Args)]
#[command(long_about = "Evaluate one question set over many records.\n\n\
                  Each input record becomes the `state` of one request; the questions are the \
                  same for every record. Output is JSONL in input order by default, one line \
                  per record, each carrying the record's index and id so results can be joined \
                  back to their inputs.\n\n\
                  Rows fail independently. A run with some failures exits 5, and the \
                  successful rows are still written. Nothing is cached: --resume reads the \
                  output file to see what is already done, so a resumed run never replays a \
                  stale model judgment.\n\n\
                  --require classifies each answered row with the same expression language \
                  `jev noul --require` gates on, and --review-file diverts the rows it did \
                  not pass so they can be looked at separately. In `map` the expression \
                  routes rather than gates: it never changes the exit code, which keeps \
                  reporting whether the API answered. No row is ever discarded.",
    after_long_help = REQUIRE_HELP)]
pub struct MapArgs {
    /// A questions map, or a full request document whose `state` is ignored.
    #[arg(long, short = 'r', value_name = "PATH")]
    pub(crate) request: PathBuf,

    /// Input records. `-` or omitted means standard input.
    #[arg(long, short = 'i', value_name = "PATH")]
    pub(crate) input: Option<PathBuf>,

    /// Treat each input line as plain text rather than as a JSON value.
    #[arg(long)]
    pub(crate) lines: bool,

    /// Use this field of each JSON record as the state, instead of the whole record.
    #[arg(long, value_name = "FIELD", conflicts_with = "lines")]
    pub(crate) state_field: Option<String>,

    /// Use this field of each JSON record as the row id. Defaults to the input index.
    #[arg(long, value_name = "FIELD", conflicts_with = "lines")]
    pub(crate) id_field: Option<String>,

    /// Write results here instead of to standard output. Required by --resume.
    #[arg(long, value_name = "PATH")]
    pub(crate) output_file: Option<PathBuf>,

    /// Classify each row against this expression. It routes; it never changes the exit code.
    #[arg(long, value_name = "EXPR")]
    pub(crate) require: Option<String>,

    /// Divert rows whose --require did not pass here, for review, instead of the main stream.
    #[arg(long, value_name = "PATH", requires = "require")]
    pub(crate) review_file: Option<PathBuf>,

    /// Skip records already present in the output file, and append to it.
    #[arg(long, requires = "output_file")]
    pub(crate) resume: bool,

    /// Requests in flight at once; at most 64
    #[arg(long, short = 'j', default_value_t = 4, value_name = "N")]
    pub(crate) concurrency: usize,

    /// Stop at the first failing record instead of continuing.
    #[arg(long)]
    pub(crate) fail_fast: bool,
}

/// What a `jev eval` threshold is chosen to achieve.
///
/// There is no default, deliberately. A threshold encodes what being wrong costs you,
/// and `docs/cli-contract.md` promises `jev` will never pick that for anyone. Without
/// an objective, `jev eval` reports how the question performed and selects nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[allow(
    clippy::doc_markdown,
    reason = "these doc comments are rendered verbatim as --help text by clap"
)]
pub enum Objective {
    /// Highest F1. Noul questions only; takes no --target.
    MaximizeF1,
    /// Highest recall among cuts whose precision is at least --target. Noul only.
    MinPrecision,
    /// Highest precision among cuts whose recall is at least --target. Noul only.
    MinRecall,
    /// Widest coverage among cuts whose covered rows are at least --target accurate.
    /// Choice and score only.
    MinAccuracy,
    /// The cut whose coverage is closest to --target. Choice and score only.
    TargetCoverage,
}

/// `jev eval`
#[derive(Debug, Args)]
#[command(
    long_about = "Measure a question against labelled examples you already have, and \
                  choose a threshold you can defend.\n\n\
                  This is not a benchmark of Jev. It answers one question: given examples \
                  you have already judged, how well does THIS question, answered by THIS \
                  model version, perform on YOUR data -- and where should the cut go? \
                  Nothing is trained, and a label is never sent: ground truth is compared \
                  locally, after the answer comes back.\n\n\
                  The dataset is JSONL, one labelled example per line:\n  \
                    {\"schema\": \"jev.eval.row/v1\", \"id\": \"1\", \"state\": \"...\", \
                     \"labels\": {\"<question id>\": <ground truth>}}\n\
                  A noul label is true/false (or 1/0), a choice label is one of that \
                  question's option names, and a score label is a level index counting from \
                  0. A row may label a subset of the questions.\n\n\
                  With --objective, a threshold is chosen on a held-out-from split and \
                  reported on the rest, so the number beside it is not the number it was \
                  picked to maximize. Without one, every row is scored and no threshold is \
                  selected.\n\n\
                  A result describes one question, one dataset, and one model version. It \
                  does not transfer to another of the three -- pin --model once a threshold \
                  is in use."
)]
pub struct EvalArgs {
    /// A questions map, or a full request document whose `state` is ignored.
    #[arg(long, short = 'r', value_name = "PATH")]
    pub(crate) request: PathBuf,

    /// Labelled examples, as JSONL. `-` means standard input.
    #[arg(long, short = 'd', value_name = "PATH", conflicts_with_all = ["calibration", "test"])]
    pub(crate) dataset: Option<PathBuf>,

    /// Choose the threshold from these rows instead of from a split of --dataset.
    #[arg(long, value_name = "PATH", requires = "test")]
    pub(crate) calibration: Option<PathBuf>,

    /// Report the chosen threshold's performance on these rows.
    #[arg(long, value_name = "PATH", requires = "calibration")]
    pub(crate) test: Option<PathBuf>,

    /// What the threshold should achieve. Without it, no threshold is chosen.
    #[arg(long, value_enum, value_name = "OBJECTIVE")]
    pub(crate) objective: Option<Objective>,

    /// The floor, or the coverage, the objective aims at, from 0 to 1.
    #[arg(long, value_name = "F")]
    pub(crate) target: Option<f64>,

    /// Seed for the held-out split; the same seed always splits the same rows
    #[arg(long, default_value_t = 0, value_name = "N", conflicts_with_all = ["calibration", "test", "no_split"])]
    pub(crate) seed: u64,

    /// Share of rows held out to report on; strictly between 0 and 1
    #[arg(long, default_value_t = 0.3, value_name = "F", conflicts_with_all = ["calibration", "test", "no_split"])]
    pub(crate) test_fraction: f64,

    /// Choose and report on the same rows. The result will be optimistic; it says so.
    #[arg(long, conflicts_with_all = ["calibration", "test"])]
    pub(crate) no_split: bool,

    /// Evaluate only the first N rows of the dataset; at least 1
    #[arg(long, value_name = "N")]
    pub(crate) limit: Option<usize>,

    /// Also write the JSON report here, for comparing against a later model version.
    #[arg(long, value_name = "PATH")]
    pub(crate) report: Option<PathBuf>,

    /// Include each reported row's label and answer in the JSON report.
    #[arg(long)]
    pub(crate) show_rows: bool,

    /// Requests in flight at once; at most 64
    #[arg(long, short = 'j', default_value_t = 4, value_name = "N")]
    pub(crate) concurrency: usize,

    /// Stop at the first failing row instead of continuing.
    #[arg(long)]
    pub(crate) fail_fast: bool,
}

/// `jev doctor`
#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Also make one minimal API call to confirm the credential works.
    #[arg(long)]
    pub(crate) live: bool,
}

/// `jev mcp`
#[derive(Debug, Subcommand)]
#[command(
    long_about = "Connect `jev` to an AI agent host -- Claude Code, Codex, Cursor, Grok, or any \
                  MCP client -- as a local Model Context Protocol server.\n\n\
                  The server offers five tools: noul, choice, score, ask, and map. They run \
                  the same code as the commands of the same names and return the same JSON \
                  documents. Every state passed to a tool is sent to the configured TypeSafe \
                  endpoint, exactly as it would be from the command line.\n\n\
                  Setup for each host is in docs/mcp.md."
)]
pub enum McpCommand {
    /// Run an MCP server on standard input and output, until the host disconnects.
    #[command(
        long_about = "Run a Model Context Protocol server over stdio. The host starts this \
                      process and talks to it on stdin and stdout; there is no port, no \
                      daemon, and nothing to install besides `jev`.\n\n\
                      stdout carries protocol messages and nothing else. Diagnostics, when \
                      there are any, go to stderr. The server is silent by default; --verbose \
                      adds a line per call, and a non-official endpoint is always warned \
                      about once at startup.\n\n\
                      Credentials resolve exactly as for every other command: `jev auth \
                      login`, or TYPESAFE_API_KEY / JEV_API_KEY in the host's environment. \
                      Never put a key in the host's MCP configuration as an argument.\n\n\
                      Global flags set the server's defaults, for example \
                      `jev --model jev-1.13.0 mcp serve`. --dry-run is refused."
    )]
    Serve,
}

/// `jev auth`
#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Store a credential in the operating system credential store.
    Login(LoginArgs),
    /// Report where a credential would come from, without showing it.
    Status,
    /// Remove the credential this CLI stored, and nothing else.
    Logout,
}

/// `jev auth login`
#[derive(Debug, Args)]
#[command(
    long_about = "Store a TypeSafe API key in the operating system credential store: the macOS \
                  Keychain, the Windows Credential Manager, or the Secret Service on Linux.\n\n\
                  There is no --api-key flag and there will not be one: arguments are visible \
                  in `ps`, in shell history, and in CI logs.\n\n\
                  If no secure store is available, this fails and tells you to use \
                  JEV_API_KEY. It never writes a key to a plaintext file."
)]
pub struct LoginArgs {
    /// Read the key from standard input instead of prompting.
    #[arg(long)]
    pub(crate) stdin: bool,
}

/// `jev config`
#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print every setting that is set.
    List,
    /// Print one setting.
    Get {
        /// The setting name.
        key: String,
    },
    /// Set one setting. Secrets are refused.
    Set {
        /// The setting name.
        key: String,
        /// The value.
        value: String,
    },
    /// Clear one setting.
    Unset {
        /// The setting name.
        key: String,
    },
    /// Print the configuration file path.
    Path,
}

/// `jev completions`
#[derive(Debug, Args)]
pub struct CompletionsArgs {
    /// The shell to generate for.
    #[arg(value_enum)]
    pub(crate) shell: clap_complete::Shell,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::*;

    #[test]
    fn the_definition_is_internally_consistent() {
        // Catches conflicting short flags, duplicate names, and bad defaults at test
        // time rather than on a user's first invocation.
        Cli::command().debug_assert();
    }

    #[test]
    fn there_is_no_way_to_pass_a_credential_as_an_argument() {
        // Threat model T1, enforced structurally: walk every argument of every
        // subcommand and assert none of them is a credential.
        fn walk(command: &clap::Command, path: &str) {
            for argument in command.get_arguments() {
                let id = argument.get_id().as_str();
                let long = argument.get_long().unwrap_or("");
                for name in [id, long] {
                    let lowered = name.to_ascii_lowercase();
                    let looks_like_a_secret = [
                        "api-key", "api_key", "apikey", "token", "secret", "password",
                    ]
                    .iter()
                    .any(|needle| lowered.contains(needle));
                    assert!(
                        !looks_like_a_secret,
                        "{path} accepts a credential as an argument: `{name}`"
                    );
                }
            }
            for sub in command.get_subcommands() {
                walk(sub, &format!("{path} {}", sub.get_name()));
            }
        }
        walk(&Cli::command(), "jev");
    }

    #[test]
    fn state_flags_are_mutually_exclusive() {
        for pair in [
            ["--state", "--state-file"],
            ["--state", "--state-json"],
            ["--state-file", "--state-json-file"],
        ] {
            let result = Cli::try_parse_from(["jev", "noul", "q", pair[0], "a", pair[1], "b"]);
            assert!(result.is_err(), "accepted {pair:?} together");
        }
    }

    #[test]
    fn value_and_json_output_cannot_be_combined() {
        // `--value` prints a bare scalar; `--output json` prints a document. Asking for
        // both is a mistake, not a preference to resolve silently.
        assert!(Cli::try_parse_from(["jev", "noul", "q", "--value", "--output", "json"]).is_err());
    }

    #[test]
    fn resume_requires_an_output_file() {
        // Without one there is nothing to resume from, and a silent full re-run would
        // cost the user real money.
        assert!(Cli::try_parse_from(["jev", "map", "-r", "q.json", "--resume"]).is_err());
        assert!(
            Cli::try_parse_from([
                "jev",
                "map",
                "-r",
                "q.json",
                "--resume",
                "--output-file",
                "o.jsonl"
            ])
            .is_ok()
        );
    }

    #[test]
    fn every_subcommand_accepts_the_global_flags() {
        for command in ["doctor", "models"] {
            assert!(
                Cli::try_parse_from(["jev", command, "--output", "json"]).is_ok(),
                "{command} rejected --output"
            );
        }
    }

    #[test]
    fn the_about_text_disclaims_endorsement_and_names_where_data_goes() {
        let rendered = Cli::command().render_long_help().to_string();
        let lowered = rendered.to_lowercase();
        assert!(lowered.contains("not affiliated"));
        assert!(lowered.contains("transmitted"));
        assert!(lowered.contains("not a stable interface"));
    }

    #[test]
    fn the_command_set_is_the_documented_one() {
        // Locks the stable surface: adding a command is fine, renaming one is not.
        let mut names: Vec<String> = Cli::command()
            .get_subcommands()
            .map(|sub| sub.get_name().to_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "ask",
                "auth",
                "choice",
                "completions",
                "config",
                "doctor",
                "eval",
                "map",
                "mcp",
                "models",
                "noul",
                "score",
            ]
        );
    }
}
