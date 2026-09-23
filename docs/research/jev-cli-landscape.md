# The Jev CLI landscape

A factual survey of the Jev / TypeSafe System One command-line tools that exist as of
**2026-09-19**, and what `jev` should conclude from them.

Every claim here was read from the corpus recorded in
`docs/research/reference-manifest.md`, at the commits pinned there. Nothing was executed;
nothing was copied. **Community projects are evidence about the landscape, never evidence
about the API.** Where a project's behaviour disagrees with the official TypeSafe
documentation, this document says so and the official documentation wins (`AGENTS.md` §6).

The whole field is days old. Every project in the table below has a single squashed or
initial commit in its cloned history and was last touched within a week of the snapshot.
Treat maturity judgments accordingly: nobody here has a track record yet, which is
precisely the gap `jev` can fill.

## The nine implementations

| Short name | Slug | Language | Distribution |
| --- | --- | --- | --- |
| **MC-jev** | `model-clis/jev` | Rust | Native binary; Homebrew, Scoop, `curl`/`irm` installers |
| **tumf** | `tumf/jev-cli` | Python | PyPI via `uv tool install` |
| **semdecide** | `sharziki/semdecide` | Python | GitHub release wheel (`pipx`/`uv`); not on PyPI |
| **jev-axi** | `shiftynick/jev-axi` | TypeScript | npm (`jev-axi`) |
| **jtsang** | `jtsang4/jev-cli` | TypeScript | npm (`@jtsang/jev-cli`) |
| **geilt** | `geilt/typesafe-cli` | Python | `git clone` + `chmod +x`; no package |
| **y0usaf** | `y0usaf/typesafe-cli` | TypeScript | npm, plus a Nix flake |
| **jevctl** | `Nasrallah-AL/jev-cli` | TypeScript | npm (`jevctl`) |
| **jev-go** | `Stumble/jev-go` | Go | `go install`; primarily an SDK |

---

## Feature matrix

`✅` present · `⚠️` present with a caveat named below the table · `❌` absent ·
`—` not observed in the snapshot.

### Runtime, installation, and API surface

| | MC-jev | tumf | semdecide | jev-axi | jtsang | geilt | y0usaf | jevctl | jev-go |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Language / runtime | Rust | Python 3.13+ | Python 3.10+ | Node | Node 22+ / Bun | Python 3.9+ | Node 22.18+ | Node 20.12+ | Go |
| Single native binary | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ⚠️ |
| Runtime dependency at use time | none | `uv` + Python | Python | Node | Node/Bun | Python | Node | Node | none |
| Direct TypeSafe API | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Alternate providers | ❌ | ✅ Vercel, OpenRouter, custom | ❌ | ❌ | ✅ Vercel | ❌ | ❌ | ✅ OpenRouter, Cloudflare | ✅ Vercel |
| Custom endpoint / base URL | ✅ `JEV_API_BASE` | ✅ `--endpoint` | — | — | ✅ `baseURL` | ✅ `TYPESAFE_BASE_URL` | ✅ config `endpoint` | ✅ | ✅ `-base-url` |
| Choice | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Score | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Noul | ✅ | ✅ | ✅ | ✅ | ⚠️ as `boolean` | ✅ | ✅ | ✅ | ✅ |
| Mixed multi-question request | ✅ `ask` | ✅ `run` | ❌ | ✅ `ask` | ✅ `eval` | ✅ `ask` | ✅ `ask` | ✅ `ask` | ✅ |
| Model selection / pinning | ⚠️ pinned to `jev-latest` | ✅ `--model` | — | ✅ `--model` | ✅ `--model` | ✅ env | ✅ config | ✅ `-m` | ✅ `-model` |
| `models` listing | ❌ | ❌ | ❌ | ✅ | ❌ | ✅ | ❌ | ✅ | ❌ |

Caveats:

