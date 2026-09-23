#!/usr/bin/env bash
# Measure the CLI itself.
#
# What this measures is `jev`'s own overhead: how long the process takes to start, to
# build a request, and to decode and render a response. It does **not** measure Jev's
# intelligence, its accuracy, or its latency. Those belong to TypeSafe, vary with the
# state and the question, and are not this tool's to benchmark.
#
# Everything here runs against a local mock server on loopback, so no API call is made
# and no credential is needed.
#
# Usage:
#   scripts/bench.sh            # 200 iterations per measurement
#   scripts/bench.sh 1000
set -Eeuo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

ITERATIONS="${1:-200}"
BIN=target/release/jev

printf 'building the release binary\n'
cargo build --release --locked --quiet

if [ ! -x "$BIN" ]; then
  printf 'no release binary at %s\n' "$BIN" >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"; [ -n "${SERVER_PID:-}" ] && kill "$SERVER_PID" 2>/dev/null || true' EXIT

export JEV_CONFIG_DIR="$WORK/config"
export JEV_CUSTOM_API_KEY="sk-bench-not-a-real-key"
unset JEV_API_KEY TYPESAFE_API_KEY JEV_API_KEY_FILE || true

# --- A local mock API. Python's stdlib only; nothing is installed. -------------------
cat > "$WORK/server.py" <<'PY'
import json, sys, threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

BODY = json.dumps({
    "model": "jev-1.13.0",
    "answers": {
        "urgent": {"type": "noul", "noul": 0.92},
        "team": {"type": "choice", "choice": "billing", "confidence": 0.8,
                 "probabilities": {"billing": 0.8, "technical": 0.15, "sales": 0.05}},
        "severity": {"type": "score", "score": 1.3, "confidence": 0.54,
                     "legend": {"0": "low", "1": "medium", "2": "high"},
                     "probabilities": {"0": 0.0, "1": 0.7, "2": 0.3}},
    },
    "usage": {"input_tokens": 312, "output_tokens": 48},
}).encode()
MODELS = json.dumps({"models": [
    {"name": "jev-latest", "description": "flagship", "release_date": "2026-09-15"}
]}).encode()

class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    # Without this the mock, not `jev`, dominates the measurement: Python writes the
    # headers and the body separately, and Nagle plus delayed ACK adds ~40 ms to every
    # keep-alive request. Measuring that would be measuring the harness.
    disable_nagle_algorithm = True
    def _reply(self, payload):
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)
    def do_GET(self):
        self._reply(MODELS)
    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        self.rfile.read(length)
        self._reply(BODY)
    def log_message(self, *args):
        pass

# The default listen backlog is 5. Above about eight concurrent connections the mock
# starts refusing them, `jev` correctly retries with backoff, and the measurement
# becomes a measurement of the mock's accept queue.
ThreadingHTTPServer.request_queue_size = 256
server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
print(server.server_address[1], flush=True)
server.serve_forever()
PY

python3 "$WORK/server.py" > "$WORK/port" &
SERVER_PID=$!
for _ in $(seq 1 100); do
  PORT="$(cat "$WORK/port" 2>/dev/null || true)"
  [ -n "$PORT" ] && break
  sleep 0.05
done
if [ -z "${PORT:-}" ]; then
  printf 'the mock server did not start\n' >&2
  exit 1
fi
ENDPOINT="http://127.0.0.1:$PORT"

# --- Fixtures -------------------------------------------------------------------------
printf 'Shoes arrived two weeks late and in the wrong size. Also two charges on my card.' \
  > "$WORK/state.txt"
cat > "$WORK/request.json" <<'JSON'
{
  "questions": {
    "urgent": {"type": "noul", "instructions": "Does this convey urgency?"},
    "team": {"type": "choice", "instructions": "Which team should handle this?",
             "criteria": {"billing": "Charges and invoices",
                          "technical": "Bugs and outages",
                          "sales": "Pricing and upgrades"}},
    "severity": {"type": "score", "instructions": "How severe is this?",
                 "criteria": ["low", "medium", "high"]}
  }
}
JSON
records() {
  local count="$1" path="$2" i
  : > "$path"
  for i in $(seq 1 "$count"); do
    printf '{"id":"r-%s","body":"ticket number %s, shoes arrived late and wrong size"}\n' \
      "$i" "$i" >> "$path"
  done
}
records 200 "$WORK/records.jsonl"
records 2000 "$WORK/records-2000.jsonl"
# Labelled rows for `jev eval`, carrying the three ids the mock answers. The label
# alternates so the metrics have something to distinguish; what is being timed is the
# local arithmetic and the report, not whether the answers are any good.
labelled() {
  local count="$1" path="$2" i
  : > "$path"
  for i in $(seq 1 "$count"); do
    printf '{"schema":"jev.eval.row/v1","id":"e-%s","state":"ticket number %s, shoes arrived late and wrong size","labels":{"urgent":%s,"team":"billing","severity":1}}\n' \
      "$i" "$i" "$( [ $((i % 2)) -eq 0 ] && printf true || printf false )" >> "$path"
  done
}
labelled 200 "$WORK/labelled.jsonl"
labelled 2000 "$WORK/labelled-2000.jsonl"

