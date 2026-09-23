//! Process exit codes.
//!
//! These values are part of the CLI's machine-facing contract. Scripts and CI jobs
//! branch on them, so a code may be added but never repurposed. See
//! `docs/adr/0003-cli-compatibility.md` and `docs/cli-contract.md`.
//!
//! # The distinction that matters
//!
//! **A successful API call is exit `0`, whatever the model said.** `jev noul "is this
//! spam?"` exits `0` when the evaluation succeeds, even if the probability is `0.01`.
//! Model semantics reach the exit status only when the user asks for it with
//! `--require`, and then a failed gate is `1` — never `0`, and never confused with a
//! network failure.
//!
//! A gate that *cannot be evaluated* gets its own code. Collapsing "the answer was no"
//! and "I could not tell" into one status is how a CI gate silently stops gating.

/// The command completed successfully. For an evaluation this means the API answered,
/// not that the answer was affirmative.
pub const SUCCESS: u8 = 0;

/// A `--require` gate was evaluated and did not hold.
///
/// Also returned by `jev config get` for a setting that is not set, by analogy with
/// `git config --get`. The two uses do not overlap in practice — `config get` takes no
/// `--require` and sends no request — and this is recorded here because
/// `docs/cli-contract.md` names this file as the authority for what a code means.
pub const UNSATISFIED: u8 = 1;

/// The invocation was wrong: unknown flag, bad argument, malformed input.
pub const USAGE: u8 = 2;

/// Authentication failed or no credential was available.
pub const AUTH: u8 = 3;

/// The API could not be reached, timed out, or reported overload.
pub const UNAVAILABLE: u8 = 4;

/// A batch completed with some rows failing. Data for the successful rows was still
/// written, so a caller can use it and act on the failures separately.
pub const PARTIAL: u8 = 5;

/// A `--require` gate could not be evaluated — it named a question or a field that the
/// response does not contain.
///
/// Deliberately distinct from [`UNSATISFIED`]: "the model said no" and "the gate is
/// broken" call for different responses from whoever is reading the exit status, and a
/// gate that cannot be evaluated must never be mistaken for a passing one.
pub const GATE_UNEVALUABLE: u8 = 6;

/// Output could not be written: a full disk, a read-only filesystem, a closed stream
/// that is not a pipe.
///
/// `EX_IOERR` from `sysexits.h`. Deliberately not [`INTERNAL`]: a full disk is not a
/// bug in `jev`, and telling the user to file an issue about it is wrong.
pub const IO: u8 = 74;

/// The CLI itself failed in a way that is a bug.
///
/// The gap between 6 and 70 is deliberate: 70 is `EX_SOFTWARE` from `sysexits.h`, and
/// leaving 7..=69 unused keeps room for `jev`-specific codes without colliding with the
/// conventional `sysexits` range.
pub const INTERNAL: u8 = 70;

/// Interruption by `SIGINT`, by the shell convention of `128 + signal`.
///
/// `jev` installs a handler so that an interrupt during a retry wait or a batch stops
/// cleanly and reports this code, rather than leaving a partially written output file
/// with no indication that it is partial.
pub const INTERRUPTED: u8 = 130;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_distinct_and_stable() {
        // Locks the documented contract in `docs/cli-contract.md`. Changing any of
        // these numbers breaks every script that branches on them.
        let codes = [
            ("success", SUCCESS, 0),
            ("unsatisfied", UNSATISFIED, 1),
            ("usage", USAGE, 2),
            ("auth", AUTH, 3),
            ("unavailable", UNAVAILABLE, 4),
            ("partial", PARTIAL, 5),
            ("gate-unevaluable", GATE_UNEVALUABLE, 6),
            ("io", IO, 74),
            ("internal", INTERNAL, 70),
            ("interrupted", INTERRUPTED, 130),
        ];
        for (name, actual, expected) in codes {
            assert_eq!(actual, expected, "exit code `{name}` changed");
        }

        let mut seen: Vec<u8> = codes.iter().map(|(_, code, _)| *code).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), before, "two exit codes collide");
    }

    #[test]
    fn a_failed_gate_is_not_a_provider_failure() {
        // The single most important property of this table: a script can tell "the
        // model said no" from "the API was down" from "the gate is broken".
        assert_ne!(UNSATISFIED, UNAVAILABLE);
        assert_ne!(UNSATISFIED, GATE_UNEVALUABLE);
        assert_ne!(UNSATISFIED, SUCCESS);
    }
}