- **jtsang / Noul.** It exposes `{"type":"boolean"}` and returns `{"probability": …}`.
  The official wire vocabulary is `{"type":"noul"}` returning `{"noul": …}`
  (`references/05-typesafe-docs/pages/api.md`). It also relegates `confidence` to
  `providerMetadata` behind `--full`. This is a gateway abstraction presented as the API.
- **y0usaf / request format.** Its `ask` file uses a `questions` **array** with `id`,
  `options`, and `levels`. The wire format is a `questions` **map** with `criteria`. Its
  file format is an invention, not the protocol.
- **jev-go / binary.** Go produces a single binary, but the shipped `jev` command is an
  interactive wizard rather than a scriptable tool.
- **MC-jev / model.** Pinning to `jev-latest` and exposing no override means an
  alias move silently changes answers, and users who tuned thresholds against
  `jev-1.13.0` cannot pin.

### Input, output, and the machine contract

| | MC-jev | tumf | semdecide | jev-axi | jtsang | geilt | y0usaf | jevctl | jev-go |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| stdin | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ⚠️ prompts |
| File input | ✅ `@PATH` | ✅ `@PATH` | ✅ `--file` | ✅ `--state` | ✅ `--state-file` | ✅ | ✅ `--state-file` | ✅ `@path` | ❌ |
| JSON state | ✅ | ✅ `--json-state` | ⚠️ JSONL records | ✅ `--state-json` | ✅ `--state-json` | ✅ | ✅ | ✅ | ✅ |
| JSON output | ✅ always | ✅ default | ✅ `--json` | ✅ `--json` | ✅ default | ✅ `--json` | ✅ `--json` | ✅ `--json` | ✅ |
| Human-readable output | ⚠️ stderr only | ⚠️ `--value` | ✅ | ✅ | ❌ | ⚠️ | ✅ best in class | ✅ tables | ✅ |
| JSONL | ✅ `map` | ❌ | ✅ `filter` | ✅ | ❌ | ❌ | ❌ | ✅ `--format jsonl` | ❌ |
| Other formats | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ csv, tsv, md | ❌ |
| Single-value extraction | ❌ | ✅ `--value` | ⚠️ `--quiet` + code | — | ❌ | ❌ | ❌ | ✅ `--pluck` | ❌ |
| Declared stable output schema | ⚠️ prose promise | ❌ | ✅ `schema_version` field | ❌ | ❌ | ❌ | ❌ | ⚠️ prose promise | ❌ |
| stdout/stderr separation stated | ✅ | ✅ | ✅ | ✅ | — | — | ✅ | ✅ | ✅ |
| Exit-code contract documented | ✅ 5 codes | ✅ 5 codes | ✅ 5 codes + guard's 3 | ⚠️ per-command (3 = block) | ❌ | ❌ | ❌ | ✅ 3 codes | ❌ |
| Semantic gating / assertions | ✅ `--assert` expression language | ❌ | ✅ thresholds | ✅ bands, `guard` exit 3 | ❌ | ❌ | ❌ | ✅ `--fail-on` | ❌ |
| Distinguishes uncertain from false | ❌ | ❌ | ✅ exit 3 | ✅ `escalate` band | ❌ | ❌ | ❌ | ⚠️ `--fail-on` only | ❌ |

`semdecide` is the only project whose JSON documents carry an explicit
`schema_version`, and the only one that gives *uncertain* its own exit code
(`0` true, `1` false, `2` usage, `3` uncertain, `4` provider failure).

### Execution: batching, concurrency, reliability

