//! The non-secret configuration file.
//!
//! # What may live here
//!
//! Preferences only: default model, default output format, timeouts, retry count.
//! **Never a credential.** That is not a convention, it is enforced: any key whose name
//! looks like a secret is rejected with an error, so the file cannot quietly become a
//! plaintext credential store the way it does in most CLIs
//! (`docs/threat-model.md` T3).
//!
//! # Where it lives
//!
//! One file, in the user's configuration directory, never in the working directory. A
//! config file discovered next to a cloned repository could change where credentials
//! go or which endpoint is used; `jev` therefore looks in exactly one place, and
//! `jev doctor` prints which.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::env::Environment;

/// Environment variable overriding the configuration directory.
///
/// Documented and honoured so that a user can keep `jev`'s state somewhere else, and so
/// that tests never touch a developer's real configuration.
pub const CONFIG_DIR_ENV: &str = "JEV_CONFIG_DIR";

/// File name inside the configuration directory.
pub const CONFIG_FILE_NAME: &str = "config.toml";

/// Largest configuration file `jev` will read.
const MAX_CONFIG_BYTES: u64 = 64 * 1024;

/// Name fragments that indicate someone is trying to store a secret.
///
/// Matched case-insensitively against every key in the configuration file, and — via
/// [`looks_like_a_secret_name`] — against every command-line argument name by a test in
/// `jev-cli`. One list, so a new `--bearer` or `--credential-file` flag cannot pass one
/// guard and fail the other.
///
/// A false positive here costs a user one rename; a false negative costs them a
/// plaintext credential.
pub const SECRET_LIKE: &[&str] = &[
    "key",
    "secret",
    "token",
    "password",
    "passwd",
    "credential",
    "auth",
    "bearer",
];

/// Whether a setting or argument name looks like it would hold a credential.
#[must_use]
pub fn looks_like_a_secret_name(name: &str) -> bool {
    let lowered = name.to_ascii_lowercase();
    SECRET_LIKE.iter().any(|needle| lowered.contains(needle))
}

/// Reasons the configuration file could not be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SettingsError {
    /// The file exists but is not valid TOML.
    #[error("{path}: not valid TOML: {reason}")]
    Malformed {
        /// The file, which the user owns and named.
        path: String,
        /// The parser's complaint.
        reason: String,
    },
    /// The file contains a key that looks like a credential.
    #[error(
        "{path}: the key `{key}` looks like a credential\n\
         \n\
         `jev` never reads a secret from a configuration file. Remove the key and use \
         `jev auth login`, or set JEV_API_KEY."
    )]
    SecretLikeKey {
        /// The file.
        path: String,
        /// The offending key.
        key: String,
    },
    /// The file contains a key `jev` does not know.
    ///
    /// Rejected rather than ignored: a typo in `model` that is silently discarded means
    /// the user quietly gets a different model than they configured.
    #[error("{path}: unknown setting `{key}`\n\nRun `jev config list` to see valid settings.")]
    UnknownKey {
        /// The file.
        path: String,
        /// The offending key.
        key: String,
    },
    /// A value was out of range or the wrong type.
    #[error("{path}: `{key}`: {reason}")]
    InvalidValue {
        /// The file.
        path: String,
        /// The setting.
        key: String,
        /// What is wrong with it.
        reason: String,
    },
    /// The file could not be read or written.
    #[error("{path}: {reason}")]
    Io {
        /// The file.
        path: String,
        /// The I/O failure, by kind.
        reason: String,
    },
    /// The file was larger than the configuration-file size bound.
    #[error("{path}: larger than {MAX_CONFIG_BYTES} bytes; that is not a configuration file")]
    TooLarge {
        /// The file.
        path: String,
    },
    /// The path exists but is not a regular file.
    #[error(
        "{path}: not a regular file; `jev` will not read a directory, a device, or a pipe here"
    )]
    NotARegularFile {
        /// The path.
        path: String,
    },
    /// No configuration directory could be determined.
    #[error(
        "could not determine a configuration directory; set ${CONFIG_DIR_ENV} to a path \
         `jev` may use"
    )]
    NoConfigDir,
}

