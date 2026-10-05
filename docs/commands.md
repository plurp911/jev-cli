# Command reference

Every command, every flag, and what each one does.

Help text is not a stable interface; this document and
[`cli-contract.md`](cli-contract.md) are the reference. Run `jev <command> --help` for
the short form.

## Global flags

These flags are parsed globally. Media and inference-option flags are accepted
only by commands that consume them; unrelated commands refuse them.

| Flag | Default | Effect |
| --- | --- | --- |
| `--provider <typesafe\|cloudflare\|ollama\|llamacpp\|huggingface>` | `typesafe` | Explicit inference protocol; [hosted/local Clef setup](clef.md). |
| `--cloudflare-account-id <ID>` | environment/configuration | Cloudflare-only, 32 hexadecimal characters; flag beats `CLOUDFLARE_ACCOUNT_ID`, then configuration. |
| `--image <PATH>` | none | Explicit PNG/JPEG/WebP image file; repeat in order for Cloudflare, Ollama, or the Hugging Face bridge. |
| `--video-frame <PATH>` | none | Prepared PNG/JPEG/WebP frames of one ordered video; repeat for the local Hugging Face bridge. |
| `--video-fps <FPS>` | none | Source cadence of the prepared video, greater than zero and at most 120; requires `--video-frame`. |
| `--max-length <TOKENS>` | bridge default | Local Hugging Face context control; 1–65,536. |
| `--max-state-tokens <TOKENS>` | none | Independent local Hugging Face state token budget; 0–65,536, also constrained by the total context budget. |
| `--media-kwargs <JSON>` | bridge default | Explicit validated local processor controls; see [Clef](clef.md). |
| `--reject-if-busy` | off | Requests Cloudflare capacity rejection without queueing; [Clef live availability remains unverified](clef.md#cloudflare). |
| `--keep-alive <DURATION>` | server default | Ollama-only model lifetime, such as `5m`, `0`, or `-1`. |
| `-o, --output <text\|json>` | `text` | Output format. `json` is the stable machine contract. |
| `--color <auto\|always\|never>` | `auto` | Colour policy for human output. Under `auto`, colour is used only when **stdout** is a terminal and `NO_COLOR` is unset. `--color always` is an explicit instruction and outranks `NO_COLOR`; `NO_COLOR` is about defaults. |
| `-m, --model <MODEL>` | `jev-latest` | Model identifier or alias. |
| `--endpoint <URL>` | `https://api.typesafe.ai` | API base URL. See [Custom endpoints](#custom-endpoints). |
| `--timeout <SECONDS>` | `10` | Per-attempt HTTP timeout, 1–3600 seconds. A timeout does not cancel running local inference. |
| `--retries <N>` | `2` | Retries after the first attempt. |
| `--max-input-bytes <BYTES>` | `1048576` | Ceiling on one input source. |
| `--no-config` | off | Ignore the configuration file entirely. |
| `--dry-run` | off | Print the request that would be sent; send nothing. |
| `-q, --quiet` | off | Suppress non-error diagnostics. |
| `-v, --verbose` | off | Extra diagnostics on stderr. Never prints credentials or bodies. |

There is no `--api-key`, and there will not be one. Arguments are visible in `ps`, in
shell history, and in CI logs.

The model/base defaults in this table describe TypeSafe. The other providers default
to `clef` and their documented hosted/local endpoint; see [Clef](clef.md). Requests,
batch/evaluation rows, and MCP calls may carry embedded images. `map --images-field
FIELD` explicitly selects per-record media. `--videos-field FIELD` selects prepared
embedded video frames for the Python bridge. Local loopback providers never read or
send credentials, and a request that fails provider validation is refused before
credential lookup or network use.

## Supplying state

Four mutually exclusive flags, plus stdin. Passing two is a usage error, not a
precedence rule to memorise.

| Flag | Meaning |
| --- | --- |
| `--state <TEXT>` | Literal text. |
| `--state-file <PATH>` | Text from a file. `-` means stdin. |
| `--state-json <JSON>` | Literal JSON: an object, an array, or a string. The [publisher Python bridge](clef.md) also accepts scalar JSON and blank strings. |
| `--state-json-file <PATH>` | JSON from a file. `-` means stdin. |
| *(none)* | Text from stdin. If stdin is a terminal, this is an error rather than a hang. |

In the synopses below, `[state source]` stands for whichever one of those you use. It
is not a positional argument — there is no bare `jev noul "…" "some state"`.

All providers reject invalid UTF-8, binary content (a NUL byte), a directory,
input over the byte limit, invalid JSON, and duplicate JSON object keys before
anything is sent. An empty JSON source and empty implicit stdin are also rejected.
TypeSafe, Ollama, and llama.cpp reject blank text state and numeric, boolean, or null
JSON state. Cloudflare permits blank text with validated images but still rejects
numeric, boolean, or null JSON state.
The publisher Python bridge accepts explicitly supplied blank text and JSON scalars,
including null. One trailing newline is stripped from text input, because the shell
added it. Nothing else is trimmed, and the CLI never silently truncates input.
Cloudflare and the Python publisher encoder may truncate state to their token budget;
Ollama instead refuses inputs exceeding its loaded context.

---

## `jev noul`

One yes/no question. The answer is `noul`: the probability that the answer is yes.

```
jev noul <INSTRUCTIONS> [--true TEXT] [--false TEXT] [--id ID] [state source] [--value] [--require EXPR]
```

| Flag | Effect |
| --- | --- |
| `--true <TEXT>` | What a yes means. Optional; use it when the boundary is subtle. |
| `--false <TEXT>` | What a no means. |
| `--id <ID>` | Question id, used as the JSON key and in `--require`. Default `answer`. |
| `--value` | Print just the probability. |
| `--require <EXPR>` | See [Gating](#gating). |

```console
$ echo "My payouts have been failing for 3 days" | jev noul "Does this convey urgency?"
answer
  yes  0.9200  ██████████████████████··
```

A Noul carries **no confidence**. The API does not return one, and `jev` does not invent
one. `0.5` means yes and no are similarly likely — not "medium".

---

## `jev choice`

One question that selects from named options.

```
jev choice <INSTRUCTIONS> (-O NAME[=DESC] ... | --options-file PATH) [--id ID] [state source] [--value] [--require EXPR]
```

| Flag | Effect |
| --- | --- |
| `-O, --option <NAME[=DESCRIPTION]>` | One option. Repeat. Split on the first `=`. |
| `--options-file <PATH>` | A JSON object mapping each name to a description or `null`. |

```console
$ jev choice "Which team should handle this?" \
    -O returns="Exchanges, refunds, wrong or damaged items" \
    -O shipping="Delivery status, delays, lost packages" \
    -O billing="Charges, invoices, payment problems" \
    --state "My running shoes arrived in the wrong size."
```

Give the model every option, not a shortlist, and add an `other` option when the list
might not cover an input. `jev` accepts 2–255 options, except Ollama accepts 2–26.
A one-option Choice has only one possible answer. See [provider limits](clef.md#provider-limits).

`--options-file` exists for large option sets:

```json
{ "python": "Python source", "rust": "Rust source", "other": null }
```

---

## `jev score`

One question that rates against ordered levels.

```
jev score <INSTRUCTIONS> (-L TEXT ... | --levels-file PATH) [--id ID] [state source] [--value] [--require EXPR]
```

| Flag | Effect |
| --- | --- |
| `-L, --level <TEXT>` | One level, lowest first. Repeat. |
| `--levels-file <PATH>` | A JSON array of level descriptions, lowest first. |

```console
$ jev score "How severe is the reported issue?" \
    -L "Cosmetic; no impact to functionality" \
    -L "Broken or degraded feature, but workaround exists" \
    -L "Blocking issue; no workaround exists" \
    --state-file bug-report.txt
```

Levels are numbered from 0 in the order given. `jev` accepts 2 to 10 by default,
2 to 26 for Ollama, and 2 to 255 for the explicit Hugging Face provider. See
[provider limits](clef.md#provider-limits). The answer can fall between levels:
it is the probability-weighted mean.

---

## `jev ask`

Several independent questions about one state, in a single request.

```
jev ask [-r PATH | --questions PATH] [state source] [--value] [--require EXPR]
```

| Flag | Effect |
| --- | --- |
| `-r, --request <PATH>` | A full request document. `-` or omitted means stdin. |
| `--questions <PATH>` | A questions map alone; state comes from the state flags. |

With TypeSafe, System One evaluates questions in parallel against one reading of the
state. N separate calls charge for that state N times; one batched call charges once.
Other providers retain their own pricing and runtime behavior. TypeSafe measures the difference
on a long document in its
[parallel questions cookbook](https://docs.typesafe.ai/cookbooks/parallel_questions);
read the figures there, since two official pages quote different numbers for the same
run. Include speculative questions and let your code read only the
answers the branch it took needs.

For the default TypeSafe provider, the request document uses the **official API request body**:

```json
{
  "state": "Shoes arrived two weeks late and in the wrong size. Also two charges.",
  "model": "jev-latest",
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

An example copied from <https://docs.typesafe.ai/api> runs unchanged. `model` is
optional; `--model` overrides it. Every question is validated locally — types,
cardinality, duplicate ids, duplicate option names, unknown fields — before anything is
sent, so a malformed request costs nothing.

```console
$ jev ask -r ticket-questions.json -o json | jq '.answers.department.choice'
```

### The request file is the API's format, and how it is versioned

For the default TypeSafe provider, the same file works with `jev ask`, `jev map`,
`jev --dry-run`, `curl`, and the official SDKs. That is the whole design: several community CLIs invented their own question
vocabulary, and the result is that the official documentation stops applying to their
tool.

**There is deliberately no `version` field**, and there will not be one. The document is
the API's request body; a key the API does not define would make the file invalid as an
API body, which is the one property worth protecting. Adding one and stripping it before
sending would be worse — a private extension to a format this project explicitly does not
own. So the format is versioned the way the API versions it, and the JSON Schema that
describes it is versioned by its own `$id`:

- [`schema/request.schema.json`](../schema/request.schema.json) — JSON Schema
  (2020-12), describing TypeSafe requests. Other providers have additional media
  and option fields; use their [provider guide](clef.md) and `--dry-run` for
  validation. Point an editor at the TypeSafe schema for completion and inline validation:

  ```json
  { "$schema": "https://json-schema.org/draft/2020-12/schema",
    "yaml.schemas": {}, "json.schemas": [
      { "fileMatch": ["*.jev.json", "**/requests/*.json"],
        "url": "./schema/request.schema.json" } ] }
  ```

- `scripts/check-request-schema.py` keeps the schema and the parser in step: every
  committed example must validate, and a list of documents `jev` refuses must not.

There is no YAML. The API speaks JSON, `jev` already has a JSON parser, and a second
format would be a second parser, a second dependency, and a second set of edge cases for
no capability that JSON does not have.

**Order and duplicates.** Questions, Choice options, and Score levels are kept in the
order they were written and sent in that order — the parser is order-preserving for
exactly this reason, because option order is something a model can be sensitive to. A
repeated question id, a repeated field inside a question, or a repeated option name is a
load error, never a silent last-one-wins.

One display caveat: `--dry-run` assembles its document through `serde_json`, whose object
type is sorted, so the body it *prints* has its keys in alphabetical order. The request
itself preserves your order; `a_choice_reaches_the_api_in_the_order_it_was_written` pins
that against a real socket.

### Worked examples

[`examples/`](../examples/) has seven recipes — issue classification, semantic log
filtering, a pull-request risk gate, RAG candidate relevance, dataset triage with an
uncertain-row file, agent action classification, and labelled threshold calibration,
with committed request files in
[`examples/requests/`](../examples/requests/). They are examples, not commands: `jev`
ships primitives so recipes are expressible, and does not ship the recipes.

---

## `jev map`

One question set over many records.

```
jev map -r PATH [-i PATH] [--lines] [--state-field F] [--id-field F]
        [--limit N [--seed S]]
        [--output-file PATH] [--resume] [-j N] [--fail-fast]
        [--require EXPR] [--review-file PATH]
```

| Flag | Default | Effect |
| --- | --- | --- |
| `-r, --request <PATH>` | required | Questions, or a request document whose `state` is ignored without semantic validation; row state remains validated. |
| `-i, --input <PATH>` | stdin | Input records, one per line. |
| `--lines` | off | Treat each line as plain text rather than a JSON value. |
| `--state-field <F>` | whole record | Use this field of each JSON record as the state. |
| `--id-field <F>` | input index | Use this field as the row id. |
| `--images-field <F>` | none | Explicit embedded per-record images for Cloudflare, Ollama, or the Python bridge. |
| `--videos-field <F>` | none | Explicit embedded prepared-frame videos for the Python bridge. |
| `--limit <N>` | every record | Run only N of the input records; at least 1. |
| `--seed <S>` | first N | Choose the `--limit` records by a seeded hash of each id instead. Requires `--limit`. |
| `--output-file <PATH>` | stdout | Write rows here; the summary still goes to stdout. Refused if it already has rows, unless `--resume`. |
| `--resume` | off | Skip records already in the output file, and append. Requires `--output-file`. |
| `-j, --concurrency <N>` | `4` | Requests in flight. Maximum 64. |
| `--fail-fast` | off | Stop at the first failing record. |
| `--require <EXPR>` | off | Classify each answered row. Routes; never changes the exit code. |
| `--review-file <PATH>` | main stream | Divert rows that did not pass `--require` here. Requires `--require`. |

```console
$ jev map -r classify.json -i tickets.jsonl --id-field ticket --state-field body \
    --output-file out.jsonl -j 8
$ jq -r 'select(.ok) | [.id, .answers.department.choice] | @tsv' out.jsonl
```

Properties worth relying on:

- **Input order is output order**, whatever order the requests finished in.
- **Rows fail independently.** A run with failures exits `5` and every successful row is
  still written.
- **The summary totals the tokens the API reported.** Its `usage` adds up the counts on
  the records this run answered, so there is no need to sum the rows with `jq`. It is
  not a bill: a failed record reports no usage, even one whose response arrived and
  could not be decoded, and neither does a failed earlier attempt of a retried record.
  Records `--resume` skipped are not in it, because this run did not evaluate them, and
  `rows_without_usage` counts answered records whose response carried no usage, so a
  total that is only a lower bound says so. The same totals are one line on stderr. See
  [`output-schema.md`](output-schema.md#jevmaprowv1-and-jevmapsummaryv1).
- **Nothing is cached.** `--resume` reads the output file to see which indexes are
  already done; it never replays a stored model judgment, so a resumed run is as fresh
  as a new one.
- **Ctrl-C stops at a record boundary**, flushes, and exits `130`, leaving a file
  `--resume` can pick up correctly.
- **A row file this run would append to must be empty, unless you asked to resume.**
  Both `--output-file` and `--review-file` open in append mode, which is what makes
  `--resume` work — but applied unconditionally it meant running the same command twice
  silently left every record in the file twice, and a later `--resume` read a file whose
  rows came from two different runs. A file that already has rows is now a usage error
  naming all three remedies: remove it, pass `--resume`, or name a different file.
- **Both row files are in completion order, not input order.** Rows are flushed as they
  finish, so a killed process keeps them; every row carries its `index`, so `sort -t: -k`
  or `jq -s 'sort_by(.index)'` recovers input order. Only stdout is in input order.
- **Without `--output-file`, nothing reaches stdout until the batch ends.** Rows are
  buffered so they can be printed in input order, so `jev map … | head -1` still sends
  and bills every record, and `producer | jev map | consumer` gives the consumer nothing
  until the run finishes. For a large batch use `--output-file` and tail it, or read the
  progress count on stderr.
- **`--resume` appends; it never rewrites.** A row that is only half-written — the shape
  a killed process leaves — is not counted as done, so that record is evaluated again
  and a fresh, complete row is appended. The truncated line itself **stays in the
  file**: rewriting the file to remove it would risk losing rows that were paid for, to
  tidy up one line. So a consumer reading a resumed output file should skip lines that
  do not parse, exactly as `--resume` does:

  ```sh
  # A resumed file can contain one truncated line per interrupted run.
  jq -c 'select(.ok == true)' out.jsonl 2>/dev/null
  ```
- **A failed write ends the run** with exit `74` and no summary, rather than reporting a
  partial batch in which nothing failed.

### Trying a question set on a few records first

`--limit N` runs only N of the input records, so a question set can be tried before the
whole batch is evaluated. On its own it takes the first N, which is what `head` would give
you — and the first N lines of a file sorted by date or by source are a biased sample of
it. `--seed S` takes N chosen by a hash of each record's id (the `--id-field` value, or
the input index without one) instead: spread across the file, and the same N every time
for the same input and seed.

```console
$ jev map -r classify.json -i tickets.jsonl --id-field ticket \
    --limit 50 --seed 1 --output-file out.jsonl
--limit: evaluating 50 of 12000 input record(s), chosen by --seed 1
```

- **Records keep their input `index`**, and are sent in input order; stdout is in input
  order and row files in completion order, as for any run. A limit at or above the
  number of records changes nothing. `--dry-run` previews the selection.
- **The summary describes the selection.** `total` is the size of the selection — N, or
  fewer when the input holds fewer records — so `complete` against `total`
  still answers "is this run done?"; a `limit` object records what was asked for and
  how many records the input held. See [`output-schema.md`](output-schema.md).
- **Selections nest.** With the same seed, the records chosen for `--limit 50` are among
  those chosen for `--limit 500`, and the first 50 are among the first 500. So widening a
  pilot with `--resume` — or dropping `--limit` to run the rest — re-uses every row
  already answered and sends only the new ones.
- **`--resume` with a different selection is safe.** Every row already in the output
  file is still checked against the whole input, so a changed input is refused whether or
  not the changed record is in this run's selection. Rows for records outside the
  selection are left in the file as they are, are not re-sent, and are not counted in the
  summary; a line on stderr says how many there were. The file can therefore hold more
  rows than the summary's `total`.

### Separating the rows worth looking at

`--require` takes the same expression `jev noul --require` gates on, and applies it to
every row the API answered. In `jev map` it **routes rather than gates**: it never
changes the exit code, which keeps reporting whether the API answered. `--review-file`
then sends the rows that did not pass somewhere else, so the main stream is the set you
were willing to act on automatically.

```console
$ jev map -r classify.json -i tickets.jsonl \
    --require 'department.confidence >= 0.85' \
    --output-file auto.jsonl --review-file review.jsonl
$ wc -l auto.jsonl review.jsonl
```

Every row carries a `gate` object saying which expression judged it and how it came out:

| `gate.outcome` | Meaning | Goes to |
| --- | --- | --- |
| `passed` | The expression held. | the main stream |
| `failed` | The expression was evaluated and did not hold. | `--review-file` |
| `unevaluable` | The expression named a field this answer does not have. | `--review-file` |
| `not-evaluated` | The row failed before there was an answer. | the main stream |

Four properties worth relying on:

- **Nothing is discarded.** Without `--review-file` every row still reaches the main
  stream, annotated. A row you paid for and cannot see would be worse than no feature.
- **`unevaluable` is not `failed`.** A gate that could not be evaluated never reads as
  one that passed — the same rule that keeps exit `6` distinct from exit `1`. The most
  common cause is asking a Noul for a `confidence` it does not have; see below.
- **A failed row is never diverted.** "The API did not answer" and "the API answered and
  the answer needs a look" are different problems for different people, and the review
  file is worth much less if it is also the error log.
- **`--resume` reads both files.** A diverted row is as done as one that passed, so a
  resumed run does not re-send — or re-bill — the records already set aside.

### Thresholds, and what `confidence` does and does not mean

**A threshold is a claim about your data, and `jev` cannot make it for you.** There is
no default threshold anywhere in this CLI, and there will not be one. TypeSafe's own
documentation says it plainly: *"The correct threshold values depend on your domain and
the performance of the model for your use case. Start with conservative thresholds, test
with your own data, and adjust as you observe results."*
(<https://docs.typesafe.ai/confidence>). Take any number in these examples as a
placeholder for one you measured on labelled examples of your own.

The three primitives do not have interchangeable uncertainty, and writing an expression
as though they did is the easiest mistake to make here:

- **Choice and Score carry a `confidence`.** It is a statistic computed from the
  `probabilities` the answer already gives you: concentrated on one outcome is near `1`,
  spread out is lower. It describes *the shape of the model's answer, not the
  probability that the answer is correct*, and it is not established to be comparable
  across different questions or models. Write `id.confidence >= 0.8`.
- **A Noul carries none, and `jev` does not invent one.** The API returns no confidence
  for a Noul and `<id>.confidence` on one is `unevaluable`, deliberately. A Noul's single
  probability already describes its whole two-outcome distribution. Express uncertainty
  as a band instead: `not (urgent.noul > 0.3 and urgent.noul < 0.7)` keeps the confident
  answers at both ends and sends the middle for review.
- **A mid-range Noul is not "medium".** `0.5` means the model gives yes and no similar
  probability — not that the thing you asked about is half true. If your question is
  really about degree, it wants a Score.

Several community CLIs synthesize a Noul confidence to make the three look uniform. That
gives a script a number with no defined meaning, and `jev` will not do it.

### What `jev map` sends

Every selected record becomes the `state` of one request to the selected endpoint.
Hosted providers and explicitly configured remote servers receive that content
off-machine. A loopback recipient can also forward or offload it. `--state-field`
sends one field instead of the whole record. `--images-field` and `--videos-field`
add only the explicitly selected embedded media, including video metadata. Template
media is sent with every selected row. The CLI never reads image paths found in a
record, follows media URLs, or gathers context from a directory.

`--dry-run` prints the request that would be sent and sends nothing, but it samples only
the first few records (of the `--limit` selection, when there is one); it tells you the
shape, not the whole payload. The row files hold
the model's answers about your state and are created `0600` on Unix for that reason. The
`state_digest` in each row is a 16-character FNV-1a change detector and carries no state
content.

### Choosing a concurrency

`jev` does no proactive rate limiting. It reacts: a `429` is retried with jittered
exponential backoff, honouring `Retry-After` and `retry-after-ms`, which is what
<https://docs.typesafe.ai/api> prescribes. There is no client-side token bucket, because
the API exposes no header from which a client could learn its own remaining budget —
there is nothing to read and therefore nothing to pace against.

That leaves the arithmetic to you, so here it is. TypeSafe publishes per-model limits at
<https://docs.typesafe.ai/models> — at the time of writing 1,200 requests per minute and
250,000 tokens per second — **and publishes them with the warning that they adjust
dynamically and can change without notice**, and that custom and enterprise plans differ.
Do not treat the numbers here as current; read that page.

At `-j 4`, the default, you would need sub-200 ms round trips to approach 1,200 requests
per minute. At the `-j 64` maximum you can exceed it comfortably. If you raise `-j` and
start seeing retries in the `attempts` field of your rows, that is the signal to lower it
again: a batch that spends its time backing off finishes no sooner than one that was
paced correctly, and it costs the same tokens.

Remember also that `jev ask` sends several questions in **one** request. Asking ten
questions per record at `-j 4` is four requests in flight, not forty.

`jev map` is deliberately not a data-processing framework. The state varies; the
questions do not. Chunking, record splitting, and language-aware parsing belong in a
purpose-built tool.

---

## `jev eval`

**A threshold you measured beats a threshold you guessed.** Everywhere else in this
documentation, a number in a `--require` expression is a placeholder. `jev eval` is how
you replace it: give it examples you have already judged, and it reports how the
question performs on *your* data and what cut is defensible for it.

It is not a benchmark of Jev, and it says nothing about the model in general. It
measures **one question, one dataset, and one model version**, and the report records
all three so the result can never be quoted without them. Nothing is trained, and a
label is never sent — ground truth is compared locally, after the answer comes back,
the same way `--require` is evaluated locally.

```console
$ jev eval -r triage.json -d labelled.jsonl --objective min-precision --target 0.95
evaluated 72 labelled row(s) against jev-1.13.0
threshold chosen on 168 calibration row(s), reported on 72 held-out row(s)

urgent (noul, n=72)
  accuracy               0.861 (95% 0.760-0.925)
  precision              0.952
  recall                 0.714
  f1                     0.816
  brier score            0.098
  calibration error      0.041
  threshold 0.780 on urgent.noul, under --objective min-precision
    gate with: --require 'urgent.noul >= 0.780'

This measures one question, one dataset, and one model version. It does not
transfer to another of any of the three.
```

### The dataset

JSONL, one labelled example per line. The questions stay in the same `-r` request file
`jev ask` and `jev map` already take, so a committed question set can be run, batched,
and evaluated without being written three times.

```json
{"schema":"jev.eval.row/v1","id":"T-1","state":"payouts have failed for three days","labels":{"urgent":true,"team":"billing"}}
{"schema":"jev.eval.row/v1","id":"T-2","state":"typo in the footer","labels":{"urgent":false,"team":"other"}}
```

Only `state` and explicitly supplied `images`/`videos` from a row are sent. IDs and
labels stay local; the template supplies questions, model, and options.
Media uses the selected provider's bounds and cannot name files or URLs. A row may label a subset of the questions,
and each question is scored over whichever rows label it. The full rules, and the label
shape for each question type, are in
[`output-schema.md`](output-schema.md#jevevalrowv1--the-labelled-dataset-jev-eval-reads).

Validation is total and happens before a single request: an unknown question id, a
Choice label that is not one of the declared options, a Score label outside the legend,
or a duplicate row id is refused rather than ignored. A label that is quietly dropped
looks scored and was not, which is the worst possible failure for a tool whose output is
a number somebody will trust.

### With no objective: how good is this question?

```console
$ jev eval -r triage.json -d labelled.jsonl --output json | jq '.questions.team'
```

No threshold is chosen, so no rows are held back and every example is evidence. You get
accuracy, macro precision/recall/F1, per-class scores, a confusion matrix, a Brier
score, and a reliability diagram. For a Noul you get the Brier score, log loss, and the
calibration error — but **not** accuracy, because accuracy is a property of a decision
and a Noul is a probability until a cut turns it into one. Ask for a cut with
`--objective`.

### With an objective: where should the cut go?

| `--objective` | Selects | For | `--target` |
| --- | --- | --- | --- |
| `maximize-f1` | The cut with the highest F1. | noul | none |
| `min-precision` | The highest recall among cuts whose precision is at least the target. | noul | required |
| `min-recall` | The highest precision among cuts whose recall is at least the target. | noul | required |
| `min-accuracy` | The widest coverage among cuts whose automatically handled rows are at least that accurate. | choice, score | required |
| `target-coverage` | The cut whose coverage is closest to the target. | choice, score | required |

The split is not cosmetic. A noul is swept on its **probability**, which is a decision
cut: every row lands on one side, so there is no abstention to trade coverage against. A
choice and a score are swept on their **confidence**, which is exactly an abstention
axis: above the cut you act, below it you escalate. A noul has no confidence — the API
returns none and `jev` does not invent one — so asking for a coverage objective on one
is a usage error, before anything is sent, rather than a synthesized number.

Every threshold is reported together with the objective it was chosen under, and the
word "optimal" appears nowhere: the best cut under one objective is a bad cut under
another. When nothing in the data reaches the target, `threshold` is `null`,
`threshold_reachable` is `false`, and the run exits `1` — never a relaxed number that
does not do what the flag asked for.

### Choosing on one set of rows and reporting on another

Picking a threshold on the same examples you then quote its performance from is the
classic way to publish a number that does not survive contact with new data. So with
`--objective`, `jev eval` holds rows back by default:

- The cut is chosen on the **calibration** rows.
- The numbers printed beside it come from the **reported** rows, which the choice never
  saw.
- `--seed` and `--test-fraction` control the split. It is keyed on each row's `id`, not
  its position, so re-sorting the file or appending twenty more examples leaves the
  original rows on the sides they were already on — which is what makes two reports
  comparable.
- `--calibration PATH --test PATH` supplies the two sides yourself, for a team that
  keeps a versioned golden set. A row id appearing in both is refused.
- `--no-split` chooses and reports on everything. It is allowed, because with forty
  examples you may have no better option, but the result is optimistic by an unknown
  amount and both stderr and the report's `warnings` say so.

Every headline number carries a 95% Wilson interval and the `n` it came from. "87%" from
forty rows and "87%" from four thousand are different claims, and a report that printed
them identically would invite acting on the wrong one.

### Cost, and stopping before you spend

`jev eval` sends one request per row, with the same bounded concurrency, retry policy,
and interrupt handling as `jev map`. Before it sends anything it says how much:

```console
$ jev eval -r triage.json -d labelled.jsonl
sending 240 row(s), 3 question(s) each, concurrency 4
```

- `--dry-run` prints the exact bytes that would be sent for the first few rows, plus the
  split, and sends nothing.
- `--limit N` caps the run at the first N rows. The dataset fingerprint is recomputed
  over the rows actually used, so the report never describes more than it measured.
- `-j`/`--concurrency` and `--retries` mean what they mean everywhere else.
- `--show-rows` adds each reported row's label and answer to the JSON report, for
  looking at what the model got wrong. Only the reported rows: the calibration rows are
  selection-time working, and listing them beside the estimate would invite reading the
  two as one set.

Rows fail independently. A run with some failures exits `5`, the metrics cover only the
rows that answered, and `rows.errors` in the report groups the failures by kind with one
example message each.

### Keeping a result to compare against later

```sh
jev eval -r triage.json -d labelled.jsonl \
         --objective min-precision --target 0.95 \
         --model jev-1.13.0 --report baseline.json
```

`--report` writes the `jev.eval/v1` document to a file (mode `0600` on Unix). It records
the model version that actually answered, the dataset fingerprint, the question
fingerprint, the objective, the threshold, and the metrics — so you can compare the same dataset and questions against another model release.
Clef fingerprints also include provider/account, media, and inference options.
They do not establish the identity of weights or a runtime behind a model name.

**Pin the model once a threshold is in production.** `jev-latest` is a moving alias, and
a threshold calibrated against what it meant last month is not a threshold against what
it means today. See [model pinning](#model-pinning).

### What `jev eval` deliberately does not do

- **It does not train anything.** Every judgment comes from the API; the rest is
  arithmetic.
- **It does not search for an abstention band.** A two-sided band on a noul —
  auto-approve above, auto-reject below, escalate the middle — is a two-dimensional
  search with compound tie-breaking, and shipping a half-considered optimizer for it
  would be worse than not shipping one. What it gives you instead is the whole
  `threshold_sweep`, including `negative_predictive_value`, so you can read a band
  straight off two rows of the table and write it as
  `not (x.noul > 0.3 and x.noul < 0.8)`.
- **It does not grade.** There is no letter, star, or composite score. Collapsing these
  numbers into one would be a judgment about what being wrong costs you, which is the
  part that is yours.
- **It does not claim the result transfers.** A different dataset, a different question,
  or a different model version is a different measurement.

---

## `jev models`

```console
$ jev models
jev-latest   2026-09-10T18:38:01.391457+00:00  The latest iteration of TypeSafe's System One Model: Jev
jev-preview  2026-09-10T18:39:06.057655+00:00  A preview version of `jev-latest`: should be better in most ways
```

That is real output, recorded on 2026-09-20. Two things about it are worth knowing:

- **`release_date` is a timestamp, not a date.** The API returns full RFC-3339 with
  microseconds. `jev` prints it verbatim rather than reformatting it, so `--output json`
  gives you exactly what the API said.
- **The list is aliases only.** `jev-1.13.0` is a perfectly valid `model` value and does
  not appear here. This endpoint is not an enumeration of what you may send, which is
  why `jev` never validates `--model` against it.

Columns are padded to the longest name.

For TypeSafe, the list comes from `GET /v1/models` every time and is not hard-coded.
Ollama uses `/api/tags`; llama.cpp and the Python bridge use `/v1/models`. Cloudflare
uses its account model-search endpoint and normalizes a successful result to the two
supported Clef identifiers. That list is neither an entitlement check nor a complete
Workers AI catalogue. Listing models does not perform inference. See [Clef](clef.md).

---

## `jev doctor`

```
jev doctor [--live]
```

Reports the endpoint, the model, the configuration file, which credential sources are
populated, and the limits in force — and where each setting came from.

**It makes no network request unless you pass `--live`**, and says which mode it ran in.
`--live` makes one logical model-listing call using the selected provider's route,
with the configured bounded retry policy. It does not perform model inference.
An incomplete saved Cloudflare provider instead reports `configuration_error` and
`live.checked: false` without looking up a credential or contacting a server.

It never prints a credential. It reports *which source* one would come from.

**`doctor` exits `0` even when the `--live` check fails.** It is a report, not a gate:
read `.live.ok` from `-o json`, or use `jev models`, which exits non-zero when the API
cannot be reached or refuses the key.

```console
$ jev doctor
$ jev doctor --live -o json | jq '.credentials.effective_source, .live.ok'
```

---

## `jev auth`

```
jev auth login [--stdin]
jev auth status
jev auth logout
```

`login` is only for the official TypeSafe endpoint. Cloudflare and remote custom
endpoints use `JEV_CUSTOM_API_KEY` or `JEV_CUSTOM_API_KEY_FILE`; loopback local
providers need no key. `login` prompts without echo and stores the key in the operating system credential
store: the macOS Keychain, the Windows Credential Manager, or the Secret Service on
Linux. `--stdin` reads it from a pipe instead, for scripted provisioning.

**If no secure store is available, `login` fails and tells you to use `JEV_API_KEY`.**
It never writes a key to a plaintext file. That is
[ADR-0002](adr/0002-security-and-credentials.md), and it is the one thing no other CLI
in this space does.

`login` warns if a higher-precedence environment variable is set, because the stored key
would otherwise appear to do nothing.

`status` reports where a credential would come from, without showing it, and exits `3`
when a required credential is unavailable. Loopback local providers report `anonymous`
and exit `0` without consulting credential sources.

`logout` removes only the entry `jev` stored, and says so when there was none. It also
names any environment variable still set, because that is not `jev`'s to remove.

---

## `jev config`

```
jev config list
jev config get <KEY>
jev config set <KEY> <VALUE>
jev config unset <KEY>
jev config path
```

Non-secret settings only: `color`, `endpoint`, `max_input_bytes`, `model`, `output`,
`retries`, `timeout_seconds`, `provider`, `cloudflare_account_id`.

**A credential is not a setting.** `jev config set api_key …` is refused, and so is a
hand-written file containing a key-shaped key at any nesting level. The file is created
`0600` inside a `0700` directory, written atomically.

An unknown setting is an error rather than a silently ignored line: a typo'd `modle`
that is discarded means you get a model you did not choose and never find out.

`jev config get` on a setting that is not set exits `1`, so it works in a shell
conditional.

---

## `jev completions`

```console
$ jev completions bash > /etc/bash_completion.d/jev
$ jev completions zsh  > "${fpath[1]}/_jev"
$ jev completions fish > ~/.config/fish/completions/jev.fish
$ jev completions powershell | Out-String | Invoke-Expression
$ jev completions elvish > ~/.config/elvish/lib/jev.elv
```

Five shells: `bash`, `zsh`, `fish`, `powershell`, and `elvish`.

Generated from the same command definition the parser uses, so a new flag is completable
the moment it exists. Nothing is installed for you: where completions belong differs by
shell and distribution, and writing into your shell configuration uninvited is not
something a CLI should do.

---

## `jev mcp serve`

Runs a Model Context Protocol server on stdin and stdout until the host closes the
connection. It exposes the tools `noul`, `choice`, `score`, `ask`, and `map`, and no
others. It is meant to be started by an agent host; host setup, the tool schemas, and
the `map` limits are in [`mcp.md`](mcp.md).

```sh
jev mcp serve
jev --model jev-1.13.0 --timeout 30 mcp serve     # global flags set the defaults
```

- **Streams.** stdout carries only protocol messages. stderr is silent unless
  `--verbose` is given, or a non-official `--endpoint` is in force, which is warned
  about once at startup.
- **Refused flags.** `--dry-run` is refused. `--output` and `--color` have no effect.
- **Exit status.** `0` when the host closes stdin, `130` on Ctrl-C, and `74` if the
  connection fails.

## Gating

`--require` turns a model judgment into a process exit status, without a shell pipeline
that silently passes when `jev` fails.

```console
$ jev noul "Is this a security issue?" --state-file report.md --require 'answer.noul > 0.9'
$ jev ask -r triage.json --require 'severity.score >= 2 and team.choice == security'
```

Grammar, deliberately tiny:

```
expr       := or
or         := and ('or' and)*
and        := unary ('and' unary)*
unary      := 'not' unary | '(' expr ')' | comparison
comparison := path operator literal
path       := ident ('.' ident)*
operator   := '>' | '>=' | '<' | '<=' | '==' | '!='
literal    := number | 'single quoted' | "double quoted" | bare-word
```

Addressable paths:

| Path | Reads |
| --- | --- |
| `<id>.noul` | a Noul's yes-probability |
| `<id>.choice` | a Choice's selected option (text; use `==` or `!=`) |
| `<id>.score` | a Score's value |
| `<id>.confidence` | a Choice or Score confidence — **not** available on a Noul |
| `<id>.probabilities.<option>` | one Choice option's probability |
| `<id>.probabilities.<level>` | one Score level's probability |

Properties:

- **A path that does not resolve exits `6`, never `0`.** A typo is not a negative
  judgment.
- **Both sides of `and`/`or` are always evaluated.** A broken right-hand side is
  reported even when the left decides the outcome, so a typo cannot hide until the day
  the other side flips.
- **The answer is still printed.** The data was produced; only the exit status changes.
- **A malformed expression is caught before the request is sent**, so it costs nothing.
- **Nothing is ever handed to a shell.** There is no `eval` in this project.
- **`==` and `!=` on a number are exact**, like `>=` and `<=`. There is no tolerance,
  because a tolerance has to be a fixed size and any fixed size means something different
  at `0.001` than at `3` — `jev` shipped one and it made `x == 0` and `x > 0` both true.
  So `severity.score == 1.7` is false for a score of `1.7000000000000002`. **Prefer a
  range for anything the model computed**: `severity.score >= 1.5 and severity.score < 2.5`
  says what you meant and is not at the mercy of the last bit. Exact equality is right for
  values you chose — an integer Score level, or a Choice option name, which is text.
- **A question id that starts with a digit cannot be addressed.** Ids are otherwise
  unrestricted, but a leading digit reads as a number in an expression. Rename the id.

### Thresholds, again

The numbers in a gate are the part that has to come from your data. See
[thresholds and what `confidence` does and does not mean](#thresholds-and-what-confidence-does-and-does-not-mean)
— in particular that a Noul has no `confidence`, so `<id>.confidence` on one exits `6`,
and that a Noul's uncertainty is a band around the middle of its probability rather than a
low number.

---

## Custom endpoints

`--endpoint` is a security boundary, not a configuration string.

- The default is `https://api.typesafe.ai`, and nothing else is reachable by accident.
- Cloudflare and remote custom endpoints use `JEV_CUSTOM_API_KEY` / `JEV_CUSTOM_API_KEY_FILE` and
  **only** those. `JEV_API_KEY`, `TYPESAFE_API_KEY`, and the OS credential store are not
  consulted, so a production key cannot reach another host.
- Loopback Ollama, llama.cpp, and Hugging Face providers never resolve or send a
  credential. The default TypeSafe protocol against a loopback mock still uses the
  custom namespace.
- `jev auth login` refuses to run against a non-official endpoint.
- Plain HTTP is refused unless the host is unambiguously loopback.
  `localhost.evil.example` is not loopback.
- A non-official endpoint warns on stderr on **every** invocation, and `--quiet` does
  not suppress that warning.
- Redirects are never followed.

See [ADR-0008](adr/0008-credential-precedence-and-endpoint-isolation.md).

### Trying `jev` against a local mock

Because loopback is the one place plain HTTP is allowed, you can exercise every
code path — flags, output shapes, exit codes, `--require` — without a key and
without spending a token. Save this as `mock.py`:

```python
import http.server, json

BODY = json.dumps({
    "model": "jev-1.13.0",
    "answers": {"answer": {"type": "noul", "noul": 0.92}},
    "usage": {"input_tokens": 18, "output_tokens": 4},
}).encode()

class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers.get("Content-Length", 0)))
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(BODY)))
        self.end_headers()
        self.wfile.write(BODY)
    def log_message(self, *args):
        pass

http.server.HTTPServer(("127.0.0.1", 8799), Handler).serve_forever()
```

```console
$ python3 mock.py &
$ export JEV_CUSTOM_API_KEY=not-a-real-key
$ echo "payouts have been failing for 3 days" \
    | jev noul "Does this convey urgency?" --endpoint http://127.0.0.1:8799
answer
  yes  0.9200  ██████████████████████··
18 tokens in, 4 out, answered by jev-1.13.0
```

`JEV_CUSTOM_API_KEY` is required rather than optional: a non-official endpoint never
reads your real credential, so there is no way to leak one into the mock. The
non-official-endpoint warning appears on every call, by design.

`jev … --dry-run` needs no server at all — it prints the request body, built by the
same code that builds a real request, and sends
nothing.

---

## Using `jev` in CI

Two things make this work: the exit-code contract, and `--require` moving a model
judgment into that exit status. The credential comes from the environment; there is no
`--api-key` flag, because an argument is visible in `ps` and in CI logs.

```yaml
# .github/workflows/triage.yml
- name: Check the pull request description for a breaking change
  env:
    JEV_API_KEY: ${{ secrets.TYPESAFE_API_KEY }}
    BODY: ${{ github.event.pull_request.body }}
  run: |
    # `--require` exits 1 when the gate does not hold, and 6 when it could not be
    # evaluated. Treating 6 as a pass is the mistake this separation exists to prevent.
    set +e
    printf '%s' "$BODY" | jev noul "Does this describe a breaking change?" \
      --require 'answer.noul > 0.8' --output json > result.json
    status=$?
    set -e
    cat result.json
    case "$status" in
      0) echo "breaking change: add the migration note" >&2; exit 1 ;;
      1) echo "no breaking change" ;;
      *) echo "::error::jev exited $status" >&2; exit "$status" ;;
    esac
```

For a batch, `jev map --output-file` plus `--resume` makes a rerun cheap after a
timeout: rows already written are not re-evaluated, and complete rows survive an interruption. A truncated final row may remain; see
[resume behavior](#jev-map).

---

## Model pinning

`jev-latest` and `jev-preview` are **moving aliases**. When TypeSafe ships a new release,
the answers behind them change with no change on your side.

If you have calibrated a threshold — a `--require` bound, a routing cut-off, a review
queue trigger — pin the version you calibrated against:

```console
$ jev config set model jev-1.13.0
```

`jev` always reports the concrete model that answered, in `model`, separately from what
you asked for in `model_requested`. That is the only way to reason later about a
threshold. `jev doctor` flags a moving alias, and `--verbose` notes when one resolved to
something else.


For Clef, pin an installed Ollama model tag/digest or a reviewed local publisher/GGUF
revision and runtime separately. `clef` and `clef-flash` are model names, not weight
hashes. A returned model name, `jev doctor`, and resume digests cannot detect changed
weights behind the same name. Record the selected provider and runtime settings,
then recalibrate when the model or processor changes. Cloudflare does not expose a
weight hash through this CLI. The opt-in [live-test provenance](development/clef-live-testing.md#execution-provenance)
records explicitly named disk artifacts with limits on what those hashes establish.
