//! `Confidence` — how concentrated an answer's probability distribution is.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

use crate::probability::{Probability, ProbabilityError};

/// A confidence value in `[0, 1]`, as reported on Choice and Score answers.
///
/// Confidence is a *separate concept* from probability, and conflating them is the most
/// common mistake in this problem space:
///
/// * A Choice or Score answer's `confidence` summarizes how concentrated its
///   `probabilities` distribution is. A single peak means high confidence; a flat
///   spread means low confidence.
/// * A Noul answer has **no** confidence. Its `noul` value is the probability of "yes",
///   and `0.5` means "yes and no are similarly likely" — not "medium intensity".
///
/// Because the API never returns a confidence for a Noul, `jev` never synthesizes one.
/// Several community CLIs do; the official documentation is explicit that it does not
/// exist. See <https://docs.typesafe.ai/confidence>.
///
/// # Examples
///
/// ```
/// use jev_core::Confidence;
///
/// let c = Confidence::new(0.82)?;
/// assert!((c.get() - 0.82).abs() < f64::EPSILON);
/// assert!(Confidence::new(1.2).is_err());
/// # Ok::<(), jev_core::ProbabilityError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Confidence(Probability);

impl Confidence {
    /// Validates `value` and wraps it.
    ///
    /// # Errors
    ///
    /// Returns [`ProbabilityError`] for NaN, infinities, and values outside `[0, 1]`.
    pub fn new(value: f64) -> Result<Self, ProbabilityError> {
        Probability::new(value).map(Self)
    }

    /// Returns the underlying value, always finite and within `[0, 1]`.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0.get()
    }
}

impl fmt::Display for Confidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<'de> Deserialize<'de> for Confidence {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Probability::deserialize(deserializer).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_like_a_probability() {
        assert!(Confidence::new(0.0).is_ok());
        assert!(Confidence::new(1.0).is_ok());
        assert!(Confidence::new(-0.1).is_err());
        assert!(Confidence::new(f64::NAN).is_err());
    }

    #[test]
    fn rejects_hostile_json() {
        assert!(serde_json::from_str::<Confidence>("2").is_err());
        assert!(serde_json::from_str::<Confidence>("null").is_err());
    }
}
