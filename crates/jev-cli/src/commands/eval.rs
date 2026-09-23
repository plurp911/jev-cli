//! `jev eval` — measure a question against your own labelled examples.
//!
//! # What this command is for
//!
//! One question, asked precisely: *given examples I have already judged, how well does
//! this question, answered by this model version, perform on **my** data — and what
//! threshold can I defend?*
//!
//! It is not a benchmark of Jev. It says nothing about the model in general, and it is
//! not meant to. Every number it prints describes one question, one dataset, and one
//! model version, and the report records all three so that a threshold can never be
//! quoted without them.
//!
//! # Why it exists
//!
//! Everywhere else in this CLI — in `--require`, in the cookbook, in the agent skill —
//! thresholds are placeholders, and the documentation says so. `docs/cli-contract.md`
//! promises `jev` will never ship a default threshold, because a threshold encodes what
//! being wrong costs *you*. That promise leaves a gap: a user told not to invent a
//! number still has to write one. This is how they stop inventing it.
//!
//! # What it deliberately does not do
//!
//! * **It never trains anything.** Every judgment comes from the API; everything else
//!   here is arithmetic over the answers.
//! * **It never sends a label.** Ground truth is compared locally, the way `gate.rs`
//!   compares a `--require` expression locally. See [`crate::dataset`].
//! * **It calls no threshold "optimal".** A threshold is reported together with the
//!   objective it was chosen under, because the best cut under one objective is a bad
//!   cut under another.
//! * **It does not claim a calibration transfers.** A different dataset, a different
//!   question, or a different model version is a different measurement.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use jev_client::{ClientError, Credential, Transport};
use jev_core::{Answer, EvaluationRequest, ModelId, Question, QuestionId};
use serde_json::{Value, json};

use crate::batch;
use crate::cli::{EvalArgs, Objective};
use crate::commands::Session;
use crate::dataset::{self, Dataset, Label, LabeledRow, Side};
use crate::errors::{CliError, Result};
use crate::exit;
use crate::metrics::{
    self, ChoiceObservation, Confusion, CoveragePoint, NoulObservation, NoulSweepPoint,
    ScoreObservation,
};
use crate::paths::same_file;
use crate::render::{DRY_RUN_SCHEMA, EVAL_SCHEMA, json as render_json};
use crate::request;

/// Rows below which the report carries an explicit "this is a small sample" warning.
///
/// Not a statistical rule, and not presented as one. It is a conservative floor: at
/// fifty rows a 95% Wilson interval around an accuracy near one half is already about
/// fourteen points wide in each direction, which is informative but wide; below it the
/// interval swamps the estimate it surrounds. The interval itself is always reported,
/// so a reader never has to take this threshold's word for anything.
const SMALL_SAMPLE: usize = 50;

/// Occurrences of the rarer outcome below which class-conditional metrics are flagged.
const THIN_CLASS: usize = 10;

/// Runs `jev eval`.
pub(crate) fn run(
    session: &mut Session<'_>,
    args: &EvalArgs,
    transport: Option<&(dyn Transport + Send + Sync)>,
) -> Result<u8> {
    batch::check_concurrency(args.concurrency, "--concurrency")?;
    let objective = resolve_objective(args)?;
    check_report_path(args)?;

    let (questions, model) = load_questions(session, args)?;
    let plan = load_rows(session, args, &questions, objective)?;

    if plan.evaluated().is_empty() {
        return Err(CliError::usage(
            "no labelled rows were read; there is nothing to evaluate",
        ));
    }

    if let Some((objective, _)) = objective {
        let applicable = questions
            .iter()
            .filter(|(_, question)| objective.applies_to(question))
            .count();
        if applicable == 0 {
            return Err(CliError::usage(format!(
                "--objective {objective} applies to no question in this request.\n\n{}",
                objective.applicability()
            )));
        }
    }

    session.warn_about_endpoint();

    if session.context.dry_run {
        return dry_run(session, &questions, &plan, &model);
    }

    let (credential, source) = session.credential()?;
    session.note(&format!("credential source: {source}"));
    // `warn`, not `note`: the same reasoning as `jev map`. This is a command whose job
    // is sending bulk local content, and the count is one of the controls that stands in
    // for a confirmation prompt. A visibility control nobody sees is not a control.
    session.warn(&format!(
        "sending {} row(s), {} question(s) each, concurrency {}",
        plan.evaluated().len(),
        questions.len(),
        args.concurrency
    ));
    if let Some(warning) = plan.split_warning() {
        session.note_always(&format!("warning: {warning}"));
    }

    let (outcomes, stopped_early) = evaluate_all(
        session,
        plan.evaluated(),
        &questions,
        &model,
        &credential,
        args.concurrency,
        args.fail_fast,
        transport,
    );

    report(
        session,
        args,
        &questions,
        &model,
        &plan,
        &outcomes,
        objective,
        stopped_early,
    )
}

/// Refuses a `--report` that names one of the files this run reads.
///
/// The report is written with `truncate`, and every other path here is an input the run
/// has already read by the time it is written. Without this,
/// `jev eval -d data.jsonl --report data.jsonl` reads the dataset, measures it, and then
/// destroys it — silently, and after the user has paid for the requests. `jev map` was
/// hardened against the same shape of mistake; this is the same guard, on the same
/// best-effort comparison, and for the same reason: it is a guard against a typo, not a
/// security control.
fn check_report_path(args: &EvalArgs) -> Result<()> {
    let Some(report) = args.report.as_deref() else {
        return Ok(());
    };
    for (path, flag) in [
        (Some(args.request.as_path()), "--request"),
        (args.dataset.as_deref(), "--dataset"),
        (args.calibration.as_deref(), "--calibration"),
        (args.test.as_deref(), "--test"),
    ] {
        if let Some(path) = path
            && same_file(report, path)
        {
            return Err(CliError::usage(format!(
                "--report and {flag} name the same path; the report is written over what \
                 is there, so this would destroy the file this run reads from"
            )));
        }
    }
    Ok(())
}

// --- Objectives ---------------------------------------------------------------------

impl Objective {
    /// Whether this objective can be applied to a question of this kind.
    ///
    /// The split is not a convention, it is what ADR-0010 §5 forces. A Noul carries no
    /// confidence — the API returns none and `jev` refuses to invent one — so a Noul is
    /// swept on its *probability*, which is a decision cut and therefore has no
    /// abstention to trade coverage against. A Choice and a Score do carry a
    /// confidence, which is exactly an abstention axis. Blending the two would mean
    /// synthesizing the certainty score the ADR forbids.
    pub(crate) const fn applies_to(self, question: &Question) -> bool {
        match self {
            Self::MaximizeF1 | Self::MinPrecision | Self::MinRecall => {
                matches!(question, Question::Noul { .. })
            }
            Self::MinAccuracy | Self::TargetCoverage => {
                matches!(question, Question::Choice { .. } | Question::Score { .. })
            }
        }
    }

    /// The sentence explaining which question types this objective is for.
    pub(crate) const fn applicability(self) -> &'static str {
        match self {
            Self::MaximizeF1 | Self::MinPrecision | Self::MinRecall => {
                "It selects a decision cut on a noul's probability, so it needs at least \
                 one noul question. For a choice or a score, use --objective min-accuracy \
                 or --objective target-coverage, which select a confidence cut instead."
            }
            Self::MinAccuracy | Self::TargetCoverage => {
                "It selects a confidence cut, so it needs at least one choice or score \
                 question. A noul has no confidence -- the API returns none and jev does \
                 not invent one -- so for a noul use --objective maximize-f1, \
                 min-precision, or min-recall, which select a cut on the probability \
                 itself."
            }
        }
    }

    /// Whether this objective needs a `--target`.
    pub(crate) const fn needs_target(self) -> bool {
        !matches!(self, Self::MaximizeF1)
    }

    /// The field the threshold is a cut on, for the report.
    pub(crate) const fn field(self) -> &'static str {
        match self {
            Self::MaximizeF1 | Self::MinPrecision | Self::MinRecall => "noul",
            Self::MinAccuracy | Self::TargetCoverage => "confidence",
        }
    }
}

