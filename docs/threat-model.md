# Threat model

Status: current as of the implemented command surface. Revise whenever the attack
surface changes — a new input source, a new output sink, a new credential path, or a new
release channel.

This document explains *what we are defending against*. The resulting rules are in
[`AGENTS.md`](../AGENTS.md) §4–§5, and the commitments users can rely on are in
[`SECURITY.md`](../SECURITY.md).

## What is being protected

| Asset | Why it matters |
| --- | --- |
| The user's TypeSafe API key | Direct financial loss and impersonation. The highest-value asset here. |
| The user's content | Source code, customer records, and internal documents pass through `jev` as *state*. Anything sent leaves the machine. |
| The integrity of `jev` itself | It runs on developer machines and may hold real credentials in the environment. A compromised `jev` is a compromised supply chain. |
| The user's terminal and filesystem | `jev` renders remote content and may write files. |

## Who we are defending against

| Adversary | Capability |
| --- | --- |
| Network attacker | Observes or modifies traffic; controls DNS or a proxy. |
| Malicious content author | Controls text that reaches `jev` as input — a file, stdin, a diff, a ticket body. |
| Malicious API responder | Controls what comes back, either as a hostile custom endpoint or through a compromise upstream. |
| Local unprivileged observer | Reads `ps`, shell history, world-readable files, and build logs. |
| Supply-chain attacker | Compromises a crate, a GitHub Action, a release artifact, or a maintainer account. |
| A well-meaning AI agent | Modifies this repository and removes a safeguard without understanding it. |

Out of scope: a compromised operating system, a malicious local administrator, physical
access, and vulnerabilities in the TypeSafe service itself.

---

## Threats and controls

### T1. API key theft from the environment or process table

A key passed as `--api-key sk-...` appears in `ps` output for every user on the
machine, in shell history, and in build logs, and it is captured by process accounting.

**Control.** `jev` never accepts a credential as a command-line argument — there is no
such flag, and one must not be added. Credentials arrive through the environment, a
file named by an environment variable, or the OS credential store; the order is in
ADR-0008. `jev auth login` prompts without echo, or reads from stdin.
**Verification.** `there_is_no_way_to_pass_a_credential_as_an_argument` in
`crates/jev-cli/src/cli.rs` walks every argument of every subcommand and fails if any
name looks like a credential, so the rule survives a new subcommand nobody re-read this
document before adding.
**Residual risk.** The environment of a process is readable by the same user and by
root, and is inherited by children. This is accepted; it is the standard mechanism and
the alternative is worse.

### T2. Secrets in logs, errors, and panic payloads

The realistic leak is not `println!("{key}")`. It is a `{:?}` of a config struct in an
error path, an HTTP error that includes the request headers, or a panic message.

**Control.** `jev_config::Secret` renders `<redacted>` for both `Debug` and `Display`,
does not implement `Serialize`, `Clone`, or `Deref`, and zeroizes on drop; reading it
requires the greppable `.expose()`. `jev_client::Request`'s `Debug` redacts every
header value and prints only the body's length. No error variant in `jev-client` may
carry a header value or a full body.
**Closed follow-up.** The credential does **not** travel inside `Request`. It is a
separate argument to `Transport::execute`, typed as `jev_client::Credential`, which is
not `Clone`, not `Serialize`, and zeroizes on drop. The plaintext becomes an
`Authorization` header only inside the concrete transport, in a `Zeroizing<String>`
built at the moment of sending. A retry loop therefore cannot multiply copies, because
there is no copy in the structure it clones.
**Residual risk.** Once the header value is handed to the HTTP library, what that
library copies into its own buffers is outside our control and is not zeroized. The
mitigation is that the window is one function call; eliminating it entirely would mean
writing our own HTTP client, which would be a far larger risk.
**Closed follow-up.** An endpoint that quotes the key back in an error body -- a proxy,
a debugging echo server, a hostile custom host -- no longer has it shown. `jev-client`
replaces the credential in API error text before the message is built, below both the
CLI, which prints it, and `jev mcp serve`, which returns it into an agent's context.
**Verification.** Canary unit tests in `secret.rs`, `credential.rs`, and
`transport.rs`; `the_credential_never_enters_the_request_struct` in `jev-client`;
`the_authorization_header_is_sent_and_nothing_else_carries_the_key` in
`crates/jev-client/tests/transport.rs`, which checks against a real socket that no other
header carries the key; `keyring_errors_are_converted_through_display_not_debug` in
`jev-config`, because `keyring_core::Error::BadEncoding` carries the raw stored bytes
and a `{:?}` of it would print them. `scripts/credential-canary.sh` runs outside the
Rust test suite with fake keys and scans both streams and files. It omits real login and
logout operations because those could change the maintainer's secure store.

