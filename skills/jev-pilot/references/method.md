# Running the pilot: the detail behind the sequence

`SKILL.md` carries the order and the non-negotiables. This is how each step is actually
done, what it looks like when it is done badly, and the arithmetic.

---

## Success criteria that survive contact

The criteria from step 1 are the whole reason a pilot means anything. Write them as a
file in the pilot directory before the first request, because a criterion in your head is
one you will adjust without noticing.

**State the asymmetry, not a single number.** "95% accurate" is rarely what anyone
means. What they mean is one of:

| Shape | Write it as |
| --- | --- |
| A wrong yes is expensive, a wrong no is cheap | minimum precision, with recall reported |
| A miss is expensive, a false alarm is cheap | minimum recall, with precision reported |
| Both matter about equally | F1, and say so |
| A human will handle what the model is unsure about | minimum accuracy **on the rows it handles**, plus the maximum escalation rate that is affordable |

That last row is the one most real workflows are in, and it is why `jev eval`'s
`min-accuracy` and `target-coverage` objectives exist for a Choice or a Score: above the
confidence cut you act, below it a person does.

**Name the cases that must not regress.** The three tickets everyone remembers, the
failure that caused the incident, the edge case the current rule was written for. They go
in the dataset, and they are checked individually in the error analysis regardless of
what the aggregate says.

**When a criterion was wrong, say so out loud.** Sometimes the bar was set without
knowing what was achievable. Revising it is legitimate; revising it silently is not.
Report both numbers and the reason, and let the reader decide whether the revision is
honest. The revised bar applies to the next measurement, on fresh rows — never to the run
that showed the original was unreasonable. Where a second human's judgments of the same
rows exist, their agreement is the reference point to check a bar against before
committing to it — not a hard ceiling, since a model can agree with the label more often
than a second person does, but a bar well above it is suspect, and where the two people
disagree is where the labels need adjudicating.

---

## Labels

### Finding them

Ask for these in order. Most teams have the first or second and have not thought of them
as a dataset.

1. A decision already recorded: a resolved ticket's category, a merged PR's label, the
   queue something was routed to, an approval or rejection.
2. A correction: a column recording that a human overrode the system, a re-route, a
   reopened ticket, an audit log.
3. A spreadsheet someone maintains by hand.
4. A sample labelled for the pilot, by a person, with the labelling rule written down.

### How many

There is no threshold that makes a number safe, but the shape of the answer changes:

- **Under ~50 rows** you are looking at a shape, not measuring a rate. Say that, and
  prefer `PROMISING — NEED MORE DATA` over a precision figure.
- **A few hundred** supports a headline number with a wide interval, and supports a
  calibration/test split.
- Per class matters more than in total. Forty rows spread over six classes measures
  nothing about the rare ones, and the rare ones are usually why the workflow is hard.

`jev eval` prints a 95% Wilson interval beside every headline number and the `n` it came
from. Quote both. "87% on 40 rows" and "87% on 4,000 rows" are different claims.

### The dataset file

JSONL, one labelled example per line, keyed by the same question ids the request file
declares:

```json
{"schema":"jev.eval.row/v1","id":"T-1","state":"…","labels":{"urgent":true,"team":"billing"}}
```

A row's `state` and explicitly supplied `images`/`videos` are sent with the request
file's model, questions, and selected provider options. Labels are compared locally
after the answer comes back; row `id` and `schema` are local dataset metadata.
Unsupported media is rejected for the selected provider before transmission. A Noul label is `true`/`false`, a Choice label is one of that question's option
names exactly, and a Score label is a level index from `0`. Validation is total and
happens before anything is sent — an unknown question id or an undeclared option is
refused rather than skipped, which is what stops a row looking scored when it was not.

---

## Measuring the incumbent

The comparison is only as good as this half, and it is the half that gets skipped.

- **An existing model call**: run it on the same rows. Keep its prompt and its model
  version in the pilot directory. If it is behind a service you cannot call, say so.
- **A heuristic or regex**: run the function. It is usually a few lines and the result is
  frequently much better than anyone expects — which is the finding.
