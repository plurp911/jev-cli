//! Reading the process environment, behind a trait.
//!
//! # Why
//!
//! Tests that mutate the real process environment are order-dependent and racy: Rust's
//! test harness runs them on threads of one process, and `std::env::set_var` is a
//! global mutation. Injecting the environment instead makes every precedence test
//! deterministic and parallel-safe, which is what `AGENTS.md` §9 requires.
//!
//! `jev` reads only the variables it documents. It never enumerates the environment,
//! and it never loads a `.env` file.

use std::collections::BTreeMap;
use std::fmt;

/// A source of environment variables.
pub trait Environment: fmt::Debug {
    /// Returns the value of `name`, or `None` when it is unset.
    ///
    /// A value that is not valid Unicode is reported as unset: `jev` has no use for one
    /// and silently lossy-converting a credential would be worse than not finding it.
    fn var(&self, name: &str) -> Option<String>;
}

/// The real process environment.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemEnvironment;

impl Environment for SystemEnvironment {
    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// An in-memory environment, for tests.
#[derive(Debug, Default, Clone)]
pub struct MapEnvironment {
    values: BTreeMap<String, String>,
}

impl MapEnvironment {
    /// Sets a variable, returning `self` for chaining.
    #[must_use]
    pub fn with(mut self, name: &str, value: &str) -> Self {
        self.values.insert(name.to_owned(), value.to_owned());
        self
    }
}

impl<const N: usize> From<[(&str, &str); N]> for MapEnvironment {
    fn from(entries: [(&str, &str); N]) -> Self {
        Self {
            values: entries
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
        }
    }
}

impl Environment for MapEnvironment {
    fn var(&self, name: &str) -> Option<String> {
        self.values.get(name).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_map_environment_answers_only_what_it_holds() {
        let environment = MapEnvironment::from([("A", "1")]).with("B", "2");
        assert_eq!(environment.var("A").as_deref(), Some("1"));
        assert_eq!(environment.var("B").as_deref(), Some("2"));
        assert_eq!(environment.var("C"), None);
    }

    #[test]
    fn the_system_environment_reports_an_unset_variable_as_none() {
        assert_eq!(SystemEnvironment.var("JEV_DEFINITELY_NOT_SET_2C7B1F"), None);
    }
}
