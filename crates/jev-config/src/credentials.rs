//! Credential resolution.
//!
//! # The order, and why
//!
//! For the **official** endpoint:
//!
//! | # | Source | Intended for |
//! | - | ------ | ------------ |
//! | 1 | `JEV_API_KEY` | CI, containers, and a deliberate one-invocation override |
//! | 2 | `JEV_API_KEY_FILE` | secret managers that materialize a key on disk |
//! | 3 | `TYPESAFE_API_KEY` | an environment already set up for the official SDKs |
//! | 4 | the OS credential store | interactive local use, via `jev auth login` |
//!
//! Environment before keychain is the point that matters. A stored key that silently
//! wins over an explicitly exported one is a trap: it makes `JEV_API_KEY=… jev …`
//! quietly do the wrong thing, and it makes a stale keychain entry impossible to work
//! around without deleting it. This supersedes the ordering in ADR-0002; the reasoning
//! is in ADR-0008.
//!
//! For a **non-official** endpoint the TypeSafe sources are not consulted at all — only
//! `JEV_CUSTOM_API_KEY` and `JEV_CUSTOM_API_KEY_FILE`. That is what makes an endpoint
//! override safe rather than merely discouraged: a production key is structurally
//! unreachable when `jev` is pointed at another host (`docs/threat-model.md` T4).
//!
//! There is **no plaintext-file fallback** anywhere. If nothing is available, `jev`
//! fails with an error that names every source it tried.

use std::io::Read as _;
use std::path::Path;

use zeroize::Zeroizing;

use crate::env::Environment;
use crate::secret::Secret;
use crate::source::{
    API_KEY_ENV, API_KEY_FILE_ENV, CUSTOM_API_KEY_ENV, CUSTOM_API_KEY_FILE_ENV, CredentialSource,
    CredentialSourceError, TYPESAFE_API_KEY_ENV,
};
use crate::store::{OFFICIAL_ACCOUNT, SecretStore};

/// Largest file `jev` will read when following `JEV_API_KEY_FILE`.
///
/// An API key is tens of bytes. Anything approaching this is a misconfiguration —
/// `JEV_API_KEY_FILE=/dev/zero`, or a path pointed at the wrong artifact — and reading
/// it would be an unbounded allocation from a value the user only indirectly controls.
pub const MAX_KEY_FILE_BYTES: u64 = 4096;

/// A resolved credential and the source it came from.
#[derive(Debug)]
pub struct ResolvedCredential {
    /// The key.
    pub secret: Secret,
    /// Where it was found. Reportable; the key itself never is.
    pub source: CredentialSource,
}

/// Why a key cannot be sent, if it cannot.
///
/// The one rule, shared by resolution and by `jev auth login`, so that `login` cannot
/// store a key that every later command would refuse. Asked of a [`Secret`] without
/// exposing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyDefect {
    /// The key has no characters. Callers trim the ends first.
    Empty,
    /// The key has a line break or other control character inside it, so it cannot be
    /// an HTTP header.
    ControlCharacter,
}

impl KeyDefect {
    /// The defect in `secret`, or `None` when it could be sent.
    #[must_use]
    pub fn of(secret: &Secret) -> Option<Self> {
        if secret.is_empty() {
            Some(Self::Empty)
        } else if secret.has_control_character() {
            Some(Self::ControlCharacter)
        } else {
            None
        }
    }
}

/// One credential source, as [`Credentials::availability`] found it.
///
/// `set` and `present` are separate because they come apart: a variable exported
/// as blank, or a key file with a second line, is set -- and resolution stops at it
/// rather than falling through -- but it holds nothing `jev` will send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceStatus {
    /// Which source.
    pub source: CredentialSource,
    /// Whether it is set: the variable exists, even if blank, or the store holds an
    /// entry. `None` when the source could not be checked.
    pub set: Option<bool>,
    /// Whether it holds a key `jev` would send: the documented meaning of `present` in
    /// `jev.doctor/v1` and `jev.auth/v1`. `Some(false)` both for a source that is set
    /// but refused and for one that is not set. `None` when it could not be checked.
    pub present: Option<bool>,
}

/// Whether each credential source is populated, without reading any value.
///
/// This is what `jev doctor` and `jev auth status` report: the shape of the user's
/// configuration, with no secret anywhere in the answer. Sources are listed in
/// resolution order, so the report reads the way the resolver works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialAvailability {
    /// The source that would win, if any.
    pub effective: Option<CredentialSource>,
    /// Every source `jev` would consult for this endpoint, in order, with whether it is
    /// set and whether it is usable.
    pub sources: Vec<SourceStatus>,
    /// Why the OS credential store could not be consulted, when it could not be.
    ///
    /// A platform message, never credential material.
    pub keychain_error: Option<String>,
    /// Why resolution failed, when a source was populated but unusable.
    ///
    /// `effective` being `None` is ambiguous on its own: it means either "no source is
    /// set" or "a source is set and is broken". The second case used to be reported as
    /// the first — a blank `JEV_API_KEY` showed as "not set", and a `JEV_API_KEY_FILE`
    /// pointing at a directory showed as "present" with "in use: none" — so the
    /// diagnostic disagreed with the resolver about the same environment and offered
    /// remediation for a problem the user did not have. ADR-0008 is explicit that a
    /// populated-but-empty source is an error naming that source, not a skip.
    ///
    /// The message names the source and the reason; it never carries a value or a path
    /// (see `CredentialSourceError`). It is `None` when nothing is set at all, which is
    /// the genuinely unconfigured case.
    pub resolution_error: Option<String>,
}

