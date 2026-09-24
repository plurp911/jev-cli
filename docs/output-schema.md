# Output schema

Every `--output json` document carries a `schema` field. Branch on it.

Compatibility rules are in [`cli-contract.md`](cli-contract.md): fields may be added,
never removed or renamed; read by name, tolerate unknown fields, tolerate unknown values
in enumerated string fields.

**A document on stdout does not mean success.** Exit status carries the outcome, and a
gate that did not hold (`1`) or a batch with failing rows (`5`) still writes the data
you paid for. Two statuses write *nothing* to stdout, and a script that reads stdout
unconditionally will misparse them: `70`, an internal error, and `74`, a failure to
write output such as a full disk. The full table is in
[`cli-contract.md`](cli-contract.md).

---

## `jev.evaluation/v1`

Emitted by `noul`, `choice`, `score`, and `ask`.

```json
{
  "schema": "jev.evaluation/v1",
  "model": "jev-1.13.0",
  "model_requested": "jev-latest",
  "endpoint": "https://api.typesafe.ai",
  "answers": {
    "is_urgent": { "type": "noul", "noul": 0.92 }
  },
  "usage": { "input_tokens": 312, "output_tokens": 48 },
  "request_id": "req_abc123"
}
```

| Field | Type | Notes |
| --- | --- | --- |
| `schema` | string | Always `jev.evaluation/v1`. |
| `model` | string | The model the API says answered. Log this, not `model_requested`. |
| `model_requested` | string | What was asked for. Differs when a moving alias resolved. |
| `endpoint` | string | Where the request went. |
| `answers` | object | One entry per question, keyed by the id you chose. |
| `usage.input_tokens` | integer or null | Billable. `null` if the API did not report it. |
| `usage.output_tokens` | integer or null | Currently free of charge. |
| `request_id` | string or null | The API's own identifier for the call, from the `x-typesafe-request-id` response header. `null` if the API did not send one. Log it: it is what TypeSafe support can use to find a specific call, and it cannot be recovered afterwards. |
| `gate` | object | Present only with `--require`. See below. |
| `missing_answers` | array of strings | Present only when the API returned no answer for some question ids. Never read a missing answer as a negative one. |

### Answer objects

The answer objects **are the API's own shapes**, field for field. Knowledge transfers
between <https://docs.typesafe.ai/api>, the official SDKs, and this CLI without
translation.

#### Noul

```json
{ "type": "noul", "noul": 0.92 }
```

`noul` is the probability that the answer is yes, from 0 to 1. **There is no
`confidence` field, and there never will be**: the API does not return one for a Noul.
`0.5` means yes and no are similarly likely — not "medium".

#### Choice

```json
{
  "type": "choice",
  "choice": "technical",
  "confidence": 0.82,
  "probabilities": { "billing": 0.08, "technical": 0.85, "sales": 0.07 }
}
```

`probabilities` always contains every option and sums to 1. `confidence` summarizes how
concentrated that distribution is.

#### Score

```json
{
  "type": "score",
  "score": 1.6,
  "confidence": 0.78,
  "legend": { "0": "Calm", "1": "Frustrated", "2": "Very angry" },
  "probabilities": { "0": 0.05, "1": 0.3, "2": 0.65 }
}
```

`score` is the probability-weighted mean of the level numbers and can fall between
levels. `legend` maps each level number back to the description that was sent.

#### An answer type this version does not know

```json
{ "type": "ranking", "unrecognized": true, "…": "the API's payload, verbatim" }
```

If TypeSafe adds a fourth primitive, `jev` passes the answer through and flags it rather
than failing the response or dropping the answer. Check `unrecognized` before assuming a
`type` is one of the three.

### `gate`

Present only when `--require` was given.

```json
{ "expression": "is_urgent.noul > 0.9", "result": "passed", "passed": true }
```

