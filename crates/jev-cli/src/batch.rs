//! Bounded-concurrency execution of many API calls.
//!
//! `jev map` and `jev eval` both send one request per record with a ceiling on how many
//! are in flight. That loop looks trivial and is not: it has to share one pooled HTTP
//! agent across workers, notice an interrupt between records, survive a panicking
//! worker without losing a record that was already billed, and let the caller stop the
//! run early. Written twice it would have to be *fixed* twice, so it is written here
//! once and both commands drive it.
//!
//! What this module deliberately does **not** know about: output files, gates, metrics,
//! exit codes, or what a result means. It hands each item to a closure and collects
//! what comes back, in completion order. Every policy decision stays in the command.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use jev_client::{Client, Endpoint, HttpTransport, RetryPolicy, Transport};

use crate::errors::{CliError, Result};
use crate::interrupt::InterruptibleClock;

/// Largest concurrency any batch command will accept.
///
/// Above this, the client is the bottleneck and the API's rate limit is the wall. A cap
/// keeps a mistyped `-j 100000` from trying to spawn a hundred thousand threads.
pub(crate) const MAX_CONCURRENCY: usize = 64;

// The cap and `jev-client`'s idle-connection pool are a pair, in different crates, with
// nothing else linking them. Every worker shares one pooled agent, so a pool smaller
// than the cap means the surplus workers reconnect on every record -- a TLS handshake
// each time, invisibly, for no reason. Raising one without the other is the easy
// mistake; this refuses to compile instead.
const _: () = assert!(
    jev_client::MAX_POOLED_CONNECTIONS >= MAX_CONCURRENCY,
    "the idle-connection pool is smaller than the concurrency cap"
);

/// Rejects a concurrency outside `1..=MAX_CONCURRENCY`.
///
/// A free function rather than an inline `if` so the boundary is testable without
/// standing up a session, a credential, and a transport. `0` would spawn no workers and
/// hang forever with no output and no error.
///
/// `flag` names the option in the message, because `map` and `eval` spell it the same
/// way but a future command need not.
///
/// # Errors
///
/// Returns a usage-class [`CliError`] when the value is outside the range.
pub(crate) fn check_concurrency(concurrency: usize, flag: &str) -> Result<()> {
    if concurrency == 0 || concurrency > MAX_CONCURRENCY {
        return Err(CliError::usage(format!(
            "{flag} must be between 1 and {MAX_CONCURRENCY}"
        )));
    }
    Ok(())
}

/// The client a worker closure is handed.
///
/// Spelled out because the type is long and appears in every caller's signature.
pub(crate) type Worker<'a> = Client<&'a (dyn Transport + Send + Sync), InterruptibleClock>;

/// What a worker closure says about the run after finishing one item.
pub(crate) enum Step<R> {
    /// Keep this result and take the next item.
    Continue(R),
    /// Keep this result, and stop the run without taking another item.
    ///
    /// For a failure the caller has decided is not per-item: a rejected credential
    /// would fail every remaining record the same way, and `--fail-fast`.
    Last(R),
    /// Stop the run and discard this result.
    ///
    /// For when the caller could not *record* the result — a full disk — and counting
    /// it would report work as delivered that nobody can read.
    Abort,
}

/// What a batch needs from the session, gathered once.
///
/// A struct rather than five arguments because every field is read inside a thread
/// scope, where a borrow of `Session` would not be `Sync`.
pub(crate) struct Plan<'a> {
    /// Where requests go.
    pub(crate) endpoint: &'a Endpoint,
    /// The retry policy each worker's client is built with.
    pub(crate) retry: RetryPolicy,
    /// Per-attempt HTTP timeout, used only when this batch owns its transport.
    pub(crate) timeout: Duration,
    /// Requests in flight at once. Validated by [`check_concurrency`].
    pub(crate) concurrency: usize,
    /// The clock every worker's client waits on. Its stop signal is also checked
    /// between items, so a cancelled batch takes no further item.
    pub(crate) clock: InterruptibleClock,
}