impl std::fmt::Display for Objective {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MaximizeF1 => "maximize-f1",
            Self::MinPrecision => "min-precision",
            Self::MinRecall => "min-recall",
            Self::MinAccuracy => "min-accuracy",
            Self::TargetCoverage => "target-coverage",
        })
    }
}

/// Reconciles `--objective` and `--target`, which are meaningless apart.
fn resolve_objective(args: &EvalArgs) -> Result<Option<(Objective, f64)>> {
    let Some(objective) = args.objective else {
        if args.target.is_some() {
            return Err(CliError::usage(
                "--target has nothing to aim at without --objective.\n\n\
                 Without an objective, `jev eval` reports how the question performed and \
                 selects no threshold.",
            ));
        }
        return Ok(None);
    };
    match (objective.needs_target(), args.target) {
        (true, None) => Err(CliError::usage(format!(
            "--objective {objective} needs a --target between 0 and 1.\n\n\
             It is the floor (or, for target-coverage, the level) you are asking the \
             threshold to reach."
        ))),
        (false, Some(_)) => Err(CliError::usage(format!(
            "--objective {objective} takes no --target; it maximizes F1 outright."
        ))),
        (true, Some(target)) if !(0.0..=1.0).contains(&target) => Err(CliError::usage(
            "--target must be between 0 and 1".to_owned(),
        )),
        (_, target) => Ok(Some((objective, target.unwrap_or(0.0)))),
    }
}

// --- Inputs -------------------------------------------------------------------------

/// Loads the question set once. It is the same for every row.
///
/// The same `--model`-beats-request-file precedence `jev ask` and `jev map` apply, so
/// one committed request file behaves identically under all three.
fn load_questions(
    session: &mut Session<'_>,
    args: &EvalArgs,
) -> Result<(Vec<(QuestionId, Question)>, ModelId)> {
    let (bytes, origin) = session.reader().read(&args.request, session.stdin)?;
    let text = String::from_utf8(bytes)
        .map_err(|_| CliError::usage(format!("{origin} is not valid UTF-8")))?;
    let document = request::parse_document(&text, &origin.to_string())?;
    if document.state.is_some() {
        session.warn(
            "note: the request document's `state` is ignored by `jev eval`; each \
             labelled row supplies the state",
        );
    }
    let model = if session.context.model.from == crate::context::Provenance::Flag {
        session.context.model.value.clone()
    } else {
        document
            .model
            .unwrap_or_else(|| session.context.model.value.clone())
    };
    Ok((document.questions, model))
}

/// How the rows were divided, and which of them each side holds.
struct Plan {
    /// Every row that will be sent, in file order: calibration first is not assumed.
    rows: Vec<LabeledRow>,
    /// Indexes into `rows` that a threshold is chosen on.
    calibration: Vec<usize>,
    /// Indexes into `rows` that the result is reported on.
    reported: Vec<usize>,
    /// What to call the split in the report.
    mode: &'static str,
    /// Where the rows came from, for the report.
    origins: Vec<String>,
    /// The fingerprint of the rows actually evaluated.
    fingerprint: String,
    /// Seed and fraction, when a seeded split was used.
    seed: Option<u64>,
    test_fraction: Option<f64>,
    /// The sentence printed when selection and reporting share rows.
    warning: Option<String>,
}

impl Plan {
    fn evaluated(&self) -> &[LabeledRow] {
        &self.rows
    }

    fn split_warning(&self) -> Option<String> {
        self.warning.clone()
    }
}

/// Joins an explicit calibration file and test file into one plan.
fn join(
    calibration: Dataset,
    test: Dataset,
    calibration_path: &Path,
    test_path: &Path,
) -> Result<Plan> {
    let boundary = calibration.rows.len();
    let mut rows = calibration.rows;
    rows.extend(test.rows);
    // The one way two files can hide the very leak the split exists to prevent: the
    // same example in both. Ids are unique *within* a file, so this has to be checked
    // across the join rather than by the parser.
    let mut seen = std::collections::BTreeSet::new();
    for row in &rows {
        if !seen.insert(row.id.as_str()) {
            return Err(CliError::usage(format!(
                "row id `{}` appears in both --calibration and --test.\n\n\
                 A threshold chosen on an example and then reported on that same \
                 example is not a held-out estimate.",
                crate::output::Safe::new(&row.id)
            )));
        }
    }
    let fingerprint = dataset::fingerprint_of(&rows);
    Ok(Plan {
        calibration: (0..boundary).collect(),
        reported: (boundary..rows.len()).collect(),
        rows,
        mode: "files",
        origins: vec![
            calibration_path.display().to_string(),
            test_path.display().to_string(),
        ],
        fingerprint,
        seed: None,
        test_fraction: None,
        warning: None,
    })
}

/// Reads the dataset, or the calibration and test files, and divides the rows.
fn load_rows(
    session: &mut Session<'_>,
    args: &EvalArgs,
    questions: &[(QuestionId, Question)],
    objective: Option<(Objective, f64)>,
) -> Result<Plan> {
    let read = |session: &mut Session<'_>, path: &Path| -> Result<Dataset> {
        let (bytes, origin) = session.reader().read(path, session.stdin)?;
        let text = String::from_utf8(bytes)
            .map_err(|_| CliError::usage(format!("{origin} is not valid UTF-8")))?;
        let origin = origin.to_string();
        let parsed = dataset::parse(
            &text,
            &origin,
            questions,
            &args.request.display().to_string(),
        )?;
        Ok(parsed)
    };

    if let (Some(calibration), Some(test)) = (&args.calibration, &args.test) {
        let left = read(session, calibration)?;
        let right = read(session, test)?;
        return join(left, right, calibration, test);
    }

    let Some(path) = &args.dataset else {
        return Err(CliError::usage(
            "`jev eval` needs labelled examples.\n\n\
             Pass --dataset PATH with a JSONL file of them, or --calibration PATH and \
             --test PATH to supply the two sides yourself.",
        ));
    };
    let mut parsed = read(session, path)?;
    let origin = path.display().to_string();
    if let Some(limit) = args.limit {
        if limit == 0 {
            return Err(CliError::usage("--limit must be at least 1"));
        }
        if parsed.rows.len() > limit {
            session.warn(&format!(
                "--limit: evaluating the first {limit} of {} labelled row(s)",
                parsed.rows.len()
            ));
            parsed = parsed.truncated(limit);
        }
    }

    let rows = parsed.rows;
    let indexes: Vec<usize> = (0..rows.len()).collect();

    // No objective means no threshold is chosen, so there is nothing for a holdout to
    // protect against: every metric is descriptive and every row is evidence. Splitting
    // anyway would throw away a third of a user's labelling work to guard against a
    // leak that cannot happen.
    if objective.is_none() {
        return Ok(Plan {
            calibration: Vec::new(),
            reported: indexes,
            rows,
            mode: "none",
            origins: vec![origin],
            fingerprint: parsed.fingerprint,
            seed: None,
            test_fraction: None,
            warning: None,
        });
    }

    if args.no_split {
        return Ok(Plan {
            calibration: indexes.clone(),
            reported: indexes,
            rows,
            mode: "none",
            origins: vec![origin],
            fingerprint: parsed.fingerprint,
            seed: None,
            test_fraction: None,
            warning: Some(
                "the threshold was selected and reported on the same rows (--no-split), \
                 so the reported performance is optimistic by an unknown amount. Drop \
                 --no-split, or pass separate --calibration and --test files, for a \
                 held-out estimate."
                    .to_owned(),
            ),
        });
    }

    split(
        rows,
        parsed.fingerprint,
        origin,
        args.seed,
        args.test_fraction,
    )
}

