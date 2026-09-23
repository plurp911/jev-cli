# ADR-0002: Credential storage — secure storage or environment, no plaintext fallback

* Status: Accepted. The **resolution order** below is superseded by
  [ADR-0008](0008-credential-precedence-and-endpoint-isolation.md); every other decision
  here stands.
* Date: 2026-09-19

## Context

`jev` needs a TypeSafe API key on every call. The key is the highest-value asset the
tool touches: it is billable, and it grants access to the user's account.

Two usage modes have genuinely different needs:

- **Interactive local use.** A developer authenticates once and expects it to stick.
  Retyping a key is not acceptable, so something must persist it.
- **CI and headless use.** No keychain, no interactive prompt, no terminal. The key
  comes from a secret manager or a CI secret store, usually as an environment variable.

The default design in this space — write the key to `~/.config/<tool>/credentials` in
plaintext, usually with a warning nobody reads — is how keys end up in backups, in
container image layers, in synced home directories, and in `find / -name '*.json'`
output. It is a convenience decision with a security consequence the user never
consented to.

## Options considered

1. **Plaintext file by default.** Simple, universally portable, and what many CLIs do.
   Rejected: it converts one-time authentication into indefinite at-rest exposure on
   every machine and in every backup.
2. **OS-native secure storage only.** Strongest at rest. Rejected as the only option: it
   is unavailable in CI, in containers, and in headless Linux without a running Secret
   Service, which covers a large share of intended use.
3. **Secure storage with a silent plaintext fallback.** The common compromise. Rejected:
   the fallback fires exactly when the user is least able to notice — in a container, in
   CI, over SSH — and the resulting file is plaintext without the user ever choosing it.
   A warning printed once is not consent.
4. **Secure storage, then environment, with no fallback.** Chosen.

## Decision

Credential resolution order:

1. **OS-native secure storage** — macOS Keychain, Windows Credential Manager, Linux
   Secret Service / `kwallet`. Preferred for interactive local use.
2. **`JEV_API_KEY`** environment variable. Preferred for CI and headless use.
3. **`JEV_API_KEY_FILE`** — path to a file containing the key, for secret managers that
   materialize secrets on disk. `jev` reads it; it never writes it.

And these rules:

- **No plaintext fallback.** If secure storage is unavailable and no environment source
  is set, `jev` fails with an actionable error naming `JEV_API_KEY`. It does not write
  a key to disk. `jev-config` exposes exactly this as
  `CredentialSourceError::SecureStorageUnavailable`.
- **A credential is never a command-line argument.** There is no `--api-key` flag, and
  one must not be added. Arguments are visible in `ps`, in shell history, and in CI
  logs (threat model T1).
- **Credential material lives in `Secret`**, which redacts on `Debug` and `Display`,
  zeroizes on drop, and does not implement `Serialize`, `Clone`, or `Deref`. Reading the
  plaintext requires `.expose()`, so every disclosure point is greppable.
- **`jev` reports the source, never the value.** `CredentialSource::as_str` returns a
  stable identifier (`environment`, `environment-file`, `os-keychain`) that is part of
  the output contract.
- **No `.env` discovery.** Loading a file the user did not name is a supply-chain hazard:
  cloning a repository should not change where your credentials come from.
- **A user who genuinely wants plaintext can have it, explicitly.** They can write the
  key to a file they control and point `JEV_API_KEY_FILE` at it. That is an informed
  choice made by the user, not a silent default made by us.
- **The stored credential stays on the machine it was stored on.** This is what "OS
  credential store" means everywhere else in this document, and on two platforms it is
  the default: the macOS login keychain without `kSecAttrSynchronizable`, and the Secret
  Service login collection. Windows is not — `windows-native-keyring-store` creates
  credentials with `Enterprise` persistence, which writes them to the user's *roaming*
  profile, so on a domain-joined machine the key follows the user to every other machine
  they sign in to. `jev` therefore passes `persistence = Local`
  (`CRED_PERSIST_LOCAL_MACHINE`) on Windows, matching the other two platforms.

  A user who wants the key to roam can still get that: it is their credential and their
  credential manager. What they should not get is roaming they never asked for, from a
  default they never saw. Note that persistence is fixed when the secret is written, so
  anyone who ran `jev auth login` before this keeps the roaming credential until they
  log in again.

## Consequences

- A headless Linux user without a Secret Service must set `JEV_API_KEY`. This is the
  intended outcome: the error message says exactly that, and CI users were going to use
  the environment variable anyway.
- `jev login` must fail cleanly rather than degrading, and that failure path needs a
  test.
- An OS keychain dependency will be needed. It must be evaluated against ADR-0004 with
  particular care, since it will handle the key: prefer a crate with a small transitive
  tree and no C toolchain requirement, and confirm it does not log.
- Environment variables are readable by the same user and inherited by child processes.
  Accepted (threat model T1); the alternatives are worse.

## Revisit if

- A cross-platform secure-storage mechanism appears that works in containers and CI.
- TypeSafe introduces short-lived tokens or OAuth device flow, which would change the
  persistence question substantially.