/// Every setting `jev` understands, and nothing else.
///
/// `deny_unknown_fields` is load-bearing: it turns a typo into an error the user sees
/// rather than a preference that silently does not apply.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Inference protocol: typesafe, cloudflare, ollama, llamacpp, or huggingface.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Nonsecret Cloudflare account identifier, overridden by the flag or environment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloudflare_account_id: Option<String>,
    /// Default model identifier. Overridden by `--model`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Default output format, `text` or `json`. Overridden by `--output`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Colour policy: `auto`, `always`, or `never`. Overridden by `--color` and by
    /// `NO_COLOR`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Per-attempt HTTP timeout, in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u64>,
    /// Retries after the first attempt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retries: Option<u32>,
    /// Ceiling on bytes read from one input source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_input_bytes: Option<u64>,
    /// API base URL.
    ///
    /// Present so that a self-hosted proxy can be configured once rather than exported
    /// on every call. It is **not** a way to make an override invisible: a non-official
    /// endpoint is reported by `jev doctor`, warns on every use, and uses a separate
    /// credential namespace. See `docs/threat-model.md` T4.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
}

/// The names a user may pass to `jev config get|set|unset`.
pub const SETTING_NAMES: &[&str] = &[
    "cloudflare_account_id",
    "provider",
    "color",
    "endpoint",
    "max_input_bytes",
    "model",
    "output",
    "retries",
    "timeout_seconds",
];

impl Settings {
    /// Loads settings from `path`, returning defaults when the file does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError`] for an unreadable, oversized, malformed, or
    /// secret-bearing file.
    pub fn load(path: &Path) -> Result<Self, SettingsError> {
        use std::io::Read as _;

        let display = path.display().to_string();
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => {
                return Err(SettingsError::Io {
                    path: display,
                    reason: error.kind().to_string(),
                });
            }
        };

        // Checked *before* opening. `File::open` on a FIFO with no writer blocks
        // forever, which in CI is an unbounded stall with no diagnostic, and a
        // character device would defeat the size bound entirely.
        if !metadata.is_file() {
            return Err(SettingsError::NotARegularFile { path: display });
        }
        // `metadata().len()` is a hint, not a bound: it is zero for anything that is
        // not a regular file, and a regular file can grow between the stat and the
        // read. The `take` below is the actual bound. This early check only produces a
        // better message for the common case.
        if metadata.len() > MAX_CONFIG_BYTES {
            return Err(SettingsError::TooLarge { path: display });
        }

