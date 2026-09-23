#!/usr/bin/env bash
# Build every fuzz target and run each one briefly.
#
# This is a smoke test, not a campaign. It catches a target that stopped building, an
# invariant that broke, and a crash reachable in seconds. It does not replace a long run
# before a release; see `fuzz/README.md`.
#
# Usage:
#   scripts/fuzz-smoke.sh            # 10 seconds per target
#   scripts/fuzz-smoke.sh 120        # 120 seconds per target
set -Eeuo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

SECONDS_PER_TARGET="${1:-10}"

if ! command -v cargo-fuzz >/dev/null 2>&1; then
  printf 'cargo-fuzz is not installed.\n  cargo install cargo-fuzz --locked\n' >&2
  exit 127
fi
if ! rustup toolchain list 2>/dev/null | grep -q '^nightly'; then
  printf 'a nightly toolchain is required.\n  rustup toolchain install nightly\n' >&2
  exit 127
fi

TARGETS=(api_response request_document gate_expression endpoint_url state_input eval_dataset)

# Seeding, so a short run starts from interesting inputs instead of from random bytes.
# Two sources, both idempotent:
#
#   * `fuzz/seeds/<target>/` -- small, hand-written, and committed. It is what makes a
#     30-second CI run reach a real parse rather than spend its budget discovering that
#     input is JSON, and it is where an input that once found a bug is kept.
#   * the compatibility fixtures, which are real API documents.
#
# The working corpus under `fuzz/corpus/` is regenerable and is not committed.
for target in "${TARGETS[@]}"; do
  mkdir -p "fuzz/corpus/$target"
  if [ -d "fuzz/seeds/$target" ]; then
    cp -f "fuzz/seeds/$target"/* "fuzz/corpus/$target/" 2>/dev/null || true
  fi
done
cp -f crates/jev-client/tests/fixtures/response-*.json fuzz/corpus/api_response/ 2>/dev/null || true

# The target is pinned explicitly. `cargo fuzz` picks its own default, and on a GitHub
# runner it chose `x86_64-unknown-linux-musl`, whose standard library is not installed
# -- so every target failed to build with E0463 while the same command succeeded
# locally. Naming the host target makes the two agree.
FUZZ_TARGET="$(rustc -vV | awk '/^host:/ {print $2}')"
printf 'building %d fuzz targets for %s\n' "${#TARGETS[@]}" "$FUZZ_TARGET"
cargo +nightly fuzz build --target "$FUZZ_TARGET"

failed=0
for target in "${TARGETS[@]}"; do
  printf '\n==> %s (%ss)\n' "$target" "$SECONDS_PER_TARGET"
  if ! cargo +nightly fuzz run --target "$FUZZ_TARGET" "$target" -- \
      -max_total_time="$SECONDS_PER_TARGET" -print_final_stats=1; then
    printf '!! %s found a failure; the input is in fuzz/artifacts/%s/\n' "$target" "$target" >&2
    failed=1
  fi
done

if [ "$failed" -ne 0 ]; then
  printf '\nA fuzz target failed. Reproduce it, turn it into a unit test next to the\n'
  printf 'parser it found, and fix the cause. Do not delete the target.\n' >&2
  exit 1
fi
printf '\nAll fuzz targets ran clean for %ss each.\n' "$SECONDS_PER_TARGET"
