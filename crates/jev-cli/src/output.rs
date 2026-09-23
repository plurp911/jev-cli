//! Rendering.
//!
//! Two rules hold everywhere in this module:
//!
//! 1. **Data goes to stdout, everything else goes to stderr.** Progress notes,
//!    warnings, and diagnostics must never contaminate a pipeline.
//! 2. **Untrusted text is sanitized before display.** API responses and file contents
//!    can contain ANSI escape sequences that rewrite the terminal, forge output, or
//!    hide text; see `docs/threat-model.md`, "Terminal escape injection".

use std::fmt::Write as _;

/// Escapes every character a terminal would act on rather than display, leaving
/// ordinary text, tabs, and newlines intact.
///
/// Two classes are neutralized:
///
/// * **Control characters** — C0, `DEL`, and C1. These carry ANSI escape sequences,
///   which can clear the screen, reposition the cursor, forge output, or retitle the
///   window.
/// * **Unicode format characters (category `Cf`)** — bidirectional overrides, isolates,
///   and zero-width characters. These are not control characters, so a naive
///   `is_control` filter misses them, yet they reorder and hide displayed text. This is
///   the Trojan Source class of spoofing, and it is exactly the harm `SECURITY.md`
///   item 8 promises to prevent.
///
/// The `Cf` set is written out explicitly rather than pulled from a Unicode-tables
/// dependency: it is small, stable, and a dependency here would be disproportionate
/// (see `docs/adr/0004-dependency-policy.md`).
#[must_use]
pub fn sanitize(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\n' | '\t' => out.push(ch),
            c if is_terminal_actionable(c) => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// Escapes everything [`sanitize`] does, and the line breaks and tabs it preserves.
///
/// `--value` promises "one scalar and a newline, nothing else" (`docs/cli-contract.md`).
/// A Choice's selected option is API-supplied text, so it can contain a newline; with
/// [`sanitize`], which keeps `\n` and `\t` because diagnostics are prose, that promise
/// broke silently. `x=$(jev … --value)` came back holding an embedded newline, `read x`
/// truncated at the first line, and a `while read` loop over the output desynchronized.
///
/// One value, one line. The escape is visible rather than silent, so a user who does
/// get a multi-line option name can see that is what happened.
#[must_use]
pub fn sanitize_scalar(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if is_terminal_actionable(ch) || ch == '\n' || ch == '\r' || ch == '\t' {
            let _ = write!(out, "\\u{{{:x}}}", ch as u32);
        } else {
            out.push(ch);
        }
    }
    out
}