/// Runs `work` over every item, with at most `plan.concurrency` in flight.
///
/// Returns the results in **completion order** — not input order. A caller that needs
/// input order sorts by a key it put in `R` itself; doing it here would force every
/// caller to have one.
///
/// The second return value is whether the run stopped before reaching every item, for
/// either of the two reasons that can cause it: a worker returned [`Step::Last`] or
/// [`Step::Abort`], or the user interrupted.
///
/// `transport` is `None` in production, where this builds and shares one
/// [`HttpTransport`]. Tests pass a mock, which is what makes the whole batch path
/// exercisable offline.
pub(crate) fn each<T, R>(
    plan: &Plan<'_>,
    items: &[T],
    transport: Option<&(dyn Transport + Send + Sync)>,
    work: impl Fn(&Worker<'_>, &T) -> Step<R> + Sync,
) -> (Vec<R>, bool)
where
    T: Sync,
    R: Send,
{
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let results: Mutex<Vec<R>> = Mutex::new(Vec::with_capacity(items.len()));

    // One agent, shared by every worker. `ureq::Agent` is `Send + Sync` and pools
    // connections, so sharing it is what keeps a batch on warm connections instead of
    // paying for a TLS handshake per record. `HttpTransport`'s idle-connection pool is
    // sized to `MAX_CONCURRENCY` for exactly this reason.
    let owned: Option<HttpTransport> = if transport.is_none() {
        Some(HttpTransport::new(plan.timeout))
    } else {
        None
    };

    std::thread::scope(|scope| {
        for _ in 0..plan.concurrency.min(items.len()) {
            scope.spawn(|| {
                let transport: &(dyn Transport + Send + Sync) = match (transport, owned.as_ref()) {
                    (Some(injected), _) => injected,
                    (None, Some(owned)) => owned,
                    // Unreachable by construction: `owned` is built exactly when
                    // `transport` is `None`. Returning rather than panicking keeps the
                    // no-panic rule intact even for an impossible state.
                    (None, None) => return,
                };
                let client =
                    Client::with_clock(transport, plan.endpoint.clone(), plan.clock.clone())
                        .with_retry(plan.retry);

                loop {
                    if stop.load(Ordering::SeqCst) || plan.clock.stop_requested() {
                        return;
                    }
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(item) = items.get(index) else {
                        return;
                    };

                    let (result, last) = match work(&client, item) {
                        Step::Continue(result) => (Some(result), false),
                        Step::Last(result) => (Some(result), true),
                        Step::Abort => (None, true),
                    };
                    if let Some(result) = result {
                        // Poison recovery, not `if let Ok`: a panic in another worker
                        // must not silently discard a record that was sent, billed, and
                        // answered.
                        results
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(result);
                    }
                    if last {
                        stop.store(true, Ordering::SeqCst);
                        return;
                    }
                }
            });
        }
    });

    let results = results
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let stopped_early = stop.load(Ordering::SeqCst) || results.len() < items.len();
    (results, stopped_early)
}

#[cfg(test)]
mod tests {
    use jev_client::testing::MockTransport;

    use super::*;

    fn plan(concurrency: usize) -> (Endpoint, usize) {
        (Endpoint::official(), concurrency)
    }

    #[test]
    fn a_cancelled_clock_takes_no_further_item() {
        let (endpoint, concurrency) = plan(4);
        let transport = MockTransport::new();
        let cancel = std::sync::Arc::new(AtomicBool::new(true));
        let items: Vec<usize> = (0..50).collect();
        let (seen, stopped) = each(
            &Plan {
                endpoint: &endpoint,
                retry: RetryPolicy::none(),
                timeout: Duration::from_secs(1),
                concurrency,
                clock: InterruptibleClock::with_cancel(cancel),
            },
            &items,
            Some(&transport),
            |_client, item| Step::Continue(*item),
        );
        assert!(
            seen.is_empty(),
            "a cancelled batch evaluated {} item(s)",
            seen.len()
        );
        assert!(stopped);
    }

    #[test]
    fn concurrency_outside_the_range_is_refused() {
        assert!(check_concurrency(0, "--concurrency").is_err());
        assert!(check_concurrency(MAX_CONCURRENCY + 1, "--concurrency").is_err());
        assert!(check_concurrency(1, "--concurrency").is_ok());
        assert!(check_concurrency(MAX_CONCURRENCY, "--concurrency").is_ok());
    }

    #[test]
    fn the_refusal_names_the_flag_the_caller_used() {
        // `map` and `eval` both spell it `--concurrency`; a future command need not, and
        // a message naming the wrong flag sends the user to the wrong place.
        let message = check_concurrency(0, "--jobs").unwrap_err().to_string();
        assert!(message.contains("--jobs"), "{message}");
    }

    #[test]
    fn every_item_is_visited_exactly_once() {
        let (endpoint, concurrency) = plan(8);
        let transport = MockTransport::new();
        let items: Vec<usize> = (0..200).collect();
        let (mut seen, stopped) = each(
            &Plan {
                endpoint: &endpoint,
                retry: RetryPolicy::none(),
                timeout: Duration::from_secs(1),
                concurrency,
                clock: InterruptibleClock::default(),
            },
            &items,
            Some(&transport),
            |_client, item| Step::Continue(*item),
        );
        assert!(!stopped);
        seen.sort_unstable();
        assert_eq!(seen, items);
    }

    #[test]
    fn a_last_step_stops_the_run_and_keeps_its_own_result() {
        let (endpoint, concurrency) = plan(1);
        let transport = MockTransport::new();
        let items: Vec<usize> = (0..50).collect();
        let (seen, stopped) = each(
            &Plan {
                endpoint: &endpoint,
                retry: RetryPolicy::none(),
                timeout: Duration::from_secs(1),
                concurrency,
                clock: InterruptibleClock::default(),
            },
            &items,
            Some(&transport),
            |_client, item| {
                if *item == 3 {
                    Step::Last(*item)
                } else {
                    Step::Continue(*item)
                }
            },
        );
        assert!(stopped);
        assert!(seen.contains(&3), "the stopping item's result was dropped");
        assert!(seen.len() < items.len());
    }

    #[test]
    fn an_abort_stops_the_run_and_discards_its_own_result() {
        let (endpoint, concurrency) = plan(1);
        let transport = MockTransport::new();
        let items: Vec<usize> = (0..50).collect();
        let (seen, stopped) = each(
            &Plan {
                endpoint: &endpoint,
                retry: RetryPolicy::none(),
                timeout: Duration::from_secs(1),
                concurrency,
                clock: InterruptibleClock::default(),
            },
            &items,
            Some(&transport),
            |_client, item| {
                if *item == 3 {
                    Step::Abort
                } else {
                    Step::Continue(*item)
                }
            },
        );
        assert!(stopped);
        assert!(!seen.contains(&3));
    }

    #[test]
    fn no_more_than_the_concurrency_are_ever_in_flight() {
        // Asserted rather than assumed: a batch that quietly ignored `-j` would send a
        // thousand requests at once from a command sold on bounding them.
        const LIMIT: usize = 4;
        let (endpoint, _) = plan(LIMIT);
        let transport = MockTransport::new();
        let items: Vec<usize> = (0..400).collect();
        let in_flight = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let (seen, _) = each(
            &Plan {
                endpoint: &endpoint,
                retry: RetryPolicy::none(),
                timeout: Duration::from_secs(1),
                concurrency: LIMIT,
                clock: InterruptibleClock::default(),
            },
            &items,
            Some(&transport),
            |_client, item| {
                let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                // No sleep: the nextest profile forbids it and a sleep would only make
                // the race more likely, not the assertion stronger. Spinning briefly
                // keeps several workers inside the closure at once on a real machine.
                for _ in 0..1000 {
                    std::hint::spin_loop();
                }
                in_flight.fetch_sub(1, Ordering::SeqCst);
                Step::Continue(*item)
            },
        );
        assert_eq!(seen.len(), items.len());
        assert!(
            peak.load(Ordering::SeqCst) <= LIMIT,
            "{} were in flight at once, over the limit of {LIMIT}",
            peak.load(Ordering::SeqCst)
        );
    }

    #[test]
    fn more_workers_than_items_is_not_an_error() {
        let (endpoint, _) = plan(MAX_CONCURRENCY);
        let transport = MockTransport::new();
        let items = [1_usize, 2];
        let (seen, stopped) = each(
            &Plan {
                endpoint: &endpoint,
                retry: RetryPolicy::none(),
                timeout: Duration::from_secs(1),
                concurrency: MAX_CONCURRENCY,
                clock: InterruptibleClock::default(),
            },
            &items,
            Some(&transport),
            |_client, item| Step::Continue(*item),
        );
        assert!(!stopped);
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn an_empty_batch_spawns_nothing_and_reports_no_early_stop() {
        let (endpoint, _) = plan(4);
        let transport = MockTransport::new();
        let items: [usize; 0] = [];
        let (seen, stopped) = each(
            &Plan {
                endpoint: &endpoint,
                retry: RetryPolicy::none(),
                timeout: Duration::from_secs(1),
                concurrency: 4,
                clock: InterruptibleClock::default(),
            },
            &items,
            Some(&transport),
            |_client, item| Step::Continue(*item),
        );
        assert!(seen.is_empty());
        assert!(!stopped);
    }
}
