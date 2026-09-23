# ADR-0007: Four crates, and a blocking transport behind a trait

* Status: Accepted; the "Blocking" decision is superseded for `jev mcp serve` only by
  [ADR-0012](0012-mcp-server.md)
* Date: 2026-09-19

## Context

Two structural decisions shape everything else in this codebase: how it is divided, and
whether it is async.

The dividing constraint is testability. Almost all of `jev` should be exercisable
without a network, without an API key, and without a recording framework — both because
that makes tests fast and deterministic, and because this repository is written by AI
agents whose work is only as trustworthy as the tests that check it. Tests that need a
live API are tests that get skipped.

The second constraint is credential isolation. If any crate may touch a secret, then
reviewing secret handling means reviewing everything.

## Options considered

### Structure

1. **One crate.** Simplest, fastest to compile. Rejected: nothing structurally prevents
   the CLI layer from opening a socket or the domain model from reading the environment,
   so the testability and credential-isolation properties would rest on discipline alone
   — exactly what does not hold up under agent-authored change.
2. **Two crates (lib + bin).** The common Rust shape. Better, but still leaves
   credentials, transport, and domain model in one compilation unit.
3. **Four crates by responsibility.** Chosen.
4. **More than four.** Rejected as premature. Each crate must earn its boundary.

### Async

1. **Async with `tokio`.** The default in Rust networking. For `jev` it buys concurrent
   in-flight requests — but System One batching happens by sending multiple *questions*
   in one request, which is the documented way to parallelize, so the concurrency
   argument is much weaker here than it looks. The costs are concrete: `tokio` is a large
   dependency with a wide surface, it adds startup cost to a process that must start in
   milliseconds, `async` traits complicate the `Transport` seam, and it makes the code
   harder for contributors and agents to get right.
2. **Blocking.** Chosen.

## Decision

### Four crates

```
jev-cli  ──▶  jev-client  ──▶  jev-core
   └─────▶  jev-config  ──▶  (nothing)
```

| Crate | Responsibility | Prohibited from |
| --- | --- | --- |
| `jev-core` | Value types and total functions | I/O, environment, filesystem, network, credentials |
| `jev-client` | API client behind `Transport` | Reading credentials from anywhere; owning global state |
| `jev-config` | Configuration and credentials | Network access |
| `jev-cli` | Parsing, dispatch, rendering | Business logic; `println!` |

Dependencies point downward only; a reverse edge is a design error. The boundaries are
partly enforced by the compiler — `jev-core` does not depend on `jev-config`, so it
*cannot* touch a `Secret` — and partly by lints: `clippy::print_stdout` and
`clippy::print_stderr` are denied workspace-wide, so rendering cannot leak out of
`jev-cli` by accident.

### The `Transport` seam

```rust
pub trait Transport: Debug + Send + Sync {
    fn execute(&self, request: &Request) -> Result<Response, TransportError>;
}
```

No HTTP-library type appears in the signature. `Request` and `Response` are plain data.
A non-2xx status is **not** a transport error — it is returned as a `Response` so the
caller can decode the API's structured error body, which keeps HTTP semantics out of the
transport layer.

`MockTransport`, behind the `testing` feature, records requests and returns queued
responses, so request shaping, retry policy, and decoding are all testable offline.
`Request`'s `Debug` redacts header values and prints only the body length.

### Blocking

`jev` is a short-lived process issuing a small number of requests. `tokio` is banned in
`deny.toml`, so it cannot arrive silently through a dependency. Introducing async
requires superseding this ADR.

## Consequences

- Nearly the whole codebase is testable without a network. That is the point.
- Credential handling is reviewable by reading one crate.
- Four manifests to maintain, and some types must be public across crate boundaries that
  would otherwise be private. Accepted.
- Choosing an HTTP client is constrained to one with a usable blocking API. This is not
  limiting in practice.
- If genuine request concurrency is ever needed, `std::thread` over a `Transport` is
  available without a runtime.

## Revisit if

- A command needs many concurrent long-lived requests that batching cannot express.
- A crate boundary stops paying for itself, or a fifth is genuinely needed.
