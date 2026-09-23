//! Configuration and credential handling.
//!
//! This is the **only** crate in the workspace permitted to read credential material.
//! Everything it exposes is designed so that a secret cannot reach a log line, an error
//! message, a panic payload, or a serialized structure by accident:
//!
//! * [`Secret`] redacts on `Debug` and `Display`, zeroizes on drop, and is neither
//!   `Clone` nor `Serialize`. Reading the plaintext requires the greppable
//!   [`Secret::expose`].
//! * No error variant in this crate carries credential material, and none carries a
//!   filesystem path the user did not supply.
//! * The configuration file cannot hold a credential: a key whose name looks like one
//!   is a load error, not a silently accepted setting.
//! * There is no plaintext-file fallback. When secure storage is unavailable, `jev`
//!   says so and names the environment variables to use instead.
//!
//! Both the environment ([`Environment`]) and the credential store ([`SecretStore`])
//! are traits with in-memory doubles, so every precedence and failure path is tested
//! without touching a developer's real keychain or mutating the process environment.
//!
//! See `docs/adr/0002-security-and-credentials.md`,
//! `docs/adr/0008-credential-precedence-and-endpoint-isolation.md`, and
//! `docs/threat-model.md`.

mod credentials;
mod env;
mod secret;
mod settings;
mod store;

pub use credentials::{
    CredentialAvailability, Credentials, KeyDefect, MAX_KEY_FILE_BYTES, ResolvedCredential,
    SourceStatus,
};
pub use env::{Environment, MapEnvironment, SystemEnvironment};
pub use secret::Secret;
pub use settings::{
    CONFIG_DIR_ENV, CONFIG_FILE_NAME, SECRET_LIKE, SETTING_NAMES, Settings, SettingsError,
    config_dir, config_path, looks_like_a_secret_name,
};
pub use source::{
    API_KEY_ENV, API_KEY_FILE_ENV, CUSTOM_API_KEY_ENV, CUSTOM_API_KEY_FILE_ENV, CredentialSource,
    CredentialSourceError, TYPESAFE_API_KEY_ENV,
};
pub use store::{
    LazyStore, MemoryStore, OFFICIAL_ACCOUNT, SERVICE_NAME, SecretStore, UnavailableStore,
    platform_store,
};

#[cfg(feature = "os-keychain")]
pub use store::OsStore;

mod source;
