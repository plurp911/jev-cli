//! Fuzzes the `--require` expression parser and evaluator.
//!
//! The parser is recursive descent over user input with a depth bound. Property tests
//! in `crates/jev-cli/src/gate.rs` cover totality over generated strings; this extends
//! it to arbitrary bytes, and additionally asserts the **safety** property: an
//! expression that parses must never evaluate to a pass through the unevaluable path.
#![no_main]

use jev_cli::gate::{self, GateOutcome};
use jev_core::{
    Answer, Confidence, EvaluationResponse, ModelId, Probability, QuestionId, Usage, Weighted,
};
use libfuzzer_sys::fuzz_target;

fn response() -> EvaluationResponse {
    EvaluationResponse {
        model: ModelId::default(),
        answers: vec![
            (
                QuestionId::new("a").expect("literal id"),
                Answer::Noul {
                    noul: Probability::new(0.5).expect("in range"),
                },
            ),
            (
                QuestionId::new("b").expect("literal id"),
                Answer::Choice {
                    choice: "x".to_owned(),
                    probabilities: vec![Weighted {
                        key: "x".to_owned(),
                        probability: Probability::new(1.0).expect("in range"),
                    }],
                    confidence: Confidence::new(1.0).expect("in range"),
                },
            ),
        ],
        usage: Usage::default(),
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(expression) = gate::parse(text) else {
        return;
    };
    let outcome = gate::evaluate(&expression, &response());
    if let GateOutcome::Unevaluable { .. } = outcome {
        assert!(
            !outcome.passed(),
            "an unevaluable gate reported a pass: {text:?}"
        );
    }
});