        let io = |error: std::io::Error| SettingsError::Io {
            path: display.clone(),
            reason: error.kind().to_string(),
        };
        let file = std::fs::File::open(path).map_err(io)?;
        let mut text = String::new();
        let read = file
            .take(MAX_CONFIG_BYTES + 1)
            .read_to_string(&mut text)
            .map_err(io)?;
        if read as u64 > MAX_CONFIG_BYTES {
            return Err(SettingsError::TooLarge { path: display });
        }
        Self::parse(&text, &display)
    }

    /// Parses settings from TOML text.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError`] for malformed TOML, a secret-like key, an unknown key,
    /// or an out-of-range value.
    pub fn parse(text: &str, path: &str) -> Result<Self, SettingsError> {
        // The secret-key check runs on the raw document, before typed parsing, so that
        // it fires even for a key that `deny_unknown_fields` would also reject — the
        // user gets the security-relevant message, not the generic one.
        let document: toml::Table =
            toml::from_str(text).map_err(|error| SettingsError::Malformed {
                path: path.to_owned(),
                reason: error.message().to_owned(),
            })?;
        reject_secret_like_keys(&document, path)?;

        let settings: Self = document
            .clone()
            .try_into()
            .map_err(|error: toml::de::Error| match unknown_key(&document) {
                Some(key) => SettingsError::UnknownKey {
                    path: path.to_owned(),
                    key,
                },
                None => SettingsError::Malformed {
                    path: path.to_owned(),
                    reason: error.message().to_owned(),
                },
            })?;
        settings.validate(path)?;
        Ok(settings)
    }

    /// Checks value ranges that the type system does not.
    fn validate(&self, path: &str) -> Result<(), SettingsError> {
        let invalid = |key: &str, reason: &str| SettingsError::InvalidValue {
            path: path.to_owned(),
            key: key.to_owned(),
            reason: reason.to_owned(),
        };
        if let Some(provider) = &self.provider
            && !matches!(
                provider.as_str(),
                "typesafe" | "cloudflare" | "ollama" | "llamacpp" | "llama-cpp" | "huggingface"
            )
        {
            return Err(invalid(
                "provider",
                "expected `typesafe`, `cloudflare`, `ollama`, `llamacpp` (alias `llama-cpp`), or `huggingface`",
            ));
        }
        if let Some(account) = &self.cloudflare_account_id
            && (account.len() != 32 || !account.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(invalid(
                "cloudflare_account_id",
                "expected exactly 32 hexadecimal characters",
            ));
        }
        if let Some(output) = &self.output
            && !matches!(output.as_str(), "text" | "json")
        {
            return Err(invalid("output", "expected `text` or `json`"));
        }
        if let Some(color) = &self.color
            && !matches!(color.as_str(), "auto" | "always" | "never")
        {
            return Err(invalid("color", "expected `auto`, `always`, or `never`"));
        }
        if let Some(endpoint) = &self.endpoint
            && let Some(reason) = reject_secret_bearing_value("endpoint", endpoint)
        {
            return Err(invalid("endpoint", reason));
        }
        if self.timeout_seconds == Some(0) {
            return Err(invalid("timeout_seconds", "must be at least 1"));
        }
        if self.max_input_bytes == Some(0) {
            return Err(invalid("max_input_bytes", "must be at least 1"));
        }
        Ok(())
    }

    /// Writes settings to `path`, creating the directory if needed.
    ///
    /// The file is written atomically — to a temporary file in the same directory, then
    /// renamed — so a crash cannot leave a half-written configuration. On Unix the
    /// directory is created `0700` and the file `0600`: the file holds no secret, but
    /// it does hold an endpoint, and a world-writable one would let another local user
    /// redirect this user's requests.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError::Io`] when the write fails.
    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        let display = path.display().to_string();
        let io = |error: std::io::Error| SettingsError::Io {
            path: display.clone(),
            reason: error.kind().to_string(),
        };

        if let Some(parent) = path.parent() {
            create_private_dir(parent).map_err(io)?;
        }
        let rendered = toml::to_string_pretty(self).map_err(|error| SettingsError::Malformed {
            path: display.clone(),
            reason: error.to_string(),
        })?;
        let body = format!("{}{rendered}", header());

        // Same directory, so the rename is atomic and stays on one filesystem. The
        // name carries the process id so two concurrent writers do not collide, and
        // `write_private_file` uses `create_new`, so a planted file or symlink at this
        // path is an error rather than a target.
        let temporary = path.with_extension(format!("toml.tmp.{}", std::process::id()));
        write_private_file(&temporary, body.as_bytes()).map_err(io)?;
        std::fs::rename(&temporary, path).map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            io(error)
        })
    }

    /// Reads one setting by name.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<String> {
        match key {
            "provider" => self.provider.clone(),
            "cloudflare_account_id" => self.cloudflare_account_id.clone(),
            "model" => self.model.clone(),
            "output" => self.output.clone(),
            "color" => self.color.clone(),
            "endpoint" => self.endpoint.clone(),
            "timeout_seconds" => self.timeout_seconds.map(|value| value.to_string()),
            "retries" => self.retries.map(|value| value.to_string()),
            "max_input_bytes" => self.max_input_bytes.map(|value| value.to_string()),
            _ => None,
        }
    }

    /// Sets one setting by name, validating the value.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError::UnknownKey`] for an unrecognized name,
    /// [`SettingsError::SecretLikeKey`] for a name that looks like a credential, and
    /// [`SettingsError::InvalidValue`] for a value the setting cannot take.
    pub fn set(&mut self, key: &str, value: &str, path: &str) -> Result<(), SettingsError> {
        if is_secret_like(key) {
            return Err(SettingsError::SecretLikeKey {
                path: path.to_owned(),
                key: key.to_owned(),
            });
        }
        let invalid = |reason: &str| SettingsError::InvalidValue {
            path: path.to_owned(),
            key: key.to_owned(),
            reason: reason.to_owned(),
        };
        if let Some(reason) = reject_secret_bearing_value(key, value) {
            return Err(invalid(reason));
        }
        // Applied to a copy and validated there, so a rejected value never leaves the
        // in-memory settings half-updated — which would then be written to disk by the
        // next successful `set`.
        let mut candidate = self.clone();
        match key {
            "provider" => candidate.provider = Some(value.to_owned()),
            "cloudflare_account_id" => candidate.cloudflare_account_id = Some(value.to_owned()),
            "model" => candidate.model = Some(value.to_owned()),
            "output" => candidate.output = Some(value.to_owned()),
            "color" => candidate.color = Some(value.to_owned()),
            "endpoint" => candidate.endpoint = Some(value.to_owned()),
            "timeout_seconds" => {
                candidate.timeout_seconds = Some(
                    value
                        .parse()
                        .map_err(|_| invalid("expected a whole number of seconds"))?,
                );
            }
            "retries" => {
                candidate.retries = Some(
                    value
                        .parse()
                        .map_err(|_| invalid("expected a whole number"))?,
                );
            }
            "max_input_bytes" => {
                candidate.max_input_bytes = Some(
                    value
                        .parse()
                        .map_err(|_| invalid("expected a whole number of bytes"))?,
                );
            }
            _ => {
                return Err(SettingsError::UnknownKey {
                    path: path.to_owned(),
                    key: key.to_owned(),
                });
            }
        }
        candidate.validate(path)?;
        *self = candidate;
        Ok(())
    }

    /// Clears one setting by name.
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError::UnknownKey`] for an unrecognized name.
    pub fn unset(&mut self, key: &str, path: &str) -> Result<(), SettingsError> {
        match key {
            "provider" => self.provider = None,
            "cloudflare_account_id" => self.cloudflare_account_id = None,
            "model" => self.model = None,
            "output" => self.output = None,
            "color" => self.color = None,
            "endpoint" => self.endpoint = None,
            "timeout_seconds" => self.timeout_seconds = None,
            "retries" => self.retries = None,
            "max_input_bytes" => self.max_input_bytes = None,
            _ => {
                return Err(SettingsError::UnknownKey {
                    path: path.to_owned(),
                    key: key.to_owned(),
                });
            }
        }
        Ok(())
    }
}

fn header() -> String {
    "# jev configuration. Non-secret settings only.\n\
     #\n\
     # `jev` will refuse to load this file if it contains a key that looks like a\n\
     # credential. Store your API key with `jev auth login`, or in JEV_API_KEY.\n\
     #\n\
     # Managed by `jev config set`; hand edits are fine.\n\n"
        .to_owned()
}

fn is_secret_like(key: &str) -> bool {
    looks_like_a_secret_name(key)
}

/// Rejects a *value* that carries a credential, as opposed to a key that names one.
///
/// The only setting that can plausibly embed one is `endpoint`, where
/// `https://user:sk-key@host` is both a credential written to disk in cleartext and a
/// URL `jev` refuses to use anyway.
fn reject_secret_bearing_value(key: &str, value: &str) -> Option<&'static str> {
    if key == "endpoint" && value.contains('@') {
        return Some(
            "an endpoint must not contain userinfo; `jev` will not send a credential \
             embedded in a URL, and storing one here would write it to disk in cleartext",
        );
    }
    None
}

