//! `jev doctor` — what `jev` sees, and where it would send it.
//!
//! # Offline by default
//!
//! `jev doctor` makes **no** network request unless `--live` is given, and it says
//! which mode it ran in. A diagnostic command that quietly spends tokens is a bad
//! diagnostic command.
//!
//! # No credential ever appears
//!
//! It reports *which source* a credential would come from and whether each source is
//! populated. It never reads a value into anything that is printed, and the JSON
//! document is built field by field rather than derived from a struct, so a refactor
//! cannot add a field that carries one.

use jev_client::{Client, Transport};
use jev_config::CredentialSource;
use serde_json::{Value, json};
use std::fmt::Write as _;

use crate::cli::DoctorArgs;
use crate::commands::{Session, http_client};
use crate::errors::Result;
use crate::exit;
use crate::output::Safe;
use crate::render::{DOCTOR_SCHEMA, json as render_json};

/// Runs `jev doctor`.
pub(crate) fn run(
    session: &mut Session<'_>,
    args: &DoctorArgs,
    transport: Option<&(dyn Transport + Send + Sync)>,
) -> Result<u8> {
    // "Every invocation" means every invocation, including the diagnostic ones.
    session.warn_about_endpoint();

    let credentials =
        crate::commands::credentials(&session.context, session.environment, session.store);
    let availability = credentials.availability();

    let live = if args.live {
        Some(live_check(session, transport))
    } else {
        None
    };

    if session.json() {
        let document = doctor_document(session, &availability, live.as_ref());
        render_json::write_document(session.out, &document)?;
    } else {
        let rendered = doctor_text(session, &availability, live.as_ref());
        render_json::write_all(session.out, rendered.as_bytes())?;
    }

    // Exit 0 even when unhealthy, including a failed `--live` check: `doctor` succeeded
    // at diagnosing. A CI job that wants a hard check should read the JSON, or run
    // `jev models`, which exits non-zero when the call fails.
    Ok(exit::SUCCESS)
}

/// The result of the optional live probe.
struct LiveCheck {
    ok: bool,
    detail: String,
    attempts: u32,
    milliseconds: u128,
}

/// The cheapest call that proves the credential works.
///
/// `GET /v1/models` rather than an evaluation: it authenticates, it confirms the
/// endpoint is really the API, and it consumes no model tokens.
fn live_check(
    session: &mut Session<'_>,
    transport: Option<&(dyn Transport + Send + Sync)>,
) -> LiveCheck {
    let credential = match session.credential() {
        Ok((credential, _)) => credential,
        Err(error) => {
            return LiveCheck {
                ok: false,
                detail: error.to_string(),
                attempts: 0,
                milliseconds: 0,
            };
        }
    };

    let (result, stats) = match transport {
        Some(transport) => Client::with_clock(
            transport,
            session.context.endpoint.value.clone(),
            crate::interrupt::InterruptibleClock::default(),
        )
        .with_retry(session.context.retry)
        .models(&credential),
        None => http_client(
            &session.context,
            crate::interrupt::InterruptibleClock::default(),
        )
        .models(&credential),
    };

    match result {
        Ok(models) => LiveCheck {
            ok: true,
            detail: format!("{} model(s) listed", models.len()),
            attempts: stats.attempts,
            milliseconds: stats.elapsed.as_millis(),
        },
        Err(error) => LiveCheck {
            ok: false,
            detail: error.to_string(),
            attempts: stats.attempts,
            milliseconds: stats.elapsed.as_millis(),
        },
    }
}

/// Builds the JSON document by hand.
///
/// Deliberately not derived from a struct: this document is a stability contract, and
/// it should be impossible to change its shape — or to add a field carrying a secret —
/// as a side effect of a refactor.
fn doctor_document(
    session: &Session<'_>,
    availability: &jev_config::CredentialAvailability,
    live: Option<&LiveCheck>,
) -> Value {
    let sources: Vec<Value> = availability
        .sources
        .iter()
        .map(|status| {
            json!({
                "source": status.source.as_str(),
                "env_var": status.source.env_var(),
                "present": status.present,
                "set": status.set,
            })
        })
        .collect();

    json!({
        "schema": DOCTOR_SCHEMA,
        "version": env!("CARGO_PKG_VERSION"),
        "platform": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        },
        "endpoint": {
            "url": session.context.endpoint.value.to_string(),
            "provider": session.context.provider.value.as_str(),
            "cloudflare_account_id": session.context.endpoint.value.cloudflare_account_id(),
            "official": session.context.endpoint_is_official(),
            "secure": session.context.endpoint.value.is_secure(),
            "source": session.context.endpoint.from.as_str(),
        },
        "model": {
            "id": session.context.model.value.as_str(),
            "source": session.context.model.from.as_str(),
            "moving_alias": session.context.model.value.is_moving_alias(),
        },
        "config": {
            "path": session.context.config_path.as_ref().map(|path| path.display().to_string()),
            "state": session.context.config_state.as_str(),
        },
        "credentials": {
            "effective_source": availability.effective.map(CredentialSource::as_str),
            "sources": sources,
            "store": session.store.describe(),
            "store_error": availability.keychain_error,
            // Distinguishes "nothing is configured" from "something is configured and
            // broken", which `effective_source: null` alone cannot. Never a value or a
            // path; see `CredentialAvailability::resolution_error`.
            "error": availability.resolution_error,
        },
        "limits": {
            "timeout_seconds": session.context.timeout.value.as_secs(),
            "max_retries": session.context.retry.max_retries,
            "max_input_bytes": session.context.max_input_bytes.value,
        },
        "live": live.map_or_else(|| json!({"checked": false}), |check| json!({
            "checked": true,
            "ok": check.ok,
            "detail": check.detail,
            "attempts": check.attempts,
            "milliseconds": check.milliseconds,
        })),
    })
}

