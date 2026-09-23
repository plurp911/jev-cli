//! `jev map` — one question set over many records.
//!
//! # Scope
//!
//! Deliberately not a data-processing framework. It does one thing: the **state**
//! varies per record, the **questions** do not. That covers the batch case Jev is
//! actually for and needs no templating language, no expression evaluation over the
//! record, and no chunking or record-splitting heuristics. Those belong in a
//! purpose-built tool, not in the foundational CLI.
//!
//! # Properties
//!
//! * **Input order is output order**, even with concurrency, so results can be `paste`d
//!   or `join`ed against the input.
//! * **Rows fail independently.** One bad record does not lose the run; the exit code
//!   is `5` and every successful row is still written.
//! * **Nothing is cached.** `--resume` reads the *output file* to see which indexes are
//!   already done. That gives restartability without a response cache, so a resumed run
//!   never replays a stale model judgment and nothing is written to disk that the user
//!   did not ask for.
//! * **Bounded concurrency**, with plain threads. No async runtime (ADR-0007).
//! * **Durable.** With `--output-file`, each row is written and flushed as it
//!   completes, so a `SIGKILL` or a power cut loses at most the records still in
//!   flight. The trade is that the file is in **completion** order rather than input
//!   order; every row carries its `index`, so `sort` recovers it. Without
//!   `--output-file`, rows are buffered and printed in input order, because there is
//!   nothing to recover from if the process dies.
//! * **Interruptible.** Ctrl-C stops at the next record boundary, flushes, and exits
//!   `130`, leaving an output file that `--resume` can pick up correctly.

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;

use jev_client::{ClientError, Credential, Transport};
use jev_core::{Content, EvaluationRequest, ModelId, Question, QuestionId, State};
use serde_json::{Value, json};

use crate::batch;
use crate::cli::MapArgs;
use crate::commands::Session;
use crate::digest::{self, fnv1a};
use crate::errors::{CliError, Result};
use crate::exit;
use crate::gate::{self, Expr, GateOutcome};
use crate::interrupt;
use crate::paths::same_file;
use crate::render::{MAP_ROW_SCHEMA, MAP_SUMMARY_SCHEMA, json as render_json};
use crate::request;

/// Most records `jev map` will hold in memory at once.
///
/// Records are read fully before dispatch so that output can be written in input order
/// and `--resume` can reason about indexes. This bound keeps that from becoming an
/// unbounded allocation driven by the size of a piped file.
const MAX_RECORDS: usize = 1_000_000;

/// One input record.
#[derive(Debug, Clone)]
pub(crate) struct Record {
    index: usize,
    id: String,
    pub(crate) state: State,
    /// A digest of the state, written into the row so `--resume` can tell whether the
    /// record at this position is still the same one. See [`state_digest`].
    digest: String,
}

/// A stable, non-cryptographic digest of a record's state.
///
/// `--resume` matches on the record's position in the input. Without `--id-field` the
/// row's `id` *is* that position, so comparing ids is a tautology and the guard that is
/// supposed to catch a changed input cannot fire in the default configuration -- which
/// is the configuration nearly every run uses. Comparing a digest of the state makes it
/// real there too.
///
/// This is FNV-1a, written out rather than taken from `DefaultHasher`: the standard
/// library explicitly does not promise `DefaultHasher`'s output is stable across
/// releases, and this value is written to a file and compared by a later run that may
/// be a different build. A toolchain upgrade must not make every resume refuse.
///
/// It is a change-detector, not a security control. Nothing here depends on it being
/// hard to collide, and it carries no state content: it is a 16-character hex value.
fn state_digest(state: &State) -> String {
    fnv1a(&state.content().to_value().to_string())
}

impl Record {
    /// A record at `index`, digested once here so no caller can forget to.
    pub(crate) fn new(index: usize, id: String, state: State) -> Self {
        let digest = state_digest(&state);
        Self {
            index,
            id,
            state,
            digest,
        }
    }
}

/// A digest of everything about the request that is the *same* for every record.
///
/// `state_digest` catches a changed input. Nothing caught a changed **question set** or
/// a changed **model**, and editing the prompt after a disappointing first run is at
/// least as common as editing the input. Resuming across that edit produced a file whose
/// early rows answered one question and whose later rows answered another, reported the
/// batch complete, and exited `0`.
///
/// Digested from the serialized questions, which is what actually goes on the wire, so a
/// changed instruction, an added option, a reordered Score level, and a renamed id all
/// move it. The model is included because the same question answered by a different
/// model is a different result.
pub(crate) fn request_fingerprint(questions: &[(QuestionId, Question)], model: &ModelId) -> String {
    // Serialized straight to text, never through `serde_json::Value`. `Value`'s object
    // type is a `BTreeMap`, so building the digest input as a `Value` sorted each
    // question's fields and its Choice option names before hashing -- and option order
    // is precisely what `jev-core`'s hand-written serializers go out of their way to
    // preserve on the wire. Reordering the options changed what was sent and did not
    // change the digest, so `--resume` accepted a file answered by a different request.
    let mut rendered = String::new();
    rendered.push_str(model.as_str());
    for (id, question) in questions {
        rendered.push(digest::UNIT); // No identifier or instruction can contain it.
        rendered.push_str(id.as_str());
        rendered.push(digest::UNIT);
        // Infallible for any question that exists; a digest that silently changed on an
        // encoding failure would refuse every later resume, so fall back to a constant
        // rather than to something derived from the error.
        rendered.push_str(
            &serde_json::to_string(question).unwrap_or_else(|_| "<unencodable>".to_owned()),
        );
    }
    fnv1a(&rendered)
}

/// The `--require` expression `jev map` classifies each answered row with, if any.
///
/// The raw text travels with the parsed form so that every row can carry the
/// expression it was judged by. A row in a file read days later is worth much less if
/// the reader has to guess which invocation produced it.
pub(crate) struct Classifier {
    raw: String,
    expr: Expr,
}