impl CredentialAvailability {
    /// Whether a given source is populated.
    #[must_use]
    pub fn is_present(&self, source: CredentialSource) -> Option<bool> {
        self.sources
            .iter()
            .find(|status| status.source == source)
            .and_then(|status| status.present)
    }
}

/// Resolves credentials for one endpoint.
#[derive(Debug)]
pub struct Credentials<'a> {
    environment: &'a dyn Environment,
    store: &'a (dyn SecretStore + Send + Sync),
    /// `true` when the configured endpoint is the official TypeSafe API.
    official: bool,
    /// The configured endpoint, for error messages only.
    endpoint: String,
}

impl<'a> Credentials<'a> {
    /// Builds a resolver.
    ///
    /// `official` decides which credential namespace applies, and is the single switch
    /// that keeps a TypeSafe key away from a third-party host.
    #[must_use]
    pub fn new(
        environment: &'a dyn Environment,
        store: &'a (dyn SecretStore + Send + Sync),
        official: bool,
        endpoint: impl Into<String>,
    ) -> Self {
        Self {
            environment,
            store,
            official,
            endpoint: endpoint.into(),
        }
    }

    /// Finds a credential, or explains why there is none.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialSourceError`] when no source has a usable value, when a
    /// source holds an empty value or one with a line break or control character in it,
    /// or when a named key file cannot be read.
    pub fn resolve(&self) -> Result<ResolvedCredential, CredentialSourceError> {
        if self.official {
            self.resolve_official()
        } else {
            self.resolve_custom()
        }
    }

    fn resolve_official(&self) -> Result<ResolvedCredential, CredentialSourceError> {
        if let Some(found) = self.lookup_env(API_KEY_ENV, CredentialSource::Environment)? {
            return Ok(found);
        }
        if let Some(found) =
            self.lookup_env_file(API_KEY_FILE_ENV, CredentialSource::EnvironmentFile)?
        {
            return Ok(found);
        }
        if let Some(found) =
            self.lookup_env(TYPESAFE_API_KEY_ENV, CredentialSource::TypesafeEnvironment)?
        {
            return Ok(found);
        }
        // The store is consulted last, and a store that is simply unavailable is not an
        // error here — it only becomes one if nothing else supplied a key.
        //
        // The failure reason is kept rather than discarded. A cancelled or locked
        // Keychain, or a duplicate Secret Service entry, used to surface from `jev ask`
        // as a plain "no TypeSafe API key found" listing the store among the places
        // looked — with no hint that it had actually *failed*, and with advice to run
        // `jev auth login`, which writes to the store that is broken.
        let store_error = match self.store.get(OFFICIAL_ACCOUNT) {
            Ok(Some(secret)) => return usable(secret, CredentialSource::OsKeychain),
            Ok(None) => None,
            Err(error) => Some(error.to_string()),
        };
        Err(match store_error {
            Some(reason) => CredentialSourceError::NotFoundAndStoreUnusable { reason },
            None => CredentialSourceError::NotFound,
        })
    }

    fn resolve_custom(&self) -> Result<ResolvedCredential, CredentialSourceError> {
        if let Some(found) = self.lookup_env(
            CUSTOM_API_KEY_ENV,
            CredentialSource::CustomEndpointEnvironment,
        )? {
            return Ok(found);
        }
        if let Some(found) = self.lookup_env_file(
            CUSTOM_API_KEY_FILE_ENV,
            CredentialSource::CustomEndpointEnvironmentFile,
        )? {
            return Ok(found);
        }
        Err(CredentialSourceError::NotFoundForCustomEndpoint {
            endpoint: self.endpoint.clone(),
        })
    }

    fn lookup_env(
        &self,
        name: &'static str,
        source: CredentialSource,
    ) -> Result<Option<ResolvedCredential>, CredentialSourceError> {
        let Some(raw) = self.environment.var(name) else {
            return Ok(None);
        };
        // The plaintext lands in an ordinary `String` on the way out of the
        // environment, so it is wrapped immediately: without this the original copy is
        // dropped without being cleared, on every invocation.
        let raw = Zeroizing::new(raw);
        usable(Secret::new(raw.trim().to_owned()), source).map(Some)
    }

    fn lookup_env_file(
        &self,
        name: &'static str,
        source: CredentialSource,
    ) -> Result<Option<ResolvedCredential>, CredentialSourceError> {
        let Some(path) = self.environment.var(name) else {
            return Ok(None);
        };
        let secret = read_key_file(Path::new(path.trim()), name)?;
        usable(secret, source).map(Some)
    }

