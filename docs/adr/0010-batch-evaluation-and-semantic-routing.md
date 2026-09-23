# ADR-0010: Batch evaluation, semantic routing, and the bulk-content confirmation question

* Status: Accepted
* Date: 2026-09-20
* Relates to: [ADR-0003](0003-cli-compatibility.md), whose revisit trigger *"a streaming
  (JSONL) mode is added, which will need its own framing contract"* has fired;
  [ADR-0007](0007-workspace-architecture.md), which offered `std::thread` as a
  hypothetical and is now describing something that shipped; and
  [`docs/threat-model.md`](../threat-model.md) T9, which requires this record.

## Context

`jev map` sends one request per input record. `--require` turns a model judgment into a
process exit status for a single request, and classifies rows in a batch. Request files
let a team commit a semantic policy to version control. Three documents said these needed
a decision recorded and none had one.

**T9 is the binding one.** `docs/threat-model.md` states: *"Any future feature that makes
bulk local content easy to send needs an explicit confirmation path and an ADR."*
`jev map` is precisely that feature. It shipped with neither, which is a gap in the
record, not a licence to skip it now.

## Decision

### 1. JSONL is the batch framing, and the row document is a stable surface

One JSON document per line on stdout, one per input record, plus a summary document. Each
carries a versioned `schema` field like every other machine document (`jev.map.row/v1`,
`jev.map.summary/v1`), so the framing contract ADR-0003 asked for is the same contract
every other document already has: adding a field is compatible, removing or renaming one
is not. This ADR records that ADR-0003's trigger is discharged rather than outstanding.

Input order is output order on stdout. With `--output-file` the file is in *completion*
order, because rows are flushed as they finish so a killed process keeps them; every row
carries its `index`, so `sort` recovers input order. That asymmetry is deliberate and
documented rather than papered over.

### 2. No confirmation prompt. The controls are visibility and narrowing

T9 asks for "an explicit confirmation path". We considered and rejected an interactive
confirmation:

- `jev map` reads a pipe. A prompt in `producer | jev map | consumer` has no terminal to
  prompt on, so it would have to be skipped for non-TTY — which is every real use — and
  would therefore protect nobody while implying it protects everyone.
- The user named the input. Unlike a glob, a directory walk, or automatic context
  gathering — all of which `AGENTS.md` §3.3 bans outright — `jev map` sends exactly the
  records it was handed. There is no discovery step and no file the user did not name.
- A prompt people reflexively dismiss is worse than none: it moves the responsibility
  without moving the outcome.

What is provided instead, and what this ADR commits to keeping:

- **`--dry-run` sends nothing** and prints the request that would be built, with a
  transport that refuses every request so the guarantee is structural.
- **`--state-field`** narrows what leaves the machine to one field per record, rather
  than the whole record.
- **The count is on stderr before the batch runs**
  (`sending N record(s), M question(s) each, concurrency J`), at ordinary verbosity. It
  was first written with the verbose-only helper, which made it a visibility control
  nobody saw; `--quiet` still silences it, because that is an explicit instruction.
- **Row files are `0600` on Unix** — both `--output-file` and `--review-file`, which hold
  the model's answers about the user's state.
- **The documentation says it plainly**, in the `jev map` section rather than only in the
  threat model, because the threat model is not what a user reads before running a
  command.

This is the decision T9 asked for. If a future feature *does* discover content the user
did not name, the confirmation question reopens and the answer above does not carry.

### 3. `--require` in `map` routes; it does not gate

The same expression language, two uses. On a single request it becomes the exit status
(`1` failed, `6` unevaluable). In a batch it classifies each answered row, and
`--review-file` diverts the rows that did not pass.

It deliberately does **not** change the batch's exit code. The exit code answers "did the
API answer?", and a batch's semantic outcome is a distribution, not a verdict — collapsing
it into one status would destroy the distinction between "the API is down" and "sixteen
rows need a look", which is the distinction this project exists to preserve. The counts
are in the summary and the verdict is on every row, so a caller that wants to fail on them
does so explicitly.

