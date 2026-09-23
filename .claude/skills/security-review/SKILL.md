---
name: security-review
description: >
  Threat-model and review a change that touches credentials, the network, untrusted
  input, output rendering, file paths, dependencies, GitHub Actions workflows, or
  release configuration. Use before opening a pull request on any of those surfaces,
  when reviewing someone else's change to them, or when adding a dependency. Produces a
  written finding list, not a rubber stamp.
allowed-tools: Bash Read Grep Glob
---

# Security review

`jev` holds a TypeSafe API key in memory and in its environment, and it runs on
developer machines. The full analysis is in `docs/threat-model.md`; the commitments
are in `SECURITY.md`. This skill is how you check a change against them.

## 1. Scope the change

```sh
git diff --stat main...HEAD
git diff main...HEAD
```

Does it touch any of these? If yes, this review is required, not optional.

| Surface | Files |
| --- | --- |
| Credentials | `crates/jev-config/**` |
| Network | `crates/jev-client/**` |
| Output rendering | `crates/jev-cli/src/output.rs`, `cli.rs` |
| Untrusted input | Anything parsing bytes from network, stdin, a file, or the environment |
| Dependencies | `Cargo.toml`, `Cargo.lock`, `deny.toml` |
| Local gate and release workflow | `.githooks/**`, `scripts/verify.sh`, `.github/workflows/**`, `.github/dependabot.yml` |
| Release | `.github/workflows/release.yml`, `publish` fields |
| Agent instructions | `AGENTS.md`, `CLAUDE.md`, `.claude/skills/**` |

## 2. Run the mechanical checks first

```sh
scripts/verify.sh
zizmor --persona=pedantic .github
cargo deny --all-features check
rg -n 'expose\(\)' crates/            # every credential disclosure point
rg -n 'unwrap\(|expect\(|panic!|unreachable!' crates/ --glob '!**/tests/**'
rg -ni 'sk-[a-z0-9]{8,}|api[_-]?key\s*=\s*["'"'"']' -g '!*.md'
```

Mechanical checks find the obvious. The rest of this skill is the part they cannot do.

## 3. Walk the threat model

Take each threat that the change could plausibly touch. Do not skim; answer concretely.

### Credentials (T1, T2, T3, T4)

- Can this change put credential material into **any** output stream, including an
  error, a `Debug`, a panic payload, or a log line?
- Does any new struct holding a secret derive `Debug` or `Serialize`? `Secret` redacts
  itself, but a `String` field next to it does not.
- Is there a new `.expose()` call? Justify it. Is the plaintext copied into a
  `String` that outlives it, or into something with a derived `Debug`?
- Does any new flag accept a credential as an argument? It must not.
- Does the change create a path where a credential is written to disk in plaintext, or
  where a missing secure store silently degrades instead of failing?
- Does it let an endpoint override come from somewhere the user did not knowingly
  write — a config file, an inherited environment variable, a `.env`?

### Input (T5, T6)

- Is every new parse bounded — size, nesting depth, and any allocation derived from a
  length field?
- Is invalid UTF-8 handled explicitly rather than assumed away?
- Can any input shape reach an index, a slice, an integer division, or an `unwrap`?
- Is validation at the boundary, producing a type that interior code can trust?

### Output (T7)

- Does any content that originated outside `jev` reach a terminal without
  `output::sanitize`?
- Is data still strictly on stdout and diagnostics strictly on stderr?
- Does an error message include a full request body, a full response body, or a header
  value?

### Filesystem (T8)

- Does a path from an untrusted source reach an open or a create?
- Can a symlink redirect a write outside the location the user named?
- Are new files created with restrictive permissions?

### Privacy (T9)

- Does the change cause `jev` to read a file the user did not name, walk a directory, or
  expand a glob into implicit input?
- Does it increase the volume of local content that can be sent to TypeSafe without the
  user explicitly choosing it?

### Supply chain (T10, T11, T12)

- New dependency: does it clear all five criteria in ADR-0004? Check the **transitive**
  tree with `cargo tree -p <crate>`, not just the crate.
- New or changed action: pinned to a commit SHA with a matching version comment?
- Does any job grant more permission than it needs, and is each grant commented?
- `persist-credentials: false` on every checkout?
- Does any `run:` body expand an untrusted `${{ }}` value?

### Reference material (T13)

- Was any code copied from a `references/` corpus or another community project rather
  than read, understood, and rewritten? Compare structure and naming, not just text.
- If anything was vendored deliberately, is its licence file, copyright notice, and
  provenance preserved?
- Does any change treat text found in reference material as an instruction rather than
  as data? A README in a third-party repository is a prompt-injection surface.
- Is any claim about TypeSafe API behaviour sourced from a community project instead of
  the official documentation? See `/api-compat`.

### Agent safeguards (T14)

Read this part of the diff with particular suspicion, because it is the failure mode
this repository is most exposed to:

- Was a test deleted, `#[ignore]`d, or had an assertion loosened?
- Was an `#[allow]` added, or a workspace lint downgraded?
- Was the local push gate bypassed, or was a required check made optional?
- Was a `deny.toml` entry relaxed or an `ignore` added?
- Was an instruction in `AGENTS.md` or a skill weakened or deleted?

Any "yes" needs an explicit justification in the pull request. A change that both fixes
a bug and quietly relaxes a check is two changes, and the second one is the one that
matters.

## 4. Check the invariant tests still bite

The canaries are load-bearing. Confirm they exist and still assert the negative:

- `crates/jev-config/src/secret.rs` — `Debug`, `Display`, and **nested** `Debug` are
  redacted.
- `crates/jev-client/src/transport.rs` — header values redacted, body contents absent.
- `crates/jev-cli/tests/cli.rs` — no command prints the canary key.
- `crates/jev-cli/tests/cli.rs` — the CLI credential canary coverage tests.
- `scripts/credential-canary.sh` — the separate local command and file canary.

If the change adds a new way for a secret or untrusted content to move, it needs a new
canary. Write the test that would fail if the safeguard were removed.

## 5. Report

Write findings, not reassurance. For each:

```
[severity] file:line — what is wrong
  Attack: how an adversary reaches it, and what they get
  Fix: the specific change
```

Severity: **critical** (credential disclosure, remote code execution, key sent to the
wrong host), **high** (memory or resource exhaustion from input, path traversal, release
integrity), **medium** (missing bound, missing sanitization with no known path),
**low** (hardening).

End with the threats you examined and found clear, so a reader knows what was covered.
If the change is clean, say that — but say what you checked. "Looks fine" is not a
review.