/// Parses `--require`, and refuses a review file that would collide with the output.
///
/// Both checks happen before a single request is sent. `--require 'x >'` discovered
/// after two hundred billed records would be the worst possible time to find out, and
/// the same path opened twice in append mode gives two independent write offsets on
/// Windows and interleaved rows everywhere.
fn parse_classifier(args: &MapArgs) -> Result<Option<Classifier>> {
    for (other, what) in [
        (args.input.as_deref(), "--input"),
        (args.review_file.as_deref(), "--review-file"),
    ] {
        if let (Some(output), Some(other)) = (args.output_file.as_deref(), other)
            && same_file(output, other)
        {
            return Err(CliError::usage(format!(
                "--output-file and {what} name the same path; \
                 they must be separate files"
            )));
        }
    }

    // Appending to a file that already has rows in it, without being asked to resume,
    // silently doubles it: running the same command twice left every record in the file
    // twice, with no warning, and a later `--resume` then read a file whose rows came
    // from two different runs. Append is the *resume* contract, not the default one.
    if !args.resume {
        // Both files, for the same reason. The review file is the artifact a person
        // opens and greps, so duplicated rows in it are exactly as bad there.
        for (path, what) in [
            (args.output_file.as_deref(), "output"),
            (args.review_file.as_deref(), "review"),
        ] {
            if let Some(path) = path
                && std::fs::metadata(path).is_ok_and(|meta| meta.len() > 0)
            {
                return Err(CliError::usage(format!(
                    "the {what} file {} already has content.\n\n\
                     To start over, remove it. To continue it, pass --resume. To keep \
                     it, name a different file.",
                    path.display()
                )));
            }
        }
    }

    // A review file may also collide with the input, and the input is read fully before
    // either sink opens -- so the damage is the same as for `--output-file`: result rows
    // appended onto a file that is then no longer valid input.
    if let (Some(review), Some(input)) = (args.review_file.as_deref(), args.input.as_deref())
        && same_file(review, input)
    {
        return Err(CliError::usage(
            "--review-file and --input name the same path; they must be separate files",
        ));
    }

    let Some(raw) = args.require.as_deref() else {
        return Ok(None);
    };
    let expr = gate::parse(raw).map_err(|error| {
        CliError::usage(format!(
            "--require: {error}\n\n{}",
            crate::commands::evaluate::gate_help()
        ))
    })?;
    Ok(Some(Classifier {
        raw: raw.to_owned(),
        expr,
    }))
}

/// Runs `jev map`.
pub(crate) fn run(
    session: &mut Session<'_>,
    args: &MapArgs,
    transport: Option<&(dyn Transport + Send + Sync)>,
) -> Result<u8> {
    batch::check_concurrency(args.concurrency, "--concurrency")?;
    let classifier = parse_classifier(args)?;

    let (questions, model) = load_questions(session, args)?;
    let records = read_records(session, args)?;
    if records.is_empty() {
        return Err(CliError::usage(
            "no input records were read; there is nothing to evaluate",
        ));
    }

    // Computed before the resume check, which needs it, and reused on every row. The
    // model here is the one the run will actually send, after `--model` and the request
    // file have been reconciled by `load_questions`.
    let request_fingerprint = request_fingerprint(&questions, &model);

    let done = if args.resume {
        // Both files, because a row this run diverted for review is just as done as one
        // that passed. Reading only the output file would re-evaluate -- and re-bill --
        // every reviewed record on every resume, which is precisely the set a user
        // resuming a long batch has already paid for.
        // Each file is validated on its own *before* the merge. Merging first and
        // validating the result let a good row in one file mask a bad row at the same
        // index in the other: `insert` overwrites, so a review file written from the
        // current request hid an output-file row written from a different one, and the
        // run exited 0 reporting the batch complete while the incompatible row stayed on
        // disk untouched.
        let from_output = read_completed(args.output_file.as_deref())?;
        check_resume_matches(&from_output, &records, &request_fingerprint)?;
        let from_review = read_completed(args.review_file.as_deref())?;
        check_resume_matches(&from_review, &records, &request_fingerprint)?;

        let mut already = from_output;
        already.extend(from_review);
        check_resume_matches(&already, &records, &request_fingerprint)?;
        warn_about_a_review_file(session, &already, args);
        if !already.is_empty() {
            session.warn(&format!(
                "resuming: {} record(s) already succeeded in the output file; \
                 any that failed will be retried",
                already.len()
            ));
        }
        already
    } else {
        BTreeMap::new()
    };

    let pending: Vec<Record> = records
        .into_iter()
        .filter(|record| !done.contains_key(&record.index))
        .collect();

    session.warn_about_endpoint();

    if session.context.dry_run {
        return dry_run(session, &questions, &pending, &model);
    }

    let (credential, source) = session.credential()?;
    session.note(&format!("credential source: {source}"));
    // `warn`, not `note`. ADR-0010 §2 and `docs/threat-model.md` T9 both name this count
    // as one of the controls that stands in for a confirmation prompt on a command whose
    // whole job is sending bulk local content -- and it was written with `note`, which
    // prints only under `--verbose`. A visibility control nobody sees is not a control.
    // It respects `--quiet`, like every other warning, because that is an explicit
    // instruction rather than a default.
    session.warn(&format!(
        "sending {} record(s), {} question(s) each, concurrency {}",
        pending.len(),
        questions.len(),
        args.concurrency
    ));

    // Opened before any request is sent: discovering that the output file is
    // unwritable after spending tokens on two hundred records would be the worst
    // possible time to find out. Both files, for the same reason.
    let sink = Sink {
        primary: match &args.output_file {
            Some(path) => Some(open_row_file(path, "output")?),
            None => None,
        },
        review: match &args.review_file {
            Some(path) => Some(open_row_file(path, "review")?),
            None => None,
        },
    };

    let (outcomes, stopped_early, write_error) = evaluate_all(
        &session.context,
        &interrupt::InterruptibleClock::default(),
        &pending,
        &questions,
        &model,
        &credential,
        args.concurrency,
        args.fail_fast,
        transport,
        &sink,
        classifier.as_ref(),
        &request_fingerprint,
    );

    // A failure to write the output file ends the run as a failure, before the summary.
    // Reporting it as a partial batch instead would be actively misleading: the summary
    // would say nothing failed, and `--resume` against a file that could not be written
    // has nothing to resume from.
    if let Some(reason) = write_error {
        return Err(CliError::io(reason));
    }

    write_results(
        session,
        &sink,
        &outcomes,
        &Totals {
            resumed: done.len(),
            pending: pending.len(),
            stopped_early,
        },
        classifier.as_ref(),
    )
}

/// Loads the question set once. It is the same for every record.
fn load_questions(
    session: &mut Session<'_>,
    args: &MapArgs,
) -> Result<(Vec<(QuestionId, Question)>, ModelId)> {
    let (bytes, origin) = session.reader().read(&args.request, session.stdin)?;
    let text = String::from_utf8(bytes)
        .map_err(|_| CliError::usage(format!("{origin} is not valid UTF-8")))?;
    let document = request::parse_document(&text, &origin.to_string())?;
    if document.state.is_some() {
        session.warn(
            "note: the request document's `state` is ignored by `jev map`; each input \
             record supplies the state",
        );
    }
    // The same precedence `jev ask` applies, and for the same reason: an explicit
    // --model is the most specific statement of intent, and it is what makes a committed
    // request file reusable across models.
    //
    // `map` did not apply it at all. It read `session.context.model` directly, so a
    // request file naming a model was billed on a different one without a word -- while
    // `docs/commands.md` promised the same file works with `ask` and `map` alike. It
    // also made `request_digest` blind to the one edit it exists to catch: changing
    // `model` inside the file moved nothing, so `--resume` accepted a file answered by
    // another model.
    let model = if session.context.model.from == crate::context::Provenance::Flag {
        session.context.model.value.clone()
    } else {
        document
            .model
            .unwrap_or_else(|| session.context.model.value.clone())
    };
    Ok((document.questions, model))
}

