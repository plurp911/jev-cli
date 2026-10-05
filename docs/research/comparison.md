# How `jev` compares

An honest assessment of this CLI against the other Jev / System One command-line tools
surveyed in [`jev-cli-landscape.md`](jev-cli-landscape.md), written after implementing
it rather than before.

Recorded **2026-09-19**, against the commits pinned in
[`reference-manifest.md`](reference-manifest.md). The whole field is days old; nobody
here has a track record, including us.

This is a dated comparison. Its CI and release-workflow descriptions record the state
at that date, before Clef support. The current provider/media capabilities are in
[Clef](../clef.md). Current verification and release rules are in [ADR-0013](../adr/0013-local-verification.md)
and [ADR-0014](../adr/0014-manual-release-gate.md).

There is no winner declared below. Several of these tools do things this one does not,
and some of those things are genuinely better.

---

## Where `jev` is ahead

### 1. Credential handling — the clearest gap in the field

Eight of the nine surveyed projects will write an API key to a plaintext file. For two
(`model-clis/jev`, `jtsang4/jev-cli`) that is the *primary* mechanism. One (`jevctl`)
reaches for an OS keychain and falls back to `~/.config/jev/credentials.json`. One
(`jev-go`) has no store at all, which sidesteps the problem rather than solving it.

`jev` has no plaintext path, at all, by construction:

- OS credential store, `JEV_API_KEY`, `JEV_API_KEY_FILE`, or `TYPESAFE_API_KEY` — then
  an actionable failure. `jev auth login` refuses rather than degrading.
- The configuration file **cannot** become a credential store: a setting whose name
  looks like a credential, at any nesting level, is a load error.
- No key in `argv`, enforced by a test that walks every argument of every subcommand.
- The key lives in a type that redacts on `Debug` and `Display`, zeroizes on drop, and
  is neither `Clone` nor `Serialize`. It is passed to the transport as a separate
  argument, so a retry loop has nothing to duplicate.

### 2. A TypeSafe key is structurally unreachable from a non-official endpoint

Every project that supports a custom endpoint sends whatever key it has to whatever host
it was given. Some warn. A warning is a mitigation, not a control.

`jev` uses a separate credential namespace (`JEV_CUSTOM_API_KEY`) for non-official
endpoints and does not consult the TypeSafe sources or the OS store for them at all.
There is no code path by which a production key reaches another host — plus HTTPS
enforcement with a loopback exception that rejects `localhost.evil.example`, no redirect
following, and a warning on every invocation that `--quiet` does not suppress.

> **Correction, 2026-09-20.** That claim was false when it was written, and is true now.
> `ureq`'s default configuration reads `HTTP_PROXY`, `HTTPS_PROXY`, and `ALL_PROXY`, and
> `jev` did not override it — so a variable this project neither documented nor reported
> decided where every request went, and for the sanctioned cleartext-to-loopback case
> the agent opened a `CONNECT` tunnel and sent the `Authorization` header through the
> proxy in the clear. The endpoint module's own justification for that exception is
> "there is no network to observe"; an inherited proxy created the observer. Proxy
> inheritance is now disabled and a test with a listening, counting proxy guards it.
>
> Recorded here rather than by editing the paragraph above, because the paragraph is
> what this document asserted on 2026-09-19 and a research snapshot that quietly becomes
> correct is worth less than one that shows where it was wrong. The lesson generalizes:
> "there is no code path" is a claim about a whole dependency tree, not just about the
> code in this repository.

### 3. Uncertainty is a first-class concept, and the exit codes reflect it

`semdecide` gets the exit-code idea right (uncertain is not false). `model-clis/jev`
gets unevaluable-assertion handling right. Neither does both, and most of the field
flattens the distribution into a verdict.

`jev`:

- exit `0` means the API answered, never "the answer was yes";
- `1` is an evaluated gate that did not hold;
- **`6` is a gate that could not be evaluated** — a distinct code, never a pass, never
  confused with a negative judgment;
- `5` is a partial batch;
- the full probability distribution is in `--output json` always, and a Noul never grows
  a `confidence` field, because the API does not return one.

### 4. Verifiable artifacts

One project in the field publishes checksums. None publishes build provenance.

