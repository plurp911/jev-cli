//! The resolved runtime context: what `jev` will actually do, after flags, the
//! configuration file, and the environment have all been considered.
//!
//! # Precedence
//!
//! For every setting, most specific first:
//!
//! 1. a command-line flag;
//! 2. the configuration file (skipped entirely with `--no-config`);
//! 3. the built-in default.
//!
//! `NO_COLOR` sits between (1) and (2) for colour only, per <https://no-color.org>.
//! `CLOUDFLARE_ACCOUNT_ID` is consulted only for explicitly selected Cloudflare.
//! No `.env` file is ever read.
//!
//! Resolution happens once, in one place, and the result is a value that every command
//! reads. `jev doctor` prints exactly this structure, so what the user is told is what
//! the code uses rather than a second description of it that can drift.

use std::path::PathBuf;
use std::time::Duration;

use jev_client::{Endpoint, RetryPolicy};
use jev_config::{Environment, Settings};
use jev_core::ModelId;
use jev_core::limits::DEFAULT_MAX_INPUT_BYTES;

use crate::cli::{ColorArg, OutputFormat, ProviderArg};
use crate::errors::{CliError, Result};
use crate::output::ColorChoice;

/// Default per-attempt HTTP timeout.
///
/// Matches the official Python SDK's `DEFAULT_TIMEOUT`, so `jev` waits as long as a
/// user's SDK-based code would.
pub(crate) const DEFAULT_TIMEOUT_SECONDS: u64 = 10;

/// Longest per-attempt timeout that may be requested.
///
/// `--retries` is clamped (`RetryPolicy::clamp_retries`) because an unbounded count
/// would make the retry budget unbounded, and the budget is what proves the loop
/// terminates. `--timeout` multiplies into that same budget, so leaving it unbounded
/// defeated the invariant from the other side: `--timeout 999999999` hung indefinitely,
/// and `--timeout 9223372036854775807` overflowed `Instant + Duration` and **panicked**
/// with exit 101 — a panic on user input, which `AGENTS.md` §3.2 forbids outright.
///
/// One hour is far beyond any real System One call, leaves the existing
/// `--retries`-clamping test's 3600-second timeout valid, and is finite — which is the
/// whole point.
pub(crate) const MAX_TIMEOUT_SECONDS: u64 = 3600;

/// What happened to the configuration file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigState {
    /// A file existed and was read.
    Loaded,
    /// No file exists at the expected path.
    Absent,
    /// `--no-config` was given.
    Skipped,
    /// No configuration directory could be determined.
    NoDirectory,
}

impl ConfigState {
    /// A stable identifier, part of the `jev doctor` JSON contract.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Loaded => "loaded",
            Self::Absent => "absent",
            Self::Skipped => "skipped",
            Self::NoDirectory => "no-directory",
        }
    }
}

/// Where a setting's value came from, for `jev doctor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// A command-line flag.
    Flag,
    /// The configuration file.
    ConfigFile,
    /// The built-in default.
    Default,
}

impl Provenance {
    /// A stable identifier, part of the `jev doctor` JSON contract.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::ConfigFile => "config-file",
            Self::Default => "default",
        }
    }
}

/// A resolved value and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Sourced<T> {
    /// The value in force.
    pub(crate) value: T,
    /// Where it came from.
    pub(crate) from: Provenance,
}

impl<T> Sourced<T> {
    fn new(value: T, from: Provenance) -> Self {
        Self { value, from }
    }
}

/// How much `jev` says on stderr.
///
/// Grouped rather than left as two loose booleans so that "quiet and verbose at once"
/// has one place to be resolved, and so the surrounding structs stay readable.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Verbosity {
    /// Suppress non-error diagnostics.
    pub(crate) quiet: bool,
    /// Emit extra diagnostics.
    pub(crate) verbose: bool,
}

