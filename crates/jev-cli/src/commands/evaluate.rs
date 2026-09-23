//! `jev noul`, `jev choice`, and `jev score`, and the shared evaluation path.
//!
//! All three build a single-question [`EvaluationRequest`] and hand it to
//! [`execute`], which is also what `jev ask` uses. Keeping one execution path means the
//! dry-run behaviour, the endpoint warning, the output contract, and the gate semantics
//! cannot drift between commands.

use jev_client::{Client, Credential, Transport};
use jev_core::{Content, EvaluationRequest, EvaluationResponse, Question, QuestionId, Scalar};
use serde_json::{Value, json};
use std::fmt::Write as _;

use crate::cli::{AnswerArgs, ChoiceArgs, NoulArgs, ScoreArgs, StateArgs};
use crate::commands::{Session, http_client};
use crate::errors::{CliError, Result};
use crate::exit;
use crate::gate::{self, GateOutcome};
use crate::input::StateSource;
use crate::render::{DRY_RUN_SCHEMA, json as render_json, text as render_text};
use crate::request;

/// `jev noul`
pub(crate) fn noul(
    session: &mut Session<'_>,
    args: &NoulArgs,
    transport: Option<&(dyn Transport + Send + Sync)>,
) -> Result<u8> {
    let instructions = instructions(&args.instructions)?;
    let criteria = match (&args.yes, &args.no) {
        (None, None) => None,
        (yes, no) => {
            let side = |value: &Option<String>, flag: &str| -> Result<Option<Content>> {
                value
                    .as_ref()
                    .map(|text| {
                        Content::text(text.clone())
                            .map_err(|_| CliError::usage(format!("--{flag} must not be empty")))
                    })
                    .transpose()
            };
            Some(
                jev_core::NoulCriteria::new(side(yes, "true")?, side(no, "false")?)
                    .map_err(|error| CliError::usage(error.to_string()))?,
            )
        }
    };
    let question = Question::noul(instructions, criteria)
        .map_err(|error| CliError::usage(error.to_string()))?;
    single(
        session,
        &args.id,
        question,
        &args.state,
        &args.answer,
        transport,
    )
}

/// `jev choice`
pub(crate) fn choice(
    session: &mut Session<'_>,
    args: &ChoiceArgs,
    transport: Option<&(dyn Transport + Send + Sync)>,
) -> Result<u8> {
    let instructions = instructions(&args.instructions)?;
    let options = if let Some(path) = &args.options_file {
        let (bytes, origin) = session.reader().read(path, session.stdin)?;
        let text = String::from_utf8(bytes)
            .map_err(|_| CliError::usage(format!("{origin} is not valid UTF-8")))?;
        request::parse_options_file(&text, &origin.to_string())?
    } else {
        if args.options.is_empty() {
            return Err(CliError::usage(
                "a choice question needs options\n\n\
                 Pass --option NAME or --option NAME=DESCRIPTION for each one, or \
                 --options-file PATH with a JSON object of them.",
            ));
        }
        args.options
            .iter()
            .map(|raw| request::parse_option(raw))
            .collect::<Result<Vec<_>>>()?
    };
    let question = Question::choice(instructions, options)
        .map_err(|error| CliError::usage(error.to_string()))?;
    single(
        session,
        &args.id,
        question,
        &args.state,
        &args.answer,
        transport,
    )
}

/// `jev score`
pub(crate) fn score(
    session: &mut Session<'_>,
    args: &ScoreArgs,
    transport: Option<&(dyn Transport + Send + Sync)>,
) -> Result<u8> {
    let instructions = instructions(&args.instructions)?;
    let levels = if let Some(path) = &args.levels_file {
        let (bytes, origin) = session.reader().read(path, session.stdin)?;
        let text = String::from_utf8(bytes)
            .map_err(|_| CliError::usage(format!("{origin} is not valid UTF-8")))?;
        request::parse_levels_file(&text, &origin.to_string())?
    } else {
        if args.levels.is_empty() {
            return Err(CliError::usage(
                "a score question needs levels\n\n\
                 Pass --level TEXT for each level, lowest first, or --levels-file \
                 PATH with a JSON array of them.",
            ));
        }
        args.levels
            .iter()
            .map(|text| {
                Content::text(text.clone())
                    .map_err(|_| CliError::usage("--level must not be empty"))
            })
            .collect::<Result<Vec<_>>>()?
    };
    let question = Question::score(instructions, levels)
        .map_err(|error| CliError::usage(error.to_string()))?;
    single(
        session,
        &args.id,
        question,
        &args.state,
        &args.answer,
        transport,
    )
}

