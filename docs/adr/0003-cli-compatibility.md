# ADR-0003: The command line is the product; the crates promise nothing

* Status: Accepted
* Date: 2026-09-19

## Context

`jev` has two audiences with opposite needs. Humans want output that improves — better
errors, better formatting, colour. Machines want output that never changes; a script
written today must work in two years.

There is a second, subtler question. A Rust workspace published to crates.io implicitly
offers a library API, and once someone depends on it, every internal refactor becomes a
semver event. For a project whose internals will change substantially as the API client
is built, that is a large and unnecessary commitment.

## Options considered

1. **Everything stable.** Rejected: freezes human-facing text forever, so errors and
   help can never improve.
2. **Nothing stable.** Rejected: unusable for the CI, pipeline, and agent use cases that
   motivate the project.
3. **Split the contract by stream and format, and decline to promise a library API.**
   Chosen.

## Decision

**The command-line interface is the supported interface. The Rust crates are not.**

All crates are `publish = false`. If a library is ever wanted, it will be a deliberate
decision with its own ADR, its own API review, and `cargo-semver-checks` in local
verification — not an accident of having published a workspace.

### Stable

Breaking any of these requires a major version and a `CHANGELOG.md` entry:

- Command and subcommand names; flag names, short forms, and meanings.
- Exit code meanings (`crates/jev-cli/src/exit.rs`, locked by a test).
- Which stream each kind of output goes to.
- `--output json` shapes, including the `schema` field value.
- Environment variable names.
- `CredentialSource` string identifiers.

### Not stable

- All human-readable output: help text, error wording, colour, ordering, spacing.
  Documented as not parseable, in `docs/cli-contract.md` and in the help text.
- Everything inside the Rust crates.
- Timing and performance.

### Rules that follow

- **Data on stdout, everything else on stderr.** No exceptions. `--help` is data
  (exit `0`, stdout); a usage *error* is not (exit `2`, stderr).
- **Broken pipe is success.** `jev ... | head` exits `0`.
- **Every JSON document carries a `schema` field**, such as `"jev.doctor/v1"`. A
  breaking change to a document's shape increments the version inside that string rather
  than silently changing the document.
- **Adding a JSON field is compatible; removing or renaming one is not.** Consumers are
  documented to read by field name and tolerate unknown fields.
- **Adding a subcommand or an optional flag is compatible; changing a default is not.**
- **The `doctor` JSON document is written by hand**, not derived from a struct, so that
  a refactor cannot change the wire shape as a side effect.

## Consequences

- Human-facing output can be improved freely, which is the point.
- Machine consumers must use `--output json`. The documentation says so plainly, and the
  text format's instability is stated rather than implied.
- Two output paths must be maintained and tested.
- `cargo-semver-checks` is deliberately **not** in local verification today: with
  `publish = false` and no promised API, it would check a contract that does not exist.
  It becomes required the moment a crate is published.

## Revisit if

- A library API is genuinely wanted by users, not merely available.
- A streaming (JSONL) mode is added, which will need its own framing contract.