/// Everything resolved, ready for a command to use.
#[derive(Debug, Clone)]
pub struct Context {
    /// Selected inference protocol and its provenance.
    pub(crate) provider: Sourced<ProviderArg>,
    /// Explicit image file paths; read only while constructing inference requests.
    pub(crate) image_paths: Vec<PathBuf>,
    /// Explicitly prepared frames for one local video.
    pub(crate) video_frames: Vec<PathBuf>,
    /// Source cadence for the explicitly supplied prepared video frames.
    pub(crate) video_fps: Option<f64>,
    /// Bridge context length control.
    pub(crate) max_length: Option<u32>,
    /// Independent local state token budget.
    pub(crate) max_state_tokens: Option<u32>,
    /// Explicit processor media controls.
    pub(crate) media_kwargs: Option<serde_json::Value>,
    /// Cloudflare capacity control for this invocation.
    pub(crate) reject_if_busy: bool,
    /// Ollama model lifetime override for this invocation.
    pub(crate) keep_alive: Option<String>,
    /// The API base URL.
    pub(crate) endpoint: Sourced<Endpoint>,
    /// A saved endpoint belonged to a different provider and was not inherited.
    pub(crate) ignored_config_endpoint: bool,
    /// The model to request.
    pub(crate) model: Sourced<ModelId>,
    /// The output format.
    pub(crate) output: Sourced<OutputFormat>,
    /// Whether colour is enabled on stderr. stdout is never coloured for JSON.
    pub(crate) color: ColorChoice,
    /// Per-attempt HTTP timeout.
    pub(crate) timeout: Sourced<Duration>,
    /// The retry policy.
    pub(crate) retry: RetryPolicy,
    /// Ceiling on bytes read from one input source.
    pub(crate) max_input_bytes: Sourced<u64>,
    /// Where the configuration file would be, whether or not one exists.
    pub(crate) config_path: Option<PathBuf>,
    /// How the configuration file was treated.
    pub(crate) config_state: ConfigState,
    /// How much to say on stderr.
    pub(crate) verbosity: Verbosity,
    /// Show the request instead of sending it.
    pub(crate) dry_run: bool,
}

/// The command-line values that feed resolution.
///
/// A plain struct rather than the `clap` type so that resolution can be unit-tested
/// without building an argument vector.
#[derive(Debug, Default, Clone)]
pub struct Overrides {
    /// `--provider`
    pub(crate) provider: Option<ProviderArg>,
    /// `--cloudflare-account-id`
    pub(crate) cloudflare_account_id: Option<String>,
    /// Repeated `--image` paths.
    pub(crate) image_paths: Vec<PathBuf>,
    /// Repeated `--video-frame` paths.
    pub(crate) video_frames: Vec<PathBuf>,
    /// Source cadence for the explicitly supplied prepared video frames.
    pub(crate) video_fps: Option<f64>,
    /// `--max-length`
    pub(crate) max_length: Option<u32>,
    /// Independent local state token budget.
    pub(crate) max_state_tokens: Option<u32>,
    /// `--media-kwargs` JSON.
    pub(crate) media_kwargs: Option<String>,
    /// `--reject-if-busy`
    pub(crate) reject_if_busy: bool,
    /// `--keep-alive`
    pub(crate) keep_alive: Option<String>,
    /// `--endpoint`
    pub(crate) endpoint: Option<String>,
    /// `--model`
    pub(crate) model: Option<String>,
    /// `--output`
    pub(crate) output: Option<OutputFormat>,
    /// `--color`
    pub(crate) color: Option<ColorArg>,
    /// `--timeout`
    pub(crate) timeout_seconds: Option<u64>,
    /// `--retries`
    pub(crate) retries: Option<u32>,
    /// `--max-input-bytes`
    pub(crate) max_input_bytes: Option<u64>,
    /// `--no-config`
    pub(crate) no_config: bool,
    /// `--quiet` and `--verbose`
    pub(crate) verbosity: Verbosity,
    /// `--dry-run`
    pub(crate) dry_run: bool,
}

/// Resolves the retry policy from the flag, the configuration file, and the timeout.
///
/// A flag is this invocation's explicit intent, so an out-of-range one is **refused**
/// and the user learns the bound — the same rule `--timeout` applies. It used to be
/// clamped silently, so a CI job asking for `--retries 50` got 10 with nothing said,
/// even under `--verbose`. A stale value in a configuration file is still clamped
/// rather than refused, because it should not make every invocation fail.
fn resolve_retry(
    from_flag: Option<u32>,
    from_file: Option<u32>,
    timeout: Duration,
) -> Result<RetryPolicy> {
    if let Some(requested) = from_flag
        && requested > jev_client::MAX_RETRIES
    {
        return Err(CliError::usage(format!(
            "--retries must be at most {}",
            jev_client::MAX_RETRIES
        )));
    }
    let default_retry = RetryPolicy::default();
    // Clamped before it is used anywhere. An unbounded count would make the total
    // budget unbounded too, and the budget is what guarantees the retry loop
    // terminates.
    let max_retries =
        RetryPolicy::clamp_retries(from_flag.or(from_file).unwrap_or(default_retry.max_retries));
    Ok(RetryPolicy {
        max_retries,
        // The budget must cover the attempts the user asked for, or a raised
        // `--retries` would be silently capped by the default 30-second budget. It
        // stays bounded because `max_retries` is.
        total_budget: default_retry
            .total_budget
            .max(timeout.saturating_mul(max_retries.saturating_add(1))),
        ..default_retry
    })
}

