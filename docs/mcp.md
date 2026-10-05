# `jev` as an MCP server

`jev mcp serve` runs a local [Model Context Protocol](https://modelcontextprotocol.io)
server over stdio. An agent host (Claude Code, Codex, Cursor, the Grok CLI, or any MCP
client) starts it and gets five typed tools: `noul`, `choice`, `score`, `ask`, and `map`.

The tools are not a second implementation. They call the functions `jev noul`,
`jev ask`, and `jev map` call, with the same validation, credentials, retries, and
concurrency, and they return the same JSON documents. See
[ADR-0012](adr/0012-mcp-server.md) for the design.

```text
MCP host ──stdio──▶ jev mcp serve ──▶ the jev core ──▶ selected provider
                    (protocol only)    (same code as the CLI)
```

There is no port, no daemon, and no background service. The host starts the process
and stops it. The MCP server itself needs only `jev`. Local model providers run
separately; the Hugging Face bridge also needs its explicit Python environment and weights.

**State and explicitly supplied images and videos are sent to the selected endpoint**,
exactly as they would be from the command line. The rules in [Where your data goes](../README.md#where-your-data-goes)
apply unchanged.

---

## Set up

The following setup uses the default TypeSafe provider. For Clef, use
[provider startup and credential settings](clef.md).
First make sure `jev` works from a terminal:

```sh
jev auth login      # stores the key in the OS credential store
jev doctor          # checks credentials, endpoint, and model; no network request
```

A host then needs only the command `jev` with the arguments `mcp serve`. **Do not put an
API key in any MCP configuration.** The server resolves credentials exactly as every
other `jev` command does (`AGENTS.md` §4): the environment variables first, then the OS
credential store. A user who has run `jev auth login` needs to configure nothing else.

### Claude Code

```sh
claude mcp add --transport stdio --scope user jev -- jev mcp serve
```

- Everything before `--` is Claude Code's; everything after it is the command.
- `--scope` is `local` (the default: this project, private), `project` (writes a shared
  `.mcp.json` to the project root), or `user` (every project, private).
- The tools appear as `mcp__jev__noul`, `mcp__jev__choice`, and so on.
- `claude mcp list` and `claude mcp get jev` report whether the server connected.
  `/mcp` does the same inside a session.

The same entry in a project's `.mcp.json`:

```json
{ "mcpServers": { "jev": { "type": "stdio", "command": "jev", "args": ["mcp", "serve"] } } }
```

Sources: <https://code.claude.com/docs/en/mcp>, checked 2026-09-23 against Claude Code
2.1.280.

### Codex

```sh
codex mcp add jev -- jev mcp serve
```

Or in `~/.codex/config.toml`, or a trusted project's `.codex/config.toml`:

```toml
[mcp_servers.jev]
command = "jev"
args = ["mcp", "serve"]
# Codex's default is 60 seconds. A `map` of 100 records against a slow or rate-limited
# API can take longer, and Codex would give up while jev kept sending.
tool_timeout_sec = 300
```

Codex starts stdio servers with a restricted environment. If you authenticate with an
environment variable rather than `jev auth login`, forward the variable by name. That
passes the value through without writing it into the file:

```toml
env_vars = ["TYPESAFE_API_KEY"]
```

On Linux, the OS credential store is reached over the D-Bus session bus. If the server
reports that secure storage is unavailable, try adding `"DBUS_SESSION_BUS_ADDRESS"` to
`env_vars`. This requirement is an inference and has not been verified.

`codex mcp list` shows the configured servers. Sources:
<https://developers.openai.com/codex/mcp> and the configuration reference, checked
2026-09-23 against codex-cli 0.156.1.

### Cursor

`.cursor/mcp.json` in a project, or `~/.cursor/mcp.json` for every project:

```json
{ "mcpServers": { "jev": { "command": "jev", "args": ["mcp", "serve"] } } }
```

The Cursor CLI, `agent` (formerly `cursor-agent`), reads the same file.
`agent mcp list-tools jev` shows what the server offers. Cursor asks you to approve a
new server before it starts it. Source: <https://cursor.com/docs/mcp>, checked 2026-09-23.

### Grok CLI

```sh
grok mcp add jev -- jev mcp serve
```

Or in `~/.grok/config.toml`. A project's `.grok/config.toml` is also read, but the Grok
CLI starts a project-scoped server only in a folder you have marked as trusted; its
official page documents only the user file.

```toml
[mcp_servers.jev]
command = "jev"
args = ["mcp", "serve"]
```

- The tools appear as `jev__noul` and so on.
- `grok mcp doctor jev` checks the server.
- The server's stderr is logged to `~/.grok/logs/mcp/jev.stderr.log`.
- The Grok CLI also loads servers from `~/.claude.json`, `.cursor/mcp.json`, and
  `.mcp.json`. A `jev` configured for another host may therefore appear in Grok too.

Source: <https://docs.x.ai/build/features/mcp-servers>, checked 2026-09-23 against grok
1.0.40. The xAI *API* supports only remote MCP servers reached by URL, so it cannot use
this local server.

### Any other MCP client

Launch `jev mcp serve` as a stdio server. Global flags set the server's defaults. For
example, this pins the model every call uses unless a call names its own:

```sh
jev --model jev-1.13.0 mcp serve
```

`--endpoint`, `--timeout`, `--retries`, and `--max-input-bytes` work the same way.
`--dry-run` is refused: the server's job is to send.

---

## The tools

| Tool | Does | Returns |
| --- | --- | --- |
| `noul` | One yes/no judgement about a state | `answers.<id>.noul`, the probability of yes |
| `choice` | One of 2–255 named options (up to 26 with Ollama) | `choice`, `confidence`, and the probability of every option |
| `score` | A position on 2–10 ordered, described levels (up to 26 with Ollama; up to 255 with the Python bridge) | `score` (the probability-weighted mean level), `confidence`, `legend`, and the full distribution |
| `ask` | Several independent Noul, Choice, and Score questions about **one** state, in one request | One answer per question id |
| `map` | One question set over up to 100 inline records | One row per record sent, in input order, and a summary |

All five tools:

- **Results.** Every result is object-rooted, with a declared `outputSchema`. It is sent
  as `structuredContent` and also as the same JSON in a text block, for hosts that read
  only text. There is one document, not two formats.
- **Result documents.** `noul`, `choice`, `score`, and `ask` return `jev.evaluation/v1`,
  exactly as `jev … --output json` prints it. `map` returns `jev.mcp.map/v1`, an object
  holding the CLI's `jev.map.row/v1` rows and its `jev.map.summary/v1` summary. See
  [output-schema.md](output-schema.md).
- **No echo.** A result never includes the state it judged.
- **Annotations.** `readOnlyHint: true`, `destructiveHint: false`,
  `idempotentHint: false`, `openWorldHint: true`. The spec gives `destructiveHint` and
  `idempotentHint` meaning only when `readOnlyHint` is false, so a host may ignore them;
  they are set because they are true, since a repeated call is another inference request, potentially billed by a hosted
  provider, and another sample from a probabilistic model. All four are hints to the host, not a
  security boundary. The boundary is that the server has no
  code that writes a file, runs a command, or changes configuration.

### Arguments

Instructions, option descriptions, Score levels, and `state` accept strings,
objects, or arrays. The `huggingface` publisher Python bridge also preserves JSON
numbers, booleans, null, and blank strings. State must be supplied explicitly;
omitting it is a usage error even for that bridge. Other providers reject numbers,
booleans, and null as JSON state; Cloudflare permits blank text state with validated
images, while TypeSafe, Ollama, and llama.cpp require nonempty text state.

Instructions may be omitted with Ollama or the Python bridge, which then use the
validated question ID. Only the Python bridge uses that fallback for explicit null
or exactly `""`; it preserves whitespace strings. Other providers require supplied,
nonempty instructions, and Ollama also rejects explicit null or blank instructions.

Clef vision calls accept
embedded PNG/JPEG/WebP images and, with `huggingface`, ordered video-frame
arrays with optional source timing metadata, `max_length`, `max_state_tokens`, and
constrained `media_kwargs`; they never open host
filesystem paths. Provider and account
are selected at server startup (`jev --provider ollama mcp serve`, for example),
while a call can override the model. Score accepts 2–10 levels for TypeSafe,
Cloudflare, or llama.cpp, 2–26 for Ollama, and 2–255 for the Python bridge as a client
resource bound. Provider options and media use the same validation as the CLI.
See [Clef capabilities and limits](clef.md).

For Cloudflare capacity control, `noul`, `score`, `ask`, and `map` accept
`"options":{"rejectIfBusy":true}`. The `choice` tool reserves its existing `options`
array for alternatives and accepts `"reject_if_busy":true` instead. Both encode the
same native Cloudflare request option. `--reject-if-busy` at server startup sets the
default; a call can override it. Other providers reject this option.

The field names follow the API's vocabulary: `state` and `instructions`, plus
`criteria` for a Noul.

```json
{"state": "Help! My payouts have been failing for 3 days.",
 "instructions": "Does this convey urgency?",
 "criteria": {"true": "Explicitly time-sensitive", "false": "No urgency expressed"}}
```

```json
{"state": {"subject": "refund", "body": "…"},
 "instructions": "Which team should own this ticket?",
 "options": [{"name": "billing", "description": "Payments and refunds"},
             {"name": "auth"}, {"name": "infrastructure"}, {"name": "support"}]}
```

```json
{"state": "diff --git …",
 "questions": [
   {"id": "api_change", "type": "noul", "instructions": "Does this change a public API?"},
   {"id": "risk", "type": "score", "instructions": "How risky is this to deploy?",
    "levels": ["Cosmetic", "Reversible behaviour change", "Irreversible"]}
 ]}
```

```json
{"questions": [{"id": "outage", "type": "noul", "instructions": "Is this an outage?"}],
 "records": [{"id": "a-1", "state": "db primary down"}, {"id": "a-2", "state": "disk 81%"}],
 "concurrency": 4}
```

- **Optional fields.** `id` names a single question's answer; the default is `answer`.
  `model` overrides the server's default for one call.
- **Order.** Choice options and `ask` or `map` questions are arrays, so they reach the
  API in the order written. That is the one deliberate difference from a CLI request
  file, where they are JSON objects. MCP arguments arrive already decoded into sorted
  maps, which would silently reorder the options a model sees.

The schemas are bundled in `crates/jev-cli/src/mcp/schema/`. The whole `tools/list`
result is pinned in `crates/jev-cli/tests/snapshots/mcp-tools.json` and treated as a
public API: changing it needs a `CHANGELOG.md` entry.

### Errors

| What happened | What the host sees |
| --- | --- |
| Unknown tool name | JSON-RPC error `-32602` |
| Arguments that do not fit the schema, or a request Jev would reject (one option, an empty instruction, state over the limit) | A tool result with `isError: true` and `{"error": {"kind": "usage", "message": …}}`. Nothing was sent. |
| The API rejected the request as invalid (HTTP 400, including an unknown model, or 422) | `isError: true`, `kind: "usage"`. The request was sent. |
| No credential, or the API rejected it | `isError: true`, `kind: "auth"` |
| Timeout, 429 or 5xx after retries, connection refused, a response `jev` cannot decode | `isError: true`, `kind: "unavailable"` |
| The call was cancelled | No reply, per the MCP spec |
| A bug in `jev`, or a failure to write | `isError: true`, `kind: "internal"` |
| One `map` record failed | A row with `"ok": false` and an `error`. The other rows are unaffected, and `summary.failed` counts it. |
| `map` hit a rejected credential before any record was answered | `isError: true`, `kind: "auth"`. The batch stops at the first rejection, as `jev map` does. |

The tool-error kinds, `usage`, `auth`, `unavailable`, and `internal`, are the CLI's
exit-code classes. A failed `map` row uses the row kinds `jev map` already writes:
`auth`, `unavailable`, `request` (the API refused the request), and `invalid-request`
(refused locally). Messages never carry a credential, or even a prefix of one.

When the API returns no answer for a question, `jev.evaluation/v1` and each `map` row
carry `missing_answers` with the ids, so an absent answer is never read as a negative. **A
failure is never an answer**: a failed record has no `answers` and is never reported as
`false` or `0`.

### `map` limits

An MCP result lands in the agent's context window all at once, which a shell pipeline
never does. `map` therefore refuses a batch before sending anything when any of these
is exceeded:

| Limit | Value | Why |
| --- | --- | --- |
| Records per call | 100 | More rows than an agent can use in one turn |
| Aggregate state and media | `--max-input-bytes` (1 MiB by default) | Serialized record states plus compressed image/video bytes after base64 decoding; template media counted once |
| Estimated result size | 80 KiB, about 20k–27k tokens | Near Claude Code's 25k-token default maximum for one result, above which it saves the result to disk instead |
| Concurrency | 1–16, default 4 | The server also runs at most 4 tool calls at once, so it never has more than 64 requests outstanding: the CLI's ceiling for one batch |

MCP counts template media once, even when a record replaces it. These units differ
from CLI `jev map`, which caps the serialized input stream (JSONL or `--lines`) and
therefore includes base64 text for embedded media. Separately named `--image` and
`--video-frame` files are outside that CLI input-stream cap.

Nothing is truncated or dropped. The refusal names the limit and recommends `jev map`
from a shell, which streams JSONL to a file and keeps the output out of the agent's
context.

---

## When to use MCP and when to use the CLI

### Use MCP when

- an interactive agent needs a bounded Jev judgement;
- typed tool calling is preferable to building a shell command and quoting state;
- the result should come straight back to the agent.

### Use the CLI when

- processing large files or JSONL (`jev map -i … --output-file …`);
- writing CI or shell workflows, where `--require` and exit codes are the interface;
- running `jev eval`, which is deliberately not an MCP tool;
- managing auth or configuration (`jev auth`, `jev config`, `jev doctor`);
- the run must be reproducible from a script;
- the output should stay out of the agent's context.

---

## What the server does not do

- It exposes no tool that reads or writes files, runs commands, fetches URLs, or changes
  configuration or credentials. There is no `auth`, `config`, `doctor`, or `eval` tool.
- It offers no MCP resources, prompts, sampling, elicitation, roots, subscriptions, or
  tasks. The `jev` Agent Skill covers when and how to use Jev; the server only executes.
- It reads nothing local except the credential, through the same subsystem the CLI
  uses. It does not look at the working directory, the repository, `.env` files, or the
  host's transcripts.
- It never listens on a network port. HTTP transport, OAuth, and remote hosting are
  out of scope ([ADR-0012](adr/0012-mcp-server.md), "Revisit if").

The server treats state strictly as data. Instructions embedded in a document are
evaluated by the model as part of the text to judge; the server cannot act on them.

---

## Protocol

- **SDK.** `rmcp` 3.4.1, the official Rust SDK, with only its `server` and
  `transport-io` features.
- **Revisions.** 2024-11-05, 2025-03-26, 2025-06-18, and 2025-11-25 over the
  `initialize` handshake, and the current stateless revision, 2026-07-28, over
  `server/discover`. The server echoes an older client's version, and the tools behave
  the same at every revision.
- **Capabilities.** Tools only.
- **Server instructions.** One short paragraph: Jev judges and does not generate,
  supplied content goes to the configured endpoint, and uncertainty should be kept.
  Claude Code and Codex surface it. State and explicitly supplied images and videos
  follow the selected provider: TypeSafe by default, Cloudflare when selected, or the
  configured local server. Loopback reaches that server; content stays on the machine
  only if the server runs locally without cloud offload or proxy forwarding.
- **Cancellation.** `notifications/cancelled` stops the call's retry wait and its `map`
  loop. An HTTP attempt already in flight ends at `--timeout` (10 s by default), and the
  call keeps its place among the four that may run at once until it does, so a very
  large `--timeout` can make later calls wait behind cancelled ones.
- **Shutdown.** Closing stdin ends the server with status 0. With a call still in
  flight, rmcp first waits up to about 5 seconds for it to finish. Ctrl-C ends the
  server with status 130. A single message longer than the line limit ends it with
  status 74 and says why on stderr.
- **Known gap.** A request whose arguments cannot be parsed at all, for example state
  nested deeper than the JSON parser's recursion limit, gets no reply from rmcp. The
  server stays up and memory stays bounded, but the host waits for that one id until
  its own timeout.

### stdout and stderr

stdout carries protocol messages and nothing else; a stray byte there corrupts the
session. Diagnostics go to stderr, and there are none by default. The exceptions:

- the non-official-endpoint warning, printed once at startup whenever `--endpoint`
  points somewhere else;
- with `--verbose`, one line per call naming the tool, its outcome, and the time taken.
  The line never includes arguments or state.

### Conformance

- **Tested.** The real binary is driven through the official Rust SDK's client and over
  raw pipes (`crates/jev-cli/tests/mcp.rs`), and checked with the official MCP Inspector
  (`scripts/mcp-inspector.sh`; `scripts/verify.sh` runs it only with
  `JEV_MCP_INSPECTOR=1`, because it downloads from npm on a cold cache).
- **Not run.** The official conformance suite, `@modelcontextprotocol/conformance`,
  tests servers only over an HTTP URL, and stdio support is an open request upstream
  (modelcontextprotocol/conformance#258). A test-only HTTP wrapper would test a
  transport `jev` does not ship. This project therefore does not claim conformance
  beyond what is listed here.

### Tool selection, measured

Tool descriptions are written for the model that chooses among them, so they are tested
on one. `scripts/mcp-tool-selection.py` runs the cases in
`evals/mcp/tool-selection.json` through `claude -p` (Claude Opus 5.5, low effort) with
only this server connected and a mock backend. It grades the tool calls the agent
actually attempted.

- **Positive cases.** A Noul question, a team-routing Choice, a five-level risk Score,
  three checks against one diff (`ask`), and 80 alerts under one question (`map`).
- **Negative cases.** Rewrite a function, `1783 * 29`, find `FooBar`, write release
  notes, and run a calibrated evaluation. The last one must go to the `jev eval` CLI and
  no MCP tool.

| Run, 2026-09-23 | Result |
| --- | --- |
| First pass, one repetition | 9/10. The three-checks-on-one-diff case called `map` instead of `ask` (1 of 4 runs of that case). |
| After the `map` description said that several questions about one state are an `ask`, "even one with several parts, such as a multi-file diff" | 20/20 (two repetitions) |
| After review edits to the `map` description ("one row per record sent"), the `ask` and `map` cases again | 4/4 (two repetitions) |

This is a small sample on one host and one model. It shows the descriptions steer
correctly on these cases; it does not prove they do everywhere. The eval costs about
$0.08 per case on the caller's Claude credential, is not part of `scripts/verify.sh`,
and sends nothing to the selected Jev inference endpoint. The transcript goes to the evaluator
model service. These dated measurements are historical, not new Clef tool-selection
evidence.

---

## Check a connection with the MCP Inspector

```sh
scripts/mcp-inspector.sh
```

The script runs the pinned Inspector version (2.7.0) in CLI mode against the built
binary, with no key and a loopback mock. It lists the tools, validates their schemas,
and calls each one. To poke at your own installed `jev` by hand:

```sh
npx @modelcontextprotocol/inspector@2.7.0 --cli jev mcp serve -- --method tools/list
npx @modelcontextprotocol/inspector@2.7.0 --cli jev mcp serve -- \
  -e TYPESAFE_API_KEY="$TYPESAFE_API_KEY" \
  --method tools/call --tool-name noul --tool-args-json '{"state":"x","instructions":"?"}'
```

The Inspector, like other clients built on the MCP TypeScript SDK, passes the server
only a short allow-list of environment variables, so a key in the environment reaches
`jev` only through `-e`. `jev auth login` needs nothing.

A `tools/call` against your real configuration sends state and supplied media to
the selected inference endpoint. Hosted usage can be billed; local runtime execution
consumes local resources. Forward Cloudflare custom credential variables and its
account ID by name when selecting that provider. Loopback local providers need no key.

---

## Troubleshooting

| Symptom | Likely cause and fix |
| --- | --- |
| The host says it failed to start or connect | `jev` is not on the PATH the host sees. Use an absolute path as `command`, from `command -v jev`. |
| Every call returns `kind: "auth"` | No credential in the host's environment. Run `jev auth login`, or forward `TYPESAFE_API_KEY` (Codex: `env_vars`). `jev doctor` from a terminal shows what `jev` can see. |
| `kind: "auth"` although `TYPESAFE_API_KEY` is exported | The host did not pass it on. Hosts built on the MCP TypeScript SDK's defaults, and Codex, give a server only a short allow-list of variables such as `PATH` and `HOME`. Forward the variable in the host's configuration by name (Codex: `env_vars`), or use `jev auth login`. |
| `secure credential storage is unavailable` | The host started `jev` without access to the OS credential store: over SSH, in a container, or with a restricted environment. Use `TYPESAFE_API_KEY`. |
| `kind: "auth"` with Cloudflare or a remote custom endpoint | Set or forward `JEV_CUSTOM_API_KEY` or `JEV_CUSTOM_API_KEY_FILE`, and the Cloudflare account ID when applicable. TypeSafe keys and its OS store are not consulted. |
| `kind: "unavailable"` | Check provider availability and the separately started local runtime. For slow CPU bridge inference, use an explicit deadline such as `--timeout 600 --retries 0`; the host tool timeout must allow that wait. A timeout/disconnect does not cancel PyTorch work already running. |
| `map` refuses a batch | It is over a limit above. Split it, or use `jev map` from a shell. |
| The connection drops right away | Something wrote to stdout. `jev` does not; check for a wrapper script that prints. The Inspector's `--method initialize` shows the first bytes. |
| An old host cannot negotiate | The server accepts every revision back to 2024-11-05. Report the host and version in an issue. |

Use the host's own diagnostics too: `claude mcp get jev`, `codex mcp list`,
`agent mcp list-tools jev`, and `grok mcp doctor jev`.

---

## Cost of the feature

Measured on x86_64 Linux, 2026-09-23:

| | Without MCP | With MCP |
| --- | --- | --- |
| Release binary | 4,766,968 B | 6,291,368 B (+1.5 MB) |
| `jev --version`, median of 30 runs | 5.7 ms | 5.2 ms |
| `jev mcp serve`: spawn to `initialize` reply, median of 20 | | 6.0 ms |
| `tools/list`: payload, median latency | | 19.5 KB, 1.2 ms |

- **Where the size goes.** Most of the growth is `rmcp`'s protocol-model serde code
  (about 430 KB of symbols) and `tokio` (about 90 KB).
- **Other commands.** No other command builds the runtime, so their startup is
  unchanged within noise.
- **Per call.** The adapter's per-call overhead is a thread hand-off and one JSON
  encoding, both negligible next to the network round trip. It was not measured
  separately, so no figure is claimed.