### T3. Plaintext credential files

A CLI that writes `~/.config/jev/credentials` in plaintext makes every backup, every
container image layer, and every misconfigured sync tool a credential leak.

**Control.** There is **no plaintext fallback**. Secure storage or environment
injection only; if neither is available, `jev` fails with an actionable error naming
every source it tried. See [ADR-0002](adr/0002-security-and-credentials.md).

The configuration file is the other way this goes wrong, and it is closed separately: a
setting whose *name* looks like a credential — at any nesting level — is a load error,
not an accepted value, so the file cannot quietly become a credential store by hand
either.
**Verification.** `an_unavailable_store_fails_every_operation_the_same_way` in
`jev-config`; `a_credential_shaped_key_is_refused` and
`a_credential_hidden_in_a_nested_table_is_also_refused` in `jev-config`; and
`config_refuses_to_store_a_secret` in `crates/jev-cli/tests/cli.rs`, which additionally
asserts that nothing was written.

### T4. A malicious custom endpoint receives TypeSafe credentials

`--endpoint https://evil.example` is credential exfiltration wearing a feature's
clothes. The dangerous variant is a *silently inherited* override — from a config file
in a cloned repository, or an environment variable set by a CI template.

**Control.** The control is *namespace isolation*, not visibility. A non-official
endpoint resolves its credential from `JEV_CUSTOM_API_KEY` or `JEV_CUSTOM_API_KEY_FILE`
and from nowhere else: `JEV_API_KEY`, `TYPESAFE_API_KEY`, and the OS credential store
are not consulted at all, and `jev auth login` refuses to run against one. There is no
code path by which a TypeSafe credential reaches another host. See ADR-0008.

Layered on top of that: the default is `https://api.typesafe.ai`; plain HTTP is refused
unless the host is unambiguously loopback (`localhost.evil.example` and
`127.0.0.1.evil.example` are not); redirects are never followed, so a `Location` header
cannot move a credential; the endpoint and where the setting came from are reported by
`jev doctor`; a warning is printed on every invocation and is **not** suppressed by
`--quiet`; and a URL carrying userinfo, a query, or a fragment is rejected.

**"Unambiguously" is load-bearing, and it was once wrong.** `jev` decides whether
cleartext is acceptable by reading the host string; the operating system then resolves
that *same string* to decide where the bytes go. If the two readings can differ, the
decision means nothing. They can: `getaddrinfo` follows `inet_aton`, in which a leading
zero marks an octal literal, while Rust's `"0000127".parse::<u8>()` is `Ok(127)`. So
`http://0000127.00000000000012.077.2` read as loopback to `jev` and resolved to
**87.10.63.2** — a public address — for the kernel. A credential would have crossed the
internet in cleartext with no warning. The `endpoint_url` fuzz target found it; the fix
accepts an IPv4 octet only as one to three digits with no leading zero, the one
spelling on which every resolver agrees. `0177.0.0.1` is now rejected even though it
*does* resolve to loopback: a false negative costs a retype, a false positive leaks a
key. The input is kept in `fuzz/seeds/endpoint_url/octal-ambiguous-loopback.txt`.
**Verification.** `a_custom_endpoint_cannot_use_the_typesafe_credential` and
`a_custom_endpoint_uses_its_own_credential_and_always_warns` in
`crates/jev-cli/tests/cli.rs`; `a_custom_endpoint_cannot_reach_a_typesafe_credential` in
`jev-config`; `a_redirect_is_not_followed` in `crates/jev-client/tests/transport.rs`;
`an_octal_ambiguous_host_is_not_loopback` and
`canonical_loopback_addresses_are_still_accepted` in `jev-client`; and the
`endpoint_url` fuzz target, which asserts that every URL the parser accepts is
TLS-protected or unambiguously loopback.

