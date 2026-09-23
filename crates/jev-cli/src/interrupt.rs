//! Ctrl-C handling.
//!
//! # Why a handler at all
//!
//! Without one, `SIGINT` terminates the process immediately. That is acceptable for a
//! single request, but not for `jev map`: an abrupt exit mid-batch leaves an output
//! file that looks complete and is not, and a `--resume` against it would silently skip
//! records that were never evaluated.
//!
//! With a handler, the first interrupt sets a flag. Long waits and batch loops check
//! it, stop at a consistent point, flush what they have, and exit `130` — the shell
//! convention of `128 + SIGINT`. A **second** interrupt is left to the default
//! disposition, so a user can always force the issue.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Set by the signal handler. Read by anything that waits or loops.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Installs the handler.
///
/// A failure to install is not fatal: the process simply keeps the default behaviour,
/// which is still correct, just less graceful. It is reported nowhere because there is
/// nothing the user could do about it.
pub fn install() {
    let _ = ctrlc::set_handler(|| {
        if INTERRUPTED.swap(true, Ordering::SeqCst) {
            // Second interrupt: the user is insisting. `ctrlc` has replaced the default
            // disposition, so terminating here is what restores "Ctrl-C twice always
            // works". `process::abort` is not used -- it can write a core dump, and a
            // core dump of this process would contain the API key
            // (`docs/threat-model.md` T2). `clippy::exit` is denied workspace-wide
            // precisely so that this one call site has to be justified.
            #[allow(
                clippy::exit,
                reason = "the only way to honour a second interrupt from a signal handler"
            )]
            std::process::exit(i32::from(crate::exit::INTERRUPTED));
        }
    });
}

/// Whether an interrupt has been requested.
pub fn requested() -> bool {
    INTERRUPTED.load(Ordering::SeqCst)
}

/// A [`jev_client::Clock`] whose waits end early when an interrupt arrives.
///
/// [`jev_client::SystemClock`]'s `sleep` is a bare `thread::sleep`, so a `Retry-After: 5` made the
/// process deaf to Ctrl-C for five seconds per attempt — and, because the retry loop
/// then simply started the next attempt, the interrupt was swallowed entirely: the run
/// finished every remaining attempt and exited `4`. `main.rs` says the handler is
/// installed "so that an interrupt during a long retry wait or a batch stops cleanly
/// and reports 130"; this is what makes that true.
///
/// Waiting in slices costs one flag read per slice and keeps the worst-case
/// unresponsiveness at one slice (100 ms) rather than at the whole backoff.
///
/// A clock may also carry a flag of its own, for a caller that must stop *one* piece of
/// work without stopping the process: `jev mcp serve` cancels a single tool call when
/// the host sends `notifications/cancelled`, and every other call keeps running. The
/// process-wide interrupt still ends every wait, flag or no flag.
#[derive(Debug, Default, Clone)]
pub struct InterruptibleClock {
    cancel: Option<Arc<AtomicBool>>,
}

impl InterruptibleClock {
    /// A clock that also stops when `cancel` is set.
    #[must_use]
    pub const fn with_cancel(cancel: Arc<AtomicBool>) -> Self {
        Self {
            cancel: Some(cancel),
        }
    }

    /// Whether this clock's work should stop: the process was interrupted, or this
    /// clock's own flag was set.
    #[must_use]
    pub fn stop_requested(&self) -> bool {
        requested()
            || self
                .cancel
                .as_ref()
                .is_some_and(|cancel| cancel.load(Ordering::SeqCst))
    }
}

/// How long a single slice of a wait lasts.
///
/// Short enough that Ctrl-C feels immediate, long enough that a full backoff costs a
/// negligible number of wake-ups.
const SLICE: std::time::Duration = std::time::Duration::from_millis(100);

impl jev_client::Clock for InterruptibleClock {
    fn now(&self) -> std::time::Instant {
        std::time::Instant::now()
    }

    fn sleep(&self, duration: std::time::Duration) {
        let mut remaining = duration;
        while !remaining.is_zero() {
            if self.stop_requested() {
                return;
            }
            let slice = remaining.min(SLICE);
            std::thread::sleep(slice);
            remaining = remaining.saturating_sub(slice);
        }
    }

    fn jitter_sample(&self) -> f64 {
        jev_client::Clock::jitter_sample(&jev_client::SystemClock)
    }

    fn cancelled(&self) -> bool {
        self.stop_requested()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_starts_clear() {
        // The handler is not installed in tests, so nothing can set it. This asserts
        // the default rather than the signal path, which is exercised by the
        // integration tests.
        assert!(!requested());
    }

    #[test]
    fn a_cancel_flag_stops_only_its_own_clock() {
        use jev_client::Clock as _;
        let flag = Arc::new(AtomicBool::new(false));
        let mine = InterruptibleClock::with_cancel(Arc::clone(&flag));
        let other = InterruptibleClock::default();
        assert!(!mine.cancelled());
        flag.store(true, Ordering::SeqCst);
        assert!(mine.cancelled());
        assert!(!other.cancelled());
        // A cancelled clock's wait returns at once instead of sleeping it out.
        let started = std::time::Instant::now();
        mine.sleep(std::time::Duration::from_secs(30));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn installing_twice_is_harmless() {
        install();
        install();
    }
}
