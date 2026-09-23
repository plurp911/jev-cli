#!/usr/bin/env bash
# Exercise the CLI with fake keys and fail if one reaches output or a file it writes.
# Authentication writes are excluded: running them locally could alter a real keychain.
set -Eeuo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
cargo build --workspace --locked >/dev/null

canary_dir=$(mktemp -d)
trap 'rm -r -- "$canary_dir"' EXIT
mkdir -p "$canary_dir/config" "$canary_dir/out"
export JEV_CONFIG_DIR="$canary_dir/config"
export JEV_API_KEY=sk-local-canary-do-not-print-0123456789
export JEV_CUSTOM_API_KEY=sk-local-canary-custom-do-not-print-9876543210
export TYPESAFE_API_KEY=sk-local-canary-typesafe-do-not-print-2468013579
JEV_FILE_CANARY=sk-local-canary-file-do-not-print-1357924680
JEV_CUSTOM_FILE_CANARY=sk-local-canary-custom-file-do-not-print-0864213579
printf '%s' "$JEV_FILE_CANARY" > "$canary_dir/official.key"
printf '%s' "$JEV_CUSTOM_FILE_CANARY" > "$canary_dir/custom.key"
: > "$canary_dir/blank.key"
export JEV_API_KEY_FILE="$canary_dir/official.key"
export JEV_CUSTOM_API_KEY_FILE="$canary_dir/custom.key"

canaries=(
  "$JEV_API_KEY" "$JEV_CUSTOM_API_KEY" "$TYPESAFE_API_KEY"
  "$JEV_FILE_CANARY" "$JEV_CUSTOM_FILE_CANARY"
)
failed=0

check() {
  local label=$1 output canary
  shift
  # Closing stdin prevents `mcp serve` or another reader from waiting at a prompt.
  output=$("$@" 2>&1 </dev/null || true)
  for canary in "${canaries[@]}"; do
    if [[ $output == *"$canary"* ]]; then
      printf 'credential leaked from: %s\n' "$label" >&2
      failed=1
    elif [[ $output == *"${canary:0:16}"* ]]; then
      printf 'credential prefix leaked from: %s\n' "$label" >&2
      failed=1
    fi
  done
}

check_files() {
  local canary
  for canary in "${canaries[@]}"; do
    if grep -rqF -- "$canary" "$JEV_CONFIG_DIR" "$canary_dir/out"; then
      printf 'credential reached a file written by jev\n' >&2
      failed=1
    fi
  done
}

# A Rust test derives the command set from `jev --help` and checks this list.
for args in "--help" "--version" "--bad-flag" \
  "doctor" "models" "noul" "choice" "score" "ask" "map" "eval" \
  "completions" \
  "auth" "auth login" "auth status" "auth logout" \
  "config" "config list" "config get" "config set" "config unset" \
  "config path" \
  "mcp" "mcp serve"; do
  # A local auth login or logout could change the maintainer's actual secure store.
  # Exercise their parser and help paths; Rust integration tests use a fake store for
  # the operation itself.
  case "$args" in
    "auth login"|"auth logout")
      # shellcheck disable=SC2086
      check "jev $args --help" ./target/debug/jev $args --help
      continue
      ;;
  esac
  # These are fixed, space-separated command names in this script, not user input.
  # shellcheck disable=SC2086
  check "jev $args" ./target/debug/jev $args
  # shellcheck disable=SC2086
  check "jev $args --verbose" ./target/debug/jev $args --verbose
  # shellcheck disable=SC2086
  check "jev $args --output json" ./target/debug/jev $args --output json
done

# Reach credential resolution, error paths, MCP, and file writers against loopback.
check "doctor --live" ./target/debug/jev doctor --live --endpoint http://127.0.0.1:1 --verbose
check "auth status" ./target/debug/jev auth status --output json
check "noul over http" sh -c 'printf hello | ./target/debug/jev noul "urgent?" --endpoint http://127.0.0.1:1 --retries 0 --verbose'
check "noul custom endpoint" sh -c 'printf hello | ./target/debug/jev noul "urgent?" --endpoint https://127.0.0.1:1 --retries 0 --verbose'
check "noul dry run" sh -c 'printf hello | ./target/debug/jev noul "urgent?" --dry-run --verbose'