fn reject_secret_like_keys(table: &toml::Table, path: &str) -> Result<(), SettingsError> {
    // Walks nested tables too: `[auth] key = "..."` is the same hazard as a top-level
    // key, and a check that only looked at the top level would be trivially bypassed.
    let mut pending: Vec<(String, &toml::Table)> = vec![(String::new(), table)];
    while let Some((prefix, current)) = pending.pop() {
        for (key, value) in current {
            let qualified = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            if is_secret_like(key) {
                return Err(SettingsError::SecretLikeKey {
                    path: path.to_owned(),
                    key: qualified,
                });
            }
            if let toml::Value::Table(nested) = value {
                pending.push((qualified, nested));
            }
        }
    }
    Ok(())
}

/// Names the first key in the document that is not a known setting.
fn unknown_key(table: &toml::Table) -> Option<String> {
    let known: BTreeSet<&str> = SETTING_NAMES.iter().copied().collect();
    table
        .keys()
        .find(|key| !known.contains(key.as_str()))
        .cloned()
}

/// Determines the configuration directory.
///
/// `JEV_CONFIG_DIR` wins. Otherwise the platform convention: `XDG_CONFIG_HOME` or
/// `~/.config` on Linux and other Unixes, `~/Library/Application Support` on macOS,
/// `%APPDATA%` on Windows.
///
/// # Errors
///
/// Returns [`SettingsError::NoConfigDir`] when no directory can be determined, rather
/// than guessing at a path.
pub fn config_dir(environment: &dyn Environment) -> Result<PathBuf, SettingsError> {
    if let Some(explicit) = environment.var(CONFIG_DIR_ENV) {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    // Every branch filters an empty value. Without it, `APPDATA=""` produced the
    // *relative* path `jev\config.toml` -- resolved against the current working
    // directory, which is exactly what this module's header and the test
    // `nothing_in_the_working_directory_is_ever_consulted` promise can never happen.
    // `HOME=""` did the same on macOS and Unix. An empty `APPDATA` is real in service
    // and scheduled-task contexts, and `env -i` does it on Unix.
    let set = |name: &str| {
        environment
            .var(name)
            .filter(|value| !value.trim().is_empty())
    };

    let base = if cfg!(target_os = "windows") {
        set("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        set("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        set("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| set("HOME").map(|home| PathBuf::from(home).join(".config")))
    };

    let base = base.ok_or(SettingsError::NoConfigDir)?;
    // A backstop for anything the filters above cannot anticipate -- a variable holding
    // a relative path, say. Refusing is right: a guessed path in the working directory
    // is worse than an actionable error.
    if !base.is_absolute() {
        return Err(SettingsError::NoConfigDir);
    }
    Ok(base.join("jev"))
}

/// The configuration file path.
///
/// # Errors
///
/// Returns [`SettingsError::NoConfigDir`] when the directory cannot be determined.
pub fn config_path(environment: &dyn Environment) -> Result<PathBuf, SettingsError> {
    config_dir(environment).map(|dir| dir.join(CONFIG_FILE_NAME))
}

/// Creates a directory that only the owner can enter, and tightens one that exists.
///
/// **On Unix.** The `cfg(not(unix))` arm creates the directory and sets no ACL, so on
/// Windows the protection is whatever the parent grants — user-only for the default
/// `%APPDATA%\Roaming`, and nothing in particular for a `JEV_CONFIG_DIR` the user
/// points somewhere shared. Nor is a pre-existing loose directory tightened there, as
/// it is on Unix. See `docs/threat-model.md` T8.
///
/// Re-asserting the mode matters: a pre-existing group- or world-writable directory —
/// a shared CI workspace, a container volume, a `JEV_CONFIG_DIR` pointed somewhere
/// careless — is what lets another local user plant a file at the path `save` is about
/// to write.
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};

        if path.is_dir() {
            let mode = std::fs::metadata(path)?.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
            }
            return Ok(());
        }
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
    }
    #[cfg(not(unix))]
    {
        if path.is_dir() {
            return Ok(());
        }
        std::fs::create_dir_all(path)
    }
}