### T5. Malicious or malformed input

Content reaching `jev` as state is attacker-influenced in the realistic cases: a
support ticket, a pull request diff, a log line.

**Control.** Bound everything — response size, nesting depth, and any allocation
derived from a length field. Never trust a `Content-Length`. Handle invalid UTF-8
explicitly rather than assuming. No indexing or integer division outside tests
(`clippy::indexing_slicing`, `clippy::integer_division` are denied), so a panic cannot
be reached by input shape. Values are validated at the boundary:
`jev_core::Probability` rejects NaN, infinities, and out-of-range values during
deserialization, so a hostile response cannot produce an impossible probability
downstream. Beyond the documented shape, a response is checked for internal
consistency: a Choice whose selection is absent from its own distribution, and a Score
outside the range its own legend spans, are both refused rather than reported as
answers.

Input is validated before a request is built, so a mistake costs nothing: empty input,
invalid UTF-8, binary content (detected by a NUL byte), a directory, and input over the
byte limit are all rejected, and nothing is ever silently truncated.
**Verification.** `crates/jev-cli/tests/cli.rs` covers each rejection end to end;
`an_internally_inconsistent_response_is_refused` in
`crates/jev-client/tests/compatibility.rs`; and the `api_response` and `state_input`
fuzz targets assert the invariants rather than only the absence of a crash.

### T6. Gigantic or deeply nested JSON

A 4 GB response or a document nested ten thousand deep is a denial of service against
the user's machine, and deep nesting can overflow the stack during recursive parsing.

**Control.** The response body is read through `Read::take` at a 16 MiB cap, so a
`Content-Length` — which is attacker-controlled — never sizes an allocation. Nesting is
bounded at 64 levels, checked *iteratively* with an explicit stack, because a recursive
depth checker over attacker-supplied JSON is itself the bug. `serde_json` applies its
own 128-level limit while parsing, so there are two independent bounds. Input from
stdin and files is capped the same way, adjustable with `--max-input-bytes`, and a file
named by `JEV_API_KEY_FILE` is capped at 4 KiB.
**Verification.** `an_oversized_response_is_refused_rather_than_buffered` in
`crates/jev-client/tests/transport.rs`; `a_deeply_nested_response_is_refused` and
`oversized_input_is_refused_and_explains_the_limit` in `crates/jev-cli/tests/cli.rs`;
`a_very_deep_document_does_not_overflow_the_stack` in `jev-core`, which walks a
100,000-level document; and the `api_response` and `state_input` fuzz targets.

### T7. Terminal escape and control-character injection

Text containing `\x1b[2J` can clear the screen; other sequences can forge output, hide
text, retitle the window, or, on some terminals, inject input. The source can be an API
response or a file the user did not write.

A naive `is_control` filter is not enough. Unicode *format* characters (category `Cf`)
are not control characters, yet U+202E RIGHT-TO-LEFT OVERRIDE reorders displayed text
and U+200B/U+FEFF hide it — the Trojan Source class of spoofing.

**Control.** `jev_cli::output::sanitize` escapes C0 controls, `DEL`, the C1 range, and
the `Cf` format characters — bidi overrides and isolates, zero-width characters, the
soft hyphen, and the BOM — leaving newline and tab. Untrusted text is sanitized before
it reaches a terminal.
Colour is a `jev`-side decision that honours `--color` and `NO_COLOR`; it is never
carried by content.

### T8. Path traversal and symlink abuse

A path from a response or a config file — `../../.ssh/authorized_keys`, or a symlink
into `/etc` — turns an output flag into arbitrary file write.