impl Context {
    /// Resolves the context.
    ///
    /// `stdout_is_terminal` is injected rather than probed so that colour resolution is
    /// testable. It is stdout's terminal-ness deliberately: that is the stream the
    /// colour would be written to.
    ///
    /// # Errors
    ///
    /// Returns a usage-class [`CliError`] when the configuration file is unusable or a
    /// flag value is invalid.
    pub fn resolve(
        overrides: &Overrides,
        environment: &dyn Environment,
        stdout_is_terminal: bool,
    ) -> Result<Self> {
        let config_path = jev_config::config_path(environment).ok();
        let (settings, config_state) = if overrides.no_config {
            (Settings::default(), ConfigState::Skipped)
        } else {
            match &config_path {
                // `Settings::load` treats a missing file as defaults, so the distinction
                // between "loaded" and "absent" is made here rather than lost.
                Some(path) if path.is_file() => (Settings::load(path)?, ConfigState::Loaded),
                Some(_) => (Settings::default(), ConfigState::Absent),
                // No configuration directory is not an error: it only means there is no
                // file to read, and `jev doctor` says so.
                None => (Settings::default(), ConfigState::NoDirectory),
            }
        };

        let provider = resolve_provider(overrides.provider, settings.provider.as_deref());
        validate_provider_flags(overrides, provider.value)?;

        let model = match (&overrides.model, &settings.model) {
            (Some(raw), _) => Sourced::new(parse_model(raw, "--model")?, Provenance::Flag),
            (None, Some(raw)) => Sourced::new(
                parse_model(raw, "the `model` setting")?,
                Provenance::ConfigFile,
            ),
            (None, None) => Sourced::new(
                if provider.value == ProviderArg::Typesafe {
                    ModelId::default()
                } else {
                    parse_model("clef", "provider default")?
                },
                Provenance::Default,
            ),
        };

        let output = match (overrides.output, settings.output.as_deref()) {
            (Some(format), _) => Sourced::new(format, Provenance::Flag),
            (None, Some("json")) => Sourced::new(OutputFormat::Json, Provenance::ConfigFile),
            (None, Some("text")) => Sourced::new(OutputFormat::Text, Provenance::ConfigFile),
            // `Settings::validate` has already rejected anything else.
            (None, _) => Sourced::new(OutputFormat::Text, Provenance::Default),
        };

        let color = resolve_color(
            overrides.color,
            settings.color.as_deref(),
            environment,
            stdout_is_terminal,
        );

        let timeout = match (overrides.timeout_seconds, settings.timeout_seconds) {
            (Some(0), _) => {
                return Err(CliError::usage("--timeout must be at least 1 second"));
            }
            (Some(seconds), _) if seconds > MAX_TIMEOUT_SECONDS => {
                return Err(CliError::usage(format!(
                    "--timeout must be at most {MAX_TIMEOUT_SECONDS} seconds"
                )));
            }
            (Some(seconds), _) => Sourced::new(Duration::from_secs(seconds), Provenance::Flag),
            (None, Some(seconds)) if seconds > 0 => Sourced::new(
                // A config file is not a usage error, so an out-of-range value there is
                // clamped rather than refused: a stale setting should not make every
                // invocation fail.
                Duration::from_secs(seconds.min(MAX_TIMEOUT_SECONDS)),
                Provenance::ConfigFile,
            ),
            _ => Sourced::new(
                Duration::from_secs(DEFAULT_TIMEOUT_SECONDS),
                Provenance::Default,
            ),
        };

        let max_input_bytes = match (overrides.max_input_bytes, settings.max_input_bytes) {
            (Some(0), _) => {
                return Err(CliError::usage("--max-input-bytes must be at least 1"));
            }
            (Some(bytes), _) => Sourced::new(bytes, Provenance::Flag),
            (None, Some(bytes)) if bytes > 0 => Sourced::new(bytes, Provenance::ConfigFile),
            _ => Sourced::new(DEFAULT_MAX_INPUT_BYTES, Provenance::Default),
        };

        let retry = resolve_retry(overrides.retries, settings.retries, timeout.value)?;

        let endpoint = resolve_endpoint(overrides, &settings, environment, provider.value)?;
        let ignored_config_endpoint =
            settings.endpoint.is_some() && endpoint.from == Provenance::Default;

        Ok(Self {
            provider,
            image_paths: overrides.image_paths.clone(),
            video_frames: overrides.video_frames.clone(),
            video_fps: overrides.video_fps,
            max_length: overrides.max_length,
            max_state_tokens: overrides.max_state_tokens,
            media_kwargs: overrides
                .media_kwargs
                .as_deref()
                .map(parse_media_kwargs)
                .transpose()?,
            reject_if_busy: overrides.reject_if_busy,
            keep_alive: overrides.keep_alive.clone(),
            endpoint,
            ignored_config_endpoint,
            model,
            output,
            color,
            timeout,
            retry,
            max_input_bytes,
            config_path,
            config_state,
            verbosity: Verbosity {
                quiet: overrides.verbosity.quiet,
                // `clap` already rejects the two together, but resolving it here as
                // well means the invariant holds for any caller of `resolve`.
                verbose: overrides.verbosity.verbose && !overrides.verbosity.quiet,
            },
            dry_run: overrides.dry_run,
        })
    }