/// Reads and validates every input record up front.
fn read_records(session: &mut Session<'_>, args: &MapArgs) -> Result<Vec<Record>> {
    let path = args
        .input
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("-"));
    // The per-source byte ceiling applies to the whole JSONL file here, not to one
    // state, so its message -- "a truncated state produces a confident answer to a
    // question you did not ask" -- describes something that is not happening: there are
    // thousands of separate states, each far inside the per-request budget, and nothing
    // would be truncated. It is also `map`'s real batch ceiling, biting at roughly half
    // a million trivial lines, long before `MAX_RECORDS`. Re-explained in `map`'s own
    // terms, pointing at the same flag.
    let (bytes, origin) = session
        .reader()
        .read(&path, session.stdin)
        .map_err(|error| {
            if error.to_string().contains("byte input limit") {
                CliError::usage(format!(
                    "the whole input is larger than the {} byte limit, which applies to the \
                 batch as a whole and not to one record.\n\n\
                 Raise it with --max-input-bytes, or split the input and run `jev map` \
                 once per part -- the rows carry their own index, so the parts \
                 concatenate.",
                    session.context.max_input_bytes.value
                ))
            } else {
                error
            }
        })?;
    let text = String::from_utf8(bytes)
        .map_err(|_| CliError::usage(format!("{origin} is not valid UTF-8")))?;

    let mut records = Vec::new();
    for (line_number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        if records.len() >= MAX_RECORDS {
            return Err(CliError::usage(format!(
                "more than {MAX_RECORDS} input records; split the input"
            )));
        }
        let index = records.len();
        let where_ = format!("{origin} line {}", line_number + 1);

        let (state, id) =
            if args.lines {
                let state = State::text(line.to_owned())
                    .map_err(|_| CliError::usage(format!("{where_} is empty")))?;
                (state, index.to_string())
            } else {
                let value: Value = serde_json::from_str(line).map_err(|error| {
                    CliError::usage(format!(
                        "{where_} is not valid JSON: {error}\n\n\
                     Use --lines to treat each line as plain text instead."
                    ))
                })?;
                let id = match &args.id_field {
                    Some(field) => value.get(field).map(render_id).ok_or_else(|| {
                        CliError::usage(format!("{where_} has no `{field}` field"))
                    })?,
                    None => index.to_string(),
                };
                let state_value = match &args.state_field {
                    Some(field) => value.get(field).cloned().ok_or_else(|| {
                        CliError::usage(format!("{where_} has no `{field}` field"))
                    })?,
                    None => value,
                };
                let content = Content::try_from(state_value)
                    .map_err(|error| CliError::usage(format!("{where_}: {error}")))?;
                (State::new(content), id)
            };

        records.push(Record::new(index, id, state));
    }
    Ok(records)
}

pub(crate) fn render_id(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// What a successful row from an earlier run says about the record at its index.
///
/// Both fields are `None` for a row written by a version that did not record them, and
/// for a hand-edited row that omits them or sets them to `null`. A comparison that has
/// nothing to compare is skipped rather than failed, so an older output file still
/// resumes.
struct CompletedRow {
    /// The row's `id`, which is the record's position unless `--id-field` was given.
    id: Option<String>,
    /// The digest of the state that was evaluated. See [`state_digest`].
    digest: Option<String>,
    /// The digest of the question set and model. See [`request_digest`].
    request: Option<String>,
    /// Whether the run that wrote this row was classifying with `--require`.
    ///
    /// Read back so a resume can notice that an earlier run may have diverted records
    /// into a review file this invocation was not told about. See [`warn_about_a_review_file`].
    classified: bool,
}

/// Reads the rows that **succeeded** in an earlier run, by index.
///
/// Only the index and the `ok` flag are read back — never an answer. That is what keeps
/// `--resume` from being a response cache: a record that is not listed as succeeded is
/// evaluated fresh, and one that is is left exactly as it was written.
///
/// A row with `"ok": false` is deliberately **not** treated as done. Skipping failures
/// would mean a resumed run never retries the records that failed, reports
/// `"failed": 0`, and exits `0` — turning a partial run into an apparently complete one.
fn read_completed(path: Option<&Path>) -> Result<BTreeMap<usize, CompletedRow>> {
    let Some(path) = path else {
        return Ok(BTreeMap::new());
    };
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BTreeMap::new());
        }
        Err(error) => {
            return Err(CliError::usage(format!(
                "cannot read the output file to resume: {}",
                crate::input::io_reason(&error)
            )));
        }
    };
    let mut done = BTreeMap::new();
    for line in std::io::BufReader::new(file).lines() {
        let line = line.map_err(|error| {
            CliError::usage(format!(
                "cannot read the output file: {}",
                crate::input::io_reason(&error)
            ))
        })?;
        if line.trim().is_empty() {
            continue;
        }
        // A truncated final line — the shape a killed run leaves — is skipped rather
        // than treated as done, so an interrupted record is retried.
        if let Ok(value) = serde_json::from_str::<Value>(&line) {
            if value.get("ok").and_then(Value::as_bool) != Some(true) {
                continue;
            }
            if let Some(index) = value.get("index").and_then(Value::as_u64)
                && let Ok(index) = usize::try_from(index)
            {
                // Kept so the caller can check that this row really describes the
                // record now sitting at that position. See `check_resume_matches`.
                // An explicit `"id": null` is treated as absent rather than as the
                // string "null", which would refuse a hand-edited file spuriously.
                let field = |name: &str| {
                    value
                        .get(name)
                        .filter(|found| !found.is_null())
                        .map(render_id)
                };
                done.insert(
                    index,
                    CompletedRow {
                        id: field("id"),
                        digest: field("state_digest"),
                        request: field("request_digest"),
                        classified: value.get("gate").is_some_and(|gate| !gate.is_null()),
                    },
                );
            }
        }
    }
    Ok(done)
}

/// Warns when a resume may be missing records an earlier run set aside for review.
///
/// `--resume` reads the output file and the review file. Drop `--review-file` from an
/// otherwise identical command and the diverted records look unevaluated: they are sent
/// again, billed again, and written to the output file -- so the same index ends up in
/// both files, `cat`ing them together yields duplicates, and nothing reports that it
/// happened. A later resume with both flags papers over it, because the merge
/// de-duplicates.
///
/// There is no honest way to *refuse* this: a run that used `--require` without ever
/// naming a review file is perfectly normal and has nothing missing. What the rows do
/// say is whether the earlier run was classifying at all, which is the precondition for
/// the mistake -- so that is what this warns on, and only then.
fn warn_about_a_review_file(
    session: &mut Session<'_>,
    done: &BTreeMap<usize, CompletedRow>,
    args: &MapArgs,
) {
    if args.review_file.is_some() {
        return;
    }
    if done.values().any(|row| row.classified) {
        session.warn(
            "the output file was written by a run using --require. If that run also \
             used --review-file, pass the same path now: records it set aside are not \
             in this file, and will be evaluated and billed again",
        );
    }
}