| | MC-jev | tumf | semdecide | jev-axi | jtsang | geilt | y0usaf | jevctl | jev-go |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Batch / map over many inputs | ✅ `map` | ❌ | ✅ `filter` | ✅ `filter`/`rank` | ❌ | ❌ | ❌ | ✅ `batch` | ❌ |
| Concurrency control | ✅ `--concurrency` (4) | ❌ | ❌ | ✅ env | ❌ | ❌ | ❌ | ✅ pool | ❌ |
| Resumability | ✅ `--out --resume` | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Timeouts | ✅ 30 s fixed | ✅ | ✅ `--timeout` (10 s) | ✅ | ✅ `--timeout` | ❌ | ✅ | ✅ `--timeout` | ✅ `-timeout` |
| Retries with backoff | ✅ 5, honours `retry_after_ms` | ✅ | ✅ `--retries` (2, max 5) | ✅ | ✅ | ⚠️ | ✅ | ✅ | ✅ |
| Client-side rate limiting | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Cost / token usage reporting | ⚠️ stderr summary | ✅ in JSON | ✅ `usage` in JSON | ✅ best in class | ✅ `--full` | ❌ | ✅ `--verbose` | ✅ | ✅ `-show-metadata` |
| Dry run | ❌ | ❌ | ❌ | ⚠️ `--estimate`-like | ❌ | ❌ | ❌ | ✅ `--dry-run` | ❌ |
| Input size limits enforced | ❌ | ❌ | ✅ `--max-input-bytes` (1 MB) | ⚠️ | ❌ | ❌ | ❌ | ⚠️ | ❌ |
| Rejects binary / invalid UTF-8 | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Response caching | ❌ explicitly not | ❌ | ❌ | ⚠️ 24 h on disk | ❌ | ❌ | ❌ | ❌ | ❌ |

Nobody implements client-side rate limiting, although the published limits are
250,000 tokens/second and 1,200 requests/minute and are documented as changing without
notice (`references/05-typesafe-docs/pages/models.md`).

### Credentials, privacy, and trust

| | MC-jev | tumf | semdecide | jev-axi | jtsang | geilt | y0usaf | jevctl | jev-go |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Environment variable | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| OS keychain | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ |
| Plaintext file store | ⚠️ primary | ⚠️ fallback | ⚠️ fallback | ⚠️ fallback | ⚠️ primary | ⚠️ opt-in | ⚠️ fallback | ⚠️ fallback | ❌ |
| Automatic `.env` discovery | ❌ | ❌ | ❌ | ⚠️ walks to repo root | ❌ | ❌ | ⚠️ fixed paths | ❌ | ❌ |
| Key refused as a CLI argument | ✅ | ✅ | ✅ stated | ❌ `config set apiKey` | ❌ `config set` | ⚠️ `--creds` path | ❌ `--key` | ✅ | ✅ stated |
| Hidden interactive prompt | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ |
| Restrictive file permissions | ✅ 0600 | ✅ 0700 dir + atomic 0600 | ⚠️ documented, not enforced | ✅ 0600 | ✅ 0600 | — | — | ✅ 0600 | n/a |
| Explicit privacy disclosure | ⚠️ | ✅ | ✅ | ⚠️ | ❌ | ❌ | ❌ | ✅ | ✅ |
| Writes non-credential data to disk | ⚠️ diagnostics (opt-in) | ❌ | ❌ | ⚠️ cache + `stats/*.jsonl` | ❌ | ❌ | ❌ | ⚠️ config | ❌ |
| Telemetry / analytics | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Self-update mechanism | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ⚠️ `jev update` | ❌ |

Two findings stand out.

1. **No project avoids plaintext credential storage.** `jevctl` is the only one that
   reaches for an OS keychain, and it falls back to `~/.config/jev/credentials.json` when
   none is available. MC-jev and jtsang store the key in plaintext as their *primary*
   mechanism. `jev-go` is the only one with no credential store at all, which sidesteps
   the problem rather than solving it.
2. **Nobody ships telemetry.** That is a floor, not a differentiator — but it means a
   telemetry-free promise must be backed by something stronger than a claim.

### Engineering, distribution, and evidence