Rejected: a second predicate flag for uncertainty (`--min-confidence`,
`--uncertainty-margin`). The gate language already addresses `confidence`, `noul`,
`choice`, `score`, and individual probabilities, and a dedicated confidence flag would
have to pretend the three primitives have uniform uncertainty. They do not — see §5.

### 4. Nothing is discarded, and failure is not uncertainty

Without `--review-file`, rows that did not pass still go to the main stream, annotated.
The alternative — dropping them and signalling only through an exit code, which is what
`semdecide` does — means a user paid for a judgment they will never see.

A row that *failed* is never diverted to the review file. "The API did not answer" and
"the API answered and the answer needs a look" are different problems for different
people, and a review file that is also the error log is worth much less.

An `unevaluable` gate routes to review, never to the main stream, for the same reason
exit `6` is not exit `0`.

### 5. No synthesized Noul confidence, and no default threshold

The official documentation is explicit: *"There is no separate `confidence` value for a
Noul… A Noul's probability distribution has only two outcomes, yes and no, so the single
`noul` value describes it completely"* (<https://docs.typesafe.ai/primitives/noul>), and
`confidence` on a Choice or Score is *"a statistic computed from the probability
distribution the answer already gives you"* (<https://docs.typesafe.ai/confidence>).

So `<id>.confidence` on a Noul is `unevaluable`, and `jev` computes nothing to fill the
gap. `|2p − 1|` is tempting and would be an invention: it would give a script a number
the API does not define, and it is symmetric about `0.5` while real Noul thresholds
usually are not.

No threshold has a default. A default threshold is a claim about data the tool has never
seen, and the official guidance is to set them from labelled examples and the cost of
being wrong. This is why there is no `jev triage` and no `jev review-pr`: a task-shaped
command's real payload is a prompt and a threshold that were not evaluated on the user's
data. `examples/` makes the recipes expressible instead.

### 6. Request files are the API's body, and carry no version field

The document `-r` reads is the official request body. Adding a `version` key would make
it invalid as an API body; stripping one before sending would make `jev` the owner of a
private extension to a format it explicitly does not own. The format is versioned the way
the API versions it, and `schema/request.schema.json` — a second, checked statement of
the same format — is versioned by its `$id`.

No YAML. The API speaks JSON, and a second surface syntax is a second parser and a second
set of edge cases for no capability JSON lacks.

### 7. Resume compares the request, not just the input

`--resume` reads the output file rather than caching judgments, so a resumed run never
replays a stale answer. It refuses to continue a file produced from different input *or*
from a different question set or model: rows carry a `state_digest` and a
`request_digest`, and both are compared. Rows written before a digest existed still
resume, because refusing every older file to gain a check is a worse trade.

Model judgments are not deterministic. `--resume` guarantees that a record is evaluated
once, not that evaluating it twice would agree.

## Consequences

- The row and summary documents join the compatibility surface; changing them is a
  breaking change.
- `jev map`'s real batch ceiling is `--max-input-bytes` over the whole file, not
  `MAX_RECORDS`. Records are read up front so output can be ordered and `--resume` can
  reason about indexes; that is a bounded-memory decision, and the bound is documented
  where `map` is documented rather than only where the flag is.
- Two expression uses share one parser, so a change to the grammar changes both. That is
  the intent — a second language would be the failure mode.
- We are committed to *not* shipping task-shaped commands. Pressure to add them will
  recur; the answer is a recipe in `examples/`.

## Revisit if

- A feature is proposed that reads content the user did not explicitly name. §2's answer
  does not carry to it.
- The API defines a confidence value for Noul, or publishes the formula for the existing
  one.
- A batch gate's exit status is genuinely needed in CI and `jq` over the summary turns out
  not to be enough.
- Streaming becomes necessary — inputs that cannot be read into memory at all would force
  giving up either input-order output or `--resume`, and that is a different decision.
