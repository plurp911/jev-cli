//! Resolving state and other input from arguments, files, and stdin.
//!
//! # Rules
//!
//! * **Exactly one source.** Two `--state*` flags, or a flag plus a piped stdin that
//!   something else already claimed, is a usage error rather than a silent precedence
//!   rule the user has to learn.
//! * **Nothing is read that the user did not name.** No globbing, no directory walking,
//!   no `.env`. `-` means stdin and nothing else does.
//! * **Bounded.** Every read goes through a byte cap. Exceeding it is an error that
//!   says so; input is never silently truncated, because a truncated state produces a
//!   confident answer to a question the user did not ask.
//! * **Validated before it costs anything.** Empty input without supported media,
//!   invalid UTF-8, binary content, and malformed JSON are rejected locally.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use jev_core::{Content, EmbeddedImage, State};
use serde_json::Value;

use crate::errors::{CliError, Result};

/// Where a piece of input came from, for error messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A literal command-line value.
    Argument(&'static str),
    /// A named file.
    File(PathBuf),
    /// Standard input.
    Stdin,
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Argument(flag) => write!(f, "--{flag}"),
            // The path came from the user's own command line, so echoing it is not a
            // disclosure — but it is still sanitized, because a crafted filename can
            // carry terminal escapes.
            Self::File(path) => write!(
                f,
                "{}",
                crate::output::Safe::new(&path.display().to_string())
            ),
            Self::Stdin => f.write_str("standard input"),
        }
    }
}

/// How the caller asked for state.
///
/// Modelled as an enum rather than four `Option<String>` fields so that "two sources at
/// once" is rejected by `clap`'s conflict checking and by this type, not by a runtime
/// precedence rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateSource {
    /// Literal text.
    Text(String),
    /// Text read from a file, or from stdin when the path is `-`.
    TextFile(PathBuf),
    /// Literal JSON.
    Json(String),
    /// JSON read from a file, or from stdin when the path is `-`.
    JsonFile(PathBuf),
    /// Text read from stdin because no flag was given.
    ImplicitStdin,
}

/// Reads and validates state.
#[derive(Debug)]
pub struct InputReader {
    /// Ceiling on bytes read from one source.
    pub max_bytes: u64,
    /// Whether stdin is a terminal. Injected so tests do not depend on how they are run.
    pub stdin_is_terminal: bool,
}

#[derive(Clone, Copy)]
enum TextRequirement {
    Nonempty,
    ImagesPresent,
    Publisher,
}

impl InputReader {
    /// Resolves a [`StateSource`] into validated [`State`].
    ///
    /// # Errors
    ///
    /// Returns a usage-class [`CliError`] for missing, empty, oversized, non-UTF-8,
    /// binary, or malformed input.
    pub fn state(&self, source: &StateSource, stdin: &mut dyn std::io::Read) -> Result<State> {
        self.read_state(source, stdin, TextRequirement::Nonempty)
    }

    /// Resolves explicitly supplied state that accompanies validated images.
    ///
    /// Cloudflare permits an empty string when the image carries the content. Other
    /// providers must keep using [`Self::state`] if they require nonempty text.
    /// Implicit stdin retains its text requirement so a missing source is not hidden.
    ///
    /// # Errors
    ///
    /// Enforces the same source, size, UTF-8, binary, and JSON checks as [`Self::state`].
    pub(crate) fn state_with_images(
        &self,
        source: &StateSource,
        stdin: &mut dyn std::io::Read,
        images: &[EmbeddedImage],
    ) -> Result<State> {
        let requirement = if images.is_empty() {
            TextRequirement::Nonempty
        } else {
            TextRequirement::ImagesPresent
        };
        self.read_state(source, stdin, requirement)
    }

    /// Reads explicitly supplied publisher JSON without changing strict state parsing.
    pub(crate) fn publisher_state(
        &self,
        source: &StateSource,
        stdin: &mut dyn std::io::Read,
    ) -> Result<State> {
        self.read_state(source, stdin, TextRequirement::Publisher)
    }

