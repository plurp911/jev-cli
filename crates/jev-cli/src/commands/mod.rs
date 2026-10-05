//! Dispatch, and the plumbing every command shares.
//!
//! # Testability
//!
//! [`run`] takes its streams, its stdin, its environment, and its terminal-detection
//! results as arguments. Nothing here reads a global. That is what lets the entire
//! command surface — including credential precedence, colour resolution, and exit codes
//! — be tested in process, with a mock transport, and in parallel.

mod ask;
mod auth;
mod completions;
mod config;
mod doctor;
mod eval;
pub(crate) mod evaluate;
pub(crate) mod map;
mod models;

use std::ffi::OsString;
use std::io::{Read, Write};
use std::process::ExitCode;

use clap::Parser as _;
use jev_client::{Client, Credential, HttpTransport, Transport};
use jev_config::{Environment, SecretStore};

use crate::cli::{Cli, Command, OutputFormat};
use crate::context::Context;
use crate::errors::{CliError, Result};
use crate::exit;
use crate::input::InputReader;
use crate::interrupt::InterruptibleClock;
use crate::output::sanitize;

/// What the process knows about its own streams.
#[derive(Debug, Clone, Copy)]
pub struct Streams {
    /// Whether stdin is a terminal, which decides if implicit stdin input is sensible.
    pub stdin_is_terminal: bool,
    /// Whether **stdout** is a terminal.
    ///
    /// This is what decides automatic colour, because stdout is where the coloured
    /// output goes. Reading stderr's terminal-ness instead — an easy mistake, and one
    /// this field exists to prevent — means `jev … > file` writes ANSI escapes into the
    /// file whenever the shell's stderr happens to be a terminal.
    pub stdout_is_terminal: bool,
    /// Whether stderr is a terminal.
    pub stderr_is_terminal: bool,
}

/// Everything a command needs, assembled once.
///
/// Holds trait objects for the output streams, the environment, and the credential
/// store, so `Debug` is hand-written rather than derived -- and deliberately omits the
/// store, whose description is the only part worth printing and which is reachable
/// through `jev doctor` anyway.
pub struct Session<'a> {
    /// The resolved configuration.
    pub(crate) context: Context,
    /// Where data goes.
    pub(crate) out: &'a mut dyn Write,
    /// Where diagnostics go.
    pub(crate) err: &'a mut dyn Write,
    /// Standard input, for `-` and for implicit state.
    pub(crate) stdin: &'a mut dyn Read,
    /// The process environment.
    pub(crate) environment: &'a dyn Environment,
    /// The credential store.
    pub(crate) store: &'a (dyn SecretStore + Send + Sync),
    /// Whether stdin is a terminal.
    pub(crate) stdin_is_terminal: bool,
}

impl std::fmt::Debug for Session<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("context", &self.context)
            .field("stdin_is_terminal", &self.stdin_is_terminal)
            .finish_non_exhaustive()
    }
}