/// Divides rows into a calibration side and a reported side, by seeded hash of the id.
fn split(
    rows: Vec<LabeledRow>,
    fingerprint: String,
    origin: String,
    seed: u64,
    fraction: f64,
) -> Result<Plan> {
    if !(0.0 < fraction && fraction < 1.0) {
        return Err(CliError::usage(
            "--test-fraction must be strictly between 0 and 1",
        ));
    }
    let mut calibration = Vec::new();
    let mut reported = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        match dataset::side(&row.id, seed, fraction) {
            Side::Test => reported.push(index),
            Side::Calibration => calibration.push(index),
        }
    }
    // A hash split on a handful of rows can land everything on one side. Said plainly
    // rather than silently, because the report would otherwise carry a held-out
    // estimate computed from the selection rows.
    let warning = (calibration.is_empty() || reported.is_empty()).then(|| {
        format!(
            "the seeded split put every row on one side ({} calibration, {} reported). \
             There are too few rows for a holdout to mean anything, so the threshold \
             and the numbers reported beside it come from the same examples.",
            calibration.len(),
            reported.len()
        )
    });

    Ok(Plan {
        calibration,
        reported,
        rows,
        mode: "seeded",
        origins: vec![origin],
        fingerprint,
        seed: Some(seed),
        test_fraction: Some(fraction),
        warning,
    })
}

// --- Execution ------------------------------------------------------------------------

/// What one row's API call produced.
struct RowOutcome {
    index: usize,
    /// The answers, by question id. Empty when the call failed.
    answers: BTreeMap<String, Answer>,
    /// The model that actually answered.
    model: Option<ModelId>,
    error: Option<(String, String)>,
    auth_failed: bool,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

#[allow(
    clippy::too_many_arguments,
    reason = "each argument is an independent input to the batch; grouping them would \
              only move the list"
)]
fn evaluate_all(
    session: &Session<'_>,
    rows: &[LabeledRow],
    questions: &[(QuestionId, Question)],
    model: &ModelId,
    credential: &Credential,
    concurrency: usize,
    fail_fast: bool,
    transport: Option<&(dyn Transport + Send + Sync)>,
) -> (Vec<RowOutcome>, bool) {
    let indexed: Vec<(usize, &LabeledRow)> = rows.iter().enumerate().collect();
    let plan = batch::Plan {
        endpoint: &session.context.endpoint.value,
        retry: session.context.retry,
        timeout: session.context.timeout.value,
        concurrency,
        clock: crate::interrupt::InterruptibleClock::default(),
    };
    let (mut outcomes, stopped_early) =
        batch::each(&plan, &indexed, transport, |client, (index, row)| {
            let outcome = evaluate_one(client, *index, row, questions, model, credential);
            let stop = outcome.auth_failed || (fail_fast && outcome.error.is_some());
            if stop {
                batch::Step::Last(outcome)
            } else {
                batch::Step::Continue(outcome)
            }
        });
    outcomes.sort_by_key(|outcome| outcome.index);
    (outcomes, stopped_early)
}

fn evaluate_one(
    client: &batch::Worker<'_>,
    index: usize,
    row: &LabeledRow,
    questions: &[(QuestionId, Question)],
    model: &ModelId,
    credential: &Credential,
) -> RowOutcome {
    let failure = |kind: &str, message: String| RowOutcome {
        index,
        answers: BTreeMap::new(),
        model: None,
        error: Some((kind.to_owned(), crate::output::sanitize(&message))),
        auth_failed: kind == "auth",
        input_tokens: None,
        output_tokens: None,
    };

    // Built from the row's `state` alone. `row.labels` is not in scope for this
    // expression, which is what makes "the ground truth was sent to the API" a thing
    // the types prevent rather than a thing a reviewer has to notice.
    let request = match EvaluationRequest::new(row.state.clone(), model.clone(), questions.to_vec())
    {
        Ok(request) => request,
        Err(error) => return failure("invalid-request", error.to_string()),
    };

    let (result, _stats) = client.evaluate(&request, credential);
    match result {
        Ok(response) => RowOutcome {
            index,
            answers: response
                .answers
                .iter()
                .map(|(id, answer)| (id.as_str().to_owned(), answer.clone()))
                .collect(),
            model: Some(response.model),
            error: None,
            auth_failed: false,
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
        },
        Err(error) => {
            let kind = if error.is_auth() {
                "auth"
            } else if error.is_unavailable()
                || matches!(error, ClientError::MalformedResponse { .. })
            {
                "unavailable"
            } else {
                "request"
            };
            failure(kind, error.to_string())
        }
    }
}

// --- Scoring --------------------------------------------------------------------------

/// The observations for one question, on one side of the split.
enum Observations {
    Noul(Vec<NoulObservation>),
    Choice {
        points: Vec<ChoiceObservation>,
        distributions: Vec<BTreeMap<String, f64>>,
    },
    Score(Vec<ScoreObservation>),
}

impl Observations {
    fn empty_for(question: &Question) -> Self {
        match question {
            Question::Noul { .. } => Self::Noul(Vec::new()),
            Question::Choice { .. } => Self::Choice {
                points: Vec::new(),
                distributions: Vec::new(),
            },
            Question::Score { .. } => Self::Score(Vec::new()),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Noul(points) => points.len(),
            Self::Choice { points, .. } => points.len(),
            Self::Score(points) => points.len(),
        }
    }

    /// Adds one row's answer, when the row labelled this question and the API answered
    /// it with the type the question declared.
    ///
    /// A mismatch is skipped rather than coerced: an answer whose type is not the one
    /// asked for is a response that did not match the request, and guessing what the
    /// user meant would silently score something else.
    fn push(&mut self, answer: &Answer, label: &Label) {
        match (self, answer, label) {
            (Self::Noul(points), Answer::Noul { noul }, Label::Noul(label)) => {
                points.push(NoulObservation {
                    probability: noul.get(),
                    label: *label,
                });
            }
            (
                Self::Choice {
                    points,
                    distributions,
                },
                Answer::Choice {
                    choice,
                    probabilities,
                    confidence,
                },
                Label::Choice(label),
            ) => {
                points.push(ChoiceObservation {
                    predicted: choice.clone(),
                    label: label.clone(),
                    confidence: confidence.get(),
                });
                distributions.push(
                    probabilities
                        .iter()
                        .map(|weighted| (weighted.key.clone(), weighted.probability.get()))
                        .collect(),
                );
            }
            (
                Self::Score(points),
                Answer::Score {
                    score, confidence, ..
                },
                Label::Score(label),
            ) => {
                points.push(ScoreObservation {
                    score: *score,
                    label: *label,
                    confidence: confidence.get(),
                });
            }
            _ => {}
        }
    }
}

/// Collects the observations for one question over a set of row indexes.
fn observe(
    question: &Question,
    id: &str,
    rows: &[LabeledRow],
    outcomes: &[RowOutcome],
    indexes: &[usize],
) -> Observations {
    let mut observations = Observations::empty_for(question);
    for index in indexes {
        let (Some(row), Some(outcome)) = (rows.get(*index), outcomes.get(*index)) else {
            continue;
        };
        let (Some(label), Some(answer)) = (row.labels.get(id), outcome.answers.get(id)) else {
            continue;
        };
        observations.push(answer, label);
    }
    observations
}

/// A threshold and everything needed to defend it.
struct Selection {
    /// `None` when no candidate met the objective's target. Never a number that failed
    /// the requirement: a threshold that does not do what the user asked for is not an
    /// answer to their question, and reporting one anyway is how a "calibrated" gate
    /// ends up weaker than the number beside it claims.
    threshold: Option<f64>,
    /// What happened at that threshold **on the calibration rows**.
    at_calibration: Value,
    reachable: bool,
    tie_break: &'static str,
}

impl Selection {
    /// The verdict when the data contains no threshold meeting the objective's target.
    const fn unreachable() -> Self {
        Self {
            threshold: None,
            at_calibration: Value::Null,
            reachable: false,
            tie_break: "",
        }
    }
}

