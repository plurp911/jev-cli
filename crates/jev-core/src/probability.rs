//! A validated probability in the closed interval `[0, 1]`.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

/// Reasons a floating-point value cannot be a [`Probability`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ProbabilityError {
    /// The value was NaN or an infinity.
    #[error("probability must be a finite number")]
    NotFinite,
    /// The value was finite but outside `[0, 1]`.
    #[error("probability must be within the closed interval [0, 1]")]
    OutOfRange,
}

/// A probability mass reported by a System One model, guaranteed to be finite and
/// within `[0, 1]`.
///
/// Values are validated on construction *and* on deserialization, so a malformed or
/// hostile API response cannot produce an out-of-range probability anywhere downstream.
///
/// # Examples
///
/// ```
/// use jev_core::Probability;
///
/// let p = Probability::new(0.95)?;
/// assert!((p.get() - 0.95).abs() < f64::EPSILON);
/// assert!(Probability::new(1.5).is_err());
/// assert!(Probability::new(f64::NAN).is_err());
/// # Ok::<(), jev_core::ProbabilityError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Probability(f64);

impl Probability {
    /// The certain-`false` end of the interval.
    pub const ZERO: Self = Self(0.0);
    /// The certain-`true` end of the interval.
    pub const ONE: Self = Self(1.0);

    /// Validates `value` and wraps it.
    ///
    /// # Errors
    ///
    /// Returns [`ProbabilityError::NotFinite`] for NaN and infinities, and
    /// [`ProbabilityError::OutOfRange`] for finite values outside `[0, 1]`.
    pub fn new(value: f64) -> Result<Self, ProbabilityError> {
        if !value.is_finite() {
            return Err(ProbabilityError::NotFinite);
        }
        if !(0.0..=1.0).contains(&value) {
            return Err(ProbabilityError::OutOfRange);
        }
        Ok(Self(value))
    }

    /// Returns the underlying value, which is always finite and within `[0, 1]`.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl fmt::Display for Probability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<f64> for Probability {
    type Error = ProbabilityError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for Probability {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = f64::deserialize(deserializer)?;
        Self::new(raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn accepts_the_closed_interval() {
        assert_eq!(Probability::new(0.0).unwrap(), Probability::ZERO);
        assert_eq!(Probability::new(1.0).unwrap(), Probability::ONE);
    }

    #[test]
    fn rejects_out_of_range() {
        assert_eq!(
            Probability::new(-0.000_001),
            Err(ProbabilityError::OutOfRange)
        );
        assert_eq!(
            Probability::new(1.000_001),
            Err(ProbabilityError::OutOfRange)
        );
    }

    #[test]
    fn rejects_non_finite() {
        assert_eq!(Probability::new(f64::NAN), Err(ProbabilityError::NotFinite));
        assert_eq!(
            Probability::new(f64::INFINITY),
            Err(ProbabilityError::NotFinite)
        );
        assert_eq!(
            Probability::new(f64::NEG_INFINITY),
            Err(ProbabilityError::NotFinite)
        );
    }

    #[test]
    fn rejects_hostile_json() {
        // A malformed or adversarial API response must not smuggle an out-of-range
        // probability past the type system.
        assert!(serde_json::from_str::<Probability>("1.0001").is_err());
        assert!(serde_json::from_str::<Probability>("-1").is_err());
        assert!(serde_json::from_str::<Probability>("\"0.5\"").is_err());
    }

    proptest! {
        /// `serde_json`'s float parser is not guaranteed to be correctly rounded, so a
        /// JSON round trip can move a value by one unit in the last place. That is
        /// acceptable for a probability — no downstream decision changes by 1e-16 —
        /// but the invariants that *do* matter must survive exactly: the value stays
        /// finite, stays inside `[0, 1]`, and never becomes a different number.
        #[test]
        fn json_round_trip_preserves_the_value(raw in 0.0_f64..=1.0) {
            let probability = Probability::new(raw).unwrap();
            let encoded = serde_json::to_string(&probability).unwrap();
            let decoded: Probability = serde_json::from_str(&encoded).unwrap();

            prop_assert!(decoded.get().is_finite());
            prop_assert!((0.0..=1.0).contains(&decoded.get()));
            // Both values are non-negative finite doubles, so their bit patterns are
            // monotonic and `abs_diff` on the raw bits is the ULP distance.
            let ulps = probability.get().to_bits().abs_diff(decoded.get().to_bits());
            prop_assert!(ulps <= 1, "round trip moved {} by {ulps} ULPs", probability.get());
        }

        /// The acceptance predicate itself, over every shape of `f64` — NaN, both
        /// infinities, subnormals, negative zero. Asserting only "does not panic" would
        /// pass even if the range check were widened to `-1.0..=2.0`, which is exactly
        /// the regression worth catching.
        #[test]
        fn construction_accepts_exactly_the_closed_unit_interval(
            raw in proptest::num::f64::ANY
        ) {
            let expected = raw.is_finite() && (0.0..=1.0).contains(&raw);
            prop_assert_eq!(
                Probability::new(raw).is_ok(),
                expected,
                "Probability::new({}) disagreed with the predicate", raw
            );
            if let Ok(probability) = Probability::new(raw) {
                // And the value survives construction unchanged.
                prop_assert!((probability.get() - raw).abs() < f64::EPSILON);
            }
        }
    }
}
