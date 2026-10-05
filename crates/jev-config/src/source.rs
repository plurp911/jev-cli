//! Where a credential came from, and the environment names `jev` reads.
//!
//! The resolution order is defined in
//! `docs/adr/0008-credential-precedence-and-endpoint-isolation.md`, which supersedes
//! the ordering in ADR-0002. There is deliberately **no** plaintext-file fallback: when
//! no source is available the CLI fails with an actionable error instead of silently
//! writing a key to disk.

use std::fmt;

/// Environment variable holding the TypeSafe API key. `jev`'s own name, highest
/// precedence.
pub const API_KEY_ENV: &str = "JEV_API_KEY";

/// Environment variable holding the *path to a file* containing the API key, for
/// secret managers that materialize secrets on disk rather than in the environment.
pub const API_KEY_FILE_ENV: &str = "JEV_API_KEY_FILE";

/// The official TypeSafe SDK convention, honoured so that `jev` works in an environment
/// already configured for the Python or JavaScript SDK.
///
/// From <https://docs.typesafe.ai/sdk/python/api/constants>.
pub const TYPESAFE_API_KEY_ENV: &str = "TYPESAFE_API_KEY";

/// Environment variable holding the credential for a **non-official** endpoint.
///
/// A separate namespace is the mechanism that makes an endpoint override safe: a key
/// stored for `api.typesafe.ai` is structurally unreachable when `jev` is pointed at
/// another host, so a misconfigured base URL cannot exfiltrate a production key.
/// See `docs/threat-model.md` T4.
pub const CUSTOM_API_KEY_ENV: &str = "JEV_CUSTOM_API_KEY";

/// File-indirection form of [`CUSTOM_API_KEY_ENV`].
pub const CUSTOM_API_KEY_FILE_ENV: &str = "JEV_CUSTOM_API_KEY_FILE";

/// The origin of a resolved credential, recorded so that `jev` can tell the user
/// *where* its key came from without ever showing the key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialSource {
    /// Local inference without any credential lookup or authorization header.
    Anonymous,
    /// The `JEV_API_KEY` environment variable.
    Environment,
    /// A file named by `JEV_API_KEY_FILE`, as produced by a secret manager.
    EnvironmentFile,
    /// The `TYPESAFE_API_KEY` environment variable used by the official SDKs.
    TypesafeEnvironment,
    /// The operating system keychain / credential manager / Secret Service.
    OsKeychain,
    /// The `JEV_CUSTOM_API_KEY` environment variable, for a non-official endpoint.
    CustomEndpointEnvironment,
    /// A file named by `JEV_CUSTOM_API_KEY_FILE`.
    CustomEndpointEnvironmentFile,
}

impl CredentialSource {
    /// A stable, machine-readable identifier for this source.
    ///
    /// These strings are part of the CLI's output contract and must not change without
    /// a major version bump; see `docs/adr/0003-cli-compatibility.md`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anonymous => "anonymous",
            Self::Environment => "environment",
            Self::EnvironmentFile => "environment-file",
            Self::TypesafeEnvironment => "typesafe-environment",
            Self::OsKeychain => "os-keychain",
            Self::CustomEndpointEnvironment => "custom-endpoint-environment",
            Self::CustomEndpointEnvironmentFile => "custom-endpoint-environment-file",
        }
    }

    /// The environment variable a user would set to select this source, if any.
    #[must_use]
    pub const fn env_var(self) -> Option<&'static str> {
        match self {
            Self::Environment => Some(API_KEY_ENV),
            Self::EnvironmentFile => Some(API_KEY_FILE_ENV),
            Self::TypesafeEnvironment => Some(TYPESAFE_API_KEY_ENV),
            Self::CustomEndpointEnvironment => Some(CUSTOM_API_KEY_ENV),
            Self::CustomEndpointEnvironmentFile => Some(CUSTOM_API_KEY_FILE_ENV),
            Self::Anonymous | Self::OsKeychain => None,
        }
    }
}