fn instructions(raw: &str) -> Result<Content> {
    Content::text(raw.to_owned()).map_err(|_| CliError::usage("the instructions must not be empty"))
}

/// Shared path for the three single-question commands.
fn single(
    session: &mut Session<'_>,
    id: &str,
    question: Question,
    state_args: &StateArgs,
    answer_args: &AnswerArgs,
    transport: Option<&(dyn Transport + Send + Sync)>,
) -> Result<u8> {
    let id = QuestionId::new(id.to_owned())
        .map_err(|error| CliError::usage(format!("--id: {error}")))?;
    let state = session
        .reader()
        .state(&state_source(state_args), session.stdin)?;
    let model = session.context.model.value.clone();
    let request = request::build(state, model, vec![(id, question)])?;
    execute(session, &request, answer_args, transport)
}

/// Maps the state flags onto a [`StateSource`].
///
/// `clap` has already rejected any two of them together, so the order here is not a
/// precedence rule.
pub(crate) fn state_source(args: &StateArgs) -> StateSource {
    if let Some(text) = &args.state {
        return StateSource::Text(text.clone());
    }
    if let Some(path) = &args.state_file {
        return StateSource::TextFile(path.clone());
    }
    if let Some(raw) = &args.state_json {
        return StateSource::Json(raw.clone());
    }
    if let Some(path) = &args.state_json_file {
        return StateSource::JsonFile(path.clone());
    }
    StateSource::ImplicitStdin
}

/// Sends a request, renders the response, and applies any gate.
///
/// # Errors
///
/// Returns the classified failure. A gate that fails or cannot be evaluated is
/// reported through the return code rather than as an error, because the data was
/// still produced and still belongs on stdout.
pub(crate) fn execute(
    session: &mut Session<'_>,
    request: &EvaluationRequest,
    answer_args: &AnswerArgs,
    transport: Option<&(dyn Transport + Send + Sync)>,
) -> Result<u8> {
    // Parsed before the request is sent: a typo in the expression should cost nothing.
    let gate_expression = answer_args
        .require
        .as_ref()
        .map(|raw| {
            gate::parse(raw)
                .map(|parsed| (raw.clone(), parsed))
                .map_err(|error| CliError::usage(format!("--require: {error}\n\n{}", gate_help())))
        })
        .transpose()?;

    session.warn_about_endpoint();

    if session.context.dry_run {
        // `--value` promises one scalar and nothing else, and a dry run has no scalar
        // to print. Silently emitting a JSON document instead broke that promise in the
        // quietest possible way: `x=$(jev … --value --dry-run)` came back holding a
        // whole document.
        if answer_args.value {
            session.note_always(
                "note: --value has nothing to print in a dry run; showing the request \
                 instead",
            );
        }
        return dry_run(
            session,
            request,
            gate_expression.as_ref().map(|(raw, _)| raw.as_str()),
        );
    }

    let (credential, source) = session.credential()?;
    session.note(&format!("credential source: {source}"));
    session.note(&format!(
        "POST {}/v1/systemone ({} question(s))",
        session.context.endpoint.value,
        request.question_count()
    ));

    let (result, stats) = send(
        &session.context,
        &crate::interrupt::InterruptibleClock::default(),
        request,
        &credential,
        transport,
    );
    session.note(&format!(
        "{} attempt(s) in {} ms",
        stats.attempts,
        stats.elapsed.as_millis()
    ));
    if let Some(request_id) = &stats.request_id {
        // Worth saying out loud: it is what TypeSafe support can use to find this call,
        // and it is gone once the process exits.
        session.note(&format!(
            "request id: {}",
            crate::output::Safe::new(request_id)
        ));
    }
    // Checked before the result is unwrapped. An interrupt ends a backoff wait early
    // (`interrupt::InterruptibleClock`) and the retry loop then stops, so the value in
    // hand is whatever the last attempt produced -- usually the 429 or 503 that caused
    // the wait. Reporting that as exit 4 would say "the API is unavailable" when what
    // actually happened is that the user pressed Ctrl-C.
    if crate::interrupt::requested() {
        return Err(CliError::Interrupted);
    }
    let response = result?;

    report_missing_answers(session, request, &response);
    warn_about_alias(session, request, &response);

    let outcome = gate_expression
        .as_ref()
        .map(|(_, parsed)| gate::evaluate(parsed, &response));

    if answer_args.value {
        write_scalar(session, &response)?;
    } else if session.json() {
        let document = render_json::evaluation(
            &response,
            request.model().as_str(),
            &session.context.endpoint.value.to_string(),
            gate_expression
                .as_ref()
                .zip(outcome.as_ref())
                .map(|((raw, _), outcome)| (raw.as_str(), outcome)),
            stats.request_id.as_deref(),
            &response.missing(request.questions().iter().map(|(id, _)| id)),
        );
        render_json::write_document(session.out, &document)?;
    } else {
        // The renderer writes answers to stdout and hands back its commentary, which
        // goes to stderr: stdout carries data and nothing else.
        let color = session.context.color;
        let notes = render_text::evaluation(session.out, &response, request.questions(), color)?;
        for line in notes.lines {
            session.note_always(&line);
        }
    }

    let expression = gate_expression.as_ref().map_or("", |(raw, _)| raw.as_str());
    Ok(match outcome {
        None => exit::SUCCESS,
        Some(ref outcome @ GateOutcome::Passed) => {
            session.note(&render_text::gate(expression, outcome));
            exit::SUCCESS
        }
        Some(ref outcome @ GateOutcome::Failed) => {
            // Exit 1 alone tells a person nothing. Say which expression, and say it
            // even under `--quiet`, because it is the reason for the exit status.
            session.note_always(&render_text::gate(expression, outcome));
            exit::UNSATISFIED
        }
        Some(ref outcome @ GateOutcome::Unevaluable { .. }) => {
            // Never silently a pass. The distinct code is the whole point.
            let _ = writeln!(
                session.err,
                "error: {}",
                render_text::gate(expression, outcome)
            );
            exit::GATE_UNEVALUABLE
        }
    })
}

