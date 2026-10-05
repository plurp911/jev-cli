# ADR-0012: A local, stdio-only MCP server, and the one async runtime it needs

* Status: Accepted
* Date: 2026-09-23
* Supersedes, in part: [ADR-0007](0007-workspace-architecture.md) ("Blocking"), for
  `jev mcp serve` only

## Context

AI coding agents are one of the four callers `AGENTS.md` §1 names. Today they reach Jev
by shelling out to `jev` and parsing stdout. That works, but it means every agent
re-derives the flag vocabulary from `--help`, quotes state through a shell, and gets
untyped text back. The Model Context Protocol lets a host discover typed tools and call
them directly, and every major agent host speaks it: Claude Code, Codex, Cursor, and the
Grok CLI all launch a local server over stdio.

The requirements were set by the maintainer:

* Install `jev` once; `jev mcp serve` is the whole setup. No second package, no Node or
  Python runtime, no second credential store.
* Five tools: `noul`, `choice`, `score`, `ask`, `map`. No administration, no `eval`, no
  filesystem, no shell.
* **Not a second implementation.** The CLI and the MCP tools must share behaviour by
  construction.
* Use the official Rust SDK, `rmcp`, rather than a hand-rolled protocol.

The last requirement conflicts with ADR-0007. `rmcp` 3.4.1 requires `tokio`, `futures`,
`tokio-util`, `tracing`, `chrono`, and `indexmap` unconditionally; `deny.toml` banned
`tokio`, and `AGENTS.md` §3.3 required an ADR and a human decision to add an async
runtime. The maintainer made that decision on 2026-09-23. This ADR records it and its
limits.

## Options considered

### Protocol implementation

