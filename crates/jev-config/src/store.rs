//! The OS-native credential store, behind a trait.
//!
//! # Why a trait
//!
//! Unit tests must not depend on a developer's real keychain: it would make results
//! machine-dependent, it would prompt for unlock on macOS, and it would leave entries
//! behind. [`SecretStore`] is therefore an interface with two implementations — the
//! real one and an in-memory double — and every code path above this module is tested
//! against the double.

use std::fmt;

use crate::secret::Secret;
use crate::source::CredentialSourceError;

/// The service name `jev` registers under in the OS credential store.
///
/// Stable: changing it would orphan every key users have already stored.
pub const SERVICE_NAME: &str = "jev-cli";

/// The account name used for the official TypeSafe endpoint.
///
/// Credentials for non-official endpoints are deliberately **not** storable here; see
/// [`crate::Credentials`] and `docs/threat-model.md` T4.
pub const OFFICIAL_ACCOUNT: &str = "api.typesafe.ai";

/// A place to keep a credential that is not a plaintext file.
pub trait SecretStore: fmt::Debug {
    /// Reads the stored credential, or `None` when there is none.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialSourceError::SecureStorageUnavailable`] when the platform
    /// has no usable store. "No entry" is `Ok(None)`, not an error.
    fn get(&self, account: &str) -> Result<Option<Secret>, CredentialSourceError>;

    /// Stores a credential, replacing any existing one.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialSourceError::SecureStorageUnavailable`] when the platform
    /// has no usable store. It must never fall back to writing a file.
    fn set(&self, account: &str, secret: &Secret) -> Result<(), CredentialSourceError>;

    /// Deletes the credential this CLI owns, if present.
    ///
    /// Returns `true` when an entry was removed. Deleting nothing is not an error, so
    /// that `jev auth logout` is idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialSourceError::SecureStorageUnavailable`] when the platform
    /// has no usable store.
    fn delete(&self, account: &str) -> Result<bool, CredentialSourceError>;

    /// A short description of the backing store, for `jev doctor`.
    fn describe(&self) -> String;
}

/// A store that reports itself unavailable for everything.
///
/// Used on builds without the `os-keychain` feature, and in tests that exercise the
/// no-secure-storage path. Every method fails the same way, which is exactly the
/// behaviour ADR-0002 requires: fail closed, never degrade to a file.
#[derive(Debug, Clone)]
pub struct UnavailableStore {
    reason: String,
}

impl UnavailableStore {
    /// Builds a store that reports `reason`.
    #[must_use]
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl Default for UnavailableStore {
    fn default() -> Self {
        Self::new("this build of jev was compiled without OS credential-store support")
    }
}

impl SecretStore for UnavailableStore {
    fn get(&self, _account: &str) -> Result<Option<Secret>, CredentialSourceError> {
        Err(self.error())
    }

    fn set(&self, _account: &str, _secret: &Secret) -> Result<(), CredentialSourceError> {
        Err(self.error())
    }

    fn delete(&self, _account: &str) -> Result<bool, CredentialSourceError> {
        Err(self.error())
    }

    fn describe(&self) -> String {
        format!("unavailable ({})", self.reason)
    }
}

impl UnavailableStore {
    fn error(&self) -> CredentialSourceError {
        CredentialSourceError::SecureStorageUnavailable {
            reason: self.reason.clone(),
        }
    }
}

/// An in-memory store, for tests.
///
/// Behaves like a working platform store without touching one. Lives outside
/// `#[cfg(test)]` so that integration tests and other crates can use it too.
#[derive(Debug, Default)]
pub struct MemoryStore {
    entries: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
}

impl MemoryStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, std::collections::BTreeMap<String, String>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<Secret>, CredentialSourceError> {
        Ok(self
            .lock()
            .get(account)
            .map(|value| Secret::new(value.clone())))
    }

    fn set(&self, account: &str, secret: &Secret) -> Result<(), CredentialSourceError> {
        self.lock()
            .insert(account.to_owned(), secret.expose().to_owned());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<bool, CredentialSourceError> {
        Ok(self.lock().remove(account).is_some())
    }

    fn describe(&self) -> String {
        "in-memory (test double)".to_owned()
    }
}

/// The platform credential store.
///
/// Built from `keyring-core` plus one target-gated store crate, rather than from the
/// `keyring` facade: the facade's `v1` feature pulls a zbus-based Secret Service client
/// and roughly two hundred transitive crates, which ADR-0004 would not accept. The
/// keyring project's own documentation recommends this arrangement for applications
/// that want to choose their stores.
#[cfg(feature = "os-keychain")]
#[derive(Debug)]
pub struct OsStore {
    store: std::sync::Arc<keyring_core::api::CredentialStore>,
}

#[cfg(feature = "os-keychain")]
impl OsStore {
    /// Connects to the platform store.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialSourceError::SecureStorageUnavailable`] when this platform
    /// has no store, or when the store cannot be reached — a headless Linux session
    /// with no running Secret Service, most often.
    pub fn new() -> Result<Self, CredentialSourceError> {
        let store = platform_backend()?;
        Ok(Self { store })
    }