impl Session<'_> {
    /// Writes a diagnostic to stderr unless `--quiet`.
    ///
    /// Sanitized, because a diagnostic can quote a filename or an API message.
    pub(crate) fn warn(&mut self, message: &str) {
        if self.context.verbosity.quiet {
            return;
        }
        let _ = writeln!(self.err, "{}", sanitize(message));
    }

    /// Writes a diagnostic only under `--verbose`.
    /// Writes a diagnostic that `--quiet` does not suppress.
    ///
    /// For the few things a user needs even when they asked for silence: why the exit
    /// status is what it is, and where their data is going.
    pub fn note_always(&mut self, message: &str) {
        let _ = writeln!(self.err, "{}", sanitize(message));
    }

    /// Writes a diagnostic only under `--verbose`.
    pub(crate) fn note(&mut self, message: &str) {
        if !self.context.verbosity.verbose {
            return;
        }
        let _ = writeln!(self.err, "{}", sanitize(message));
    }

    /// An input reader configured from the context.
    pub(crate) fn reader(&self) -> InputReader {
        InputReader {
            max_bytes: self.context.max_input_bytes.value,
            stdin_is_terminal: self.stdin_is_terminal,
        }
    }

    /// Resolves a credential for the configured endpoint.
    ///
    /// # Errors
    ///
    /// Returns an auth-class [`CliError`] naming every source that was tried.
    pub(crate) fn credential(&self) -> Result<(Credential, jev_config::CredentialSource)> {
        resolve_credential(&self.context, self.environment, self.store)
    }

    /// Reports whether a credential is *configured*, and where from, without reading
    /// one and without touching the credential store.
    ///
    /// This is deliberately weaker than [`jev_config::Credentials::availability`],
    /// which resolves: resolving reads the file named by `JEV_API_KEY_FILE` into
    /// memory and queries the OS credential store, which on macOS can raise an unlock
    /// prompt. Doing that is right for `jev doctor`, whose job is to find out. It is
    /// wrong for `--dry-run`, which is sold on touching nothing — a rehearsal that
    /// opens the user's key file and prompts their Keychain is not a rehearsal.
    ///
    /// So this looks only at the *shape* of each source, in resolution order, and
    /// reports the first one that is populated. It can therefore say "available" for a
    /// source that would turn out to be unusable — a key file that exists but cannot be
    /// read — which is the right trade for a preview: it answers "have you configured
    /// anything?" without reading what you configured. `jev doctor` answers the
    /// stronger question.
    pub(crate) fn credential_availability(&self) -> serde_json::Value {
        let credentials = credentials(&self.context, self.environment, self.store);
        if local_anonymous(&self.context) {
            return serde_json::json!({"available": true, "source": "anonymous",
                "checked": "local inference; no credential is used"});
        }
        // `sources_shape` consults the environment only. The OS store is not listed for
        // a custom endpoint and is not queried here for the official one.
        let configured = credentials
            .sources_shape()
            .into_iter()
            .find(|(_, present)| *present == Some(true));
        serde_json::json!({
            "available": configured.is_some(),
            "source": configured.map(|(source, _)| source.to_string()),
            // Says plainly that this is the cheap answer, so nobody reads a `false`
            // here as "doctor would agree".
            "checked": "configuration only; no credential was read",
        })
    }

    /// Emits the standing warning when a non-official endpoint is in force.
    pub(crate) fn warn_about_endpoint(&mut self) {
        if self.context.ignored_config_endpoint {
            self.warn("note: ignoring the saved endpoint because the selected provider differs; use --endpoint to override this provider");
        }
        if let Some(warning) = self.context.endpoint_warning() {
            // Not suppressed by `--quiet`: where the user's credential and data are
            // being sent is not a routine progress note.
            let _ = writeln!(self.err, "{}", sanitize(&warning));
        }
    }

    /// Whether output should be JSON.
    pub(crate) fn json(&self) -> bool {
        self.context.output.value == OutputFormat::Json
    }
}

/// Resolves a credential for the context's endpoint.
///
/// The one place a credential is resolved, for the CLI and for `jev mcp serve` alike, so
/// the endpoint isolation of ADR-0008 cannot differ between the two.
///
/// # Errors
///
/// Returns an auth-class [`CliError`] naming every source that was tried.
pub(crate) fn resolve_credential(
    context: &Context,
    environment: &dyn Environment,
    store: &(dyn SecretStore + Send + Sync),
) -> Result<(Credential, jev_config::CredentialSource)> {
    let credentials = credentials(context, environment, store);
    let resolved = credentials.resolve()?;
    if resolved.source == jev_config::CredentialSource::Anonymous {
        return Ok((Credential::anonymous(), resolved.source));
    }
    // The plaintext crosses from `jev-config`'s `Secret` to `jev-client`'s `Credential`
    // here and nowhere else. Both redact on `Debug` and zeroize on drop, so the window
    // is this one expression.
    Ok((
        Credential::new(resolved.secret.expose().to_owned()),
        resolved.source,
    ))
}

/// Whether an explicitly selected local protocol may run without authentication.
fn local_anonymous(context: &Context) -> bool {
    context.endpoint.value.is_local_provider() && context.endpoint.value.is_loopback()
}

/// The shared credential namespace decision for inference and diagnostics.
pub(crate) fn credentials<'a>(
    context: &Context,
    environment: &'a dyn Environment,
    store: &'a (dyn SecretStore + Send + Sync),
) -> jev_config::Credentials<'a> {
    if local_anonymous(context) {
        jev_config::Credentials::anonymous(environment, store, context.endpoint.value.to_string())
    } else {
        jev_config::Credentials::new(
            environment,
            store,
            context.endpoint_is_official(),
            context.endpoint.value.to_string(),
        )
    }
}

/// A transport that refuses every request, installed for the duration of a dry run.
///
/// It exists to be unreachable. `--dry-run` promises that nothing leaves the machine,
/// and this turns that promise from "every command remembered to check a boolean" into
/// "there is no transport that could send". If it is ever exercised, the message says
/// plainly that it is a bug in `jev` rather than anything the user did.
#[derive(Debug, Clone, Copy)]
struct RefusingTransport;

impl Transport for RefusingTransport {
    fn execute(
        &self,
        _request: &jev_client::Request,
        _credential: &Credential,
    ) -> std::result::Result<jev_client::Response, jev_client::TransportError> {
        Err(jev_client::TransportError::Unreachable {
            reason: "a request was attempted during a dry run; this is a bug in jev".to_owned(),
        })
    }
}