/// Picks a Noul decision cut under `objective`.
fn select_noul(sweep: &[NoulSweepPoint], objective: Objective, target: f64) -> Option<Selection> {
    let describe = |confusion: Confusion| {
        json!({
            "accuracy": confusion.accuracy(),
            "precision": confusion.precision(),
            "recall": confusion.recall(),
            "specificity": confusion.specificity(),
            "f1": confusion.f1(),
            "negative_predictive_value": confusion.negative_predictive_value(),
            "predicted_yes": confusion.true_positive + confusion.false_positive,
        })
    };

    // Each arm states its own tie-break, because two thresholds can score identically
    // and "whichever the sort happened to leave first" is not a defensible answer.
    let chosen = match objective {
        // The larger cut wins a tie: it predicts yes for fewer rows, so among equally
        // scoring thresholds it is the one that commits less.
        Objective::MaximizeF1 => sweep
            .iter()
            .filter(|point| point.confusion.f1().is_some())
            .max_by(|left, right| {
                let score = |point: &NoulSweepPoint| point.confusion.f1().unwrap_or(f64::MIN);
                score(left)
                    .total_cmp(&score(right))
                    .then(left.threshold.total_cmp(&right.threshold))
            }),
        // Among the cuts that clear the floor, the most useful is the one that catches
        // the most; the smallest such cut wins a tie, because it covers the most rows.
        Objective::MinPrecision => sweep
            .iter()
            .filter(|point| {
                point
                    .confusion
                    .precision()
                    .is_some_and(|value| value >= target)
            })
            .max_by(|left, right| {
                let score = |point: &NoulSweepPoint| point.confusion.recall().unwrap_or(f64::MIN);
                score(left)
                    .total_cmp(&score(right))
                    .then(right.threshold.total_cmp(&left.threshold))
            }),
        Objective::MinRecall => sweep
            .iter()
            .filter(|point| {
                point
                    .confusion
                    .recall()
                    .is_some_and(|value| value >= target)
            })
            .max_by(|left, right| {
                let score =
                    |point: &NoulSweepPoint| point.confusion.precision().unwrap_or(f64::MIN);
                score(left)
                    .total_cmp(&score(right))
                    .then(right.threshold.total_cmp(&left.threshold))
            }),
        Objective::MinAccuracy | Objective::TargetCoverage => None,
    };

    if sweep.is_empty() {
        return None;
    }
    // Nothing reached the floor. Reported as unreachable rather than quietly relaxed.
    let Some(point) = chosen else {
        return Some(Selection::unreachable());
    };
    Some(Selection {
        threshold: Some(point.threshold),
        at_calibration: describe(point.confusion),
        reachable: true,
        tie_break: match objective {
            Objective::MaximizeF1 => "the higher threshold",
            _ => "the lower threshold",
        },
    })
}

/// Picks a Choice or Score confidence cut under `objective`.
fn select_coverage(
    sweep: &[CoveragePoint],
    objective: Objective,
    target: f64,
) -> Option<Selection> {
    let describe = |point: CoveragePoint| {
        json!({
            "coverage": point.coverage(),
            "abstention": point.abstention(),
            "accuracy_among_covered": point.accuracy_among_covered(),
            "risk_among_covered": point.risk(),
            "covered": point.covered,
        })
    };

    let chosen = match objective {
        // Among the cuts whose automatically handled rows are accurate enough, take the
        // one that handles the most; the lower cut wins a tie for the same reason.
        Objective::MinAccuracy => sweep
            .iter()
            .filter(|point| {
                point
                    .accuracy_among_covered()
                    .is_some_and(|value| value >= target)
            })
            .max_by(|left, right| {
                let score = |point: &CoveragePoint| point.coverage().unwrap_or(f64::MIN);
                score(left)
                    .total_cmp(&score(right))
                    .then(right.threshold.total_cmp(&left.threshold))
            }),
        // Closest to the requested coverage; on a tie, prefer covering at least as much
        // as was asked for rather than slightly less, then the lower cut.
        //
        // Filtered on a *defined* coverage first. A sweep always carries the synthetic
        // cut at zero, even when there were no observations at all, so without this an
        // empty calibration set returned `threshold: 0.0, reachable: true` -- a number
        // presented as chosen, from literally no data, which is exactly the guess this
        // whole command exists to replace.
        Objective::TargetCoverage => sweep
            .iter()
            .filter(|point| point.coverage().is_some())
            .min_by(|left, right| {
                let distance = |point: &CoveragePoint| {
                    point
                        .coverage()
                        .map_or(f64::MAX, |coverage| (coverage - target).abs())
                };
                let shortfall =
                    |point: &CoveragePoint| u8::from(point.coverage().is_some_and(|c| c < target));
                distance(left)
                    .total_cmp(&distance(right))
                    .then(shortfall(left).cmp(&shortfall(right)))
                    .then(left.threshold.total_cmp(&right.threshold))
            }),
        Objective::MaximizeF1 | Objective::MinPrecision | Objective::MinRecall => None,
    };

    if sweep.is_empty() {
        return None;
    }
    let Some(point) = chosen else {
        return Some(Selection::unreachable());
    };
    Some(Selection {
        threshold: Some(point.threshold),
        at_calibration: describe(*point),
        reachable: true,
        tie_break: match objective {
            Objective::TargetCoverage => {
                "the coverage at or above the target, then the lower threshold"
            }
            _ => "the lower threshold",
        },
    })
}

// --- The report -----------------------------------------------------------------------

/// What scoring every question produced.
struct Scored {
    questions: serde_json::Map<String, Value>,
    /// Whether any applicable objective failed to reach its target.
    unreachable: bool,
    warnings: Vec<String>,
}

/// Scores every question, selecting a threshold where the objective applies.
fn score_questions(
    args: &EvalArgs,
    questions: &[(QuestionId, Question)],
    plan: &Plan,
    outcomes: &[RowOutcome],
    objective: Option<(Objective, f64)>,
) -> Scored {
    let mut document = serde_json::Map::new();
    let mut unreachable = false;
    let mut warnings: Vec<String> = Vec::new();

    for (id, question) in questions {
        let id = id.as_str();
        let reported = observe(question, id, &plan.rows, outcomes, &plan.reported);
        let calibration = observe(question, id, &plan.rows, outcomes, &plan.calibration);
        // Over the *reported* rows, not the whole dataset: `n` counts those, and a
        // `labelled` counted over both sides made a perfectly healthy 70/30 split look
        // like a run in which seventy rows had failed.
        let labelled = plan
            .reported
            .iter()
            .filter_map(|index| plan.rows.get(*index))
            .filter(|row| row.labels.contains_key(id))
            .count();

        let applicable = objective.is_some_and(|(objective, _)| objective.applies_to(question));
        let selection = if applicable {
            objective.and_then(|(objective, target)| {
                // Selected on the calibration rows, always. With no objective there is
                // no selection, and with `--no-split` the two sets are the same set --
                // which is exactly what the warning above says out loud.
                match &calibration {
                    Observations::Noul(points) => {
                        select_noul(&metrics::noul(points).sweep, objective, target)
                    }
                    Observations::Choice {
                        points,
                        distributions,
                    } => select_coverage(
                        &metrics::choice(points, &option_names(question), distributions).sweep,
                        objective,
                        target,
                    ),
                    Observations::Score(points) => select_coverage(
                        &metrics::score(points, level_count(question)).sweep,
                        objective,
                        target,
                    ),
                }
            })
        } else {
            None
        };
        if selection.as_ref().is_some_and(|choice| !choice.reachable) {
            unreachable = true;
            warnings.push(format!("{id}: {}", unreachable_reason(objective)));
        }

        if reported.len() < SMALL_SAMPLE && reported.len() > 0 {
            warnings.push(format!(
                "{id}: only {} labelled row(s) were scored; the 95% interval beside the \
                 headline number is how wide the uncertainty is on a sample this small",
                reported.len()
            ));
        }
        // The threshold is chosen from the *calibration* rows, so that is the count that
        // decides whether it means anything. Warning only on the reported side let a cut
        // be fitted to three examples without a word, as long as the held-out side
        // happened to be large.
        if selection.is_some() && calibration.len() < SMALL_SAMPLE {
            warnings.push(format!(
                "{id}: the threshold was chosen from {} calibration row(s); a cut fitted \
                 to a sample this small is unlikely to hold on new data",
                calibration.len()
            ));
        }

        document.insert(
            id.to_owned(),
            question_document(
                question,
                &reported,
                labelled,
                objective,
                applicable,
                selection.as_ref(),
                &mut warnings,
                id,
                args.show_rows
                    .then(|| row_details(question, id, &plan.rows, outcomes, &plan.reported)),
            ),
        );
    }

    Scored {
        questions: document,
        unreachable,
        warnings,
    }
}