**Control.** `jev` writes exactly four files, and only when asked: the configuration
file, `jev map --output-file`, `jev map --review-file`, and `jev eval --report`. Every
one is named by the user on the command line. No path is ever constructed from a
response, from input content, or from a configuration value, and no output path is ever
derived from an input path. The three result files are opened with `create` and, on
Unix, mode `0600`, for the reason given below: they hold the model's judgments about the
user's own content.

A symlink at a path the user named **is** followed, deliberately — that is what naming
it means, and refusing would break the ordinary case of a symlinked output directory.
What `jev` does not do is follow a symlink into a location the user did not name, which
it cannot, because there is no path it did not receive as an argument. The collision
check between `--output-file`, `--review-file`, and `--input` is a guard against a
mistake and is documented as such in the code: it compares resolved directories and file
names, so two paths that reach one file through a link are not caught. Nothing depends
on that check for safety.

The
configuration file is written atomically — to a temporary file in the same directory,
then renamed — and `fsync`ed before the rename so a crash cannot leave an empty
configuration.

**On Unix** it sits inside a `0700` directory with mode `0600`, and a pre-existing
world-writable directory is tightened. **On Windows** neither is done: no ACL is set,
and the file inherits from its parent. For the default `%APPDATA%\Roaming` that is
user-only and therefore adequate; for a `JEV_CONFIG_DIR` the user points at a shared
location — `C:\ProgramData`, a build directory — it is not, and `jev` does not tighten
it. The file holds no credential (a secret-shaped key is a load error), so what is at
stake is another local user editing the `endpoint`, which `jev doctor` shows and which
the separate credential namespace for non-official endpoints already contains.

A symlink the user names is followed, deliberately: they named it, and refusing would
break a legitimate setup. What `jev` does not do is *construct* a path it was not given.
**Verification.** `saved_files_and_directories_are_private` and
`saving_leaves_no_temporary_file_behind` in `jev-config`.

### T9. Accidental transmission of sensitive content

The quiet catastrophe: a developer pipes something into `jev` and ships proprietary
source or customer data to a third party without noticing.

**Control.** `jev` transmits only what the user explicitly supplied in this invocation.
No directory walking, no glob expansion into implicit input, no automatic context
gathering, no `.env` loading, no caching of request or response content. Any future
feature that makes bulk local content easy to send needs an explicit confirmation path
and an ADR.

`jev map` is such a feature, and
[ADR-0010](adr/0010-batch-evaluation-and-semantic-routing.md) §2 is that ADR. It records
why the confirmation path is `--dry-run`, `--state-field`, a pre-flight count printed on
stderr at ordinary verbosity, and `0600` row files rather than a prompt: `jev map` reads a pipe, so a prompt would be
skipped for every real invocation, and it sends only records the user named — there is no
discovery step. A feature that *does* read content the user did not name reopens the
question, and that answer does not carry to it.

### T9b. Ground truth reaching the API

`jev eval` holds the answers. A dataset row carries both the state to send **and** the
label — the user's own judgment, which is often the most commercially sensitive part of
the file and is, by construction, the thing the model is not supposed to see.

**Control.** Structural, not procedural. A row's ground truth lives in a `Label`, and
there is no path from a `Label` into an `EvaluationRequest`: the request is built from
`state` alone, in one expression, with `labels` not in scope. Comparison against ground
truth happens locally after the answer returns, the same way `--require` is evaluated
locally. A test asserts that no request body carries a `labels` object and that the body
has exactly the fields the official API request body has — so a field added to the
request by accident fails the suite rather than shipping.

The same reasoning covers the report: `--report` writes metrics and, only under
`--show-rows`, per-row labels and answers. It is a local file, `0600` on Unix, and
nothing about it is transmitted.

### T10. Compromised dependency

A malicious release of a transitive crate executes in `build.rs` or at runtime, with
access to the environment — which holds the API key.

