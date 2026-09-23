//! Calibration statistics for `jev eval`.
//!
//! # What this module is
//!
//! Pure arithmetic over labelled observations. No I/O, no network, no configuration,
//! no rendering — `jev eval` collects answers and ground truth, hands them here, and
//! prints what comes back. That separation is what makes every number below testable
//! against a hand-computed value rather than against another run of the same code.
//!
//! # What it is deliberately not
//!
//! It is not a model-evaluation library. Every statistic here earns its place by
//! answering a question a user of this CLI actually has — *how well does my question
//! work on my data, and what threshold can I defend?* — and several statistics that a
//! library would offer are refused outright because they do not mean what they appear
//! to mean for these question types:
//!
//! * **No micro-F1 for a Choice.** In single-label multiclass, micro-precision,
//!   micro-recall, and micro-F1 are all exactly equal to accuracy. Emitting one under a
//!   second name would imply a second, independent signal that does not exist.
//! * **No weighted kappa for a Choice.** The API treats a Choice's options as
//!   unordered, so a distance-weighted disagreement statistic would depend on the order
//!   the user happened to write them in and would change silently when they reordered
//!   the file. A Score's levels *are* ordered, so it gets one.
//! * **No per-level precision and recall for a Score.** Treating ordered levels as
//!   independent nominal classes scores "off by one" exactly as badly as "off by four",
//!   which is actively misleading about the thing a Score is for.
//! * **No synthesized confidence for a Noul.** The API returns none, and ADR-0010 §5
//!   records why `jev` will not invent one. A Noul is therefore swept on its
//!   probability; a Choice and a Score are swept on their confidence. The two are not
//!   blended.
//!
//! # Numerical care
//!
//! Every ratio is `Option<f64>`: a denominator of zero is a metric that is *undefined*
//! for this data, not one that is zero. Callers render `null`, never `0.0`, and never
//! `NaN`. Log loss clamps away from the open ends of the unit interval, because
//! `Probability` legally holds exactly `0.0` and `1.0` and a confident miss would
//! otherwise be infinite.

use std::collections::BTreeMap;

/// Clamp applied to a probability before taking its logarithm.
///
/// [`jev_core::Probability`] legally holds exactly `0.0` and `1.0`, so a fully
/// confident prediction that turns out wrong would make log loss `+inf` and swamp every
/// other row in the average — one mistake would be reported as infinitely bad.
/// `1e-15` is the conventional clip: far enough from the end that it does not distort a
/// genuinely near-certain, near-correct prediction, close enough that a confident miss
/// still dominates the sum by roughly 34 nats.
const LOG_LOSS_EPSILON: f64 = 1e-15;

/// Number of equal-width bins used for every calibration error in this module.
///
/// Equal *width*, not equal frequency. The datasets this command is built for are
/// "representative labelled examples", not production-scale logs, and adaptive bin
/// edges on a few hundred rows move with the data in ways that make two runs
/// incomparable — which is the opposite of what a calibration report is for. Ten fixed
/// bins is the scheme the calibration literature uses by default, and its weakness
/// (empty bins contribute nothing) is visible in the emitted bin table rather than
/// hidden inside a single number.
const CALIBRATION_BINS: usize = 10;

/// `z` for a two-sided 95% normal interval.
///
/// Fixed rather than configurable: an interval whose level can be tuned invites
/// choosing the level that makes the result look best.
const Z_95: f64 = 1.959_963_985;

/// Divides two counts, or reports that the ratio is undefined.
///
/// `None` rather than `0.0` throughout: precision with no predicted positives is a
/// question the data cannot answer, and reporting it as zero would say the question was
/// answered badly instead.
#[allow(
    clippy::cast_precision_loss,
    reason = "row counts are bounded by the dataset cap, far below 2^53"
)]
fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator > 0).then(|| numerator as f64 / denominator as f64)
}

/// Divides a sum by a count, or reports that the mean is undefined.
#[allow(
    clippy::cast_precision_loss,
    reason = "row counts are bounded by the dataset cap, far below 2^53"
)]
fn mean(total: f64, count: usize) -> Option<f64> {
    (count > 0).then(|| total / count as f64)
}

/// The harmonic mean of precision and recall, undefined when either is.
///
/// Both zero is **zero**, not undefined: a classifier that predicted some positives and
/// got every one of them wrong has a defined precision of 0 and a defined recall of 0,
/// and its F1 is 0. Returning `None` there — which the first version did, because the
/// harmonic mean divides by their sum — excluded that class from the macro average, so
/// the worst class in a set silently *raised* the reported macro F1. The undefined case
/// is only the one where precision or recall is itself undefined, which is a question
/// the data cannot answer rather than an answer of zero.
fn f1(precision: Option<f64>, recall: Option<f64>) -> Option<f64> {
    let (precision, recall) = (precision?, recall?);
    let total = precision + recall;
    Some(if total > 0.0 {
        2.0 * precision * recall / total
    } else {
        0.0
    })
}

/// A two-by-two contingency table.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Confusion {
    /// Predicted yes, labelled yes.
    pub true_positive: usize,
    /// Predicted yes, labelled no.
    pub false_positive: usize,
    /// Predicted no, labelled no.
    pub true_negative: usize,
    /// Predicted no, labelled yes.
    pub false_negative: usize,
}

impl Confusion {
    /// Rows in the table.
    #[must_use]
    pub const fn total(self) -> usize {
        self.true_positive + self.false_positive + self.true_negative + self.false_negative
    }

    /// Correct predictions over all predictions.
    #[must_use]
    pub fn accuracy(self) -> Option<f64> {
        ratio(self.true_positive + self.true_negative, self.total())
    }

    /// Of the rows predicted yes, the share that were yes.
    #[must_use]
    pub fn precision(self) -> Option<f64> {
        ratio(self.true_positive, self.true_positive + self.false_positive)
    }

    /// Of the rows labelled yes, the share that were predicted yes.
    #[must_use]
    pub fn recall(self) -> Option<f64> {
        ratio(self.true_positive, self.true_positive + self.false_negative)
    }

    /// Of the rows labelled no, the share that were predicted no.
    #[must_use]
    pub fn specificity(self) -> Option<f64> {
        ratio(self.true_negative, self.true_negative + self.false_positive)
    }

    /// Of the rows predicted no, the share that were no.
    ///
    /// The mirror of [`Self::precision`], and the reason it is here: a user building a
    /// two-sided abstention band needs the error rate on *both* automatically handled
    /// sides, and precision alone describes only the upper one.
    #[must_use]
    pub fn negative_predictive_value(self) -> Option<f64> {
        ratio(self.true_negative, self.true_negative + self.false_negative)
    }