    /// Whether the endpoint is the official TypeSafe API.
    pub(crate) fn endpoint_is_official(&self) -> bool {
        self.endpoint.value.is_official()
    }

    /// The warning owed to the user when a non-default endpoint is in force.
    ///
    /// Printed on stderr on every call that uses one. A silent override is exactly the
    /// failure mode `docs/threat-model.md` T4 is about.
    pub(crate) fn endpoint_warning(&self) -> Option<String> {
        if self.endpoint_is_official() {
            return None;
        }
        if self.endpoint.value.is_local_provider() && self.endpoint.value.is_loopback() {
            return Some(format!(
                "note: using local {} server at {}; no credentials are used",
                self.provider.value.as_str(),
                self.endpoint.value
            ));
        }
        Some(format!(
            "warning: sending to a non-official endpoint: {} (from {})\n\
             warning: TypeSafe credentials are not used for this host; \
             {} applies",
            self.endpoint.value,
            self.endpoint.from.as_str(),
            jev_config::CUSTOM_API_KEY_ENV,
        ))
    }
}

fn resolve_provider(flag: Option<ProviderArg>, file: Option<&str>) -> Sourced<ProviderArg> {
    if let Some(provider) = flag {
        return Sourced::new(provider, Provenance::Flag);
    }
    let provider = match file {
        Some("cloudflare") => ProviderArg::Cloudflare,
        Some("ollama") => ProviderArg::Ollama,
        Some("huggingface") => ProviderArg::Huggingface,
        Some("llamacpp" | "llama-cpp") => ProviderArg::LlamaCpp,
        _ => ProviderArg::Typesafe,
    };
    Sourced::new(
        provider,
        if file.is_some() {
            Provenance::ConfigFile
        } else {
            Provenance::Default
        },
    )
}

fn validate_provider_flags(overrides: &Overrides, provider: ProviderArg) -> Result<()> {
    if provider != ProviderArg::Cloudflare {
        if overrides.cloudflare_account_id.is_some() {
            return Err(CliError::usage(
                "--cloudflare-account-id requires --provider cloudflare",
            ));
        }
        if overrides.reject_if_busy {
            return Err(CliError::usage(
                "--reject-if-busy requires --provider cloudflare",
            ));
        }
    }
    if overrides.keep_alive.is_some() && provider != ProviderArg::Ollama {
        return Err(CliError::usage("--keep-alive requires --provider ollama"));
    }
    if !overrides.image_paths.is_empty()
        && !matches!(
            provider,
            ProviderArg::Cloudflare | ProviderArg::Ollama | ProviderArg::Huggingface
        )
    {
        return Err(CliError::usage(
            "--image requires --provider cloudflare, ollama, or huggingface",
        ));
    }
    if (!overrides.video_frames.is_empty()
        || overrides.video_fps.is_some()
        || overrides.max_length.is_some()
        || overrides.max_state_tokens.is_some()
        || overrides.media_kwargs.is_some())
        && provider != ProviderArg::Huggingface
    {
        return Err(CliError::usage(
            "--video-frame, --video-fps, --max-length, --max-state-tokens, and --media-kwargs require --provider huggingface",
        ));
    }
    if overrides
        .max_state_tokens
        .is_some_and(|value| value > 65536)
    {
        return Err(CliError::usage(
            "--max-state-tokens must be between 0 and 65536",
        ));
    }
    if overrides
        .video_fps
        .is_some_and(|value| !value.is_finite() || value <= 0.0 || value > 120.0)
    {
        return Err(CliError::usage(
            "--video-fps must be finite, greater than zero, and at most 120",
        ));
    }
    if overrides.video_fps.is_some() && overrides.video_frames.is_empty() {
        return Err(CliError::usage(
            "--video-fps requires explicitly supplied --video-frame paths; document videos carry their own metadata",
        ));
    }
    if overrides.video_frames.len() > 32 {
        return Err(CliError::usage(
            "--video-frame accepts at most 32 frames per video",
        ));
    }
    if overrides
        .max_length
        .is_some_and(|value| value == 0 || value > 65_536)
    {
        return Err(CliError::usage("--max-length must be between 1 and 65536"));
    }
    if overrides.image_paths.len() > 4 {
        return Err(CliError::usage(
            "--image accepts at most 4 images per request",
        ));
    }
    Ok(())
}

