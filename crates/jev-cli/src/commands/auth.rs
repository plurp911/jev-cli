//! `jev auth` — storing, reporting, and removing the credential.
//!
//! # Rules
//!
//! * **No key on the command line, ever.** There is no `--api-key` flag, and
//!   `crates/jev-cli/src/cli.rs` has a test that walks every argument of every
//!   subcommand to keep it that way. Arguments show up in `ps`, in shell history, and
//!   in CI logs (`docs/threat-model.md` T1).
//! * **No plaintext fallback.** If secure storage is unavailable, `login` fails and
//!   says to use `JEV_API_KEY`. It does not write a file. That is ADR-0002's central
//!   decision and the one thing no other CLI in this space does.
//! * **`logout` removes only what `jev` stored.** It touches one entry, under one
//!   service name, and reports honestly when there was nothing there.
//! * **The key is never echoed**, at the prompt or afterwards.

use std::fmt::Write as _;
use std::io::Read as _;

use jev_config::{CredentialSource, Secret};
use serde_json::{Value, json};

use crate::cli::{AuthCommand, LoginArgs};
use crate::commands::Session;
use crate::errors::{CliError, Result};
use crate::exit;
use crate::output::Safe;
use crate::render::{AUTH_SCHEMA, json as render_json};

/// Largest key `jev` will accept from a prompt or stdin.
///
/// Matches the file-based bound in `jev-config`. A paste of the wrong buffer should be
/// refused, not stored.
const MAX_KEY_LENGTH: usize = 4096;

/// Runs `jev auth`.
pub(crate) fn run(session: &mut Session<'_>, command: AuthCommand) -> Result<u8> {
    match command {
        AuthCommand::Login(args) => login(session, &args),
        AuthCommand::Status => status(session),
        AuthCommand::Logout => logout(session),
    }
}

fn login(session: &mut Session<'_>, args: &LoginArgs) -> Result<u8> {
    if !session.context.endpoint_is_official() {
        // Storing a key for an arbitrary host in the same slot the official endpoint
        // reads is precisely the confusion T4 is about. The custom-endpoint namespace
        // is environment-only, deliberately.
        return Err(CliError::usage(format!(
            "`jev auth login` stores a credential for the official TypeSafe API only, \
             and the configured endpoint is {}.\n\n\
             For a non-official endpoint, set {} in your environment.",
            session.context.endpoint.value,
            jev_config::CUSTOM_API_KEY_ENV,
        )));
    }

    let secret = if args.stdin {
        read_from_stdin(session)?
    } else {
        prompt(session)?
    };

    // The resolver's own rule: a key it would refuse on every later command is
    // refused here, before it reaches the store. The message never quotes the input.
    match jev_config::KeyDefect::of(&secret) {
        Some(jev_config::KeyDefect::Empty) => {
            return Err(CliError::usage("no key was entered; nothing was stored"));
        }
        Some(jev_config::KeyDefect::ControlCharacter) => {
            return Err(CliError::usage(
                "the key contains a line break or control character; a key is a single \
                 line of printable text, so nothing was stored",
            ));
        }
        None => {}
    }

    session
        .store
        .set(jev_config::OFFICIAL_ACCOUNT, &secret)
        .map_err(CliError::from)?;

    session.warn(&format!(
        "stored a credential in {}",
        session.store.describe()
    ));

    // An environment variable outranks the store (ADR-0008), so a key stored while one
    // is set would appear to have no effect. Saying so beats silent confusion.
    let shadowing = [
        jev_config::API_KEY_ENV,
        jev_config::API_KEY_FILE_ENV,
        jev_config::TYPESAFE_API_KEY_ENV,
    ]
    .into_iter()
    .find(|name| {
        session
            .environment
            .var(name)
            .is_some_and(|value| !value.trim().is_empty())
    });
    if let Some(name) = shadowing {
        session.warn(&format!(
            "note: ${name} is set and takes precedence over the credential store, \
             so the stored key will not be used until it is unset"
        ));
    }

    if session.json() {
        let document = json!({
            "schema": AUTH_SCHEMA,
            "action": "login",
            "stored": true,
            "store": session.store.describe(),
            "shadowed_by": shadowing,
        });
        render_json::write_document(session.out, &document)?;
    }
    Ok(exit::SUCCESS)
}