/// Sends through the injected transport when there is one, and over HTTP otherwise.
///
/// Shared with `jev mcp serve`, which passes a clock carrying the tool call's own
/// cancellation flag.
pub(crate) fn send(
    context: &crate::context::Context,
    clock: &crate::interrupt::InterruptibleClock,
    request: &EvaluationRequest,
    credential: &Credential,
    transport: Option<&(dyn Transport + Send + Sync)>,
) -> (Result<EvaluationResponse>, jev_client::CallStats) {
    if let Some(transport) = transport {
        let client = Client::with_clock(transport, context.endpoint.value.clone(), clock.clone())
            .with_retry(context.retry);
        let (result, stats) = client.evaluate(request, credential);
        (result.map_err(Into::into), stats)
    } else {
        let client = http_client(context, clock.clone());
        let (result, stats) = client.evaluate(request, credential);
        (result.map_err(Into::into), stats)
    }
}

/// Prints what would be sent, with no credential anywhere in it.
///
/// The document is derived from [`jev_client::build_evaluation_request`] — the same
/// function a real run uses — rather than from a second, parallel construction of the
/// URL, the header set, and the body. A rehearsal that is assembled separately from
/// the performance can diverge from it, and the whole value of `--dry-run` is that it
/// cannot.
fn dry_run(
    session: &mut Session<'_>,
    request: &EvaluationRequest,
    gate: Option<&str>,
) -> Result<u8> {
    let built = jev_client::build_evaluation_request(&session.context.endpoint.value, request)
        .map_err(|error| CliError::internal(format!("could not encode the request: {error}")))?;

    // Decoded from the bytes that would go on the wire, so this cannot show a body the
    // request does not have. `body_bytes` is the exact length of what would be sent.
    //
    // The decode does not preserve JSON object key order, so a Choice written zebra,
    // apple, mango is *displayed* apple, mango, zebra. The request itself keeps the
    // order the user wrote -- `jev-core`'s serializers are hand-written for that reason,
    // and `a_choice_reaches_the_api_in_the_order_it_was_written` pins it against a real
    // socket. Only this display sorts, because the document is assembled as a
    // `serde_json::Value` and `serde_json::Map` is a `BTreeMap`. `docs/commands.md`
    // says so rather than leaving a reader to infer that the model sees this order.
    let body: Value = serde_json::from_slice(&built.body)
        .map_err(|error| CliError::internal(format!("could not decode the request: {error}")))?;

    // Names only, from the request itself, plus `authorization`: that header is added
    // by the transport at send time and is deliberately not built here, so no
    // credential is read to produce this document. `host` and `content-length` are
    // added by the HTTP layer below this crate and are not listed for the same reason.
    let mut headers: Vec<&str> = built.headers.keys().map(String::as_str).collect();
    headers.push("authorization");
    headers.sort_unstable();

    let document = json!({
        "schema": DRY_RUN_SCHEMA,
        "method": built.method,
        "url": built.url,
        "headers": headers,
        "body": body,
        "body_bytes": built.body.len(),
        "credential": session.credential_availability(),
        // The parsed expression, so a user can confirm `--require` was read the way
        // they meant before spending a request on it. The key is always present; its
        // value is `null` when no expression was given.
        "gate": gate.map(|expression| json!({"expression": expression})),
        "sent": false,
    });
    if session.json() {
        render_json::write_document(session.out, &document)?;
    } else {
        let rendered = serde_json::to_string_pretty(&document)
            .map_err(|error| CliError::internal(error.to_string()))?;
        // Escaped, exactly as the one-line encoder escapes. This branch is the *human*
        // one -- it is read on a terminal, by definition -- and it is the one that was
        // writing the pretty bytes straight out. A `--dry-run` exists to rehearse a
        // request built from untrusted state: a ticket body, a log line, a diff. Left
        // raw, a bidirectional override in that state reorders the preview on the
        // reviewer's own screen, so the thing they approve is not the thing they read.
        // `\uXXXX` parses back to the identical string, so nothing is lost.
        let rendered = render_json::escape_terminal_hazards_pretty(&rendered);
        render_json::write_all(session.out, rendered.as_bytes())?;
        render_json::write_all(session.out, b"\n")?;
    }
    session.warn("dry run: nothing was sent");
    Ok(exit::SUCCESS)
}