/// Refuses to resume against an output file produced from different input.
///
/// `--resume` matches on the record's position in the input, which is the only key a
/// run without `--id-field` has. That is safe when the input is the same file it was
/// last time -- and silently wrong when it is not. Editing, filtering, or re-sorting
/// the input between a crash and the retry is the normal thing to do, and it made `jev`
/// skip records that had never been evaluated while reporting the batch complete at
/// exit `0`.
///
/// Two things are compared, because one is not enough:
///
/// * the row's `id`, which catches a changed input when `--id-field` names a real key;
/// * the row's `state_digest`, which catches it otherwise -- without `--id-field` the
///   `id` *is* the position, so comparing ids alone is a tautology and the guard could
///   never fire in the configuration nearly every run uses.
///
/// A row that carries neither -- one written before these were recorded -- has nothing
/// to compare and is accepted, so an older output file still resumes.
///
/// An index the current input cannot contain is the same evidence of a different input,
/// and is refused for the same reason: counted as `resumed`, it inflated `total` and
/// `complete` and reported a shrunken batch as finished.
fn check_resume_matches(
    done: &BTreeMap<usize, CompletedRow>,
    records: &[Record],
    request: &str,
) -> Result<()> {
    let refuse = |index: usize, now: &str, before: &str, what: &str| {
        CliError::usage(format!(
            "the output file was produced from different input: record {index} has {what} \
             `{}` now but `{}` in the file.\n\n\
             Resume only against the input the file was written from, or drop --resume \
             to evaluate every record again.",
            crate::output::Safe::new(now),
            crate::output::Safe::new(before),
        ))
    };

    for (index, row) in done {
        // Checked first, and reported differently: a changed question set is not a
        // changed input, and telling the user to "resume only against the input the
        // file was written from" would send them to look at the wrong file.
        if let Some(recorded) = &row.request
            && recorded != request
        {
            return Err(CliError::usage(format!(
                "the output file was produced from a different request: record {index} \
                 was answered with another question set or model.\n\n\
                 Resume only with the request the file was written from, or drop \
                 --resume to evaluate every record again."
            )));
        }
        let Some(record) = records.get(*index) else {
            return Err(CliError::usage(format!(
                "the output file was produced from different input: it holds a result \
                 for record {index}, but this input has only {} record(s).\n\n\
                 Resume only against the input the file was written from, or drop \
                 --resume to evaluate every record again.",
                records.len()
            )));
        };
        if let Some(recorded) = &row.id
            && *recorded != record.id
        {
            return Err(refuse(*index, &record.id, recorded, "id"));
        }
        if let Some(recorded) = &row.digest
            && *recorded != record.digest
        {
            return Err(refuse(*index, &record.digest, recorded, "state digest"));
        }
    }
    Ok(())
}

/// Which stream a completed row belongs to.
///
/// Only a row that was *answered* and then classified by `--require` can be
/// [`Route::Review`]. A row that failed is never diverted: "the API did not answer" and
/// "the API answered and the answer needs a look" are the distinction this whole CLI is
/// built around, and merging them into one file would undo it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    /// The main stream: stdout, or `--output-file`.
    Primary,
    /// The `--review-file` stream, when there is one.
    Review,
}

/// One record's result.
pub(crate) struct Outcome {
    index: usize,
    /// The `jev.map.row/v1` document, exactly as the CLI writes it.
    pub(crate) document: Value,
    ok: bool,
    /// The credential was rejected. Per-credential, not per-record: every remaining
    /// row would fail the same way, so the batch stops and reports exit 3.
    auth_failed: bool,
    route: Route,
    /// The gate verdict, for the summary counts. `None` without `--require`, and for a
    /// row that failed before there was an answer to classify.
    gate: Option<GateOutcome>,
}

/// One file rows are appended to, and what to call it in an error.
struct RowFile {
    writer: Mutex<BufWriter<std::fs::File>>,
    /// `output` or `review`, so a failure names the file the user actually has to fix.
    label: &'static str,
}

impl RowFile {
    fn write(&self, document: &Value) -> Result<()> {
        let mut writer = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        write_line(&mut *writer, document, self.label)?;
        // Flushed per row, deliberately. Buffering would undo the point: the reason to
        // write as we go is that the process might not reach the end.
        writer.flush().map_err(|error| {
            // `io`, not `internal`: a full disk, a revoked permission, or a vanished
            // mount is the environment's problem, and telling the user they have found
            // a bug in `jev` sends them to the issue tracker for nothing.
            CliError::io(format!(
                "cannot write the {} file: {}",
                self.label,
                crate::input::io_reason(&error)
            ))
        })
    }
}

/// Where completed rows go while a batch is running.
///
/// With an output file, a row is written and flushed the moment it is done, so a run
/// that is killed outright still leaves every finished record on disk. Without one,
/// rows are collected and printed in input order at the end.
pub(crate) struct Sink {
    /// The main stream. `None` means stdout, which is buffered until the end so that
    /// rows can be printed in input order.
    primary: Option<RowFile>,
    /// `--review-file`, when the user named one.
    review: Option<RowFile>,
}

impl Outcome {
    /// Whether the API answered this record.
    pub(crate) const fn succeeded(&self) -> bool {
        self.ok
    }

    /// Whether this record failed because the credential was rejected.
    pub(crate) const fn auth_failed(&self) -> bool {
        self.auth_failed
    }
}

impl Sink {
    /// A sink with no files: every row is kept in memory and handed back to the caller.
    pub(crate) const fn in_memory() -> Self {
        Self {
            primary: None,
            review: None,
        }
    }

    /// Records a completed row, writing it through if its destination is a file.
    ///
    /// A [`Route::Review`] row with no `--review-file` falls through to the primary
    /// stream. Nothing is ever dropped: a row the user cannot see is a row they paid
    /// for and will never know about.
    fn record(&self, outcome: &Outcome) -> Result<()> {
        match (outcome.route, self.review.as_ref()) {
            (Route::Review, Some(review)) => review.write(&outcome.document),
            _ => match self.primary.as_ref() {
                Some(primary) => primary.write(&outcome.document),
                None => Ok(()),
            },
        }
    }

    /// Whether a [`Route::Review`] row has already been written elsewhere.
    const fn diverts(&self) -> bool {
        self.review.is_some()
    }
}

