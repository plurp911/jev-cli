//! `jev config` — non-secret settings only.
//!
//! The configuration file cannot hold a credential. `jev config set api_key …` is
//! refused, and so is a hand-written file containing a key-shaped key, including one
//! nested inside a table. That check lives in `jev-config` and is enforced on both the
//! read and the write path, so the file cannot become a plaintext credential store by
//! either route (`docs/threat-model.md` T3).

use jev_config::{SETTING_NAMES, Settings};
use serde_json::{Value, json};

use crate::cli::ConfigCommand;
use crate::commands::Session;
use crate::errors::{CliError, Result};
use crate::exit;
use crate::output::Safe;
use crate::render::{CONFIG_SCHEMA, json as render_json};

/// Runs `jev config`.
pub(crate) fn run(session: &mut Session<'_>, command: &ConfigCommand) -> Result<u8> {
    let path = jev_config::config_path(session.environment)?;
    let display = path.display().to_string();

    match command {
        ConfigCommand::Path => show_path(session, &display),
        ConfigCommand::List => list(session, &path, &display),
        ConfigCommand::Get { key } => get(session, &path, key),
        ConfigCommand::Set { key, value } => set(session, &path, &display, key, value),
        ConfigCommand::Unset { key } => unset(session, &path, &display, key),
    }
}

fn show_path(session: &mut Session<'_>, display: &str) -> Result<u8> {
    if session.json() {
        render_json::write_document(
            session.out,
            &json!({"schema": CONFIG_SCHEMA, "action": "path", "path": display}),
        )?;
    } else {
        render_json::write_all(session.out, format!("{}\n", Safe::new(display)).as_bytes())?;
    }
    Ok(exit::SUCCESS)
}

fn list(session: &mut Session<'_>, path: &std::path::Path, display: &str) -> Result<u8> {
    let settings = Settings::load(path)?;
    if session.json() {
        let values: serde_json::Map<String, Value> = SETTING_NAMES
            .iter()
            .filter_map(|name| {
                settings
                    .get(name)
                    .map(|value| ((*name).to_owned(), json!(value)))
            })
            .collect();
        render_json::write_document(
            session.out,
            &json!({
                "schema": CONFIG_SCHEMA,
                "action": "list",
                "path": display,
                "exists": path.exists(),
                "settings": values,
            }),
        )?;
    } else {
        let rendered = settings.to_string();
        if rendered.is_empty() {
            session.warn(&format!("no settings are set in {display}"));
        }
        render_json::write_all(session.out, Safe::new(&rendered).to_string().as_bytes())?;
    }
    Ok(exit::SUCCESS)
}

fn get(session: &mut Session<'_>, path: &std::path::Path, key: &str) -> Result<u8> {
    reject_unknown(key)?;
    let settings = Settings::load(path)?;
    let Some(value) = settings.get(key) else {
        // Exit 1 for "known setting, not set", so `jev config get x` is usable in a
        // shell conditional the way `git config --get` is. In JSON mode a document is
        // still emitted: the contract promises one per invocation, and a consumer
        // should not have to special-case zero bytes before parsing.
        if session.json() {
            render_json::write_document(
                session.out,
                &json!({
                    "schema": CONFIG_SCHEMA,
                    "action": "get",
                    "key": key,
                    "set": false,
                    "value": Value::Null,
                }),
            )?;
        }
        session.warn(&format!("`{key}` is not set"));
        return Ok(exit::UNSATISFIED);
    };
    if session.json() {
        render_json::write_document(
            session.out,
            &json!({
                "schema": CONFIG_SCHEMA,
                "action": "get",
                "key": key,
                "set": true,
                "value": value,
            }),
        )?;
    } else {
        render_json::write_all(session.out, format!("{}\n", Safe::new(&value)).as_bytes())?;
    }
    Ok(exit::SUCCESS)
}

fn set(
    session: &mut Session<'_>,
    path: &std::path::Path,
    display: &str,
    key: &str,
    value: &str,
) -> Result<u8> {
    let mut settings = Settings::load(path)?;
    settings.set(key, value, display)?;
    settings.save(path)?;
    session.warn(&format!("{key} = {value}  ({display})"));
    if session.json() {
        render_json::write_document(
            session.out,
            &json!({
                "schema": CONFIG_SCHEMA,
                "action": "set",
                "key": key,
                "value": value,
                "path": display,
            }),
        )?;
    }
    Ok(exit::SUCCESS)
}

fn unset(
    session: &mut Session<'_>,
    path: &std::path::Path,
    display: &str,
    key: &str,
) -> Result<u8> {
    reject_unknown(key)?;
    let mut settings = Settings::load(path)?;
    settings.unset(key, display)?;
    settings.save(path)?;
    session.warn(&format!("unset {key}  ({display})"));
    if session.json() {
        render_json::write_document(
            session.out,
            &json!({"schema": CONFIG_SCHEMA, "action": "unset", "key": key, "path": display}),
        )?;
    }
    Ok(exit::SUCCESS)
}

/// Rejects a name that is not a setting, listing the ones that are.
fn reject_unknown(key: &str) -> Result<()> {
    if SETTING_NAMES.contains(&key) {
        return Ok(());
    }
    Err(CliError::usage(format!(
        "unknown setting `{}`\n\nValid settings: {}",
        Safe::new(key),
        SETTING_NAMES.join(", ")
    )))
}