**Control.** Deliberately few runtime dependencies. `Cargo.lock` is committed, and
local verification builds with `--locked`. `cargo-deny` checks the RustSec advisory
database, a license allow-list, banned crates, and registry sources before each push.
`openssl` is banned, as is `tokio` outside its documented MCP exception. The local TLS
check resolves selected features and fails if native TLS enters the build. Dependabot
proposes updates; majors are reviewed individually. There is no scheduled advisory run.
**Residual risk.** A compromise published and pulled before any advisory exists. Small
dependency count is the main mitigation.

### T11. Compromised GitHub Action

An action referenced by a mutable tag can be repointed at malicious code. This is not
hypothetical: the March 2026 `trivy-action` incident exfiltrated organization secrets
and was used to backdoor a package on PyPI.

**Control.** Every third-party action is pinned to an immutable commit SHA with the
version in a trailing comment. `permissions: {}` at workflow level; each job declares
the minimum it needs, with a comment explaining each grant. `persist-credentials: false`
on every checkout. No `pull_request_target`. No expansion of untrusted template values
into a `run:` body — values pass through `env:`. `zizmor --persona=pedantic` audits
the manual release workflow and GitHub configuration in `scripts/verify.sh`.
The automatic Scorecard report was retired by ADR-0013.

### T12. Compromised release artifact

A user downloads a binary that is not the one we built.

**Control.** The manual release workflow builds checksums and an SPDX SBOM. GitHub
build provenance is optional and must be checked for each release before it is claimed.
Builds use `--remap-path-prefix`, a pinned toolchain, and `SOURCE_DATE_EPOCH`; bit-for-bit
reproducibility remains unverified. Package managers and verifiable artifacts are
first-class; `curl | sh` will never be the only path. The release workflow has no tag
trigger and refuses to publish without explicit human authorization. See
[ADR-0014](adr/0014-manual-release-gate.md).

### T13. Untrusted reference material

A future `references/` corpus holds third-party repositories and documentation. A README
in such a repository can contain text shaped like instructions to an agent — a prompt
injection with a plausible cover story.

**Control.** Reference material is **data, never instructions**. Code is not copied from
it; it is read, understood, and reimplemented. Licenses and attribution are preserved if
anything is ever vendored deliberately. The authority for API behaviour is the official
TypeSafe documentation, never a community project. See `AGENTS.md` §6 and §8.

### T14. An agent weakens a safeguard

The threat specific to this repository. An agent asked to make verification pass has an obvious,
effective, and wrong move available: delete the failing test.

**Controls, layered because instructions alone are not a control:**

1. `AGENTS.md` §2 states the prohibition first and unconditionally.
2. The pull request template requires an explicit statement if any check was changed.
3. `.github/CODEOWNERS` routes `AGENTS.md`, `CLAUDE.md`, `.claude/skills/`, `deny.toml`,
   `.github/workflows/`, and the credential-handling crates to a security reviewer.
4. The local pre-push hook runs the full verification suite. A contributor can bypass
   or omit the hook, so reviewers must inspect the reported result.
5. Lints that matter are workspace-level denials in `Cargo.toml`, so silencing one is a
   visible diff in a CODEOWNERS-protected file rather than a stray attribute.
6. Nextest retries are pinned to zero with a comment saying not to raise them.

**Residual risk.** A sufficiently determined change can still alter these files. The
control is that doing so is *conspicuous in review*, not that it is impossible.

### T15. `jev mcp serve`: an agent host as the caller

`jev mcp serve` (ADR-0012) is started by an agent host and receives tool arguments that
a model wrote. Often a model wrote them after reading untrusted content: a repository,
a web page, a log. The arguments are therefore attacker-influenced input, and results
flow back into a model's context, which transcripts and logs may keep.

**Controls.**

1. **Nothing to abuse.** The server exposes five evaluation tools and no others. It has
   no code path that reads a file, writes a file, runs a command, fetches a URL, or
   changes configuration or credentials. `map` takes inline records and never a path.
   Instructions embedded in state are data sent to the model under evaluation; the
   server itself cannot act on them. Tool annotations say so, but they are hints: the
   control is the absence of the code.