/// Builds the real HTTP client for the session's endpoint.
pub(crate) fn http_client(
    context: &Context,
    clock: InterruptibleClock,
) -> Client<HttpTransport, InterruptibleClock> {
    // `InterruptibleClock`, not `SystemClock`: a backoff wait must end when the user
    // presses Ctrl-C. See `interrupt::InterruptibleClock`.
    Client::with_clock(
        HttpTransport::new(context.timeout.value),
        context.endpoint.value.clone(),
        clock,
    )
    .with_retry(context.retry)
}

/// Parses `args` and runs the requested command.
///
/// Never panics on user input, and always returns an exit code from the documented
/// table.
pub fn run<I, T>(
    args: I,
    out: &mut impl Write,
    err: &mut impl Write,
    stdin: &mut impl Read,
    environment: &dyn Environment,
    streams: Streams,
) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let store = jev_config::platform_store();
    run_with_store(
        args,
        out,
        err,
        stdin,
        environment,
        streams,
        store.as_ref(),
        None,
    )
}

/// The dispatch body, with the credential store and transport injected.
///
/// `transport` is `None` in production, where each command builds a real HTTP client.
/// Tests pass a mock, which is what makes the whole surface exercisable offline.
#[allow(
    clippy::too_many_arguments,
    reason = "every argument is an injected dependency; bundling them into a struct \
              would only move the list"
)]
pub fn run_with_store<I, T>(
    args: I,
    out: &mut impl Write,
    err: &mut impl Write,
    stdin: &mut impl Read,
    environment: &dyn Environment,
    streams: Streams,
    store: &(dyn SecretStore + Send + Sync),
    transport: Option<&(dyn Transport + Send + Sync)>,
) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            // clap distinguishes "the user asked for help" from "the user got it
            // wrong"; only the latter is a failure, and only help belongs on stdout.
            let rendered = sanitize(&error.render().to_string());
            return if error.use_stderr() {
                let _ = writeln!(err, "{rendered}");
                ExitCode::from(exit::USAGE)
            } else {
                let _ = writeln!(out, "{rendered}");
                ExitCode::from(exit::SUCCESS)
            };
        }
    };

    let mut overrides = cli.global.overrides();
    if !matches!(
        &cli.command,
        Command::Noul(_)
            | Command::Choice(_)
            | Command::Score(_)
            | Command::Ask(_)
            | Command::Map(_)
            | Command::Eval(_)
            | Command::Mcp(_)
    ) && (!overrides.image_paths.is_empty()
        || !overrides.video_frames.is_empty()
        || overrides.video_fps.is_some()
        || overrides.max_length.is_some()
        || overrides.max_state_tokens.is_some()
        || overrides.media_kwargs.is_some()
        || overrides.reject_if_busy
        || overrides.keep_alive.is_some())
    {
        let _ = writeln!(
            err,
            "error: media and inference options require an inference command"
        );
        return ExitCode::from(exit::USAGE);
    }

    // Configuration editing must remain usable while the user is setting up a
    // provider whose inference configuration is not complete yet.
    if matches!(&cli.command, Command::Config(_)) && overrides.provider.is_none() {
        overrides.provider = Some(crate::cli::ProviderArg::Typesafe);
    }
    let context = match Context::resolve(&overrides, environment, streams.stdout_is_terminal) {
        Ok(context) => context,
        Err(CliError::IncompleteCloudflareConfiguration)
            if overrides.provider.is_none()
                && matches!(
                    &cli.command,
                    Command::Doctor(_) | Command::Auth(crate::cli::AuthCommand::Status)
                ) =>
        {
            let doctor = matches!(&cli.command, Command::Doctor(_));
            let result = report_incomplete_cloudflare(&overrides, environment, doctor, out);
            if let Err(error) = result {
                let _ = writeln!(err, "error: {error}");
                return ExitCode::from(error.code());
            }
            return ExitCode::from(if doctor { exit::SUCCESS } else { exit::AUTH });
        }
        Err(error) => {
            let _ = writeln!(err, "error: {error}");
            return ExitCode::from(error.code());
        }
    };

    let mut session = Session {
        context,
        out,
        err,
        stdin,
        environment,
        store,
        stdin_is_terminal: streams.stdin_is_terminal,
    };

    // Under `--dry-run` the transport is replaced by one that refuses. Each sending
    // command already checks the flag and returns before it builds a client, but that
    // is five call sites remembering the same thing: a new command, or a new path
    // inside `map`, could reach the network with `dry_run` still set and nothing would
    // fail. With `RefusingTransport` in place the guarantee is structural -- a request
    // attempted during a dry run is an internal error, loudly, instead of a send.
    let refusing = RefusingTransport;
    let transport: Option<&(dyn Transport + Send + Sync)> = if session.context.dry_run {
        Some(&refusing)
    } else {
        transport
    };

    let outcome = dispatch(&mut session, cli.command, transport);
    match outcome {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            let _ = writeln!(session.err, "error: {error}");
            ExitCode::from(error.code())
        }
    }
}

