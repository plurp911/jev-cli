//! Entry point for the `jev` binary.
//!
//! `main` does four things: install an interrupt handler, work out what the process can
//! see about its own streams, run, and map the outcome onto an exit code. Everything
//! else lives in the library target, where it can be tested in process and fuzzed.

use std::io::{self, IsTerminal as _, Write as _};
use std::process::ExitCode;

use jev_cli::commands::{self, Streams};
use jev_cli::interrupt;

fn main() -> ExitCode {
    // Installed before anything else so that an interrupt during a long retry wait or a
    // batch stops cleanly and reports 130, rather than leaving a half-written output
    // file with no indication that it is partial.
    interrupt::install();

    // Unlocked handles, not `lock()`: `jev mcp serve` hands stdin and stdout to the
    // async runtime's own threads, which take these same locks per write. Held here for
    // the life of the process, they would deadlock the server on its first message.
    // Each write takes the lock briefly instead, which costs nothing measurable.
    let mut out = io::stdout();
    let mut err = io::stderr();

    let streams = Streams {
        stdin_is_terminal: io::stdin().is_terminal(),
        // Colour follows *stdout*, because that is where it would be written.
        stdout_is_terminal: io::stdout().is_terminal(),
        stderr_is_terminal: io::stderr().is_terminal(),
    };

    let code = commands::run(
        std::env::args_os(),
        &mut out,
        &mut err,
        &mut io::stdin(),
        &jev_config::SystemEnvironment,
        streams,
    );

    // A broken pipe (`jev … | head`) is normal Unix behaviour, not an error worth
    // reporting.
    let _ = out.flush();
    let _ = err.flush();

    code
}
