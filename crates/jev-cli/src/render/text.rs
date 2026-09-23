//! The human view.
//!
//! **Not a stable interface.** Wording, spacing, alignment, and colour may change in
//! any release, including a patch release. Anything that parses this output is broken
//! by design; `--output json` exists for that.
//!
//! What it *does* promise:
//!
//! * Every string that came from a file, the API, or the user is sanitized before it
//!   reaches the terminal. Option names and Score legends are echoed back from the
//!   request, and a request can come from a file the user did not write.
//! * The probability distribution is shown, not summarized away. A bar chart makes
//!   "0.6 / 0.38 / 0.02" legible at a glance, which is the whole reason to prefer a
//!   calibrated model over a text generator.
//! * Confidence is shown for Choice and Score, and *not* for Noul — with a one-line
//!   note saying why, because the asymmetry surprises people.

use std::fmt::Write as _;
use std::io::Write;

use jev_core::{Answer, ChoiceOption, EvaluationResponse, Question, QuestionId, Weighted};

use crate::errors::Result;
use crate::output::{ColorChoice, Safe};
use crate::render::json::write_all;

/// Width of the distribution bar, in characters.
const BAR_WIDTH: usize = 24;

/// ANSI styling, applied only when colour is enabled.
struct Style {
    color: bool,
}

impl Style {
    fn dim(&self, text: &str) -> String {
        if self.color {
            format!("\u{1b}[2m{text}\u{1b}[0m")
        } else {
            text.to_owned()
        }
    }

    fn bold(&self, text: &str) -> String {
        if self.color {
            format!("\u{1b}[1m{text}\u{1b}[0m")
        } else {
            text.to_owned()
        }
    }
}

/// The commentary that accompanies a rendering, for stderr.
///
/// Returned rather than written, because `docs/cli-contract.md` promises stdout carries
/// "data, and nothing else". A pedagogical paragraph and a token count are useful and
/// are not data: `jev noul … > answer.txt` must capture the answer, not an essay.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Notes {
    /// Lines to print on stderr, unless `--quiet`.
    pub lines: Vec<String>,
}

/// Renders a gate result for a person.
///
/// A bare exit `1` with nothing said is the least helpful thing a gate can do to
/// somebody running the command by hand.
#[must_use]
pub fn gate(expression: &str, outcome: &crate::gate::GateOutcome) -> String {
    match outcome {
        crate::gate::GateOutcome::Passed => format!("gate satisfied: {}", Safe::new(expression)),
        crate::gate::GateOutcome::Failed => {
            format!("gate not satisfied: {}", Safe::new(expression))
        }
        crate::gate::GateOutcome::Unevaluable { reason } => format!(
            "gate could not be evaluated: {} ({})",
            Safe::new(expression),
            Safe::new(reason)
        ),
    }
}