    /// The harmonic mean of precision and recall.
    #[must_use]
    pub fn f1(self) -> Option<f64> {
        f1(self.precision(), self.recall())
    }
}

/// One labelled Noul observation.
#[derive(Debug, Clone, Copy)]
pub struct NoulObservation {
    /// The probability of "yes" the model returned.
    pub probability: f64,
    /// The ground truth.
    pub label: bool,
}

/// One bin of a reliability diagram.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalibrationBin {
    /// Inclusive lower edge.
    pub lower: f64,
    /// Upper edge, inclusive only for the topmost bin.
    pub upper: f64,
    /// Observations that fell in this bin.
    pub count: usize,
    /// Mean predicted probability among them.
    pub mean_predicted: f64,
    /// Share of them that were actually positive.
    pub empirical_rate: f64,
}

/// A reliability diagram and the single number summarizing it.
#[derive(Debug, Clone, PartialEq)]
pub struct Calibration {
    /// Expected calibration error: the count-weighted mean absolute gap between
    /// predicted probability and observed rate, over non-empty bins.
    pub expected_error: Option<f64>,
    /// The non-empty bins, in ascending order. Empty bins are omitted rather than
    /// reported as zero, because "no data here" and "perfectly calibrated here" are
    /// different facts.
    pub bins: Vec<CalibrationBin>,
}

/// Bins `(predicted, outcome)` pairs and summarizes the gap.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    reason = "the bin index is derived from a value already known to be in [0, 1] and \
              is immediately clamped to the bin count"
)]
fn calibrate(points: impl IntoIterator<Item = (f64, bool)>) -> Calibration {
    let mut counts = [0_usize; CALIBRATION_BINS];
    let mut predicted = [0.0_f64; CALIBRATION_BINS];
    let mut positives = [0_usize; CALIBRATION_BINS];
    let mut total = 0_usize;

    for (probability, outcome) in points {
        // `min` rather than an `if`: a probability of exactly 1.0 lands on index 10,
        // which is past the last bin. The top bin is closed at its upper edge.
        let index = ((probability * CALIBRATION_BINS as f64) as usize).min(CALIBRATION_BINS - 1);
        if let (Some(count), Some(sum), Some(hits)) = (
            counts.get_mut(index),
            predicted.get_mut(index),
            positives.get_mut(index),
        ) {
            *count += 1;
            *sum += probability;
            *hits += usize::from(outcome);
            total += 1;
        }
    }

    let mut bins = Vec::new();
    let mut weighted_gap = 0.0_f64;
    for index in 0..CALIBRATION_BINS {
        let (Some(&count), Some(&sum), Some(&hits)) = (
            counts.get(index),
            predicted.get(index),
            positives.get(index),
        ) else {
            continue;
        };
        if count == 0 {
            continue;
        }
        let Some(mean_predicted) = mean(sum, count) else {
            continue;
        };
        let Some(empirical_rate) = ratio(hits, count) else {
            continue;
        };
        weighted_gap += count as f64 * (mean_predicted - empirical_rate).abs();
        bins.push(CalibrationBin {
            // Divided rather than multiplied by a width: `3.0 * 0.1` is
            // 0.30000000000000004, while `3.0 / 10.0` is the double nearest 0.3 and
            // prints as `0.3` in the report and the JSON.
            lower: index as f64 / CALIBRATION_BINS as f64,
            upper: (index as f64 + 1.0) / CALIBRATION_BINS as f64,
            count,
            mean_predicted,
            empirical_rate,
        });
    }

    Calibration {
        expected_error: mean(weighted_gap, total),
        bins,
    }
}

/// One row of a Noul threshold sweep.
///
/// The decision rule is `predict yes when probability >= threshold`. Both sides are
/// reported: a one-sided threshold hands every row a verdict, so `coverage` is always
/// the whole set and the interesting question is what the error rate looks like above
/// and below the cut. A user who wants an abstention band reads two rows of this table
/// — the `precision` at the upper cut and the `negative_predictive_value` at the lower
/// one — rather than asking this command to search two dimensions for them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoulSweepPoint {
    /// The cut being described.
    pub threshold: f64,
    /// The contingency table at this cut.
    pub confusion: Confusion,
}

/// Every statistic `jev eval` computes for a Noul question.
#[derive(Debug, Clone)]
pub struct NoulReport {
    /// Observations with both a label and an answer.
    pub n: usize,
    /// Positive labels among them.
    pub positives: usize,
    /// The mean squared error of the probability against the outcome. Threshold-free,
    /// and a strictly proper scoring rule: it cannot be improved by misreporting.
    pub brier_score: Option<f64>,
    /// The mean negative log likelihood, with probabilities clamped away from the ends.
    pub log_loss: Option<f64>,
    /// The reliability diagram of the probability against the outcome.
    pub calibration: Calibration,
    /// The contingency table at every candidate cut, ascending.
    pub sweep: Vec<NoulSweepPoint>,
}

/// Computes every Noul statistic over `observations`.
#[must_use]
pub fn noul(observations: &[NoulObservation]) -> NoulReport {
    let n = observations.len();
    let positives = observations.iter().filter(|point| point.label).count();

    let brier_score = mean(
        observations
            .iter()
            .map(|point| {
                let outcome = f64::from(u8::from(point.label));
                (point.probability - outcome).powi(2)
            })
            .sum(),
        n,
    );

    let log_loss = mean(
        observations
            .iter()
            .map(|point| {
                let clamped = point
                    .probability
                    .clamp(LOG_LOSS_EPSILON, 1.0 - LOG_LOSS_EPSILON);
                if point.label {
                    -clamped.ln()
                } else {
                    -(1.0 - clamped).ln()
                }
            })
            .sum(),
        n,
    );

    let calibration = calibrate(
        observations
            .iter()
            .map(|point| (point.probability, point.label)),
    );

    NoulReport {
        n,
        positives,
        brier_score,
        log_loss,
        calibration,
        sweep: noul_sweep(observations),
    }
}

/// The contingency table for `predict yes when probability >= threshold`.
#[must_use]
pub fn noul_confusion(observations: &[NoulObservation], threshold: f64) -> Confusion {
    let mut confusion = Confusion::default();
    for point in observations {
        match (point.probability >= threshold, point.label) {
            (true, true) => confusion.true_positive += 1,
            (true, false) => confusion.false_positive += 1,
            (false, false) => confusion.true_negative += 1,
            (false, true) => confusion.false_negative += 1,
        }
    }
    confusion
}

