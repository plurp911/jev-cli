# Contributing

Thanks for considering it. This document covers the practical mechanics; the
engineering rules live in [`AGENTS.md`](AGENTS.md), which applies to human and AI
contributors alike. Please read it before your first change.

## Setup

```sh
git clone https://github.com/plurp911/jev-cli
cd jev-cli
scripts/install-hooks.sh
cargo build --workspace
scripts/verify.sh
```

The Rust toolchain version is pinned in `rust-toolchain.toml`; `rustup` will install it
for you on first build.

Optional tools, which `scripts/verify.sh` reports as `skip` when missing:

```sh
cargo install --locked cargo-nextest cargo-deny typos-cli
uv tool install zizmor            # or: pipx install zizmor
# shellcheck comes from your package manager, e.g. apt install shellcheck
```

Install these from outside the repository directory — inside it, the pinned toolchain
applies and may be older than a tool requires.

## Verify before pushing

```sh
scripts/verify.sh --push
```

The local push gate runs formatting, Clippy with warnings denied, tests,
doctests, documentation, the credential canary, the lockfile check, agent-skill
validation, the dependency policy, spelling, a shell-script lint, an MSRV build, the
workflow security audit, and a release build. `--push` fails if a required tool is
missing. The installed Git hook runs this command before each push. Use
`scripts/verify.sh --fast` while editing and
`scripts/verify.sh` before reporting that a change is done.

Read the summary. A `skip` line means a check did not run; it does not mean the check
passed. Git hooks are local to each clone and can be bypassed. The person pushing must
confirm that verification ran and passed. GitHub does not run CI for this repository.

## Making a change

1. **Open an issue first** for anything beyond a bug fix or a typo. This project is
   deliberately minimal, and the fastest way to waste your afternoon is to implement a
   feature that is out of scope. See `AGENTS.md` §3.3 for what is permanently out.
2. **Branch from `main`.**
3. **Write the test first** where you reasonably can. A behaviour change without a test
   that fails before it will not be merged.
4. **Keep the change small.** Do not refactor while fixing. Do not reformat unrelated
   code.
5. **Update the docs in the same PR**: `CHANGELOG.md` for anything user-visible, the
   relevant ADR if you are changing a decision, and the rustdoc on anything you touched.

### Commit messages

[Conventional Commits](https://www.conventionalcommits.org/): `feat:`, `fix:`, `docs:`,
`test:`, `refactor:`, `ci:`, `deps:`, `chore:`. Breaking changes get a `!` and a
`BREAKING CHANGE:` trailer. Say *why* in the body; the diff already says what.

## Things that will get a change rejected

These are not style preferences.

- **Weakening a check to make verification green.** Deleting a test, adding `#[ignore]`,
  loosening an assertion, widening an `allow`, or adding a retry. If a
  check is genuinely wrong, change it as its own reviewed decision and say so.
- **A credential anywhere in the diff**, including tests, fixtures, comments, and commit
  messages. Use an obviously fake canary value.
- **`unsafe`.** The workspace forbids it.
- **A claim about TypeSafe API behaviour without a citation** to the official
  documentation. Recollection is not a source. See `AGENTS.md` §6.
- **An unjustified dependency.** See `AGENTS.md` §7.
- **Copied code from another project.** Read it, understand it, write your own. See
  `AGENTS.md` §8.
- **A breaking change to the CLI contract** without an explicit note and a changelog
  entry. See `docs/cli-contract.md`.

## Testing conventions

Tests must be deterministic: no network, no sleeping, no dependence on wall-clock time,
no dependence on your environment or on test ordering. Integration tests clear
`JEV_API_KEY` and `JEV_API_KEY_FILE` before invoking the binary.

Security invariants get canary tests — assert that a known fake secret does **not**
appear in the output. Follow the existing examples in `crates/jev-config/src/secret.rs`
and `crates/jev-client/src/transport.rs`.

`proptest` regression seeds under `proptest-regressions/` are committed on purpose: a
failure found once is replayed forever. Do not delete them.

## Contributing as, or with, an AI agent

Agents are welcome here; most of this repository was written by one. Two requests:

- **Say so in the pull request.** The template asks. It is for reviewer calibration,
  not judgement.
- **You are responsible for what you submit.** Read the diff, run
  `scripts/verify.sh` yourself, and check the claims. "The agent said it was fine" is
  not a review.

If you are an agent reading this: `AGENTS.md` is binding, and `CLAUDE.md` lists the
mistakes that are most common here.

## Reporting security issues

Do not open a public issue. See [`SECURITY.md`](SECURITY.md).

## Licensing

By contributing you agree that your contribution is dual-licensed under
`MIT OR Apache-2.0`, matching the project. Do not add a file under a different license,
and do not remove or alter an existing license header or attribution notice.

## Code of conduct

Participation is governed by the [Code of Conduct](CODE_OF_CONDUCT.md).