fn resolve_endpoint(
    overrides: &Overrides,
    settings: &Settings,
    environment: &dyn Environment,
    provider: ProviderArg,
) -> Result<Sourced<Endpoint>> {
    let override_endpoint = match (&overrides.endpoint, &settings.endpoint) {
        (Some(raw), _) => Some(Sourced::new(
            parse_endpoint(raw, "--endpoint")?,
            Provenance::Flag,
        )),
        (None, Some(raw))
            if resolve_provider(None, settings.provider.as_deref()).value == provider =>
        {
            Some(Sourced::new(
                parse_endpoint(raw, "the `endpoint` setting")?,
                Provenance::ConfigFile,
            ))
        }
        _ => None,
    };
    match provider {
        ProviderArg::Typesafe => Ok(override_endpoint
            .unwrap_or_else(|| Sourced::new(Endpoint::official(), Provenance::Default))),
        ProviderArg::Cloudflare => {
            let account = overrides
                .cloudflare_account_id
                .clone()
                .or_else(|| environment.var("CLOUDFLARE_ACCOUNT_ID"))
                .or_else(|| settings.cloudflare_account_id.clone())
                .ok_or(CliError::IncompleteCloudflareConfiguration)?;
            let configured = match override_endpoint {
                Some(configured) => Sourced::new(
                    configured
                        .value
                        .with_cloudflare_account(&account)
                        .map_err(|e| CliError::usage(e.to_string()))?,
                    configured.from,
                ),
                None => Sourced::new(
                    Endpoint::cloudflare(&account).map_err(|e| CliError::usage(e.to_string()))?,
                    Provenance::Default,
                ),
            };
            Ok(configured)
        }
        ProviderArg::Ollama => Ok(match override_endpoint {
            Some(configured) => Sourced::new(configured.value.with_ollama(), configured.from),
            None => Sourced::new(Endpoint::ollama(), Provenance::Default),
        }),
        ProviderArg::Huggingface => Ok(match override_endpoint {
            Some(configured) => Sourced::new(configured.value.with_huggingface(), configured.from),
            None => Sourced::new(Endpoint::huggingface(), Provenance::Default),
        }),
        ProviderArg::LlamaCpp => Ok(match override_endpoint {
            Some(configured) => Sourced::new(configured.value.with_llama_cpp(), configured.from),
            None => Sourced::new(Endpoint::llama_cpp(), Provenance::Default),
        }),
    }
}

fn parse_media_kwargs(raw: &str) -> Result<serde_json::Value> {
    if raw.len() > 4096 {
        return Err(CliError::usage(
            "--media-kwargs exceeds the 4096 byte client limit",
        ));
    }
    crate::ordered::parse_unambiguous_value(raw).map_err(|error| {
        CliError::usage(format!(
            "--media-kwargs must be valid JSON with unique object keys: {error}"
        ))
    })
}

fn parse_endpoint(raw: &str, origin: &str) -> Result<Endpoint> {
    Endpoint::parse(raw).map_err(|error| CliError::usage(format!("{origin}: {error}")))
}

fn parse_model(raw: &str, origin: &str) -> Result<ModelId> {
    ModelId::new(raw).map_err(|error| CliError::usage(format!("{origin}: {error}")))
}

