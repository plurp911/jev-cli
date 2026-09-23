//! `jev models` — what this account may send in the `model` field.
//!
//! The list comes from `GET /v1/models` every time. `jev` deliberately keeps **no**
//! hard-coded model catalogue: the set changes without a release on our side, and the
//! official documentation is explicit that versioned identifiers such as `jev-1.13.0`
//! are accepted whether or not they appear in the listing
//! (<https://docs.typesafe.ai/models>). A baked-in list would be wrong within weeks and
//! would reject valid input.

use jev_client::{Client, Transport};
use serde_json::json;
use std::fmt::Write as _;

use crate::commands::{Session, http_client};
use crate::errors::Result;
use crate::exit;
use crate::output::Safe;
use crate::render::{MODELS_SCHEMA, json as render_json};

/// Runs `jev models`.
pub(crate) fn run(
    session: &mut Session<'_>,
    transport: Option<&(dyn Transport + Send + Sync)>,
) -> Result<u8> {
    session.warn_about_endpoint();
    let (credential, source) = session.credential()?;
    session.note(&format!("credential source: {source}"));

    let (result, stats) = match transport {
        Some(transport) => {
            let client = Client::with_clock(
                transport,
                session.context.endpoint.value.clone(),
                crate::interrupt::InterruptibleClock::default(),
            )
            .with_retry(session.context.retry);
            client.models(&credential)
        }
        None => http_client(
            &session.context,
            crate::interrupt::InterruptibleClock::default(),
        )
        .models(&credential),
    };
    session.note(&format!(
        "{} attempt(s) in {} ms",
        stats.attempts,
        stats.elapsed.as_millis()
    ));
    let models = result?;

    if session.json() {
        let document = json!({
            "schema": MODELS_SCHEMA,
            "endpoint": session.context.endpoint.value.to_string(),
            "models": models.iter().map(|model| json!({
                "name": model.name,
                "description": model.description,
                "release_date": model.release_date,
            })).collect::<Vec<_>>(),
        });
        render_json::write_document(session.out, &document)?;
    } else {
        // Bounded, like the distribution renderer's label column. The name is
        // API-supplied, and an unbounded width would be an allocation of
        // `rows × width` driven by a hostile response. `jev-client` also clips the
        // field itself; this is the second bound.
        let width = models
            .iter()
            .map(|model| model.name.chars().count())
            .max()
            .unwrap_or(0)
            .min(40);
        let mut rendered = String::new();
        for model in &models {
            // Every field here is API-supplied and reaches a terminal.
            let _ = writeln!(
                rendered,
                "{:<width$}  {}  {}",
                Safe::new(&model.name),
                Safe::new(&model.release_date),
                Safe::new(&model.description),
            );
        }
        render_json::write_all(session.out, rendered.as_bytes())?;
    }
    Ok(exit::SUCCESS)
}