    fn entry(&self, account: &str) -> Result<keyring_core::Entry, CredentialSourceError> {
        // On Windows the backend defaults to `Enterprise` persistence
        // (`windows-native-keyring-store` 1.1.0, `src/store.rs`: `.unwrap_or("Enterprise")`),
        // which is *roaming*: the credential is written to the user's roaming profile
        // and follows them to every other machine they sign in to on the domain. The
        // other two platforms store locally -- the macOS login keychain, without
        // `kSecAttrSynchronizable`, and the Secret Service login collection -- and
        // ADR-0002 and the threat model both describe OS-native storage as a *local*
        // secure store. Roaming was never a decision anyone recorded.
        //
        // `Local` is `CRED_PERSIST_LOCAL_MACHINE`, which matches what the other
        // platforms do. The modifier is Windows-only: the Apple and D-Bus stores reject
        // unknown modifiers in `parse_attributes`.
        //
        // This applies when a secret is written, so a user who already ran
        // `jev auth login` keeps the roaming credential until they log in again.
        #[cfg(windows)]
        let modifiers = Some(std::collections::HashMap::from([("persistence", "Local")]));
        #[cfg(not(windows))]
        let modifiers: Option<std::collections::HashMap<&str, &str>> = None;

        self.store
            .build(SERVICE_NAME, account, modifiers.as_ref())
            .map_err(|error| unavailable(&error))
    }
}

/// Selects the one store crate compiled for this target.
#[cfg(feature = "os-keychain")]
fn platform_backend()
-> Result<std::sync::Arc<keyring_core::api::CredentialStore>, CredentialSourceError> {
    #[cfg(target_vendor = "apple")]
    {
        apple_native_keyring_store::keychain::Store::new()
            .map(|store| store as std::sync::Arc<keyring_core::api::CredentialStore>)
            .map_err(|error| unavailable(&error))
    }
    #[cfg(target_os = "windows")]
    {
        windows_native_keyring_store::Store::new()
            .map(|store| store as std::sync::Arc<keyring_core::api::CredentialStore>)
            .map_err(|error| unavailable(&error))
    }
    #[cfg(all(unix, not(target_vendor = "apple"), not(target_os = "android")))]
    {
        dbus_secret_service_keyring_store::Store::new()
            .map(|store| store as std::sync::Arc<keyring_core::api::CredentialStore>)
            .map_err(|error| unavailable(&error))
    }
    #[cfg(not(any(
        target_vendor = "apple",
        target_os = "windows",
        all(unix, not(target_vendor = "apple"), not(target_os = "android"))
    )))]
    {
        Err(CredentialSourceError::SecureStorageUnavailable {
            reason: format!(
                "no OS credential store is available for {}",
                std::env::consts::OS
            ),
        })
    }
}

#[cfg(feature = "os-keychain")]
impl SecretStore for OsStore {
    fn get(&self, account: &str) -> Result<Option<Secret>, CredentialSourceError> {
        match self.entry(account)?.get_password() {
            Ok(password) => Ok(Some(Secret::new(password))),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(error) => Err(unavailable(&error)),
        }
    }

    fn set(&self, account: &str, secret: &Secret) -> Result<(), CredentialSourceError> {
        self.entry(account)?
            .set_password(secret.expose())
            .map_err(|error| unavailable(&error))
    }

    fn delete(&self, account: &str) -> Result<bool, CredentialSourceError> {
        match self.entry(account)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring_core::Error::NoEntry) => Ok(false),
            Err(error) => Err(unavailable(&error)),
        }
    }

    fn describe(&self) -> String {
        // Named by platform rather than probed further: `jev doctor --offline` promises
        // not to do anything surprising.
        match std::env::consts::OS {
            "macos" => "macOS Keychain".to_owned(),
            "windows" => "Windows Credential Manager".to_owned(),
            "linux" => "Secret Service (D-Bus)".to_owned(),
            other => format!("platform credential store ({other})"),
        }
    }
}