/// Renders an evaluation for a person.
///
/// Writes only the answers to `out`. Anything explanatory comes back in [`Notes`] for
/// the caller to put on stderr.
///
/// `questions` is the request, and sets the display order: questions as they were
/// asked and a Choice's options as they were listed. The client decodes the response
/// through a sorted map, so without it both would read alphabetically. Anything the
/// request does not name keeps its response order, after the rest.
///
/// # Errors
///
/// Propagates a write failure other than a broken pipe.
pub fn evaluation(
    out: &mut dyn Write,
    response: &EvaluationResponse,
    questions: &[(QuestionId, Question)],
    color: ColorChoice,
) -> Result<Notes> {
    let style = Style {
        color: color == ColorChoice::Always,
    };
    let mut buffer = String::new();
    let mut needs_noul_note = false;

    let asked: Vec<&str> = questions.iter().map(|(id, _)| id.as_str()).collect();
    let answers = in_order(&response.answers, &asked, |(id, _)| id.as_str());
    for (index, (id, answer)) in answers.into_iter().enumerate() {
        if index > 0 {
            buffer.push('\n');
        }
        buffer.push_str(&style.bold(&Safe::new(id.as_str()).to_string()));
        buffer.push('\n');
        match answer {
            Answer::Noul { noul } => {
                needs_noul_note = true;
                let _ = writeln!(
                    buffer,
                    "  yes  {:.4}  {}",
                    noul.get(),
                    bar(noul.get(), &style)
                );
            }
            Answer::Choice {
                choice,
                probabilities,
                confidence,
            } => {
                let _ = writeln!(buffer, "  choice      {}", Safe::new(choice));
                let _ = writeln!(buffer, "  confidence  {:.4}", confidence.get());
                let listed = option_names(questions, id);
                buffer.push_str(&distribution(
                    in_order(probabilities, &listed, |entry| entry.key.as_str())
                        .into_iter()
                        .map(|entry| (Safe::new(&entry.key).to_string(), entry.probability.get())),
                    &style,
                ));
            }
            Answer::Score {
                score,
                legend,
                probabilities,
                confidence,
            } => {
                let _ = writeln!(buffer, "  score       {score:.4}");
                let _ = writeln!(buffer, "  confidence  {:.4}", confidence.get());
                buffer.push_str(&distribution(
                    probabilities.iter().map(|entry: &Weighted<u32>| {
                        let label = legend.get(&entry.key).map_or_else(
                            || entry.key.to_string(),
                            |description| {
                                format!("{}  {}", entry.key, Safe::new(&description.to_string()))
                            },
                        );
                        (label, entry.probability.get())
                    }),
                    &style,
                ));
            }
            Answer::Unrecognized { kind, .. } => {
                let _ = writeln!(
                    buffer,
                    "  {} this build of jev does not understand the answer type {:?};\n  \
                     use --output json to see it verbatim",
                    style.dim("note:"),
                    Safe::new(kind).to_string()
                );
            }
        }
    }

    let mut notes = Notes::default();
    if needs_noul_note {
        notes.lines.push(
            "a Noul answer is the probability of \"yes\"; the API reports no separate \
             confidence for it, and 0.5 means \"yes and no are similarly likely\""
                .to_owned(),
        );
    }
    if let (Some(input), Some(output)) = (response.usage.input_tokens, response.usage.output_tokens)
    {
        notes.lines.push(format!(
            "{input} token{} in, {output} out, answered by {}",
            if input == 1 { "" } else { "s" },
            Safe::new(response.model.as_str())
        ));
    }

    write_all(out, buffer.as_bytes())?;
    Ok(notes)
}

/// `items` in the order `names` lists them; anything unlisted follows, in its own order.
///
/// A stable sort on the position in `names`, so the fallback order is the one `items`
/// already had. Both lists are bounded by the API's limits, so the linear lookup is
/// cheap.
fn in_order<'a, T>(items: &'a [T], names: &[&str], key: impl Fn(&T) -> &str) -> Vec<&'a T> {
    let mut ordered: Vec<&T> = items.iter().collect();
    ordered.sort_by_key(|item| {
        names
            .iter()
            .position(|name| *name == key(item))
            .unwrap_or(usize::MAX)
    });
    ordered
}

/// The option names the request listed for Choice question `id`, in its order.
fn option_names<'a>(questions: &'a [(QuestionId, Question)], id: &QuestionId) -> Vec<&'a str> {
    match questions.iter().find(|(asked, _)| asked == id) {
        Some((_, Question::Choice { options, .. })) => {
            options.iter().map(ChoiceOption::name).collect()
        }
        _ => Vec::new(),
    }
}

/// Renders a distribution as aligned labels, numbers, and bars.
fn distribution(entries: impl Iterator<Item = (String, f64)>, style: &Style) -> String {
    let entries: Vec<(String, f64)> = entries.collect();
    if entries.is_empty() {
        return String::new();
    }
    let width = entries
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0)
        .min(40);

    let mut out = String::new();
    for (label, probability) in entries {
        let _ = writeln!(
            out,
            "    {label:<width$}  {probability:.4}  {}",
            bar(probability, style),
        );
    }
    out
}