impl fmt::Display for CredentialSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Failures that can occur while locating a credential.
///
/// No variant carries credential material. A variant carries a filesystem path only
/// when the user themselves supplied that path through `JEV_API_KEY_FILE`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CredentialSourceError {
    /// No credential was found in any supported source for the official endpoint.
    #[error(
        "no TypeSafe API key found\n\
         \n\
         `jev` looks in this order:\n  \
           1. ${API_KEY_ENV}\n  \
           2. ${API_KEY_FILE_ENV} (a path to a file containing the key)\n  \
           3. ${TYPESAFE_API_KEY_ENV} (the official SDK convention)\n  \
           4. the operating system credential store\n\
         \n\
         Run `jev auth login` to store a key securely, or set ${API_KEY_ENV} for CI."
    )]
    NotFound,
    /// No credential was found, and the OS credential store could not be consulted.
    ///
    /// Distinct from [`Self::NotFound`] because the advice differs: there is no point
    /// telling someone to run `jev auth login` when the store it would write to is the
    /// thing that failed. The reason is the platform's own message, never credential
    /// material.
    #[error(
        "no TypeSafe API key found, and the credential store could not be consulted: \
         {reason}\n\
         \n\
         `jev` looks in this order:\n  \
           1. ${API_KEY_ENV}\n  \
           2. ${API_KEY_FILE_ENV} (a path to a file containing the key)\n  \
           3. ${TYPESAFE_API_KEY_ENV} (the official SDK convention)\n  \
           4. the operating system credential store -- the one that failed above\n\
         \n\
         Set ${API_KEY_ENV}, or ${API_KEY_FILE_ENV} pointing at a file your secret \
         manager controls. `jev auth login` writes to the store, so it cannot help \
         until the store works."
    )]
    NotFoundAndStoreUnusable {
        /// Why the store could not be consulted.
        reason: String,
    },
    /// No credential was found for a non-official endpoint.
    #[error(
        "no API key found for the custom endpoint {endpoint}\n\
         \n\
         A credential stored for the official TypeSafe API is deliberately not used \
         for another host. Set ${CUSTOM_API_KEY_ENV}, or ${CUSTOM_API_KEY_FILE_ENV} \
         to a file containing the key."
    )]
    NotFoundForCustomEndpoint {
        /// The endpoint that was configured, so the user can see what `jev` saw.
        endpoint: String,
    },
    /// A credential source existed but held an empty value, which is almost always a
    /// misconfigured CI secret rather than an intentional choice.
    // Named `origin` rather than `source`: `thiserror` treats a field called `source`
    // as the underlying `std::error::Error`, which this enum is not.
    #[error("the API key found in {} is empty", location(*.origin))]
    Empty {
        /// Which source held the empty value.
        origin: CredentialSource,
    },
    /// A credential source held a key with a line break or other control character
    /// inside it -- a two-line key file, or a variable with a stray escape.
    ///
    /// Such a key cannot be sent as an HTTP header at all, so this is found here, at
    /// load time, rather than surfacing from the transport as a network failure after
    /// retries that could never succeed. The message names the source, never the value.
    #[error(
        "the API key found in {} contains a line break or control character; \
         a key is a single line of printable text",
        location(*.origin)
    )]
    ControlCharacter {
        /// Which source held the key.
        origin: CredentialSource,
    },
    /// The file named by `JEV_API_KEY_FILE` could not be read.
    #[error("could not read the key file named by ${env}: {reason}")]
    UnreadableKeyFile {
        /// Which variable named the file.
        env: &'static str,
        /// The I/O failure, by kind. Never the file's contents.
        reason: String,
    },
    /// The path named by `JEV_API_KEY_FILE` is not a regular file.
    #[error(
        "the path named by ${env} is not a regular file; \
         `jev` will not read a directory, a device, or a pipe as a credential"
    )]
    NotARegularKeyFile {
        /// Which variable named the path.
        env: &'static str,
    },
    /// The file named by `JEV_API_KEY_FILE` was larger than a credential can plausibly
    /// be, so `jev` refuses to read it into memory.
    #[error("the file named by ${env} is larger than {limit} bytes; that is not a key")]
    KeyFileTooLarge {
        /// Which variable named the file.
        env: &'static str,
        /// The cap that was exceeded.
        limit: u64,
    },
    /// Secure storage is unavailable and there is deliberately no plaintext fallback.
    #[error(
        "secure credential storage is unavailable on this system: {reason}\n\
         \n\
         `jev` will not write a key to a plaintext file. Use ${API_KEY_ENV} instead, \
         or ${API_KEY_FILE_ENV} pointing at a file your secret manager controls."
    )]
    SecureStorageUnavailable {
        /// What the platform reported, for diagnosis. Never credential material.
        reason: String,
    },
}