/// Builds the report document, prints it, and returns the exit code.
#[allow(
    clippy::too_many_arguments,
    reason = "every argument is a distinct input to the document; a struct would only \
              move the list"
)]
fn report(
    session: &mut Session<'_>,
    args: &EvalArgs,
    questions: &[(QuestionId, Question)],
    requested_model: &ModelId,
    plan: &Plan,
    outcomes: &[RowOutcome],
    objective: Option<(Objective, f64)>,
    stopped_early: bool,
) -> Result<u8> {
    let interrupted = crate::interrupt::requested();
    let failed: Vec<&RowOutcome> = outcomes
        .iter()
        .filter(|outcome| outcome.error.is_some())
        .collect();
    let auth_failed = outcomes.iter().any(|outcome| outcome.auth_failed);

    let answered_by = answered_by(outcomes);

    let Scored {
        questions: questions_document,
        unreachable,
        mut warnings,
    } = score_questions(args, questions, plan, outcomes, objective);
    if let Some(warning) = plan.split_warning() {
        warnings.insert(0, warning);
    }
    // Recorded in the document, not only in the exit code. A report that was built from
    // a run somebody stopped halfway describes fewer rows than the dataset has, and a
    // `--report` file read back weeks later has no other way to say so.
    if stopped_early || interrupted {
        warnings.insert(
            0,
            format!(
                "the run did not reach every row: {} of {} were evaluated, so every \
                 number here describes only those",
                outcomes.len(),
                plan.rows.len()
            ),
        );
    }

    let document = json!({
        "schema": EVAL_SCHEMA,
        "endpoint": session.context.endpoint.value.to_string(),
        "model_requested": requested_model.as_str(),
        "model": answered_by,
        "evaluated_at": timestamp(),
        "dataset": {
            "sources": plan.origins,
            "rows": plan.rows.len(),
            "fingerprint": plan.fingerprint,
        },
        "request": {
            "source": args.request.display().to_string(),
            "fingerprint": crate::commands::map::request_fingerprint(questions, requested_model),
        },
        "split": {
            "mode": plan.mode,
            "seed": plan.seed,
            "test_fraction": plan.test_fraction,
            "calibration_rows": plan.calibration.len(),
            "reported_rows": plan.reported.len(),
        },
        "objective": objective.map(|(objective, target)| json!({
            "name": objective.to_string(),
            "target": objective.needs_target().then_some(target),
            "threshold_field": objective.field(),
        })),
        "questions": Value::Object(questions_document),
        "rows": {
            "total": plan.rows.len(),
            "evaluated": outcomes.len(),
            "stopped_early": stopped_early || interrupted,
            "interrupted": interrupted,
            "failed": failed.len(),
            // Grouped, with one representative message each. A run in which every row
            // failed for one reason previously reported only a count, and the reason --
            // which is the only actionable part -- was nowhere in the output at all.
            "errors": error_summary(&failed),
        },
        "usage": usage(outcomes),
        // Carried in the document as well as printed on stderr, so a report read back
        // from a file months later still says what was wrong with it.
        "warnings": warnings,
    });

    if let Some(path) = &args.report {
        write_report(path, &document)?;
        session.warn(&format!("report written to {}", path.display()));
    }

    if session.json() {
        render_json::write_document(session.out, &document)?;
    } else {
        let rendered = render_text(&document, questions, objective);
        render_json::write_all(session.out, rendered.as_bytes())?;
    }

    // The split warning was already printed before the run, while there was still time
    // to stop it; it stays in the document, but saying it twice on stderr is noise.
    let split_warning = plan.split_warning();
    for warning in &warnings {
        if split_warning.as_ref() != Some(warning) {
            session.note_always(&format!("warning: {warning}"));
        }
    }

    Ok(finish(
        session,
        &Finish {
            interrupted,
            stopped_early,
            auth_failed,
            failed: failed.len(),
            first_error: failed
                .first()
                .and_then(|outcome| outcome.error.as_ref())
                .map(|(kind, message)| format!("{kind}: {message}")),
            evaluated: outcomes.len(),
            total: plan.rows.len(),
            unreachable,
        },
    ))
}

/// The facts the exit code is decided from.
#[allow(
    clippy::struct_excessive_bools,
    reason = "each flag is an independent fact about the run, and the exit code is \
              decided by checking them in order; collapsing them into an enum would \
              force a precedence decision into the type rather than into the one place \
              that reads them"
)]
struct Finish {
    interrupted: bool,
    stopped_early: bool,
    auth_failed: bool,
    failed: usize,
    /// One representative failure, so the stderr line says *why* rather than only how
    /// many. A run in which every row failed for one reason used to report the count
    /// and leave the reason out of the output entirely.
    first_error: Option<String>,
    evaluated: usize,
    total: usize,
    /// An objective applied and no threshold in the data reached its target.
    unreachable: bool,
}

/// Turns the run's outcome into an exit code, most specific fact first.
fn finish(session: &mut Session<'_>, run: &Finish) -> u8 {
    // Checked before everything else: an interrupted run is not a result, and reporting
    // one as a success -- which is what happened when the batch's stop signal was
    // dropped on the floor -- means a report built from whatever finished first, exit 0,
    // and no indication that anything is missing.
    if run.interrupted {
        session.warn(&format!(
            "interrupted after {} of {} row(s); the report covers only those",
            run.evaluated, run.total
        ));
        return exit::INTERRUPTED;
    }
    if run.auth_failed {
        session.warn(
            "the API rejected the credential; the run stopped rather than sending a \
             request for every remaining row",
        );
        return exit::AUTH;
    }
    if run.failed > 0 {
        let example = run
            .first_error
            .as_ref()
            .map_or_else(String::new, |reason| format!(" ({reason})"));
        session.warn(&format!(
            "{} of {} row(s) failed; the metrics cover only the rows that answered{example}",
            run.failed, run.evaluated
        ));
        return exit::PARTIAL;
    }
    if run.stopped_early {
        session.warn(&format!(
            "stopped after {} of {} row(s); the report covers only those",
            run.evaluated, run.total
        ));
        return exit::PARTIAL;
    }
    if run.unreachable {
        // Exit 1, the code that already means "a condition was evaluated and did not
        // hold". It did: the data was measured, and no threshold in it meets the floor
        // the user asked for. That is a finding, not a failure of the run -- the report
        // is still on stdout -- and a CI job asking "does this question still clear 95%
        // precision?" needs to be able to branch on it.
        return exit::UNSATISFIED;
    }
    exit::SUCCESS
}

/// The token totals the API reported, or `null` where it reported none.
///
/// `null` rather than zero for a run that got no counts at all: "the API did not say"
/// and "it cost nothing" are different facts, and a cost report that confuses them is
/// worse than one that admits it does not know.
fn usage(outcomes: &[RowOutcome]) -> Value {
    let total = |pick: fn(&RowOutcome) -> Option<u64>| -> Option<u64> {
        let counted: Vec<u64> = outcomes.iter().filter_map(pick).collect();
        (!counted.is_empty()).then(|| counted.iter().sum())
    };
    json!({
        "input_tokens": total(|outcome| outcome.input_tokens),
        "output_tokens": total(|outcome| outcome.output_tokens),
    })
}

/// Groups the row failures by kind, with one representative message each.
fn error_summary(failed: &[&RowOutcome]) -> Value {
    let mut by_kind: BTreeMap<&str, (usize, &str)> = BTreeMap::new();
    for outcome in failed {
        let Some((kind, message)) = &outcome.error else {
            continue;
        };
        let entry = by_kind
            .entry(kind.as_str())
            .or_insert((0, message.as_str()));
        entry.0 += 1;
    }
    Value::Object(
        by_kind
            .into_iter()
            .map(|(kind, (count, example))| {
                (kind.to_owned(), json!({"count": count, "example": example}))
            })
            .collect(),
    )
}

/// Every concrete model version that answered, sorted and deduplicated.
///
/// Not the alias that was asked for. Recording the version is the whole reason a
/// calibration can be checked again after TypeSafe ships a release: `jev-latest` is a
/// moving target, so a threshold measured against it was measured against whatever it
/// meant that day. Normally one entry; more than one means the alias resolved
/// differently partway through a run, which is worth seeing rather than collapsing.
fn answered_by(outcomes: &[RowOutcome]) -> Vec<&str> {
    let mut seen: Vec<&str> = outcomes
        .iter()
        .filter_map(|outcome| outcome.model.as_ref().map(ModelId::as_str))
        .collect();
    seen.sort_unstable();
    seen.dedup();
    seen
}