| `result` | `passed` | Exit code |
| --- | --- | --- |
| `passed` | `true` | `0` |
| `failed` | `false` | `1` |
| `unevaluable` | `false` | `6` |

`unevaluable` additionally carries `reason`. **Do not treat `unevaluable` as `failed`.**
One is the model's judgment; the other is a broken gate.

---

## `jev.models/v1`

```json
{
  "schema": "jev.models/v1",
  "endpoint": "https://api.typesafe.ai",
  "models": [
    { "name": "jev-latest", "description": "…", "release_date": "2026-09-15" }
  ]
}
```

This is whatever `GET /v1/models` returned. `jev` keeps no model catalogue of its own,
and the API accepts versioned identifiers that do not appear in this list.

---

## `jev.doctor/v1`

```json
{
  "schema": "jev.doctor/v1",
  "version": "0.0.0",
  "platform": { "os": "linux", "arch": "x86_64" },
  "endpoint": {
    "url": "https://api.typesafe.ai",
    "official": true,
    "secure": true,
    "source": "default"
  },
  "model": { "id": "jev-latest", "source": "default", "moving_alias": true },
  "config": { "path": "/home/you/.config/jev/config.toml", "state": "absent" },
  "credentials": {
    "effective_source": "environment",
    "sources": [
      { "source": "environment", "env_var": "JEV_API_KEY", "present": true, "set": true },
      { "source": "environment-file", "env_var": "JEV_API_KEY_FILE", "present": false, "set": false },
      { "source": "typesafe-environment", "env_var": "TYPESAFE_API_KEY", "present": false, "set": false },
      { "source": "os-keychain", "env_var": null, "present": false, "set": false }
    ],
    "store": "Secret Service (D-Bus)",
    "store_error": null,
    "error": null
  },
  "limits": { "timeout_seconds": 10, "max_retries": 2, "max_input_bytes": 1048576 },
  "live": { "checked": false }
}
```

`source` fields take `flag`, `config-file`, or `default`. `config.state` takes `loaded`,
`absent`, `skipped`, or `no-directory`. `present` is `null` when a source could not be
checked, which is not the same as `false`.

`credentials.error` distinguishes **nothing is configured** from **something is
configured and broken**, which `effective_source: null` alone cannot. It is `null` on an
ordinary unconfigured machine, and otherwise names the reason — a blank `JEV_API_KEY`,
an unreadable `JEV_API_KEY_FILE`, a key with a line break in it — without echoing a
value or a path. Such a source reports `present: false`, because `present` describes
whether a usable value was found, and `set: true`, because the variable exists (or the
store holds an entry) and resolution stops at it rather than falling through. `set` is
`null` when `present` is.

With `--live`, `live` becomes
`{"checked": true, "ok": …, "detail": "…", "attempts": …, "milliseconds": …}`.

`doctor` exits `0` whenever it managed to produce a diagnosis. Branch on the fields, not
the status.

---

## `jev.auth/v1`

`jev auth status` reports where a credential would come from, never its value:

```json
{
  "schema": "jev.auth/v1",
  "action": "status",
  "endpoint": "https://api.typesafe.ai",
  "official_endpoint": true,
  "store": "macOS Keychain",
  "store_error": null,
  "error": null,
  "effective_source": "os-keychain",
  "sources": [ { "source": "environment", "env_var": "JEV_API_KEY", "present": false, "set": false } ]
}
```

`action` is `login`, `status`, or `logout`, and the other fields depend on it:

| `action` | Document |
| --- | --- |
| `status` | As above: `endpoint`, `official_endpoint`, `store`, `store_error`, `error`, `effective_source`, `sources`. `error` is the same field as `credentials.error` in `jev.doctor/v1`. |
| `login` | `store`, `stored` (bool), and `shadowed_by` — the sources that would still win over what was just stored, because environment beats keychain (ADR-0008). A non-empty `shadowed_by` means the new credential will not be the one used. |
| `logout` | `store`, `removed` (bool), and `still_set` — the environment variables that remain, so "logged out" is not mistaken for "no credential available". |

