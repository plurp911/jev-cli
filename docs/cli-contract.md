# CLI contract

What a script, a CI job, or an AI agent can depend on.

Rationale is in [ADR-0003](adr/0003-cli-compatibility.md).

## The rule

**The command line is the product. The Rust crates are not.** Crates are
`publish = false` and promise no API. Internals may be restructured at any time.

## Streams

| Stream | Carries |
| --- | --- |
| stdout | Data, and nothing else. Requested output only. |
| stderr | Diagnostics, warnings, progress, errors, and help shown because of an error. |

`jev --help` writes to stdout and exits `0`, because the user asked for it. A usage
*error* writes to stderr and exits `2`.

Guaranteed consequences:

- `jev … > file` captures exactly the data.
- `jev … | head` is not an error. A broken pipe exits `0`.
- `jev … 2>/dev/null` never suppresses data.
- `--quiet` silences diagnostics but never data, and never the non-official-endpoint
  warning.

## Exit codes

Defined in `crates/jev-cli/src/exit.rs` and locked by a test. A code may be added; an
existing code is never repurposed.

| Code | Meaning | What to do |
| --- | --- | --- |
| `0` | Success. The API answered, or a command that makes no API call finished. `jev doctor` is a report, not a check, and exits `0` even when its `--live` probe fails. | Read the answer. For `doctor --live`, read `.live.ok`, or use `jev models`, which fails when the API does. |
| `1` | A `--require` gate was evaluated and did not hold. Also `jev eval` when no threshold reaches `--target`, and `jev config get` on a setting that is not set. | The model's answer did not meet your condition. |
| `2` | Usage error: unknown flag, bad argument, malformed input, or a request the API rejected as invalid. | Fix your command or your request. |
| `3` | Authentication failed, or no credential was available. | Fix your credentials. |
| `4` | The API could not be reached, timed out, was overloaded, or returned something undecodable. | Retry later. |
| `5` | A batch finished with some rows failing, or stopped before reaching every record. | Successful rows were still written; inspect the failures and `--resume`. |
| `6` | A `--require` gate could not be evaluated. | Your expression names a question or field the response does not contain. |
| `70` | Internal error. This is a bug. | Please report it. Do not retry; it will fail the same way. |
| `74` | Could not write output: a full disk, a revoked permission, a vanished mount. | Fix the environment and retry. Deliberately not `70`: it is not a bug in `jev`, and telling you to file one would waste your time. |
| `130` | Interrupted (`SIGINT`). | Nothing to do. Whatever was written before the interrupt is complete and readable; `jev map --resume` continues from it. |

`70` and `74` are the two statuses that write **nothing** to stdout. Every other
non-zero status above still writes the document it produced, so a script that reads
stdout should check the status first.

### The distinction that matters most

**A successful API call is exit `0`, whatever the model said.**
`jev noul "is this spam?"` exits `0` when the evaluation succeeds, even if the
probability comes back `0.01`. Model semantics reach the exit status only when you ask
for them with `--require`.

**`1` and `6` are different on purpose.** `1` means the gate was evaluated and the
answer did not satisfy it. `6` means the gate could not be evaluated at all — a typo in
a question id, a field the primitive does not have, a response that did not contain the
answer. A gate that cannot be evaluated is never a pass, and it is never reported as a
negative judgment, because those call for different responses from whoever reads the
status.

**`2`, `3`, and `4` are worth branching on.** `2` means fix your command, `3` means fix
your credentials, `4` means try again later.

**`3` beats `5` in a batch.** A rejected credential is a property of the credential, not
of a record, so `jev map` stops at the first one and exits `3` rather than sending a
doomed request for every remaining record and then reporting a partial batch. A job that
branches on `3` to re-authenticate needs to actually see it.

**In `jev map`, `--require` routes; it does not gate.** It classifies each answered row
and, with `--review-file`, sends the ones that did not pass somewhere else. It never
changes the exit code, which keeps reporting whether the API answered. The counts are in
the summary's `gate` object and the verdict is on every row, so a caller that wants to
fail on them can, explicitly. The single-request commands are where a gate becomes an
exit status.

### Thresholds are yours

