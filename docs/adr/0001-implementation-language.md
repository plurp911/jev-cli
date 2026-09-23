# ADR-0001: Rust as the implementation language

* Status: Accepted
* Date: 2026-09-19

## Context

`jev` must be a tool people reach for from a terminal, a shell script, a CI job, an AI
coding agent, and a data pipeline. That set of callers constrains the language more than
it might appear:

- **Startup cost is a feature.** An agent or a pipeline may invoke `jev` in a loop.
  Tens of milliseconds of interpreter or VM startup, multiplied by a loop, is the
  difference between a tool people use and one they work around.
- **Installation must be trivial and offline-safe.** In CI, "first install a runtime,
  then resolve dependencies" is a failure mode: it is slow, it is a supply-chain event
  on every run, and it breaks in air-gapped environments.
- **Cross-platform means Windows too.** Not "Linux plus best effort".
- **It handles credentials.** Memory-safety bugs in a tool that holds an API key in
  process memory are expensive.
- **It is written almost entirely by AI agents.** Errors the compiler catches are errors
  a reviewer does not have to catch, which raises the value of a strong type system
  considerably above its usual level.

## Options considered

### Rust

*For.* Single statically linked binary with no runtime. Startup in low single-digit
milliseconds. Memory safety without a garbage collector, and `#![forbid(unsafe_code)]`
turns that into an enforced property rather than an aspiration. An exceptionally strong
type system: newtypes, exhaustive enums, and `Result` let invariants like "a probability
is within `[0, 1]`" be encoded so that violating code does not compile. Mature CLI
ecosystem (`clap`), mature cross-compilation and packaging (`dist`, `cross`), and
first-class tooling for exactly the guarantees this project needs — `cargo-deny`,
`cargo-nextest`, `cargo-llvm-cov`, `cargo-fuzz`, `proptest`, Clippy, `cargo-semver-checks`.

*Against.* Slower to write. Compile times are long. A smaller contributor pool than Go
or Python. Async Rust is genuinely difficult — sidestepped here by choosing a blocking
transport (ADR-0007).

### Go

*For.* Also a single binary with fast startup. Simpler language, faster compiles, larger
contributor pool, excellent cross-compilation, strong standard library for HTTP.

*Against.* A materially weaker type system for this problem. Go cannot express "a
validated probability" or "a secret that cannot be printed" as a type the compiler
enforces; `interface{}`, zero values, and the absence of sum types push those invariants
into runtime checks and convention. For a codebase written by AI agents, that moves
error detection from compile time to review time, which is exactly the wrong direction.
Secret hygiene in particular is harder: nothing prevents a struct containing a key from
being `%+v`-printed.

### Python

*For.* The fastest to write, the largest contributor pool, and the official TypeSafe
Python SDK is available.

*Against.* Disqualifying on the primary constraints. Interpreter startup is 30–100 ms
before any work; installation requires a Python environment and dependency resolution,
which is unreliable in CI and unpleasant on Windows; shipping a single binary requires
PyInstaller-class tooling with its own failure modes; and dynamic typing removes the
main defence against agent-written errors. Python remains the right choice for *using*
Jev in an application — that is what the official SDK is for — but not for the tool that
must be present everywhere a shell is.

## Decision

**Rust.**

The deciding factor is not performance; Go would be fast enough. It is that this
repository is written almost entirely by AI agents, and Rust's type system converts a
whole class of agent mistakes into compile errors. Combined with `forbid(unsafe_code)`,
a mature security-tooling ecosystem, and genuinely trivial single-binary distribution,
nothing in the Go or Python case materially outweighs it.

Supporting choices:

- Edition 2024, resolver 3.
- MSRV declared in `Cargo.toml` and verified locally before push (ADR-0013). The
  MSRV-aware resolver keeps dependency selection consistent with it. It started at
  `1.85`, the minimum for edition 2024, and was raised to `1.88` when the OS credential
  store landed: the
  Secret Service backend requires it. Raising the MSRV is a compatibility decision and
  belongs in `CHANGELOG.md`, not in a silent dependency bump.
- The development toolchain is pinned in `rust-toolchain.toml` for reproducible
  formatting and lint results; the local MSRV check overrides it with
  `RUSTUP_TOOLCHAIN`.
- `unsafe_code = "forbid"` workspace-wide. Overriding this requires an ADR and a
  human decision, and would need an extraordinary justification.

## Consequences

- Contributors need a Rust toolchain. `rustup` makes this a one-liner, and
  `rust-toolchain.toml` makes it automatic.
- Compile times will grow. Mitigated by a small dependency tree and CI caching.
- The contributor pool is smaller than Go's. Accepted: this is a small, security-
  sensitive tool where correctness matters more than contribution volume.
- Some functionality that would be a library call in Python must be written here. That
  is a dependency-policy benefit as much as a cost (ADR-0004).

## Revisit if

- The project needs an in-process plugin or scripting surface, which it currently
  forbids by design.
- Cross-compilation or Windows packaging becomes a recurring maintenance burden that
  Go would demonstrably avoid.
- A Rust-specific supply-chain or toolchain problem makes the ecosystem advantages
  evaporate.