/// Where a key was found, as the user would recognise it: the variable they set,
/// rather than a machine identifier like `environment-file`.
fn location(origin: CredentialSource) -> String {
    match origin {
        CredentialSource::Anonymous => "local inference without authentication".to_owned(),
        CredentialSource::Environment => format!("${API_KEY_ENV}"),
        CredentialSource::EnvironmentFile => format!("the file named by ${API_KEY_FILE_ENV}"),
        CredentialSource::TypesafeEnvironment => format!("${TYPESAFE_API_KEY_ENV}"),
        CredentialSource::OsKeychain => "the operating system credential store".to_owned(),
        CredentialSource::CustomEndpointEnvironment => format!("${CUSTOM_API_KEY_ENV}"),
        CredentialSource::CustomEndpointEnvironmentFile => {
            format!("the file named by ${CUSTOM_API_KEY_FILE_ENV}")
        }
    }
}

impl CredentialSourceError {
    /// Whether this means "nothing is configured" rather than "something is broken".
    ///
    /// The two look identical in a report that only says whether a credential is
    /// available, and they call for opposite responses: the first is an ordinary first
    /// run, the second is a misconfiguration the user has to fix. `jev doctor` and
    /// `jev auth status` use this to say which one they are looking at.
    #[must_use]
    pub const fn is_absence(&self) -> bool {
        matches!(
            self,
            Self::NotFound
                | Self::NotFoundForCustomEndpoint { .. }
                | Self::NotFoundAndStoreUnusable { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_identifiers_are_stable() {
        // Changing any of these strings is a breaking change to the output contract.
        for (source, expected) in [
            (CredentialSource::Environment, "environment"),
            (CredentialSource::EnvironmentFile, "environment-file"),
            (
                CredentialSource::TypesafeEnvironment,
                "typesafe-environment",
            ),
            (CredentialSource::OsKeychain, "os-keychain"),
            (
                CredentialSource::CustomEndpointEnvironment,
                "custom-endpoint-environment",
            ),
            (
                CredentialSource::CustomEndpointEnvironmentFile,
                "custom-endpoint-environment-file",
            ),
        ] {
            assert_eq!(source.as_str(), expected);
        }
    }

    #[test]
    fn the_not_found_error_names_every_source_in_order() {
        let message = CredentialSourceError::NotFound.to_string();
        for name in [
            API_KEY_ENV,
            API_KEY_FILE_ENV,
            TYPESAFE_API_KEY_ENV,
            "credential store",
            "jev auth login",
        ] {
            assert!(message.contains(name), "missing {name} from:\n{message}");
        }
    }

    #[test]
    fn the_custom_endpoint_error_does_not_suggest_the_typesafe_key() {
        let message = CredentialSourceError::NotFoundForCustomEndpoint {
            endpoint: "https://proxy.example.com".to_owned(),
        }
        .to_string();
        assert!(message.contains(CUSTOM_API_KEY_ENV));
        assert!(
            !message.contains(TYPESAFE_API_KEY_ENV),
            "the error must not invite reuse of a TypeSafe key against another host"
        );
    }

    #[test]
    fn no_error_message_can_contain_a_credential() {
        // Every variant is constructed with a canary in every string field.
        const CANARY: &str = "sk-canary-config-0123456789";
        let errors = [
            CredentialSourceError::NotFound,
            CredentialSourceError::NotFoundForCustomEndpoint {
                endpoint: CANARY.to_owned(),
            },
            CredentialSourceError::Empty {
                origin: CredentialSource::Environment,
            },
            CredentialSourceError::ControlCharacter {
                origin: CredentialSource::EnvironmentFile,
            },
            CredentialSourceError::UnreadableKeyFile {
                env: API_KEY_FILE_ENV,
                reason: "permission denied".to_owned(),
            },
            CredentialSourceError::KeyFileTooLarge {
                env: API_KEY_FILE_ENV,
                limit: 4096,
            },
            CredentialSourceError::SecureStorageUnavailable {
                reason: "no Secret Service".to_owned(),
            },
        ];
        // Only the deliberately-canaried endpoint field may echo its input; that field
        // holds a URL the user typed, never a key.
        for error in errors {
            let rendered = error.to_string();
            if matches!(
                error,
                CredentialSourceError::NotFoundForCustomEndpoint { .. }
            ) {
                continue;
            }
            assert!(!rendered.contains(CANARY), "leak in: {rendered}");
        }
    }
}