At the time of this comparison, the planned release pipeline produced archives,
per-artifact checksums, an SPDX SBOM, and optional build provenance. Its smoke test
unpacked and ran the binary on four platforms through automatic CI. That CI has since
been removed. The current workflow is manual and can publish after explicit human
confirmation; see [ADR-0013](../adr/0013-local-verification.md) and
[ADR-0014](../adr/0014-manual-release-gate.md).

### 5. Hostile input is actually tested

No project in the field uses property or fuzz testing. For tools whose job is parsing
untrusted JSON off a network, that is conspicuous.

`jev` has 357 tests across unit, property, doctest, integration, real-socket transport,
and compatibility layers, plus five fuzz targets. (642 and six targets as of 2026-09-20,
after `jev eval` landed; the figure above is the one this snapshot was taken against, and
the counts below it move with every session — re-measure rather than quoting them.) Each
fuzz target asserts a **domain
invariant**, not just absence of a crash: an unevaluable gate never passes, every
accepted endpoint is TLS or loopback, no decoded answer violates its own distribution,
and the input byte limit is exact.

### 6. A calibration command, which nothing else in the field has

Every CLI here, this one included, tells the user that a threshold is theirs to choose
and that `0.8` is a placeholder. Only `jev` ships something that lets them stop guessing.

`jev eval` takes the user's **own** labelled JSONL, reuses the same request file `ask`
and `map` take, selects a threshold under a **named objective** routed by question type,
holds rows back by default so the reported number is not the number the threshold was
picked to maximize, reports a 95% Wilson interval and the `n` beside every headline, and
records the dataset fingerprint, the question fingerprint, and the concrete model version
that answered.

Nothing in the corpus is equivalent. The nearest things are all maintainer tools rather
than user-facing features: `sufianetaouil/every`'s `--selftest` is a fixed twenty-row
bundled check at a hard-coded `0.75`; `shiftynick/jev-axi`'s `pnpm eval` is a regression
harness over its own recipes, run from the TypeScript source tree; `keltokhy/jgrep`'s
`bench/accuracy.py` is a one-off script against public datasets. None of them accepts the
user's own labels. `semdecide`'s own specification names "calibration tooling" as a
differentiator it wants and has not built.

Whether it is *useful* on somebody else's real labelled data is untested — see the
blockers below. What is true today is that the category is empty apart from this.

### 7. The machine contract is a mechanism, not a promise

`semdecide` is the only other project that versions its JSON. `jev` puts a `schema`
field in every document, documents an additive-only policy, and backs it with
compatibility fixtures transcribed from official TypeSafe documents that fail CI if
decoding drifts.

### 8. Faithfulness to the API's vocabulary