/// Resolves the colour policy.
///
/// Split out of [`Context::resolve`] to keep that function within the workspace's line
/// limit; the ordering it implements is the documented one, and is the reason it is not
/// a one-liner. Precedence, highest first: `--color`, then `NO_COLOR`, then the
/// configuration file, then automatic (`docs/cli-contract.md`).
fn resolve_color(
    flag: Option<ColorArg>,
    file: Option<&str>,
    environment: &dyn Environment,
    stdout_is_terminal: bool,
) -> ColorChoice {
    // `Settings::validate` has already rejected any value outside the three names.
    let (choice, from_flag) = match flag {
        Some(ColorArg::Always) => (ColorChoice::Always, true),
        Some(ColorArg::Never) => (ColorChoice::Never, true),
        Some(ColorArg::Auto) => (ColorChoice::Auto, true),
        None => (
            match file {
                Some("always") => ColorChoice::Always,
                Some("never") => ColorChoice::Never,
                _ => ColorChoice::Auto,
            },
            false,
        ),
    };

    // Any value at all, including the empty string, disables colour.
    let no_color = environment.var("NO_COLOR").is_some();
    // `NO_COLOR` sits *between* the flag and the file, so it vetoes `color = "always"`
    // from the configuration file while leaving `--color always` -- an instruction the
    // user typed for this invocation -- untouched. Without this, `ColorChoice::Always`
    // returned `true` unconditionally and the file silently outranked the environment,
    // which three documents and this module's own comment all said it did not.
    let choice = if no_color && !from_flag {
        ColorChoice::Never
    } else {
        choice
    };

    if choice.enabled(no_color, stdout_is_terminal) {
        ColorChoice::Always
    } else {
        ColorChoice::Never
    }
}

#[cfg(test)]
mod tests {
    use jev_config::{CONFIG_DIR_ENV, MapEnvironment};

    use super::*;

    fn empty_env() -> MapEnvironment {
        // Points the config directory at a path that does not exist, so resolution sees
        // no file and the developer's real configuration is never read.
        MapEnvironment::from([(CONFIG_DIR_ENV, "/nonexistent/jev-context-test")])
    }

    fn resolve(overrides: &Overrides) -> Result<Context> {
        Context::resolve(overrides, &empty_env(), false)
    }

    #[test]
    fn a_missing_configuration_file_is_reported_as_absent_not_loaded() {
        let context = resolve(&Overrides::default()).unwrap();
        assert_eq!(context.config_state, ConfigState::Absent);
    }

    #[test]
    fn defaults_are_the_documented_ones() {
        let context = resolve(&Overrides::default()).unwrap();
        assert_eq!(context.endpoint.value.base(), "https://api.typesafe.ai");
        assert_eq!(context.endpoint.from, Provenance::Default);
        assert_eq!(context.model.value.as_str(), "jev-latest");
        assert_eq!(context.output.value, OutputFormat::Text);
        assert_eq!(context.timeout.value, Duration::from_secs(10));
        assert_eq!(context.retry.max_retries, 2);
        assert_eq!(context.max_input_bytes.value, DEFAULT_MAX_INPUT_BYTES);
        assert!(context.endpoint_is_official());
        assert!(context.endpoint_warning().is_none());
    }