fn report_incomplete_cloudflare(
    overrides: &crate::context::Overrides,
    environment: &dyn Environment,
    doctor: bool,
    out: &mut impl Write,
) -> Result<()> {
    let message = CliError::IncompleteCloudflareConfiguration.to_string();
    let configured_json = !overrides.no_config
        && jev_config::config_path(environment)
            .ok()
            .and_then(|path| jev_config::Settings::load(&path).ok())
            .is_some_and(|settings| settings.output.as_deref() == Some("json"));
    let json = overrides
        .output
        .map_or(configured_json, |format| format == OutputFormat::Json);
    if json {
        let document = if doctor {
            serde_json::json!({"schema":crate::render::DOCTOR_SCHEMA,"configuration_error":message,"live":{"checked":false}})
        } else {
            serde_json::json!({"schema":crate::render::AUTH_SCHEMA,"action":"status","configuration_error":message,"error":message,"effective_source":null,"sources":[]})
        };
        crate::render::json::write_document(out, &document)
    } else {
        crate::render::json::write_all(out, format!("configuration error: {message}\n").as_bytes())
    }
}

fn dispatch(
    session: &mut Session<'_>,
    command: Command,
    transport: Option<&(dyn Transport + Send + Sync)>,
) -> Result<u8> {
    if crate::interrupt::requested() {
        return Err(CliError::Interrupted);
    }

    // `--dry-run` is global because every command that sends a request should honour
    // it. The commands that send nothing must *reject* it rather than accept and
    // ignore it: `jev config set … --dry-run` silently writing the file is exactly the
    // surprise the flag exists to prevent.
    if session.context.dry_run {
        let unsupported = match &command {
            Command::Auth(_) => Some("auth"),
            Command::Config(_) => Some("config"),
            Command::Completions(_) => Some("completions"),
            Command::Mcp(_) => Some("mcp"),
            Command::Doctor(_) => Some("doctor"),
            Command::Models => Some("models"),
            Command::Noul(_)
            | Command::Choice(_)
            | Command::Score(_)
            | Command::Ask(_)
            | Command::Map(_)
            | Command::Eval(_) => None,
        };
        if let Some(name) = unsupported {
            // `jev doctor --live` is the one refused command that *does* send a
            // request, so it gets its own sentence. Saying "doctor does not send a
            // request" and then "unless you pass --live" in the same message
            // contradicted itself for the one user who had.
            let detail = if matches!(&command, Command::Doctor(args) if args.live) {
                "`jev doctor --live` sends one request on purpose; omit --live to stay \
                 offline."
            } else if matches!(&command, Command::Mcp(_)) {
                // `mcp serve` does send, on every tool call; the refusal is because a
                // server that silently sent nothing would answer every call wrongly.
                return Err(CliError::usage(
                    "`jev mcp serve` does not support --dry-run: every tool call sends a \
                     request, and a server that sent nothing could only fail them.\n\n\
                     To rehearse a request, run the matching command with --dry-run, for \
                     example `jev noul … --dry-run`.",
                ));
            } else {
                "--dry-run applies to noul, choice, score, ask, map, and eval."
            };
            return Err(CliError::usage(format!(
                "`jev {name}` does not send a request, so --dry-run has nothing to \
                 show.\n\n{detail}"
            )));
        }
    }

    match command {
        Command::Noul(args) => evaluate::noul(session, &args, transport),
        Command::Choice(args) => evaluate::choice(session, &args, transport),
        Command::Score(args) => evaluate::score(session, &args, transport),
        Command::Ask(args) => ask::run(session, &args, transport),
        Command::Map(args) => map::run(session, &args, transport),
        Command::Eval(args) => eval::run(session, &args, transport),
        Command::Models => models::run(session, transport),
        Command::Doctor(args) => doctor::run(session, &args, transport),
        Command::Auth(command) => auth::run(session, command),
        Command::Config(command) => config::run(session, &command),
        Command::Completions(args) => completions::run(session, &args),
        Command::Mcp(crate::cli::McpCommand::Serve) => {
            let stdin_is_terminal = session.stdin_is_terminal;
            crate::mcp::serve(session, transport, stdin_is_terminal)
        }
    }
}
