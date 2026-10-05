# Fuzzing

Six targets, over six places attacker-influenced bytes enter `jev`.

| Target | Input | Also asserts |
| --- | --- | --- |
| `api_response` | bytes off the network | no out-of-range probability, no Choice absent from its own distribution, no Score off its own scale |
| `request_document` | a request file or raw image headers | accepted questions satisfy cardinality limits; accepted images respect byte/pixel bounds and round-trip through canonical base64 |
| `gate_expression` | a `--require` expression | an unevaluable gate never reports a pass |
| `endpoint_url` | a base URL from a flag or the config file | anything accepted is TLS-protected or unambiguously loopback; normalization is idempotent |
| `state_input` | state from stdin or a file | the byte limit is respected exactly; nothing is silently truncated |
| `eval_dataset` | a labelled JSONL dataset | each label is valid for its question and every row id is unique |

Each target asserts more than "does not panic". A fuzzer that only checks for crashes
finds crashes; these check the invariants the rest of the codebase is allowed to assume.

## Running

Requires a nightly toolchain and `cargo-fuzz`:

```sh
cargo install cargo-fuzz --locked
cargo +nightly fuzz run api_response -- -max_total_time=60
```

`scripts/fuzz-smoke.sh` runs every target briefly. The full local verifier runs it when
`cargo-fuzz` and nightly are available; `scripts/verify.sh --push` requires the local
tools (ADR-0013). It is a smoke test, not a campaign: it catches a target that stopped
building or an invariant that broke, not deep bugs. Long runs are worth doing by hand
before a release.

## Corpus

`corpus/<target>/` is not committed. The smoke script seeds every target from the
committed `seeds/<target>/` fixtures and seeds `api_response` from the compatibility
fixtures, which are real documents. To seed that target manually:

```sh
mkdir -p fuzz/corpus/api_response
cp crates/jev-client/tests/fixtures/response-*.json fuzz/corpus/api_response/
```

## Why this is a separate workspace

`libfuzzer-sys` brings `unsafe`, which the main workspace forbids, and `cargo-fuzz`
builds with sanitizer instrumentation on nightly. Keeping it out of the workspace means
an ordinary `cargo build` is unaffected by either.

## A finding

A crash file lands in `fuzz/artifacts/<target>/`. Reproduce it with:

```sh
cargo +nightly fuzz run <target> fuzz/artifacts/<target>/<file>
```

Then turn it into a unit test in the crate that owns the parser, fix the cause, and
keep the test. The regression belongs next to the code, not in the corpus.
