# Cookbook

Seven recipes, built from the generic capabilities `jev` has: `map`, `--require`,
request files, and exit codes. They are **examples, not features**. There is no
`jev triage`, no `jev review-pr`, and no `jev rerank`, and there will not be — a
task-shaped command bakes in someone else's prompt and someone else's threshold, and the
threshold is the part that has to be yours.

Every request file in [`requests/`](requests/) is the official API request body
([`docs.typesafe.ai/api`](https://docs.typesafe.ai/api)), so the same file works with
`jev ask`, `jev map`, `jev --dry-run`, `curl`, and the official SDKs. Validate one
against [`../schema/request.schema.json`](../schema/request.schema.json).

> **Every threshold below is a placeholder.** `0.9` and `0.85` are there so the commands
> run, not because they are right for your data. TypeSafe's own guidance is to *"start
> with conservative thresholds, test with your own data, and adjust as you observe
> results"* ([confidence](https://docs.typesafe.ai/confidence)). Measure yours on
> labelled examples before you let one decide anything. See
> [thresholds](../docs/commands.md#thresholds-and-what-confidence-does-and-does-not-mean).

> **Everything you pipe in is sent to TypeSafe.** Each record becomes the `state` of one
> request. Read [what `jev map` sends](../docs/commands.md#what-jev-map-sends) before
> pointing any of these at a log export, a ticket dump, or a private repository.

---

## 1. Issue classification

Sort a JSONL export of issues into kind, severity, and whether anyone can act on it.

```sh
gh issue list --limit 500 --json number,title,body \
  | jq -c '.[] | {id: .number, text: (.title + "\n\n" + .body)}' \
  | jev map -r examples/requests/issue-triage.json \
      --id-field id --state-field text \
      --output-file triaged.jsonl -j 8

# The ones worth waking someone for.
jq -c 'select(.ok and .answers.severity.score >= 2.5 and .answers.actionable.noul > 0.8)' \
  triaged.jsonl
```

`severity` is a Score, so `score` is a probability-weighted position on the levels and
can fall between two of them. Read `probabilities` alongside it: a score of `1.0` can
mean all the probability sat on level 1, or half on level 0 and half on level 2.

## 2. Semantic log filtering

Keep the lines an operator should see. `--lines` treats each line as plain text rather
than as JSON.

```sh
kubectl logs deploy/api --since=1h \
  | jev map -r examples/requests/log-triage.json --lines -j 16 \
  | jq -r 'select(.ok and .answers.actionable.noul > 0.9) | .id'
```

The `id` is the input line number, so the result joins back to the source:

```sh
# Print the original lines that survived.
kubectl logs deploy/api --since=1h > lines.txt
jev map -r examples/requests/log-triage.json --lines -i lines.txt \
  | jq -r 'select(.ok and .answers.actionable.noul > 0.9) | (.id | tonumber) + 1' \
  | while read -r n; do sed -n "${n}p" lines.txt; done
```

## 3. A pull-request risk gate

This is the shape that needs `--require` rather than `map`: one state, one judgment, one
exit code.

```sh
git diff origin/main... \
  | jev ask -r examples/requests/change-risk.json --state-file - \
      --require 'public_api.noul < 0.5 or risk.score < 3'
```

`--state-file -` is doing real work there: with `-r` the state must be named, because a
request document can carry its own `state` and silently preferring stdin over the file's
own field would make the same document mean two things. (`--questions` is the form that
takes state from stdin implicitly, because a questions map has nowhere to put one.)

```
exit 0  the API answered and the gate held
exit 1  the API answered and the gate did not hold
exit 6  the gate could not be evaluated -- never a pass
exit 3  authentication
exit 4  the API could not be reached
```

That separation is the point. A CI job can tell "the policy says no" from "the API is
down", which `jev ... | awk` cannot:

```yaml
- name: Check change risk
  run: |
    set -o pipefail
    # The status is captured, not read after the fact. A GitHub Actions `run:` block is
    # `bash -e`, so the pipeline's non-zero exit ends the step *before* `case` runs --
    # which would collapse "the policy says no" back into "the job failed", the one
    # distinction this whole example exists to draw.
    git diff origin/main... \
      | jev ask -r examples/requests/change-risk.json --state-file - \
          --require 'public_api.noul < 0.5 or risk.score < 3' \
      && status=0 || status=$?
    case $status in
      0) echo "ok" ;;
      1) echo "::warning::this change looks risky; asking for a second review" ;;
      *) echo "::error::jev could not evaluate the gate (exit $status)"; exit 1 ;;
    esac
```

## 4. RAG candidate relevance

Rerank retrieved passages by asking about each one. The question is the same for every
candidate, which is exactly what `map` is for.

```sh
jq -c --arg q "$QUERY" '.[] | {id: .doc_id, state: {query: $q, passage: .text}}' candidates.json \
  | jev map -r examples/requests/relevance.json --id-field id --state-field state -j 16 \
  | jq -s 'map(select(.ok)) | sort_by(-.answers.relevant.noul) | .[0:5] | .[].id'
```

Nested state is addressed from the question text with backticked paths — the request
file says `` `query` `` and `` `passage` `` because that is what the record contains.

## 5. Dataset triage, with the uncertain rows set aside

The routing case. Rows that clear the bar go one way; everything else goes to a file a
person opens.

```sh
jev map -r examples/requests/issue-triage.json -i issues.jsonl \
  --require 'kind.confidence >= 0.85 and (actionable.noul > 0.7 or actionable.noul < 0.3)' \
  --output-file auto.jsonl --review-file review.jsonl -j 8
```

`auto.jsonl` holds the rows the expression passed; `review.jsonl` holds the rest, each
carrying a `gate` object saying whether it `failed` or was `unevaluable`. Nothing is
dropped, and the exit code still reports only whether the API answered.

**Look at why the two halves of that expression are written differently.** `kind` is a
Choice, so it has a `confidence` and a threshold on it is the natural thing to write.
`actionable` is a **Noul**, which has none — `actionable.confidence` would be
`unevaluable` for every row, so *every* record would land in `review.jsonl`, `auto.jsonl`
would be empty, and the run would still exit `0`. A Noul's uncertainty is the middle of
its probability range, so it is written as a band: keep the confident answers at both
ends and send the middle for review.

That is the single easiest mistake to make with this feature, which is why the row says
`unevaluable` rather than `failed` and why the summary counts them separately:

```sh
jq -c '.gate | select(.outcome == "unevaluable")' review.jsonl | head -1
```

## 6. Agent action classification

Classify what a proposed tool call would do, before running it.

```sh
echo '{"action": "rm -rf ./build && git push --force"}' \
  | jev map -r examples/requests/agent-action.json --state-field action \
  | jq -c 'select(.ok) | {reversible: .answers.reversible.noul, scope: .answers.scope.choice}'
```

Used as a gate on a single action:

```sh
jev ask -r examples/requests/agent-action.json --state-json "$ACTION" \
  --require 'reversible.noul > 0.9 and scope.choice != outside' \
  || echo "asking a human first"
```

`scope.choice != outside` compares the *selected* option by name. `scope.confidence`
would ask a different question — how concentrated the distribution is — and the two are
not interchangeable.

---

## 7. Calibrating the threshold the other recipes only guess at

Every number above is a placeholder. This is the recipe that replaces one with a
measurement. Label examples you already have — past triage decisions, resolved tickets,
a spreadsheet somebody kept — as
[`datasets/issue-triage.labelled.jsonl`](datasets/issue-triage.labelled.jsonl) does,
using the same question ids the request file declares:

```json
{"schema":"jev.eval.row/v1","id":"1841","state":"Crash on startup after upgrading…","labels":{"kind":"bug","severity":2,"actionable":true}}
```

Ask first how the question does at all, over every example:

```sh
jev eval -r examples/requests/issue-triage.json \
         -d examples/datasets/issue-triage.labelled.jsonl \
         --output json | jq '.questions.kind | {accuracy, macro_f1, per_class}'
```

Then ask where the cut should go. Recipe 1 filters on `actionable.noul > 0.8`; this is
what that `0.8` should have been:

```sh
jev eval -r examples/requests/issue-triage.json \
         -d examples/datasets/issue-triage.labelled.jsonl \
         --objective min-precision --target 0.9 \
         --model jev-1.13.0 --report baseline.json
```

The threshold is chosen on one part of the dataset and reported on another, so the
number printed beside it is not the number it was picked to maximize. Take the
`--require` line `jev eval` prints, put it in recipe 1, and pin the model version the
report names — the alias moves, and a threshold measured against what it meant last
month is not a threshold against what it means today.

Ten labelled rows, as here, is enough to see the shape and nowhere near enough to trust
a threshold: `jev eval` says so on stderr and in the report's `warnings`, and the 95%
interval beside each number shows how little ten rows settle. Collect examples that look
like the traffic you will actually see.

---

## Running these without an API key

`--dry-run` prints the exact request that would be sent and sends nothing, which is the
honest way to try these without a key:

```sh
echo '{"text": "hello"}' \
  | jev map -r examples/requests/issue-triage.json --state-field text --dry-run
```

The local mock in [`docs/commands.md`](../docs/commands.md) will also *run* these, but it
answers a single question called `answer`, and every recipe here asks for its own ids —
so the commands exit `0` while every `jq` filter and every `--require` finds nothing. If
you want a mock that exercises a recipe end to end, echo back the ids the request asked
for.