    /// The environment-only shape of each source, in resolution order.
    ///
    /// Consults the environment and nothing else: no file is opened and the OS
    /// credential store is not queried. That is what makes it safe for `--dry-run`,
    /// which promises to touch nothing — [`Self::availability`] resolves, and resolving
    /// reads the key file and can raise a Keychain unlock prompt.
    ///
    /// The OS store is therefore absent from the list rather than reported as unset:
    /// "not checked" and "not present" are different answers, and only the caller that
    /// is willing to pay for the check should get the second one.
    #[must_use]
    pub fn sources_shape(&self) -> Vec<(CredentialSource, Option<bool>)> {
        let set = |name: &str| {
            Some(
                self.environment
                    .var(name)
                    .is_some_and(|value| !value.trim().is_empty()),
            )
        };
        let present = |name: &str| Some(self.environment.var(name).is_some());

        if self.official {
            vec![
                (CredentialSource::Environment, set(API_KEY_ENV)),
                (CredentialSource::EnvironmentFile, present(API_KEY_FILE_ENV)),
                (
                    CredentialSource::TypesafeEnvironment,
                    set(TYPESAFE_API_KEY_ENV),
                ),
            ]
        } else {
            vec![
                (
                    CredentialSource::CustomEndpointEnvironment,
                    set(CUSTOM_API_KEY_ENV),
                ),
                (
                    CredentialSource::CustomEndpointEnvironmentFile,
                    present(CUSTOM_API_KEY_FILE_ENV),
                ),
            ]
        }
    }

    /// Reports which sources are populated, without reading any value into a place it
    /// could be printed.
    ///
    /// Unlike [`Self::sources_shape`], this **resolves**: it reads the file named by
    /// `JEV_API_KEY_FILE` and queries the OS credential store, which is what lets it
    /// report *why* a populated source is unusable. That cost is right for `jev doctor`
    /// and `jev auth status`, whose job is to find out, and wrong for `--dry-run`.
    #[must_use]
    pub fn availability(&self) -> CredentialAvailability {
        // Each source is judged by the resolver's own lookup, so `present` here cannot
        // drift from what `resolve` would accept. `set` is only "the variable is set":
        // a blank value is set, because resolution stops at it.
        let status = |source: CredentialSource,
                      name: &'static str,
                      lookup: &dyn Fn() -> Result<
            Option<ResolvedCredential>,
            CredentialSourceError,
        >| {
            let set = self.environment.var(name).is_some();
            SourceStatus {
                source,
                set: Some(set),
                present: Some(set && matches!(lookup(), Ok(Some(_)))),
            }
        };
        let env = |source, name| status(source, name, &|| self.lookup_env(name, source));
        let file = |source, name| status(source, name, &|| self.lookup_env_file(name, source));

        let mut sources = Vec::new();
        let mut keychain_error = None;

        if self.official {
            sources.push(env(CredentialSource::Environment, API_KEY_ENV));
            sources.push(file(CredentialSource::EnvironmentFile, API_KEY_FILE_ENV));
            sources.push(env(
                CredentialSource::TypesafeEnvironment,
                TYPESAFE_API_KEY_ENV,
            ));
            let (set, present) = match self.store.get(OFFICIAL_ACCOUNT) {
                Ok(Some(secret)) => (Some(true), Some(KeyDefect::of(&secret).is_none())),
                Ok(None) => (Some(false), Some(false)),
                Err(error) => {
                    keychain_error = Some(error.to_string());
                    (None, None)
                }
            };
            sources.push(SourceStatus {
                source: CredentialSource::OsKeychain,
                set,
                present,
            });
        } else {
            // The OS store is deliberately not consulted for a non-official endpoint,
            // so it is not listed: showing it would imply it could be used.
            sources.push(env(
                CredentialSource::CustomEndpointEnvironment,
                CUSTOM_API_KEY_ENV,
            ));
            sources.push(file(
                CredentialSource::CustomEndpointEnvironmentFile,
                CUSTOM_API_KEY_FILE_ENV,
            ));
        }

        let (effective, resolution_error) = match self.resolve() {
            Ok(resolved) => (Some(resolved.source), None),
            // `NotFound` and its custom-endpoint twin mean nothing is configured, which
            // the per-source list already says; repeating it as an error would turn an
            // ordinary first run into something that looks broken.
            Err(error) if error.is_absence() => (None, None),
            Err(error) => (None, Some(error.to_string())),
        };

        CredentialAvailability {
            effective,
            sources,
            keychain_error,
            resolution_error,
        }
    }
}

/// Accepts a key only if it could actually be sent.
///
/// A blank value is a misconfigured CI secret, not a choice to be unauthenticated;
/// saying so beats a confusing 401 later. A key with a line break or control character
/// inside it -- a two-line key file, a stray escape -- cannot be an HTTP header at all,
/// and used to surface from the transport as "could not reach the API endpoint" after
/// three retries, though nothing was ever sent. Both are refused here, naming the
/// source and never the value. Only the ends are trimmed, by the callers: an interior
/// character is not whitespace to forgive, it is evidence of the wrong input.
fn usable(
    secret: Secret,
    source: CredentialSource,
) -> Result<ResolvedCredential, CredentialSourceError> {
    match KeyDefect::of(&secret) {
        Some(KeyDefect::Empty) => Err(CredentialSourceError::Empty { origin: source }),
        Some(KeyDefect::ControlCharacter) => {
            Err(CredentialSourceError::ControlCharacter { origin: source })
        }
        None => Ok(ResolvedCredential { secret, source }),
    }
}

