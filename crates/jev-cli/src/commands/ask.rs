//! `jev ask` — several independent questions about one state, in one request.
//!
//! This is the command that matters most for cost and latency. System One evaluates
//! every question in a request against one reading of the state, in parallel, so ten
//! questions in one call cost far less and finish in roughly the same time as one. The
//! official documentation measures 12.2× cheaper and 10.0× faster for a 13-question
//! briefing (<https://docs.typesafe.ai/cookbooks/parallel_questions>). Speculative
//! questions are worth including for the same reason: the code reads only the answers
//! the branch it took needs.

use jev_client::Transport;

use crate::cli::AskArgs;
use crate::commands::{Session, evaluate};
use crate::errors::{CliError, Result};
use crate::request;

/// Runs `jev ask`.
pub(crate) fn run(
    session: &mut Session<'_>,
    args: &AskArgs,
    transport: Option<&(dyn Transport + Send + Sync)>,
) -> Result<u8> {
    let cli_features =
        crate::media::Features::for_cli(&session.context, crate::media::Features::default())?;
    let state_args = &args.state;
    let state_given = state_args.state.is_some()
        || state_args.state_file.is_some()
        || state_args.state_json.is_some()
        || state_args.state_json_file.is_some();

    // Without a named document, stdin carries the whole request; this makes
    // `jev ask < request.json` work without claiming stdin as a separate text state.
    let path = args
        .questions
        .as_ref()
        .or(args.request.as_ref())
        .cloned()
        .unwrap_or_else(|| std::path::PathBuf::from("-"));
    let (text, origin) = read_text(session, &path)?;
    let document = request::parse_document_for_endpoint_with_images(
        &text,
        &origin,
        &session.context.endpoint.value,
        &cli_features.images,
    )?;

    if args.questions.is_some() && document.state.is_some() {
        return Err(CliError::usage(format!(
            "{origin} carries a `state`, but --questions reads only the questions map.\n\n\
             Use --request to send the whole document, or remove the `state` field."
        )));
    }
    if args.questions.is_some()
        && (!document.features.images.is_empty()
            || document.features.options.is_some()
            || document.features.keep_alive.is_some()
            || !document.features.videos.is_empty()
            || document.features.max_length.is_some()
            || document.features.max_state_tokens.is_some()
            || document.features.media_kwargs.is_some())
    {
        return Err(CliError::usage(format!(
            "{origin} carries media or provider options, but --questions reads only the questions map.\n\n\
             Use --request to send the whole document, or remove images, videos, options, \
             keep_alive, max_length, max_state_tokens, and media_kwargs from the questions file."
        )));
    }

    let features = cli_features.merge_cli(&session.context, document.features)?;
    let state = if let Some(state) = document.state {
        if state_given {
            return Err(CliError::usage(format!(
                "{origin} carries a `state` and a --state flag was also given.\n\n\
                 Pick one: `jev` will not guess which state you meant."
            )));
        }
        state
    } else {
        // The request document supplied no state, so it comes from the flags — or from
        // stdin, unless stdin was already consumed reading the document.
        let source = evaluate::state_source(state_args);
        // With `--questions`, stdin is not the request document, so it is available for
        // the state and `echo … | jev ask --questions q.json` works like every other
        // command. With `--request` (or no flag at all) stdin *is* the document, and
        // there is nothing left to read the state from.
        let stdin_is_available = args.questions.is_some() && !session.stdin_is_terminal;
        if matches!(source, crate::input::StateSource::ImplicitStdin)
            && !state_given
            && !stdin_is_available
        {
            return Err(CliError::usage(format!(
                "no state was supplied, and {origin} does not carry one.\n\n\
                 Pass --state, --state-file, --state-json, or --state-json-file — or, \
                 with --questions, pipe the state into `jev` on standard input."
            )));
        }
        let images = if session.context.endpoint.value.is_cloudflare() {
            features.images.as_slice()
        } else {
            &[]
        };
        if session.context.endpoint.value.provider() == "huggingface" {
            session.reader().publisher_state(&source, session.stdin)?
        } else {
            session
                .reader()
                .state_with_images(&source, session.stdin, images)?
        }
    };

    // Precedence: an explicit --model beats the document, which beats the default. The
    // flag is the most specific statement of intent, and it is what makes a committed
    // request file reusable across models.
    let model = if session.context.model.from == crate::context::Provenance::Flag {
        session.context.model.value.clone()
    } else {
        document
            .model
            .unwrap_or_else(|| session.context.model.value.clone())
    };

    let request = features.apply(request::build(state, model, document.questions)?)?;
    session.note(&format!(
        "{} question(s) in one request",
        request.question_count()
    ));
    evaluate::execute(session, &request, &args.answer, transport)
}

/// Reads a document as UTF-8 text, from a file or from stdin.
fn read_text(session: &mut Session<'_>, path: &std::path::Path) -> Result<(String, String)> {
    let (bytes, origin) = session.reader().read(path, session.stdin)?;
    let label = origin.to_string();
    let text = String::from_utf8(bytes)
        .map_err(|_| CliError::usage(format!("{label} is not valid UTF-8")))?;
    if text.trim().is_empty() {
        return Err(CliError::usage(format!("{label} is empty")));
    }
    Ok((text, label))
}
