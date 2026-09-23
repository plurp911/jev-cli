//! Rendering results.
//!
//! Two representations, with different promises:
//!
//! * [`json`] is the **machine contract**. Every document carries a `schema` field,
//!   fields are only ever added, and the answer objects mirror the API's own shapes so
//!   that knowledge transfers between the TypeSafe docs, the SDKs, and this CLI.
//! * [`text`] is for humans and is explicitly **not stable**. Wording, spacing, and
//!   colour may change in any release. The documentation says so, and so does
//!   `--help`.
//!
//! Neither ever writes to stderr, and neither reaches for `println!`: both take an
//! explicit `impl io::Write`, which is what keeps stream discipline a compile-time
//! property rather than a convention.

pub mod json;
pub mod text;

/// Schema identifier of the evaluation document produced by `noul`, `choice`, `score`,
/// and `ask`.
pub const EVALUATION_SCHEMA: &str = "jev.evaluation/v1";
/// Schema identifier of the `jev models` document.
pub const MODELS_SCHEMA: &str = "jev.models/v1";
/// Schema identifier of the `jev doctor` document.
pub const DOCTOR_SCHEMA: &str = "jev.doctor/v1";
/// Schema identifier of the `jev auth status` document.
pub const AUTH_SCHEMA: &str = "jev.auth/v1";
/// Schema identifier of the `jev config` document.
pub const CONFIG_SCHEMA: &str = "jev.config/v1";
/// Schema identifier of the `--dry-run` document.
pub const DRY_RUN_SCHEMA: &str = "jev.dry-run/v1";
/// Schema identifier of one `jev map` output line.
pub const MAP_ROW_SCHEMA: &str = "jev.map.row/v1";
/// Schema identifier of the `jev map` summary line.
pub const MAP_SUMMARY_SCHEMA: &str = "jev.map.summary/v1";
/// Schema identifier of the `jev eval` report.
pub const EVAL_SCHEMA: &str = "jev.eval/v1";

/// Every schema identifier this version emits.
///
/// Exists for the test below, which asserts they are distinct and versioned, so a new
/// document cannot be added without a deliberate identifier.
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "used by the schema-stability test")
)]
pub const ALL_SCHEMAS: &[&str] = &[
    EVALUATION_SCHEMA,
    MODELS_SCHEMA,
    DOCTOR_SCHEMA,
    AUTH_SCHEMA,
    CONFIG_SCHEMA,
    DRY_RUN_SCHEMA,
    MAP_ROW_SCHEMA,
    MAP_SUMMARY_SCHEMA,
    EVAL_SCHEMA,
    crate::dataset::ROW_SCHEMA,
    crate::mcp::MCP_MAP_SCHEMA,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The two documents that promise these identifiers to users.
    ///
    /// Included at compile time so the promise is checked by the test suite rather
    /// than by a reviewer noticing. A schema added to the code and not to the
    /// documentation is a contract a consumer cannot discover; one documented and not
    /// emitted is a contract that does not exist.
    const OUTPUT_SCHEMA_DOC: &str = include_str!("../../../../docs/output-schema.md");
    const CLI_CONTRACT_DOC: &str = include_str!("../../../../docs/cli-contract.md");

    #[test]
    fn every_emitted_schema_is_documented() {
        for schema in ALL_SCHEMAS {
            assert!(
                OUTPUT_SCHEMA_DOC.contains(schema),
                "{schema} is emitted but does not appear in docs/output-schema.md"
            );
            assert!(
                CLI_CONTRACT_DOC.contains(schema),
                "{schema} is emitted but is not in the table in docs/cli-contract.md"
            );
        }
    }

    #[test]
    fn every_documented_schema_is_emitted() {
        // The reverse direction: a stale identifier left behind by a rename would
        // otherwise keep telling consumers to branch on something that never arrives.
        for document in [OUTPUT_SCHEMA_DOC, CLI_CONTRACT_DOC] {
            for mentioned in schema_identifiers_in(document) {
                assert!(
                    ALL_SCHEMAS.contains(&mentioned.as_str()),
                    "{mentioned} is documented but no command emits it"
                );
            }
        }
    }

    /// Every `jev.<name>/v<n>` identifier mentioned in a document.
    ///
    /// Deliberately a hand-rolled scan rather than a regex dependency: the shape is
    /// fixed and this is test-only code.
    fn schema_identifiers_in(document: &str) -> Vec<String> {
        let mut found = Vec::new();
        for (start, _) in document.match_indices("jev.") {
            let rest = &document[start..];
            let end = rest
                .find(|character: char| {
                    !(character.is_ascii_alphanumeric() || character == '.' || character == '/')
                })
                .unwrap_or(rest.len());
            let candidate = rest[..end].trim_end_matches('.');
            // `jev.evaluation/v1` qualifies; `jev.rs` and prose like `jev.` do not.
            if let Some((_, version)) = candidate.split_once("/v")
                && !version.is_empty()
                && version.chars().all(|character| character.is_ascii_digit())
            {
                found.push(candidate.to_owned());
            }
        }
        found
    }

    #[test]
    fn schema_identifiers_are_distinct_and_versioned() {
        let mut seen: Vec<&str> = ALL_SCHEMAS.to_vec();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(
            seen.len(),
            before,
            "two documents share a schema identifier"
        );

        for schema in ALL_SCHEMAS {
            assert!(schema.starts_with("jev."), "{schema} is not namespaced");
            assert!(
                schema.contains("/v"),
                "{schema} carries no version; a consumer cannot detect a break"
            );
        }
    }
}