/// Why no threshold could be chosen, in the objective's own terms.
///
/// `maximize-f1` takes no `--target`, so a message saying "no threshold reaches
/// --target 0" named a flag the CLI explicitly refuses to accept and described a
/// requirement the user never stated. What actually happened for that objective is that
/// F1 is undefined at every cut, which happens when the calibration rows are all one
/// label — and saying *that* is what tells the user to go and find some counterexamples.
fn unreachable_reason(objective: Option<(Objective, f64)>) -> String {
    match objective {
        Some((objective, target)) if objective.needs_target() => format!(
            "no threshold in the calibration rows reaches --target {target}; the \
             question as written may not support the guarantee you asked for"
        ),
        _ => "no threshold could be scored: the metric is undefined at every cut, which \
              happens when the calibration rows carry only one of the two labels"
            .to_owned(),
    }
}

/// The names a Choice declared, in the order the request file wrote them.
fn option_names(question: &Question) -> Vec<String> {
    match question {
        Question::Choice { options, .. } => options
            .iter()
            .map(|option| option.name().to_owned())
            .collect(),
        Question::Noul { .. } | Question::Score { .. } => Vec::new(),
    }
}

/// How many levels a Score declared.
fn level_count(question: &Question) -> u32 {
    match question {
        Question::Score { levels, .. } => u32::try_from(levels.len()).unwrap_or(u32::MAX),
        Question::Noul { .. } | Question::Choice { .. } => 0,
    }
}

/// The per-question section of the report.
#[allow(
    clippy::too_many_arguments,
    reason = "the document is assembled from independent parts; a struct would only \
              move the list"
)]
fn question_document(
    question: &Question,
    reported: &Observations,
    labelled: usize,
    objective: Option<(Objective, f64)>,
    applicable: bool,
    selection: Option<&Selection>,
    warnings: &mut Vec<String>,
    id: &str,
    rows: Option<Value>,
) -> Value {
    let mut document = serde_json::Map::new();
    document.insert("type".to_owned(), json!(question.kind().as_str()));
    document.insert("n".to_owned(), json!(reported.len()));
    // Labelled but unanswered: the rows whose API call failed, or whose answer came
    // back as a different type than the question asked for. Reported so that `n` can
    // never be quietly smaller than the dataset without saying why.
    document.insert("labelled".to_owned(), json!(labelled));
    document.insert(
        "objective_applies".to_owned(),
        json!(objective.is_some().then_some(applicable)),
    );
    document.insert(
        "threshold".to_owned(),
        json!(selection.and_then(|choice| choice.threshold)),
    );
    document.insert(
        "threshold_reachable".to_owned(),
        json!(selection.map(|choice| choice.reachable)),
    );
    document.insert(
        "threshold_tie_break".to_owned(),
        json!(
            selection.and_then(|choice| (!choice.tie_break.is_empty()).then_some(choice.tie_break))
        ),
    );
    document.insert(
        "at_threshold_on_calibration".to_owned(),
        selection.map_or(Value::Null, |choice| choice.at_calibration.clone()),
    );

    match reported {
        Observations::Noul(points) => {
            noul_section(&mut document, points, question, selection, warnings, id);
        }
        Observations::Choice {
            points,
            distributions,
        } => choice_section(&mut document, points, distributions, question, selection),
        Observations::Score(points) => {
            score_section(&mut document, points, question, selection);
        }
    }

    if let Some(rows) = rows {
        document.insert("rows".to_owned(), rows);
    }
    Value::Object(document)
}

/// The Noul half of a question's report section.
fn noul_section(
    document: &mut serde_json::Map<String, Value>,
    points: &[NoulObservation],
    question: &Question,
    selection: Option<&Selection>,
    warnings: &mut Vec<String>,
    id: &str,
) {
    let _ = question;
    let report = metrics::noul(points);
    let correct = selection
        .and_then(|choice| choice.threshold)
        .map(|threshold| metrics::noul_confusion(points, threshold));
    if report.positives < THIN_CLASS || report.n.saturating_sub(report.positives) < THIN_CLASS {
        warnings.push(format!(
            "{id}: the rarer outcome appears in fewer than {THIN_CLASS} scored \
             row(s); precision, recall, and F1 involving it are unstable"
        ));
    }
    document.insert("positives".to_owned(), json!(report.positives));
    document.insert("brier_score".to_owned(), json!(report.brier_score));
    document.insert("log_loss".to_owned(), json!(report.log_loss));
    document.insert(
        "calibration".to_owned(),
        calibration_value(&report.calibration),
    );
    if let Some(confusion) = correct {
        document.insert("at_threshold".to_owned(), confusion_value(confusion));
        document.insert(
            "headline".to_owned(),
            headline(
                "accuracy",
                confusion.true_positive + confusion.true_negative,
                confusion.total(),
            ),
        );
    }
    document.insert(
        "threshold_sweep".to_owned(),
        Value::Array(
            report
                .sweep
                .iter()
                .map(|point| {
                    let mut row = json!({"threshold": point.threshold});
                    if let (Some(object), Value::Object(detail)) =
                        (row.as_object_mut(), confusion_value(point.confusion))
                    {
                        object.extend(detail);
                    }
                    row
                })
                .collect(),
        ),
    );
}

/// The Choice half of a question's report section.
fn choice_section(
    document: &mut serde_json::Map<String, Value>,
    points: &[ChoiceObservation],
    distributions: &[BTreeMap<String, f64>],
    question: &Question,
    selection: Option<&Selection>,
) {
    let names = option_names(question);
    let report = metrics::choice(points, &names, distributions);
    document.insert("accuracy".to_owned(), json!(report.accuracy));
    document.insert("macro_precision".to_owned(), json!(report.macro_precision));
    document.insert("macro_recall".to_owned(), json!(report.macro_recall));
    document.insert("macro_f1".to_owned(), json!(report.macro_f1));
    document.insert("brier_score".to_owned(), json!(report.brier_score));
    document.insert(
        "per_class".to_owned(),
        Value::Object(
            report
                .per_class
                .iter()
                .map(|(name, score)| {
                    (
                        name.clone(),
                        json!({
                            "precision": score.precision,
                            "recall": score.recall,
                            "f1": score.f1,
                            "support": score.support,
                        }),
                    )
                })
                .collect(),
        ),
    );
    document.insert("confusion".to_owned(), nested_counts(&report.confusion));
    document.insert(
        "calibration".to_owned(),
        calibration_value(&report.calibration),
    );
    let correct = points
        .iter()
        .filter(|point| point.predicted == point.label)
        .count();
    document.insert(
        "headline".to_owned(),
        headline("accuracy", correct, points.len()),
    );
    document.insert("coverage_sweep".to_owned(), coverage_value(&report.sweep));
    insert_at_threshold(document, selection, &report.sweep);
}

/// The Score half of a question's report section.
fn score_section(
    document: &mut serde_json::Map<String, Value>,
    points: &[ScoreObservation],
    question: &Question,
    selection: Option<&Selection>,
) {
    let report = metrics::score(points, level_count(question));
    document.insert("exact_agreement".to_owned(), json!(report.exact_agreement));
    document.insert(
        "adjacent_agreement".to_owned(),
        json!(report.adjacent_agreement),
    );
    document.insert(
        "mean_absolute_error".to_owned(),
        json!(report.mean_absolute_error),
    );
    document.insert(
        "quadratic_weighted_kappa".to_owned(),
        json!(report.quadratic_weighted_kappa),
    );
    document.insert(
        "confusion".to_owned(),
        nested_counts(
            &report
                .confusion
                .iter()
                .map(|(label, row)| {
                    (
                        label.to_string(),
                        row.iter()
                            .map(|(level, count)| (level.to_string(), *count))
                            .collect(),
                    )
                })
                .collect(),
        ),
    );
    document.insert(
        "calibration".to_owned(),
        calibration_value(&report.calibration),
    );
    let exact = points
        .iter()
        .filter(|point| metrics::rounded_level(point.score, level_count(question)) == point.label)
        .count();
    document.insert(
        "headline".to_owned(),
        headline("exact_agreement", exact, points.len()),
    );
    document.insert("coverage_sweep".to_owned(), coverage_value(&report.sweep));
    insert_at_threshold(document, selection, &report.sweep);
}

