# ADR-0008: Environment before keychain, and a separate credential namespace per endpoint

* Status: Accepted
* Date: 2026-09-19
* Supersedes: the *resolution order* in [ADR-0002](0002-security-and-credentials.md).
  Everything else in ADR-0002 — no plaintext fallback, no credential as an argument,
  `Secret` redaction and zeroization — stands unchanged.

## Context

ADR-0002 fixed the set of credential sources and the rule that there is no plaintext
fallback. It also stated an order: OS-native secure storage, then `JEV_API_KEY`, then
`JEV_API_KEY_FILE`. Implementing the transport surfaced two problems with that order,
and a third question ADR-0002 did not answer at all.

**Problem one: a stored key that outranks an exported one is a trap.** With the keychain
first, `JEV_API_KEY=sk-other jev …` quietly does *not* use `sk-other`. Every other tool
in this space — `gh`, `aws`, `docker`, the official TypeSafe SDKs — treats the
environment as the more specific statement of intent, and for good reason: it is the
mechanism people reach for to switch accounts for one command, to run CI against a
scoped key, and to work around a stale stored entry. Making the *less* specific source
win produces a failure with no visible cause.

**Problem two: the official SDK convention.** TypeSafe's Python and JavaScript SDKs read
`TYPESAFE_API_KEY` (`typesafe_sdk.constants.API_KEY_ENV`). A developer whose environment
already works with the SDK expects `jev` to work there too. ADR-0002 did not mention it.

**Problem three: what credential does a custom endpoint use?** ADR-0002 and
`docs/threat-model.md` T4 both say an endpoint override must be explicit and visible,
but neither says which key is sent. Left unanswered, the obvious implementation sends
whatever `JEV_API_KEY` holds — which means a single mistyped `--endpoint`, or a config
file in a cloned repository, exfiltrates a production TypeSafe key to an arbitrary host.
Visibility is a mitigation; it is not a control.

## Options considered

### Resolution order

1. **Keychain first, as ADR-0002 stated.** Rejected for problem one. The strongest
   argument for it — "the interactive user's deliberate choice should win" — does not
   hold, because setting an environment variable is also deliberate and is *more*
   recent.
2. **Environment first, keychain last.** Chosen. Most specific wins, which is the rule
   users already have for every other tool.
3. **A `--use-keychain` flag to force the store.** Rejected as a flag that exists to
   work around a surprising default rather than to express anything a user wants.

### The custom-endpoint credential

1. **Send whatever the TypeSafe sources hold.** Rejected: this is the exfiltration path,
   and a printed warning is not consent.
2. **Refuse custom endpoints entirely.** Rejected: a self-hosted proxy and a local mock
   are legitimate, and `jev`'s own integration tests need one.
3. **A separate environment namespace, with the OS store never consulted.** Chosen.
4. **Per-host entries in the OS store.** Rejected for now: it re-creates the confusion
   it is meant to prevent — a user who runs `jev auth login` once and later adds
   `--endpoint` would find a key already present for it — and it adds a store schema
   that must then be migrated.

## Decision

### Resolution order, official endpoint

| # | Source | Intended for |
| - | ------ | ------------ |
| 1 | `JEV_API_KEY` | CI, containers, a deliberate one-invocation override |
| 2 | `JEV_API_KEY_FILE` | secret managers that materialize a key on disk |
| 3 | `TYPESAFE_API_KEY` | an environment already configured for the official SDKs |
| 4 | OS credential store | interactive local use, via `jev auth login` |

`JEV_*` beats `TYPESAFE_*` because the CLI-specific name is the more precise statement
of intent, and because a user who has both set almost certainly set the `JEV_` one for
this tool. The file form sits above `TYPESAFE_API_KEY` for the same reason: naming a
file is more deliberate than an ambient variable a shell profile may have exported.

A source that is present but **empty** is an error naming that source, not a skip. A
blank CI secret is the classic cause of a mystifying 401.

`jev auth login` warns when it stores a key while a higher-precedence variable is set,
because the stored key would otherwise appear to have no effect.

### Resolution order, non-official endpoint

| # | Source |
| - | ------ |
| 1 | `JEV_CUSTOM_API_KEY` |
| 2 | `JEV_CUSTOM_API_KEY_FILE` |

**And nothing else.** `JEV_API_KEY`, `JEV_API_KEY_FILE`, `TYPESAFE_API_KEY`, and the OS
credential store are not consulted at all. `jev auth login` refuses to run against a
non-official endpoint and says why.

The consequence is the point: with `--endpoint https://evil.example`, there is no code
path by which a TypeSafe credential reaches that host. It is not a warning, it is not a
prompt, it is an absence of a mechanism.

Supporting rules, all tested:

- A non-official endpoint warns on **every** invocation, on stderr, naming the endpoint
  and where the setting came from. The warning is not suppressed by `--quiet`.
- `jev doctor` reports the endpoint, whether it is official, and which credential
  sources apply to it.
- Plain HTTP is refused unless the host is unambiguously loopback. `localhost.evil.example`
  and `127.0.0.1.evil.example` are not loopback.
- Redirects are not followed: a `Location` header must not be able to move a credential
  to another host.

### What has not changed

- No plaintext-file fallback, anywhere, for any source.
- No credential accepted as a command-line argument, and a test that walks every
  argument of every subcommand to keep it that way.
- No `.env` discovery, no reading of a file the user did not name.

## Consequences

- A user with a stored key who exports `JEV_API_KEY` gets the exported one. That is the
  behaviour they expect from every comparable tool.
- A user of a custom endpoint must set a second variable. That is one line of friction
  in exchange for making credential exfiltration through an endpoint override
  structurally impossible.
- `CredentialSource` gains three identifiers — `typesafe-environment`,
  `custom-endpoint-environment`, `custom-endpoint-environment-file`. Adding a variant to
  an enumerated string field is a compatible change under ADR-0003, and consumers are
  documented to tolerate unknown values.
- The environment is readable by the same user and inherited by children. Unchanged from
  ADR-0002 and still accepted; the alternatives are worse.

## Revisit if

- TypeSafe issues scoped or short-lived tokens, which would make per-endpoint storage
  meaningful in a way it is not today.
- Enough users run against self-hosted proxies that a per-host store entry, with an
  explicit `jev auth login --endpoint`, is worth the migration it would require.