- **A human**: you already have this if the labels came from recorded decisions, but be
  careful not to use the same labels as both the baseline and the ground truth. If the
  labels *are* the human's decisions, the human scores 100% by construction and the
  comparison is meaningless. Then the honest baseline is inter-rater disagreement, or no
  baseline at all, stated as such.

**Report the baseline with the same metrics, on the same rows, in the same table.** A
baseline measured differently is not a baseline.

---

## Reading the evaluation

### What to look at, in order

1. **The interval, not the point — and, for two systems on the same rows, the paired
   count.** Overlapping marginal intervals do not show that the two are equivalent, and
   they can overlap when a paired comparison would separate them. For accuracy, count the
   discordant rows — Jev right where the incumbent is wrong, and the reverse — and where
   the difference decides the verdict, apply an exact sign (McNemar) test to those two
   counts. That test is about accuracy only: when the criterion is a class's precision
   or recall, or a cost that weighs one error above the other, compare on that metric,
   on the rows it rests on, and do not let an accuracy difference stand in for it. Never
   read overlap as parity.
2. **Calibration.** The Brier score and the calibration error say whether the
   probabilities mean what they claim. That matters where a raw probability is used as
   it stands — a band written in probabilities, a number shown to a person. A threshold
   chosen empirically on calibration rows needs the ranking to be good, not the
   calibration, so do not reject a usable model for calibration alone. On a few dozen
   rows both figures are noise.
3. **The per-class numbers and the confusion matrix.** An aggregate hides a class that
   is entirely wrong — and `min-accuracy` has no per-class floor, so a target can be met
   on the aggregate while a rare, high-stakes class is wrong every time. Check the
   per-class figures against the cases step 1 said must not regress, by name, rather than
   against the headline.
4. **The threshold sweep.** `jev eval` reports the whole sweep, including negative
   predictive value — **computed on the reported rows**. It describes them; it is not
   for choosing. A two-sided band read off it — auto-accept above, auto-reject below,
   escalate the middle, written `not (x.noul > 0.3 and x.noul < 0.8)` — fits both cuts
   to the test rows, and every number beside them is then in-sample. Treat such a band
   as a candidate and re-measure it on fresh rows before quoting a figure for it.
   `jev eval` does not search for that band for you.

### The split

With `--objective`, rows are held back by default: the cut is chosen on the calibration
rows and the numbers beside it come from rows the choice never saw. The split is keyed on
each row's `id`, not its position, so appending examples or re-sorting the file leaves the
original rows on the sides they were already on — which is what makes two reports
comparable. Because it is keyed rather than counted, the realised split only approximates
`--test-fraction`; quote the `reported_rows` the report states, not the fraction you
asked for. `--dry-run` prints the split before anything is sent, which is the cheap way
to see how many rows the number will actually rest on.

`--calibration PATH --test PATH` supplies both sides yourself, for a versioned golden
set. A row id in both is refused.

`--no-split` chooses and reports on everything. It is allowed, because with forty
examples there may be nothing better, and then **the result is optimistic by an unknown
amount** and both the report's warnings and yours must say so.

### Cost control

`--limit N` caps the run at the first N rows, and the dataset fingerprint is recomputed
over the rows actually used, so the report never describes more than it measured. Use it
for the first pass. `--dry-run` prints the bytes for the first few rows and the split,
and sends nothing.

Rows fail independently; a run with failures exits `5`, the metrics cover the rows that
answered, and `rows.errors` groups the failures. Do not average over a partial run
without saying it was partial.

---

## Error analysis

Pull the rows out with `--show-rows`, which adds each **reported** row's `id`, `label`,
`predicted` and `correct` to the JSON report — for a Noul, `predicted` is the
probability; for a Choice or a Score it is the answer, without its confidence. Calibration
rows are deliberately excluded: they are selection-time working, and listing them beside
the estimate invites reading the two as one set. Where the grouping below needs a Choice
or Score confidence, it comes from a `jev map` over those rows — a second draw, so say
so beside anything read from it. The report does not carry the state either: look each
error up by `id` in the dataset the report names, because what the model was given is
where most confident errors are explained.