    fn read_state(
        &self,
        source: &StateSource,
        stdin: &mut dyn std::io::Read,
        requirement: TextRequirement,
    ) -> Result<State> {
        match source {
            StateSource::Text(text) => {
                let origin = Origin::Argument("state");
                self.check_size(text.len() as u64, &origin)?;
                text_state(text, &origin, requirement)
            }
            StateSource::TextFile(path) => {
                let (bytes, origin) = self.read(path, stdin)?;
                let text = decode_utf8(&bytes, &origin)?;
                text_state(&text, &origin, requirement)
            }
            StateSource::Json(raw) => {
                let origin = Origin::Argument("state-json");
                self.check_size(raw.len() as u64, &origin)?;
                json_state(raw, &origin, requirement)
            }
            StateSource::JsonFile(path) => {
                let (bytes, origin) = self.read(path, stdin)?;
                let text = decode_utf8(&bytes, &origin)?;
                json_state(&text, &origin, requirement)
            }
            StateSource::ImplicitStdin => {
                if self.stdin_is_terminal {
                    return Err(CliError::usage(
                        "no state was supplied\n\n\
                         Pass --state \"…\", --state-file PATH, or --state-json \"…\", \
                         or pipe the state into `jev` on standard input.",
                    ));
                }
                let bytes = self.read_stream(stdin, &Origin::Stdin)?;
                let text = decode_utf8(&bytes, &Origin::Stdin)?;
                text_state(&text, &Origin::Stdin, TextRequirement::Nonempty)
            }
        }
    }

    /// Reads a named file, or stdin when the path is `-`.
    ///
    /// # Errors
    ///
    /// Returns a usage-class [`CliError`] when the file cannot be read or exceeds the
    /// cap.
    pub fn read(&self, path: &Path, stdin: &mut dyn std::io::Read) -> Result<(Vec<u8>, Origin)> {
        if path == Path::new("-") {
            let bytes = self.read_stream(stdin, &Origin::Stdin)?;
            return Ok((bytes, Origin::Stdin));
        }
        let origin = Origin::File(path.to_path_buf());

        // Everything is decided from `metadata` *before* opening. `File::open` on a
        // FIFO with no writer blocks forever — a silent, unbounded stall in a pipeline
        // — and a directory opens fine on Unix, failing later with a confusing message.
        let metadata = std::fs::metadata(path).map_err(|error| {
            CliError::usage(format!("cannot read {origin}: {}", io_reason(&error)))
        })?;
        if metadata.is_dir() {
            return Err(CliError::usage(format!(
                "{origin} is a directory; `jev` reads one file at a time and never \
                 walks a directory"
            )));
        }
        if !metadata.is_file() {
            return Err(CliError::usage(format!(
                "{origin} is not a regular file; `jev` will not read a device or a \
                 pipe by name. Pipe it on standard input instead."
            )));
        }

        // `File::open` follows symlinks, which is correct here: the user named this
        // path, so following a link they created is doing what they asked. What `jev`
        // must not do — and does not — is construct a path from untrusted content.
        let file = std::fs::File::open(path).map_err(|error| {
            CliError::usage(format!("cannot read {origin}: {}", io_reason(&error)))
        })?;

        let bytes = self.read_stream(&mut { file }, &origin)?;
        Ok((bytes, origin))
    }

    /// Reads a stream through the byte cap.
    fn read_stream(&self, source: &mut dyn std::io::Read, origin: &Origin) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        // `take` is the bound. A `Content-Length`, a file's stated size, or a pipe's
        // claim about itself are all untrusted, so none of them sizes the allocation.
        let read = source
            .take(self.max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| {
                CliError::usage(format!("cannot read {origin}: {}", io_reason(&error)))
            })?;
        if read as u64 > self.max_bytes {
            return Err(CliError::usage(format!(
                "{origin} is larger than the {} byte input limit\n\n\
                 `jev` never silently truncates state: a truncated state produces a \
                 confident answer to a question you did not ask. Raise the limit with \
                 --max-input-bytes, or send less.",
                self.max_bytes
            )));
        }
        Ok(bytes)
    }

    fn check_size(&self, len: u64, origin: &Origin) -> Result<()> {
        if len > self.max_bytes {
            return Err(CliError::usage(format!(
                "{origin} is larger than the {} byte input limit",
                self.max_bytes
            )));
        }
        Ok(())
    }
}

/// Decodes bytes as UTF-8, refusing anything else.
///
/// Lossy conversion is deliberately not offered: replacing bad bytes with `U+FFFD`
/// changes the content the user is billed to evaluate, and does it invisibly.
fn decode_utf8(bytes: &[u8], origin: &Origin) -> Result<String> {
    if let Some(position) = bytes.iter().position(|byte| *byte == 0) {
        // A NUL byte means this is almost certainly a binary file. Jev takes text; the
        // useful thing to do is say so rather than send a megabyte of ELF header.
        return Err(CliError::usage(format!(
            "{origin} looks like binary data (a NUL byte at offset {position})\n\n\
             Jev evaluates natural-language text. Convert the input to text first."
        )));
    }
    String::from_utf8(bytes.to_vec()).map_err(|error| {
        CliError::usage(format!(
            "{origin} is not valid UTF-8 (at byte {})",
            error.utf8_error().valid_up_to()
        ))
    })
}