| | MC-jev | tumf | semdecide | jev-axi | jtsang | geilt | y0usaf | jevctl | jev-go |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Runtime dependency count | ⚠️ 12 direct / ~200 transitive | moderate | ✅ zero | moderate | moderate | ✅ stdlib only | ✅ minimal | large | ✅ stdlib only |
| Test suite | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | ❌ | ✅ | ✅ |
| Property or fuzz tests | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Network-free test strategy | ✅ `wiremock` | ✅ | ✅ | ✅ | ✅ | ❌ | ❌ | ✅ | ✅ |
| CI workflows | ✅ 2 | ✅ 2 | ✅ 1 | ✅ 4 | ✅ 2 | ❌ | ❌ | ✅ 2 | ✅ 1 |
| `SECURITY.md` | ❌ | ✅ | ✅ | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ |
| Checksums on release artifacts | ✅ `.sha256` | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Build-provenance attestations | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Homebrew | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Windows | ✅ Scoop + binary | ⚠️ via Python | ⚠️ via Python | ⚠️ via Node | ⚠️ via Node | ❌ | ⚠️ via Node | ⚠️ via Node | ⚠️ via Go |
| Linux / macOS | ✅ / ✅ | ✅ / ✅ | ✅ / ✅ | ✅ / ✅ | ✅ / ✅ | ✅ / ✅ | ✅ / ✅ | ✅ / ✅ | ✅ / ✅ |
| Agent skill shipped | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ | ❌ | ⚠️ Claude Code plugin | ✅ |
| MCP support | ❌ | ✅ separate `jev-mcp` binary | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Benchmark / evaluation evidence | ❌ | ❌ | ❌ | ✅ with a null result | ❌ | ❌ | ❌ | ❌ | ❌ |
| Project maturity | days | days | days | days | days | days | days | days | days |

**Not one project in the corpus publishes build-provenance attestations, and only
MC-jev publishes checksums.** Not one uses property-based or fuzz testing.

---

## Best ideas worth adopting

| Idea | Demonstrated by | Why it matters |
| --- | --- | --- |
| **Uncertain is not false.** Separate exit codes for true, false, uncertain, usage error, and infrastructure failure. | semdecide | A gate that silently collapses "the model is unsure" into "no" is a broken gate. This is the single best idea in the landscape. |
| **A version field in every JSON document.** | semdecide (`schema_version`) | Turns a prose compatibility promise into something a consumer can branch on. Our `--output json` already reserves a `schema` field; this confirms the shape. |
| **An unevaluable assertion must fail loudly.** MC-jev exits `1`, not `0`, when an assertion references an unknown path. | MC-jev | A gate that cannot be evaluated must never pass. This belongs in the exit-code contract. |
| **`--dry-run` that prints exactly what would be sent.** | jevctl | The cleanest answer to "what leaves my machine". Directly serves `AGENTS.md` §5, and it is auditable by the user rather than promised by us. |
| **Refuse a credential as a command-line argument, and prompt hidden instead.** | tumf, jevctl, jev-go | Already our rule (`AGENTS.md` §4); three independent projects converged on it. tumf's atomic `0600` write inside a `0700` directory is the reference implementation of the mechanics. |
| **`auth status` and `auth test` as separate commands.** | tumf | "A key is present" and "the key works" are different questions, and conflating them produces bad diagnostics. |
| **Offline validation before anything costs tokens.** | MC-jev (`presets validate`), jtsang (`doctor --offline`), jgrep (`--estimate` with no auth and no API call) | Catches malformed questions and bad assertions for free. `--estimate` is the strongest form: it reads to EOF and reports call count and approximate cost without authenticating. |
| **Cost, latency, token counts, and the concrete model version on every call.** | jev-axi, y0usaf | `jev-latest` is an alias; recording which version actually answered is the only way to reason about a threshold later. The API returns the resolved ID in `model`. |
| **Deterministic local checks before spending a request.** | jev-axi | Routine cases decided locally cost nothing and cannot be wrong for model reasons. |
| **Checkpoint the output file, do not cache the results.** | MC-jev (`--out --resume`) | Resumability without a response cache: no hidden state, no stale answers, no privacy surprise. The distinction is stated explicitly in its README and is the right one. |
| **One question per item — never a shared numbered list.** | every | Reports positional degradation past ~16 items while per-question embedding stays flat past 128. Shapes how any batch mode should build requests. |
| **A blunt "what leaves your machine" section.** | every, semdecide, jevctl | Names the endpoint, names what is sent, names what is not. |
| **Publish the negative result.** | jev-axi (`docs/skills-do-not-get-used.md`, `bench/agent/`) | It reports that file ranking moved agent cost by less than run-to-run noise, and that six runs per condition cannot call a direction. The only intellectually honest evaluation in the corpus. |
| **grep semantics done properly.** | jgrep | Streaming, input-order output, `-q`/`-c`/`-v`/`-m`, truncation warnings. The right model for stream discipline even though we will not build a grep. |
| **A stable, aligned human rendering of the probability distribution.** | y0usaf | The clearest human output in the corpus, and it explains *why* a confidence of 0.45 is worth branching on. |
| **Third-party attribution in a `NOTICE` file.** | jevctl | The correct mechanics for the rare case where §8 permits adaptation. |
| **Typed verdicts over raw numbers, with exhaustive matching.** | s1-rs (`Verdict::{Act,Review,Escalate}`) | An SDK idea, but it argues for our own confidence-band vocabulary being a first-class, documented thing rather than a per-command flag. |
| **Sync clients without an async runtime.** | Twister915 (optional `ureq`), jev-go (stdlib only) | Independent confirmation that ADR-0007's blocking transport is a viable design, not a limitation. |
| **Explicit parity tracking against a named SDK revision.** | typesafe-ai-rs (`docs/PARITY.md`) | The model for our compatibility fixtures: record the revision, record the deliberate deviations. |