/// Writes a new file that only the owner can read.
///
/// **On Unix**, via `OpenOptions::mode(0o600)`. On Windows no ACL is set and the file
/// inherits from its directory; see [`create_private_dir`].
///
/// `create_new` rather than `create().truncate()`, deliberately. The latter follows a
/// symlink planted at this path and truncates whatever it points at — and because
/// `mode` is ignored for a file that already exists, it would not even be private.
/// With `create_new` a planted path is `AlreadyExists`, which the caller reports.
fn write_private_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;

    // A leftover from a killed run would otherwise make every later save fail. Removing
    // it is safe: the name carries this process's id, so nothing else owns it.
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    // Durability matters here: an atomic rename over an unflushed file can leave an
    // empty configuration after a crash.
    file.sync_all()
}

impl fmt::Display for Settings {
    /// Renders the settings that are set, one per line, `key = value`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for name in SETTING_NAMES {
            if let Some(value) = self.get(name) {
                writeln!(f, "{name} = {value}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn provider_and_account_settings_round_trip_and_refuse_invalid_values() {
        let mut settings = Settings::default();
        settings.set("provider", "cloudflare", "config").unwrap();
        settings
            .set(
                "cloudflare_account_id",
                "0123456789abcdef0123456789abcdef",
                "config",
            )
            .unwrap();
        assert_eq!(settings.get("provider").as_deref(), Some("cloudflare"));
        assert!(settings.set("provider", "unknown", "config").is_err());
        assert!(
            settings
                .set("cloudflare_account_id", "../escape", "config")
                .is_err()
        );
        assert_eq!(settings.get("provider").as_deref(), Some("cloudflare"));
        settings.unset("provider", "config").unwrap();
        assert!(settings.get("provider").is_none());
    }
    use super::*;
    use crate::env::MapEnvironment;

    const PATH: &str = "/tmp/jev-test/config.toml";

    #[test]
    fn a_valid_file_parses() {
        let settings = Settings::parse(
            r#"
            model = "jev-1.13.0"
            output = "json"
            retries = 5
            "#,
            PATH,
        )
        .unwrap();
        assert_eq!(settings.model.as_deref(), Some("jev-1.13.0"));
        assert_eq!(settings.output.as_deref(), Some("json"));
        assert_eq!(settings.retries, Some(5));
    }

    #[test]
    fn a_missing_file_is_defaults_not_an_error() {
        let settings = Settings::load(Path::new("/nonexistent/jev/config.toml")).unwrap();
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn a_credential_shaped_key_is_refused() {
        // The security property this module exists for: the config file cannot silently
        // become a credential store.
        for key in [
            "api_key",
            "apiKey",
            "KEY",
            "token",
            "secret",
            "password",
            "auth_token",
            "bearer",
            "credentials",
        ] {
            let text = format!("{key} = \"sk-whatever\"");
            let error = Settings::parse(&text, PATH).unwrap_err();
            assert!(
                matches!(error, SettingsError::SecretLikeKey { .. }),
                "{key} was not refused: {error}"
            );
        }
    }

    #[test]
    fn a_credential_hidden_in_a_nested_table_is_also_refused() {
        let error = Settings::parse("[auth]\nvalue = \"x\"", PATH).unwrap_err();
        assert!(matches!(error, SettingsError::SecretLikeKey { .. }));
        let error = Settings::parse("[profile]\napi_key = \"x\"", PATH).unwrap_err();
        match error {
            SettingsError::SecretLikeKey { key, .. } => assert_eq!(key, "profile.api_key"),
            other => panic!("unexpected: {other}"),
        }
    }

    #[test]
    fn the_refusal_message_does_not_echo_the_value() {
        let error = Settings::parse("api_key = \"sk-canary-in-config\"", PATH).unwrap_err();
        assert!(!error.to_string().contains("sk-canary-in-config"));
    }

    #[test]
    fn an_unknown_key_is_an_error_not_a_silent_no_op() {
        // A typo'd `modle = "..."` that is ignored means the user gets a model they did
        // not choose and never finds out.
        let error = Settings::parse("modle = \"jev-latest\"", PATH).unwrap_err();
        match error {
            SettingsError::UnknownKey { key, .. } => assert_eq!(key, "modle"),
            other => panic!("unexpected: {other}"),
        }
    }

    #[test]
    fn invalid_values_are_rejected_with_the_allowed_set() {
        let error = Settings::parse("output = \"yaml\"", PATH).unwrap_err();
        assert!(error.to_string().contains("`text` or `json`"));
        assert!(Settings::parse("timeout_seconds = 0", PATH).is_err());
        assert!(Settings::parse("color = \"rainbow\"", PATH).is_err());
    }

    #[test]
    fn malformed_toml_is_reported_not_ignored() {
        assert!(matches!(
            Settings::parse("this is not toml", PATH),
            Err(SettingsError::Malformed { .. })
        ));
    }

    #[test]
    fn set_get_and_unset_round_trip() {
        let mut settings = Settings::default();
        settings.set("model", "jev-1.13.0", PATH).unwrap();
        assert_eq!(settings.get("model").as_deref(), Some("jev-1.13.0"));
        settings.unset("model", PATH).unwrap();
        assert_eq!(settings.get("model"), None);
    }

    #[test]
    fn an_endpoint_carrying_userinfo_is_refused_before_it_reaches_disk() {
        // `https://user:sk-key@host` would be a credential written to the config file
        // in cleartext, which is exactly what this file must never hold.
        let mut settings = Settings::default();
        let error = settings
            .set("endpoint", "https://user:sk-secret@example.com", PATH)
            .unwrap_err();
        assert!(error.to_string().contains("userinfo"), "{error}");
        assert!(!error.to_string().contains("sk-secret"));
        assert!(Settings::parse("endpoint = \"https://u:sk-secret@example.com\"", PATH).is_err());
    }

    #[test]
    fn a_non_regular_configuration_file_is_refused_rather_than_read() {
        // A FIFO would block `File::open` forever, and `metadata().len()` is zero for
        // one, so the size bound would not apply to it either.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILE_NAME);
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(
            Settings::load(&path),
            Err(SettingsError::NotARegularFile { .. })
        ));
    }

    #[test]
    fn a_planted_file_at_the_temporary_path_is_refused_rather_than_followed() {
        // `create(true).truncate(true)` would follow a symlink planted here, overwrite
        // whatever it points at, and then leave the config file *as* that symlink.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILE_NAME);
        let temporary = path.with_extension(format!("toml.tmp.{}", std::process::id()));
        let victim = dir.path().join("victim");
        std::fs::write(&victim, "ORIGINAL").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&victim, &temporary).unwrap();
        #[cfg(not(unix))]
        std::fs::write(&temporary, "leftover").unwrap();

        Settings::default()
            .save(&path)
            .expect("the save should succeed");

        assert_eq!(
            std::fs::read_to_string(&victim).unwrap(),
            "ORIGINAL",
            "the planted symlink was followed and its target overwritten"
        );
        assert!(
            !std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the configuration file is now a symlink"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_world_writable_config_directory_is_tightened() {
        use std::os::unix::fs::PermissionsExt as _;

        // A shared CI workspace or a careless JEV_CONFIG_DIR is what lets another local
        // user plant a file at the path `save` is about to write.
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("loose");
        std::fs::create_dir(&nested).unwrap();
        std::fs::set_permissions(&nested, std::fs::Permissions::from_mode(0o777)).unwrap();

        Settings::default()
            .save(&nested.join(CONFIG_FILE_NAME))
            .unwrap();

        let mode = std::fs::metadata(&nested).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "a pre-existing loose directory was left loose");
    }

