# ADR-0004: Minimal dependencies, enforced by `cargo-deny`

* Status: Accepted
* Date: 2026-09-19

## Context

Every dependency of `jev` runs on the user's machine in a process that holds their
TypeSafe API key in memory and in its environment. A `build.rs` in any transitive crate
runs on every contributor's machine. The dependency tree is not a convenience
question; it is the attack surface.

The March 2026 compromise of a widely used GitHub Action, which exfiltrated organization
secrets and was then used to backdoor a package on PyPI, is the relevant shape of attack:
not a flaw in our code, but trust extended to something we did not audit.

There is a second pressure. This repository is written by AI agents, and adding a crate
is the path of least resistance for almost any problem. Without an enforced policy, the
tree grows monotonically.

## Options considered

1. **No policy, review each PR.** Rejected: review is exactly what does not scale when
   most changes are agent-authored.
2. **Vendor everything / no dependencies.** Rejected: writing our own TLS or argument
   parser would be far more dangerous than depending on `rustls` or `clap`.
3. **Small curated set, machine-enforced.** Chosen.

## Decision

### The bar for a new runtime dependency

All five must hold, and the justification goes in the pull request:

1. **Necessary.** Neither `std` nor an existing dependency covers it, and what we would
   otherwise write is non-trivial or security-sensitive.
2. **Maintained.** Recent releases, a real maintainer, a responsive issue tracker.
3. **Proportionate.** Judged on the *transitive* tree. A crate that drags in an async
   runtime or a C toolchain is not small.
4. **Acceptably licensed**, per the allow-list in `deny.toml`.
5. **Documented.** Purpose and maintenance assessment stated in the PR.

Prefer `default-features = false` and enable only what is used. Dev-dependencies clear a
lower bar but still run on maintainer machines, so they are not free.

### Current runtime dependencies

Declared in `[workspace.dependencies]`, every one with `default-features = false`:

| Crate | Why |
| --- | --- |
| `clap`, `clap_complete` | Argument parsing, help generation, shell completions. Writing our own would be worse in every respect. |
| `serde` + `serde_json` | The API is JSON. Deriving typed decoding at the boundary is what makes hostile input safe to handle. |
| `toml` | The configuration file format. `parse`, `display`, and `serde` only. |
| `thiserror` | Error definitions without hand-written boilerplate. Compile-time only in effect; no runtime surface. |
| `zeroize` | Clears credential material from memory on drop. Directly serves ADR-0002. |
| `ureq` (+ `rustls`) | Blocking HTTPS with no async runtime, per ADR-0007. Features off keeps out gzip, charset conversion, and the `native-tls` path that would reintroduce OpenSSL. |
| `keyring-core` + one target-gated store | The OS credential store. See below. |
| `rpassword` | Reads the key from a TTY without echoing it. The alternative — raw `termios`/console handling — is exactly the security-sensitive code criterion 1 says not to write ourselves. |
| `ctrlc` | A signal handler so an interrupt exits `130` and does not leave a half-written output file. `std` has no portable equivalent. |

### Where the tree is bigger than "small"

Criterion 3 judges the *transitive* tree, so the places it is strained are recorded
here rather than left for a reader to discover:

| Fact | Consequence | Why it was accepted |
| --- | --- | --- |
| `rustls` pulls **`ring`**, which builds C and assembly through `cc` | A C toolchain is required to build `jev`, and `build.rs` compiles native code on every contributor's machine | The alternative is `aws-lc-rs` (also C) or no TLS. `ring` is the most audited option, and a pure-Rust TLS stack with equivalent review does not exist. |
| The Linux Secret Service store vendors **libdbus**, built from source | A Linux build compiles a C library | Chosen over linking the system `libdbus-1`: no `libdbus-1-dev` on the build host, no shared-library dependency on the target, and `crypto-rust` keeps the banned OpenSSL out. |
| Three keyring store crates are declared | Looks like three dependencies for one job | Each is `[target.'cfg(...)'.dependencies]`, so exactly one is ever compiled. A macOS build links neither the Windows nor the D-Bus code. |
| The `keyring` facade was rejected for `keyring-core` | More explicit wiring in our code | The facade's `v1` feature drags a zbus-based Secret Service client and roughly **200** transitive crates. This is the ecosystem's own recommendation for an application that chooses its stores. |

Transitive crate count for `jev-cli`, normal dependencies only: **88** on
`x86_64-unknown-linux-gnu`, **71** on `x86_64-apple-darwin`, **61** on
`x86_64-pc-windows-msvc`. The Linux figure is the vendored libdbus tree. A change that
moves any of these materially belongs in a pull-request description.

### Machine enforcement

`deny.toml`, checked locally by `scripts/verify.sh` before every push (ADR-0013):

- **advisories** — RustSec database; yanked crates denied; no ignores.
- **licenses** — allow-list only, so a copyleft obligation cannot arrive transitively.
- **bans** — `openssl` and `openssl-sys` (use `rustls`), `tokio` (blocking by design,
  ADR-0007; admitted only under `rmcp` and `jev-cli` for `jev mcp serve`, ADR-0012), `libloading` (no dynamic plugin loading). Wildcard versions denied.
- **sources** — crates.io only. No git or path dependencies in the release graph, so
  builds stay reproducible and auditable.

Plus: `Cargo.lock` is committed, local builds use `--locked`, and
`scripts/check-no-native-tls.sh` rejects a native TLS dependency.

### On `cargo-audit`

`cargo-audit` is deliberately **not** run. `cargo-deny check advisories` consults the
RustSec advisory database and also covers licenses, bans, and sources. Running both
would duplicate the work without adding a separate source of findings.

### Updates

Dependabot, weekly. Patch and minor updates are grouped into one reviewable PR each;
security updates are grouped separately; **major updates are excluded from automation**
and get their own PR and their own review, because a major bump is a design decision.

## Consequences

- Some functionality must be written here rather than pulled in. That is the intended
  trade.
- Adding a dependency is slower. Intended.
- `deny.toml` will need maintenance as the tree grows — particularly the license
  allow-list. Relaxing it is a reviewable security decision, and `.github/CODEOWNERS`
  routes it to a security reviewer.
- An HTTP client and a keychain crate are still to be chosen. Both are significant
  under this policy and each needs its own justification in the PR that adds it. The
  expected direction is `rustls`-backed HTTP with no C toolchain requirement.

## Revisit if

- A dependency on the ban list becomes unavoidable — that is an ADR, not a `deny.toml`
  edit.
- `cargo-deny`'s scope changes such that a separate advisory tool becomes useful again.