```json
{ "schema": "jev.auth/v1", "action": "logout", "store": "Secret Service (D-Bus)",
  "removed": false, "still_set": [] }
```

No value ever appears in any of them.

`jev auth status` exits `3` when no credential is available, so it works as a
precondition check.

---

## `jev.config/v1`

One document per invocation, with `action` naming which shape it is:

```json
{ "schema": "jev.config/v1", "action": "list", "path": "…/config.toml",
  "exists": true, "settings": { "model": "jev-1.13.0" } }
```

```json
{ "schema": "jev.config/v1", "action": "get", "key": "model",
  "set": true, "value": "jev-1.13.0" }
```

```json
{ "schema": "jev.config/v1", "action": "set", "key": "model",
  "value": "jev-1.13.0", "path": "…/config.toml" }
```

```json
{ "schema": "jev.config/v1", "action": "unset", "key": "model", "path": "…/config.toml" }
```

```json
{ "schema": "jev.config/v1", "action": "path", "path": "…/config.toml" }
```

| `action` | Fields beyond `schema` and `action` |
| --- | --- |
| `list` | `path`, `exists`, `settings`. `settings` is `{}` when the file does not exist; `exists` tells the two apart. |
| `get` | `key`, `set`, `value`. `value` is `null` when `set` is `false`. |
| `set` | `key`, `value`, `path`. |
| `unset` | `key`, `path`. Unsetting something already unset is not an error. |
| `path` | `path`. Printed whether or not the file exists, because it is where the file *would* go. |

`jev config get` on a setting that is not set exits `1`, so it works in a shell
conditional. The configuration file holds no secrets — a key whose name looks like a
credential is a load error — so nothing here is redacted.

---

## `jev.dry-run/v1`

```json
{
  "schema": "jev.dry-run/v1",
  "method": "POST",
  "url": "https://api.typesafe.ai/v1/systemone",
  "headers": ["accept", "authorization", "content-type", "user-agent"],
  "body": { "state": "…", "model": "jev-latest", "questions": { } },
  "body_bytes": 108,
  "credential": { "available": true, "source": "environment" },
  "gate": { "expression": "answer.noul > 0.9" },
  "sent": false
}
```

`body`, `url`, `method`, and the header names all come from the same function that
builds a real request (`jev_client::build_evaluation_request`), so a dry run cannot
describe a request that differs from the one a live run would send. A test asserts the
two are equal.

`body` is that request's body, decoded from its bytes. JSON object key order is not
preserved by the decode and carries no meaning; `body_bytes` is the exact length of what
would go on the wire.

`headers` lists names only. `authorization` is listed because the request carries one,
but no credential is read to produce this document — which is why there is nothing to
redact. `host` and `content-length` are added below this layer by the HTTP client and
are not listed.

`credential` reports whether a credential is available and which source it would come
from, so a rehearsal can tell you the run would fail on authentication. It never
contains a value. `source` is `null` when none is available.

`gate` carries the parsed `--require` expression, so it can be confirmed before a
request is spent on it. It is `null` when `--require` was not given.

`sent` is always `false`.

For `jev map --dry-run`, the document carries `records` (the total), `questions`,
`sample` (up to three record bodies, each with `index`, `id`, `body`, and `body_bytes`),
and `sample_truncated`, which says whether `sample` is all of them.

For `jev eval --dry-run`, the document carries `records` (the total that would be sent —
the same key `map` uses, for the same thing), `sample`, `sample_truncated`, and `split`
(`mode`, `seed`, `test_fraction`, `calibration_rows`, `reported_rows`), which is the same
object `jev.eval/v1` reports. The split is computed locally, so a dry run shows exactly
how the rows would be divided without sending anything.

---

## `jev.map.row/v1` and `jev.map.summary/v1`