/// Prints exactly one scalar.
///
/// `--value` promises "one scalar and a newline, nothing else". Printing one line per
/// answer would break that promise silently, and the lines would be unlabelled — a
/// script reading `$(jev … --value)` into a variable would get three lines and no way
/// to tell which was which. A multi-answer response is therefore an error that names
/// the answers and points at the flag that does work.
fn write_scalar(session: &mut Session<'_>, response: &EvaluationResponse) -> Result<()> {
    let [(id, answer)] = &response.answers[..] else {
        let ids: Vec<&str> = response.answers.iter().map(|(id, _)| id.as_str()).collect();
        return Err(CliError::usage(format!(
            "--value prints one scalar, but this request has {} answers ({}).\n\n\
             Use --output json and pick one, for example:\n  \
               jev … --output json | jq -r '.answers.{}.noul'",
            ids.len(),
            ids.join(", "),
            ids.first().copied().unwrap_or("<id>"),
        )));
    };

    let rendered = match answer.scalar() {
        Some(Scalar::Number(value)) => format!("{value}\n"),
        // `sanitize_scalar`, not `sanitize`: the option name came back from the API and
        // lands in a terminal, and it must also stay on one line, because `--value`
        // promises exactly one. See `output::sanitize_scalar`.
        Some(Scalar::Text(text)) => format!("{}\n", crate::output::sanitize_scalar(text)),
        None => {
            return Err(CliError::usage(format!(
                "--value cannot render the answer to `{}`, whose type is `{}`; \
                 use --output json to see it",
                crate::output::Safe::new(id.as_str()),
                crate::output::Safe::new(answer.kind_str()),
            )));
        }
    };
    render_json::write_all(session.out, rendered.as_bytes())
}

/// Warns when a question was asked but not answered.
///
/// The API returns one answer per question, so a gap means the response did not match
/// the request. Saying nothing would let a caller read a missing answer as a negative
/// one.
fn report_missing_answers(
    session: &mut Session<'_>,
    request: &EvaluationRequest,
    response: &EvaluationResponse,
) {
    let asked: Vec<&QuestionId> = request.questions().iter().map(|(id, _)| id).collect();
    let missing = response.missing(asked);
    if !missing.is_empty() {
        session.warn(&format!(
            "warning: the API returned no answer for: {}",
            missing.join(", ")
        ));
    }
}

/// Notes, under `--verbose`, that a moving alias was used and which version answered.
fn warn_about_alias(
    session: &mut Session<'_>,
    request: &EvaluationRequest,
    response: &EvaluationResponse,
) {
    if request.model().is_moving_alias() && request.model() != &response.model {
        session.note(&format!(
            "`{}` resolved to `{}`; pin that identifier if you have calibrated a threshold",
            request.model(),
            response.model
        ));
    }
}

/// The `--require` grammar summary, appended to a parse error.
pub(crate) fn gate_help() -> String {
    let mut text = String::from("Paths you can compare:\n");
    for (path, description) in gate::addressable_fields() {
        let _ = writeln!(text, "  {path:<38}  {description}");
    }
    text.push_str(
        "Operators: > >= < <= == !=   Combine with `and`, `or`, `not`, and parentheses.\n\
         Example: --require 'urgent.noul > 0.9 and team.choice == billing'",
    );
    text
}