    #[test]
    fn a_flag_beats_the_configuration_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "model = \"from-config\"\nretries = 7\n",
        )
        .unwrap();
        let environment =
            MapEnvironment::from([(CONFIG_DIR_ENV, dir.path().to_string_lossy().as_ref())]);

        let from_config = Context::resolve(&Overrides::default(), &environment, false).unwrap();
        assert_eq!(from_config.model.value.as_str(), "from-config");
        assert_eq!(from_config.model.from, Provenance::ConfigFile);
        assert_eq!(from_config.retry.max_retries, 7);

        let overridden = Context::resolve(
            &Overrides {
                model: Some("from-flag".to_owned()),
                ..Overrides::default()
            },
            &environment,
            false,
        )
        .unwrap();
        assert_eq!(overridden.model.value.as_str(), "from-flag");
        assert_eq!(overridden.model.from, Provenance::Flag);
    }

    #[test]
    fn no_config_ignores_the_file_entirely() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), "model = \"from-config\"\n").unwrap();
        let environment =
            MapEnvironment::from([(CONFIG_DIR_ENV, dir.path().to_string_lossy().as_ref())]);
        let context = Context::resolve(
            &Overrides {
                no_config: true,
                ..Overrides::default()
            },
            &environment,
            false,
        )
        .unwrap();
        assert_eq!(context.model.value.as_str(), "jev-latest");
        assert_eq!(context.config_state, ConfigState::Skipped);
    }

    #[test]
    fn a_non_official_endpoint_always_warns_and_names_its_own_key_variable() {
        // T4: an override must never be invisible.
        let context = resolve(&Overrides {
            endpoint: Some("https://proxy.example.com".to_owned()),
            ..Overrides::default()
        })
        .unwrap();
        assert!(!context.endpoint_is_official());
        let warning = context.endpoint_warning().expect("a warning is owed");
        assert!(warning.contains("proxy.example.com"));
        assert!(warning.contains("JEV_CUSTOM_API_KEY"));
        assert!(warning.contains("flag"));
    }

    #[test]
    fn an_endpoint_from_the_config_file_warns_and_says_where_it_came_from() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            "endpoint = \"https://proxy.example.com\"\n",
        )
        .unwrap();
        let environment =
            MapEnvironment::from([(CONFIG_DIR_ENV, dir.path().to_string_lossy().as_ref())]);
        let context = Context::resolve(&Overrides::default(), &environment, false).unwrap();
        let warning = context.endpoint_warning().unwrap();
        assert!(warning.contains("config-file"), "{warning}");
    }

    #[test]
    fn an_insecure_endpoint_is_refused_at_resolution_time() {
        let error = resolve(&Overrides {
            endpoint: Some("http://evil.example.com".to_owned()),
            ..Overrides::default()
        })
        .unwrap_err();
        assert!(error.to_string().contains("plain HTTP"));
    }

    #[test]
    fn colour_follows_stdout_not_stderr() {
        // `jev … > file` in an ordinary terminal must not write ANSI escapes into the
        // file just because stderr is still a tty.
        let context = Context::resolve(&Overrides::default(), &empty_env(), false).unwrap();
        assert_eq!(context.color, ColorChoice::Never);
        let context = Context::resolve(&Overrides::default(), &empty_env(), true).unwrap();
        assert_eq!(context.color, ColorChoice::Always);
    }

    #[test]
    fn no_color_disables_colour_even_on_a_terminal() {
        let environment = empty_env().with("NO_COLOR", "");
        let context = Context::resolve(&Overrides::default(), &environment, true).unwrap();
        assert_eq!(context.color, ColorChoice::Never);
    }

    #[test]
    fn no_color_overrides_always_from_the_configuration_file() {
        // `docs/cli-contract.md`: "`NO_COLOR` sits between the flag and the file". Only
        // the flag outranks it. `ColorChoice::Always` used to return `true`
        // unconditionally, so a file setting silently won -- the one ordering none of
        // the existing tests covered.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), "color = \"always\"\n").unwrap();
        let environment = MapEnvironment::from([
            (CONFIG_DIR_ENV, dir.path().to_string_lossy().as_ref()),
            ("NO_COLOR", "1"),
        ]);
        let context = Context::resolve(&Overrides::default(), &environment, true).unwrap();
        assert_eq!(context.color, ColorChoice::Never);
    }

    #[test]
    fn a_configuration_file_still_enables_colour_without_no_color() {
        // The veto above must not turn into "the file setting does nothing".
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), "color = \"always\"\n").unwrap();
        let environment =
            MapEnvironment::from([(CONFIG_DIR_ENV, dir.path().to_string_lossy().as_ref())]);
        let context = Context::resolve(&Overrides::default(), &environment, false).unwrap();
        assert_eq!(context.color, ColorChoice::Always);
    }

    #[test]
    fn color_always_overrides_no_color() {
        let environment = empty_env().with("NO_COLOR", "1");
        let context = Context::resolve(
            &Overrides {
                color: Some(ColorArg::Always),
                ..Overrides::default()
            },
            &environment,
            false,
        )
        .unwrap();
        assert_eq!(context.color, ColorChoice::Always);
    }

    #[test]
    fn a_raised_retry_count_raises_the_total_budget_with_it() {
        // Otherwise `--retries 20` would be silently capped by the default 30-second
        // budget, and the user would never learn why.
        let context = resolve(&Overrides {
            retries: Some(8),
            timeout_seconds: Some(10),
            ..Overrides::default()
        })
        .unwrap();
        assert_eq!(context.retry.max_retries, 8);
        assert!(context.retry.total_budget >= Duration::from_secs(90));
    }

    #[test]
    fn an_absurd_timeout_is_refused_rather_than_panicking() {
        // `--timeout 9223372036854775807` overflowed `Instant + Duration` inside the
        // standard library and aborted with exit 101. A panic on user input is
        // forbidden (`AGENTS.md` §3.2), and the value also defeated the bound that
        // makes the retry loop provably terminate.
        let error = resolve(&Overrides {
            timeout_seconds: Some(u64::MAX),
            ..Overrides::default()
        })
        .expect_err("an absurd timeout should be a usage error");
        assert!(
            error.to_string().contains("at most"),
            "unhelpful message: {error}"
        );

        // The ceiling itself is still accepted, so the refusal is a bound and not an
        // off-by-one that makes the documented maximum unusable.
        assert!(
            resolve(&Overrides {
                timeout_seconds: Some(MAX_TIMEOUT_SECONDS),
                ..Overrides::default()
            })
            .is_ok()
        );
    }

    #[test]
    fn an_absurd_timeout_in_the_configuration_file_is_clamped_not_refused() {
        // A stale setting in a file the user may not remember writing should not make
        // every invocation fail; a flag, which is this invocation's explicit intent,
        // should be refused so the user learns the bound.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            format!("timeout_seconds = {}\n", MAX_TIMEOUT_SECONDS * 1000),
        )
        .unwrap();
        let environment =
            MapEnvironment::from([(CONFIG_DIR_ENV, dir.path().to_string_lossy().as_ref())]);
        let context = Context::resolve(&Overrides::default(), &environment, false)
            .expect("a file value should be clamped, not refused");
        assert_eq!(
            context.timeout.value,
            Duration::from_secs(MAX_TIMEOUT_SECONDS)
        );
    }

    #[test]
    fn an_absurd_retry_count_on_the_flag_is_refused() {
        // The same rule `--timeout` applies: a flag is this invocation's explicit
        // intent, so an out-of-range one is refused and the user learns the bound. It
        // used to be clamped silently, so a CI job asking for `--retries 50` got 10
        // with nothing said, even under `--verbose`.
        let error = resolve(&Overrides {
            retries: Some(u32::MAX),
            ..Overrides::default()
        })
        .expect_err("an out-of-range --retries should be refused");
        assert_eq!(error.code(), crate::exit::USAGE);
        assert!(error.to_string().contains("at most 10"), "{error}");

        // The boundary itself is accepted.
        assert!(
            resolve(&Overrides {
                retries: Some(jev_client::MAX_RETRIES),
                ..Overrides::default()
            })
            .is_ok()
        );
    }

    #[test]
    fn an_absurd_retry_count_in_the_configuration_file_is_clamped_so_the_budget_stays_bounded() {
        // A stale configuration file should not make every invocation fail, so the
        // value there is clamped rather than refused -- and the clamp is what keeps the
        // retry loop terminating. `retries = 4294967295` used to produce a budget of
        // roughly 1,361 years and an attempt counter that saturated below the limit, so
        // the loop never ended.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.toml"),
            format!("retries = {}\ntimeout_seconds = 3600\n", u32::MAX),
        )
        .unwrap();
        let environment =
            MapEnvironment::from([(CONFIG_DIR_ENV, dir.path().to_string_lossy().as_ref())]);
        let context = Context::resolve(&Overrides::default(), &environment, false)
            .expect("a file value should be clamped, not refused");
        assert_eq!(
            context.retry.max_retries,
            jev_client::MAX_RETRIES,
            "the retry count was not clamped"
        );
        assert!(
            context.retry.total_budget < Duration::from_secs(60 * 60 * 24),
            "the total budget is not bounded: {:?}",
            context.retry.total_budget
        );
    }

    #[test]
    fn zero_valued_bounds_are_refused() {
        assert!(
            resolve(&Overrides {
                timeout_seconds: Some(0),
                ..Overrides::default()
            })
            .is_err()
        );
        assert!(
            resolve(&Overrides {
                max_input_bytes: Some(0),
                ..Overrides::default()
            })
            .is_err()
        );
    }

    #[test]
    fn quiet_suppresses_verbose() {
        let context = resolve(&Overrides {
            verbosity: Verbosity {
                quiet: true,
                verbose: true,
            },
            ..Overrides::default()
        })
        .unwrap();
        assert!(context.verbosity.quiet);
        assert!(!context.verbosity.verbose);
    }

    #[test]
    fn a_malformed_config_file_fails_loudly() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), "api_key = \"sk-oops\"\n").unwrap();
        let environment =
            MapEnvironment::from([(CONFIG_DIR_ENV, dir.path().to_string_lossy().as_ref())]);
        let error = Context::resolve(&Overrides::default(), &environment, false).unwrap_err();
        assert!(error.to_string().contains("looks like a credential"));
        assert!(!error.to_string().contains("sk-oops"));
    }
}
