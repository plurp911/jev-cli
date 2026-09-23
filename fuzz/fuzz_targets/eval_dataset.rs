//! Fuzzes the labelled-dataset parser `jev eval --dataset` reads.
//!
//! A dataset is a file somebody wrote, exported, or piped in, so it is untrusted bytes
//! like any other input. This target asserts two things beyond "does not panic":
//!
//! * **Every label that parsed is valid for the question it labels.** A Choice label
//!   that is not one of the declared options, or a Score label outside the legend, would
//!   silently score a row against ground truth the question could never produce — and
//!   the whole point of this command is that the number it prints can be trusted.
//! * **Every row id is unique.** The id is the holdout split key, so a duplicate would
//!   put the same example on both sides of the split and quietly inflate the result.
#![no_main]

use std::collections::BTreeSet;

use jev_cli::dataset::{self, Label};
use jev_core::{ChoiceOption, Content, Question, QuestionId};
use libfuzzer_sys::fuzz_target;

fn questions() -> Vec<(QuestionId, Question)> {
    let text = |value: &str| Content::text(value).expect("literal content");
    vec![
        (
            QuestionId::new("a").expect("literal id"),
            Question::noul(text("yes or no?"), None).expect("valid noul"),
        ),
        (
            QuestionId::new("b").expect("literal id"),
            Question::choice(
                text("which?"),
                vec![
                    ChoiceOption::new("one", None).expect("valid option"),
                    ChoiceOption::new("two", None).expect("valid option"),
                ],
            )
            .expect("valid choice"),
        ),
        (
            QuestionId::new("c").expect("literal id"),
            Question::score(text("how much?"), vec![text("low"), text("high")])
                .expect("valid score"),
        ),
    ]
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let questions = questions();
    let Ok(parsed) = dataset::parse(text, "fuzz", &questions, "fuzz-request") else {
        return;
    };

    let mut ids = BTreeSet::new();
    for row in &parsed.rows {
        assert!(
            ids.insert(row.id.clone()),
            "accepted a duplicate row id, which would split both ways"
        );
        assert!(!row.labels.is_empty(), "accepted a row with no labels");
        for (question_id, label) in &row.labels {
            let question = questions
                .iter()
                .find(|(id, _)| id.as_str() == question_id)
                .map(|(_, question)| question)
                .expect("accepted a label for a question that does not exist");
            match (question, label) {
                (Question::Noul { .. }, Label::Noul(_)) => {}
                (Question::Choice { options, .. }, Label::Choice(name)) => {
                    assert!(
                        options.iter().any(|option| option.name() == name),
                        "accepted a choice label that is not a declared option"
                    );
                }
                (Question::Score { levels, .. }, Label::Score(level)) => {
                    assert!(
                        (*level as usize) < levels.len(),
                        "accepted a score label outside the legend"
                    );
                }
                _ => panic!("accepted a label of the wrong shape for its question"),
            }
        }
    }

    // The fingerprint has to be total over anything that parsed: it goes into a report
    // and into a comparison against a later run.
    assert_eq!(parsed.fingerprint.len(), 16);
});