/// Reads a key from a terminal without echoing it.
fn prompt(session: &mut Session<'_>) -> Result<Secret> {
    if session.stdin_is_terminal {
        // `rpassword` disables terminal echo for the duration of the read and restores
        // it afterwards, including on interrupt.
        // `rpassword` hands back a plain `String`; wrapping it immediately means the
        // original copy is cleared rather than dropped intact.
        let entered = zeroize::Zeroizing::new(
            rpassword::prompt_password("TypeSafe API key (input hidden): ").map_err(|error| {
                CliError::usage(format!("could not read the key: {}", error.kind()))
            })?,
        );
        return Ok(finish(&entered));
    }
    Err(CliError::usage(
        "standard input is not a terminal, so `jev auth login` cannot prompt.\n\n\
         Use `jev auth login --stdin` to read the key from a pipe, or set JEV_API_KEY.",
    ))
}

/// Reads a key from a pipe, for scripted provisioning.
fn read_from_stdin(session: &mut Session<'_>) -> Result<Secret> {
    let mut buffer = zeroize::Zeroizing::new(String::new());
    session
        .stdin
        .take(MAX_KEY_LENGTH as u64 + 1)
        .read_to_string(&mut buffer)
        .map_err(|error| CliError::usage(format!("could not read the key: {}", error.kind())))?;
    if buffer.len() > MAX_KEY_LENGTH {
        return Err(CliError::usage(format!(
            "the input is longer than {MAX_KEY_LENGTH} bytes; that is not an API key"
        )));
    }
    Ok(finish(&buffer))
}

/// Trims a key read from a prompt or a pipe.
///
/// A trailing newline from `echo` or from a secret manager would otherwise be sent as
/// part of the key and produce a baffling 401.
fn finish(raw: &str) -> Secret {
    Secret::new(raw.trim().to_owned())
}

fn status(session: &mut Session<'_>) -> Result<u8> {
    session.warn_about_endpoint();
    let credentials = jev_config::Credentials::new(
        session.environment,
        session.store,
        session.context.endpoint_is_official(),
        session.context.endpoint.value.to_string(),
    );
    let availability = credentials.availability();

    if session.json() {
        let document = status_document(session, &availability);
        render_json::write_document(session.out, &document)?;
    } else {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "endpoint  {}",
            Safe::new(&session.context.endpoint.value.to_string())
        );
        let _ = write!(
            out,
            "store     {}\n\n",
            Safe::new(&session.store.describe())
        );
        for status in &availability.sources {
            let _ = writeln!(
                out,
                "  {:<24}  {}",
                status.source.env_var().unwrap_or(status.source.as_str()),
                source_state(*status)
            );
        }
        match availability.effective {
            Some(source) => {
                let _ = writeln!(out, "\nin use: {}", source.as_str());
            }
            // A source that is set but unusable gets its own reason: reporting it as
            // "none" sent the user to `jev auth login` for a blank variable.
            None => match &availability.resolution_error {
                Some(error) => {
                    let _ = writeln!(out, "\nin use: none. {}", Safe::new(error));
                }
                // `auth login` refuses a custom endpoint, and neither the TypeSafe variables
                // nor the store are read for one (ADR-0008). Its own namespace is the only
                // advice that can work, and the store's state is irrelevant to it.
                None if !session.context.endpoint_is_official() => {
                    out.push_str(
                        "\nin use: none. This endpoint is not the official one, so set \
                         JEV_CUSTOM_API_KEY, or JEV_CUSTOM_API_KEY_FILE pointing at a file \
                         your secret manager controls.\n",
                    );
                }
                // Only suggest `auth login` when there is a store to log in to. In a
                // build compiled without one -- the distroless case ADR-0002 exists for
                // -- or on a machine whose store is broken, that command always fails,
                // and sending the user to it wastes their time.
                None if availability.keychain_error.is_some() => {
                    out.push_str(
                        "\nin use: none. Secure storage is unavailable here, so set \
                         JEV_API_KEY, or JEV_API_KEY_FILE pointing at a file your \
                         secret manager controls.\n",
                    );
                }
                None => out.push_str("\nin use: none. Run `jev auth login`, or set JEV_API_KEY.\n"),
            },
        }
        render_json::write_all(session.out, out.as_bytes())?;
    }

    // Exit 3 when there is no credential at all, so `jev auth status` is usable as a
    // precondition check in a script.
    Ok(if availability.effective.is_some() {
        exit::SUCCESS
    } else {
        exit::AUTH
    })
}