`jev map` writes JSONL: one row document per input record, in input order, followed by
one summary document on stdout.

```json
{ "schema": "jev.map.row/v1", "index": 0, "id": "T-1", "state_digest": "8f1b2c3d4e5f6071",
  "request_digest": "77aa11bb22cc33dd", "ok": true, "model": "jev-1.13.0",
  "answers": { }, "usage": { }, "attempts": 1, "request_id": "req_abc123",
  "gate": { "expression": "urgent.noul > 0.9", "outcome": "passed", "reason": null } }
```

```json
{ "schema": "jev.map.row/v1", "index": 1, "id": "1", "state_digest": "1a2b3c4d5e6f7080",
  "request_digest": "77aa11bb22cc33dd", "ok": false, "attempts": 3, "request_id": "req_fail99",
  "error": { "kind": "unavailable", "message": "…" },
  "gate": { "expression": "urgent.noul > 0.9", "outcome": "not-evaluated", "reason": null } }
```

`request_id` is the same field as in `jev.evaluation/v1`, and is present on failed rows
too — that is where it matters, because a row that failed is what you would ask TypeSafe
about. It is `null` when no response arrived or the API sent no header.

`attempts` is the number of HTTP attempts made for the row, retries included, and is
present on failed rows as well as successful ones. Each attempt may have reached the
server and been billed — a timed-out one included — so a failed row is where the count
matters. It is `0` only for an `invalid-request` row, where nothing was sent. Rows
written by an earlier version carry it only when `ok` is `true`.

`error.kind` is `auth`, `unavailable`, `request`, or `invalid-request`. Check `ok`
before reading `answers`.

`state_digest` is a change-detector `--resume` uses to tell whether the record at an
index is still the same one. Without `--id-field` the `id` is the record's position, so
comparing ids alone cannot detect a changed input; comparing the digest can. It is a
non-cryptographic hash of the state, it carries no state content, and it is not a
stable interface to compute yourself — treat it as opaque. A row without it, from an
earlier version, still resumes.

`request_digest` is the same kind of change-detector for the parts of the request that
are the *same* for every record: the question set and the model. `state_digest` catches a
changed input; nothing caught a changed question set, so resuming after editing the
prompt produced a file whose early rows answered one question and whose later rows
answered another, reported the batch complete, and exited `0`. Also opaque, also
tolerated when absent.

`gate` is present on every row, and `null` unless `--require` was given — a key that is
always there, so a consumer filtering on `.gate.outcome` gets a missing verdict rather
than a missing field.

| `gate.outcome` | Meaning | Written to |
| --- | --- | --- |
| `passed` | The expression held. | the main stream |
| `failed` | The expression was evaluated and did not hold. | `--review-file` |
| `unevaluable` | The expression named a field this answer does not have; `reason` says which. | `--review-file` |
| `not-evaluated` | The row failed before there was an answer to classify. | the main stream |

`reason` is a string only for `unevaluable`, and `null` otherwise. In `jev map` the
expression **routes; it does not gate** — the exit code keeps reporting whether the API
answered. Without `--review-file` every row still reaches the main stream, annotated;
nothing is ever discarded.

```json
{ "schema": "jev.map.summary/v1", "total": 100, "evaluated": 98, "resumed": 2,
  "succeeded": 97, "failed": 1, "complete": 99, "stopped_early": true,
  "interrupted": false,
  "gate": { "expression": "urgent.noul > 0.9", "passed": 80, "failed": 16,
            "unevaluable": 1 },
  "usage": { "input_tokens": 30264, "output_tokens": 4656, "rows_without_usage": 0 } }
```