/// Returns `true` for characters that change how a terminal renders surrounding text
/// instead of rendering themselves.
///
/// Shared with `render::json`, which escapes the same set rather than keeping a second
/// list of its own: two definitions of "hazardous character" would drift, and the one
/// that drifted would be the one nobody was reading.
pub(crate) fn is_terminal_actionable(c: char) -> bool {
    // C0 controls, DEL, and the C1 range.
    if c.is_control() || ('\u{80}'..='\u{9f}').contains(&c) {
        return true;
    }

    matches!(
        c,
        '\u{ad}'                    // SOFT HYPHEN
            | '\u{600}'..='\u{605}'  // Arabic number signs
            | '\u{61c}'             // ARABIC LETTER MARK, a bidi control
            | '\u{6dd}'             // ARABIC END OF AYAH
            | '\u{70f}'             // SYRIAC ABBREVIATION MARK
            | '\u{890}'..='\u{891}'  // Arabic pound/piastre marks
            | '\u{8e2}'             // ARABIC DISPUTED END OF AYAH
            | '\u{180e}'            // MONGOLIAN VOWEL SEPARATOR
            | '\u{200b}'..='\u{200f}' // zero-width space/joiners, LRM, RLM
            // LINE SEPARATOR and PARAGRAPH SEPARATOR. Not category `Cf` -- they are
            // `Zl` and `Zp` -- so a set built only from `Cf` misses them. They belong
            // here for the same reason as the rest: several consumers, notably
            // anything that feeds a line to a JavaScript parser, treat them as line
            // terminators, which splits one JSONL record into two.
            | '\u{2028}' | '\u{2029}'
            | '\u{202a}'..='\u{202e}' // bidi embedding and override
            | '\u{2060}'..='\u{2064}' // word joiner and invisible operators
            | '\u{2066}'..='\u{2069}' // bidi isolates
            | '\u{feff}'            // BOM / zero-width no-break space
            | '\u{fff9}'..='\u{fffb}' // interlinear annotation
            | '\u{110bd}' | '\u{110cd}' // Kaithi number signs
            | '\u{13430}'..='\u{1343f}' // Egyptian hieroglyph format controls
            | '\u{1bca0}'..='\u{1bca3}' // Shorthand format controls
            | '\u{1d173}'..='\u{1d17a}' // musical beam/slur/phrase controls
            | '\u{e0001}'           // LANGUAGE TAG
            | '\u{e0020}'..='\u{e007f}' // the Unicode tag block: invisible text
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_ordinary_text_alone() {
        assert_eq!(sanitize("hello world"), "hello world");
        assert_eq!(sanitize("line\nnext\tcol"), "line\nnext\tcol");
        assert_eq!(sanitize("émoji 🎯"), "émoji 🎯");
    }

    #[test]
    fn escapes_ansi_sequences() {
        let hostile = "\u{1b}[2J\u{1b}[1;1Hyou have been pwned";
        let rendered = sanitize(hostile);
        assert!(!rendered.contains('\u{1b}'));
        assert!(rendered.contains("\\u{1b}"));
    }

    #[test]
    fn escapes_carriage_returns_used_to_overwrite_output() {
        assert_eq!(sanitize("safe\rforged"), "safe\\u{d}forged");
    }

    #[test]
    fn escapes_c1_controls() {
        assert_eq!(sanitize("\u{9b}"), "\\u{9b}");
    }

    #[test]
    fn escapes_bidi_overrides_used_for_trojan_source_spoofing() {
        // U+202E reverses display order, so "gnp/" renders as "/png" and a user can be
        // shown a filename or command that is not the one present in the bytes.
        let hostile = "safe\u{202e}txt.exe";
        let rendered = sanitize(hostile);
        assert!(!rendered.contains('\u{202e}'));
        assert_eq!(rendered, "safe\\u{202e}txt.exe");
    }

    #[test]
    fn escapes_the_whole_format_category_not_just_the_obvious_ones() {
        // U+061C is a bidi control in the Trojan Source set, and the U+E0000 tag block
        // is the standard invisible-text smuggling channel. Both are `Cf` and both were
        // missing from an earlier hand-list that claimed to cover the category.
        for hidden in [
            '\u{61c}',
            '\u{e0041}',
            '\u{e0001}',
            '\u{2062}',
            '\u{1d173}',
            '\u{110bd}',
            '\u{13430}',
            '\u{1bca0}',
            '\u{600}',
            '\u{6dd}',
        ] {
            let rendered = sanitize(&format!("a{hidden}b"));
            assert!(
                !rendered.contains(hidden),
                "format character U+{:04X} survived sanitization",
                hidden as u32
            );
        }
    }

    #[test]
    fn escapes_zero_width_and_format_characters() {
        // Not control characters, so `char::is_control` alone would let them through.
        for hidden in ['\u{200b}', '\u{200e}', '\u{2066}', '\u{feff}', '\u{ad}'] {
            let rendered = sanitize(&format!("a{hidden}b"));
            assert!(
                !rendered.contains(hidden),
                "format character U+{:04X} survived sanitization",
                hidden as u32
            );
        }
    }
}

/// Whether colour should be used on a given stream.
///
/// Resolution order, most specific first:
///
/// 1. `--color always|never` on the command line.
/// 2. `NO_COLOR`, per <https://no-color.org>: set to anything, including the empty
///    string, disables colour.
/// 3. `color` in the configuration file.
/// 4. Automatic: colour only when the stream is a terminal.
///
/// `jev` never colours `--output json`, on any setting: colour codes in a document a
/// script parses are a bug, not a preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorChoice {
    /// Colour when the stream is a terminal.
    #[default]
    Auto,
    /// Always colour.
    Always,
    /// Never colour.
    Never,
}

