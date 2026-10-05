//! Fuzzes the request-document parser.
//!
//! This is the parser that reads a file the user wrote, or piped in from somewhere
//! else. It must never panic, and it must never produce a request that violates the
//! documented cardinality rules — because if it did, `jev` would spend the user's
//! tokens on a request the API is going to reject.
#![no_main]

use jev_core::{EmbeddedImage, Question, limits};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Raw image seeds let mutations reach headers directly, without first
    // rediscovering valid JSON and base64. Accepted headers must remain bounded
    // and survive the same canonical encoding used by request documents.
    if data.len() <= jev_core::MAX_IMAGE_BYTES {
        if let Ok(image) = EmbeddedImage::from_bytes(data.to_vec()) {
            assert!(image.width() > 0 && image.height() > 0);
            assert!(image.pixel_len() <= jev_core::MAX_IMAGE_PIXELS);
            assert!(image.byte_len() <= jev_core::MAX_IMAGE_BYTES);
            assert_eq!(EmbeddedImage::from_raw_base64(&image.base64()), Ok(image));
        }
    }
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(document) = jev_cli::request::parse_document(text, "fuzz") else {
        return;
    };

    // Anything that parsed must satisfy the limits the documentation states, so a
    // fuzzed input cannot become a request the API would reject.
    assert!(
        !document.questions.is_empty(),
        "accepted an empty question set"
    );
    for (_, question) in &document.questions {
        match question {
            Question::Choice { options, .. } => {
                assert!(options.len() >= limits::CHOICE_MIN_OPTIONS);
                assert!(options.len() <= limits::CHOICE_MAX_OPTIONS);
            }
            Question::Score { levels, .. } => {
                assert!(levels.len() >= limits::SCORE_MIN_LEVELS);
                assert!(levels.len() <= limits::SCORE_MAX_LEVELS);
            }
            Question::Noul { .. } => {}
        }
    }
});