| Field | Meaning |
| --- | --- |
| `total` | Records in this run: every record read from the input, or the `--limit` selection. |
| `resumed` | Records skipped because `--resume` found them already done. |
| `evaluated` | Records actually sent this run. |
| `succeeded` / `failed` | How those `evaluated` records turned out. |
| `complete` | `succeeded + resumed` — how many of `total` now have an answer. |
| `stopped_early` | The run did not reach every pending record. |
| `interrupted` | The specific reason was a signal. |
| `gate` | How `--require` classified the rows **this run answered**, or `null`. |
| `limit` | Present only when `--limit` was given; see below. |
| `usage.input_tokens` | integer or null. Total the API reported for the records **this run answered**. Billable. `null` when no record reported a count. |
| `usage.output_tokens` | integer or null. The same, for output tokens. Currently free of charge. |
| `usage.rows_without_usage` | Answered records whose response lacked either count. Non-zero means the totals are only a lower bound. |

`limit` records a `--limit` run, and is **absent** otherwise, so a summary without it
means every input record was in the run:

```json
"limit": { "limit": 50, "seed": 1, "input_records": 12000 }
```

`limit` is the `--limit` value, `seed` the `--seed` value or `null` for the first N, and
`input_records` the number of records read from the input. `total` is the size of the
selection, which is `input_records` when the limit was at or above it. A resumed output
file can hold rows for records outside the selection, from an earlier run with a wider
or different one; they are not counted in `total`, `resumed`, or `complete`.

The gate counts cover only the rows this run evaluated. A resumed row is not
re-classified, because it is not re-evaluated — that is the point of `--resume` — so
counting it would mean reading a verdict back out of a file and reporting it as though
this run had reached it.

`usage` is what the rows' own `usage` objects add up to, so it replaces summing them with
`jq`. It follows `jev.eval/v1`'s `usage`, plus one field:

- **It covers this run only.** A record `--resume` skipped was paid for by the run that
  answered it, not this one, and is not in the total. Across resumed runs, add up each
  run's summary, or sum the rows of the output file — and of the `--review-file` too,
  when one diverted rows, since an answered record's row is in exactly one of them.
- **`null` is not `0`.** A total is `null` when no answered record reported that count:
  "the API did not say" and "it cost nothing" are different facts.
- **A total is never low without saying so.** A record whose response carried no usage
  cannot be added, so it is counted in `rows_without_usage` instead. When that is not
  `0`, the totals are only a lower bound.
- **Failed records are in neither.** A failed row carries no usage — including one
  whose response arrived but could not be decoded, since the count is read from the
  decoded response — and `jev` does not estimate one. Each of its `attempts` may still have
  been billed; `failed` and the row's `attempts` are where that shows.
- **A retried record reports the response that answered.** Earlier attempts that failed
  reported no usage; `attempts` on the row says whether there were any.

Whether a count is billable is TypeSafe's pricing, not this CLI's: at the time of writing
<https://docs.typesafe.ai/models> says *"Charged per input token. Output tokens are
free."* The same totals are printed on stderr after the run, as one line such as
`624 tokens in, 96 out, over 2 answered record(s)`, unless `--quiet`.

A successful row also carries `missing_answers`, an array of question ids, when the API
returned no answer for them; the field is absent otherwise.

**Read `complete` against `total`, and check `stopped_early`.** `succeeded` alone
answers a different question: it counts only this run, so a resumed batch can report a
small `succeeded` and still be finished. `stopped_early` is true for `--fail-fast`
stopping as well as for an interrupt, which is why it is separate from `interrupted`.

With `--output-file`, the rows go to that file and only the summary goes to stdout —
which is what keeps `--resume` able to read the file back. A failure to write that file
ends the run with exit `74` and no summary, rather than reporting a partial batch in
which nothing failed.

## `jev.mcp.map/v1`

Returned by the `map` tool of `jev mcp serve` ([`mcp.md`](mcp.md)). An MCP result is one
value, so the rows and the summary that `jev map` writes as separate lines arrive
together:

```json
{
  "schema": "jev.mcp.map/v1",
  "rows": [ { "schema": "jev.map.row/v1", "index": 0, "id": "a-1", "ok": true, "…": "…" } ],
  "summary": { "schema": "jev.map.summary/v1", "total": 1, "succeeded": 1, "…": "…" }
}
```

