//! `jev mcp serve`: the Jev tools over the Model Context Protocol, on stdio.
//!
//! # Shape
//!
//! ```text
//! MCP host ──stdio──▶ server.rs (rmcp)  ──▶ tools.rs ──▶ the CLI's own request,
//!                     protocol only          arguments     credential, retry, batch,
//!                                                          and rendering code
//! ```
//!
//! This is a second *interface*, not a second implementation: `tools.rs` calls the
//! functions `jev noul`, `jev ask`, and `jev map` call, and returns the documents they
//! print. See ADR-0012 for why it exists, why it is stdio-only, and why it is the one
//! place in `jev` that runs an async runtime.
//!
//! # Streams
//!
//! stdout carries protocol messages and nothing else. Nothing in this module writes to
//! it except rmcp's transport, and the CLI's renderers are never reached from here.
//! Diagnostics go to stderr, and there are none by default.

mod server;
mod tools;

use std::sync::Arc;
use std::time::Duration;

use jev_client::{HttpTransport, Transport};
use rmcp::ServiceExt as _;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::commands::Session;
use crate::errors::{CliError, Result};
use crate::exit;

pub(crate) use tools::{Deps, MCP_MAP_SCHEMA};

/// How often the serve loop checks for Ctrl-C.
///
/// The signal handler only sets a flag (`crate::interrupt`), so the loop has to look.
const INTERRUPT_POLL: Duration = Duration::from_millis(100);

/// The longest single protocol message accepted: generous next to the input limit, so
/// that JSON escaping and a full `map` batch fit, and bounded, so that one line cannot
/// exhaust memory. The ceiling is absolute: `--max-input-bytes` is unbounded, and a
/// limit derived only from it would stop bounding anything when it is raised far enough.
fn max_message_bytes(max_input_bytes: u64) -> usize {
    const FLOOR: usize = 16 << 20;
    const CEILING: usize = 256 << 20;
    usize::try_from(max_input_bytes)
        .unwrap_or(usize::MAX)
        .saturating_mul(8)
        .clamp(FLOOR, CEILING)
}

/// Runs `jev mcp serve` until the host disconnects.
///
/// # Errors
///
/// Returns an I/O-class error when the connection fails before or during the session,
/// and an internal one if the runtime cannot be built.
pub(crate) fn serve(
    session: &mut Session<'_>,
    injected: Option<&(dyn Transport + Send + Sync)>,
    stdin_is_terminal: bool,
) -> Result<u8> {
    if injected.is_some() {
        // Only the in-process test harness injects a transport, and it cannot hand over
        // ownership of a borrowed one. MCP is tested through a real child process
        // (`tests/mcp.rs`) and through `serve_io` with owned doubles instead.
        return Err(CliError::internal(
            "jev mcp serve cannot run with a borrowed test transport",
        ));
    }

    if stdin_is_terminal {
        // Not on stdout: stdout is the protocol channel even when nobody is listening.
        session.note_always(
            "jev mcp serve speaks the Model Context Protocol on stdin and stdout; an MCP \
             host starts it. See docs/mcp.md for setup. Press Ctrl-C to stop.",
        );
    }
    // Once, at startup, and never suppressed: every call this server makes goes there.
    session.warn_about_endpoint();

    let context = Arc::new(session.context.clone());
    let deps = Deps {
        transport: Arc::new(HttpTransport::new(context.timeout.value)),
        // The process environment and the platform store: the same sources every other
        // command reads, owned here because tool calls run on other threads.
        environment: Arc::new(jev_config::SystemEnvironment),
        store: Arc::from(jev_config::platform_store()),
        context,
    };
    let verbose = session.context.verbosity.verbose;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|error| CliError::internal(format!("could not start the MCP runtime: {error}")))?;
    let outcome = runtime.block_on(serve_io(
        deps,
        verbose,
        tokio::io::stdin(),
        tokio::io::stdout(),
    ));
    // Not `drop`: dropping a runtime waits for every blocking task, and a tool call
    // still inside an HTTP attempt would hold the process open after the host has gone.
    runtime.shutdown_background();

    match outcome {
        Ok(Ended::Closed) => Ok(exit::SUCCESS),
        Ok(Ended::Interrupted) => Ok(exit::INTERRUPTED),
        Err(message) => {
            let _ = writeln!(session.err, "error: {}", crate::output::sanitize(&message));
            Ok(exit::IO)
        }
    }
}

/// How a session ended.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Ended {
    /// The host closed the connection.
    Closed,
    /// Ctrl-C.
    Interrupted,
}

/// Serves one MCP session over any reader and writer.
///
/// Generic over the streams so that nothing here assumes the process's own stdio;
/// `tests/mcp.rs` drives it through the real binary over real pipes.
///
/// # Errors
///
/// Returns the reason when the session could not start or failed at the transport.
pub(crate) async fn serve_io<R, W>(
    deps: Deps,
    verbose: bool,
    reader: R,
    writer: W,
) -> std::result::Result<Ended, String>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let limit = max_message_bytes(deps.context.max_input_bytes.value);
    let server = server::JevServer::new(deps, verbose)?;
    let exceeded = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let too_long = || {
        format!(
            "an MCP message was longer than {limit} bytes; the session was closed \
             rather than buffer it"
        )
    };
    let running = server
        .serve((
            server::BoundedLines::new(reader, limit, Arc::clone(&exceeded)),
            writer,
        ))
        .await
        .map_err(|error| {
            if exceeded.load(std::sync::atomic::Ordering::SeqCst) {
                too_long()
            } else {
                format!("the MCP session did not start: {error}")
            }
        })?;
    let stop = running.cancellation_token();

    let interrupted = async {
        while !crate::interrupt::requested() {
            tokio::time::sleep(INTERRUPT_POLL).await;
        }
    };
    tokio::select! {
        ended = running.waiting() => match ended {
            Err(error) => Err(format!("the MCP session failed: {error}")),
            Ok(_) if exceeded.load(std::sync::atomic::Ordering::SeqCst) => Err(too_long()),
            Ok(_) => Ok(Ended::Closed),
        },
        () = interrupted => {
            stop.cancel();
            Ok(Ended::Interrupted)
        }
    }
}