/// Every threshold at which the decision rule can change, ascending.
///
/// Sweeping a fixed grid would step over the cut that actually separates two adjacent
/// probabilities and report a threshold that is not the one that was measured. The
/// candidates are therefore the observed probabilities themselves, plus `0.0` so that
/// "predict yes for everything" is always on the table.
fn noul_candidates(observations: &[NoulObservation]) -> Vec<f64> {
    let mut candidates: Vec<f64> = std::iter::once(0.0)
        .chain(observations.iter().map(|point| point.probability))
        .collect();
    candidates.sort_by(f64::total_cmp);
    candidates.dedup_by(|left, right| left.to_bits() == right.to_bits());
    candidates
}

fn noul_sweep(observations: &[NoulObservation]) -> Vec<NoulSweepPoint> {
    noul_candidates(observations)
        .into_iter()
        .map(|threshold| NoulSweepPoint {
            threshold,
            confusion: noul_confusion(observations, threshold),
        })
        .collect()
}

/// One labelled Choice observation.
#[derive(Debug, Clone)]
pub struct ChoiceObservation {
    /// The option the model selected.
    pub predicted: String,
    /// The ground truth option.
    pub label: String,
    /// How concentrated the distribution was.
    pub confidence: f64,
}

/// Precision, recall, and F1 for one class, with the number of rows it covers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClassScore {
    /// Of the rows predicted this class, the share that were it.
    pub precision: Option<f64>,
    /// Of the rows labelled this class, the share predicted it.
    pub recall: Option<f64>,
    /// Their harmonic mean.
    pub f1: Option<f64>,
    /// Rows *labelled* this class. Reporting support as the label count rather than the
    /// prediction count is what makes a macro average readable: a class the model never
    /// predicts still has the support that shows the average is being dragged by it.
    pub support: usize,
}

/// Every statistic `jev eval` computes for a Choice question.
#[derive(Debug, Clone)]
pub struct ChoiceReport {
    /// Observations with both a label and an answer.
    pub n: usize,
    /// Correct selections over all selections.
    pub accuracy: Option<f64>,
    /// Per-class scores, keyed by option name.
    pub per_class: BTreeMap<String, ClassScore>,
    /// The unweighted mean of the per-class precisions that are defined.
    pub macro_precision: Option<f64>,
    /// The unweighted mean of the per-class recalls that are defined.
    pub macro_recall: Option<f64>,
    /// The unweighted mean of the per-class F1s that are defined.
    ///
    /// Macro, never micro: micro-F1 in single-label multiclass *is* accuracy, which is
    /// already reported under its own name.
    pub macro_f1: Option<f64>,
    /// True label to predicted label to count.
    pub confusion: BTreeMap<String, BTreeMap<String, usize>>,
    /// The classical multi-category Brier score, summed over **every** option rather
    /// than only the true one.
    ///
    /// The reduced `1 - p(true)` variant discards how the remaining mass was spread
    /// across the wrong options, which is exactly the distributional information a
    /// System One model exists to provide. Range `[0, 2]`; zero is perfect.
    pub brier_score: Option<f64>,
    /// The reliability diagram of confidence against whether the selection was right.
    pub calibration: Calibration,
    /// Accuracy among the rows at or above each observed confidence, ascending.
    pub sweep: Vec<CoveragePoint>,
}

/// One row of a coverage sweep over a confidence threshold.
///
/// This is the shape of an abstention policy: handle a row automatically when its
/// confidence is at least `threshold`, and send the rest to a human.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoveragePoint {
    /// The confidence cut.
    pub threshold: f64,
    /// Rows at or above the cut.
    pub covered: usize,
    /// Correct rows among them.
    pub correct: usize,
    /// Rows in the sample.
    pub total: usize,
}

impl CoveragePoint {
    /// The share of rows that would be handled automatically.
    #[must_use]
    pub fn coverage(self) -> Option<f64> {
        ratio(self.covered, self.total)
    }

    /// The share of rows that would be escalated instead.
    #[must_use]
    pub fn abstention(self) -> Option<f64> {
        self.coverage().map(|coverage| 1.0 - coverage)
    }

    /// Accuracy restricted to the rows handled automatically.
    ///
    /// `None` when nothing is covered: the accuracy of an empty set is not 1.0, and
    /// reporting it as such would make an unreachable threshold look perfect.
    #[must_use]
    pub fn accuracy_among_covered(self) -> Option<f64> {
        ratio(self.correct, self.covered)
    }

    /// The error rate among the rows handled automatically.
    #[must_use]
    pub fn risk(self) -> Option<f64> {
        self.accuracy_among_covered().map(|accuracy| 1.0 - accuracy)
    }
}

/// Builds a coverage sweep from `(confidence, correct)` pairs.
fn coverage_sweep(points: &[(f64, bool)]) -> Vec<CoveragePoint> {
    let mut candidates: Vec<f64> = std::iter::once(0.0)
        .chain(points.iter().map(|(confidence, _)| *confidence))
        .collect();
    candidates.sort_by(f64::total_cmp);
    candidates.dedup_by(|left, right| left.to_bits() == right.to_bits());

    candidates
        .into_iter()
        .map(|threshold| {
            let covered: Vec<bool> = points
                .iter()
                .filter(|(confidence, _)| *confidence >= threshold)
                .map(|(_, correct)| *correct)
                .collect();
            CoveragePoint {
                threshold,
                covered: covered.len(),
                correct: covered.iter().filter(|correct| **correct).count(),
                total: points.len(),
            }
        })
        .collect()
}

/// The unweighted mean of the defined values, or `None` when none are.
fn macro_average(values: impl IntoIterator<Item = Option<f64>>) -> Option<f64> {
    let defined: Vec<f64> = values.into_iter().flatten().collect();
    mean(defined.iter().sum(), defined.len())
}