## Things to improve — gaps common across the field

1. **Credential storage is the weakest point in the entire landscape.** Eight of nine
   projects will put an API key in a plaintext file, and for two that is the primary path.
   No project fails closed when secure storage is unavailable.
2. **Nobody proves what they shipped.** One project publishes checksums; none publishes
   build-provenance attestations. A user installing any of these takes the binary on faith.
3. **Hostile input is barely considered.** Only semdecide rejects binary, invalid-UTF-8,
   oversized, and malformed input. No project sanitises API-returned text before it
   reaches a terminal, although Jev returns caller-supplied option names and rubric
   legends that can carry ANSI escapes or bidirectional overrides.
4. **The machine contract is mostly a promise, not a mechanism.** One project versions
   its JSON. Several state that field names are stable; none gives a consumer a way to
   detect a change.
5. **Uncertainty is usually flattened.** Most tools threshold internally and return a
   verdict. Jev's whole value is the calibrated distribution; a CLI that discards it is
   throwing away the reason to use Jev at all.
6. **No property or fuzz testing anywhere.** For tools whose job is parsing untrusted
   JSON from a network, that is a conspicuous absence.
7. **No client-side rate limiting**, despite published limits that the documentation says
   change without notice.
8. **Model pinning is an afterthought.** The strongest competitor hard-pins `jev-latest`
   and offers no override, so an alias move silently changes behaviour for every user who
   calibrated a threshold.
9. **Evaluation evidence is nearly absent.** One benchmark exists in the whole corpus, and
   its headline finding is negative.
10. **Confidence is often misdescribed.** The official documentation is specific:
    Choice/Score confidence summarises distribution concentration, Noul carries no separate
    confidence and 0.5 means "similar probability for yes and no", not "medium intensity".
    Several projects paper over that asymmetry by synthesising a Noul confidence.

## Things NOT to copy