/// Maps a keyring failure to the project's error type.
///
/// **`Display` only, never `Debug`.** `keyring_core::Error::BadEncoding` and
/// `BadDataFormat` carry the raw stored bytes in their fields, which for this store are
/// the credential. Their `Display` implementations print a description and not the
/// bytes; a `{:?}` would print the credential. There is a test for this below.
#[cfg(feature = "os-keychain")]
fn unavailable(error: &keyring_core::Error) -> CredentialSourceError {
    // `Ambiguous` is not "the store is unavailable" -- the store answered, and said
    // there is more than one entry matching this service and account. On the Secret
    // Service that makes the credential both unreadable *and* unremovable, so
    // `jev auth logout` cannot clear it either, and "secure credential storage is
    // unavailable on this system" sends the user looking for a broken keyring instead
    // of a duplicate entry.
    let reason = if matches!(error, keyring_core::Error::Ambiguous(_)) {
        "more than one stored credential matches this service and account, so jev \
         cannot tell which to use. Remove the duplicates with your platform's \
         credential manager (`seahorse` or `secret-tool` on Linux, Keychain Access on \
         macOS, Credential Manager on Windows) and run `jev auth login` again"
            .to_owned()
    } else {
        error.to_string()
    };
    CredentialSourceError::SecureStorageUnavailable { reason }
}

/// A store that connects to the platform on first use, and not before.
///
/// # Why this exists
///
/// Opening the platform store is not free. On Linux it is a D-Bus connection; on macOS
/// it may prompt. Doing that eagerly meant `jev --version` and `jev --help` talked to
/// the credential service — measurably slow, and a side effect no user would expect
/// from those commands.
///
/// The connection is therefore made on the first `get`, `set`, or `delete`, which are
/// exactly the operations that need it, and a failure to connect is remembered rather
/// than retried on every call.
pub struct LazyStore {
    inner: std::sync::OnceLock<Box<dyn SecretStore + Send + Sync>>,
}

impl LazyStore {
    /// Creates a handle. Connects to nothing.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inner: std::sync::OnceLock::new(),
        }
    }

    fn get_or_connect(&self) -> &(dyn SecretStore + Send + Sync) {
        self.inner.get_or_init(connect).as_ref()
    }
}

impl Default for LazyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for LazyStore {
    /// Does not connect. A `Debug` that opened a D-Bus connection would be a trap.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LazyStore")
            .field("connected", &self.inner.get().is_some())
            .finish()
    }
}

impl SecretStore for LazyStore {
    fn get(&self, account: &str) -> Result<Option<Secret>, CredentialSourceError> {
        self.get_or_connect().get(account)
    }

    fn set(&self, account: &str, secret: &Secret) -> Result<(), CredentialSourceError> {
        self.get_or_connect().set(account, secret)
    }

    fn delete(&self, account: &str) -> Result<bool, CredentialSourceError> {
        self.get_or_connect().delete(account)
    }

    fn describe(&self) -> String {
        self.get_or_connect().describe()
    }
}

/// Environment variable that forces the unavailable store.
///
/// A **test affordance**, not a credential path. `jev`'s own integration tests spawn
/// the real binary, so they cannot inject a store the way the in-process tests do — and
/// without this they query the developer's live Secret Service or Keychain. That makes
/// two tests depend on whether the maintainer has ever run `jev auth login`, which
/// `AGENTS.md` §9 forbids, and on macOS it can raise an unlock prompt mid-suite.
///
/// Setting it can only make `jev` report secure storage as unavailable. It cannot
/// supply, redirect, or weaken a credential: the environment sources are unaffected and
/// there is still no plaintext fallback. It is deliberately absent from
/// `docs/cli-contract.md`'s environment table, which lists the variables that change
/// what `jev` does for a user.
const NO_KEYCHAIN_ENV: &str = "JEV_NO_KEYCHAIN";

/// The store this build uses, connected.
#[must_use]
fn connect() -> Box<dyn SecretStore + Send + Sync> {
    // A non-empty value, not merely "set". `JEV_NO_KEYCHAIN=` and `JEV_NO_KEYCHAIN=0`
    // read as "off" to anyone writing a shell script, and a variable that disables a
    // security-relevant subsystem should not fire on an empty assignment.
    let disabled = std::env::var(NO_KEYCHAIN_ENV)
        .is_ok_and(|value| !matches!(value.trim(), "" | "0" | "false" | "no"));
    if disabled {
        return Box::new(UnavailableStore::new(
            "disabled by JEV_NO_KEYCHAIN".to_owned(),
        ));
    }
    #[cfg(feature = "os-keychain")]
    {
        match OsStore::new() {
            Ok(store) => Box::new(store),
            // Failing to reach the platform store is not fatal here: the environment
            // sources may still supply a credential, and if they do not, the resolver's
            // error names the store among the places it looked.
            Err(CredentialSourceError::SecureStorageUnavailable { reason }) => {
                Box::new(UnavailableStore::new(reason))
            }
            Err(error) => Box::new(UnavailableStore::new(error.to_string())),
        }
    }
    #[cfg(not(feature = "os-keychain"))]
    {
        Box::new(UnavailableStore::default())
    }
}