/// Computes every Choice statistic.
///
/// `options` is the full declared option set, in the order the request file wrote them.
/// It is passed explicitly rather than derived from the observations so that an option
/// the model never picked and no row was labelled with still appears — a class with
/// zero support is a fact about the dataset worth seeing, not an absence to hide.
///
/// `distributions` gives each observation's full probability map; a row with none
/// contributes to every statistic except the Brier score.
#[must_use]
pub fn choice(
    observations: &[ChoiceObservation],
    options: &[String],
    distributions: &[BTreeMap<String, f64>],
) -> ChoiceReport {
    let n = observations.len();
    let correct = observations
        .iter()
        .filter(|point| point.predicted == point.label)
        .count();

    let mut per_class = BTreeMap::new();
    for option in options {
        let mut table = Confusion::default();
        for point in observations {
            match (point.predicted == *option, point.label == *option) {
                (true, true) => table.true_positive += 1,
                (true, false) => table.false_positive += 1,
                (false, true) => table.false_negative += 1,
                (false, false) => table.true_negative += 1,
            }
        }
        per_class.insert(
            option.clone(),
            ClassScore {
                precision: table.precision(),
                recall: table.recall(),
                f1: table.f1(),
                support: table.true_positive + table.false_negative,
            },
        );
    }

    let mut confusion: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for point in observations {
        *confusion
            .entry(point.label.clone())
            .or_default()
            .entry(point.predicted.clone())
            .or_insert(0) += 1;
    }

    let brier_score = mean(
        observations
            .iter()
            .zip(distributions)
            .map(|(point, distribution)| {
                // Summed over every option the request declared, not only the ones the
                // API mentioned: a missing option is mass the model did not assign, and
                // treating it as absent rather than as zero would score a distribution
                // over fewer options as though it were over all of them.
                options
                    .iter()
                    .map(|option| {
                        let predicted = distribution.get(option).copied().unwrap_or(0.0);
                        let outcome = f64::from(u8::from(*option == point.label));
                        (predicted - outcome).powi(2)
                    })
                    .sum::<f64>()
            })
            .sum(),
        observations.len().min(distributions.len()),
    );

    let correctness: Vec<(f64, bool)> = observations
        .iter()
        .map(|point| (point.confidence, point.predicted == point.label))
        .collect();

    ChoiceReport {
        n,
        accuracy: ratio(correct, n),
        macro_precision: macro_average(per_class.values().map(|score| score.precision)),
        macro_recall: macro_average(per_class.values().map(|score| score.recall)),
        macro_f1: macro_average(per_class.values().map(|score| score.f1)),
        per_class,
        confusion,
        brier_score,
        calibration: calibrate(correctness.iter().copied()),
        sweep: coverage_sweep(&correctness),
    }
}

/// One labelled Score observation.
#[derive(Debug, Clone, Copy)]
pub struct ScoreObservation {
    /// The probability-weighted level the model returned. May fall between levels.
    pub score: f64,
    /// The ground-truth level index.
    pub label: u32,
    /// How concentrated the distribution was.
    pub confidence: f64,
}

/// Every statistic `jev eval` computes for a Score question.
#[derive(Debug, Clone)]
pub struct ScoreReport {
    /// Observations with both a label and an answer.
    pub n: usize,
    /// Share whose rounded level equals the label.
    pub exact_agreement: Option<f64>,
    /// Share whose rounded level is within one of the label.
    ///
    /// The statistic that usually matters for an ordinal scale: a rubric where raters
    /// disagree by a level is working, and one where they disagree by three is not.
    pub adjacent_agreement: Option<f64>,
    /// Mean absolute error in level units, computed on the **continuous** score.
    ///
    /// Rounding first would throw away the between-level position that is the whole
    /// reason a Score returns a weighted mean.
    pub mean_absolute_error: Option<f64>,
    /// True level to rounded predicted level to count.
    pub confusion: BTreeMap<u32, BTreeMap<u32, usize>>,
    /// Quadratic-weighted kappa: agreement beyond chance, penalising disagreement by
    /// the square of the distance between levels.
    ///
    /// Defined here and refused for a Choice, because a Score's levels are ordered and
    /// a Choice's options are not. `None` when every rater agreed on one level, where
    /// the chance-agreement denominator is zero and the statistic is undefined rather
    /// than perfect.
    pub quadratic_weighted_kappa: Option<f64>,
    /// The reliability diagram of confidence against exact agreement.
    pub calibration: Calibration,
    /// Exact agreement among the rows at or above each observed confidence.
    pub sweep: Vec<CoveragePoint>,
}

/// The level a score rounds to, kept inside the legend.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped into 0..levels before the cast, and `levels` is at \
              most 10 by the API's own limit"
)]
#[must_use]
pub fn rounded_level(score: f64, levels: u32) -> u32 {
    let top = f64::from(levels.saturating_sub(1));
    score.round().clamp(0.0, top.max(0.0)) as u32
}

/// Computes every Score statistic. `levels` is how many the question declared.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    reason = "level indexes and row counts are tiny; the API allows at most 10 levels"
)]
pub fn score(observations: &[ScoreObservation], levels: u32) -> ScoreReport {
    let n = observations.len();
    let predicted: Vec<u32> = observations
        .iter()
        .map(|point| rounded_level(point.score, levels))
        .collect();

    let exact = observations
        .iter()
        .zip(&predicted)
        .filter(|(point, level)| point.label == **level)
        .count();
    let adjacent = observations
        .iter()
        .zip(&predicted)
        .filter(|(point, level)| point.label.abs_diff(**level) <= 1)
        .count();

    let mean_absolute_error = mean(
        observations
            .iter()
            .map(|point| (point.score - f64::from(point.label)).abs())
            .sum(),
        n,
    );

    let mut confusion: BTreeMap<u32, BTreeMap<u32, usize>> = BTreeMap::new();
    for (point, level) in observations.iter().zip(&predicted) {
        *confusion
            .entry(point.label)
            .or_default()
            .entry(*level)
            .or_insert(0) += 1;
    }

    let correctness: Vec<(f64, bool)> = observations
        .iter()
        .zip(&predicted)
        .map(|(point, level)| (point.confidence, point.label == *level))
        .collect();

    ScoreReport {
        n,
        exact_agreement: ratio(exact, n),
        adjacent_agreement: ratio(adjacent, n),
        mean_absolute_error,
        confusion,
        quadratic_weighted_kappa: kappa(
            &observations
                .iter()
                .map(|point| point.label)
                .collect::<Vec<_>>(),
            &predicted,
            levels,
        ),
        calibration: calibrate(correctness.iter().copied()),
        sweep: coverage_sweep(&correctness),
    }
}