/// Evaluates every pending record, with bounded concurrency.
///
/// The threading itself lives in [`crate::batch`], shared with `jev eval`. What stays
/// here is what is specific to `map`: writing each row through as it completes, and
/// deciding which failures end the run rather than the row.
#[allow(
    clippy::too_many_arguments,
    reason = "each argument is an independent input to the batch; grouping them would \
              only move the list"
)]
pub(crate) fn evaluate_all(
    context: &crate::context::Context,
    clock: &interrupt::InterruptibleClock,
    records: &[Record],
    questions: &[(QuestionId, Question)],
    model: &ModelId,
    credential: &Credential,
    concurrency: usize,
    fail_fast: bool,
    transport: Option<&(dyn Transport + Send + Sync)>,
    sink: &Sink,
    classifier: Option<&Classifier>,
    request_fingerprint: &str,
) -> (Vec<Outcome>, bool, Option<String>) {
    // The *reason*, not a flag. A boolean here was dropped on the floor by the caller,
    // and the run reported a partial batch in which nothing had failed.
    let write_error: Mutex<Option<String>> = Mutex::new(None);

    let plan = batch::Plan {
        endpoint: &context.endpoint.value,
        retry: context.retry,
        timeout: context.timeout.value,
        concurrency,
        clock: clock.clone(),
    };

    let (mut outcomes, stopped_early) = batch::each(&plan, records, transport, |client, record| {
        let outcome = evaluate_one(
            client,
            record,
            questions,
            model,
            credential,
            classifier,
            request_fingerprint,
        );
        let failed = !outcome.ok;
        // A rejected credential is not a property of this record. Carrying on would
        // send one doomed request per remaining row -- up to a million of them -- and
        // then report the result as a partial batch, so a CI job branching on exit 3 to
        // re-authenticate never saw it.
        let auth_failed = outcome.auth_failed;
        // Written through before it is counted, so a killed process leaves the row on
        // disk. A write failure stops the run rather than continuing to spend tokens on
        // results nobody can read, and the unwritten row is discarded rather than
        // counted as delivered.
        if let Err(error) = sink.record(&outcome) {
            let mut slot = write_error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            slot.get_or_insert_with(|| error.to_string());
            return batch::Step::Abort;
        }
        if auth_failed || (failed && fail_fast) {
            batch::Step::Last(outcome)
        } else {
            batch::Step::Continue(outcome)
        }
    });

    // Sorted for the summary and for the in-memory sink. A file sink has already
    // written the rows in completion order; every row carries its `index`, so `sort`
    // recovers input order if a consumer wants it.
    outcomes.sort_by_key(|outcome| outcome.index);
    let write_error = write_error
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    (outcomes, stopped_early, write_error)
}

fn evaluate_one(
    client: &batch::Worker<'_>,
    record: &Record,
    questions: &[(QuestionId, Question)],
    model: &ModelId,
    credential: &Credential,
    classifier: Option<&Classifier>,
    request_fingerprint: &str,
) -> Outcome {
    let request =
        match EvaluationRequest::new(record.state.clone(), model.clone(), questions.to_vec()) {
            Ok(request) => request,
            // No request was made, so there is no identifier to report.
            Err(error) => {
                return failure(
                    record,
                    &error.to_string(),
                    "invalid-request",
                    0,
                    None,
                    classifier,
                    request_fingerprint,
                );
            }
        };

    let (result, stats) = client.evaluate(&request, credential);
    match result {
        Ok(response) => {
            let mut answers = serde_json::Map::new();
            for (id, answer) in &response.answers {
                answers.insert(id.as_str().to_owned(), render_json::answer_value(answer));
            }
            // Classified here, in the worker, against the response that is about to be
            // discarded. Deciding later from the rendered row would mean re-parsing
            // JSON to recover types the gate already understands.
            let verdict = classifier.map(|c| gate::evaluate(&c.expr, &response));
            let missing = response.missing(questions.iter().map(|(id, _)| id));
            let mut outcome = Outcome {
                index: record.index,
                ok: true,
                auth_failed: false,
                // Only a `Passed` row stays in the main stream. `Failed` and
                // `Unevaluable` both go for review, for the reason `exit.rs` gives for
                // keeping exit 1 and exit 6 apart: a gate that could not be evaluated
                // must never be treated as one that passed. The row records which it
                // was, so the two are still distinguishable downstream.
                route: match verdict {
                    None | Some(GateOutcome::Passed) => Route::Primary,
                    Some(_) => Route::Review,
                },
                document: json!({
                    "schema": MAP_ROW_SCHEMA,
                    "index": record.index,
                    "id": record.id,
                    "state_digest": record.digest,
                    "request_digest": request_fingerprint,
                    "ok": true,
                    "model": response.model.as_str(),
                    "answers": Value::Object(answers),
                    "usage": {
                        "input_tokens": response.usage.input_tokens,
                        "output_tokens": response.usage.output_tokens,
                    },
                    "attempts": stats.attempts,
                    "request_id": stats.request_id,
                    "gate": gate_value(classifier, verdict.as_ref()),
                }),
                gate: verdict,
            };
            // As in `jev.evaluation/v1`: present only when the API skipped a question,
            // so a row without it is complete and a row with it says which are absent.
            if !missing.is_empty()
                && let Some(row) = outcome.document.as_object_mut()
            {
                row.insert("missing_answers".to_owned(), json!(missing));
            }
            outcome
        }
        Err(error) => {
            // The same split the single-request path makes (`errors.rs`): a response
            // this version cannot decode is the API saying something unreadable, not
            // the user's command being wrong. Reporting it as `request` told a
            // consumer that retries `unavailable` and gives up on `request` to give up
            // on a transient API fault.
            let kind = if error.is_auth() {
                "auth"
            } else if error.is_unavailable()
                || matches!(error, ClientError::MalformedResponse { .. })
            {
                "unavailable"
            } else {
                "request"
            };
            failure(
                record,
                &error.to_string(),
                kind,
                stats.attempts,
                stats.request_id.as_deref(),
                classifier,
                request_fingerprint,
            )
        }
    }
}

/// The `gate` field of a row: the expression and what it said, or `null`.
///
/// Always present as a key, `null` without `--require`. A consumer that filters on
/// `.gate.outcome` then sees a missing verdict rather than a missing field.
fn gate_value(classifier: Option<&Classifier>, verdict: Option<&GateOutcome>) -> Value {
    let Some(classifier) = classifier else {
        return Value::Null;
    };
    let (outcome, reason) = match verdict {
        Some(GateOutcome::Passed) => ("passed", None),
        Some(GateOutcome::Failed) => ("failed", None),
        Some(GateOutcome::Unevaluable { reason }) => ("unevaluable", Some(reason.clone())),
        // The row failed before there was an answer to classify.
        None => ("not-evaluated", None),
    };
    json!({
        "expression": classifier.raw,
        "outcome": outcome,
        // Sanitized: an unevaluable reason quotes the question id, which came from the
        // user's request file.
        "reason": reason.map(|reason| crate::output::sanitize(&reason)),
    })
}