fn doctor_text(
    session: &Session<'_>,
    availability: &jev_config::CredentialAvailability,
    live: Option<&LiveCheck>,
) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "jev {} on {}/{}\n\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );

    out.push_str("endpoint\n");
    let _ = writeln!(
        out,
        "  provider  {}",
        session.context.provider.value.as_str()
    );
    if let Some(account) = session.context.endpoint.value.cloudflare_account_id() {
        let _ = writeln!(out, "  account   {account}");
    }
    let _ = writeln!(
        out,
        "  url       {}  ({})",
        Safe::new(&session.context.endpoint.value.to_string()),
        session.context.endpoint.from.as_str()
    );
    let _ = writeln!(
        out,
        "  official  {}",
        if session.context.endpoint_is_official() {
            "yes"
        } else {
            "NO - TypeSafe credentials are not used for this host"
        }
    );

    out.push_str("\nmodel\n");
    let _ = writeln!(
        out,
        "  id        {}  ({})",
        Safe::new(session.context.model.value.as_str()),
        session.context.model.from.as_str()
    );
    if session.context.model.value.is_moving_alias() {
        out.push_str(
            "  note      this is a moving alias; pin a version if you have calibrated a \
             threshold\n",
        );
    }

    out.push_str("\nconfiguration\n");
    match &session.context.config_path {
        Some(path) => {
            let _ = writeln!(
                out,
                "  file      {}  ({})",
                Safe::new(&path.display().to_string()),
                session.context.config_state.as_str()
            );
        }
        None => out.push_str("  file      none (no configuration directory)\n"),
    }
    let _ = writeln!(
        out,
        "  timeout   {}s\n  retries   {}\n  max input {} bytes",
        session.context.timeout.value.as_secs(),
        session.context.retry.max_retries,
        session.context.max_input_bytes.value,
    );

    out.push_str("\ncredentials\n");
    let _ = writeln!(out, "  store     {}", Safe::new(&session.store.describe()));
    if let Some(error) = &availability.keychain_error {
        let _ = writeln!(out, "  note      {}", Safe::new(error));
    }
    for status in &availability.sources {
        let label = status.source.env_var().unwrap_or(status.source.as_str());
        let state = super::auth::source_state(*status);
        let _ = writeln!(out, "  {label:<24}  {state}");
    }
    out.push_str(&effective_line(session, availability));

    out.push_str("\nlive check\n");
    match live {
        None => out.push_str("  skipped (pass --live to make one minimal API call)\n"),
        Some(check) => {
            let _ = writeln!(
                out,
                "  {}  {}  ({} attempt(s), {} ms)",
                if check.ok { "ok  " } else { "FAIL" },
                Safe::new(&check.detail),
                check.attempts,
                check.milliseconds
            );
        }
    }
    out
}

/// The `effective` line of the credentials section: the source in use, or why none is.
fn effective_line(
    session: &Session<'_>,
    availability: &jev_config::CredentialAvailability,
) -> String {
    match availability.effective {
        Some(source) => format!("  effective {}\n", source.as_str()),
        // A populated-but-broken source is not the same as an unconfigured one, and
        // suggesting `jev auth login` for a blank `JEV_API_KEY` sends the user to fix
        // the wrong thing.
        None => match &availability.resolution_error {
            Some(error) => format!("  effective none - {}\n", Safe::new(error)),
            // ADR-0008: a custom endpoint reads its own namespace and nothing else, and
            // `auth login` refuses it, so the official advice cannot work there.
            None if !session.context.endpoint_is_official() => {
                "  effective none - set JEV_CUSTOM_API_KEY or JEV_CUSTOM_API_KEY_FILE \
                 (this endpoint is not the official one)\n"
                    .to_owned()
            }
            None => "  effective none - run `jev auth login`, or set JEV_API_KEY\n".to_owned(),
        },
    }
}
