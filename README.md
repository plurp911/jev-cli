# jev

A fast, secure, Unix-friendly command-line interface for [TypeSafe AI][typesafe]'s
System One API and the [Jev][jev-docs] model, plus Cloudflare's Clef and Clef Flash.

Version 0.3.0 supports hosted Cloudflare and local Ollama/llama.cpp
inference, plus publisher Python weights through an explicit local bridge. Vision
is supported through Cloudflare, Ollama, and the Python bridge; prepared video
frames and processor controls are supported through the bridge. See
[Clef setup and capabilities](docs/clef.md); TypeSafe remains the default provider.

> [!IMPORTANT]
> **This is an independent, community-maintained project.** It is not affiliated with,
> sponsored by, or endorsed by TypeSafe AI. "TypeSafe", "System One", and "Jev" are
> used descriptively to say what this tool talks to. For anything authoritative about
> the model or the API — behaviour, pricing, availability, support — see the
> [official TypeSafe documentation][typesafe-docs]. If TypeSafe ever formally endorses
> this project, this notice will say so; until then, assume it has not.

> [!WARNING]
> **Pre-1.0.** The current version is `0.3.0`. Exit codes and JSON documents may still
> change. Read the `schema` field and pin the version if you script against it. The
> contract in [`docs/cli-contract.md`](docs/cli-contract.md) takes effect at `1.0.0`,
> not today.

---

## Install and try it

Download the archive for your platform from the [v0.3.0 release][release]. Verify its
checksum before running it. For Linux x86-64:

```sh
base=https://github.com/plurp911/jev-cli/releases/download/v0.3.0
archive=jev-cli-x86_64-unknown-linux-gnu.tar.xz
curl -fLO "$base/$archive" -fLO "$base/$archive.sha256"
sha256sum --check "$archive.sha256"
tar -xJf "$archive"
jev=./jev-cli-x86_64-unknown-linux-gnu/jev
"$jev" doctor
```

See [Install jev](docs/install.md) for Linux ARM, macOS, Windows, and source builds.
`doctor` does not contact the API unless you pass `--live`. To ask a question, first
store your TypeSafe API key with `"$jev" auth login`, then run:

```sh
"$jev" choice "Which team should handle this?" \
  -O returns="Exchanges and refunds" \
  -O shipping="Delivery problems" \
  --state "My order arrived damaged. Can I exchange it?"
```