| Anti-pattern | Where it appears | Why we reject it |
| --- | --- | --- |
| **Plaintext credential file as the primary or fallback store** | MC-jev (primary), jtsang (primary), tumf, semdecide, jev-axi, y0usaf, jevctl (fallback) | ADR-0002: OS-native storage, then `JEV_API_KEY`, then `JEV_API_KEY_FILE`, then a clear failure. No silent write to `~/.config`. |
| **Walking parent directories for a `.env` file** | jev-axi (`src/config.ts`) | `AGENTS.md` §3.3. Reading a file the user did not name is a supply-chain hazard; in a shared checkout it is a credential-confusion bug. |
| **Reading credentials from fixed unrelated paths** | y0usaf (`~/.pi/agent/pi-jev.json`, `~/Tokens/TYPESAFE_API_KEY.txt`) | Same rule, worse: the user cannot see the resolution order without reading the source. |
| **Accepting a key as a flag or `config set` argument** | y0usaf (`--key`), jtsang, jev-axi (`config set apiKey`) | Arguments appear in `ps`, shell history, and CI logs. |
| **A self-update command** | jevctl (`jev update`) | `AGENTS.md` §3.3. Updates come from the package manager the user chose; a binary that rewrites itself is a supply-chain surface. |
| **On-disk response caching by default** | jev-axi (24 h) | `AGENTS.md` §5: no caching of request or response content without an explicit flag and an ADR. Also makes reproducibility ambiguous. MC-jev's explicit refusal to memoise is the right call. |
| **Writing usage statistics to disk** | jev-axi (`stats/*.jsonl`) | A write the user did not ask for. Even local-only, it is a behaviour that must be requested. |
| **Inventing a question vocabulary that is not the wire format** | jtsang (`"boolean"` / `probability`), y0usaf (`questions` array with `options`/`levels`) | Users transfer knowledge between the docs, the SDKs, and the CLI. A CLI that renames primitives forces them to learn a private dialect and makes official documentation misleading. |
| **Hiding `confidence` behind a verbosity flag** | jtsang (`--full` → `providerMetadata`) | Confidence is part of the answer for Choice and Score, not metadata. |
| **Hard-pinning the model with no override** | MC-jev | Prevents exactly the version pinning the official docs recommend for anyone who tuned thresholds. |
| **Task-shaped commands with baked-in prompts and thresholds** | jevctl (`verify`, `screen`, `compact`, `route`), jev-axi (`triage`, `diff`, `files`, `progress`), semdecide (`guard`) | Each embeds an opinion about question wording and a threshold that was not evaluated on the user's data. They are recipes; a canonical CLI should make recipes *expressible*, not ship them as the interface. |
| **Domain specialisation in a general tool** | every (tree-sitter function splitting), jgrep (record chunking, language-aware parsing) | Excellent in their own tools. In a general CLI they add dependencies, a parsing attack surface, and an implicit promise to keep up with languages. |
| **Shipping without a licence** | geilt | Nothing may be copied from it, and nobody can safely depend on it. |
| **`curl \| sh` as the headline install path** | MC-jev | Its installer does verify SHA-256 — but the documented flow pipes a fetched script straight into a shell. Offer verifiable artifacts and let a package manager or an explicit download be the primary path. |
| **A gateway matrix presented as "Jev"** | tumf, jtsang, jevctl, jev-go | Every provider hop is a place the user's API key and their data can go. If we ever support one it must be explicit per invocation, visible in output, and never inherited from a file the user did not write (`AGENTS.md` §4). |
| **An interactive wizard as the primary CLI** | jev-go | The opposite of a scriptable Unix tool. |
| **Unsourced API claims** | jev-axi's `EXPLORATION.md` states a "~32k tokens shared by state and questions" budget and "~100 ms" latency | The official models page gives 64k per request with a 32k budget for `state` plus the longest question. Close, but not the same, and the difference matters at the boundary. Community notes are not citable (`AGENTS.md` §6). |

## Proposed differentiators

Ranked by how much trust each buys per unit of work. None of these requires being bigger
than the competition; all of them are things no existing project does.

1. **Be the only Jev CLI with no plaintext credential path, ever.** OS-native storage,
   `JEV_API_KEY`, `JEV_API_KEY_FILE`, then an actionable failure. Eight of nine
   competitors — and `gh` itself — fall back to a file. Failing closed is a headline
   feature here, and ADR-0002 already commits to it. Back it with the existing canary
   tests that assert a known secret never appears in any output stream.