1. **The official SDK, `rmcp`.** Chosen. It is a Tier-1 SDK maintained under the
   `modelcontextprotocol` organisation. It negotiates every revision from 2024-11-05 to
   the current 2026-07-28 (including 2026-07-28's stateless `server/discover` flow) and
   implements JSON-RPC framing, cancellation bookkeeping, and the version matrix. Each
   of those is exactly what a hand-rolled server would get subtly wrong.
2. **A hand-rolled blocking JSON-RPC server.** No new dependencies. Rejected: it would
   put protocol negotiation, a specification this project does not own, into this
   repository. It would also have to track a specification that changed its lifecycle
   model entirely between 2025-11-25 and 2026-07-28.

### Transport

1. **stdio only.** Chosen. The host starts the process and owns both pipes. There is
   no listener, no port, no authentication server, and no way for another local
   process to connect.
2. **Streamable HTTP as well.** Rejected for now: a listening endpoint needs its own
   authentication, origin checks, and DNS-rebinding defences. It would turn a local tool
   into a service. See "Revisit if".

### Surface

1. **Tools only.** Chosen. The shipped Agent Skills already decide *when* Jev fits;
   MCP prompts or resources would duplicate them in a second format.
2. **Every command as a tool.** Rejected. `auth`, `config`, and `doctor` change or
   reveal local state, and an agent must not be able to reconfigure the credential it
   runs under. `eval` is a spend-heavy, file-backed workflow that belongs in an
   explicit CLI invocation.

### Long-running work

1. **Synchronous calls, bounded `map`.** Chosen. A Jev judgement completes in about a
   second. `map` is capped at 100 records and an estimated 80 KiB of result, and a
   larger batch is refused with a pointer to `jev map`.
2. **The Tasks extension** (`io.modelcontextprotocol/tasks`). Rejected for now. It needs
   the client to opt in and durable server-side task state, and it adds a
   polling-and-cancel lifecycle. None of that is justified while every call is bounded.

## Decision

### Shape

```text
MCP host ──stdio──▶ mcp/server.rs  ──▶ mcp/tools.rs ──▶ commands::resolve_credential
                    rmcp handler       arguments →      evaluate::send
                    (protocol only)    jev_core types   map::evaluate_all / map::summary
                                                        render::json::evaluation
```

`mcp/tools.rs` builds `jev_core` requests with the same constructors the CLI uses and
calls the same functions `jev noul`, `jev ask`, and `jev map` call. Those functions were
separated from the CLI's `Session` so that both surfaces can reach them. The results are
the CLI's own documents: `jev.evaluation/v1`, and `jev.map.row/v1` rows with a
`jev.map.summary/v1` summary. The one new identifier, `jev.mcp.map/v1`, wraps those rows
and that summary in one object, because an MCP result is a single value.

There is one binary. MCP support is not a feature flag; it ships in every build.

### The runtime is confined to `jev mcp serve`

* `tokio` builds a **current-thread** runtime in `mcp::serve` and nowhere else. No other
  command constructs one, so startup cost, stack traces, and behaviour of every other
  command are unchanged.
* Tool calls run on tokio's blocking pool through `spawn_blocking`. The core, the
  `Transport` seam, and the retry loop stay blocking; ADR-0007's reasons for that are
  unchanged.
* `deny.toml` still bans `tokio`, now with `wrappers`. It may appear only as a
  dependency of `jev-cli`, `rmcp`, and `tokio-util`, plus two crates that only the
  test-only MCP client pulls in. It cannot arrive through anything else.
* `rmcp` is built with `default-features = false, features = ["server", "transport-io"]`.
  No HTTP, OAuth, or client code is compiled into the binary.

### Behaviour

* **stdout is protocol only.** `main.rs` no longer holds the stdout and stdin locks for
  the life of the process. With them held, the runtime's I/O threads would deadlock.
  Diagnostics go to stderr, and there are none by default. Nothing installs a `tracing`
  subscriber, so `rmcp`'s own logging is compiled in but emits nothing.
* **Credentials** resolve per call through `commands::resolve_credential`, the function
  the CLI uses, so ADR-0008's endpoint isolation cannot differ between the surfaces.
  No key is accepted as an argument or in MCP configuration.
* **Cancellation** is cooperative. `notifications/cancelled` sets a flag on the call's
  `InterruptibleClock`. That flag ends a retry wait at once and stops `map` taking
  another record. An HTTP attempt already in flight ends at its own timeout. Ctrl-C ends
  the server with status 130. Closing stdin ends it with status 0. The runtime is shut
  down without waiting for blocking tasks.
* **Errors.** A Jev-level failure is a tool result with `isError: true` and a
  `{"error": {"kind", "message"}}` body; the kinds are the CLI's exit-code classes. An
  unknown tool is a JSON-RPC `-32602`. A failed record in `map` is a row with `ok: false`,
  never an answer.
* **Bounds (original 2026-09-23 decision; see the current addendum below).** State obeys `--max-input-bytes`, as it does from a file. `map` accepts up
  to 100 records, with their states summing to at most `--max-input-bytes`, and an
  estimated result of at most 80 KiB, with at most 16 requests in flight per call and
  4 calls at once. A single protocol line is capped at eight times `--max-input-bytes`,
  never less than 16 MiB and never more than 256 MiB. A longer line ends the session
  with status 74. A cancelled call keeps its slot until its in-flight HTTP attempt ends,
  at most `--timeout`: blocking I/O cannot be aborted, and freeing the slot early would
  let cancel-and-retry exceed the budget.

### Measured cost

| | Before | After |
| --- | --- | --- |
| Crates in the Linux runtime graph | 89 | 120 |
| Release binary (`--release`, x86_64 Linux) | 4,766,968 B | recorded in `docs/mcp.md` |

## Consequences

* Agents get typed tools with structured results, and the tools are the CLI's own code.
* The dependency graph is larger by about thirty crates, all well maintained and
  permissively licensed. The biggest are `tokio`, `futures`, and `chrono`.
* `AGENTS.md` §3.3's "no async runtime" becomes "no async runtime outside
  `jev mcp serve`".
* The MCP tool names, input schemas, and output schemas are a public integration API.
  `crates/jev-cli/tests/snapshots/mcp-tools.json` pins them, and changing one is a
  release decision recorded in `CHANGELOG.md`.
* Tool annotations (`readOnlyHint: true`, `destructiveHint: false`,
  `idempotentHint: false`, `openWorldHint: true`) are hints to the host, not controls.
  The controls are that the server has no code that writes, executes, or reconfigures,
  and that the only local secret it reads comes from the existing credential subsystem.

## Revisit if

* Real use shows calls that need to outlive a synchronous request. Then evaluate the
  Tasks extension.
* A host that matters cannot launch a local process. A remote MCP server needs its own
  project and its own threat model: hosting, authentication, billing, and tenancy. It
  is not an extension of this one.
* `rmcp` gains a feature set that drops `chrono` or `tokio-util`, or the protocol's
  stateless revision makes a blocking implementation practical.


## Addendum: provider and media bounds (2026-10-04)

The original bounds above describe the text-only MCP decision. Explicit Clef
providers and media extend it under [ADR-0015](0015-clef-providers-and-vision.md).
Current MCP `map` totals serialized record-state bytes plus compressed image/video
bytes after base64 decoding, counting template media once even when a record
replaces it. That aggregate must fit `--max-input-bytes` before any request is sent.
The 100-record and 80 KiB estimated-result limits remain in place.

This aggregate differs from CLI `jev map`'s serialized JSONL or `--lines` input cap:
embedded media remains base64 text there, while separately named `--image` and
`--video-frame` files are outside the input-stream cap.

State and explicitly supplied images/videos go to the configured endpoint:
TypeSafe by default, Cloudflare when selected, or the selected local server.
Loopback identifies the receiving server; fully local execution requires that it
runs without cloud offload or proxy forwarding. Content and recipient approval
must precede transmission. See the current [MCP guide](../mcp.md) and
[threat model](../threat-model.md) for these active boundaries.