/// Records what the chosen confidence cut does **on the reported rows**.
fn insert_at_threshold(
    document: &mut serde_json::Map<String, Value>,
    selection: Option<&Selection>,
    sweep: &[CoveragePoint],
) {
    let Some(threshold) = selection.and_then(|choice| choice.threshold) else {
        return;
    };
    // The sweep is built from the reported rows' own observed confidences, so a
    // threshold chosen on the calibration rows need not appear in it. The cut that
    // applies is the largest candidate at or below it, which classifies the reported
    // rows identically.
    let applicable = sweep
        .iter()
        .filter(|point| point.threshold <= threshold)
        .max_by(|left, right| left.threshold.total_cmp(&right.threshold));
    if let Some(point) = applicable {
        document.insert(
            "at_threshold".to_owned(),
            json!({
                "coverage": point.coverage(),
                "abstention": point.abstention(),
                "accuracy_among_covered": point.accuracy_among_covered(),
                "risk_among_covered": point.risk(),
                "covered": point.covered,
            }),
        );
    }
}

/// A proportion with the interval that says how much of it is noise.
///
/// Always reported beside the headline number, because "87%" from forty rows and "87%"
/// from four thousand are different claims and a report that prints them identically
/// invites the wrong one to be acted on.
fn headline(name: &str, successes: usize, total: usize) -> Value {
    let interval = metrics::wilson_interval(successes, total);
    json!({
        "metric": name,
        "value": if total == 0 { None } else {
            #[allow(
                clippy::cast_precision_loss,
                reason = "row counts are bounded by the dataset cap, far below 2^53"
            )]
            Some(successes as f64 / total as f64)
        },
        "interval_95": interval.map(|interval| json!({
            "lower": interval.lower,
            "upper": interval.upper,
            // Named so nobody has to guess which interval it is, and so that a change
            // of method is visible in the document rather than only in the changelog.
            "method": "wilson",
        })),
        "n": total,
    })
}

fn confusion_value(confusion: Confusion) -> Value {
    json!({
        "true_positive": confusion.true_positive,
        "false_positive": confusion.false_positive,
        "true_negative": confusion.true_negative,
        "false_negative": confusion.false_negative,
        "accuracy": confusion.accuracy(),
        "precision": confusion.precision(),
        "recall": confusion.recall(),
        "specificity": confusion.specificity(),
        "f1": confusion.f1(),
        // The mirror of precision, present so a reader can build a two-sided abstention
        // band out of two rows of the sweep. `jev eval` deliberately does not search two
        // dimensions for one; it gives the numbers a band is made of.
        "negative_predictive_value": confusion.negative_predictive_value(),
    })
}

fn coverage_value(sweep: &[CoveragePoint]) -> Value {
    Value::Array(
        sweep
            .iter()
            .map(|point| {
                json!({
                    "threshold": point.threshold,
                    "coverage": point.coverage(),
                    "abstention": point.abstention(),
                    "accuracy_among_covered": point.accuracy_among_covered(),
                    "risk_among_covered": point.risk(),
                    "covered": point.covered,
                })
            })
            .collect(),
    )
}

fn calibration_value(calibration: &metrics::Calibration) -> Value {
    json!({
        "expected_error": calibration.expected_error,
        "bins": calibration.bins.iter().map(|bin| json!({
            "lower": bin.lower,
            "upper": bin.upper,
            "count": bin.count,
            "mean_predicted": bin.mean_predicted,
            "empirical_rate": bin.empirical_rate,
        })).collect::<Vec<_>>(),
    })
}

fn nested_counts(counts: &BTreeMap<String, BTreeMap<String, usize>>) -> Value {
    Value::Object(
        counts
            .iter()
            .map(|(outer, inner)| {
                (
                    outer.clone(),
                    Value::Object(
                        inner
                            .iter()
                            .map(|(key, count)| (key.clone(), json!(count)))
                            .collect(),
                    ),
                )
            })
            .collect(),
    )
}

/// Per-row detail, for `--show-rows`.
///
/// Only the reported rows. The calibration rows are selection-time working, not the
/// estimate, and listing them beside it would invite reading the two as one set.
fn row_details(
    question: &Question,
    id: &str,
    rows: &[LabeledRow],
    outcomes: &[RowOutcome],
    indexes: &[usize],
) -> Value {
    Value::Array(
        indexes
            .iter()
            .filter_map(|index| {
                let (row, outcome) = (rows.get(*index)?, outcomes.get(*index)?);
                let label = row.labels.get(id)?;
                let answer = outcome.answers.get(id);
                let (predicted, correct) = match (answer, label) {
                    (Some(Answer::Noul { noul }), Label::Noul(_)) => {
                        (json!(noul.get()), Value::Null)
                    }
                    (Some(Answer::Choice { choice, .. }), Label::Choice(expected)) => {
                        (json!(choice), json!(choice == expected))
                    }
                    (Some(Answer::Score { score, .. }), Label::Score(expected)) => (
                        json!(score),
                        json!(metrics::rounded_level(*score, level_count(question)) == *expected),
                    ),
                    _ => (Value::Null, Value::Null),
                };
                Some(json!({
                    "id": row.id,
                    "label": label_value(label),
                    "predicted": predicted,
                    "correct": correct,
                }))
            })
            .collect(),
    )
}

fn label_value(label: &Label) -> Value {
    match label {
        Label::Noul(value) => json!(value),
        Label::Choice(name) => json!(name),
        Label::Score(level) => json!(level),
    }
}

/// The evaluation timestamp, as RFC 3339 in UTC.
///
/// Computed from the Unix epoch by hand rather than by adding a date library for one
/// string (`AGENTS.md` §7). The civil-date arithmetic is the standard days-from-epoch
/// inversion and is exercised against known dates in the tests below.
fn timestamp() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    format_rfc3339(seconds)
}

/// Formats seconds since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`.
#[allow(
    clippy::integer_division,
    reason = "every division here is exact floor arithmetic on a civil calendar -- days \
              out of seconds, months out of days -- where truncation is the operation \
              being performed rather than a precision loss. Floating point would be the \
              bug: it cannot represent the era arithmetic exactly and would put a date \
              on the wrong side of midnight."
)]
fn format_rfc3339(seconds: u64) -> String {
    let days = seconds / 86_400;
    let rest = seconds % 86_400;
    let (hour, minute, second) = (rest / 3600, (rest % 3600) / 60, rest % 60);

    // Howard Hinnant's civil_from_days, shifted so the era starts on 0000-03-01.
    let z = i64::try_from(days).unwrap_or(0) + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year + i64::from(month <= 2);

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Writes the report to a file the user named.
fn write_report(path: &Path, document: &Value) -> Result<()> {
    use std::io::Write as _;

    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // A report holds the model's judgments about the user's own examples, which
        // `docs/threat-model.md` lists among the assets worth protecting. The same
        // 0600 the configuration file and `jev map`'s row files get, and for the same
        // reason: left to the umask it would typically be world-readable on a shared
        // machine or a CI runner.
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|error| {
        CliError::usage(format!(
            "cannot open the report file: {}",
            crate::input::io_reason(&error)
        ))
    })?;
    // The same encoder stdout uses, so a report written to a file carries the same
    // escaping -- it is the artifact most likely to be read in a terminal or diffed
    // long after the run.
    let bytes = render_json::encode_line(document)?;
    file.write_all(&bytes).map_err(|error| {
        CliError::io(format!(
            "cannot write the report file: {}",
            crate::input::io_reason(&error)
        ))
    })
}

// --- Human output ---------------------------------------------------------------------