# The same records as plain text, for --lines: the difference between the two runs is
# the JSONL parse and the per-record field lookup, which is otherwise only visible
# inside an aggregate number.
cut -c1- "$WORK/records.jsonl" | sed 's/.*"body":"//; s/"}$//' > "$WORK/records.txt"

# --- Measurement ----------------------------------------------------------------------
# `time` over a loop rather than per-invocation: the per-call cost here is close to the
# resolution of the shell's own timing, so a loop is the honest way to measure it.
measure() {
  local label="$1" iterations="$2"; shift 2
  local start end total per
  start="$(date +%s%N)"
  for _ in $(seq 1 "$iterations"); do
    "$@" > /dev/null 2>&1 || true
  done
  end="$(date +%s%N)"
  total=$(( (end - start) / 1000000 ))
  per="$(awk "BEGIN {printf \"%.2f\", $total / $iterations}")"
  printf '  %-44s %8s ms/op   (%s iterations, %s ms total)\n' "$label" "$per" "$iterations" "$total"
}

printf '\njev benchmark\n'
printf '  binary:   %s\n' "$BIN"
printf '  version:  %s\n' "$("$BIN" --version)"
printf '  platform: %s\n' "$(uname -sm)"
printf '  endpoint: %s (local mock, no API call)\n\n' "$ENDPOINT"

# A `fork`+`exec` of anything costs something, and on a loaded machine it is a
# meaningful share of these numbers. Measuring `true` first gives a floor to subtract,
# so the figures below are not quietly reporting the shell's overhead as `jev`'s.
printf 'baseline (the cost of spawning any process at all)\n'
measure "/bin/true"                     "$ITERATIONS" /bin/true

printf '\nprocess overhead (no network)\n'
measure "startup: --version"            "$ITERATIONS" "$BIN" --version
measure "startup + arg parsing: --help" "$ITERATIONS" "$BIN" --help
measure "config + credential resolution: doctor" "$ITERATIONS" "$BIN" doctor --output json
measure "request construction: noul --dry-run" "$ITERATIONS" \
  "$BIN" noul "Does this convey urgency?" --state-file "$WORK/state.txt" --dry-run --output json
measure "request construction: ask --dry-run (3 questions)" "$ITERATIONS" \
  "$BIN" ask --questions "$WORK/request.json" --state-file "$WORK/state.txt" --dry-run --output json

printf '\nend to end against the local mock\n'
measure "noul, json output"   "$ITERATIONS" \
  "$BIN" noul "Does this convey urgency?" --state-file "$WORK/state.txt" \
  --endpoint "$ENDPOINT" --output json
measure "noul, text output"   "$ITERATIONS" \
  "$BIN" noul "Does this convey urgency?" --state-file "$WORK/state.txt" \
  --endpoint "$ENDPOINT"
measure "ask, 3 questions, json output" "$ITERATIONS" \
  "$BIN" ask --questions "$WORK/request.json" --state-file "$WORK/state.txt" \
  --endpoint "$ENDPOINT" --output json
measure "models" "$ITERATIONS" "$BIN" models --endpoint "$ENDPOINT" --output json

printf '\nbatch throughput (200 records, one process)\n'
for concurrency in 1 4 16 32; do
  measure "map -j $concurrency" 5 \
    "$BIN" map --request "$WORK/request.json" --input "$WORK/records.jsonl" \
    --state-field body --id-field id --endpoint "$ENDPOINT" -j "$concurrency"
done

# Two runs over the same content, differing only in how it is framed and where the rows
# go. These isolate what the aggregate `map -j N` number hides: the cost of parsing JSONL
# and looking up a field per record, and the cost of writing rows to a file rather than
# buffering them for stdout.
printf '\nwhere the batch time goes (200 records, -j 16)\n'
measure "JSONL in, stdout out" 5 \
  "$BIN" map --request "$WORK/request.json" --input "$WORK/records.jsonl" \
  --state-field body --id-field id --endpoint "$ENDPOINT" -j 16
measure "plain lines in, stdout out" 5 \
  "$BIN" map --request "$WORK/request.json" --input "$WORK/records.txt" \
  --lines --endpoint "$ENDPOINT" -j 16