/// Reads a key from a file the user explicitly named, with a hard size cap.
fn read_key_file(path: &Path, env: &'static str) -> Result<Secret, CredentialSourceError> {
    // Checked before opening. `File::open` on a FIFO with no writer blocks forever,
    // which in CI is an unbounded stall with no diagnostic, and a secret manager never
    // materializes a key as anything but a regular file.
    let metadata =
        std::fs::metadata(path).map_err(|error| CredentialSourceError::UnreadableKeyFile {
            env,
            reason: error.kind().to_string(),
        })?;
    if !metadata.is_file() {
        return Err(CredentialSourceError::NotARegularKeyFile { env });
    }

    let file = std::fs::File::open(path).map_err(|error| {
        // The kind, not the OS message: on some platforms the message embeds the path,
        // and the path may itself be sensitive in a CI log.
        CredentialSourceError::UnreadableKeyFile {
            env,
            reason: error.kind().to_string(),
        }
    })?;

    // `take` bounds the read regardless of what the file claims its length is. The
    // buffer is `Zeroizing` because it holds the key before `Secret` does.
    let mut buffer = Zeroizing::new(String::new());
    let read = file
        .take(MAX_KEY_FILE_BYTES + 1)
        .read_to_string(&mut buffer)
        .map_err(|error| CredentialSourceError::UnreadableKeyFile {
            env,
            reason: error.kind().to_string(),
        })?;
    if read as u64 > MAX_KEY_FILE_BYTES {
        return Err(CredentialSourceError::KeyFileTooLarge {
            env,
            limit: MAX_KEY_FILE_BYTES,
        });
    }
    // A key written by `printf '%s\n'` or by a secret manager carries a trailing
    // newline; sending it would produce a confusing 401.
    Ok(Secret::new(buffer.trim().to_owned()))
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;
    use crate::env::MapEnvironment;
    use crate::store::{MemoryStore, UnavailableStore};

    const KEY: &str = "sk-canary-resolution-0123456789";

    fn official<'a>(
        environment: &'a MapEnvironment,
        store: &'a (dyn SecretStore + Send + Sync),
    ) -> Credentials<'a> {
        Credentials::new(environment, store, true, "https://api.typesafe.ai")
    }

    #[test]
    fn the_environment_wins_over_the_keychain() {
        // The precedence decision of ADR-0008, asserted directly.
        let store = MemoryStore::new();
        store
            .set(OFFICIAL_ACCOUNT, &Secret::new("from-keychain".to_owned()))
            .unwrap();
        let environment = MapEnvironment::from([(API_KEY_ENV, "from-env")]);
        let resolved = official(&environment, &store).resolve().unwrap();
        assert_eq!(resolved.source, CredentialSource::Environment);
        assert_eq!(resolved.secret.expose(), "from-env");
    }

    #[test]
    fn the_full_precedence_order_holds() {
        let store = MemoryStore::new();
        store
            .set(OFFICIAL_ACCOUNT, &Secret::new("keychain".to_owned()))
            .unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "from-file").unwrap();
        let path = file.path().to_string_lossy().into_owned();

        let all = MapEnvironment::from([
            (API_KEY_ENV, "one"),
            (API_KEY_FILE_ENV, path.as_str()),
            (TYPESAFE_API_KEY_ENV, "three"),
        ]);
        assert_eq!(
            official(&all, &store).resolve().unwrap().source,
            CredentialSource::Environment
        );

        let without_first = MapEnvironment::from([
            (API_KEY_FILE_ENV, path.as_str()),
            (TYPESAFE_API_KEY_ENV, "three"),
        ]);
        assert_eq!(
            official(&without_first, &store).resolve().unwrap().source,
            CredentialSource::EnvironmentFile
        );

        let only_typesafe = MapEnvironment::from([(TYPESAFE_API_KEY_ENV, "three")]);
        assert_eq!(
            official(&only_typesafe, &store).resolve().unwrap().source,
            CredentialSource::TypesafeEnvironment
        );

        let nothing = MapEnvironment::default();
        assert_eq!(
            official(&nothing, &store).resolve().unwrap().source,
            CredentialSource::OsKeychain
        );
    }

    #[test]
    fn nothing_anywhere_is_an_actionable_error_not_a_silent_anonymous_call() {
        let store = MemoryStore::new();
        let environment = MapEnvironment::default();
        let error = official(&environment, &store).resolve().unwrap_err();
        assert!(matches!(error, CredentialSourceError::NotFound));
    }

    #[test]
    fn an_unavailable_keychain_does_not_break_environment_resolution() {
        // A headless Linux box with no Secret Service must still work from CI secrets.
        let store = UnavailableStore::new("no Secret Service");
        let environment = MapEnvironment::from([(API_KEY_ENV, KEY)]);
        assert_eq!(
            official(&environment, &store).resolve().unwrap().source,
            CredentialSource::Environment
        );
    }

    #[test]
    fn an_empty_environment_value_is_reported_rather_than_skipped() {
        // A blank CI secret is the classic cause of a mystifying 401.
        let store = MemoryStore::new();
        let environment = MapEnvironment::from([(API_KEY_ENV, "   ")]);
        assert_eq!(
            official(&environment, &store).resolve().unwrap_err(),
            CredentialSourceError::Empty {
                origin: CredentialSource::Environment
            }
        );
    }

    /// A key with an interior line break or control character cannot be sent as a
    /// header. Live testing found it surfacing as "could not reach the API endpoint"
    /// after three retries, though nothing was ever sent. It is a credential problem,
    /// and it is found here, naming the variable and never the value.
    #[test]
    fn a_key_with_a_control_character_is_refused_naming_the_variable() {
        let store = MemoryStore::new();
        for (name, source) in [
            (API_KEY_ENV, CredentialSource::Environment),
            (TYPESAFE_API_KEY_ENV, CredentialSource::TypesafeEnvironment),
        ] {
            for value in [
                format!("{KEY}\u{1}ctl"),
                format!("{KEY}\nsecond-line"),
                format!("{KEY}\ttab"),
                format!("{KEY}\u{7f}del"),
                format!("{KEY}\u{85}nel"),
            ] {
                let environment = MapEnvironment::from([(name, value.as_str())]);
                let error = official(&environment, &store).resolve().unwrap_err();
                assert_eq!(
                    error,
                    CredentialSourceError::ControlCharacter { origin: source },
                    "{name}={value:?}"
                );
                let rendered = error.to_string();
                assert!(rendered.contains(name), "does not name {name}: {rendered}");
                assert!(!rendered.contains(KEY), "the key leaked: {rendered}");
            }
        }
    }

    /// The common real-world shape: a key file with the key on one line and something
    /// else -- a comment, a second key -- on the next.
    #[test]
    fn a_two_line_key_file_is_refused_naming_the_variable() {
        let store = MemoryStore::new();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), format!("{KEY}\nsomething-else\n")).unwrap();
        let path = file.path().to_string_lossy().into_owned();
        let environment = MapEnvironment::from([(API_KEY_FILE_ENV, path.as_str())]);
        let error = official(&environment, &store).resolve().unwrap_err();
        assert_eq!(
            error,
            CredentialSourceError::ControlCharacter {
                origin: CredentialSource::EnvironmentFile
            }
        );
        let rendered = error.to_string();
        assert!(rendered.contains(API_KEY_FILE_ENV), "{rendered}");
        assert!(!rendered.contains(KEY), "the key leaked: {rendered}");
        assert!(!rendered.contains(&path), "the path leaked: {rendered}");

        let custom_environment = MapEnvironment::from([(CUSTOM_API_KEY_FILE_ENV, path.as_str())]);
        assert_eq!(
            custom(&custom_environment, &store).resolve().unwrap_err(),
            CredentialSourceError::ControlCharacter {
                origin: CredentialSource::CustomEndpointEnvironmentFile
            }
        );
    }

    #[test]
    fn a_custom_endpoint_key_with_a_control_character_is_refused() {
        let store = MemoryStore::new();
        let value = format!("{KEY}\u{1}");
        let environment = MapEnvironment::from([(CUSTOM_API_KEY_ENV, value.as_str())]);
        let error = custom(&environment, &store).resolve().unwrap_err();
        assert_eq!(
            error,
            CredentialSourceError::ControlCharacter {
                origin: CredentialSource::CustomEndpointEnvironment
            }
        );
        assert!(error.to_string().contains(CUSTOM_API_KEY_ENV));
    }

    #[test]
    fn a_stored_key_with_a_control_character_is_refused() {
        let store = MemoryStore::new();
        store
            .set(OFFICIAL_ACCOUNT, &Secret::new(format!("{KEY}\n{KEY}")))
            .unwrap();
        let environment = MapEnvironment::default();
        let error = official(&environment, &store).resolve().unwrap_err();
        assert_eq!(
            error,
            CredentialSourceError::ControlCharacter {
                origin: CredentialSource::OsKeychain
            }
        );
        assert!(error.to_string().contains("credential store"), "{error}");
    }

    /// Only the ends are whitespace to forgive; an interior space is not a control
    /// character and is left for the API to judge.
    #[test]
    fn surrounding_whitespace_is_still_trimmed_rather_than_refused() {
        let store = MemoryStore::new();
        let value = format!("\t {KEY}\r\n");
        let environment = MapEnvironment::from([(API_KEY_ENV, value.as_str())]);
        let resolved = official(&environment, &store).resolve().unwrap();
        assert_eq!(resolved.secret.expose(), KEY);
    }

    /// The empty-value error names the variable the user set, not an internal
    /// identifier: "found in environment-file" did not tell anyone what to fix.
    #[test]
    fn an_empty_value_names_the_variable_that_held_it() {
        let store = MemoryStore::new();
        for name in [API_KEY_ENV, TYPESAFE_API_KEY_ENV] {
            let environment = MapEnvironment::from([(name, "  ")]);
            let rendered = official(&environment, &store)
                .resolve()
                .unwrap_err()
                .to_string();
            assert_eq!(rendered, format!("the API key found in ${name} is empty"));
        }
        let environment = MapEnvironment::from([(CUSTOM_API_KEY_ENV, "")]);
        assert_eq!(
            custom(&environment, &store)
                .resolve()
                .unwrap_err()
                .to_string(),
            format!("the API key found in ${CUSTOM_API_KEY_ENV} is empty")
        );

        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "\n").unwrap();
        let path = file.path().to_string_lossy().into_owned();
        let environment = MapEnvironment::from([(API_KEY_FILE_ENV, path.as_str())]);
        let rendered = official(&environment, &store)
            .resolve()
            .unwrap_err()
            .to_string();
        assert_eq!(
            rendered,
            format!("the API key found in the file named by ${API_KEY_FILE_ENV} is empty")
        );
        assert!(!rendered.contains(&path), "the path leaked: {rendered}");
    }

    #[test]
    fn a_key_file_is_read_and_trimmed() {
        let store = MemoryStore::new();
        let file = tempfile::NamedTempFile::new().unwrap();
        writeln!(file.as_file(), "{KEY}").unwrap();
        let environment =
            MapEnvironment::from([(API_KEY_FILE_ENV, file.path().to_string_lossy().as_ref())]);
        let resolved = official(&environment, &store).resolve().unwrap();
        assert_eq!(resolved.secret.expose(), KEY);
        assert_eq!(resolved.source, CredentialSource::EnvironmentFile);
    }

    #[test]
    fn a_missing_key_file_reports_the_kind_not_the_path() {
        let store = MemoryStore::new();
        let environment =
            MapEnvironment::from([(API_KEY_FILE_ENV, "/nonexistent/secret/path/key.txt")]);
        let error = official(&environment, &store).resolve().unwrap_err();
        let rendered = error.to_string();
        assert!(matches!(
            error,
            CredentialSourceError::UnreadableKeyFile { .. }
        ));
        assert!(
            !rendered.contains("/nonexistent/secret/path"),
            "the path leaked into the error: {rendered}"
        );
    }

    #[test]
    fn an_oversized_key_file_is_refused_rather_than_buffered() {
        let store = MemoryStore::new();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(
            file.path(),
            "x".repeat(usize::try_from(MAX_KEY_FILE_BYTES + 100).unwrap()),
        )
        .unwrap();
        let environment =
            MapEnvironment::from([(API_KEY_FILE_ENV, file.path().to_string_lossy().as_ref())]);
        assert!(matches!(
            official(&environment, &store).resolve().unwrap_err(),
            CredentialSourceError::KeyFileTooLarge { .. }
        ));
    }

    fn custom<'a>(
        environment: &'a MapEnvironment,
        store: &'a (dyn SecretStore + Send + Sync),
    ) -> Credentials<'a> {
        Credentials::new(environment, store, false, "https://jev.internal.example")
    }

    /// The boundary itself, not a value comfortably past it.
    ///
    /// `take(MAX + 1)` and `read > MAX` is the kind of pair that is off by one in
    /// either direction and still passes a test written at `MAX + 100`.
    #[test]
    fn a_key_file_of_exactly_the_maximum_size_is_accepted() {
        let store = MemoryStore::new();
        let file = tempfile::NamedTempFile::new().unwrap();
        let limit = usize::try_from(MAX_KEY_FILE_BYTES).unwrap();
        std::fs::write(file.path(), "k".repeat(limit)).unwrap();
        let environment =
            MapEnvironment::from([(API_KEY_FILE_ENV, file.path().to_string_lossy().as_ref())]);
        let resolved = official(&environment, &store)
            .resolve()
            .expect("a file at exactly the limit is not too large");
        assert_eq!(resolved.secret.expose().len(), limit);
    }

    #[test]
    fn a_key_file_one_byte_over_the_maximum_is_refused() {
        let store = MemoryStore::new();
        let file = tempfile::NamedTempFile::new().unwrap();
        let over = usize::try_from(MAX_KEY_FILE_BYTES).unwrap() + 1;
        std::fs::write(file.path(), "k".repeat(over)).unwrap();
        let environment =
            MapEnvironment::from([(API_KEY_FILE_ENV, file.path().to_string_lossy().as_ref())]);
        assert!(matches!(
            official(&environment, &store).resolve().unwrap_err(),
            CredentialSourceError::KeyFileTooLarge { .. }
        ));
    }

    /// The non-official file variable has a resolution path of its own, and until now
    /// only its *failure* was covered.
    #[test]
    fn the_custom_endpoint_key_file_resolves_and_is_trimmed() {
        let store = MemoryStore::new();
        let file = tempfile::NamedTempFile::new().unwrap();
        // As `printf '%s\n'` or a secret manager would write it.
        std::fs::write(file.path(), format!("  {KEY}\n")).unwrap();
        let environment = MapEnvironment::from([(
            CUSTOM_API_KEY_FILE_ENV,
            file.path().to_string_lossy().as_ref(),
        )]);
        let resolved = custom(&environment, &store)
            .resolve()
            .expect("the custom key file resolves");
        assert_eq!(
            resolved.source,
            CredentialSource::CustomEndpointEnvironmentFile
        );
        assert_eq!(resolved.secret.expose(), KEY);
    }

    #[test]
    fn the_custom_variable_wins_over_the_custom_key_file() {
        let store = MemoryStore::new();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "from-file").unwrap();
        let environment = MapEnvironment::from([
            (CUSTOM_API_KEY_ENV, "from-env"),
            (
                CUSTOM_API_KEY_FILE_ENV,
                file.path().to_string_lossy().as_ref(),
            ),
        ]);
        let resolved = custom(&environment, &store).resolve().unwrap();
        assert_eq!(resolved.secret.expose(), "from-env");
    }

    /// A non-official endpoint must not even *list* the OS store, because listing it
    /// implies it could be used — and it never is (T4).
    #[test]
    fn availability_for_a_custom_endpoint_never_mentions_the_os_store() {
        let store = MemoryStore::new();
        store
            .set(OFFICIAL_ACCOUNT, &Secret::new(KEY.to_owned()))
            .unwrap();
        let environment = MapEnvironment::from([
            (API_KEY_ENV, KEY),
            (TYPESAFE_API_KEY_ENV, KEY),
            (CUSTOM_API_KEY_ENV, "custom-key"),
        ]);
        let availability = custom(&environment, &store).availability();

        let listed: Vec<CredentialSource> = availability
            .sources
            .iter()
            .map(|status| status.source)
            .collect();
        assert_eq!(
            listed,
            vec![
                CredentialSource::CustomEndpointEnvironment,
                CredentialSource::CustomEndpointEnvironmentFile,
            ],
            "a custom endpoint listed a source it cannot use"
        );
        assert_eq!(
            availability.effective,
            Some(CredentialSource::CustomEndpointEnvironment)
        );
        assert!(availability.keychain_error.is_none());
    }

    /// And with nothing set, it reports no effective source rather than falling back.
    #[test]
    fn availability_for_a_custom_endpoint_with_nothing_set_has_no_effective_source() {
        let store = MemoryStore::new();
        store
            .set(OFFICIAL_ACCOUNT, &Secret::new(KEY.to_owned()))
            .unwrap();
        let environment = MapEnvironment::from([(API_KEY_ENV, KEY)]);
        let availability = custom(&environment, &store).availability();
        assert_eq!(availability.effective, None);
        for status in &availability.sources {
            assert_eq!(status.set, Some(false), "{status:?} reported as available");
            assert_eq!(status.present, Some(false), "{status:?} reported as usable");
        }
    }

    fn status_of(availability: &CredentialAvailability, source: CredentialSource) -> SourceStatus {
        *availability
            .sources
            .iter()
            .find(|status| status.source == source)
            .expect("the source is listed")
    }

    /// A blank variable is not a credential. An exported-but-empty `JEV_API_KEY` is a
    /// common CI shape, and treating it as usable produces a 401 instead of a clear
    /// error.
    ///
    /// It is, however, *set*, and the resolver stops at it rather than falling through
    /// to the store. Reporting it as "not set" made the diagnostic disagree with the
    /// resolver about the same environment, so it is reported as set and unusable.
    #[test]
    fn a_blank_environment_variable_is_set_but_not_usable() {
        let store = MemoryStore::new();
        store
            .set(OFFICIAL_ACCOUNT, &Secret::new(KEY.to_owned()))
            .unwrap();
        let environment = MapEnvironment::from([(API_KEY_ENV, "   ")]);
        let availability = official(&environment, &store).availability();
        let status = status_of(&availability, CredentialSource::Environment);
        assert_eq!(status.set, Some(true));
        assert_eq!(status.present, Some(false));
        // The store below it is fine, and still not used: resolution stopped here.
        let keychain = status_of(&availability, CredentialSource::OsKeychain);
        assert_eq!(keychain.present, Some(true));
        assert_eq!(availability.effective, None);
    }

    /// Each source is judged by the same rule the resolver applies, whatever its kind.
    #[test]
    fn a_source_holding_a_control_character_is_set_but_not_usable() {
        let store = MemoryStore::new();
        store
            .set(
                OFFICIAL_ACCOUNT,
                &Secret::new("sk-first\nsk-second".to_owned()),
            )
            .unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "sk-first\nsk-second\n").unwrap();
        let path = file.path().to_string_lossy().into_owned();
        let environment = MapEnvironment::from([
            (API_KEY_ENV, "sk-first\u{1b}sk-second"),
            (API_KEY_FILE_ENV, path.as_str()),
            (TYPESAFE_API_KEY_ENV, "sk-first\rsk-second"),
        ]);
        let availability = official(&environment, &store).availability();
        for source in [
            CredentialSource::Environment,
            CredentialSource::EnvironmentFile,
            CredentialSource::TypesafeEnvironment,
            CredentialSource::OsKeychain,
        ] {
            let status = status_of(&availability, source);
            assert_eq!(status.set, Some(true), "{source:?}");
            assert_eq!(status.present, Some(false), "{source:?}");
        }
        assert!(!format!("{availability:?}").contains("sk-first"));
    }

    /// And a good source is both, while an unset one is neither.
    #[test]
    fn usable_is_true_only_for_a_source_that_would_supply_a_key() {
        let store = MemoryStore::new();
        let environment = MapEnvironment::from([(TYPESAFE_API_KEY_ENV, KEY)]);
        let availability = official(&environment, &store).availability();
        let good = status_of(&availability, CredentialSource::TypesafeEnvironment);
        assert_eq!((good.set, good.present), (Some(true), Some(true)));
        for source in [
            CredentialSource::Environment,
            CredentialSource::EnvironmentFile,
            CredentialSource::OsKeychain,
        ] {
            let status = status_of(&availability, source);
            assert_eq!((status.set, status.present), (Some(false), Some(false)));
        }
    }

    /// A store that cannot be consulted is neither present nor absent, usable nor not.
    #[test]
    fn an_unprobeable_store_reports_usable_as_unknown() {
        let store = UnavailableStore::new("no Secret Service");
        let environment = MapEnvironment::default();
        let availability = official(&environment, &store).availability();
        let keychain = status_of(&availability, CredentialSource::OsKeychain);
        assert_eq!((keychain.set, keychain.present), (None, None));
    }

    #[test]
    fn a_custom_endpoint_cannot_reach_a_typesafe_credential() {
        // The central T4 control, asserted rather than documented.
        let store = MemoryStore::new();
        store
            .set(OFFICIAL_ACCOUNT, &Secret::new(KEY.to_owned()))
            .unwrap();
        let environment = MapEnvironment::from([(API_KEY_ENV, KEY), (TYPESAFE_API_KEY_ENV, KEY)]);
        let credentials =
            Credentials::new(&environment, &store, false, "https://proxy.example.com");
        let error = credentials.resolve().unwrap_err();
        assert!(matches!(
            error,
            CredentialSourceError::NotFoundForCustomEndpoint { .. }
        ));
        assert!(!error.to_string().contains(KEY));
    }

    #[test]
    fn a_custom_endpoint_uses_its_own_namespace() {
        let store = MemoryStore::new();
        let environment = MapEnvironment::from([(CUSTOM_API_KEY_ENV, "custom-key")]);
        let credentials =
            Credentials::new(&environment, &store, false, "https://proxy.example.com");
        let resolved = credentials.resolve().unwrap();
        assert_eq!(resolved.source, CredentialSource::CustomEndpointEnvironment);
        assert_eq!(resolved.secret.expose(), "custom-key");
    }

    /// A broken store must say so, not be reported as "no key found, run auth login" --
    /// advice that writes to the very store that failed.
    #[test]
    fn an_unusable_store_is_named_rather_than_reported_as_a_plain_absence() {
        let environment = MapEnvironment::from([]);
        let store = UnavailableStore::new("the collection is locked".to_owned());
        let error = official(&environment, &store)
            .resolve()
            .expect_err("nothing is configured, so this must fail");

        match &error {
            CredentialSourceError::NotFoundAndStoreUnusable { reason } => {
                assert!(reason.contains("locked"), "lost the reason: {reason}");
            }
            other => panic!("expected NotFoundAndStoreUnusable, got {other:?}"),
        }
        let rendered = error.to_string();
        assert!(
            rendered.contains("could not be consulted"),
            "unhelpful message: {rendered}"
        );
        assert!(
            !rendered.contains("Run `jev auth login`"),
            "still tells the user to write to the store that failed: {rendered}"
        );
        // Still an absence, so `doctor` does not report it as a configuration error
        // when nothing is configured at all.
        assert!(error.is_absence());
    }

    #[test]
    fn availability_names_a_broken_source_instead_of_calling_it_absent() {
        // ADR-0008: "a source that is present but empty is an error naming that source,
        // not a skip". The resolver honoured that; the diagnostic did not, so
        // `jev auth status` reported a blank `JEV_API_KEY` as "not set" and sent the
        // user to `jev auth login` -- remediation for a problem they did not have.
        let environment = MapEnvironment::from([(API_KEY_ENV, "   ")]);
        let store = MemoryStore::new();
        let availability = official(&environment, &store).availability();

        assert_eq!(availability.effective, None);
        let error = availability
            .resolution_error
            .expect("a populated but unusable source must be reported");
        assert!(error.contains("empty"), "unhelpful message: {error}");
        assert!(
            !error.contains("   "),
            "the diagnostic echoed the value: {error}"
        );
    }

    #[test]
    fn an_unconfigured_environment_reports_no_error() {
        // The ordinary first run. Reporting "no API key found" as an *error* here would
        // make a fresh install look broken.
        let environment = MapEnvironment::from([]);
        let store = MemoryStore::new();
        let availability = official(&environment, &store).availability();
        assert_eq!(availability.effective, None);
        assert_eq!(availability.resolution_error, None);
    }

    #[test]
    fn availability_reports_shape_without_reading_values_into_output() {
        let store = MemoryStore::new();
        store
            .set(OFFICIAL_ACCOUNT, &Secret::new(KEY.to_owned()))
            .unwrap();
        let environment = MapEnvironment::from([(API_KEY_ENV, KEY)]);
        let availability = official(&environment, &store).availability();
        assert_eq!(availability.effective, Some(CredentialSource::Environment));
        assert_eq!(
            availability.is_present(CredentialSource::Environment),
            Some(true)
        );
        assert_eq!(
            availability.is_present(CredentialSource::TypesafeEnvironment),
            Some(false)
        );
        assert_eq!(
            availability.is_present(CredentialSource::OsKeychain),
            Some(true)
        );

        let rendered = format!("{availability:?}");
        assert!(
            !rendered.contains(KEY),
            "availability leaked a key: {rendered}"
        );
    }

    #[test]
    fn availability_records_an_unprobeable_keychain_without_failing() {
        let store = UnavailableStore::new("no Secret Service");
        let environment = MapEnvironment::default();
        let availability = official(&environment, &store).availability();
        assert_eq!(availability.is_present(CredentialSource::OsKeychain), None);
        assert!(availability.keychain_error.is_some());
        assert_eq!(availability.effective, None);
    }
}
