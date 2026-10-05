//! The CLI's error type, and the map from a failure to an exit code.
//!
//! # Invariant
//!
//! No `CliError` may carry credential material. The types that hold secrets —
//! `jev_config::Secret` and `jev_client::Credential` — redact on both `Debug` and
//! `Display` and are not `Serialize`, so the only way one could reach a message here is
//! through a deliberate `.expose()`, of which there are none in this crate.

use std::fmt;

use jev_client::ClientError;
use jev_config::{CredentialSourceError, SettingsError};

use crate::exit;

/// The result type used throughout the CLI.
pub type Result<T> = std::result::Result<T, CliError>;

/// A failure, classified by what the user should do about it.
///
/// The classification *is* the exit code: each variant maps to exactly one, and the
/// mapping is what `docs/cli-contract.md` promises.
#[derive(Debug)]
pub enum CliError {
    /// The invocation or its input was wrong. Exit `2`.
    Usage(String),
    /// Cloudflare was selected without an account; offline diagnostics can report it.
    IncompleteCloudflareConfiguration,
    /// No credential, or the API rejected it. Exit `3`.
    Auth(String),
    /// The API could not be reached or was unwell. Exit `4`.
    Unavailable(String),
    /// Output could not be written. Exit `74`.
    Io(String),
    /// The user interrupted the run. Exit `130`.
    Interrupted,
    /// A bug in `jev`. Exit `70`.
    Internal(String),
}

impl CliError {
    /// A usage error.
    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage(message.into())
    }

    /// An internal error, which is always a bug worth reporting.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    /// A failure to write output.
    ///
    /// Separate from [`CliError::internal`] so that a full disk does not tell the user
    /// they have found a bug and invite them to open an issue.
    pub fn io(message: impl Into<String>) -> Self {
        Self::Io(message.into())
    }

    /// The exit code this failure produces.
    #[must_use]
    pub const fn code(&self) -> u8 {
        // The gate and batch codes -- 1, 5, and 6 -- are deliberately absent. Those
        // outcomes still produce data on stdout, so they are returned as a code by the
        // command that produced them rather than raised as an error that would suppress
        // the output the user is entitled to.
        match self {
            Self::Usage(_) | Self::IncompleteCloudflareConfiguration => exit::USAGE,
            Self::Auth(_) => exit::AUTH,
            Self::Unavailable(_) => exit::UNAVAILABLE,
            Self::Io(_) => exit::IO,
            Self::Interrupted => exit::INTERRUPTED,
            Self::Internal(_) => exit::INTERNAL,
        }
    }

    /// A hint appended to the rendered error, where one helps.
    fn hint(&self) -> Option<&'static str> {
        match self {
            Self::Internal(_) => Some(
                "This is a bug in jev. Please report it at \
                 https://github.com/plurp911/jev-cli/issues",
            ),
            Self::Usage(_)
            | Self::IncompleteCloudflareConfiguration
            | Self::Auth(_)
            | Self::Unavailable(_)
            | Self::Io(_)
            | Self::Interrupted => None,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let body = match self {
            Self::Usage(message)
            | Self::Auth(message)
            | Self::Unavailable(message)
            | Self::Io(message)
            | Self::Internal(message) => message.as_str(),
            Self::IncompleteCloudflareConfiguration => {
                "Cloudflare requires --cloudflare-account-id, CLOUDFLARE_ACCOUNT_ID, or the cloudflare_account_id setting"
            }
            Self::Interrupted => "interrupted",
        };
        // Every message here can embed API- or file-supplied text, so all of it is
        // sanitized on the way out rather than at each construction site.
        write!(f, "{}", crate::output::Safe::new(body))?;
        if let Some(hint) = self.hint() {
            write!(f, "\n\n{hint}")?;
        }
        Ok(())
    }
}

impl From<ClientError> for CliError {
    fn from(error: ClientError) -> Self {
        if error.is_auth() {
            return Self::Auth(error.to_string());
        }
        if error.is_unavailable() {
            return Self::Unavailable(error.to_string());
        }
        match error {
            // A malformed response is not the user's fault and not a bug in their
            // command; it means the API said something this version cannot read.
            ClientError::MalformedResponse { .. } => Self::Unavailable(error.to_string()),
            _ => Self::Usage(error.to_string()),
        }
    }
}

impl From<CredentialSourceError> for CliError {
    fn from(error: CredentialSourceError) -> Self {
        Self::Auth(error.to_string())
    }
}

impl From<SettingsError> for CliError {
    fn from(error: SettingsError) -> Self {
        Self::Usage(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_variant_maps_to_its_documented_code() {
        assert_eq!(CliError::usage("x").code(), 2);
        assert_eq!(CliError::Auth("x".to_owned()).code(), 3);
        assert_eq!(CliError::Unavailable("x".to_owned()).code(), 4);
        assert_eq!(CliError::io("x").code(), 74);
        assert_eq!(CliError::Interrupted.code(), 130);
        assert_eq!(CliError::internal("x").code(), 70);
    }

    #[test]
    fn a_rejected_credential_is_an_auth_failure_not_a_usage_error() {
        let error: CliError = ClientError::from_status(401, None).into();
        assert_eq!(error.code(), exit::AUTH);
    }

    #[test]
    fn a_rejected_request_is_a_usage_error_not_an_outage() {
        let error: CliError = ClientError::from_status(422, Some("bad field".to_owned())).into();
        assert_eq!(error.code(), exit::USAGE);
    }

    #[test]
    fn an_overloaded_api_is_an_availability_failure() {
        for status in [429_u16, 500, 529] {
            let error: CliError = ClientError::from_status(status, None).into();
            assert_eq!(error.code(), exit::UNAVAILABLE, "for HTTP {status}");
        }
    }

    #[test]
    fn an_undecodable_response_is_not_reported_as_the_users_mistake() {
        let error: CliError = ClientError::MalformedResponse {
            reason: "answers.x.noul is not a number".to_owned(),
        }
        .into();
        assert_eq!(error.code(), exit::UNAVAILABLE);
    }

    #[test]
    fn a_write_failure_is_not_reported_as_a_bug() {
        // `jev … > /dev/full` is a full disk, not a defect, and an issue-tracker link
        // is the wrong thing to show for one.
        let error = CliError::io("could not write output: no storage space");
        assert_eq!(error.code(), exit::IO);
        assert!(!error.to_string().contains("bug in jev"));
    }

    #[test]
    fn rendered_errors_are_sanitized() {
        // An API error message is attacker-influenced in the custom-endpoint case, and
        // it lands in a terminal.
        let error = CliError::usage("bad \u{1b}[2J input");
        let rendered = error.to_string();
        assert!(!rendered.contains('\u{1b}'), "{rendered}");
    }

    #[test]
    fn an_internal_error_tells_the_user_it_is_a_bug() {
        assert!(
            CliError::internal("impossible")
                .to_string()
                .contains("bug in jev")
        );
    }
}