fn failure(
    record: &Record,
    message: &str,
    kind: &str,
    attempts: u32,
    request_id: Option<&str>,
    classifier: Option<&Classifier>,
    request_fingerprint: &str,
) -> Outcome {
    Outcome {
        index: record.index,
        ok: false,
        auth_failed: kind == "auth",
        // A failed row is never diverted. "The API did not answer" and "the API
        // answered and the answer needs a look" are different problems for different
        // people, and the review file is worth much less if it is also the error log.
        route: Route::Primary,
        gate: None,
        document: json!({
            "schema": MAP_ROW_SCHEMA,
            "index": record.index,
            "id": record.id,
            "state_digest": record.digest,
            "request_digest": request_fingerprint,
            "ok": false,
            // Every attempt may have reached the server and been billed, a timed-out
            // one included, so a failed row is where the count matters most. `0` means
            // no request was made.
            "attempts": attempts,
            // The API's own identifier for the call, when it sent one. It is the
            // only thing TypeSafe support can use to find a specific failure, and a
            // row that failed is exactly when a user needs it.
            "request_id": request_id,
            "gate": gate_value(classifier, None),
            "error": {
                "kind": kind,
                // Sanitized here rather than at render time: this string can come from
                // an API error body and will be read by a person as well as a script.
                "message": crate::output::sanitize(message),
            },
        }),
    }
}

/// Writes every row, then the summary, and returns the exit code.
/// The counts behind the summary line.
pub(crate) struct Totals {
    /// Rows already present, and successful, in the output file.
    pub(crate) resumed: usize,
    /// Rows this run set out to evaluate.
    pub(crate) pending: usize,
    /// Whether the run stopped before reaching every pending row.
    pub(crate) stopped_early: bool,
}

/// The `gate` field of the summary: how the classified rows came out.
///
/// `null` without `--require`, so a consumer can tell "no expression was given" from
/// "an expression was given and nothing passed".
///
/// The counts cover the rows **this run answered**. A resumed row is not re-classified,
/// because it is not re-evaluated -- that is the whole point of `--resume` -- so
/// counting it here would mean reading a verdict back out of a file and reporting it as
/// though this run had reached it.
fn gate_counts(classifier: Option<&Classifier>, outcomes: &[Outcome]) -> Value {
    let Some(classifier) = classifier else {
        return Value::Null;
    };
    let count = |matches: fn(&GateOutcome) -> bool| {
        outcomes
            .iter()
            .filter(|outcome| outcome.gate.as_ref().is_some_and(matches))
            .count()
    };
    json!({
        "expression": classifier.raw,
        "passed": count(|outcome| matches!(outcome, GateOutcome::Passed)),
        "failed": count(|outcome| matches!(outcome, GateOutcome::Failed)),
        "unevaluable": count(|outcome| matches!(outcome, GateOutcome::Unevaluable { .. })),
    })
}

/// Writes any buffered rows, then the summary, and returns the exit code.
fn write_results(
    session: &mut Session<'_>,
    sink: &Sink,
    outcomes: &[Outcome],
    totals: &Totals,
    classifier: Option<&Classifier>,
) -> Result<u8> {
    let succeeded = outcomes.iter().filter(|outcome| outcome.ok).count();
    let failed = outcomes.len() - succeeded;
    let interrupted = interrupt::requested();
    let stopped_early = totals.stopped_early || outcomes.len() < totals.pending;
    let summary = summary(outcomes, totals, classifier, interrupted);

    // Rows that went to a file were written and flushed as they completed. What is left
    // to print is whatever was destined for stdout and buffered. The summary goes to
    // stdout in every case, and never into a row file: a summary line among the rows
    // would break `--resume` and confuse a JSONL consumer.
    if sink.primary.is_none() {
        for outcome in outcomes {
            // A diverted row is already in the review file. Printing it here as well
            // would duplicate it, and put a row the user asked to set aside back into
            // the stream they are piping onward.
            if outcome.route == Route::Review && sink.diverts() {
                continue;
            }
            render_json::write_document(session.out, &outcome.document)?;
        }
    }
    render_json::write_document(session.out, &summary)?;

    if sink.diverts() {
        let diverted = outcomes
            .iter()
            .filter(|outcome| outcome.route == Route::Review)
            .count();
        if diverted > 0 {
            session.warn(&format!(
                "{diverted} of {succeeded} answered record(s) did not pass --require \
                 and were written to the review file"
            ));
        }
    }

    if interrupted {
        session.warn(&format!(
            "interrupted after {} record(s); rerun with --resume to continue",
            outcomes.len()
        ));
        return Ok(exit::INTERRUPTED);
    }
    // Checked before the partial-batch code, because it is the more specific fact: the
    // credential is wrong, and no amount of retrying these records will change that.
    // `docs/cli-contract.md` defines 3 as "authentication failed" and 5 as a batch in
    // which some rows failed -- a data problem. This is not a data problem.
    if outcomes.iter().any(|outcome| outcome.auth_failed) {
        session.warn(
            "the API rejected the credential; the batch stopped rather than sending a \
             request for every remaining record",
        );
        return Ok(exit::AUTH);
    }
    if failed > 0 {
        session.warn(&format!("{failed} of {} record(s) failed", outcomes.len()));
        return Ok(exit::PARTIAL);
    }
    if stopped_early {
        session.warn(&format!(
            "stopped after {} of {} record(s); rerun with --resume to continue",
            outcomes.len(),
            totals.pending
        ));
        return Ok(exit::PARTIAL);
    }
    Ok(exit::SUCCESS)
}

/// The `jev.map.summary/v1` document for a finished run.
///
/// Shared by the CLI's summary line and the MCP `map` tool, so the two cannot count
/// differently.
pub(crate) fn summary(
    outcomes: &[Outcome],
    totals: &Totals,
    classifier: Option<&Classifier>,
    interrupted: bool,
) -> Value {
    let succeeded = outcomes.iter().filter(|outcome| outcome.ok).count();
    let failed = outcomes.len() - succeeded;
    // `stopped_early` covers both causes, so a truncated run is distinguishable from a
    // complete one over fewer records — which `failed > 0` alone could not do.
    let stopped_early = totals.stopped_early || outcomes.len() < totals.pending;
    json!({
        "schema": MAP_SUMMARY_SCHEMA,
        // Every record this run considered, including the ones it skipped.
        "total": totals.pending + totals.resumed,
        // Records this run actually sent.
        "evaluated": outcomes.len(),
        // Records skipped because they already succeeded in the output file.
        "resumed": totals.resumed,
        // Successes *this run*; add `resumed` for the number of good rows in the file.
        "succeeded": succeeded,
        "failed": failed,
        // Good rows in the output file after this run: the number to compare against
        // `total` to ask "is everything done?".
        "complete": succeeded + totals.resumed,
        "stopped_early": stopped_early,
        "interrupted": interrupted,
        "gate": gate_counts(classifier, outcomes),
    })
}