fn text_state(text: &str, origin: &Origin, requirement: TextRequirement) -> Result<State> {
    // One trailing line terminator is removed, because a shell adds it and nobody
    // means it as part of the state. Nothing else is trimmed: leading whitespace can be
    // significant -- indented code, a quoted block -- and `jev` does not silently alter
    // what the user is billed to evaluate.
    let text = text.strip_suffix('\n').map_or(text, |trimmed| {
        trimmed.strip_suffix('\r').unwrap_or(trimmed)
    });
    if matches!(requirement, TextRequirement::Publisher) {
        return Content::local_json(Value::String(text.to_owned()))
            .map(State::new)
            .map_err(|error| CliError::usage(format!("{origin}: {error}")));
    }
    if matches!(requirement, TextRequirement::ImagesPresent) {
        return Ok(State::new(Content::Text(text.to_owned())));
    }
    State::text(text).map_err(|_| {
        CliError::usage(format!(
            "{origin} is empty\n\n\
             There is nothing for the model to evaluate, so `jev` does not send the \
             request."
        ))
    })
}

fn json_state(raw: &str, origin: &Origin, requirement: TextRequirement) -> Result<State> {
    if raw.trim().is_empty() {
        return Err(CliError::usage(format!("{origin} is empty")));
    }
    let value = crate::ordered::parse_unambiguous_value(raw)
        .map_err(|error| CliError::usage(format!("{origin} is not valid JSON: {error}")))?;
    if matches!(requirement, TextRequirement::Publisher) {
        return Content::local_json(value)
            .map(State::new)
            .map_err(|error| CliError::usage(format!("{origin}: {error}")));
    }
    if matches!(requirement, TextRequirement::ImagesPresent)
        && let Value::String(text) = &value
    {
        return Ok(State::new(Content::Text(text.clone())));
    }
    let content = Content::try_from(value).map_err(|error| {
        CliError::usage(format!(
            "{origin}: {error}\n\n\
             The API accepts a string, an object, or an array as state."
        ))
    })?;
    Ok(State::new(content))
}