/// The store this build uses, not yet connected.
///
/// Prefer this over connecting directly: most invocations of `jev` never need a
/// credential store at all.
#[must_use]
pub fn platform_store() -> Box<dyn SecretStore + Send + Sync> {
    Box::new(LazyStore::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real platform store, exercised rather than merely compiled.
    ///
    /// Every other test here uses `MemoryStore` or `UnavailableStore`, and the
    /// integration suite sets `JEV_NO_KEYCHAIN=1` for hermeticity — so after that
    /// change nothing on any platform actually called `apple-native-keyring-store`,
    /// `windows-native-keyring-store`, or `dbus-secret-service-keyring-store`. CI's
    /// four-OS matrix proved they linked, not that they worked: a store-crate bump that
    /// broke `build` or `get` at runtime would have shipped green.
    ///
    /// This asserts only the contract, never the content, so it is deterministic
    /// wherever it runs: on a machine with a working store it reaches the platform API
    /// and gets an answer, and on one without (a headless runner, a container, a build
    /// with the feature off) it gets the unavailable error. Both are passes. What it
    /// refuses to allow is a panic, or a `describe()` a user cannot act on.
    #[test]
    fn the_platform_store_answers_or_says_why_not() {
        let store = platform_store();

        let description = store.describe();
        assert!(
            !description.trim().is_empty(),
            "the store described itself as nothing, so `jev doctor` would show a blank"
        );

        match store.get(OFFICIAL_ACCOUNT) {
            // A working store: either it holds an entry or it does not.
            Ok(_) => {}
            // No store here, which is a supported configuration and must say so.
            Err(CredentialSourceError::SecureStorageUnavailable { reason }) => {
                assert!(
                    !reason.trim().is_empty(),
                    "the store was unavailable without saying why"
                );
            }
            Err(other) => {
                panic!("the platform store returned an error it is not allowed to return: {other}")
            }
        }
    }

    #[test]
    fn a_lazy_store_connects_to_nothing_until_it_is_used() {
        // `jev --version` must not open a D-Bus connection or prompt a keychain.
        let store = LazyStore::new();
        assert_eq!(format!("{store:?}"), "LazyStore { connected: false }");
    }

    #[test]
    fn the_memory_store_round_trips() {
        let store = MemoryStore::new();
        assert!(store.get(OFFICIAL_ACCOUNT).unwrap().is_none());
        store
            .set(OFFICIAL_ACCOUNT, &Secret::new("sk-test".to_owned()))
            .unwrap();
        assert_eq!(
            store
                .get(OFFICIAL_ACCOUNT)
                .unwrap()
                .map(|s| s.expose().to_owned()),
            Some("sk-test".to_owned())
        );
        assert!(store.delete(OFFICIAL_ACCOUNT).unwrap());
        assert!(!store.delete(OFFICIAL_ACCOUNT).unwrap());
    }

    #[test]
    fn an_unavailable_store_fails_every_operation_the_same_way() {
        // The invariant ADR-0002 rests on: no operation may quietly succeed by writing
        // somewhere else.
        let store = UnavailableStore::new("no Secret Service on this system");
        assert!(matches!(
            store.get("x"),
            Err(CredentialSourceError::SecureStorageUnavailable { .. })
        ));
        assert!(matches!(
            store.set("x", &Secret::new("k".to_owned())),
            Err(CredentialSourceError::SecureStorageUnavailable { .. })
        ));
        assert!(matches!(
            store.delete("x"),
            Err(CredentialSourceError::SecureStorageUnavailable { .. })
        ));
        assert!(store.describe().contains("unavailable"));
    }

    #[cfg(feature = "os-keychain")]
    #[test]
    fn keyring_errors_are_converted_through_display_not_debug() {
        // `keyring_core::Error::BadEncoding` carries the raw stored bytes, which for
        // this store are the credential. Converting through `Display` drops them;
        // converting through `Debug` would print them.
        let canary = b"sk-canary-in-keyring-bytes".to_vec();
        let error = keyring_core::Error::BadEncoding(canary.clone());
        let converted = unavailable(&error);
        let rendered = converted.to_string();
        assert!(
            !rendered.contains("sk-canary-in-keyring-bytes"),
            "credential bytes leaked through the keyring error: {rendered}"
        );
    }

    #[test]
    fn store_errors_never_contain_the_secret() {
        let store = UnavailableStore::new("reason");
        let error = store
            .set("x", &Secret::new("sk-canary-store".to_owned()))
            .unwrap_err();
        assert!(!error.to_string().contains("sk-canary-store"));
    }
}
