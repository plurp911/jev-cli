#!/usr/bin/env bash
# The repository verification suite.
#
# This is the single definition of local verification. The pre-push hook uses --push,
# which refuses to pass when a required local tool is missing.
#
# Usage:
#   scripts/verify.sh            # everything available locally
#   scripts/verify.sh --fast     # fmt + clippy + tests + docs
#   scripts/verify.sh --push     # full suite; required tools may not be skipped
#   scripts/verify.sh --list     # show what would run, and what is missing
#
# Exit status is non-zero if any selected check fails. Checks whose tool is not
# installed are reported as SKIPPED, never silently passed.
set -Eeuo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

MODE="full"
case "${1:-}" in
  --fast) MODE="fast" ;;
  --push) MODE="push" ;;
  --list) MODE="list" ;;
  --help|-h) sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
  "") ;;
  *) printf 'unknown argument: %s\n' "$1" >&2; exit 2 ;;
esac

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  BOLD=$'\033[1m'; RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; OFF=$'\033[0m'
else
  BOLD=""; RED=""; GREEN=""; YELLOW=""; OFF=""
fi

FAILED=()
SKIPPED=()
PASSED=()

have() { command -v "$1" >/dev/null 2>&1; }

# run <name> <install-hint> <command...>
run() {
  local name="$1" hint="$2"; shift 2
  if [ "$MODE" = "list" ]; then
    if have "$1" || [ "$1" = "cargo" ]; then printf '  %s\n' "$name"; else
      printf '  %s %s(missing: %s)%s\n' "$name" "$YELLOW" "$hint" "$OFF"; fi
    return 0
  fi
  printf '%s==> %s%s\n' "$BOLD" "$name" "$OFF"
  if "$@"; then
    PASSED+=("$name")
  else
    printf '%s!! %s failed%s\n' "$RED" "$name" "$OFF" >&2
    FAILED+=("$name")
  fi
}

# optional <name> <tool> <install-hint> <command...>
optional() {
  local name="$1" tool="$2" hint="$3"; shift 3
  if have "$tool"; then
    run "$name" "$hint" "$@"
  else
    [ "$MODE" = "list" ] && printf '  %s %s(missing: %s)%s\n' "$name" "$YELLOW" "$hint" "$OFF"
    if [ "$MODE" = "push" ]; then
      FAILED+=("$name (missing: $tool; install: $hint)")
    else
      SKIPPED+=("$name (install: $hint)")
    fi
  fi
}

# --- Always-on gates. These must pass before any work is called complete. ----------

run "format"  "rustup component add rustfmt" \
    cargo fmt --all -- --check

run "clippy"  "rustup component add clippy" \
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

# The live API tests are `#[ignore]`d and therefore reported as ignored, not as passed.
# Saying so here keeps the summary honest: a run that skipped them has not exercised
# them, and the count should be visible rather than buried in nextest's output.
note_ignored() {
  printf '%s  note: %s live API test(s) are ignored; run them with\n' "$YELLOW" \
    "$(grep -c '^#\[ignore' crates/jev-cli/tests/live.rs 2>/dev/null || echo '?')"
  printf '        JEV_LIVE_TESTS=1 JEV_API_KEY=... cargo test -p jev-cli --test live -- --ignored%s\n' "$OFF"
}

if have cargo-nextest; then
  run "tests"  "cargo install cargo-nextest --locked" \
      cargo nextest run --workspace --all-features --locked --profile default
  # nextest deliberately does not run doctests; they are part of the contract too.
  run "doctests" "" cargo test --workspace --all-features --locked --doc
else
  # Cargo's test runner exercises the same tests when nextest is unavailable.
  run "tests"  "" cargo test --workspace --all-features --locked
fi

[ "$MODE" = "list" ] || note_ignored

run "docs"    "" env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features

if [ "$MODE" != "fast" ]; then
  # A separate process exercises redaction on command, error, and file paths.
  run "credential canary" "" scripts/credential-canary.sh
fi

if [ "$MODE" = "fast" ]; then
  :