/// Opens a file rows are appended to, closing any partial final line first.
///
/// `label` is `output` or `review`, and names the file in every error this can raise:
/// with two of them open, "cannot open the output file" pointing at the review file
/// would send the user to fix the wrong path.
fn open_row_file(path: &Path, label: &'static str) -> Result<RowFile> {
    let mut options = std::fs::OpenOptions::new();
    options
        .create(true)
        .append(true)
        // Read access as well, so `terminate_partial_line` can look at the last byte.
        // Append mode still forces every write to the end, so this cannot turn into an
        // overwrite.
        .read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // Rows hold the model's answers about the user's state, which
        // `docs/threat-model.md` lists among the assets worth protecting -- the same
        // reasoning that makes the configuration file 0600. Left to the umask this was
        // typically 0644 or 0664, readable by anyone on a shared machine or a CI
        // runner. The review file holds the same answers and gets the same treatment.
        //
        // `mode` applies only when this call creates the file. Appending to a file the
        // user already made keeps their permissions, which is right: the mode of a file
        // they created is their decision, not ours.
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| {
        CliError::usage(format!(
            "cannot open the {label} file: {}",
            crate::input::io_reason(&error)
        ))
    })?;
    terminate_partial_line(&mut file, path, label)?;
    Ok(RowFile {
        writer: Mutex::new(BufWriter::new(file)),
        label,
    })
}

fn write_line(writer: &mut impl Write, value: &Value, label: &str) -> Result<()> {
    // The same encoder stdout uses, so a row written to a file carries the same
    // escaping. Writing raw bytes here meant a bidirectional override or a zero-width
    // character from an API response survived into the output file -- the artifact most
    // likely to be read in a terminal, grepped, or diffed long after the run.
    let bytes = render_json::encode_line(value)?;
    writer.write_all(&bytes).map_err(|error| {
        CliError::io(format!(
            "cannot write the {label} file: {}",
            crate::input::io_reason(&error)
        ))
    })
}

/// Closes a partial final line before anything is appended to an existing output file.
///
/// `read_completed` deliberately tolerates a truncated last line — a `SIGKILL` or a
/// power cut can leave one, and the row it represents is re-run rather than trusted.
/// The write side did not match: opening in append mode and writing the next row
/// concatenated it onto that partial line, producing a line that parses as neither
/// row. The re-run record was billed, answered, and then lost, while the summary
/// reported the batch complete and exited `0`.
///
/// Writing the missing newline first is the whole fix: the partial line stays
/// unparsable and is skipped by the next `--resume`, and every row written afterwards
/// is a line of its own.
fn terminate_partial_line(file: &mut std::fs::File, path: &Path, label: &str) -> Result<()> {
    use std::io::{Read as _, Seek as _, SeekFrom, Write as _};

    let length = file
        .metadata()
        .map_err(|error| {
            CliError::usage(format!(
                "cannot inspect the {label} file: {}",
                crate::input::io_reason(&error)
            ))
        })?
        .len();
    let Some(last) = length.checked_sub(1) else {
        return Ok(()); // A new or empty file has no partial line.
    };

    // The handle is in append mode, so seeking affects reads only; the write below
    // still lands at the end, which is exactly what is wanted here.
    let mut byte = [0_u8; 1];
    file.seek(SeekFrom::Start(last))
        .and_then(|_| file.read_exact(&mut byte))
        .map_err(|error| {
            CliError::usage(format!(
                "cannot read the end of the {label} file: {}",
                crate::input::io_reason(&error)
            ))
        })?;
    if byte[0] == b'\n' {
        return Ok(());
    }

    file.write_all(b"\n").map_err(|error| {
        CliError::io(format!(
            "cannot repair the {label} file {}: {}",
            path.display(),
            crate::input::io_reason(&error)
        ))
    })
}

