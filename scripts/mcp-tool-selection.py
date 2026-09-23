#!/usr/bin/env python3
"""Which tool does a real agent pick when the `jev` MCP tools are connected?

Runs each case in `evals/mcp/tool-selection.json` through `claude -p` with only the
`jev` server connected (`--strict-mcp-config`), no user or plugin settings
(`--setting-sources project`, in an empty temporary directory), Claude Opus 5.5 at low
effort, and a loopback mock standing in for the TypeSafe API. It grades on the tool
calls the agent actually attempted, deterministically: no model grades another.

Not part of `scripts/verify.sh`: every case is a billed model call on the caller's own
Claude credential. It sends nothing to TypeSafe.

Usage: scripts/mcp-tool-selection.py [REPETITIONS] [CASE,CASE...]
"""

MOCK = r'''
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
'''

import json, os, subprocess, sys, tempfile, pathlib, concurrent.futures, time, random
ROOT = pathlib.Path(__file__).resolve().parent.parent
subprocess.run(["cargo", "build", "--quiet", "--locked", "--bin", "jev"], cwd=ROOT, check=True)
JEV = str(ROOT / "target/debug/jev")
HERE = pathlib.Path(tempfile.mkdtemp(prefix="jev-tool-selection-"))
(HERE / "mock.py").write_text(MOCK)
mock = subprocess.Popen([sys.executable, str(HERE/"mock.py")], stdout=subprocess.PIPE, text=True)
port = int(mock.stdout.readline())
config = {"mcpServers": {"jev": {"type": "stdio", "command": JEV,
          "args": ["--endpoint", f"http://127.0.0.1:{port}", "mcp", "serve"],
          "env": {"JEV_CUSTOM_API_KEY": "toolsel-dummy", "JEV_NO_KEYCHAIN": "1", "JEV_CONFIG_DIR": "/nonexistent"}}}}
(HERE/"mcp.json").write_text(json.dumps(config))
cases = json.loads((ROOT/"evals/mcp/tool-selection.json").read_text())
if len(sys.argv) > 2: cases = [c for c in cases if c["id"] in sys.argv[2].split(",")]
reps = int(sys.argv[1]) if len(sys.argv) > 1 else 1
random.seed(7)
def fixture(kind, d):
    if kind == "alerts":
        lines = []
        for i in range(80):
            lines.append(random.choice([
                f"[{i}] CRITICAL db-primary replication lag 45s and rising",
                f"[{i}] INFO nightly backup completed in 12m",
                f"[{i}] WARN disk /var 81% on log-3",
                f"[{i}] CRITICAL checkout-api 5xx rate 18% for 10m",
                f"[{i}] INFO cert renewal scheduled in 29 days",
                f"[{i}] WARN pod restarts 3 in 1h on batch-worker (known flaky)"]))
        (d/"alerts.txt").write_text("\n".join(lines) + "\n")
    elif kind == "fib":
        (d/"fib.py").write_text("def fib(n):\n    if n < 2:\n        return n\n    return fib(n - 1) + fib(n - 2)\n")
    elif kind == "foobar":
        (d/"a.py").write_text("x = FooBar()\n# FooBar is legacy\n")
        (d/"b.md").write_text("Use FooBarBaz, not FooBar.\n")
    elif kind == "eval":
        (d/"question.json").write_text(json.dumps({"questions": {"urgent": {"type": "noul", "instructions": "Is this urgent?"}}}))
        (d/"labelled.jsonl").write_text("\n".join(json.dumps({"schema": "jev.eval.row/v1", "id": str(i), "state": f"msg {i}", "labels": {"urgent": i % 2 == 0}}) for i in range(40)) + "\n")
def run(case, rep):
    d = pathlib.Path(tempfile.mkdtemp(prefix=f"toolsel-{case['id']}-"))
    if case.get("fixture"): fixture(case["fixture"], d)
    allowed = "mcp__jev__noul mcp__jev__choice mcp__jev__score mcp__jev__ask mcp__jev__map Read Grep Glob"
    cmd = ["claude", "-p", "--model", "claude-opus-5-5", "--effort", "low",
           "--strict-mcp-config", "--mcp-config", str(HERE/"mcp.json"),
           "--setting-sources", "project", "--no-session-persistence",
           "--output-format", "stream-json", "--verbose",
           "--allowedTools", allowed, "--max-turns", "6", case["prompt"]]
    env = dict(os.environ); env.pop("TYPESAFE_API_KEY", None)
    t = time.time()
    p = subprocess.run(cmd, cwd=d, capture_output=True, text=True, env=env, timeout=600)
    tools, mcp_status, cost, inputs = [], None, None, []
    for line in p.stdout.splitlines():
        try: ev = json.loads(line)
        except Exception: continue
        if ev.get("type") == "system" and ev.get("subtype") == "init":
            mcp_status = [(s.get("name"), s.get("status")) for s in ev.get("mcp_servers", [])]
        if ev.get("type") == "assistant":
            for block in ev.get("message", {}).get("content", []):
                if block.get("type") == "tool_use":
                    name = block["name"]; inp = block.get("input", {})
                    tools.append(name if name != "Bash" else "Bash:" + inp.get("command", "")[:80])
                    if name.startswith("mcp__jev__"): inputs.append({name: inp})
        if ev.get("type") == "result": cost = ev.get("total_cost_usd")
    jev = [t_.split("mcp__jev__")[1] for t_ in tools if t_.startswith("mcp__jev__")]
    exp = case["expect"]
    if exp == "none": ok = not jev
    elif exp == "cli-eval": ok = not jev and any(t_.startswith("Bash:") and "jev eval" in t_ for t_ in tools)
    else: ok = bool(jev) and jev[0] == exp and set(jev) <= {exp} | ({"noul","choice","score"} if exp=="ask" else set())
    return {"case": case["id"], "rep": rep, "expect": exp, "ok": ok, "tools": tools, "mcp": mcp_status, "cost": cost, "inputs": inputs, "secs": round(time.time()-t)}
jobs = [(c, r) for r in range(reps) for c in cases]
with concurrent.futures.ThreadPoolExecutor(5) as pool:
    results = list(pool.map(lambda job: run(*job), jobs))
mock.kill()
for r in results: print(json.dumps(r))
print("PASS", sum(r["ok"] for r in results), "/", len(results), "cost", round(sum(r["cost"] or 0 for r in results), 3))