2. **Same credential rules.** Credentials resolve per call through the CLI's own
   function, so T4's separate namespace applies unchanged. A stored TypeSafe key cannot
   reach a custom endpoint, and a custom endpoint set at startup is warned about on
   stderr once. No key is ever an argument, a tool parameter, or MCP configuration.
3. **stdout is the protocol.** Nothing else writes there: `main.rs` no longer holds the
   stdio locks, the CLI's renderers are unreachable from the MCP module, and no
   `tracing` subscriber is installed. A stray line would corrupt the session.
   `stdout_carries_only_protocol_messages_and_stderr_is_silent_by_default` asserts it.
4. **Bounded input.** State obeys `--max-input-bytes`. `map` accepts at most 100
   records, with their states summing to at most `--max-input-bytes` and an estimated
   result of at most 80 KiB, and it refuses before sending anything. One protocol line
   is capped at 8 × `--max-input-bytes`, never below 16 MiB or above 256 MiB, and a longer
   one ends the session. Record ids count toward the `map` result estimate, so a small
   state cannot smuggle a large result through its id.
   serde_json's nesting limit and `MAX_JSON_DEPTH` apply as for every other input.
5. **No listener.** stdio only. There is no port, no local HTTP, and no connection
   another process could open.
6. **No echo.** Results never contain the state they judged. `map` rows carry a digest
   of the state and nothing else. The key is redacted from API error text (T2).

**Residual risk.**

* Every state is transmitted to TypeSafe. That is the tool's purpose, and each tool
  description says so. A host that lets a model call tools without approval can send
  what the model has read.
* A repeated call is a second billed request. `idempotentHint: false` tells the host,
  but a host may ignore it.
* A host that logs tool results keeps the judgements, though not the state.

**Verification.** `crates/jev-cli/tests/mcp.rs` drives the real binary through the
official Rust SDK's client and over raw pipes. It covers the canary on every stream,
with and without `--verbose`, and on failure paths. It covers a TypeSafe key present
alongside a custom endpoint, where no request is sent. It also covers the limits,
cancellation, shutdown on stdin close, and SIGINT.

---

## Known gaps

Tracked honestly rather than quietly:

Closed since the initial version: T2's `.expose()` follow-up, T4, T6, and T8 are
implemented and tested; T7 now guards API-returned text as well as local text; the OS
keychain is integrated; and fuzzing exists.

Open:

- **Reproducible builds are not verified.** The toolchain is pinned, paths are remapped,
  `SOURCE_DATE_EPOCH` is set, and the graph is locked, but bit-for-bit reproducibility
  has not been demonstrated. It is therefore not claimed
  (`docs/release-verification.md`).
- **No Windows code signing, no macOS notarization.** Both need an organizational
  identity and paid certificates this project does not have. Documented rather than
  papered over.
- **The credential is unprotected inside the HTTP library.** See the residual risk under
  T2.
- **`jev map --output-file` appends without locking.** Two concurrent runs writing to the
  same file would interleave lines. Documented rather than prevented; a lock would be a
  cross-platform dependency for a case the user controls.
- **No client-side rate limiting, and no batch-level circuit breaker.** The API's limits
  are published but documented as changing without notice, so `jev` reacts to 429 rather
  than predicting it, and `--concurrency` is the user-facing control. At batch scale
  that understates the effect: a `jev map` or `jev eval` run against an account that is
  already throttled will, without `--fail-fast`, keep attempting every remaining row and
  exhausting its retry budget on each. It spends no model tokens — a 429 is rejected
  before inference — but it prolongs an outage and can mean a great many wasted
  requests. `--fail-fast` stops on the *first* failure of any kind, including a single
  transient blip, so it is not a usable stand-in for "notice sustained throttling and
  stop". A consecutive-failure threshold, distinct from `--fail-fast`, is the obvious
  fix and is not implemented.
- **Certificate pinning is not planned.** It breaks corporate TLS inspection for little
  gain against the modelled adversaries.
- **The live integration tests have not been run.** Everything is verified against
  recorded official documents and a local mock; nothing here has been checked against
  the real API. See `docs/api-compatibility.md`.
