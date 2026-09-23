# Fuzzing

Five targets, over the five places attacker-influenced bytes enter `jev`.

| Target | Input | Also asserts |
| --- | --- | --- |
| `api_response` | bytes off the network | no out-of-range probability, no Choice absent from its own distribution, no Score off its own scale |
| `request_document` | a request file the user wrote or piped | anything accepted satisfies the documented cardinality limits |
| `gate_expression` | a `--require` expression | an unevaluable gate never reports a pass |
| `endpoint_url` | a base URL from a flag or the config file | anything accepted is TLS-protected or unambiguously loopback; normalization is idempotent |
| `state_input` | state from stdin or a file | the byte limit is respected exactly; nothing is silently truncated |

Each target asserts more than "does not panic". A fuzzer that only checks for crashes
finds crashes; these check the invariants the rest of the codebase is allowed to assume.

## Running

Requires a nightly toolchain and `cargo-fuzz`:

```sh
cargo install cargo-fuzz --locked
cargo +nightly fuzz run api_response -- -max_total_time=60
```

`scripts/fuzz-smoke.sh` runs every target briefly, which is what CI does. It is a smoke
test, not a campaign: it catches a target that stopped building or an invariant that
broke, not deep bugs. Long runs are worth doing by hand before a release.

## Corpus

`corpus/<target>/` is not committed. Seed `api_response` from the compatibility
fixtures, which are real documents:

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