/// A proportional bar. Clamped, so a hostile probability cannot allocate unboundedly —
/// though `Probability` already guarantees the range.
fn bar(probability: f64, style: &Style) -> String {
    let clamped = probability.clamp(0.0, 1.0);
    // `Probability` bounds the input, so this cast cannot lose meaningful precision or
    // wrap; the clamp makes that true for any caller.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "the value is clamped to [0, 1] and BAR_WIDTH is a small constant"
    )]
    let filled =
        (clamped * f64::from(u32::try_from(BAR_WIDTH).unwrap_or(u32::MAX))).round() as usize;
    let filled = filled.min(BAR_WIDTH);
    let empty = BAR_WIDTH.saturating_sub(filled);
    format!("{}{}", "█".repeat(filled), style.dim(&"·".repeat(empty)))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use jev_core::{ChoiceOption, Confidence, Content, ModelId, Probability, Usage};

    use super::*;

    fn probability(value: f64) -> Probability {
        Probability::new(value).unwrap()
    }

    fn render(response: &EvaluationResponse) -> String {
        let mut out = Vec::new();
        let _ = evaluation(&mut out, response, &[], ColorChoice::Never).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn notes(response: &EvaluationResponse) -> Notes {
        let mut out = Vec::new();
        evaluation(&mut out, response, &[], ColorChoice::Never).unwrap()
    }

    fn noul_response(value: f64) -> EvaluationResponse {
        EvaluationResponse {
            model: ModelId::new("jev-1.13.0").unwrap(),
            answers: vec![(
                QuestionId::new("urgent").unwrap(),
                Answer::Noul {
                    noul: probability(value),
                },
            )],
            usage: Usage::default(),
        }
    }

    #[test]
    fn a_noul_shows_the_probability_and_explains_the_missing_confidence_on_stderr() {
        let rendered = render(&noul_response(0.92));
        assert!(rendered.contains("0.9200"));
        assert!(
            !rendered.to_lowercase().contains("confidence  "),
            "a Noul must not be given a confidence: {rendered}"
        );
        // The explanation is real, and it is commentary rather than data, so it belongs
        // on stderr. `jev noul … > answer.txt` must capture the answer alone.
        assert!(
            !rendered.contains("similarly likely"),
            "commentary reached stdout: {rendered}"
        );
        assert!(
            notes(&noul_response(0.92))
                .lines
                .iter()
                .any(|line| line.contains("similarly likely"))
        );
    }

    #[test]
    fn the_token_count_is_commentary_not_data() {
        let mut response = noul_response(0.5);
        response.usage = Usage {
            input_tokens: Some(1),
            output_tokens: Some(2),
        };
        let rendered = render(&response);
        assert!(!rendered.contains("token"), "{rendered}");
        let notes = notes(&response);
        // Singular, because "1 tokens in" reads like a bug.
        assert!(
            notes.lines.iter().any(|line| line.contains("1 token in")),
            "{notes:?}"
        );
    }

    #[test]
    fn a_choice_shows_every_option_not_just_the_winner() {
        // Discarding the distribution throws away the reason to use Jev.
        let response = EvaluationResponse {
            model: ModelId::default(),
            answers: vec![(
                QuestionId::new("team").unwrap(),
                Answer::Choice {
                    choice: "billing".to_owned(),
                    probabilities: vec![
                        Weighted {
                            key: "billing".to_owned(),
                            probability: probability(0.6),
                        },
                        Weighted {
                            key: "technical".to_owned(),
                            probability: probability(0.38),
                        },
                        Weighted {
                            key: "sales".to_owned(),
                            probability: probability(0.02),
                        },
                    ],
                    confidence: Confidence::new(0.4).unwrap(),
                },
            )],
            usage: Usage::default(),
        };
        let rendered = render(&response);
        for option in ["billing", "technical", "sales"] {
            assert!(rendered.contains(option), "missing {option}: {rendered}");
        }
        assert!(rendered.contains("0.4000"));
    }

    fn choice_answer(options: &[&str]) -> Answer {
        Answer::Choice {
            choice: options
                .last()
                .map(|name| (*name).to_owned())
                .unwrap_or_default(),
            probabilities: options
                .iter()
                .map(|name| Weighted {
                    key: (*name).to_owned(),
                    probability: probability(0.25),
                })
                .collect(),
            confidence: Confidence::new(0.5).unwrap(),
        }
    }

    #[test]
    fn answers_and_options_follow_the_order_the_request_asked_them_in() {
        // The client decodes the response through a sorted map, so both the questions
        // and a Choice's options arrive alphabetized. A person reads them in the order
        // they wrote them; anything the request did not name keeps its place at the end.
        let questions = vec![
            (
                QuestionId::new("zulu").unwrap(),
                Question::choice(
                    Content::text("Which?").unwrap(),
                    ["zeta", "alpha", "mu"]
                        .iter()
                        .map(|name| ChoiceOption::new(*name, None).unwrap())
                        .collect(),
                )
                .unwrap(),
            ),
            (
                QuestionId::new("alpha").unwrap(),
                Question::noul(Content::text("Yes?").unwrap(), None).unwrap(),
            ),
        ];
        let response = EvaluationResponse {
            model: ModelId::default(),
            answers: vec![
                (
                    QuestionId::new("alpha").unwrap(),
                    Answer::Noul {
                        noul: probability(0.5),
                    },
                ),
                (
                    QuestionId::new("extra").unwrap(),
                    Answer::Noul {
                        noul: probability(0.5),
                    },
                ),
                (
                    QuestionId::new("zulu").unwrap(),
                    choice_answer(&["alpha", "mu", "unasked", "zeta"]),
                ),
            ],
            usage: Usage::default(),
        };
        let mut out = Vec::new();
        let _ = evaluation(&mut out, &response, &questions, ColorChoice::Never).unwrap();
        let rendered = String::from_utf8(out).unwrap();
        let position = |needle: &str| {
            rendered
                .find(needle)
                .unwrap_or_else(|| panic!("missing {needle:?}: {rendered}"))
        };
        assert!(position("zulu\n") < position("alpha\n"), "{rendered}");
        assert!(position("alpha\n") < position("extra\n"), "{rendered}");
        assert!(position("    zeta ") < position("    alpha "), "{rendered}");
        assert!(position("    alpha ") < position("    mu "), "{rendered}");
        assert!(position("    mu ") < position("    unasked "), "{rendered}");
    }

    #[test]
    fn a_score_shows_its_legend_beside_the_levels() {
        let response = EvaluationResponse {
            model: ModelId::default(),
            answers: vec![(
                QuestionId::new("severity").unwrap(),
                Answer::Score {
                    score: 1.3,
                    legend: BTreeMap::from([
                        (0, Content::text("Cosmetic").unwrap()),
                        (1, Content::text("Workaround exists").unwrap()),
                    ]),
                    probabilities: vec![
                        Weighted {
                            key: 0,
                            probability: probability(0.3),
                        },
                        Weighted {
                            key: 1,
                            probability: probability(0.7),
                        },
                    ],
                    confidence: Confidence::new(0.6).unwrap(),
                },
            )],
            usage: Usage::default(),
        };
        let rendered = render(&response);
        assert!(rendered.contains("1.3000"));
        assert!(rendered.contains("Cosmetic"));
        assert!(rendered.contains("Workaround exists"));
    }

    #[test]
    fn api_supplied_text_cannot_rewrite_the_terminal() {
        // Option names and legends are echoed from the request, and a request can come
        // from a file the user did not write.
        let response = EvaluationResponse {
            model: ModelId::default(),
            answers: vec![(
                QuestionId::new("team").unwrap(),
                Answer::Choice {
                    choice: "bill\u{1b}[2Jing".to_owned(),
                    probabilities: vec![Weighted {
                        key: "evil\u{202e}gnp".to_owned(),
                        probability: probability(1.0),
                    }],
                    confidence: Confidence::new(1.0).unwrap(),
                },
            )],
            usage: Usage::default(),
        };
        let rendered = render(&response);
        assert!(!rendered.contains('\u{1b}'), "escape survived: {rendered}");
        assert!(
            !rendered.contains('\u{202e}'),
            "bidi override survived: {rendered}"
        );
    }

    #[test]
    fn colour_is_absent_when_disabled_and_present_when_enabled() {
        let mut plain = Vec::new();
        evaluation(&mut plain, &noul_response(0.5), &[], ColorChoice::Never).unwrap();
        assert!(!plain.contains(&0x1b));

        let mut coloured = Vec::new();
        evaluation(&mut coloured, &noul_response(0.5), &[], ColorChoice::Always).unwrap();
        assert!(coloured.contains(&0x1b));
    }

    #[test]
    fn the_bar_is_bounded_at_both_ends() {
        let style = Style { color: false };
        assert_eq!(bar(0.0, &style).chars().filter(|c| *c == '█').count(), 0);
        assert_eq!(
            bar(1.0, &style).chars().filter(|c| *c == '█').count(),
            BAR_WIDTH
        );
    }

    #[test]
    fn an_unrecognized_answer_points_at_json_output() {
        let response = EvaluationResponse {
            model: ModelId::default(),
            answers: vec![(
                QuestionId::new("future").unwrap(),
                Answer::Unrecognized {
                    kind: "ranking".to_owned(),
                    raw: serde_json::json!({}),
                },
            )],
            usage: Usage::default(),
        };
        let rendered = render(&response);
        assert!(rendered.contains("--output json"));
    }
}