# Resolution stops at the first populated source, so unshadow each source in turn.
check "noul via JEV_API_KEY_FILE" \
  env -u JEV_API_KEY -u TYPESAFE_API_KEY sh -c 'printf hello | ./target/debug/jev noul "urgent?" --endpoint http://127.0.0.1:1 --retries 0 --verbose'
check "noul via TYPESAFE_API_KEY" \
  env -u JEV_API_KEY -u JEV_API_KEY_FILE sh -c 'printf hello | ./target/debug/jev noul "urgent?" --endpoint http://127.0.0.1:1 --retries 0 --verbose'
check "custom via JEV_CUSTOM_API_KEY_FILE" \
  env -u JEV_CUSTOM_API_KEY sh -c 'printf hello | ./target/debug/jev noul "urgent?" --endpoint https://127.0.0.1:1 --retries 0 --verbose'
check "auth status via JEV_API_KEY_FILE" \
  env -u JEV_API_KEY -u TYPESAFE_API_KEY ./target/debug/jev auth status --output json
check "key file is a directory" \
  env -u JEV_API_KEY -u TYPESAFE_API_KEY JEV_API_KEY_FILE="$canary_dir" ./target/debug/jev auth status
check "key file is blank" \
  env -u JEV_API_KEY -u TYPESAFE_API_KEY JEV_API_KEY_FILE="$canary_dir/blank.key" ./target/debug/jev auth status

printf '%s\n' \
  '{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"canary","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"noul","arguments":{"state":"hello","instructions":"Urgent?"}}}' \
  '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"map","arguments":{"questions":[{"id":"u","type":"noul","instructions":"?"}],"records":[{"state":"a"}]}}}' \
  > "$canary_dir/mcp-session.jsonl"
# Positional parameters expand in the child shell, which reads the named fixture.
# shellcheck disable=SC2016
check "mcp serve session" sh -c './target/debug/jev mcp serve --endpoint http://127.0.0.1:1 --retries 0 --verbose < "$1"' sh "$canary_dir/mcp-session.jsonl"
# shellcheck disable=SC2016
check "mcp serve over https" sh -c './target/debug/jev mcp serve --endpoint https://127.0.0.1:1 --retries 0 --verbose < "$1"' sh "$canary_dir/mcp-session.jsonl"

printf '%s' '{"urgent":{"type":"noul","instructions":"Urgent?"}}' > "$canary_dir/questions.json"
# shellcheck disable=SC2016
check "map over http" sh -c 'printf "\"hello\"\n" | ./target/debug/jev map -r "$1" --output-file "$2" --endpoint http://127.0.0.1:1 --retries 0 --verbose' sh "$canary_dir/questions.json" "$canary_dir/out/rows.jsonl"
# shellcheck disable=SC2016
check "map dry run" sh -c 'printf "\"hello\"\n" | ./target/debug/jev map -r "$1" --dry-run --verbose' sh "$canary_dir/questions.json"
printf '%s\n' '{"schema":"jev.eval.row/v1","id":"1","state":"hello","labels":{"urgent":true}}' > "$canary_dir/dataset.jsonl"
check "eval over http" ./target/debug/jev eval -r "$canary_dir/questions.json" -d "$canary_dir/dataset.jsonl" --report "$canary_dir/out/report.json" --endpoint http://127.0.0.1:1 --retries 0 --verbose
check "eval dry run" ./target/debug/jev eval -r "$canary_dir/questions.json" -d "$canary_dir/dataset.jsonl" --dry-run --verbose
check "config set" ./target/debug/jev config set model jev-latest

# A positive control prevents the file scan from passing when commands fail early.
if [ ! -s "$canary_dir/out/rows.jsonl" ] \
  || [ ! -s "$canary_dir/out/report.json" ] \
  || [ ! -f "$JEV_CONFIG_DIR/config.toml" ]; then
  printf 'the canary wrote no files to scan\n' >&2
  failed=1
fi
check_files
exit "$failed"