impl ColorChoice {
    /// Resolves the choice against `NO_COLOR` and whether the stream is a terminal.
    #[must_use]
    pub fn enabled(self, no_color_set: bool, is_terminal: bool) -> bool {
        match self {
            // `--color always` is an explicit instruction and outranks the environment;
            // `NO_COLOR`'s specification is about defaults, not about overriding a flag
            // the user just typed.
            Self::Always => true,
            Self::Never => false,
            Self::Auto => !no_color_set && is_terminal,
        }
    }
}

/// Escapes untrusted text for display and wraps it so that it cannot be interpolated
/// into a terminal unsanitized by accident.
///
/// Using this type rather than a bare `String` means the compiler, not review, is what
/// stops an API-supplied option name from reaching a terminal raw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Safe(String);

impl Safe {
    /// Sanitizes `input` for display.
    #[must_use]
    pub fn new(input: &str) -> Self {
        Self(sanitize(input))
    }
}

impl std::fmt::Display for Safe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `pad`, not `write_str`. A `Display` impl that writes directly ignores the
        // formatter's width, fill, alignment, and precision *silently*: `{:<12}` around
        // a `Safe` compiles, looks right, and pads nothing. `jev models` aligned its
        // columns that way and emitted a ragged table for any two models whose names
        // differ in length -- which every fixture hid, because they were all the same
        // length, and only the live API exposed.
        f.pad(&self.0)
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;

    /// `Safe` must honour width and alignment, or every column built with `{:<width$}`
    /// pads nothing and the table comes out ragged. This is a formatting bug that
    /// compiles, so only an assertion catches it.
    #[test]
    fn safe_honours_width_and_alignment() {
        assert_eq!(format!("[{:<8}]", Safe::new("jev")), "[jev     ]");
        assert_eq!(format!("[{:>8}]", Safe::new("jev")), "[     jev]");
        assert_eq!(format!("[{:^7}]", Safe::new("jev")), "[  jev  ]");
        assert_eq!(format!("[{:-<6}]", Safe::new("ab")), "[ab----]");
        // Wider than the field: never truncated by width alone.
        assert_eq!(
            format!("[{:<2}]", Safe::new("jev-preview")),
            "[jev-preview]"
        );
        // Plain formatting is unchanged.
        assert_eq!(format!("{}", Safe::new("jev")), "jev");
    }

    /// The case `jev models` actually hit: two names of different lengths must start
    /// their next column at the same offset.
    #[test]
    fn a_padded_column_aligns_names_of_different_lengths() {
        let names = ["jev-latest", "jev-preview"];
        let width = names.iter().map(|n| n.chars().count()).max().unwrap_or(0);
        let rows: Vec<String> = names
            .iter()
            .map(|name| format!("{:<width$}  |", Safe::new(name)))
            .collect();
        let offsets: Vec<usize> = rows.iter().map(|row| row.find('|').unwrap_or(0)).collect();
        assert_eq!(offsets[0], offsets[1], "columns misaligned: {rows:?}");
    }

    #[test]
    fn no_color_disables_automatic_colour() {
        assert!(!ColorChoice::Auto.enabled(true, true));
        assert!(ColorChoice::Auto.enabled(false, true));
        assert!(!ColorChoice::Auto.enabled(false, false));
    }

    #[test]
    fn an_explicit_flag_outranks_the_environment() {
        assert!(ColorChoice::Always.enabled(true, false));
        assert!(!ColorChoice::Never.enabled(false, true));
    }

    #[test]
    fn safe_wraps_sanitized_text() {
        // The point of the type: an API-supplied option name reaching a terminal must
        // go through sanitization, and the type is what makes that hard to forget.
        let hostile = Safe::new("billing\u{1b}[2J");
        assert!(!hostile.to_string().contains('\u{1b}'));
    }
}