The `--state` text is sent to TypeSafe. The answer includes a selected option and
probabilities for both options. [More examples](#first-question) show JSON output,
scripts, and batch processing.

## What Jev is

Jev is TypeSafe's flagship [System One][jev-docs] model. It does not generate text. You
give it some **state** and one or more typed **questions**, and it returns typed answers
with calibrated probabilities:

| Primitive | Question | Answer |
| --- | --- | --- |
| **Noul** | Does this condition hold? | A probability that the answer is yes |
| **Choice** | Which of these options? | The selected option, a probability for every option, and a confidence |
| **Score** | Where on this described scale? | A position on the scale, a probability for every level, and a confidence |

Because the output is already structured data with its uncertainty attached, it maps
unusually well onto a command-line tool: a script can branch on it without parsing prose.

## What this CLI is

`jev` puts those three primitives, and the ability to ask many of them in one request,
wherever a shell is — a terminal, a `Makefile`, a CI job, an AI coding agent, a data
pipeline.

It is a **thin, faithful** interface. It speaks the API's own vocabulary, it keeps the
whole probability distribution, and it makes what leaves your machine auditable.

## What it is not

- **Not a text generator.** Jev answers bounded questions. If you want prose, you want a
  different model.
- **Not a task toolkit.** There is no `jev review-pr`, no `jev triage`, no
  `jev guard`. Those bake in a prompt and a threshold that were never evaluated on
  *your* data. `jev` gives you the primitives and a request-file format so you can write
  them, review them, and commit them.
- **Not a multi-provider gateway.** It talks to the TypeSafe API. Custom endpoints are
  supported for proxies and local testing, behind a separate credential namespace.
- **Not a certainty machine.** Typed output guarantees the interface, not the truth.
  Validate performance on your own data before you rely on a threshold — that is what
  [`jev eval`](docs/commands.md#jev-eval) is for.

And a few things it will never grow, because they are the reason a tool like this stops
being safe to run:

- **No telemetry, analytics, crash reporting, or update pings.** Not by default, not
  behind a flag.
- **No self-update.** Updates come from the package manager you chose.
- **No plugin system, no dynamic loading, and no shelling out** to a command you
  supplied.
- **No automatic file or `.env` discovery.** `jev` reads the files you name and nothing
  else.

The full list is in [`SECURITY.md`](SECURITY.md).

## Build from source

If your platform has no archive, or you prefer to build the binary yourself:

```sh
git clone https://github.com/plurp911/jev-cli
cd jev-cli
cargo build --release --locked
./target/release/jev doctor
```

`--locked` builds the exact dependency graph in the committed `Cargo.lock` instead of
resolving fresh, so you get the versions that were tested and audited rather than
whatever is newest today.

Requires Rust 1.88 or newer; the pinned toolchain is in
[`rust-toolchain.toml`](rust-toolchain.toml). Like any Rust build, it also needs a C
toolchain for the linker — `cc` and `pkg-config` on Linux, the Xcode command line tools
on macOS, the Visual Studio Build Tools on Windows.

Release archives include checksums and an SPDX SBOM. See [the release verification
guide](docs/release-verification.md) for the integrity and provenance checks.

## Authenticate

Interactively, into your operating system's credential store:

```console
$ jev auth login
TypeSafe API key (input hidden):
stored a credential in macOS Keychain
```

In CI, or anywhere headless:

```sh
export JEV_API_KEY="…"          # or JEV_API_KEY_FILE=/run/secrets/typesafe
```

`jev` also honours `TYPESAFE_API_KEY`, the official SDK convention, so it works in an
environment already set up for the Python or JavaScript SDK.

**There is no plaintext-file fallback.** If no secure store is available, `jev auth
login` fails and tells you to use the environment. It will not quietly write your key to
`~/.config`. See [ADR-0002](docs/adr/0002-security-and-credentials.md).

Check what it can see, without making a network call:

```console
$ jev doctor
```

## First question

A **Noul** — does a condition hold?

```console
$ echo "Help! My payouts have been failing for 3 days." \
    | jev noul "Does this convey urgency?"
answer
  yes  0.9200  ██████████████████████··
a Noul answer is the probability of "yes"; the API reports no separate confidence for it, and 0.5 means "yes and no are similarly likely"
18 tokens in, 4 out, answered by jev-1.13.0
```

A **Choice** — which of these?

```console
$ jev choice "Which team should handle this?" \
    -O returns="Exchanges, refunds, wrong or damaged items" \
    -O shipping="Delivery status, delays, lost packages" \
    -O billing="Charges, invoices, payment problems" \
    --state "My running shoes arrived in the wrong size. Can I swap them?"
answer
  choice      returns
  confidence  1.0000
    billing   0.0000  ························
    returns   1.0000  ████████████████████████
    shipping  0.0000  ························
140 tokens in, 12 out, answered by jev-1.13.0
```

Options are listed alphabetically, not in the order you wrote them: the API returns the
distribution as a JSON object, so there is no wire order to preserve. The selected
option is on the `choice` line, and `--output json` gives the same list to a script.

The last line of each block is the usage footer, on stderr. `--quiet` suppresses it;
`--output json` replaces the whole thing with one document.

A **Score** — where on a described scale?

```console
$ jev score "How severe is the reported issue?" \
    -L "Cosmetic; no impact to functionality" \
    -L "Broken or degraded feature, but workaround exists" \
    -L "Blocking issue; no workaround exists" \
    --state-file bug-report.txt
```

## Ask everything at once

This is the part that matters. System One evaluates every question in a request against
one reading of the state, in parallel, so N separate calls pay for the state N times and
one batched call pays once. TypeSafe measures the difference on a long document in its
[parallel questions cookbook][parallel]; read the figures there. Ask the speculative questions too, and let your code read
only the answers it needs.

`ticket.json` — this is the **official API request body**, not a format `jev` invented:

```json
{
  "state": "Shoes arrived two weeks late and in the wrong size. Also two charges on my card.",
  "questions": {
    "department": {
      "type": "choice",
      "instructions": "Which team should handle this?",
      "criteria": {
        "returns": "Exchanges, refunds, wrong or damaged items",
        "shipping": "Delivery status, delays, lost packages",
        "billing": "Charges, invoices, payment problems"
      }
    },
    "urgent": { "type": "noul", "instructions": "Does this convey urgency?" },
    "frustration": {
      "type": "score",
      "instructions": "How frustrated is the customer?",
      "criteria": ["Calm", "Frustrated", "Very angry"]
    }
  }
}
```

```console
$ jev ask -r ticket.json -o json | jq '{team: .answers.department.choice, conf: .answers.department.confidence}'
{ "team": "returns", "conf": 0.39 }
```

An example copied straight out of <https://docs.typesafe.ai/api> runs unchanged.

## Input

| You have | Use |
| --- | --- |
| text on the command line | `--state "…"` |
| a file | `--state-file path` |
| a pipe | nothing — stdin is the default |
| structured context | `--state-json '{"subject":"…","body":"…"}'` or `--state-json-file path` |

Empty input, invalid UTF-8, binary files, oversized input, malformed JSON, and bad
question definitions are all rejected **before** a request is sent, so a mistake costs
no tokens. Input is never silently truncated.

## In a script

```sh
# Just the number.
score=$(jev noul "Is this a security issue?" --state-file report.md --value)

# Branch on the model's judgment, with the exit code.
if jev noul "Is this a security issue?" --state-file report.md \
     --require 'answer.noul > 0.9' --quiet >/dev/null; then
  open-incident
fi

# Everything, for jq.
jev ask -r triage.json -o json | jq -r '.answers.department.choice'

# Many records at once.
jev map -r classify.json -i tickets.jsonl --id-field ticket --state-field body \
    --output-file out.jsonl -j 8
```

Exit codes are a contract:

| Code | Meaning |
| --- | --- |
| `0` | The API answered. **Not** "the answer was yes." |
| `1` | A `--require` gate was evaluated and did not hold. Also `jev eval` when no threshold reaches `--target`. |
| `2` | Fix your command or your request. |
| `3` | Fix your credentials. |
| `4` | The API was unreachable, slow, or overloaded. Retry. |
| `5` | A batch finished with some rows failing. |
| `6` | A `--require` gate **could not be evaluated**. Never a pass. |
| `70` | An internal error. This is a bug; please report it. |
| `74` | Could not write output — a full disk, a revoked permission. Not a bug. |
| `130` | Interrupted. Whatever was written before the interrupt is complete and readable. |

`1` and `6` are deliberately different. "The model said no" and "your gate is broken"
call for different responses, and a gate that cannot be evaluated must never look like a
passing one.

Full detail: [`docs/cli-contract.md`](docs/cli-contract.md).

## Where your data goes

Plainly, because it matters:

- **The `state` you supply is transmitted to the configured API endpoint** — by default
  `https://api.typesafe.ai` — whenever a request runs. So are your instructions and your
  option and level descriptions.
- Run `jev noul … --dry-run` to see the exact bytes that would be sent, without sending
  them or reading your credential.
- `jev` reads **no file you did not name**. It does not walk directories, expand globs
  into input, or load `.env` files.
- `jev` writes **no file** unless you asked: `jev config set`,
  `jev map --output-file`, `jev map --review-file`, and `jev eval --report` are the only
  writers, and every one of them is a path you typed.
- `jev` caches **nothing**. `jev map --resume` reads the output file you specified; it
  never replays a stored judgment.
- There is **no telemetry**, no analytics, and no update check. The only network request
  is the API call you asked for, and `jev doctor` makes none at all without `--live`.

For what TypeSafe does with what it receives, see their
[legal documentation](https://docs.typesafe.ai/legal). That is their commitment, not
ours to make.

## Security

- Your key never appears in `argv` — there is no `--api-key` flag, and a test walks
  every argument of every subcommand to keep it that way.
- Your key never appears in output, in an error, in a panic payload, or under
  `--verbose`. Local tests and `scripts/credential-canary.sh` use fake keys and fail if
  one appears in tested output or files.
- Your key is held in a type that redacts on `Debug` and `Display` and zeroizes on drop.
- A TypeSafe credential is **structurally unreachable** from a non-official endpoint:
  those use a separate environment namespace and never touch the credential store.
- Text that comes back from the API is sanitized before it reaches your terminal — ANSI
  escapes, bidirectional overrides, and zero-width characters included.
- Responses are bounded in size and nesting depth, and are rejected rather than
  lossily decoded.

Details in [`docs/threat-model.md`](docs/threat-model.md); reporting in
[`SECURITY.md`](SECURITY.md).

## Stop guessing your thresholds

Every number in a `--require` expression in this README is a placeholder, and saying so
is not enough on its own — you still have to write *something*. `jev eval` is how you
replace a guess with a measurement. Give it examples you have already judged:

```json
{"schema":"jev.eval.row/v1","id":"1841","state":"Crash on startup after 2.4…","labels":{"urgent":true}}
```

```console
$ jev eval -r triage.json -d labelled.jsonl --objective min-precision --target 0.95
evaluated 72 labelled row(s) against jev-1.13.0
threshold chosen on 168 calibration row(s), reported on 72 held-out row(s)

urgent (noul, n=72)
  accuracy               0.861 (95% 0.760-0.925)
  precision              0.952
  recall                 0.714
  brier score            0.098
  threshold 0.780 on urgent.noul, under --objective min-precision
    gate with: --require 'urgent.noul >= 0.780'
```

It measures **one question, one dataset, and one model version** — the report records
all three — and it holds rows back, so the number beside the threshold is not the number
the threshold was picked to maximize. Nothing is trained, and your labels are never
sent: ground truth is compared locally after the answer comes back.

It will not pick an objective for you. That part encodes what being wrong costs you, and
it is yours. See [`jev eval`](docs/commands.md#jev-eval).

## Pin your model

`jev-latest` is a **moving alias**. When TypeSafe ships a new release, the answers behind
it change with no change on your side. If you have calibrated a threshold against a
particular version — with `jev eval` or otherwise — pin it:

```sh
jev config set model jev-1.13.0
```

`jev` always reports the concrete model that answered (`model`) separately from what you
asked for (`model_requested`), so a result is interpretable later.

## AI agents and MCP

`jev mcp serve` is a local [Model Context Protocol](https://modelcontextprotocol.io)
server. An agent host starts it and gets five typed tools, `noul`, `choice`, `score`,
`ask`, and `map`, which run the same code as the commands of the same names:

```sh
claude mcp add --transport stdio --scope user jev -- jev mcp serve   # Claude Code
codex mcp add jev -- jev mcp serve                        # Codex
grok mcp add jev -- jev mcp serve                         # Grok CLI
```

It uses stdio only, with no port and no daemon, and it reuses the credential from
`jev auth login`, so no key goes into any MCP configuration. The state you pass is sent
to TypeSafe, as it is from the command line. Cursor setup, the tool schemas, limits, and
when to prefer the CLI are in [`docs/mcp.md`](docs/mcp.md).

## Shell completions

```console
$ jev completions bash > /etc/bash_completion.d/jev
$ jev completions zsh  > "${fpath[1]}/_jev"
$ jev completions fish > ~/.config/fish/completions/jev.fish
```

## Documentation

| Document | What is in it |
| --- | --- |
| [`docs/commands.md`](docs/commands.md) | Every command and flag. |
| [`docs/mcp.md`](docs/mcp.md) | `jev mcp serve`: host setup, tools, limits, troubleshooting. |
| [`docs/output-schema.md`](docs/output-schema.md) | The JSON contract, document by document. |
| [`docs/cli-contract.md`](docs/cli-contract.md) | What is stable, exit codes, environment, configuration. |
| [`docs/troubleshooting.md`](docs/troubleshooting.md) | When something is not working. |
| [`docs/api-compatibility.md`](docs/api-compatibility.md) | What `jev` assumes about the API, and how that is checked. |
| [`docs/agent-skill.md`](docs/agent-skill.md) | The Agent Skills this repository ships, and how to install them. |
| [`docs/release-verification.md`](docs/release-verification.md) | Verifying a release artifact came from this repository. |
| [`docs/benchmarks.md`](docs/benchmarks.md) | What was measured, and what was not. |
| [`docs/adr/`](docs/adr/) | Why the load-bearing decisions are what they are. |
| [`docs/research/comparison.md`](docs/research/comparison.md) | How this compares with the other community CLIs. |
| [`docs/repo-settings.md`](docs/repo-settings.md) | The repository settings this project expects. |
| [`AGENTS.md`](AGENTS.md) | The engineering guide. Architecture, invariants, policy. |
| [`docs/architecture.md`](docs/architecture.md) | How the crates fit together and why. |
| [`docs/threat-model.md`](docs/threat-model.md) | What we are defending against, and how. |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | How to work on this. |
| [`SECURITY.md`](SECURITY.md) | How to report a vulnerability. |

For Jev itself — question design, confidence, state, patterns — the
[official documentation][typesafe-docs] is the authority, and this project defers to it.

## A note on how this project is built

This repository is written almost entirely by AI coding agents, under human direction
and review. That is stated openly because it changes what you should expect and how the
project is engineered.

The maintainer runs the verification suite locally before pushing. Install the tracked
pre-push hook with `scripts/install-hooks.sh`, then run `scripts/verify.sh --push` to
check the same gate yourself. The repository does not run automatic GitHub Actions;
the trade-offs, including the loss of independent platform and security jobs, are in
[ADR-0013](docs/adr/0013-local-verification.md).

It also means the instructions given to agents are treated as source code: see
[`AGENTS.md`](AGENTS.md), which forbids — among other things — weakening a test to make
a change pass, and requires that any claim about TypeSafe API behaviour be backed by a
citation to the official documentation rather than by recollection.

Review the code before you trust it. That is good advice for any dependency; it is
pointed advice here.

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE))
- MIT license ([`LICENSE-MIT`](LICENSE-MIT))

at your option. See [ADR-0006](docs/adr/0006-licensing.md) for why.

Unless you state otherwise, any contribution you intentionally submit for inclusion in
this work, as defined in the Apache-2.0 license, shall be dual-licensed as above,
without any additional terms or conditions.

[typesafe]: https://typesafe.ai
[release]: https://github.com/plurp911/jev-cli/releases/tag/v0.3.0
[typesafe-docs]: https://docs.typesafe.ai
[jev-docs]: https://docs.typesafe.ai/concepts/system-one
[parallel]: https://docs.typesafe.ai/cookbooks/parallel_questions