No threshold in `jev` has a default, and none will. A gate's numbers are a claim about
your data and the cost of being wrong, and TypeSafe's own guidance is to *"start with
conservative thresholds, test with your own data, and adjust as you observe results"*
(<https://docs.typesafe.ai/confidence>). Two properties of the API that a gate has to
respect:

- **`confidence` exists only on Choice and Score.** It is computed from the
  `probabilities` already in the answer and measures how concentrated they are — not the
  probability that the answer is correct.
- **A Noul has no `confidence`.** `<id>.confidence` on one is exit `6`, deliberately, and
  `jev` does not synthesize a value the API does not return. Express a Noul's uncertainty
  as a band on its probability instead.

`jev eval` is how you stop guessing without `jev` guessing for you: it measures a
question against examples **you** labelled and reports a threshold together with the
objective it was chosen under. It still does not pick the objective, because that is the
part that encodes what being wrong costs you.

See [thresholds and what `confidence` does and does not
mean](commands.md#thresholds-and-what-confidence-does-and-does-not-mean) and
[`jev eval`](commands.md#jev-eval).

## Output formats

`--output text` (default)
: For humans. **Not stable. Do not parse it.** Wording, layout, spacing, alignment, and
colour may change in any release, including a patch release.

`--output json`
: For machines. Stable and versioned. One JSON document, on one line, newline-terminated
— except `jev map`, which emits one document per line (JSONL) followed by a summary
document.

`--value`
: One scalar and a newline: a Noul probability, a Choice option name, or a Score value.
Nothing else. For the shell cases where JSON is more than you need. Mutually exclusive
with `--output json`.

Two commands do not follow `--output`, which is deliberate and stated here so it is not
discovered:

- `jev completions` writes a shell script whatever `--output` says. There is no JSON
  rendering of a completion script, and a wrapper that sets `output = "json"` in the
  configuration file still gets the script it asked for rather than an error.
- `jev map` always writes JSONL, including under `--output text`. Its output is a stream
  of one document per record; a human rendering of a batch of ten thousand rows is not a
  thing this command has. `--dry-run` on `map` likewise always prints its document.

### Schema identifiers

Every JSON document carries a `schema` field so a consumer can detect a change rather
than discover one.

| `schema` | Emitted by |
| --- | --- |
| `jev.evaluation/v1` | `noul`, `choice`, `score`, `ask` |
| `jev.models/v1` | `models` |
| `jev.doctor/v1` | `doctor` |
| `jev.auth/v1` | `auth login`, `auth status`, `auth logout` |
| `jev.config/v1` | `config` |
| `jev.dry-run/v1` | any command with `--dry-run` |
| `jev.map.row/v1` | one `map` result line |
| `jev.map.summary/v1` | the `map` summary line |
| `jev.eval/v1` | the `eval` report |
| `jev.eval.row/v1` | *read*, not written: the row format `eval --dataset` accepts |
| `jev.mcp.map/v1` | the `map` tool of `jev mcp serve`: the `map` rows and summary in one object |

The full shapes are in [`output-schema.md`](output-schema.md).

### What "stable JSON" means

Compatible, may happen in any release:

- Adding a field to an object.
- Adding a variant to an enumerated string field. Consumers are documented to tolerate
  unknown values — including new `CredentialSource` identifiers and new answer `type`
  values.
- Adding a new `schema` value for a new command.

Breaking, requires a major version and a changelog entry:

- Removing or renaming a field.
- Changing a field's type, or its meaning.
- Changing an existing `schema` value's shape without incrementing the version in it.

Consume JSON by field name. Do not depend on key order, on whitespace, or on the absence
of a field you have not seen.

## Stable surface

- Command and subcommand names.
- Flag names, short forms, and meanings.
- Exit code meanings.
- Which stream each kind of output goes to.
- `--output json` shapes, including `schema` values.
- Environment variable names.
- `CredentialSource` identifiers: `environment`, `environment-file`,
  `typesafe-environment`, `os-keychain`, `custom-endpoint-environment`,
  `custom-endpoint-environment-file`, `anonymous` (explicit loopback local providers).
- Configuration setting names.
- For `jev mcp serve`: the server name `jev`, the tool names, and each tool's input and
  output schema, pinned in `crates/jev-cli/tests/snapshots/mcp-tools.json`. The same
  rules apply: a new optional argument or result field is compatible, and a removed or
  renamed one is not. Tool descriptions are wording, and are not stable.

## Not stable

- All human-readable text: help wording, error wording, colours, ordering, spacing.
- Anything in the Rust crates.
- Timing and performance characteristics.
- The exact bytes sent to the selected provider, which follow that provider's contract.
  The supported command and output surfaces retain their compatibility promises.

## Environment

`jev` reads exactly these variables and no others. It never loads a `.env` file, and it
never reads a file you did not name.

| Variable | Effect |
| --- | --- |
| `CLOUDFLARE_ACCOUNT_ID` | Nonsecret account ID, consulted only with explicit Cloudflare provider selection. The account flag overrides it; it overrides configuration. It never selects a provider. |
| `JEV_API_KEY` | TypeSafe API key. Highest precedence. |
| `JEV_API_KEY_FILE` | Path to a file containing the key, for secret managers. |
| `TYPESAFE_API_KEY` | The official SDK convention, honoured so `jev` works where the SDKs already do. |
| `JEV_CUSTOM_API_KEY` | Credential for a **non-official** endpoint. Never used for the official one, and vice versa. |
| `JEV_CUSTOM_API_KEY_FILE` | File form of the above. |
| `JEV_CONFIG_DIR` | Overrides the configuration directory. |
| `NO_COLOR` | Set to anything, including empty, to disable colour. See <https://no-color.org>. |
| `JEV_NO_KEYCHAIN` | Diagnostic. Set to a non-empty value other than `0`/`false`/`no` to make `jev` report secure storage as unavailable without consulting it. It reduces capability only: it supplies no credential, redirects none, and does not enable any plaintext path. `jev auth login` then fails as it does on a machine with no store, and `jev doctor` names the variable as the reason. The integration suite sets it so tests never touch a developer's real keychain. |

`jev` does **not** read `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, or `NO_PROXY`. A proxy
named only by the environment would move every request somewhere the user did not name,
silently and without appearing in `jev doctor`, so proxy inheritance is disabled in the
transport. Adding proxy support would need an ADR, a documented variable, a `doctor`
line, and the same standing warning a custom endpoint gets.

Credential precedence is documented in
[ADR-0008](adr/0008-credential-precedence-and-endpoint-isolation.md) and reported by
`jev doctor`.

## Additional providers and media

`--provider` adds `cloudflare`, `ollama`, `llamacpp`, and `huggingface` alongside the existing
`typesafe` default. The nonsecret configuration keys `provider` and
`cloudflare_account_id`, `--cloudflare-account-id`, repeated `--image`, Cloudflare
`--reject-if-busy`, Ollama `--keep-alive`, `map --images-field`, and the local
Python bridge's `--video-frame`, `--video-fps`, `--max-length`, `--max-state-tokens`,
`--media-kwargs`, and
`map --videos-field` are additive
surfaces. Existing exit codes and output schema identifiers apply to every provider.

TypeSafe sources remain exclusive to its official endpoint. Cloudflare and explicit
remote servers use the existing custom credential namespace. Local protocols on
loopback use no credential and never consult credential files or secure storage.
Doctor endpoint metadata and evaluation JSON add `provider` and
`cloudflare_account_id` fields. MCP accepts embedded images and prepared video
frames rather than filesystem paths. See [Clef](clef.md) for
provider contracts, bounds, setup, and features not exposed by upstream endpoints.

## Configuration

One file, in the user's configuration directory, never in the working directory:

| Platform | Path |
| --- | --- |
| Linux and other Unix | `$XDG_CONFIG_HOME/jev/config.toml`, else `~/.config/jev/config.toml` |
| macOS | `~/Library/Application Support/jev/config.toml` |
| Windows | `%APPDATA%\jev\config.toml` |

`jev config path` prints it. `--no-config` ignores it.

Settings: `color`, `endpoint`, `max_input_bytes`, `model`, `output`, `retries`,
`timeout_seconds`, `provider`, `cloudflare_account_id`. **A credential is not a setting.** A key whose name looks like one —
at any nesting level — is a load error, not an accepted value.

Account resolution additionally includes `CLOUDFLARE_ACCOUNT_ID` as described above.
Precedence for other settings: command-line flag, then the configuration file, then the
built-in default. `NO_COLOR` sits between the flag and the file, for colour only: it
overrides `color = "always"` in the file, and `--color always` overrides it.
The saved `endpoint` belongs to the saved `provider` (TypeSafe when absent).
Selecting a different provider does not inherit it; that provider's default base
URL applies unless `--endpoint` is supplied. Selecting the same provider explicitly
preserves its saved endpoint.

`timeout_seconds` is bounded at 3600. A flag beyond that is a usage error; a value
beyond it in the configuration file is clamped, so a stale setting does not make every
invocation fail.

## Guarantees about what `jev` will not do

Stated as a contract because scripts and CI jobs depend on them:

- It makes no network request other than the API call you asked for. No telemetry, no
  analytics, no update check. `jev doctor` makes none at all unless you pass `--live`.
- It reads no file you did not name, and walks no directory.
- It writes no file unless you asked it to. `jev config set` and `unset` mutate the
  explicit configuration; `jev map --output-file` and `--review-file`, and
  `jev eval --report`, write to the named paths. `jev auth login` and `logout`
  explicitly mutate OS secure storage, never a plaintext credential file. On Unix, a file `jev`
  creates is `0600` — batch rows hold the model's answers about your state. A file you
  created keeps the permissions you gave it.
- It caches nothing. `jev map --resume` reads the output file you specified; it never
  replays a stored model judgment.
- The CLI executes no code from configuration, input, or a plugin. The separately
  launched Python bridge explicitly executes the named publisher model code at
  startup; see [Clef](clef.md). `--require` is a parsed
  expression, never a shell command; there is no `eval` in this project.
- It never prints your credential.
- It never sends a TypeSafe credential to a non-TypeSafe host.

## Interface versioning

Until `1.0.0`, the contract above is the intent and breaking changes are possible but
must be documented in `CHANGELOG.md`. From `1.0.0`, a break to anything under "Stable
surface" requires a major version.