Four groups, and each one means something different:

| Group | Usually means |
| --- | --- |
| Confidently wrong | the state is missing or misleading, the question means something other than intended, or the label is wrong |
| Low confidence, wrong | working as intended — this is what the escalation band is for |
| Low confidence, right | the band may be set too conservatively; check what it costs |
| One class carrying the loss | the criteria for that class are weak, or it is under-represented |

Also check deliberately for **leakage**: something in the state that gives the answer away
and will not be there in production — a resolution note in a ticket you are classifying
by urgency, a label field left in the record, a timestamp that correlates with the
outcome. A pilot that measured leakage measures nothing, and it looks like the best
result you will ever get.

### The one revision

Legitimate: an option nobody could pick, a missing `other`, a criterion that means two
things, a Score level that does not describe a concrete situation, state that omits a
field the decision obviously needs. Fix it, say what you changed and why, re-measure.

**Re-measure means new reported rows.** The split is keyed on row `id`, so re-running
the same dataset after a revision lands on the *same* held-out rows — which is the
failure this whole section is about, moved up one level. The calibration rows may be
reused; the reported rows must be ones no previous measurement was read off, ideally a
freshly labelled batch. A second held-out number taken from the rows the first revision
was checked against is not a second measurement.

**The same applies to `--objective` and `--target`.** Trying several targets against one
split and reporting whichever cleared the bar reuses the reported rows exactly the way
rewording does. Commit to the objective and the target before the first run against a
given split, and if you change them, say so and treat the result as a new measurement.

Not legitimate: rewording until the number rises. The tell is that you cannot say what
was wrong with the previous wording, only that it scored lower. At that point the finding
is that the question needs rethinking, reported as such rather than tried again: before
the one justified revision, `REVISE AND RE-EVALUATE`, naming what was wrong; after it,
`REJECT FOR THIS WORKFLOW` for this question.

---

## The comparison

| Dimension | Source | Note |
| --- | --- | --- |
| Quality on the agreed metric | `jev eval` report, and the baseline run | with intervals and `n` |
| Escalated / uncertain fraction | the confidence cut | zero escalation is a design choice, not a win |
| Latency | measured during the run | your network, that day |
| Usage | `usage` in the responses | tokens, not a bill — a price is not in the data |
| Coverage | rows answered / rows sent | failures are part of the result |
| Operational complexity | what has to exist | hosted credentials/service or a separately managed local runtime and weights; provider, runtime, and model identity to record |
| Behaviour when unavailable | design | which direction the workflow fails in |

**Say which dimensions you could not measure.** A blank is information; a guess is not.

---

## The artifacts

```
pilots/<name>/
  criteria.md        written in step 1, before anything ran
  request.json       the question set
  labelled.jsonl     the dataset, if it is safe to store here
  baseline.md        how the incumbent was measured, and what it scored
  jev-report.json    jev eval --report: model version, fingerprints, metrics
  answers.jsonl      the raw rows, distributions kept
  SUMMARY.md         verdict first, then the numbers, then the per-class picture --
                     the confusion matrix for a Choice or Score, the relevant band of
                     the threshold sweep for a Noul -- then what it does not establish
```

`--report` writes mode `0600` on Unix and records the model version that actually
answered, the dataset fingerprint, the question fingerprint, the objective, the threshold
and the metrics — which is what makes it comparable against a later model release.

**Ask before persisting a sensitive dataset.** When the answer is no, keep it outside the
repository and record only its fingerprint, its size and its shape, so the report is still
reproducible by someone who has the data.

---

## What a finished pilot never contains

- A threshold `jev eval` did not measure on held-out rows under a named objective.
- A cost, latency or accuracy figure this pilot did not produce.
- A dollar saving. Tokens are in the data; prices are not.
- A comparison against a baseline nobody ran.
- A number from a run that was partial, without saying it was partial.
- A recommendation to adopt based on the same rows the threshold was chosen on.
- A verdict softened because effort was spent.
