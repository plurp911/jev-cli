//! Fuzzes the input resolver.
//!
//! State arrives from a pipe or a file and may be anything at all. The resolver must
//! never panic, must reject non-UTF-8 and binary content rather than mangling it, and
//! must respect the byte limit exactly — silently truncating a user's state would mean
//! answering a question they did not ask.
#![no_main]

use jev_cli::input::{InputReader, StateSource};
use libfuzzer_sys::fuzz_target;

const LIMIT: u64 = 4096;

fuzz_target!(|data: &[u8]| {
    let reader = InputReader {
        max_bytes: LIMIT,
        stdin_is_terminal: false,
    };

    let mut stdin = data;
    if let Ok(state) = reader.state(&StateSource::ImplicitStdin, &mut stdin) {
        // Anything accepted must be valid UTF-8, non-empty, and within the limit.
        let rendered = serde_json_len(&state);
        assert!(rendered <= LIMIT as usize + 2, "accepted oversized input");
        assert!(rendered > 0);
    }

    // The JSON paths, over the same bytes.
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = reader.state(&StateSource::Json(text.to_owned()), &mut std::io::empty());
        let _ = reader.state(&StateSource::Text(text.to_owned()), &mut std::io::empty());
    }
});

/// Length of the state once encoded, as a stand-in for "how much was accepted".
fn serde_json_len(state: &jev_core::State) -> usize {
    state.content().to_string().len()
}