`rows` holds one `jev.map.row/v1` per record sent, in input order. A rejected credential
stops the batch, so compare `summary.evaluated` with `summary.total`. `summary` is
`jev.map.summary/v1`. Both are exactly as above: the wrapper is the only new shape, and
it adds nothing but the two fields. The other four MCP tools return `jev.evaluation/v1`,
with one addition: when the API returned no answer for a question, a `missing_answers`
array names the ids. The CLI reports the same fact on stderr, which an agent never sees.

## `jev.eval.row/v1` — the labelled dataset `jev eval` reads

This is the one document in this file `jev` **reads** rather than writes. It carries a
`schema` for the same reason the written ones do: a consumer — here, `jev` itself — can
detect a wrong file rather than misinterpret one. Pointing `--dataset` at a `jev map`
output file is the likeliest mistake, and this is what catches it.

```json
{"schema":"jev.eval.row/v1","id":"T-1","state":"payouts have failed for three days",
 "labels":{"urgent":true,"team":"billing","severity":2}}
```

| Field | Rule |
| --- | --- |
| `schema` | Required, exactly `jev.eval.row/v1`. |
| `id` | Required. A non-empty string with no control characters, unique in the file. It is the split key, so a duplicate is refused rather than deduplicated. |
| `state` | Required. A string, object, or array — the same values a request document's `state` accepts, validated the same way. **This is the only field that is ever sent.** |
| `labels` | Required, non-empty. An object keyed by question id. Every key must name a question in the `--request` file; an unknown one is refused, not ignored. |

Label values are shaped by the question they label:

| Question type | Label |
| --- | --- |
| Noul | `true` or `false`. `1` and `0` are accepted, because exported data usually has them. |
| Choice | A string exactly equal to one of that question's declared option names. Not case-folded — the same exact comparison `--require 'x.choice == name'` makes. |
| Score | A whole level **index**, counting from `0`, inside that question's legend. Not the level's description: matching ground truth against the prose you wrote to prompt a model would be a guess. |

A row may label a subset of the questions. Each question is scored over whichever rows
label it, and the report says how many that was.

## `jev.eval/v1` — the calibration report

One document, on stdout with `--output json`, and optionally written to a file with
`--report`.

```json
{ "schema": "jev.eval/v1", "endpoint": "https://api.typesafe.ai",
  "model_requested": "jev-latest", "model": ["jev-1.13.0"],
  "evaluated_at": "2026-09-20T18:04:11Z",
  "dataset": { "sources": ["issues.jsonl"], "rows": 240, "fingerprint": "a1b2c3d4e5f60718" },
  "request": { "source": "triage.json", "fingerprint": "77aa11bb22cc33dd" },
  "split": { "mode": "seeded", "seed": 0, "test_fraction": 0.3,
             "calibration_rows": 168, "reported_rows": 72 },
  "objective": { "name": "min-precision", "target": 0.95, "threshold_field": "noul" },
  "questions": { },
  "rows": { "evaluated": 240, "failed": 0 },
  "usage": { "input_tokens": 91234, "output_tokens": 2400 },
  "warnings": [] }
```

| Field | Meaning |
| --- | --- |
| `model_requested` | The alias or identifier that was asked for. |
| `model` | Every concrete version that actually answered, sorted. Normally one. **This is the version a threshold was measured against**; `jev-latest` moves, so a report that recorded only the alias records nothing. |
| `dataset.fingerprint` | An opaque change-detector over the rows actually evaluated — ids, states, and labels. Two reports with the same fingerprint measured the same examples. `--limit` re-fingerprints, so it always describes what was measured rather than the file. |
| `request.fingerprint` | The same kind of detector over the question set and the model, shared with `jev map`'s `request_digest`. |
| `split.mode` | `seeded` (a held-out split of one dataset), `files` (`--calibration` and `--test`), or `none` (no threshold was selected, or `--no-split`). |
| `objective` | `null` when no `--objective` was given. `threshold_field` is `noul` or `confidence`. |
| `rows.failed` | Rows whose API call failed. The metrics cover only the rows that answered. |
| `warnings` | Every caveat that was also printed on stderr, carried in the document so a report read back months later still says what was wrong with it. |