fn status_document(
    session: &Session<'_>,
    availability: &jev_config::CredentialAvailability,
) -> Value {
    json!({
        "schema": AUTH_SCHEMA,
        "action": "status",
        "endpoint": session.context.endpoint.value.to_string(),
        "official_endpoint": session.context.endpoint_is_official(),
        "store": session.store.describe(),
        "store_error": availability.keychain_error,
        "error": availability.resolution_error,
        "effective_source": availability.effective.map(CredentialSource::as_str),
        "sources": availability.sources.iter().map(|status| json!({
            "source": status.source.as_str(),
            "env_var": status.source.env_var(),
            "present": status.present,
            "set": status.set,
        })).collect::<Vec<_>>(),
    })
}

/// How one source reads in the text reports of `jev auth status` and `jev doctor`.
///
/// "set but unusable" is its own state: a blank variable or a two-line key file is
/// set, and resolution stops at it, so calling it "not set" contradicts the error the
/// same report prints underneath.
pub(crate) fn source_state(status: jev_config::SourceStatus) -> &'static str {
    match (status.set, status.present) {
        (Some(true), Some(true)) => "present",
        (Some(true), _) => "set but unusable",
        (Some(false), _) => "not set",
        (None, _) => "unavailable",
    }
}

fn logout(session: &mut Session<'_>) -> Result<u8> {
    let removed = session
        .store
        .delete(jev_config::OFFICIAL_ACCOUNT)
        .map_err(CliError::from)?;

    if removed {
        session.warn("removed the credential jev stored");
    } else {
        session.warn("jev had no stored credential; nothing was removed");
    }

    // Environment-supplied keys are not ours to remove, and a user who thinks `logout`
    // deauthenticated them would be wrong.
    let remaining: Vec<&str> = [
        jev_config::API_KEY_ENV,
        jev_config::API_KEY_FILE_ENV,
        jev_config::TYPESAFE_API_KEY_ENV,
    ]
    .into_iter()
    .filter(|name| {
        session
            .environment
            .var(name)
            .is_some_and(|value| !value.trim().is_empty())
    })
    .collect();
    if !remaining.is_empty() {
        session.warn(&format!(
            "note: {} still set in your environment; `jev` will keep using it",
            remaining.join(", ")
        ));
    }

    if session.json() {
        let document = json!({
            "schema": AUTH_SCHEMA,
            "action": "logout",
            "removed": removed,
            "store": session.store.describe(),
            "still_set": remaining,
        });
        render_json::write_document(session.out, &document)?;
    }
    Ok(exit::SUCCESS)
}

#[cfg(test)]
mod tests {
    use std::process::ExitCode;

    use jev_config::{Environment as _, MapEnvironment, MemoryStore, Secret, SecretStore as _};

    use crate::commands::{Streams, run_with_store};
    use crate::exit;