Two projects invented their own question vocabulary — `jtsang4` exposes `"boolean"`
returning `probability` (gateway language, not TypeSafe's), and `y0usaf` uses a
`questions` array with `options`/`levels` instead of the wire format's map with
`criteria`. Both make the official documentation misleading for their users.

`jev ask` takes the **official API request body**. An example copied from
<https://docs.typesafe.ai/api> runs unchanged, and there is a test asserting exactly
that.

### 9. Startup and footprint

1.2 ms for `jev --version` (0.6 ms net of process spawn), a 4.6 MB binary, 8 direct
runtime dependencies, 89 crates in the runtime graph, no async runtime. (Re-measured
2026-09-20 after `jev eval`; the binary and the direct-dependency count grew, the
runtime graph did not.) The only other
native binary in the field, `model-clis/jev`, carries 200 packages including a full
Tokio runtime. See [`../benchmarks.md`](../benchmarks.md) for methodology and for the
two measurement mistakes that were made first.

---

## Where others are still ahead

### `model-clis/jev` — a richer assertion language

Its grammar has `in` with list literals, boolean literals, and `["quoted"]` index
syntax, and its paths resolve against the whole response envelope including `model` and
`usage`. `jev`'s grammar addresses answers only and has no `in` operator, so
`team.choice in [billing, returns]` has to be written as two `or`-ed comparisons.

That is a real ergonomic gap. It was not copied because the grammar was kept
deliberately small for fuzzing and review, but `in` is worth adding — it is a
contained change and the common case is a set membership test.

Its treatment of an unresolvable path as a *usage error* is also arguably better than
`jev`'s distinct exit `6`, and is worth revisiting: both are correct in refusing to pass,
and reasonable people could prefer either.

### `model-clis/jev` — presets, and a real distribution story

It ships named request templates discoverable from `./.jev/presets` and the user config
directory, with offline validation. `jev` has request *files*, which is the same idea
without the discovery, and the discovery is convenient.

It is also genuinely distributed today: Homebrew tap, Scoop bucket, checksum-verified
installers, automated releases for three platforms. `jev` has none of that — the
machinery is built and rehearsed, but nothing is published.

**Deliberately not copied:** repository-local preset discovery. Reading `./.jev/presets`
from the current directory means cloning a repository can change what a command does.
That is the same class of hazard as automatic `.env` loading, which `AGENTS.md` §3.3
bans. If `jev` ever adds discovery, it will be from the user's configuration directory
only, with the repository-local path behind an explicit flag.

### `Nasrallah-AL/jev-cli` (`jevctl`) — output formats

`--format csv|tsv|jsonl|md` and `--pluck`. `jev` has JSON, JSONL for `map`, a text
rendering, and `--value`. For someone piping into a spreadsheet, CSV is genuinely
useful and `jev` does not have it. `jq -r '… | @csv'` covers it, but that is an answer,
not a feature.

### `sufianetaouil/every` — evidence

It publishes a `--selftest` with recall and AUROC on a labelled set. `jev` publishes
CLI-overhead benchmarks and explicitly declines to benchmark Jev itself. That is honest,
but `every` demonstrates something `jev` does not: measured behaviour on real data.

### `shiftynick/jev-axi` — a published negative result

It measured whether its own feature helped, found it could not detect a difference, and
said so (`docs/skills-do-not-get-used.md`). That is the highest evidential standard in
the field and `jev` has not matched it. Nothing here has been evaluated against a real
workload.

### `keltokhy/jgrep` — cost estimation before spending

`--estimate` reads to EOF and reports call count and approximate cost with no
authentication and no API call. `jev --dry-run` shows what would be sent but does not
estimate what it would cost. For a `map` over 10,000 records, that is a question people
will ask.

**Why it is not in yet:** a credible estimate needs a tokenizer, and hard-coding prices
is forbidden (they change, and the docs are the authority). A call count and a byte count
are cheap and honest, and are the right first step.

### `tumf/jev-cli` — an MCP server

Ships `jev-mcp` alongside the CLI. `jev` has an agent skill and no MCP server. For
agent integration specifically, an MCP server is a more direct interface than a CLI plus
a skill.

### `Stumble/jev-go` and `AbdelStark/s1-rs` — a usable library

Both are SDKs first. `jev`'s crates are `publish = false` and promise no API, on
purpose ([ADR-0003](../adr/0003-cli-compatibility.md)) — but someone who wants to call
Jev from Rust is better served by `s1-rs` or `typesafe-ai-rs` today, and should be told
so rather than pointed here.

---

## Intentionally omitted

Each of these exists in at least one competitor and was left out on purpose.

| Not built | Why |
| --- | --- |
| Task commands (`verify`, `screen`, `triage`, `route`, `guard`, `compact`) | Each bakes in a prompt and a threshold that were never evaluated on the user's data. They are recipes; the CLI should make recipes *expressible*. |
| Multi-provider gateway support | Every provider hop is somewhere a key and the user's data can go. Direct TypeSafe access, correct and safe, comes first. |
| Response caching | A cached model judgment is stale in a way that is invisible. `map --resume` gives restartability without it. |
| Usage statistics on disk | A write nobody asked for. |
| Self-update (`jev update`) | A binary that rewrites itself is a supply-chain surface. Updates come from the package manager the user chose. |
| Automatic `.env` loading and fixed-path credential files | Cloning a repository must not change where credentials come from. |
| Repository-local preset discovery | Same hazard, one step removed. |
| A synthesized Noul confidence | The API does not return one. Inventing it gives scripts a number with no defined meaning. |
| Record chunking, language-aware splitting, semantic grep | Excellent in purpose-built tools. In a general CLI they add dependencies, a parsing attack surface, and an implicit promise to keep up with languages. |
| An interactive wizard | The opposite of a scriptable Unix tool. |

---

## Remaining blockers before a stable 1.0

Ordered by how much each would cost to get wrong.

1. **The real API has been exercised once, on one day.** On 2026-09-23 the live test
   matrix passed and a manual pass exercised `ask`, `map` (including `--resume`, gates and
   review files), `eval`, `doctor --live`, `auth status`, credential errors, server-side
   validation, and all five `jev mcp serve` tools (about 185 requests;
   `docs/api-compatibility.md`, "Live verification"). `auth login` against a real store,
   `config`, and `completions` were not exercised. That is one account, one model
   version, and small synthetic inputs. Nothing runs it on a schedule, so a
   service change is found by a user first.
2. **Published artifacts need independent verification.** This comparison reviewed
   local rehearsals and the earlier CI setup, not a downloaded release artifact. Check
   the checksum and run `gh attestation verify` on the version you intend to install.
3. **No external user has tried it.** Every usability judgment here is the author's own.
4. **The `--require` grammar is frozen the moment someone scripts against it.** Adding
   `in` afterwards is easy; changing precedence or path syntax is not. Decide before 1.0.
5. **`map`'s output-file handling appends without locking.** Two concurrent runs against
   one file interleave. Documented, not prevented.
6. **`jev eval` has never been run on anyone else's labelled data.** Its statistics are
   verified against hand-computed values and its behaviour against a mock, which is the
   right bar for correctness and says nothing about whether the thresholds it picks hold
   up in someone's actual pipeline. There is no published evidence either way.
7. **Cost visibility is absent**, and `jev eval` sharpens it: one request per labelled
   row per run makes it the highest-volume command in the tool by construction.
   `--dry-run`, `--limit`, and the pre-flight row count are the controls; nothing
   estimates tokens or money. `jgrep --estimate` shows the shape of an honest version —
   a call count and no dollar figure, since prices are the API documentation's to state
   and they change.
8. **Reproducible builds are unverified**, and Windows signing and macOS notarization are
   absent. Documented rather than claimed, but they are real gaps for a security-forward
   tool.
9. **No client-side rate limiting, and no batch circuit breaker.** `jev` reacts to 429
   rather than predicting it. Defensible while the published limits change without
   notice, but a throttled account running `map` or `eval` will exhaust a retry budget
   on every remaining row rather than noticing and stopping.

---

## Migration and interoperability

**From `model-clis/jev`.** Command names largely coincide (`noul`, `choice`, `score`,
`ask`, `map`), which is not an accident — it is the vocabulary the API uses. Differences
that will bite:

- Credentials do not transfer. `jev` will not read
  `~/model-clis/jev/credentials.json`; run `jev auth login` or export `JEV_API_KEY`.
- Assertions: `--assert` becomes `--require`, and there is no `in` operator yet.
- Presets: copy the JSON into a request file and pass `-r`.
- Exit codes differ. Re-read [`../cli-contract.md`](../cli-contract.md) before porting
  a CI gate; in particular a failed gate and a broken gate are different codes here.
- `jev` will not pin you to `jev-latest` — set `--model` or `jev config set model`.

**From `jtsang4/jev-cli`.** That CLI has a command literally named `eval`, and it means
something entirely different: one request against shared state, with no labels, no
statistics, and no threshold. Its nearest equivalent here is `jev ask`. `jev eval` is a
calibration command and will refuse a file that is not a labelled dataset.

**From `semdecide`.** The exit-code philosophy is compatible; the codes are not
identical. `semdecide`'s "uncertain" is a threshold decision, which in `jev` is expressed
as an explicit `--require` on `confidence` or on the probability.

**From the official SDKs.** `TYPESAFE_API_KEY` works unchanged. A request body written
for the Python or JavaScript SDK works as a `jev ask -r` file verbatim.

**To anything else.** `jev`'s JSON output carries the API's own field names, so
`jq '.answers.x.choice'` reads the same as the SDK's `response.answers["x"].choice`.
Nothing in the output is proprietary to this tool except the `schema` field and the
`gate` object.

---

*Sources: the corpus in `references/`, at the commits in
[`reference-manifest.md`](reference-manifest.md). Official API behaviour is cited from
the documentation snapshot in `references/05-typesafe-docs/pages/`, re-verified live
during implementation. Community projects are evidence about the landscape, never about
the API.*