Each entry of `questions` is keyed by question id:

| Field | Present for | Meaning |
| --- | --- | --- |
| `type` | all | `noul`, `choice`, or `score`. |
| `n` | all | Rows **on the reported side** that both labelled this question and got an answer of the right type. |
| `labelled` | all | Rows on the reported side that labelled it at all. `labelled > n` means some of those rows failed, or answered with a different type than the question asked for. Both counts cover the same rows, so they are comparable. |
| `headline` | choice, score always; noul only with a threshold | The one proportion worth leading with — `accuracy`, or `exact_agreement` for a Score — with a 95% Wilson interval and the `n` it came from. **It is the only number in the document that carries an interval.** A Noul has no accuracy until a cut turns its probability into a decision, so the key is absent without a resolved threshold rather than guessed at 0.5. |
| `objective_applies` | all | `null` with no objective; otherwise whether the objective is meaningful for this question's type. |
| `threshold` | all | The selected cut, or `null`. Never a number that failed the target. |
| `threshold_reachable` | all | `false` when the objective applied and nothing in the calibration rows met the target. |
| `threshold_tie_break` | all | Which rule broke a tie, in words. |
| `at_threshold_on_calibration` | all | What the cut did on the rows it was *chosen* on, or `null` when no cut was chosen. Compare it against `at_threshold`: a large gap is the threshold having fitted noise, which is what the split exists to reveal. |
| `at_threshold` | only with a resolved threshold | What the cut did on the rows it is *reported* on. This is the honest number. Absent — the key, not a `null` — when no objective was given, when the objective does not apply to this question's type, or when the target was unreachable. |
| `calibration` | all | `expected_error` (ten equal-width bins) and the non-empty `bins`. |
| `brier_score` | noul, choice | Mean squared error against the outcome. For a Choice, the classical multi-category form, summed over every declared option. |
| `log_loss` | noul | Mean negative log likelihood, clamped away from 0 and 1 so one confident miss cannot be infinite. |
| `threshold_sweep` | noul | One row per observed probability, with the full contingency table — including `negative_predictive_value`, so a two-sided abstention band can be built from two rows of it. |
| `coverage_sweep` | choice, score | One row per observed confidence: `coverage`, `abstention`, `accuracy_among_covered`, `risk_among_covered`. |
| `accuracy`, `macro_precision`, `macro_recall`, `macro_f1`, `per_class`, `confusion` | choice | Macro averages only. Micro-F1 is not emitted because in single-label multiclass it is exactly accuracy, which is already there under its own name. |
| `exact_agreement`, `adjacent_agreement`, `mean_absolute_error`, `quadratic_weighted_kappa`, `confusion` | score | `mean_absolute_error` is computed on the continuous score, not the rounded level. Kappa is quadratic-weighted, because a Score's levels are ordered; a Choice does not get one, because its options are not. |
| `rows` | `--show-rows` | Per-row label and answer, for the **reported** rows only. |

`rows` carries `total`, `evaluated`, `failed`, `stopped_early`, `interrupted`, and
`errors` (the failures grouped by kind, with one example message each). **Check
`stopped_early`**: an interrupted or cut-short run still writes a report, and every
number in it then describes only the rows that finished. The same fact is repeated in
`warnings`, so a `--report` file read back later says so on its own.

Every ratio is `null` rather than `0` when its denominator is zero. Precision with no
predicted positives is a question the data cannot answer, and reporting it as zero would
say the question was answered badly.