2. **Be the only one whose artifacts can be verified.** Checksums *and* GitHub build
   provenance attestations, with the `gh attestation verify` command in the README. One
   competitor publishes checksums; none publishes attestations. ADR-0005 already points
   here; shipping it makes `jev` the only installable-with-evidence option.
3. **Make the output contract a mechanism, not a promise.** A `schema` field in every
   JSON document, a documented additive-only policy, and compatibility fixtures recorded
   from official responses that fail CI when decoding changes. Only semdecide versions its
   JSON, and nobody tests against recorded official responses.
4. **Own uncertainty as a first-class concept.** Distinct exit codes for true, false,
   *uncertain*, usage error, and infrastructure failure; an unevaluable gate that exits
   nonzero; thresholds that are explicit and never defaulted silently; and the full
   probability distribution always present in JSON. semdecide gets the exit codes right
   and MC-jev gets unevaluable-assertion handling right; nobody does both, and neither
   preserves the distribution as an invariant.
5. **Treat every response byte as hostile, and prove it.** Bounded response size, bounded
   nesting depth, explicit invalid-UTF-8 handling, and sanitisation of API-returned text
   before it reaches a terminal — with property tests. This is the whole category's
   blind spot, and it is cheap for us because `jev-core` is pure and already
   proptest-equipped.
6. **Be trivially auditable about what leaves the machine.** `--dry-run` printing the
   exact request body, a printed warning whenever a non-default endpoint is in use, no
   automatic file discovery, no cache, no disk writes the user did not request, no
   telemetry. jevctl has the dry run; nobody has the whole set.
7. **Be the fastest and the smallest to install.** A single static binary with sub-10 ms
   startup, no runtime, and a dependency tree small enough to read. MC-jev is the only
   other native binary, at roughly 200 transitive packages and a full Tokio runtime.
   ADR-0007's blocking transport is what makes the smaller tree possible, and jev-go and
   semdecide prove a dependency-light client is enough.
8. **Ship primitives, not recipes.** Expose Choice, Score, Noul, and mixed multi-question
   requests faithfully, in the official vocabulary, with presets or templates as *data*
   the user writes and reviews. Every competitor that grew task commands baked in a
   threshold nobody evaluated. Being the tool the others can be built on top of is a
   stronger position than being one more opinionated wrapper.
9. **Pin honestly.** Accept a model identifier, default to the documented alias, and
   always report the resolved version the API returned. The strongest competitor cannot
   do this at all.
10. **Publish evaluation evidence, including when it is negative.** jev-axi is the only
    project that measured its own value and reported that it could not detect one. That is
    the standard to match, and matching it is a credibility asset no wrapper can fake.

## Strongest competitor

**`model-clis/jev`.** It is the only other native-binary implementation, and it is ahead
of everything else on distribution (Homebrew, Scoop, checksum-verified installers,
automated releases for three platforms), on gating (a real assertion expression language
with correct unevaluable-assertion semantics), on batch execution (concurrent `map` with
JSONL output and output-file resumption — the only resumability in the field), and on
reviewable automation (committed presets with offline validation).

It is beatable on exactly the axes `AGENTS.md` already commits to: it writes the API key
to a plaintext file with no OS keychain and no failure mode, it hard-pins `jev-latest`
with no override, it publishes no provenance attestations, it versions no JSON schema, it
carries a full async runtime and roughly 200 transitive packages, and it has no
`SECURITY.md`. Those are not gaps in polish; they are the trust properties this project
exists to provide.

The right posture is to treat MC-jev's feature set as the functional floor — presets,
assertions, batch with resume, real installers — and to win on the things it did not do.

---

*Sources: `docs/research/reference-manifest.md` and the corpus in `references/`, all at
the commits pinned there. Official API behaviour cited from the documentation snapshot in
`references/05-typesafe-docs/pages/`; re-fetch the live page before relying on any of it
for a code change (`AGENTS.md` §6).*
