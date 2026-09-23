#!/usr/bin/env bash
# Drive `jev mcp serve` with the official MCP Inspector, as an external client.
#
# The Rust tests in `crates/jev-cli/tests/mcp.rs` use the official Rust SDK's client.
# This is a second, independent implementation -- the TypeScript SDK the Inspector is
# built on -- so a server both accept is not merely agreeing with itself.
#
# Development tooling only: Node is never a dependency of `jev`. The Inspector's own
# version is pinned; its transitive npm dependencies are not, and are fetched from npm
# on a cold cache, so a dependency release can still change what this runs. Nothing reaches
# the internet except npm, for the Inspector package itself: the backend is a loopback
# mock and the credential is a dummy.
#
# Usage:
#   scripts/mcp-inspector.sh          # builds the debug binary first
set -Eeuo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

INSPECTOR="@modelcontextprotocol/inspector@2.7.0"

if ! command -v npx >/dev/null 2>&1; then
  printf 'npx is not installed; the MCP Inspector needs Node.js.\n' >&2
  exit 127
fi

cargo build --quiet --locked --bin jev
JEV="$PWD/target/debug/jev"

WORK="$(mktemp -d)"
cleanup() {
  [ -n "${MOCK_PID:-}" ] && kill "$MOCK_PID" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

# A loopback stand-in for the API that answers every question in the request with a
# well-formed answer of the right type. It reads the request to do that, so each tool
# gets a document shaped like the real one.
cat >"$WORK/mock.py" <<'PY'
import http.server, json, sys

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["content-length"])))
        answers = {}
        for qid, q in body["questions"].items():
            if q["type"] == "noul":
                answers[qid] = {"type": "noul", "noul": 0.9}
            elif q["type"] == "choice":
                names = list(q["criteria"])
                answers[qid] = {"type": "choice", "choice": names[0], "confidence": 0.8,
                                "probabilities": {n: (1.0 if i == 0 else 0.0) for i, n in enumerate(names)}}
            else:
                levels = q["criteria"]
                answers[qid] = {"type": "score", "score": 0.0, "confidence": 0.9,
                                "legend": {str(i): l for i, l in enumerate(levels)},
                                "probabilities": {str(i): (1.0 if i == 0 else 0.0) for i in range(len(levels))}}
        reply = json.dumps({"model": "jev-mock", "answers": answers,
                            "usage": {"input_tokens": 1, "output_tokens": 1}}).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(reply)))
        self.end_headers()
        self.wfile.write(reply)

server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
print(server.server_address[1], flush=True)
server.serve_forever()
PY
python3 "$WORK/mock.py" >"$WORK/port" &
MOCK_PID=$!
for _ in $(seq 50); do [ -s "$WORK/port" ] && break; sleep 0.1; done
PORT="$(cat "$WORK/port")"

export JEV_CUSTOM_API_KEY="inspector-dummy-key"
export JEV_CONFIG_DIR="$WORK/config"
export JEV_NO_KEYCHAIN=1
unset JEV_API_KEY JEV_API_KEY_FILE TYPESAFE_API_KEY

# The Inspector takes the leading run of non-dash words as the server command, then the
# server's own flags up to `--`, then its own options.
inspect() {
  # `-e`, because the TypeScript SDK passes a spawned server only a short allow-list
  # of variables (PATH, HOME, ...), not the caller's environment.
  npx --yes "$INSPECTOR" --cli "$JEV" mcp serve --endpoint "http://127.0.0.1:$PORT" \
    -- --format json -e JEV_CUSTOM_API_KEY="$JEV_CUSTOM_API_KEY" \
    -e JEV_CONFIG_DIR="$JEV_CONFIG_DIR" -e JEV_NO_KEYCHAIN=1 "$@" 2>>"$WORK/stderr"
}

fail() { printf 'FAIL: %s\n' "$1" >&2; cat "$WORK/stderr" >&2 || true; exit 1; }

# Both eras: the legacy `initialize` handshake and the stateless 2026-07-28 revision.
for era in legacy modern; do
  inspect --protocol-era "$era" --method tools/list >"$WORK/tools-$era.json" \
    || fail "tools/list ($era)"
  python3 - "$WORK/tools-$era.json" "$era" <<'PY' || fail "tool surface ($2)"
import json, sys
doc = json.load(open(sys.argv[1]))
tools = doc.get("tools", doc.get("result", {}).get("tools"))
names = [t["name"] for t in tools]
assert names == ["noul", "choice", "score", "ask", "map"], names
for t in tools:
    assert t["inputSchema"]["type"] == "object", t["name"]
    assert t["outputSchema"]["type"] == "object", t["name"]
    assert t["annotations"]["readOnlyHint"] is True, t["name"]
print(f"ok    {sys.argv[2]}: tools/list returned the five tools with object schemas")
PY
done

# `--strict` asks the Inspector to reject schemas it considers non-portable.
: >"$WORK/stderr"
inspect --method tools/list --strict >/dev/null || fail "tools/list --strict"
if grep -q "Warning: tool" "$WORK/stderr"; then
  fail "the Inspector reports schema portability warnings"
fi
printf 'ok    tools/list passes --strict with no portability warnings\n'

call() {
  local tool="$1" args="$2"
  inspect --method tools/call --tool-name "$tool" --tool-args-json "$args" >"$WORK/$tool.json" \
    || fail "tools/call $tool"
  python3 - "$WORK/$tool.json" "$tool" <<'PY' || fail "result of $2"
import json, sys
doc = json.load(open(sys.argv[1]))
result = doc.get("result", doc)
assert not result.get("isError"), result
structured = result["structuredContent"]
assert structured["schema"] in ("jev.evaluation/v1", "jev.mcp.map/v1"), structured
text = json.loads(result["content"][0]["text"])
assert text == structured, "text and structured content differ"
print(f"ok    tools/call {sys.argv[2]} -> {structured['schema']}")
PY
}

call noul   '{"state":"payouts failing","instructions":"Urgent?"}'
call choice '{"state":"refund","instructions":"Team?","options":[{"name":"billing"},{"name":"auth"}]}'
call score  '{"state":"x","instructions":"Risk?","levels":["low","high"]}'
call ask    '{"state":"x","questions":[{"id":"a","type":"noul","instructions":"?"},{"id":"b","type":"score","instructions":"?","levels":["lo","hi"]}]}'
call map    '{"questions":[{"id":"a","type":"noul","instructions":"?"}],"records":[{"state":"one"},{"state":"two"}]}'

if grep -q "inspector-dummy-key" "$WORK"/*.json; then
  fail "the credential appeared in an Inspector result"
fi
printf 'ok    MCP Inspector %s accepted jev mcp serve\n' "${INSPECTOR##*@}"