    #[test]
    fn set_refuses_a_secret_shaped_key() {
        let mut settings = Settings::default();
        assert!(matches!(
            settings.set("api_key", "sk-x", PATH),
            Err(SettingsError::SecretLikeKey { .. })
        ));
    }

    #[test]
    fn set_validates_the_value() {
        let mut settings = Settings::default();
        assert!(settings.set("output", "yaml", PATH).is_err());
        assert!(settings.set("retries", "many", PATH).is_err());
        assert!(settings.set("retries", "3", PATH).is_ok());
    }

    #[test]
    fn saving_then_loading_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join(CONFIG_FILE_NAME);
        let mut settings = Settings::default();
        settings.set("model", "jev-1.13.0", PATH).unwrap();
        settings.set("retries", "4", PATH).unwrap();
        settings.save(&path).unwrap();

        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded, settings);
        // The written file must itself pass the secret-key check on reload.
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("jev-1.13.0")
        );
    }

    #[cfg(unix)]
    #[test]
    fn saved_files_and_directories_are_private() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join(CONFIG_FILE_NAME);
        Settings::default().save(&path).unwrap();

        let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600, "config file is not owner-only");
        let dir_mode = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700, "config directory is not owner-only");
    }

    #[test]
    fn saving_leaves_no_temporary_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILE_NAME);
        Settings::default().save(&path).unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| Path::new(name).extension().is_some_and(|ext| ext == "tmp"))
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    }

    #[test]
    fn an_oversized_config_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILE_NAME);
        std::fs::write(
            &path,
            "x".repeat(usize::try_from(MAX_CONFIG_BYTES + 1).unwrap()),
        )
        .unwrap();
        assert!(matches!(
            Settings::load(&path),
            Err(SettingsError::TooLarge { .. })
        ));
    }

    /// The other side of the boundary. `metadata.len()` and a `take`-bounded read are
    /// two separate checks against the same constant, and a file at exactly the limit
    /// is the input that tells them apart.
    #[test]
    fn a_config_file_of_exactly_the_maximum_size_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_FILE_NAME);
        let limit = usize::try_from(MAX_CONFIG_BYTES).unwrap();
        // Valid TOML padded to exactly the limit with a comment.
        let head = "output = \"json\"\n# ";
        let padding = limit - head.len() - 1;
        std::fs::write(&path, format!("{head}{}\n", "p".repeat(padding))).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            MAX_CONFIG_BYTES,
            "the fixture is not exactly at the limit"
        );

        let settings = Settings::load(&path).expect("a file at exactly the limit loads");
        assert_eq!(settings.output.as_deref(), Some("json"));
    }

    #[test]
    fn the_config_dir_env_var_wins() {
        let environment = MapEnvironment::from([
            (CONFIG_DIR_ENV, "/custom/place"),
            ("HOME", "/home/user"),
            ("XDG_CONFIG_HOME", "/home/user/.config"),
        ]);
        assert_eq!(
            config_dir(&environment).unwrap(),
            PathBuf::from("/custom/place")
        );
    }

    #[test]
    fn with_no_home_at_all_the_error_is_actionable() {
        let environment = MapEnvironment::default();
        let error = config_dir(&environment).unwrap_err();
        assert!(error.to_string().contains(CONFIG_DIR_ENV));
    }

    #[test]
    fn an_empty_platform_variable_is_refused_rather_than_made_relative() {
        // The same invariant as the test below, for the case it did not cover: only
        // `XDG_CONFIG_HOME` filtered an empty value, so `APPDATA=""` gave the relative
        // `jev\config.toml` and `HOME=""` gave `.config/jev` or
        // `Library/Application Support/jev` -- all resolved against the working
        // directory. An empty `APPDATA` is real in service and scheduled-task contexts.
        let variable = if cfg!(target_os = "windows") {
            "APPDATA"
        } else {
            "HOME"
        };
        for blank in ["", "   "] {
            let environment = MapEnvironment::from([(variable, blank)]);
            assert!(
                matches!(config_dir(&environment), Err(SettingsError::NoConfigDir)),
                "{variable}={blank:?} did not produce NoConfigDir"
            );
        }
    }

    #[test]
    fn a_relative_platform_variable_is_refused() {
        // The backstop: a variable holding a relative path would otherwise put the
        // configuration under the working directory just as an empty one did.
        let variable = if cfg!(target_os = "windows") {
            "APPDATA"
        } else {
            "HOME"
        };
        let environment = MapEnvironment::from([(variable, "relative/path")]);
        assert!(matches!(
            config_dir(&environment),
            Err(SettingsError::NoConfigDir)
        ));
    }

    #[test]
    fn nothing_in_the_working_directory_is_ever_consulted() {
        // A config file in a cloned repository must not change `jev`'s behaviour.
        //
        // Each platform roots this somewhere different -- `APPDATA` on Windows,
        // `~/Library/Application Support` on macOS, `XDG_CONFIG_HOME` or `~/.config`
        // elsewhere -- so the fixture has to set the variable the platform actually
        // reads. Setting only `HOME` asserted a Unix assumption, and on Windows
        // `config_path` correctly returned `NoConfigDir` and the `unwrap` panicked.
        let (variable, root) = if cfg!(target_os = "windows") {
            ("APPDATA", "C:\\Users\\user\\AppData\\Roaming")
        } else {
            ("HOME", "/home/user")
        };
        let environment = MapEnvironment::from([(variable, root)]);
        let path = config_path(&environment).unwrap();
        assert!(path.is_absolute(), "{path:?} is not absolute");
        assert!(path.starts_with(root), "{path:?} is not under {root}");
        // The real invariant: never anything relative to the working directory.
        assert!(
            !path.starts_with("."),
            "the configuration path is relative to the working directory: {path:?}"
        );
    }
}

#[cfg(test)]
mod provider_alias_tests {
    #[test]
    fn llama_cpp_cli_alias_is_also_accepted_in_config() {
        let mut settings = super::Settings::default();
        settings.set("provider", "llama-cpp", "config").unwrap();
        assert_eq!(settings.get("provider").as_deref(), Some("llama-cpp"));
        let parsed = super::Settings::parse("provider = \"llama-cpp\"", "config").unwrap();
        assert_eq!(parsed.provider.as_deref(), Some("llama-cpp"));
    }
}