measure "JSONL in, file out" 5 \
  sh -c "rm -f '$WORK/out.jsonl'; '$BIN' map --request '$WORK/request.json' \
    --input '$WORK/records.jsonl' --state-field body --id-field id \
    --endpoint '$ENDPOINT' -j 16 --output-file '$WORK/out.jsonl'"

# A second size, so the table shows whether cost is linear in records or has a fixed
# component that a single size cannot distinguish.
printf '\nscaling (-j 16)\n'
measure "200 records" 5 \
  "$BIN" map --request "$WORK/request.json" --input "$WORK/records.jsonl" \
  --state-field body --id-field id --endpoint "$ENDPOINT" -j 16
measure "2000 records" 3 \
  "$BIN" map --request "$WORK/request.json" --input "$WORK/records-2000.jsonl" \
  --state-field body --id-field id --endpoint "$ENDPOINT" -j 16

# `jev eval` sends one request per row like `map`, then does every metric locally: the
# sweeps, the confusion tables, the calibration bins, the kappa. Timing the same row
# count under both is what separates the local arithmetic from the transport, and
# timing two dataset sizes shows whether a sweep that is quadratic in distinct values
# has crept in.
printf '\ncalibration (jev eval, -j 16)\n'
measure "eval --dry-run, 200 rows (no network)" 5 \
  "$BIN" eval --request "$WORK/request.json" --dataset "$WORK/labelled.jsonl" \
  --dry-run --output json
measure "eval, 200 rows, no objective" 5 \
  "$BIN" eval --request "$WORK/request.json" --dataset "$WORK/labelled.jsonl" \
  --endpoint "$ENDPOINT" -j 16 --output json
measure "eval, 200 rows, maximize-f1 + split" 5 \
  "$BIN" eval --request "$WORK/request.json" --dataset "$WORK/labelled.jsonl" \
  --endpoint "$ENDPOINT" -j 16 --objective maximize-f1 --output json
measure "eval, 2000 rows, maximize-f1 + split" 3 \
  "$BIN" eval --request "$WORK/request.json" --dataset "$WORK/labelled-2000.jsonl" \
  --endpoint "$ENDPOINT" -j 16 --objective maximize-f1 --output json

printf '\nsize\n'
printf '  %-44s %8s\n' "release binary" "$(du -h "$BIN" | cut -f1)"
printf '  %-44s %8s\n' "release binary (bytes)" "$(wc -c < "$BIN")"
printf '  %-44s %8s\n' "direct runtime dependencies" \
  "$(cargo tree -p jev-cli --depth 1 -e normal --quiet --prefix none 2>/dev/null \
     | tail -n +2 | grep -cv '^jev-' || echo '?')"
printf '  %-44s %8s\n' "crates in the runtime graph" \
  "$(cargo tree -p jev-cli -e normal --quiet --prefix none 2>/dev/null \
     | awk '{print $1, $2}' | sort -u | grep -c . || echo '?')"

if command -v /usr/bin/time >/dev/null 2>&1; then
  printf '\npeak resident memory\n'
  rss="$(/usr/bin/time -f '%M' "$BIN" ask --questions "$WORK/request.json" \
      --state-file "$WORK/state.txt" --endpoint "$ENDPOINT" --output json 2>&1 >/dev/null \
      | tail -1)"
  printf '  %-44s %8s KB\n' "ask, 3 questions" "$rss"
  rss="$(/usr/bin/time -f '%M' "$BIN" map --request "$WORK/request.json" \
      --input "$WORK/records.jsonl" --state-field body --id-field id \
      --endpoint "$ENDPOINT" -j 16 2>&1 >/dev/null | tail -1)"
  printf '  %-44s %8s KB\n' "map, 200 records, -j 16" "$rss"
  # Records are read up front, so memory is expected to grow with the input. Measuring
  # both sizes is what makes that a stated property rather than an assumption.
  rss="$(/usr/bin/time -f '%M' "$BIN" map --request "$WORK/request.json" \
      --input "$WORK/records-2000.jsonl" --state-field body --id-field id \
      --endpoint "$ENDPOINT" -j 16 2>&1 >/dev/null | tail -1)"
  printf '  %-44s %8s KB\n' "map, 2000 records, -j 16" "$rss"
  # `eval` holds every row *and* every answer, because the metrics need the whole set
  # at once. That is a real difference from `map`, which discards a row once written,
  # and it is why this is measured rather than assumed.
  rss="$(/usr/bin/time -f '%M' "$BIN" eval --request "$WORK/request.json" \
      --dataset "$WORK/labelled-2000.jsonl" --endpoint "$ENDPOINT" -j 16 \
      --objective maximize-f1 --output json 2>&1 >/dev/null | tail -1)"
  printf '  %-44s %8s KB\n' "eval, 2000 rows, -j 16" "$rss"
fi

printf '\nMeasured: this CLI. Not measured: Jev itself.\n'