    /// Runs `jev auth login --stdin` in process against `store`, returning the exit
    /// code and everything written to either stream.
    fn login_from_stdin(input: &str, store: &MemoryStore) -> (ExitCode, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut stdin = input.as_bytes();
        let code = run_with_store(
            ["jev", "auth", "login", "--stdin"],
            &mut out,
            &mut err,
            &mut stdin,
            &MapEnvironment::default(),
            Streams {
                stdin_is_terminal: false,
                stdout_is_terminal: false,
                stderr_is_terminal: false,
            },
            store,
            None,
        );
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&err)
        );
        (code, combined)
    }

    /// The harness itself works: an ordinary key, with the trailing newline `echo`
    /// adds, is stored. Without this the refusal below could pass for any reason.
    #[test]
    fn login_stores_a_single_line_key() {
        let store = MemoryStore::new();
        let (code, _) = login_from_stdin("sk-single-line-key\n", &store);
        assert_eq!(code, ExitCode::from(exit::SUCCESS));
        let stored = store
            .get(jev_config::OFFICIAL_ACCOUNT)
            .expect("the memory store never fails")
            .expect("the key was stored");
        assert_eq!(stored.expose(), "sk-single-line-key");
    }

    /// The resolver refuses a stored key with a line break or control character in it,
    /// so storing one would only move the failure to every later command.
    #[test]
    fn login_refuses_a_key_the_resolver_would_refuse_and_stores_nothing() {
        for input in [
            "sk-first-half\nsk-second-half\n",
            "sk-first-half\rsk-second-half",
            "sk-first-half\u{1b}[2Jsk-second-half",
            "sk-first-half\tsk-second-half",
        ] {
            let store = MemoryStore::new();
            let (code, output) = login_from_stdin(input, &store);
            assert_eq!(code, ExitCode::from(exit::USAGE), "{input:?}: {output}");
            assert!(
                store
                    .get(jev_config::OFFICIAL_ACCOUNT)
                    .expect("the memory store never fails")
                    .is_none(),
                "{input:?} was stored"
            );
            assert!(output.contains("control character"), "{output}");
            assert!(output.contains("nothing was stored"), "{output}");
            assert!(
                !output.contains("sk-first-half") && !output.contains("sk-second-half"),
                "the refusal echoed the key: {output}"
            );
        }
    }

    fn logout(
        store: &MemoryStore,
        environment: &MapEnvironment,
    ) -> (ExitCode, serde_json::Value, String) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = run_with_store(
            ["jev", "--no-config", "auth", "logout", "--output", "json"],
            &mut out,
            &mut err,
            &mut std::io::empty(),
            environment,
            Streams {
                stdin_is_terminal: false,
                stdout_is_terminal: false,
                stderr_is_terminal: false,
            },
            store,
            None,
        );
        let document = serde_json::from_slice(&out).expect("logout writes a JSON document");
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&err)
        );
        (code, document, combined)
    }

    #[test]
    fn logout_removes_only_its_stored_credential_and_reports_the_remaining_environment() {
        let store = MemoryStore::new();
        let official = Secret::new("logout-official-canary".to_owned());
        let unrelated = Secret::new("logout-unrelated-canary".to_owned());
        store
            .set(jev_config::OFFICIAL_ACCOUNT, &official)
            .expect("the memory store never fails");
        store
            .set("unrelated-account", &unrelated)
            .expect("the memory store never fails");
        let environment =
            MapEnvironment::default().with("JEV_API_KEY", "logout-environment-canary");

        let (code, document, output) = logout(&store, &environment);

        assert_eq!(code, ExitCode::from(exit::SUCCESS));
        assert_eq!(document["schema"], "jev.auth/v1");
        assert_eq!(document["action"], "logout");
        assert_eq!(document["removed"], true);
        assert_eq!(document["still_set"], serde_json::json!(["JEV_API_KEY"]));
        assert!(
            store
                .get(jev_config::OFFICIAL_ACCOUNT)
                .expect("the memory store never fails")
                .is_none()
        );
        assert_eq!(
            store
                .get("unrelated-account")
                .expect("the memory store never fails")
                .expect("the unrelated credential remains")
                .expose(),
            unrelated.expose()
        );
        assert_eq!(
            environment.var("JEV_API_KEY").as_deref(),
            Some("logout-environment-canary")
        );
        assert!(output.contains("still set in your environment"));
        for canary in [
            official.expose(),
            unrelated.expose(),
            "logout-environment-canary",
        ] {
            assert!(!output.contains(canary), "logout disclosed a credential");
        }
    }

    #[test]
    fn logout_with_no_stored_credential_remains_a_success_on_repeated_calls() {
        let store = MemoryStore::new();
        let environment = MapEnvironment::default();
        for _ in 0..2 {
            let (code, document, output) = logout(&store, &environment);
            assert_eq!(code, ExitCode::from(exit::SUCCESS));
            assert_eq!(document["schema"], "jev.auth/v1");
            assert_eq!(document["action"], "logout");
            assert_eq!(document["removed"], false);
            assert_eq!(document["still_set"], serde_json::json!([]));
            assert!(output.contains("nothing was removed"));
            assert!(
                store
                    .get(jev_config::OFFICIAL_ACCOUNT)
                    .expect("the memory store never fails")
                    .is_none()
            );
        }
    }

    #[test]
    fn logout_reports_each_populated_official_source_and_omits_blank_values() {
        for name in [
            jev_config::API_KEY_ENV,
            jev_config::API_KEY_FILE_ENV,
            jev_config::TYPESAFE_API_KEY_ENV,
        ] {
            for value in ["logout-source-canary", " \t\n"] {
                let store = MemoryStore::new();
                let environment = MapEnvironment::default().with(name, value);
                let (code, document, output) = logout(&store, &environment);
                assert_eq!(code, ExitCode::from(exit::SUCCESS));
                let expected = if value.trim().is_empty() {
                    serde_json::json!([])
                } else {
                    serde_json::json!([name])
                };
                assert_eq!(document["still_set"], expected);
                assert_eq!(environment.var(name).as_deref(), Some(value));
                assert!(!output.contains("logout-source-canary"));
                assert_eq!(
                    output.contains("still set in your environment"),
                    !value.trim().is_empty()
                );
            }
        }
    }
}
