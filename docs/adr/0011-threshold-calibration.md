# ADR-0011: Threshold calibration, and the statistics this CLI will and will not report

* Status: Accepted
* Date: 2026-09-20
* Relates to: [ADR-0003](0003-cli-compatibility.md), which makes the new `jev.eval/v1`
  document and the `jev.eval.row/v1` input format compatibility surfaces;
  [ADR-0010](0010-batch-evaluation-and-semantic-routing.md) §5, whose refusal to
  synthesize a Noul confidence is the binding constraint on §3 below;
  [`docs/cli-contract.md`](../cli-contract.md) "Thresholds are yours"; and
  [`docs/threat-model.md`](../threat-model.md) T8 and T9b.

## Context

Every threshold in this project's documentation, examples, cookbook, and agent skill is
a placeholder, and every one of them says so. That is the honest position: a threshold
encodes what being wrong costs *you*, TypeSafe's own guidance is to *"start with
conservative thresholds, test with your own data, and adjust"*
(<https://docs.typesafe.ai/confidence>), and `jev` will never ship a default.

It also leaves a gap. A user told not to invent a number still has to write one, and
nothing in the CLI helped them stop inventing it. The observable result, across the
community tooling surveyed in [`../research/comparison.md`](../research/comparison.md),
is `0.8` and `0.9` appearing in production gates with no evidence behind them at all.

Closing that gap means putting statistics in a CLI, which is the part that can go
wrong. A tool that reports a confident-looking number from twelve examples, or that
picks a threshold on the same rows it then quotes the performance of, is worse than no
tool: it converts a guess into a guess with a decimal point.

## Options considered

1. **Do nothing.** Keep saying "measure it yourself" and leave the measuring to the
   user. Honest, and what shipped until now. Rejected: it is advice nobody can act on
   without writing their own harness, and the failure mode it leaves in place — an
   invented threshold in a production gate — is the one that actually happens.
2. **A full evaluation framework**: sweeps, cross-validation, model comparison, an
   experiment store, plots. Rejected outright. It contradicts `AGENTS.md` §1
   ("Minimal — every feature is load-bearing"), and the surface area needed to do it
   properly is a different product.
3. **A single scalar "quality score".** Rejected: collapsing precision, recall,
   calibration, and coverage into one number is a value judgment about the cost of being
   wrong, which is exactly the judgment this project refuses to make for anyone.
4. **A scoped calibration command** — one question set, one labelled dataset, one model
   version, local arithmetic, a held-out split by default. Accepted.

## Decision

### 1. `jev eval` measures one question against the user's own labelled examples

It is not a benchmark of Jev and does not claim to say anything about the model in
general. The report names the question fingerprint, the dataset fingerprint, the model
alias requested, and the concrete model version that answered, so a threshold cannot be
quoted without the three things it depends on.

### 2. The dataset is JSONL and carries a `schema`; the question set is the existing request file

`jev.eval.row/v1` has `id`, `state`, and `labels` keyed by question id. The questions
stay in the request document `ask` and `map` already take, unchanged, so one committed
question set can be run, batched, and evaluated without being written three times.

The row format carries a `schema` field even though the request file deliberately does
not. A request file *is* the official API request body and versioning it would break
that; a dataset row is this project's own invention and follows this project's own rule.
It also catches the likeliest mistake — pointing `--dataset` at a `jev map` output file
— with a message that says what a row should look like.

Validation is total and happens before a single request: an unknown question id, a
Choice label that is not a declared option, a Score label outside the legend, a
duplicate row id. **A label is refused, never ignored.** A silently dropped label looks
scored and was not.

### 3. Objectives are routed by question type, because uncertainty is not uniform

ADR-0010 §5 refuses to synthesize a Noul confidence, and that refusal decides this
design:

- A **Noul** is swept on its **probability**. That is a decision cut: every row lands on
  one side of it, so there is no abstention and no coverage to trade against. The
  objectives are `maximize-f1`, `min-precision`, and `min-recall`.
- A **Choice** or a **Score** is swept on its **confidence**, which *is* an abstention
  axis — act above the cut, escalate below it. The objectives are `min-accuracy` and
  `target-coverage`.

Asking for a coverage objective on a Noul is a usage error, raised before anything is
sent, rather than a number invented to make the three primitives look uniform.

Every threshold is reported together with the objective that chose it and the rule that
broke ties. The word "optimal" appears nowhere in the output: the best cut under one
objective is a bad cut under another, and a bare "optimal threshold: 0.8" is how that
gets forgotten.

When nothing in the data meets a `--target`, the threshold is `null`, `reachable` is
`false`, and the exit code is `1`. It is never a relaxed number that does not do what
the flag asked for.

### 4. A held-out split is the default whenever a threshold is selected

Choosing a cut on the rows whose performance you then report is the classic way to
publish a number that does not survive new data, and it is the single most likely way
this command could mislead someone. So:

- With `--objective`, rows are split by a seeded hash of each row's `id`. The threshold
  is chosen on the calibration side and reported on the other.
- Keying on the `id` rather than the position means appending examples or re-sorting the
  file leaves existing rows on the sides they were already on, which is what makes two
  reports comparable.
- `--calibration` and `--test` supply the two sides explicitly. A row id in both is
  refused.
- `--no-split` is allowed, because with forty examples a user may have no better option,
  and it warns on stderr *and* records the warning in the document.
- **Without `--objective` there is no split**, because nothing is selected and therefore
  nothing can leak. Holding a third of a user's labelling work back to guard against an
  impossible leak would be waste dressed as rigour.

Every headline proportion carries a 95% Wilson interval and its `n`. Wilson rather than
the textbook normal interval because it stays inside `[0, 1]` and does not need a sample
size these datasets will often not have — and both failures show up exactly where a
calibration report is most likely to mislead.

### 5. The statistics that are refused, and why

| Refused | Reason |
| --- | --- |
| Micro-F1 for a Choice | In single-label multiclass it is exactly accuracy. A second name for the same number implies a signal that is not there. |
| Weighted kappa for a Choice | The API treats options as unordered; a distance-weighted statistic would depend on the order the user wrote them and change silently when they reordered the file. A Score's levels *are* ordered, so a Score gets one. |
| Per-level precision and recall for a Score | Treating ordered levels as independent classes scores "off by one" exactly as badly as "off by four". |
| A synthesized Noul confidence | ADR-0010 §5. |
| A two-sided abstention band search | Two-dimensional with compound tie-breaking. The full sweep is emitted instead, including `negative_predictive_value`, so a band can be read off two rows of the table. |
| A default `--objective` or threshold | `docs/cli-contract.md`: thresholds are yours. |
| A composite grade | Option 3 above. |
| Silent truncation past the row cap | `jev-core`'s stated rule: exceeding a limit is always reported. |
| Any forward-looking claim | The output describes a measurement that happened, in the past tense. |

### 6. One execution engine, not two

The bounded-concurrency loop `jev map` uses — shared pooled agent, interrupt between
rows, panic-tolerant collection, caller-controlled early stop — moved to
`crates/jev-cli/src/batch.rs` and both commands drive it. It is subtle code with several
documented hazards; written twice it would have to be fixed twice.

What `eval` deliberately does **not** reuse is `map`'s durable-JSONL machinery —
`--output-file`, `--review-file`, `--resume`, partial-line repair. That exists because a
production batch is expensive to lose. An eval dataset is "representative labelled
examples" scale, and a crashed run being cheaply re-run is an accepted scope cut.

## Consequences

- Two new compatibility surfaces: the `jev.eval/v1` report and the `jev.eval.row/v1`
  input format. Both follow the existing add-only rule.
- Exit `1` gains a third meaning — a target no threshold reaches — alongside a failed
  `--require` gate and `jev config get` on an unset key. It is the same underlying
  statement ("a condition was evaluated and did not hold"), and it lets a CI job ask
  "does this question still clear 95% precision?" and branch on the answer.
- A new write sink, the `--report` file, recorded in `docs/threat-model.md` T8 and given
  the same `0600` treatment as the configuration file and `jev map`'s row files.
- A new privacy invariant, T9b: labels never reach the API. It is enforced by the types
  — there is no path from a `Label` into an `EvaluationRequest` — and by a test that
  asserts no request body carries a `labels` object.
- The metric definitions are now a thing this project has to defend. They live in one
  pure module with no I/O, and every one is tested against a hand-computed value rather
  than against another run of the same code.

## Revisit if

- TypeSafe publishes official calibration guidance that contradicts a definition here.
  Theirs wins; `AGENTS.md` §6.
- Users ask for a two-sided abstention band often enough that emitting the sweep is
  demonstrably not enough. It would need its own decision about tie-breaking in two
  dimensions.
- Datasets large enough to make holding everything in memory a problem turn out to be
  real, rather than assumed.
- Anyone proposes cross-validation, a model-comparison mode, or an experiment store.
  Each is a different product; say so and stop.