/// Renders the report for a person.
///
/// Deliberately a handful of numbers, not the whole document. Human output is not a
/// stable interface (`docs/cli-contract.md`), and a wall of twenty statistics is how a
/// reader ends up acting on the one they recognise rather than the one that matters.
/// Everything computed is in `--output json`.
fn render_text(
    document: &Value,
    questions: &[(QuestionId, Question)],
    objective: Option<(Objective, f64)>,
) -> String {
    let mut out = String::new();
    let at = |path: &[&str]| -> Option<&Value> {
        let mut cursor = document;
        for key in path {
            cursor = cursor.get(*key)?;
        }
        Some(cursor)
    };

    let rows = at(&["split", "reported_rows"])
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let _ = writeln!(
        out,
        "evaluated {rows} labelled row(s) against {}",
        at(&["model"]).and_then(Value::as_array).map_or_else(
            || "the API".to_owned(),
            |models| models
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        )
    );
    if let Some(mode) = at(&["split", "mode"]).and_then(Value::as_str)
        && mode == "seeded"
    {
        let _ = writeln!(
            out,
            "threshold chosen on {} calibration row(s), reported on {rows} held-out row(s)",
            at(&["split", "calibration_rows"])
                .and_then(Value::as_u64)
                .unwrap_or(0)
        );
    }
    let _ = writeln!(out);

    for (id, question) in questions {
        let id = id.as_str();
        let Some(section) = at(&["questions", id]) else {
            continue;
        };
        let _ = write!(out, "{}", render_question(id, question, section, objective));
    }

    let _ = writeln!(
        out,
        "This measures one question, one dataset, and one model version. It does not\n\
         transfer to another of any of the three."
    );
    out
}

/// One question's block of the human report.
fn render_question(
    id: &str,
    question: &Question,
    section: &Value,
    objective: Option<(Objective, f64)>,
) -> String {
    use crate::output::Safe;

    let mut out = String::new();
    let number = |value: Option<&Value>| -> String {
        value
            .and_then(Value::as_f64)
            .map_or_else(|| "--".to_owned(), |value| format!("{value:.3}"))
    };
    let get = |key: &str| section.get(key);
    let _ = writeln!(
        out,
        "{} ({}, n={})",
        Safe::new(id),
        question.kind().as_str(),
        get("n").and_then(Value::as_u64).unwrap_or(0)
    );

    if let Some(headline) = get("headline") {
        let interval = headline.get("interval_95").and_then(|interval| {
            Some(format!(
                " (95% {:.3}-{:.3})",
                interval.get("lower")?.as_f64()?,
                interval.get("upper")?.as_f64()?
            ))
        });
        let _ = writeln!(
            out,
            "  {:<22} {}{}",
            headline
                .get("metric")
                .and_then(Value::as_str)
                .unwrap_or("headline"),
            number(headline.get("value")),
            interval.unwrap_or_default()
        );
    }

    let _ = write!(out, "{}", render_metrics(question, section));

    if let Some((objective, target)) = objective {
        if get("objective_applies").and_then(Value::as_bool) == Some(false) {
            let _ = writeln!(
                out,
                "  no threshold: --objective {objective} does not apply to a {}",
                question.kind().as_str()
            );
        } else if get("threshold_reachable").and_then(Value::as_bool) == Some(false) {
            let _ = writeln!(
                out,
                "  under --objective {objective}, {}",
                unreachable_reason(Some((objective, target)))
            );
        } else if let Some(threshold) = get("threshold").and_then(Value::as_f64) {
            // The objective is always named beside the number. A threshold is
            // optimal only with respect to the thing it was optimized for, and
            // printing it bare is how it gets quoted as though it were a property
            // of the question.
            let _ = writeln!(
                out,
                "  threshold {threshold:.3} on {}.{}, under --objective {objective}",
                Safe::new(id),
                objective.field()
            );
            let _ = writeln!(
                out,
                "    gate with: --require '{}.{} >= {threshold:.3}'",
                Safe::new(id),
                objective.field()
            );
        }
    }
    let _ = writeln!(out);
    out
}

/// The per-type metric lines of a question's block.
///
/// A handful, chosen for deciding something rather than for completeness. The rest is
/// in `--output json`, which is where a reader who wants the confusion matrix or the
/// reliability bins should be looking anyway.
fn render_metrics(question: &Question, section: &Value) -> String {
    let mut out = String::new();
    let number = |value: Option<&Value>| -> String {
        value
            .and_then(Value::as_f64)
            .map_or_else(|| "--".to_owned(), |value| format!("{value:.3}"))
    };
    let get = |key: &str| section.get(key);
    match question {
        Question::Noul { .. } => {
            if let Some(table) = get("at_threshold") {
                for (label, key) in [
                    ("precision", "precision"),
                    ("recall", "recall"),
                    ("f1", "f1"),
                ] {
                    let _ = writeln!(out, "  {label:<22} {}", number(table.get(key)));
                }
            } else {
                // Accuracy, precision, and recall are all properties of a *decision*,
                // and a noul is a probability until a cut turns it into one. Saying so
                // is better than either omitting them silently or picking 0.5 -- which
                // would be inventing exactly the threshold this command exists to stop
                // people inventing.
                let _ = writeln!(
                    out,
                    "  {:<22} needs a threshold; add --objective to choose one",
                    "accuracy"
                );
            }
            let _ = writeln!(
                out,
                "  {:<22} {}",
                "brier score",
                number(get("brier_score"))
            );
            let _ = writeln!(
                out,
                "  {:<22} {}",
                "calibration error",
                number(
                    section
                        .get("calibration")
                        .and_then(|c| c.get("expected_error"))
                )
            );
        }
        Question::Choice { .. } => {
            let _ = writeln!(out, "  {:<22} {}", "macro f1", number(get("macro_f1")));
            let _ = writeln!(
                out,
                "  {:<22} {}",
                "brier score",
                number(get("brier_score"))
            );
        }
        Question::Score { .. } => {
            let _ = writeln!(
                out,
                "  {:<22} {}",
                "adjacent agreement",
                number(get("adjacent_agreement"))
            );
            let _ = writeln!(
                out,
                "  {:<22} {}",
                "mean absolute error",
                number(get("mean_absolute_error"))
            );
        }
    }
    out
}

// --- Dry run ---------------------------------------------------------------------------

/// Shows what would be sent, and sends nothing.
fn dry_run(
    session: &mut Session<'_>,
    questions: &[(QuestionId, Question)],
    plan: &Plan,
    model: &ModelId,
) -> Result<u8> {
    /// A preview is for confirming the shape, not for reading the data.
    const SAMPLE_LIMIT: usize = 3;

    let mut bodies = Vec::new();
    let mut url = None;
    let mut headers: Vec<String> = Vec::new();
    for row in plan.rows.iter().take(SAMPLE_LIMIT) {
        let request = EvaluationRequest::new(row.state.clone(), model.clone(), questions.to_vec())
            .map_err(|error| CliError::usage(error.to_string()))?;
        // The same builder a real row uses, so a preview cannot describe a request the
        // run would not send.
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
            "id": row.id,
            "body": body,
            "body_bytes": built.body.len(),
        }));
    }

    let url = url.unwrap_or_else(|| {
        session
            .context
            .endpoint
            .value
            .url_for(jev_client::SYSTEM_ONE_PATH)
    });
    let document = json!({
        "schema": DRY_RUN_SCHEMA,
        "method": "POST",
        "url": url,
        "headers": headers,
        "credential": session.credential_availability(),
        // `records`, not `rows`: this is the shared `jev.dry-run/v1` document, and
        // `jev map --dry-run` already calls the same thing -- how many requests would
        // be sent -- by that name. One schema with two names for one concept is a
        // schema a consumer has to special-case per command.
        "records": plan.rows.len(),
        "split": {
            "mode": plan.mode,
            "seed": plan.seed,
            "test_fraction": plan.test_fraction,
            "calibration_rows": plan.calibration.len(),
            "reported_rows": plan.reported.len(),
        },
        "sample": bodies,
        "sample_truncated": plan.rows.len() > SAMPLE_LIMIT,
        "sent": false,
    });
    if session.json() {
        render_json::write_document(session.out, &document)?;
    } else {
        let rendered = serde_json::to_string_pretty(&document)
            .map_err(|error| CliError::internal(error.to_string()))?;
        let rendered = render_json::escape_terminal_hazards_pretty(&rendered);
        render_json::write_all(session.out, rendered.as_bytes())?;
        render_json::write_all(session.out, b"\n")?;
    }
    session.warn(&format!(
        "dry run: nothing was sent; {} row(s) would be evaluated",
        plan.rows.len()
    ));
    Ok(exit::SUCCESS)
}
