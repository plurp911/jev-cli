//! Fuzzes the API response decoder.
//!
//! Every byte here comes off the network. The decoder must not panic, must not
//! overflow the stack on nested input, and must not produce a value that violates the
//! domain invariants — an out-of-range probability, or a Choice whose selection is not
//! in its own distribution.
#![no_main]

use jev_core::Answer;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(response) = jev_client::decode_evaluation(data) else {
        // Error messages are shown to users and must stay bounded, but a failed decode
        // has nothing more to check.
        return;
    };

    for (_, answer) in &response.answers {
        match answer {
            Answer::Noul { noul } => {
                assert!((0.0..=1.0).contains(&noul.get()));
            }
            Answer::Choice {
                choice,
                probabilities,
                confidence,
            } => {
                assert!((0.0..=1.0).contains(&confidence.get()));
                if !probabilities.is_empty() {
                    assert!(
                        probabilities.iter().any(|entry| &entry.key == choice),
                        "decoded a choice absent from its own distribution"
                    );
                }
                for entry in probabilities {
                    assert!((0.0..=1.0).contains(&entry.probability.get()));
                }
            }
            Answer::Score {
                score,
                legend,
                probabilities,
                confidence,
            } => {
                assert!(score.is_finite());
                assert!((0.0..=1.0).contains(&confidence.get()));
                // The same range the decoder enforces, and for the same reason: a
                // score is the probability-weighted mean of the level numbers, so both
                // the legend and the distribution contribute levels, and both ends
                // matter. Asserting only `legend`'s maximum, and only a lower bound of
                // zero, is weaker than the decoder — which is how this target reported
                // a "failure" for a response the decoder had correctly accepted.
                let levels: Vec<u32> = legend
                    .keys()
                    .copied()
                    .chain(probabilities.iter().map(|entry| entry.key))
                    .collect();
                let lowest = levels.iter().copied().min().unwrap_or(0);
                let highest = levels.iter().copied().max().unwrap_or(0);
                assert!(
                    *score >= f64::from(lowest) && *score <= f64::from(highest),
                    "decoded a score outside the range its levels span"
                );
            }
            Answer::Unrecognized { .. } => {}
        }
    }

    // Also exercise the other two decoders on the same bytes.
    let _ = jev_client::decode_models(data);
    let _ = jev_client::extract_error_message(data);
});
