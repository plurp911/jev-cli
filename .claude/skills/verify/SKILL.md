---
name: verify
description: >
  Run the repository verification suite and diagnose what fails. Use before claiming any
  work is complete, before a push, or when a check
  is failing and the tempting fix is to silence it. Covers formatting, Clippy, tests,
  doctests, docs, lockfile, dependency policy, spelling, workflow audit, and the release
  build.
allowed-tools: Bash Read Edit Grep Glob
---

# Verify

`scripts/verify.sh` is the local verification suite. The tracked pre-push hook runs its
`--push` mode and refuses to pass when a required tool is missing. GitHub does not run
automatic CI for this repository; see ADR-0013.

## Run it

```sh
scripts/verify.sh
```

Before a push, install the hook once with `scripts/install-hooks.sh` and run
`scripts/verify.sh --push`. Git does not install tracked hooks in other clones.

`--fast` (fmt, clippy, tests, docs) is for the inner loop only. **It is not sufficient
for a completion claim.** `--list` shows what would run and what tooling is missing.

## Read the summary

Three outcomes, and they are not interchangeable:

- `ok` — passed.
- `skip` — **the check did not run.** There is no remote CI backstop. Push mode fails
  when a required local tool is missing; deliberately optional checks remain listed
  as skipped. Repeat every `skip` line when reporting.
- `FAIL` — fix the cause.

Install what is missing from **outside** the repository directory, because
`rust-toolchain.toml` pins an older toolchain inside it and `cargo install` will honour
that pin:

```sh
(cd /tmp && cargo install --locked cargo-nextest cargo-deny typos-cli)
uv tool install zizmor
```

## Diagnosing each failure

### `format`

```sh
cargo fmt --all           # fixes it
```
Never edit `rustfmt.toml` to make a diff pass.

### `clippy`

Read the lint name. The workspace denies a specific set on purpose:

| Lint | What it is telling you |
| --- | --- |
| `unwrap_used`, `expect_used`, `panic` | Return a `Result`. Outside tests, a panic on user input is a bug. |
| `indexing_slicing` | Use `.get()`. Input-shaped panics are a denial-of-service path. |
| `print_stdout`, `print_stderr` | Take an `impl io::Write` parameter instead. This is what makes output testable. |
| `integer_division` | Make the truncation explicit and intentional. |
| `doc_markdown` | Backtick the identifier — or, for a proper noun, add it to `doc-valid-idents` in `clippy.toml`. |
| `missing_docs` | Write the doc comment. Say *why*, not what. |

**Do not add an `#[allow]` to clear a lint.** The one legitimate pattern in this
repository is a file-level allow with a `reason` in an integration test, because
Clippy's `allow-*-in-tests` settings do not reach `tests/`. If you believe a denial is
genuinely wrong, change it in `Cargo.toml` as its own reviewed decision and say so
explicitly — `Cargo.toml` is CODEOWNERS-protected for this reason.

### `tests` / `doctests`

Run one test:

```sh
cargo nextest run -p jev-core probability
cargo test -p jev-core --doc
```

Then stop and think about which of these it is:

1. **The code is wrong.** Fix the code.
2. **The test encodes a stale expectation.** Fix the test, and say in the PR that you
   changed a test and why.
3. **The test is flaky.** That is a bug in the test. Nextest retries are pinned to zero
   and must stay there. Find the non-determinism: a sleep, wall-clock time, ordering,
   the environment, or a leaked `JEV_API_KEY`.

**Never** delete a test, add `#[ignore]`, or loosen an assertion to get to green. If a
canary test fails — one asserting a fake secret does *not* appear in output — you have
found a credential leak. Treat it as a security incident, not a test failure.

A `proptest` failure writes a seed to `proptest-regressions/`. **Commit it.** Reproduce
with the `cc` line the failure printed.

### Clef bridge, media, and provenance

Full verification includes the offline provider/media process suite and the bridge,
live-harness, synthetic-quality, execution-provenance, runtime-profile, model-integrity,
and source-snapshot tests. These tests download no weights and call no provider.
A passing fake-inference test is not a live inference result.

The real decoder/processor checks use `JEV_CLEF_PYTHON` when explicitly set; otherwise
normal mode reports unavailable Pillow/Transformers as skips and push mode fails.
Prepare the interpreter through `docs/development/clef-live-testing.md` instead of
removing the check. Reproduce with that interpreter and:

```sh
"${JEV_CLEF_PYTHON:-python3}" scripts/test-clef-server.py --real-processor --real-pillow
```

These checks use synthetic media without loading model weights. Paid/real inference
and model downloads remain separately authorized operations, outside verification.
Report actual processor coverage separately from a skipped dependency check.

### `docs`

`RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features`. Usually a
broken intra-doc link or a missing doc on a public item.

### `lockfile is current`

`Cargo.toml` and `Cargo.lock` disagree. Run `cargo check` to refresh, and commit the
lockfile. If a dependency changed and you did not intend it, find out why.

### `dependency policy` (cargo-deny)

| Check | Meaning | Response |
| --- | --- | --- |
| `advisories` | A dependency has a RustSec advisory | Update it. Do not add it to `ignore`. |
| `licenses` | A licence is not on the allow-list | Find another crate. Adding a licence is a security review. |
| `bans` | A banned crate or unapproved runtime edge entered the tree (`openssl`, `tokio`, `libloading`) | Find out which dependency pulled it in: `cargo tree -i <crate>`. Removing a ban needs an ADR; existing reviewed `tokio` wrapper exceptions are confined to MCP (ADR-0012). |
| `sources` | An unapproved registry or Git dependency reached the release graph | Not allowed. |

### `spelling` (typos)

Real typo: fix it. False positive on a domain term such as `noul`: add it to
`typos.toml` under `extend-words`.

### `workflow security` (zizmor)

Any finding fails. Note the scope: the audit covers **all of `.github`**, not just
`workflows/`, because `dependabot.yml` carries findings of its own. Reproduce with
`zizmor --persona=pedantic .github`.

The three you will hit:

- **template-injection** — pass the value through `env:` instead of expanding
  `${{ ... }}` inside a `run:` body.
- **undocumented-permissions** — add a trailing comment to each `permissions:` entry
  saying why it is needed.
- **dependabot-cooldown** — raise the `cooldown` days in `.github/dependabot.yml`.
  Do not remove the cooldown; it is a supply-chain control, not style.

### `release build`

Catches breakage that only appears under the `release` profile: `panic = "abort"`, thin
LTO, `strip = "symbols"`.

## Reporting

State the command you ran and what it printed. Repeat the `skip` lines. If something
failed and you could not fix it, say so plainly rather than narrowing the claim to the
part that passed. "Should work" and "tests will pass" are not verification.