/// An I/O failure description that names the kind, not the OS message.
///
/// Some platforms embed the full path in the message, and a path can be sensitive in a
/// CI log even when the user supplied it. The common kinds are spelled out rather than
/// printed as their Rust names: "entity not found" is a `std::io::ErrorKind` identifier
/// leaking into a user's terminal, and nobody outside Rust recognises it.
/// Plain English for an `io::Error`, for a message a user reads.
///
/// `ErrorKind`'s own `Display` is written for Rust programmers: a missing file reports
/// "entity not found", which tells a user nothing about what to do next. Shared with
/// the output paths in `commands::map` so both directions read the same way.
#[must_use]
pub fn io_reason(error: &std::io::Error) -> String {
    use std::io::ErrorKind;

    match error.kind() {
        ErrorKind::NotFound => "no such file or directory".to_owned(),
        ErrorKind::PermissionDenied => "permission denied".to_owned(),
        ErrorKind::IsADirectory => "it is a directory".to_owned(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    fn reader() -> InputReader {
        InputReader {
            max_bytes: 1024,
            stdin_is_terminal: false,
        }
    }

    fn resolve(source: &StateSource, stdin: &str) -> Result<State> {
        reader().state(source, &mut stdin.as_bytes())
    }

    #[test]
    fn literal_text_becomes_text_state() {
        let state = resolve(&StateSource::Text("a ticket".to_owned()), "").unwrap();
        assert_eq!(state.content().as_text(), Some("a ticket"));
    }

    #[test]
    fn literal_json_keeps_its_structure() {
        let state = resolve(
            &StateSource::Json(r#"{"subject":"x","body":"y"}"#.to_owned()),
            "",
        )
        .unwrap();
        assert!(!state.content().is_text());
        assert_eq!(
            serde_json::to_value(&state).unwrap(),
            serde_json::json!({"subject": "x", "body": "y"})
        );
    }

    #[test]
    fn implicit_stdin_is_read_as_text() {
        let state = resolve(&StateSource::ImplicitStdin, "piped state").unwrap();
        assert_eq!(state.content().as_text(), Some("piped state"));
    }

    #[test]
    fn one_trailing_newline_is_removed_and_nothing_else_is() {
        // The shell adds it; the user did not type it.
        assert_eq!(
            resolve(&StateSource::ImplicitStdin, "state\n")
                .unwrap()
                .content()
                .as_text(),
            Some("state")
        );
        assert_eq!(
            resolve(&StateSource::ImplicitStdin, "state\r\n")
                .unwrap()
                .content()
                .as_text(),
            Some("state")
        );
        // A second newline is content -- a blank final line in a document, say.
        assert_eq!(
            resolve(&StateSource::ImplicitStdin, "state\n\n")
                .unwrap()
                .content()
                .as_text(),
            Some("state\n")
        );
        // Leading whitespace is significant: indented code, a quoted block.
        assert_eq!(
            resolve(&StateSource::ImplicitStdin, "    indented")
                .unwrap()
                .content()
                .as_text(),
            Some("    indented")
        );
    }

    #[test]
    fn a_terminal_with_no_state_flag_is_an_actionable_usage_error() {
        // Hanging on a TTY waiting for input nobody is going to type is the worst
        // possible behaviour here.
        let reader = InputReader {
            max_bytes: 1024,
            stdin_is_terminal: true,
        };
        let error = reader
            .state(&StateSource::ImplicitStdin, &mut std::io::empty())
            .unwrap_err();
        assert!(error.to_string().contains("--state"));
    }

    #[test]
    fn a_dash_path_means_stdin() {
        let state = resolve(&StateSource::TextFile(PathBuf::from("-")), "from pipe").unwrap();
        assert_eq!(state.content().as_text(), Some("from pipe"));
    }

    #[test]
    fn a_file_is_read_as_text() {
        let file = tempfile::NamedTempFile::new().unwrap();
        write!(file.as_file(), "file state").unwrap();
        let state = resolve(&StateSource::TextFile(file.path().to_path_buf()), "").unwrap();
        assert_eq!(state.content().as_text(), Some("file state"));
    }

    #[test]
    fn empty_input_is_refused_before_it_costs_anything() {
        for source in [
            StateSource::Text(String::new()),
            StateSource::Text("   \n\t ".to_owned()),
            StateSource::ImplicitStdin,
        ] {
            let error = resolve(&source, "").unwrap_err();
            assert!(error.to_string().contains("empty"), "for {source:?}");
        }
    }

    #[test]
    fn invalid_json_names_the_problem() {
        let error = resolve(&StateSource::Json("{not json".to_owned()), "").unwrap_err();
        assert!(error.to_string().contains("not valid JSON"));
    }

    #[test]
    fn a_bare_json_scalar_is_refused_with_the_accepted_shapes() {
        // The API accepts string | object | array; a number would 422 after being paid
        // for.
        let error = resolve(&StateSource::Json("42".to_owned()), "").unwrap_err();
        assert!(error.to_string().contains("string, an object, or an array"));
    }

    #[test]
    fn binary_input_is_named_as_binary() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), [0x7f, 0x45, 0x4c, 0x46, 0x00, 0x01]).unwrap();
        let error = resolve(&StateSource::TextFile(file.path().to_path_buf()), "").unwrap_err();
        assert!(error.to_string().contains("binary"), "{error}");
    }

    #[test]
    fn invalid_utf8_is_refused_rather_than_lossily_converted() {
        // Lossy conversion would change the billed content silently.
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), [0xe2, 0x28, 0xa1]).unwrap();
        let error = resolve(&StateSource::TextFile(file.path().to_path_buf()), "").unwrap_err();
        assert!(error.to_string().contains("not valid UTF-8"), "{error}");
    }

    #[test]
    fn oversized_input_is_reported_not_truncated() {
        let reader = InputReader {
            max_bytes: 16,
            stdin_is_terminal: false,
        };
        let error = reader
            .state(&StateSource::ImplicitStdin, &mut "x".repeat(100).as_bytes())
            .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("larger than"));
        assert!(message.contains("--max-input-bytes"));
        assert!(message.contains("never silently truncates"));
    }

    #[test]
    fn input_at_exactly_the_limit_is_accepted() {
        let reader = InputReader {
            max_bytes: 16,
            stdin_is_terminal: false,
        };
        let state = reader
            .state(&StateSource::ImplicitStdin, &mut "x".repeat(16).as_bytes())
            .unwrap();
        assert_eq!(state.content().as_text().map(str::len), Some(16));
    }

    #[test]
    fn a_missing_file_reports_the_kind_in_words_not_a_rust_identifier() {
        let error = resolve(
            &StateSource::TextFile(PathBuf::from("/nonexistent/jev-test-file")),
            "",
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("cannot read"));
        assert!(message.contains("no such file or directory"), "{message}");
        assert!(
            !message.contains("entity not found"),
            "a Rust ErrorKind identifier leaked into the message: {message}"
        );
    }

    #[test]
    fn a_directory_is_named_as_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let error = resolve(&StateSource::TextFile(dir.path().to_path_buf()), "").unwrap_err();
        assert!(error.to_string().contains("directory"), "{error}");
    }

    #[test]
    fn a_hostile_filename_cannot_rewrite_the_terminal() {
        // The path is echoed in the error, and a filename can carry ANSI escapes.
        let origin = Origin::File(PathBuf::from("evil\u{1b}[2Jname"));
        let rendered = origin.to_string();
        assert!(!rendered.contains('\u{1b}'), "{rendered}");
    }
}