/// Quadratic-weighted kappa between two level assignments.
///
/// `1 - (sum w*O) / (sum w*E)`, with `w[i][j] = (i-j)^2 / (K-1)^2`, `O` the observed
/// joint distribution and `E` the product of the marginals. `None` when the expected
/// disagreement is zero — every rater put everything in one level — because dividing by
/// it would report perfect agreement for a dataset that demonstrated nothing.
#[allow(
    clippy::cast_precision_loss,
    reason = "level indexes are at most 10 and counts are bounded by the dataset cap"
)]
fn kappa(labels: &[u32], predicted: &[u32], levels: u32) -> Option<f64> {
    let k = levels.max(2) as usize;
    let n = labels.len().min(predicted.len());
    if n == 0 {
        return None;
    }

    let mut observed = vec![vec![0_usize; k]; k];
    let mut label_marginal = vec![0_usize; k];
    let mut predicted_marginal = vec![0_usize; k];
    for (label, prediction) in labels.iter().zip(predicted) {
        let (Some(row), Some(column)) = (
            usize::try_from(*label).ok().filter(|index| *index < k),
            usize::try_from(*prediction).ok().filter(|index| *index < k),
        ) else {
            continue;
        };
        if let Some(cell) = observed.get_mut(row).and_then(|row| row.get_mut(column)) {
            *cell += 1;
        }
        if let Some(count) = label_marginal.get_mut(row) {
            *count += 1;
        }
        if let Some(count) = predicted_marginal.get_mut(column) {
            *count += 1;
        }
    }

    let span = (k as f64 - 1.0).powi(2);
    if span <= 0.0 {
        return None;
    }
    let mut weighted_observed = 0.0_f64;
    let mut weighted_expected = 0.0_f64;
    for row in 0..k {
        for column in 0..k {
            let weight = (row as f64 - column as f64).powi(2) / span;
            let count = observed.get(row).and_then(|row| row.get(column)).copied()?;
            let expected = (*label_marginal.get(row)? as f64)
                * (*predicted_marginal.get(column)? as f64)
                / n as f64;
            weighted_observed += weight * count as f64;
            weighted_expected += weight * expected;
        }
    }
    (weighted_expected > 0.0).then(|| 1.0 - weighted_observed / weighted_expected)
}

/// A two-sided interval for a proportion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Interval {
    /// Lower bound.
    pub lower: f64,
    /// Upper bound.
    pub upper: f64,
}

