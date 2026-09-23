//! Stable, non-cryptographic digests.
//!
//! `jev map` writes a state digest and a request digest into every row so a later
//! `--resume` can tell that the input or the question set changed under it. `jev eval`
//! writes a dataset fingerprint and a request fingerprint into every report so two
//! reports can be compared, and so a threshold can be traced back to the exact data and
//! question it was measured on. Both need the same property, and it is a narrow one:
//! the value must be **the same on every build and every platform**, because it is
//! written to a file and compared by a later run that may be a different binary.
//!
//! These are change detectors, not security controls. Nothing here depends on being
//! hard to collide, and no digest carries any of the content it summarizes.

/// FNV-1a over a string, as sixteen hex characters.
///
/// Written out rather than taken from `DefaultHasher`: the standard library explicitly
/// does not promise `DefaultHasher`'s output is stable across releases, and a toolchain
/// upgrade must not make every resume refuse or every report incomparable.
#[must_use]
pub(crate) fn fnv1a(text: &str) -> String {
    format!("{:016x}", fnv1a_u64(text))
}

/// FNV-1a over a string, as the raw 64-bit value.
#[must_use]
pub(crate) fn fnv1a_u64(text: &str) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = OFFSET;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Spreads a hash so that every output bit depends on every input bit.
///
/// FNV-1a is a fine change detector and a poor source of uniform numbers: its **high**
/// bits avalanche weakly for short, similar inputs, so `row-0`, `row-1`, `row-2` differ
/// in only a few of them. `jev eval`'s holdout projects a digest onto the unit interval
/// by taking the value as a fraction of `u64::MAX`, which reads exactly those high
/// bits — and an unmixed FNV value put roughly five percent of two thousand rows in a
/// holdout that had been asked for twenty. A split that badly skewed is not a holdout.
///
/// This is the `SplitMix64` finalizer: a fixed sequence of xor-shifts and odd multiplies
/// with no state, no randomness, and no platform dependence, so the assignment stays
/// reproducible on every build the way [`fnv1a`] is.
#[must_use]
pub(crate) const fn mix(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// The separator used *between* the parts of one digested item.
///
/// `U+001F` INFORMATION SEPARATOR ONE. Every identifier and every piece of content that
/// reaches a digest has already been validated to contain no control character, so
/// neither separator can appear inside a part and be mistaken for a boundary.
pub(crate) const UNIT: char = '\u{1f}';

/// The separator used *between* items in a digested sequence.
///
/// `U+001E` INFORMATION SEPARATOR TWO, one level up from [`UNIT`].
pub(crate) const RECORD: char = '\u{1e}';

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_digest_is_a_fixed_width_hex_value() {
        for text in ["", "a", "a much longer piece of text with spaces"] {
            let digest = fnv1a(text);
            assert_eq!(digest.len(), 16, "{text:?} digested to {digest}");
            assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn the_digest_is_pinned_to_known_values() {
        // Pinned, not merely self-consistent: these values are written to files and
        // compared by a later run that may be a different build, so "it hashes to
        // whatever this build hashes to" is not the property that matters. Changing the
        // algorithm has to be a deliberate act that fails this test first.
        assert_eq!(fnv1a(""), "cbf29ce484222325");
        assert_eq!(fnv1a("a"), "af63dc4c8601ec8c");
        assert_eq!(fnv1a("jev"), fnv1a("jev"));
        assert_ne!(fnv1a("jev"), fnv1a("vej"));
    }

    #[test]
    fn mixing_spreads_short_similar_keys_across_the_range() {
        // The property the holdout depends on, and the one raw FNV-1a does not have:
        // consecutive ids must not all land in the same corner of the value space.
        let below_a_fifth = |mixed: bool| {
            (0..2000)
                .filter(|index| {
                    let raw = fnv1a_u64(&format!("row-{index}"));
                    let value = if mixed { mix(raw) } else { raw };
                    #[allow(
                        clippy::cast_precision_loss,
                        reason = "the projection is the one `dataset::side` performs"
                    )]
                    let position = value as f64 / u64::MAX as f64;
                    position < 0.2
                })
                .count()
        };
        let mixed = below_a_fifth(true);
        assert!(
            (300..=500).contains(&mixed),
            "{mixed} of 2000 fell in the first fifth after mixing"
        );
    }

    #[test]
    fn mixing_is_pinned_and_reversible_only_in_the_sense_of_being_a_bijection() {
        // Pinned so that a holdout assignment cannot move under a refactor, and checked
        // for distinctness because a finalizer that collided would silently correlate
        // two rows' sides.
        assert_eq!(mix(0), 0);
        assert_eq!(mix(1), 0x5692_161d_100b_05e5);
        let mut seen: Vec<u64> = (0..1000).map(mix).collect();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), before, "the finalizer collided on small inputs");
    }

    #[test]
    fn the_separators_are_control_characters_no_validated_input_can_contain() {
        for separator in [UNIT, RECORD] {
            assert!(separator.is_control());
            assert!(crate::output::is_terminal_actionable(separator));
        }
        assert_ne!(UNIT, RECORD);
    }
}