/// Shows what would be sent for the first few records, and sends nothing.
fn dry_run(
    session: &mut Session<'_>,
    questions: &[(QuestionId, Question)],
    records: &[Record],
    model: &ModelId,
) -> Result<u8> {
    // Three, because a preview is for confirming the shape, not for reading the data;
    // `records` is the honest total and `sample_truncated` says plainly that what is
    // shown is not all of it.
    const SAMPLE_LIMIT: usize = 3;

    let mut bodies = Vec::new();
    let mut url = None;
    let mut headers: Vec<String> = Vec::new();
    for record in records.iter().take(SAMPLE_LIMIT) {
        let request =
            EvaluationRequest::new(record.state.clone(), model.clone(), questions.to_vec())
                .map_err(|error| CliError::usage(error.to_string()))?;
        // The same builder a real row uses, so a preview cannot describe a request the
        // batch would not send. See `evaluate::dry_run` for the reasoning.
        let built = jev_client::build_evaluation_request(&session.context.endpoint.value, &request)
            .map_err(|error| CliError::internal(error.to_string()))?;
        let body: Value = serde_json::from_slice(&built.body)
            .map_err(|error| CliError::internal(error.to_string()))?;
        if url.is_none() {
            url = Some(built.url.clone());
            headers = built.headers.keys().cloned().collect();
            headers.push("authorization".to_owned());
            headers.sort_unstable();
        }
        bodies.push(json!({
            "index": record.index,
            "id": record.id,
            "body": body,
            "body_bytes": built.body.len(),
        }));
    }
    let credential = session.credential_availability();
    let url = url.unwrap_or_else(|| {
        session
            .context
            .endpoint
            .value
            .url_for(jev_client::SYSTEM_ONE_PATH)
    });
    let record_count = records.len();
    render_json::write_document(
        session.out,
        &json!({
            "schema": crate::render::DRY_RUN_SCHEMA,
            "method": "POST",
            // With no records there is no request to build, so the URL is the one the
            // endpoint resolves to rather than one taken from a sample that is absent.
            "url": url,
            "headers": headers,
            "records": record_count,
            "questions": questions.len(),
            "sample": bodies,
            "sample_truncated": record_count > SAMPLE_LIMIT,
            "credential": credential,
            "sent": false,
        }),
    )?;
    session.warn(&format!(
        "dry run: nothing was sent; {} record(s) would be evaluated",
        records.len()
    ));
    Ok(exit::SUCCESS)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    use std::collections::BTreeSet;

    fn completed_rows(contents: &str) -> BTreeMap<usize, CompletedRow> {
        let mut file = tempfile::NamedTempFile::new().expect("temp file");
        file.write_all(contents.as_bytes()).expect("write");
        file.flush().expect("flush");
        read_completed(Some(file.path())).expect("read")
    }

    /// Just the indexes, for the tests that predate ids being read back.
    fn completed_from(contents: &str) -> BTreeSet<usize> {
        completed_rows(contents).into_keys().collect()
    }

    fn record_at(index: usize, id: &str) -> Record {
        let state =
            State::new(Content::try_from(Value::String(format!("s{index}"))).expect("content"));
        let digest = state_digest(&state);
        Record {
            index,
            id: id.to_owned(),
            state,
            digest,
        }
    }

    /// Resuming against a file written from *different* input used to skip records
    /// that had never been evaluated and then report the batch complete at exit 0.
    #[test]
    fn resuming_against_a_changed_id_is_refused() {
        let rows = completed_rows(
            "{\"index\":0,\"id\":\"A\",\"ok\":true}\n             {\"index\":1,\"id\":\"B\",\"ok\":true}\n",
        );
        let error = check_resume_matches(&rows, &[record_at(0, "A"), record_at(1, "X")], "same")
            .expect_err("a changed record must be refused");
        assert!(
            error.to_string().contains("different input"),
            "unhelpful message: {error}"
        );

        check_resume_matches(&rows, &[record_at(0, "A"), record_at(1, "B")], "same")
            .expect("unchanged input must still resume");
    }

    /// The case the id comparison alone cannot catch, and the one nearly every run is
    /// in: without `--id-field` the row's `id` *is* the record's position, so comparing
    /// ids is a tautology and the guard could never fire.
    #[test]
    fn resuming_against_changed_state_is_refused_even_without_an_id_field() {
        let first = record_at(0, "0");
        let rows = completed_rows(&format!(
            "{{\"index\":0,\"id\":\"0\",\"state_digest\":\"{}\",\"ok\":true}}\n",
            first.digest
        ));

        check_resume_matches(&rows, std::slice::from_ref(&first), "same")
            .expect("the same state must still resume");

        // Same position, same positional id, different content.
        let mut changed = record_at(9, "0");
        changed.index = 0;
        let error = check_resume_matches(&rows, &[changed], "same")
            .expect_err("a changed state at the same position must be refused");
        assert!(
            error.to_string().contains("state digest"),
            "the digest mismatch was not what was reported: {error}"
        );
    }

    /// A result for a record the input cannot contain is the same evidence of a
    /// different input -- and, counted as `resumed`, it inflated `total` and `complete`
    /// so a shrunken batch reported itself finished.
    #[test]
    fn resuming_against_an_index_past_the_end_of_the_input_is_refused() {
        let rows = completed_rows("{\"index\":9,\"id\":\"9\",\"ok\":true}\n");
        let error = check_resume_matches(&rows, &[record_at(0, "0")], "same")
            .expect_err("an index the input cannot contain must be refused");
        assert!(
            error.to_string().contains("only 1 record"),
            "unhelpful message: {error}"
        );
    }

    /// A row written before ids and digests were recorded has nothing to compare, and
    /// must not become an error for everyone holding an older output file. An explicit
    /// `null` counts as absent, not as the string "null".
    #[test]
    fn a_row_without_an_id_or_digest_does_not_block_a_resume() {
        for line in [
            "{\"index\":0,\"ok\":true}\n",
            "{\"index\":0,\"id\":null,\"state_digest\":null,\"ok\":true}\n",
        ] {
            let rows = completed_rows(line);
            check_resume_matches(&rows, &[record_at(0, "anything")], "same")
                .unwrap_or_else(|error| panic!("{line} should resume, but: {error}"));
        }
    }

    #[test]
    fn no_output_file_means_nothing_is_complete() {
        assert!(read_completed(None).expect("no path").is_empty());
    }

    /// Resuming against a file that does not exist yet is the first run, not an error.
    #[test]
    fn a_missing_output_file_means_nothing_is_complete() {
        let directory = tempfile::tempdir().expect("temp dir");
        let absent = directory.path().join("not-written-yet.jsonl");
        assert!(read_completed(Some(&absent)).expect("absent").is_empty());
    }

    /// The shape a killed run leaves. A truncated final line must be retried, not
    /// treated as done — treating it as done is how `--resume` silently drops a record.
    #[test]
    fn a_truncated_final_line_is_not_counted_as_complete() {
        let done = completed_from(
            "{\"index\":0,\"ok\":true}\n\
             {\"index\":1,\"ok\":true}\n\
             {\"index\":2,\"ok\":tr",
        );
        assert_eq!(done, BTreeSet::from([0, 1]));
    }

    #[test]
    fn blank_lines_are_ignored() {
        let done =
            completed_from("\n{\"index\":3,\"ok\":true}\n\n   \n{\"index\":4,\"ok\":true}\n");
        assert_eq!(done, BTreeSet::from([3, 4]));
    }

    /// A set, so a row written twice — a resumed run re-appending, say — counts once.
    #[test]
    fn a_duplicate_row_counts_once() {
        let done = completed_from(
            "{\"index\":7,\"ok\":true}\n\
             {\"index\":7,\"ok\":true}\n",
        );
        assert_eq!(done, BTreeSet::from([7]));
    }

    /// Only `ok: true` counts. A failed row must be retried, which is the bug this
    /// guards: reading "it is in the file" as "it succeeded" made `--resume` report a
    /// clean run over records that had all errored.
    #[test]
    fn a_failed_row_is_not_complete() {
        let done = completed_from(
            "{\"index\":0,\"ok\":true}\n\
             {\"index\":1,\"ok\":false,\"error\":\"boom\"}\n\
             {\"index\":2}\n\
             {\"index\":3,\"ok\":\"true\"}\n",
        );
        assert_eq!(
            done,
            BTreeSet::from([0]),
            "a row that did not succeed was treated as done"
        );
    }

    #[test]
    fn a_row_without_a_usable_index_is_ignored() {
        let done = completed_from(
            "{\"ok\":true}\n\
             {\"index\":-1,\"ok\":true}\n\
             {\"index\":1.5,\"ok\":true}\n\
             {\"index\":\"2\",\"ok\":true}\n\
             {\"index\":5,\"ok\":true}\n",
        );
        assert_eq!(done, BTreeSet::from([5]));
    }

    #[test]
    fn a_line_that_is_not_json_at_all_is_ignored() {
        let done = completed_from("not json\n{\"index\":2,\"ok\":true}\nalso not json\n");
        assert_eq!(done, BTreeSet::from([2]));
    }

    // --- The concurrency bound ------------------------------------------------------

    /// `-j 0` would spawn no workers and hang; `-j 65` is past the cap. Both ends are
    /// rejected, and the two just inside are accepted.
    ///
    /// The range itself is `crate::batch`'s, shared with `jev eval`; what this pins is
    /// that `map` still applies it, and still applies it to `--concurrency`.
    #[test]
    fn the_concurrency_bound_is_checked_at_both_ends() {
        let check = |n| batch::check_concurrency(n, "--concurrency");
        assert!(
            check(0).is_err(),
            "0 workers would spawn nothing and never finish"
        );
        assert!(check(1).is_ok());
        assert!(check(batch::MAX_CONCURRENCY).is_ok());
        assert!(check(batch::MAX_CONCURRENCY + 1).is_err());
        assert!(check(usize::MAX).is_err());
    }

    #[test]
    fn a_rejected_concurrency_says_what_the_range_is() {
        let error = batch::check_concurrency(0, "--concurrency").expect_err("0 is out of range");
        assert_eq!(error.code(), exit::USAGE);
        assert!(error.to_string().contains("between 1 and 64"), "{error}");
        assert!(error.to_string().contains("--concurrency"), "{error}");
    }
}