else
  # --- Supply chain and hygiene. Normal local runs report missing tools; the
  # --- pre-push mode requires them because there is no remote CI backstop.

  check_lockfile() { cargo metadata --locked --format-version 1 --quiet >/dev/null; }
  run "lockfile is current" "" check_lockfile

  # `os-keychain` is a separable feature (crates/jev-config/Cargo.toml) so a build for
  # an environment with no secure storage -- a distroless container -- can omit the
  # dependency. Every other check here passes --all-features, so the
  # `#[cfg(not(feature = "os-keychain"))]` paths were compiled by nothing at all: they
  # could stop building, or start failing, and the suite would still be green. This
  # runs the lints and the tests in that configuration, which is the only way the
  # claim that the configuration is supported stays true.
  run "no-keychain lints" "" \
      cargo clippy --workspace --all-targets --no-default-features --locked -- -D warnings

  if have cargo-nextest; then
    run "no-keychain tests" "" \
        cargo nextest run --workspace --no-default-features --locked --profile default
  else
    run "no-keychain tests" "" cargo test --workspace --no-default-features --locked
  fi

  # A malformed SKILL.md fails silently at run time: the agent just never loads it.
  run "agent skills" "" python3 scripts/validate-skills.py

  # A shipped skill's helper script is published surface, and its failure mode is
  # silent: an adapter that stops matching a provider's format yields nothing, and the
  # report then says the provider was examined and no pattern was found. These tests
  # run against the fixtures the eval cases point at, so what is asserted and what is
  # evaluated cannot drift apart.
  run "shipped skill scripts" "" python3 scripts/test-skill-scripts.py

  # A second, independent opinion on the shipped skills, from the client that loads
  # them. `validate-skills.py` enforces the open Agent Skills specification and this
  # repository's policy; this catches what a Claude Code release starts rejecting. The
  # eval suite (scripts/skill-eval.sh) is deliberately NOT run here: it spends real
  # money on a real credential, so it is a decision, not a gate.
  if have claude; then
    run "shipped skill manifest" "" claude plugin validate --strict skills
  else
    SKIPPED+=("shipped skill manifest (Claude Code is an optional client)")
  fi

  # The manual release workflow must stay manual and keep its tool pins in step with
  # dist-workspace.toml.
  run "workflow consistency" "" python3 scripts/check-workflows.py

  # Command, flag, and environment variable *names* are a compatibility promise
  # (ADR-0003). A promise nothing checks is one a refactor can break quietly, so this
  # compares the clap definition against the documents that publish it.
  run "cli matches its docs" "" python3 scripts/check-cli-docs.py
  run "request schema" "" python3 scripts/check-request-schema.py
  run "cookbook gates" "" python3 scripts/check-examples.py

  # A working tree is not the repository. An ignore rule once swallowed a source file
  # that was present locally, so everything here passed and only CI -- building what was
  # actually committed -- failed.
  run "sources are committed" "" python3 scripts/check-tracked-sources.py

  optional "dependency policy" cargo-deny "cargo install cargo-deny --locked" \
      cargo deny --all-features check

  # ADR-0004's TLS ban, resolved against features rather than the lockfile.
  run "no native TLS stack" "" scripts/check-no-native-tls.sh

  optional "spelling" typos "cargo install typos-cli --locked" \
      typos

  # Lint the verification and build scripts, including the installed Git hook.
  # A quoting bug in this verification command can silently weaken other checks.
  optional "shell scripts" shellcheck "apt install shellcheck" \
      shellcheck --severity=style scripts/verify.sh scripts/bench.sh \
      scripts/fuzz-smoke.sh scripts/release-dry-run.sh scripts/check-no-native-tls.sh \
      scripts/skill-authoring-setup.sh scripts/skill-eval.sh \
      scripts/install-hooks.sh scripts/credential-canary.sh .githooks/pre-push

  # The MSRV is a promise to users. The pre-push run requires its toolchain.
  MSRV="$(python3 scripts/declared-msrv.py 2>/dev/null || true)"
  if [ -n "$MSRV" ] && rustup toolchain list 2>/dev/null | grep -q "^${MSRV}"; then
    run "MSRV ${MSRV} builds" "" env RUSTUP_TOOLCHAIN="$MSRV" \
        cargo check --workspace --all-features --locked
  else
    if [ "$MODE" = "push" ]; then
      FAILED+=("MSRV build (install: rustup toolchain install ${MSRV:-<declared>})")
    else
      SKIPPED+=("MSRV build (install: rustup toolchain install ${MSRV:-<declared>})")
    fi
  fi

  # Audit all of .github, including dependabot.yml and the manual release workflow.
  optional "workflow security" zizmor "uv tool install zizmor" \
      zizmor --persona=pedantic .github

  # A release build catches profile-only breakage (panic=abort, LTO, strip).
  run "release build" "" cargo build --workspace --locked --release

  # The parsers see hostile bytes; a smoke run catches a target that stopped building
  # or an invariant that broke. A real campaign is `scripts/fuzz-smoke.sh 300`.
  optional "fuzz smoke" cargo-fuzz "cargo install cargo-fuzz --locked" \
      scripts/fuzz-smoke.sh 10

  # A second, independent MCP client: the official Inspector, on the TypeScript SDK,
  # pinned. Development tooling only; Node is never a dependency of `jev`. Opt-in,
  # because on a cold npm cache it downloads packages whose transitive dependencies are
  # not locked, and this gate must not quietly depend on the network.
  if [ "${JEV_MCP_INSPECTOR:-0}" = "1" ]; then
    optional "mcp inspector" npx "install Node.js, which provides npx" \
        scripts/mcp-inspector.sh
  else
    SKIPPED+=("mcp inspector (network; run with JEV_MCP_INSPECTOR=1, or scripts/mcp-inspector.sh)")
  fi

  # Packaging breaks silently and is discovered at release time, when it is most
  # expensive. This builds the archive, verifies its checksum, and runs the binary out
  # of it. It publishes nothing; see ADR-0005 and ADR-0009.
  optional "release artifacts" dist "cargo install cargo-dist --locked" \
      scripts/release-dry-run.sh
fi

[ "$MODE" = "list" ] && exit 0

printf '\n%s--- summary ---%s\n' "$BOLD" "$OFF"
for item in "${PASSED[@]:-}";  do [ -n "$item" ] && printf '%s  ok      %s%s\n' "$GREEN" "$item" "$OFF"; done
for item in "${SKIPPED[@]:-}"; do [ -n "$item" ] && printf '%s  skip    %s%s\n' "$YELLOW" "$item" "$OFF"; done
for item in "${FAILED[@]:-}";  do [ -n "$item" ] && printf '%s  FAIL    %s%s\n' "$RED" "$item" "$OFF"; done

if [ "${#FAILED[@]}" -gt 0 ]; then
  printf '\n%s%d check(s) failed.%s Do not weaken a check to make this pass; fix the cause.\n' \
    "$RED" "${#FAILED[@]}" "$OFF" >&2
  exit 1
fi

if [ "${#SKIPPED[@]}" -gt 0 ]; then
  printf '\n%sSome optional checks were skipped; review the summary.%s\n' "$YELLOW" "$OFF"
fi
printf '%sAll selected checks passed.%s\n' "$GREEN" "$OFF"
