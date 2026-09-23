# Architecture

## Overview

```
                      ┌──────────────────────────────────────┐
   argv, stdin ──────▶│  jev-cli        binary `jev`         │
   stdout, stderr ◀───│  parsing, dispatch, rendering        │
                      └───────┬──────────────────┬───────────┘
                              │                  │
                   ┌──────────▼────────┐  ┌──────▼─────────────┐
                   │  jev-client       │  │  jev-config        │
                   │  Transport trait  │  │  Secret, sources   │
                   │  request/response │  │  credentials only  │
                   └──────────┬────────┘  └────────────────────┘
                              │             (no workspace deps)
                      ┌───────▼──────────────────────────────┐
                      │  jev-core                            │
                      │  pure domain model, no I/O           │
                      └──────────────────────────────────────┘
```

Dependencies point downward only. A reverse edge is a design error. `jev-config`
depends on nothing else in the workspace, so `jev-core` structurally cannot reach a
`Secret`.

## The crates

### `jev-core`

Value types and total functions over them. It may not perform I/O, read the
environment, touch the filesystem, open a socket, or handle a credential.

That restriction is the point: everything here is testable exhaustively and
deterministically, including with property tests. `Probability` is the template —
construction and deserialization both validate, so an out-of-range value cannot exist
anywhere downstream and no other code needs to defend against one.

### `jev-client`

The TypeSafe System One client. Every outbound request goes through the `Transport`
trait, whose signature mentions no HTTP library:

```rust
pub trait Transport: Debug + Send + Sync {
    fn execute(&self, request: &Request, credential: &Credential)
        -> Result<Response, TransportError>;
}
```

`Request` and `Response` are plain data. The credential is a **separate argument**, not
a field: `Credential` is not `Clone` and not `Serialize`, so a retry loop cannot
duplicate plaintext, and the key becomes an `Authorization` header only inside the
concrete transport, in a buffer that zeroizes. That is the closure of the T2 follow-up
in the threat model.

A non-2xx status is deliberately **not** a transport error. It comes back as a
`Response` so the caller can decode the API's structured error body and tell a 401 from
a 422 from a 529 — a distinction the exit-code contract depends on.

`MockTransport`, behind the `testing` feature, records what it was asked to send and
returns queued responses, so request shaping, retry policy, and response decoding are
tested without a network, an API key, or a cassette framework. It records that a
credential was supplied, never its value.

Retry is a **pure function** of (attempt, outcome, headers, elapsed, jitter sample).
Deciding and waiting are separate, so backoff growth, the `Retry-After` path, and budget
exhaustion are all unit-tested without a single `thread::sleep`.

`Request`'s `Debug` implementation redacts all header values and prints only the body
length, because `Debug` output reaches logs and panic payloads.

The concrete transport is behind the `http` feature, so the decoding and retry logic can
be built and tested with no networking code linked at all.

### `jev-config`

The only crate permitted to touch credential material. `Secret` redacts on `Debug` and
`Display`, zeroizes on drop, and deliberately does not implement `Serialize`, `Clone`,
or `Deref`; reading the plaintext requires `.expose()`, which is greppable and
auditable. `CredentialSource` records *where* a key came from so `jev` can report that
without reporting the key.

Two seams make this crate testable without touching a developer's machine: `Environment`
and `SecretStore` are traits with in-memory doubles, so every precedence and failure
path is exercised without mutating the process environment or prompting a real keychain.
`LazyStore` defers connecting to the platform store until a credential is actually
needed, so `jev --version` does not open a D-Bus connection.

`Settings` is the non-secret configuration file. It refuses to load a document
containing a key whose *name* looks like a credential, at any nesting level, which is
what stops the file from quietly becoming a plaintext credential store.

### `jev-cli`

Argument parsing (clap), dispatch, and rendering. `main` parses, runs, and maps the
result to an exit code; everything else is a function taking an explicit
`impl io::Write`, which is why the entire command surface is testable in-process.
The workspace denies `clippy::print_stdout` and `clippy::print_stderr`, so the
`println!` family is unavailable. Note what that does *not* cover: a direct
`writeln!(io::stdout(), ..)` is not linted, so "output only happens through an injected
handle" is enforced for macros and upheld by review for the rest.

`output::sanitize` escapes terminal control sequences, bidirectional overrides, and
zero-width characters in untrusted text, and `output::Safe` wraps the result so the
compiler, not review, is what stops an API-supplied option name reaching a terminal raw.
`exit` holds the exit-code contract, locked by a test.