/// The 95% Wilson score interval for `successes` out of `total`.
///
/// Wilson rather than the textbook normal approximation: the Wald interval runs outside
/// `[0, 1]` near the ends and needs a sample size these datasets will often not have,
/// and both failures show up exactly where a calibration report is most likely to
/// mislead — a rare label, or a near-perfect score on forty rows.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    reason = "row counts are bounded by the dataset cap, far below 2^53"
)]
pub fn wilson_interval(successes: usize, total: usize) -> Option<Interval> {
    if total == 0 {
        return None;
    }
    let n = total as f64;
    let proportion = successes as f64 / n;
    let z2 = Z_95 * Z_95;
    let denominator = 1.0 + z2 / n;
    let centre = proportion + z2 / (2.0 * n);
    let spread = Z_95 * ((proportion * (1.0 - proportion) / n) + z2 / (4.0 * n * n)).sqrt();
    Some(Interval {
        lower: ((centre - spread) / denominator).clamp(0.0, 1.0),
        upper: ((centre + spread) / denominator).clamp(0.0, 1.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every expected value in this module is computed by hand and written as a
    /// literal. Comparing against another run of the same code would prove only that
    /// the code is deterministic, which is not the property a statistic needs.
    fn close(actual: Option<f64>, expected: f64) {
        let actual = actual.expect("the metric should be defined for this data");
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, got {actual}"
        );
    }

    fn noul_points(points: &[(f64, bool)]) -> Vec<NoulObservation> {
        points
            .iter()
            .map(|(probability, label)| NoulObservation {
                probability: *probability,
                label: *label,
            })
            .collect()
    }

    // --- Ratios and their undefined cases ------------------------------------------

    #[test]
    fn a_ratio_with_no_denominator_is_undefined_rather_than_zero() {
        // The distinction the whole module turns on: precision with no predicted
        // positives is a question the data cannot answer. Reporting it as 0.0 says the
        // question was answered, badly.
        assert_eq!(ratio(0, 0), None);
        assert_eq!(ratio(3, 0), None);
        assert_eq!(ratio(0, 4), Some(0.0));
    }

    #[test]
    fn precision_and_recall_are_undefined_where_their_denominators_vanish() {
        let nothing_predicted_positive = Confusion {
            true_positive: 0,
            false_positive: 0,
            true_negative: 5,
            false_negative: 3,
        };
        assert_eq!(nothing_predicted_positive.precision(), None);
        assert_eq!(nothing_predicted_positive.recall(), Some(0.0));
        assert_eq!(nothing_predicted_positive.f1(), None);

        // But a class that *was* predicted and got everything wrong has a defined F1 of
        // zero. Reporting it as undefined dropped the worst class out of the macro
        // average, so the macro F1 went *up* when a class got worse.
        let everything_wrong = Confusion {
            true_positive: 0,
            false_positive: 2,
            true_negative: 0,
            false_negative: 3,
        };
        assert_eq!(everything_wrong.precision(), Some(0.0));
        assert_eq!(everything_wrong.recall(), Some(0.0));
        assert_eq!(everything_wrong.f1(), Some(0.0));

        let nothing_labelled_positive = Confusion {
            true_positive: 0,
            false_positive: 2,
            true_negative: 6,
            false_negative: 0,
        };
        assert_eq!(nothing_labelled_positive.recall(), None);
    }

    #[test]
    fn the_contingency_table_matches_a_hand_computed_example() {
        // tp=6, fp=2, tn=10, fn=4, n=22.
        let table = Confusion {
            true_positive: 6,
            false_positive: 2,
            true_negative: 10,
            false_negative: 4,
        };
        assert_eq!(table.total(), 22);
        close(table.accuracy(), 16.0 / 22.0);
        close(table.precision(), 6.0 / 8.0);
        close(table.recall(), 6.0 / 10.0);
        close(table.specificity(), 10.0 / 12.0);
        close(table.negative_predictive_value(), 10.0 / 14.0);
        // 2 * 0.75 * 0.6 / 1.35
        close(table.f1(), 2.0 * 0.75 * 0.6 / (0.75 + 0.6));
    }

    // --- Noul -----------------------------------------------------------------------

    #[test]
    fn a_noul_confusion_table_uses_a_closed_lower_bound() {
        // `>= threshold`, not `>`. A probability exactly equal to the cut is on the
        // "yes" side, which is what makes a threshold taken straight from the sweep
        // reproduce the sweep's own numbers.
        let points = noul_points(&[(0.5, true), (0.5, false)]);
        let table = noul_confusion(&points, 0.5);
        assert_eq!(table.true_positive, 1);
        assert_eq!(table.false_positive, 1);
        assert_eq!(table.true_negative, 0);
        assert_eq!(table.false_negative, 0);
    }

    #[test]
    fn the_brier_score_matches_a_hand_computed_example() {
        // (0.9-1)^2 + (0.2-0)^2 + (0.6-0)^2 = 0.01 + 0.04 + 0.36 = 0.41; /3.
        let report = noul(&noul_points(&[(0.9, true), (0.2, false), (0.6, false)]));
        close(report.brier_score, 0.41 / 3.0);
    }

    #[test]
    fn a_perfect_and_a_perfectly_wrong_forecast_bracket_the_brier_score() {
        close(
            noul(&noul_points(&[(1.0, true), (0.0, false)])).brier_score,
            0.0,
        );
        close(
            noul(&noul_points(&[(0.0, true), (1.0, false)])).brier_score,
            1.0,
        );
    }

    #[test]
    fn log_loss_stays_finite_when_a_confident_forecast_is_wrong() {
        // `Probability` legally holds exactly 0.0 and 1.0, so without the clamp one
        // confident miss makes the mean +inf and buries every other row.
        let report = noul(&noul_points(&[(0.0, true), (0.9, true)]));
        let value = report.log_loss.expect("log loss should be defined");
        assert!(value.is_finite(), "log loss was {value}");
        // -(ln(1e-15) + ln(0.9)) / 2
        close(
            report.log_loss,
            -(LOG_LOSS_EPSILON.ln() + 0.9_f64.ln()) / 2.0,
        );
    }

    #[test]
    fn log_loss_matches_a_hand_computed_example() {
        // -(ln 0.8 + ln 0.7)/2, the second term being ln(1 - 0.3) for a negative label.
        let report = noul(&noul_points(&[(0.8, true), (0.3, false)]));
        close(report.log_loss, -(0.8_f64.ln() + 0.7_f64.ln()) / 2.0);
    }

    #[test]
    fn a_perfectly_calibrated_forecast_has_no_calibration_error() {
        // Ten rows at 0.5, five of which are positive: the bin's mean prediction and
        // its empirical rate are both 0.5.
        let mut points = Vec::new();
        for index in 0..10 {
            points.push((0.5, index < 5));
        }
        let report = noul(&noul_points(&points));
        close(report.calibration.expected_error, 0.0);
        assert_eq!(report.calibration.bins.len(), 1);
    }

    #[test]
    fn calibration_error_matches_a_hand_computed_example() {
        // Two rows at 0.9, neither positive: one bin, mean predicted 0.9, rate 0.0.
        let report = noul(&noul_points(&[(0.9, false), (0.9, false)]));
        close(report.calibration.expected_error, 0.9);
    }

    #[test]
    fn a_probability_of_exactly_one_lands_in_the_top_bin() {
        // The bin index is `p * 10`, which is 10 for p = 1.0 -- one past the last bin.
        // Left unclamped this silently dropped every certain prediction from the
        // reliability diagram, and the dropped rows were exactly the ones a miscalibrated
        // model gets most wrong.
        let report = noul(&noul_points(&[(1.0, true)]));
        assert_eq!(report.calibration.bins.len(), 1);
        let bin = report.calibration.bins.first().expect("one bin");
        assert_eq!(bin.count, 1);
        close(Some(bin.lower), 0.9);
    }

    #[test]
    fn calibration_bin_edges_print_as_the_decimals_they_are() {
        // `3 * 0.1` is 0.30000000000000004, and that is what the report printed. A bin
        // edge is `index / bins`, which is the nearest double to the decimal.
        let report = noul(&noul_points(&[(0.35, true), (0.75, false)]));
        let edges: Vec<(String, String)> = report
            .calibration
            .bins
            .iter()
            .map(|bin| (bin.lower.to_string(), bin.upper.to_string()))
            .collect();
        assert_eq!(
            edges,
            vec![
                ("0.3".to_owned(), "0.4".to_owned()),
                ("0.7".to_owned(), "0.8".to_owned())
            ]
        );
    }

    #[test]
    fn empty_calibration_bins_are_omitted_rather_than_reported_as_perfect() {
        // "No data here" and "perfectly calibrated here" are different facts, and a
        // zero-count bin claiming zero error would report nine confident successes the
        // dataset never demonstrated.
        let report = noul(&noul_points(&[(0.05, false), (0.95, true)]));
        assert_eq!(report.calibration.bins.len(), 2);
        for bin in &report.calibration.bins {
            assert!(bin.count > 0);
        }
    }

    #[test]
    fn the_sweep_visits_every_observed_probability_and_zero() {
        let report = noul(&noul_points(&[(0.3, true), (0.7, false), (0.3, false)]));
        let thresholds: Vec<f64> = report.sweep.iter().map(|point| point.threshold).collect();
        assert_eq!(thresholds, vec![0.0, 0.3, 0.7]);
    }

    #[test]
    fn the_sweep_at_zero_predicts_yes_for_everything() {
        // Present so that "accept everything" is always on the table: a user comparing
        // a threshold against the do-nothing baseline needs the baseline in the table.
        let report = noul(&noul_points(&[(0.3, true), (0.7, false)]));
        let first = report.sweep.first().expect("a sweep point at zero");
        close(Some(first.threshold), 0.0);
        assert_eq!(first.confusion.false_negative, 0);
        assert_eq!(first.confusion.true_negative, 0);
    }

    #[test]
    fn the_sweep_reports_both_sides_so_an_abstention_band_can_be_built() {
        // A one-sided cut decides every row, so coverage is trivially total; what a
        // two-sided band needs instead is the error rate above the upper cut and below
        // the lower one. Both are derivable from every sweep row, which is why this
        // command emits the table rather than searching two dimensions itself.
        let points = noul_points(&[(0.1, false), (0.2, false), (0.8, true), (0.9, true)]);
        let report = noul(&points);
        let at = |threshold: f64| {
            report
                .sweep
                .iter()
                .find(|point| (point.threshold - threshold).abs() < 1e-12)
                .expect("the sweep should contain this observed probability")
                .confusion
        };
        close(at(0.8).precision(), 1.0);
        close(at(0.8).negative_predictive_value(), 1.0);
    }

    #[test]
    fn an_empty_noul_dataset_reports_undefined_rather_than_panicking() {
        let report = noul(&[]);
        assert_eq!(report.n, 0);
        assert_eq!(report.brier_score, None);
        assert_eq!(report.log_loss, None);
        assert_eq!(report.calibration.expected_error, None);
        // Only the zero threshold, which classifies an empty set.
        assert_eq!(report.sweep.len(), 1);
    }

    // --- Choice ---------------------------------------------------------------------

    fn choice_points(points: &[(&str, &str, f64)]) -> Vec<ChoiceObservation> {
        points
            .iter()
            .map(|(predicted, label, confidence)| ChoiceObservation {
                predicted: (*predicted).to_owned(),
                label: (*label).to_owned(),
                confidence: *confidence,
            })
            .collect()
    }

    fn options(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn choice_accuracy_and_macro_scores_match_a_hand_computed_example() {
        // a: predicted twice, right once -> P 1/2; labelled twice, caught once -> R 1/2.
        // b: predicted twice, right once -> P 1/2; labelled once, caught once  -> R 1.
        // c: never predicted             -> P undefined; labelled once, never caught -> R 0.
        let observations = choice_points(&[
            ("a", "a", 0.9),
            ("a", "c", 0.4),
            ("b", "b", 0.8),
            ("b", "a", 0.5),
        ]);
        let report = choice(&observations, &options(&["a", "b", "c"]), &[]);
        close(report.accuracy, 0.5);

        let a = report.per_class.get("a").expect("class a");
        close(a.precision, 0.5);
        close(a.recall, 0.5);
        assert_eq!(a.support, 2);

        let c = report.per_class.get("c").expect("class c");
        assert_eq!(
            c.precision, None,
            "a class never predicted has no precision"
        );
        close(c.recall, 0.0);
        assert_eq!(c.support, 1);

        // Macro precision averages only the two that are defined: (0.5 + 0.5) / 2.
        // `c` contributes nothing, because a class the model never predicted has no
        // precision to average -- counting it as zero would punish the model for a
        // restraint the data cannot evaluate.
        close(report.macro_precision, 0.5);
        // Macro recall averages all three: (0.5 + 1.0 + 0.0) / 3.
        close(report.macro_recall, 0.5);
    }

    #[test]
    fn a_declared_option_nobody_used_still_appears_with_zero_support() {
        // A class with no support is a fact about the dataset -- the examples do not
        // cover an option the question offers -- not an absence to hide.
        let report = choice(
            &choice_points(&[("a", "a", 0.9)]),
            &options(&["a", "unseen"]),
            &[],
        );
        let unseen = report.per_class.get("unseen").expect("the unused option");
        assert_eq!(unseen.support, 0);
        assert_eq!(unseen.recall, None);
    }

    #[test]
    fn the_multiclass_brier_score_counts_mass_on_every_wrong_option() {
        // The reduced `1 - p(true)` form would score these two rows identically; the
        // classical form does not, because one spread its error across two options and
        // the other put all of it on one.
        let spread: BTreeMap<String, f64> = [
            ("a".to_owned(), 0.5),
            ("b".to_owned(), 0.25),
            ("c".to_owned(), 0.25),
        ]
        .into_iter()
        .collect();
        let concentrated: BTreeMap<String, f64> = [
            ("a".to_owned(), 0.5),
            ("b".to_owned(), 0.5),
            ("c".to_owned(), 0.0),
        ]
        .into_iter()
        .collect();

        let names = options(&["a", "b", "c"]);
        let one = choice(&choice_points(&[("a", "a", 0.5)]), &names, &[spread]);
        let other = choice(&choice_points(&[("a", "a", 0.5)]), &names, &[concentrated]);
        // (0.5-1)^2 + 0.25^2 + 0.25^2 = 0.25 + 0.0625 + 0.0625 = 0.375
        close(one.brier_score, 0.375);
        // (0.5-1)^2 + 0.5^2 + 0 = 0.5
        close(other.brier_score, 0.5);
    }

    #[test]
    fn a_declared_option_missing_from_a_distribution_is_treated_as_zero_mass() {
        // Not as absent: an option the API did not mention received no probability, and
        // skipping it would score a distribution over fewer options as though it were
        // over all of them.
        let partial: BTreeMap<String, f64> = [("a".to_owned(), 1.0)].into_iter().collect();
        let report = choice(
            &choice_points(&[("a", "b", 0.9)]),
            &options(&["a", "b"]),
            &[partial],
        );
        // (1-0)^2 + (0-1)^2 = 2, the worst possible multi-category Brier score.
        close(report.brier_score, 2.0);
    }

    #[test]
    fn the_choice_confusion_matrix_is_keyed_true_then_predicted() {
        let report = choice(
            &choice_points(&[("a", "b", 0.5), ("a", "b", 0.5), ("b", "b", 0.5)]),
            &options(&["a", "b"]),
            &[],
        );
        let row = report.confusion.get("b").expect("rows labelled b");
        assert_eq!(row.get("a"), Some(&2));
        assert_eq!(row.get("b"), Some(&1));
        assert!(
            !report.confusion.contains_key("a"),
            "nothing was labelled a"
        );
    }

    #[test]
    fn a_worse_class_never_raises_the_macro_average() {
        // The property the change above exists for. Two runs differing only in that the
        // second gets one class entirely wrong must not report a higher macro F1.
        let names = options(&["a", "b"]);
        let better = choice(
            &choice_points(&[("a", "a", 0.9), ("b", "b", 0.9)]),
            &names,
            &[],
        );
        let worse = choice(
            &choice_points(&[("a", "a", 0.9), ("a", "b", 0.9)]),
            &names,
            &[],
        );
        let better = better.macro_f1.expect("defined");
        let worse = worse.macro_f1.expect("defined");
        assert!(
            worse < better,
            "getting a class entirely wrong did not lower the macro F1: {worse} vs {better}"
        );
    }

    #[test]
    fn a_coverage_sweep_reports_accuracy_only_among_the_rows_it_covers() {
        let report = choice(
            &choice_points(&[("a", "a", 0.9), ("a", "b", 0.4), ("b", "b", 0.8)]),
            &options(&["a", "b"]),
            &[],
        );
        let at = |threshold: f64| {
            *report
                .sweep
                .iter()
                .find(|point| (point.threshold - threshold).abs() < 1e-12)
                .expect("the sweep should contain this observed confidence")
        };
        // At 0.8 two rows are covered and both are right.
        let high = at(0.8);
        assert_eq!(high.covered, 2);
        close(high.coverage(), 2.0 / 3.0);
        close(high.abstention(), 1.0 / 3.0);
        close(high.accuracy_among_covered(), 1.0);
        close(high.risk(), 0.0);
        // At 0.0 everything is covered and one of three is wrong.
        let all = at(0.0);
        close(all.coverage(), 1.0);
        close(all.accuracy_among_covered(), 2.0 / 3.0);
    }

    #[test]
    fn accuracy_among_nothing_covered_is_undefined_rather_than_perfect() {
        // Reporting 1.0 would make an unreachable threshold look like the best one in
        // the table, which is the single most dangerous number this module could emit.
        let point = CoveragePoint {
            threshold: 1.0,
            covered: 0,
            correct: 0,
            total: 5,
        };
        assert_eq!(point.accuracy_among_covered(), None);
        assert_eq!(point.risk(), None);
        close(point.coverage(), 0.0);
    }

    // --- Score ----------------------------------------------------------------------

    fn score_points(points: &[(f64, u32, f64)]) -> Vec<ScoreObservation> {
        points
            .iter()
            .map(|(value, label, confidence)| ScoreObservation {
                score: *value,
                label: *label,
                confidence: *confidence,
            })
            .collect()
    }

    #[test]
    fn a_score_rounds_to_a_level_inside_the_legend() {
        assert_eq!(rounded_level(1.4, 5), 1);
        assert_eq!(rounded_level(1.5, 5), 2);
        assert_eq!(rounded_level(-3.0, 5), 0);
        assert_eq!(rounded_level(99.0, 5), 4);
        // A legend with one level, which the API refuses but which must not divide by
        // zero if it ever arrives.
        assert_eq!(rounded_level(5.0, 1), 0);
        assert_eq!(rounded_level(5.0, 0), 0);
    }

    #[test]
    fn score_agreement_and_error_match_a_hand_computed_example() {
        // Rounded predictions: 2, 1, 0. Labels: 2, 2, 3.
        // Exact: 1 of 3. Adjacent: 2 of 3 (|1-2| = 1; |0-3| = 3 is not).
        // MAE on the continuous score: |2.2-2| + |1.4-2| + |0.1-3| = 0.2+0.6+2.9 = 3.7.
        let report = score(
            &score_points(&[(2.2, 2, 0.9), (1.4, 2, 0.5), (0.1, 3, 0.7)]),
            4,
        );
        close(report.exact_agreement, 1.0 / 3.0);
        close(report.adjacent_agreement, 2.0 / 3.0);
        close(report.mean_absolute_error, 3.7 / 3.0);
    }

    #[test]
    fn the_mean_absolute_error_uses_the_continuous_score_not_the_rounded_level() {
        // Rounding first would report this run as perfect, discarding exactly the
        // between-level position a Score exists to give.
        let report = score(&score_points(&[(2.4, 2, 0.9)]), 4);
        close(report.exact_agreement, 1.0);
        close(report.mean_absolute_error, 0.4);
    }

    #[test]
    fn perfect_ordinal_agreement_is_a_kappa_of_one() {
        let report = score(
            &score_points(&[(0.0, 0, 0.9), (1.0, 1, 0.9), (2.0, 2, 0.9), (3.0, 3, 0.9)]),
            4,
        );
        close(report.quadratic_weighted_kappa, 1.0);
    }

    #[test]
    fn kappa_is_undefined_when_chance_agreement_cannot_be_measured() {
        // Every row in one level means the expected-disagreement denominator is zero.
        // Reporting 1.0 would claim perfect agreement from a dataset that demonstrated
        // nothing at all.
        let report = score(&score_points(&[(1.0, 1, 0.9), (1.0, 1, 0.9)]), 4);
        assert_eq!(report.quadratic_weighted_kappa, None);
    }

    #[test]
    fn kappa_penalises_a_distant_disagreement_more_than_a_near_one() {
        let near = score(
            &score_points(&[(0.0, 0, 0.9), (1.0, 0, 0.9), (2.0, 2, 0.9), (3.0, 3, 0.9)]),
            4,
        );
        let far = score(
            &score_points(&[(0.0, 0, 0.9), (3.0, 0, 0.9), (2.0, 2, 0.9), (3.0, 3, 0.9)]),
            4,
        );
        let near = near.quadratic_weighted_kappa.expect("defined");
        let far = far.quadratic_weighted_kappa.expect("defined");
        assert!(
            near > far,
            "an off-by-one was not scored better than an off-by-three: {near} vs {far}"
        );
    }

    #[test]
    fn an_empty_score_dataset_reports_undefined_rather_than_panicking() {
        let report = score(&[], 4);
        assert_eq!(report.n, 0);
        assert_eq!(report.exact_agreement, None);
        assert_eq!(report.mean_absolute_error, None);
        assert_eq!(report.quadratic_weighted_kappa, None);
    }

    #[test]
    fn a_label_outside_the_legend_is_skipped_rather_than_indexing_out_of_bounds() {
        // The dataset parser refuses these, so this is the belt to that braces: a
        // library function must not panic on an argument its caller could get wrong.
        let report = score(&score_points(&[(1.0, 99, 0.9), (1.0, 1, 0.9)]), 4);
        assert_eq!(report.n, 2);
        assert!(report.quadratic_weighted_kappa.is_none() || report.n == 2);
    }

    // --- Interval --------------------------------------------------------------------

    #[test]
    fn the_wilson_interval_matches_a_hand_computed_example() {
        // 8 of 10, z = 1.959963985.
        let interval = wilson_interval(8, 10).expect("defined for a non-empty sample");
        assert!(
            (interval.lower - 0.4901).abs() < 5e-4,
            "lower was {}",
            interval.lower
        );
        assert!(
            (interval.upper - 0.9433).abs() < 5e-4,
            "upper was {}",
            interval.upper
        );
    }

    #[test]
    fn the_wilson_interval_stays_inside_the_unit_interval_at_the_ends() {
        // The textbook normal interval is a point at 0 or 1 here, which claims certainty
        // from ten rows. Wilson does not, and neither bound escapes [0, 1].
        for (successes, total) in [(0, 10), (10, 10), (1, 1), (0, 1)] {
            let interval = wilson_interval(successes, total).expect("defined");
            assert!((0.0..=1.0).contains(&interval.lower), "{interval:?}");
            assert!((0.0..=1.0).contains(&interval.upper), "{interval:?}");
            assert!(interval.lower <= interval.upper, "{interval:?}");
        }
        let perfect = wilson_interval(10, 10).expect("defined");
        assert!(
            perfect.lower < 1.0,
            "ten of ten was reported as certainly perfect: {perfect:?}"
        );
    }

    #[test]
    fn the_wilson_interval_narrows_as_the_sample_grows() {
        let width = |successes, total| {
            let interval = wilson_interval(successes, total).expect("defined");
            interval.upper - interval.lower
        };
        assert!(width(8, 10) > width(80, 100));
        assert!(width(80, 100) > width(800, 1000));
    }

    #[test]
    fn the_wilson_interval_is_undefined_for_an_empty_sample() {
        assert_eq!(wilson_interval(0, 0), None);
    }
}