`context` resolves flags, the configuration file, and the environment into one value
with documented precedence, recording *where* each setting came from. `jev doctor`
prints that structure directly, so what the user is told is what the code uses rather
than a second description of it that can drift.

`gate` is the `--require` expression language: a recursive-descent parser with an
explicit depth bound over a grammar with no arithmetic, no function call, and no way to
name anything outside the response. Nothing reaches a shell.

`mcp` is `jev mcp serve` ([ADR-0012](adr/0012-mcp-server.md)): `mcp/server.rs` is the
protocol adapter over the official `rmcp` SDK, and `mcp/tools.rs` turns tool arguments
into the same `jev_core` requests and calls the same `evaluate::send`,
`map::evaluate_all`, and renderers the commands use. It is the only module that runs an
async runtime, and it builds one only for the life of `jev mcp serve`.

The crate has a library target as well as a binary. It exists so the integration tests
can drive the command surface in process, the fuzz targets can reach the parsers, and
the benchmark can separate parsing from process startup. It is not an API; see
[ADR-0003](adr/0003-cli-compatibility.md).

## Key decisions

| Decision | Rationale | Record |
| --- | --- | --- |
| Rust | Single native binary, no runtime, fast startup, strong typing, mature cross-compilation | [ADR-0001](adr/0001-implementation-language.md) |
| No plaintext credential fallback | A silent fallback is how keys end up in backups and images | [ADR-0002](adr/0002-security-and-credentials.md) |
| CLI is the product; crates promise nothing | Lets internals be refactored freely while scripts stay stable | [ADR-0003](adr/0003-cli-compatibility.md) |
| Few dependencies, enforced by `deny.toml` | Every dependency runs with access to the user's API key | [ADR-0004](adr/0004-dependency-policy.md) |
| Blocking transport, no async runtime | A short-lived process issuing a handful of requests gains nothing from a runtime, and `tokio` is a large surface | [ADR-0007](adr/0007-workspace-architecture.md) |
| Four crates rather than one | Isolates credentials, makes the network a seam, keeps the domain model pure | [ADR-0007](adr/0007-workspace-architecture.md) |
| Environment before keychain; a separate credential namespace per endpoint | A stored key that silently outranks an exported one is a trap, and namespace isolation makes endpoint exfiltration structurally impossible | [ADR-0008](adr/0008-credential-precedence-and-endpoint-isolation.md) |
| `dist` builds artifacts; it does not own the workflow | Its generated workflow needs ~30 hand edits per regeneration to meet this repository's own security bar | [ADR-0009](adr/0009-dist-as-a-builder-not-a-workflow-generator.md) |

## Testing strategy

| Layer | Location | Proves |
| --- | --- | --- |
| Unit | `#[cfg(test)]` per module | Logic and invariants, including hostile input |
| Property | `proptest` in `jev-core`, `jev-client`, `jev-cli` | Total functions really are total; every accepted endpoint is TLS or loopback; no retry wait exceeds the budget |
| Doc | `///` examples | Documented usage compiles and runs |
| Integration | `crates/jev-cli/tests/` | Real process, real sockets, exit codes, stream separation |
| Transport | `crates/jev-client/tests/transport.rs` | The real HTTP client against a real socket: status handling, header casing, size caps, redirects |
| Compatibility | `crates/jev-client/tests/fixtures/` | Documents recorded from official TypeSafe sources still encode and decode |
| Fuzz | `fuzz/` | Five targets over every place attacker-influenced bytes enter, each asserting a domain invariant |
| Canary | `secret.rs`, `credential.rs`, `transport.rs`, `scripts/credential-canary.sh` | A known secret does not reach tested output paths |
| Packaging | `scripts/release-dry-run.sh` | The archive builds, its checksum verifies, and the binary inside runs |

No test touches the network beyond loopback, sleeps, or depends on wall-clock time,
environment, or ordering. Clocks and jitter are injected; the environment and the
credential store are traits with in-memory doubles. Retries are pinned to zero.

## Deliberately absent

The list of things this CLI does not have, and the process for proposing one, is in
`AGENTS.md` §3.3. In short: no plugin system, no arbitrary code execution, no telemetry,
no `.env` discovery, no async runtime outside `jev mcp serve`, and no self-update.

Everything the architecture above describes — the HTTP transport, the request and
response models, retry and backoff, OS keychain integration, and every user-facing
command — is implemented. The request and response models were written against the
official TypeSafe API reference, not from recollection (`AGENTS.md` §6), and the
recorded fixtures in `crates/jev-client/tests/fixtures/` fail if decoding drifts.
