# API compatibility

What `jev` assumes about the TypeSafe System One API, where each assumption comes from,
and how it is checked.

This document's existing tables describe TypeSafe. Hosted Cloudflare and local
Ollama/llama.cpp adapters follow their respective official contracts, researched on
2026-10-03 and rechecked for this documentation audit on 2026-10-05. The Python
bridge has a separately documented project-owned contract. See [provider compatibility and limitations](clef.md) and
[source evidence](development/clef-research.md). A System One-compatible question
shape does not imply compatible routing, authentication, image encoding, or limits.

## The rule

**For TypeSafe behavior, official TypeSafe sources are authoritative.** In order:

1. <https://docs.typesafe.ai> — start from `llms.txt`; Mintlify serves Markdown by
   appending `.md` to a page path.
2. The vendored official agent skill in `.claude/skills/typesafe-ai/`.
3. The official SDK repositories and their typed definitions.

Clef provider authority comes from [Cloudflare's hosted schemas](https://developers.cloudflare.com/workers-ai/models/clef/),
[Ollama System One](https://docs.ollama.com/api/systemone),
[llama.cpp's server contract](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md),
and the [publisher implementation](https://huggingface.co/Cloudflare/clef/blob/main/joint_schema_model.py)
for the local bridge. [Clef](clef.md) records the separate routes, content forms,
media encoding, options, and bounds. A community project, including one in
`references/`, is landscape evidence, not provider protocol authority.

## Verification status

**The documentation moves.** During the single session in which this was implemented,
`/api`, `/confidence`, `/primitives/{noul,choice,score}` and `/concepts/state` all
changed underneath us — `/api` grew by about 1.4 KB, and every example response was
re-run against `jev-1.13.0`. The snapshot in `references/05-typesafe-docs/pages/` is
therefore a *dated* record, not a current one, and a claim that it matches the live
pages is true only on the day it is made.

Last re-fetched and reconciled: **2026-09-20**. The 14 pages below were compared against
the live site on that date and the fixtures were re-transcribed from what they say now.

A future reviewer should re-fetch before trusting any specific quote here:

```sh
curl -sS https://docs.typesafe.ai/api.md | diff - references/05-typesafe-docs/pages/api.md
```

| Page | Used for |
| --- | --- |
| `/api` | Endpoint, request body, question types, answer types, error statuses |
| `/models` | Default model, aliases, the `GET /v1/models` shape, limits |
| `/primitives/noul` | Noul semantics, the absence of confidence |
| `/primitives/choice` | Choice semantics, the 255-option limit |
| `/primitives/score` | Score semantics, the level range, the weighted-mean definition |
| `/primitives/advanced` | Structured instructions and criteria |
| `/confidence` | What confidence means and does not mean |
| `/concepts/state` | Accepted state shapes |
| `/model-jaggedness/jev-1.13` | Known model limitations |
| `/sdk/python/api/{constants,retries,exceptions}` | Environment names, retry defaults, status mapping |
| `/sdk/python/api/types/{questions,responses}` | Optionality of fields |

Plus `typesafe-ai/typesafe-sdk-python` at `2ce5c65`, whose `_schemas/models.py` is
generated from `https://api.typesafe.ai/openapi.json`.

## What `jev` sends

`POST {base}/v1/systemone`, `Content-Type: application/json`,
`Authorization: Bearer <key>`, `Accept: application/json`,
`User-Agent: jev-cli/<version>`.

```json
{ "state": …, "model": "…", "questions": { "<id>": { … } } }
```

| Field | Shape | Source |
| --- | --- | --- |
| `state` | string, object, or array | `/api` request body |
| `model` | string | `/api`; default `jev-latest` from `/models` |
| `questions` | non-empty map of id to question | `/api`; `min_length: 1` in the OpenAPI schema |

Question shapes:

| Type | Required | Optional |
| --- | --- | --- |
| `noul` | `type`, `instructions` | `criteria: {true?, false?}` |
| `choice` | `type`, `instructions`, `criteria: {name: desc\|null}` | — |
| `score` | `type`, `instructions`, `criteria: [desc, …]` | — |

An absent optional field is **omitted**, not sent as `null`, matching the official
Python SDK's `None`-omitting serializer. A Choice option with no description *is* sent
as `null`, because the documentation defines that as "interpreted by its name alone" and
omitting the key would change the option set.

## What `jev` accepts

```json
{ "model": "…", "answers": { "<id>": { … } }, "usage": { … } }
```

| Answer | Fields |
| --- | --- |
| `noul` | `type`, `noul` |
| `choice` | `type`, `choice`, `confidence`, `probabilities` |
| `score` | `type`, `score`, `confidence`, `legend`, `probabilities` |

Beyond the documented shape, `jev` enforces internal consistency, because a response
that contradicts itself must not be reported as an answer:

- every probability and confidence is finite and within `[0, 1]`;
- a Choice's `choice` appears in its own `probabilities`;
- a Score's `score` lies within the range its own `legend` spans;
- nesting depth and body size are bounded.

## Locally enforced limits

`jev` rejects these before sending, so a mistake costs no tokens. Each is transcribed
from the documentation, except where marked.

| Limit | Value | Source |
| --- | --- | --- |
| Score levels, maximum | 10 | `/api`: "the API accepts up to 10"; `/primitives/score` agrees |
| Score levels, minimum | 2 | **Client-side.** The docs say a Score *should* have at least two, and the generated OpenAPI schema sets `min_length: 1`. A one-level Score has one possible answer and a `score` that is always 0, so `jev` rejects it locally rather than charging for it. |
| Choice options, maximum | 255 | `/api`: "You can have a maximum of 255 options per Choice" |
| Choice options, minimum | 2 | **Client-side.** Undocumented, and a one-option Choice has one possible answer. |
| Questions per request | ≥ 1 | `/api`; OpenAPI `min_length: 1` |
| `instructions` | required | `/api` marks it required; the SDK types make it optional. Where they differ, the HTTP API reference wins. |
| `instructions` | non-empty | **Client-side.** Required is not the same as non-empty, and neither source sets a minimum length. A blank instruction is a mistake worth catching before it is billed. |

Not enforced locally, because they cannot be measured without a tokenizer: the 64k
tokens per request and 32k for `state` plus the longest question documented on
`/models`. The API returns `422` for those, and `jev` reports it with the field path.

## Retries

Defaults match the official Python SDK's `RetryPolicy`, so `jev` behaves like the SDK a
user may already have calibrated against.

| Setting | Default |
| --- | --- |
| Retries after the first attempt | 2 |
| Initial backoff | 0.5 s, doubling |
| Maximum backoff | 5 s |
| Jitter | 25 %, subtractive |
| Total budget | 30 s, raised if `--retries` and `--timeout` need more |
| Per-attempt timeout | 10 s |
| Retried statuses | 408, 429, 5xx (which includes 529 Overloaded) |
| Never retried | 400, 401, 403, 404, 422 |

`retry-after-ms` is preferred over `retry-after`, matching the SDK. The HTTP-date form
of `Retry-After` is deliberately **not** honoured: parsing it needs a clock, which would
make retry behaviour non-deterministic and untestable, and the API's own SDKs send the
numeric form. Any hint is clamped to 60 seconds; a hostile endpoint must not be able to
park the process.

A **timeout or a dropped connection is retried too**, as the official SDK's
`RetryPolicy` does by default (`api_timeout_error` and `api_connection_error` are both
`True`, per <https://docs.typesafe.ai/sdk/python/api/retries>). The cost is that a
request which timed out may already have been processed, and billed, by the server. On
2026-09-23 a 255-option Choice took `3 attempt(s) in 31432 ms`: two
10-second timeouts, then an answer. The same request with `--timeout 90 --retries 0`
answered in 3.9 s. For an expensive or first-of-the-day call, raise `--timeout` rather
than relying on retries.

## Error statuses

| Status | `jev` exit code | Retried |
| --- | --- | --- |
| 400, 422 | 2 | no |
| 401, 403 | 3 | no |
| 404 | 2 | no |
| 408, 429, 529 | 4 | yes |
| 5xx | 4 | yes |

The message is extracted from `error`, `error.message`, `message`, `detail` as a string
or object, or `FastAPI`'s `detail: [{loc, msg}]` array — the same ladder as the official
SDK's `extract_message`. It is then truncated to 200 characters, which is the SDK's
`MAX_ERROR_BODY_LENGTH` but applied more widely; see "Known deviations".

## Forward compatibility

- An **unknown answer `type`** is surfaced with `"unrecognized": true` and its payload
  intact. The official Python SDK drops it with a log warning; keeping it is strictly
  more useful to a script, and neither fails the response.
- An **unknown field** on a known answer is ignored.
- An **absent `usage`** decodes. This is a deliberate loosening, not SDK parity — see
  "Known deviations" below.
- **No model catalogue is hard-coded.** `GET /v1/models` is the authority, and the API
  accepts versioned identifiers whether or not they appear in the listing.

## How this is checked

`crates/jev-client/tests/compatibility.rs` runs against documents transcribed verbatim
from the official sources, in `crates/jev-client/tests/fixtures/`. It asserts both
directions: a request `jev` builds encodes to exactly the documented body, and a
documented response decodes with nothing lost.

A fixture that stops matching is a compatibility signal, not a test to adjust. Re-fetch
the page, decide whether the API changed or `jev` did, and record the outcome here.

## Live verification

Everything above is checked against **recorded** official documents. That proves `jev`
agrees with what TypeSafe published; it does not prove TypeSafe's service behaves the
way its documentation says.

`crates/jev-cli/tests/live.rs` closes that gap when someone supplies a key. It is
opt-in, skips cleanly and visibly without one, and is deliberately cheap — a handful of
requests with short synthetic state:

```sh
JEV_LIVE_TESTS=1 JEV_API_KEY=… cargo test -p jev-cli --test live -- --ignored --nocapture
```

| Test | What it proves against the real API |
| --- | --- |
| `authentication_works_and_doctor_agrees` | The credential is accepted and `doctor --live` reports it |
| `models_returns_at_least_the_default_alias` | `GET /v1/models` shape, and that `jev-latest` is listed |
| `a_noul_returns_a_probability_and_no_confidence` | Noul shape, and that the API really returns **no** confidence for one |
| `a_choice_returns_a_selection_a_distribution_and_a_confidence` | Every option comes back, the selection is in its own distribution, and the distribution sums to 1 |
| `a_score_returns_a_value_a_legend_and_a_distribution` | The legend round-trips, and `score` really is the probability-weighted mean of the level numbers |
| `a_mixed_request_answers_every_question_in_one_call` | Four questions of three types in one request; a `null` option description is not dropped |
| `json_state_and_text_state_both_work` | Structured state is accepted and understood |
| `a_pinned_model_answers_and_reports_itself` | An alias resolves, the resolved version is reported, and pinning it reaches the same model |
| `a_validation_error_is_reported_with_its_field` | A server-side rejection is classified and reported without panicking |
| `a_gate_against_a_live_answer_exits_as_documented` | Exit `1` on a failed gate, with the answer still on stdout |
| `a_rejected_credential_exits_3_after_one_attempt` | A deliberately invalid credential produces exit `3` after one attempt |

They never print the key, never write it anywhere, and send only synthetic text.
`usage.input_tokens` is asserted present on every response, so the cost of a run is
visible in its output rather than assumed.

**Historical status: run on 2026-09-23 against `jev-1.13.0`, all ten then recorded passing**
(about 1,700 input tokens in total). The current suite contains eleven ignored tests;
the historical count does not establish a live result for every current test. The same
day, a wider manual pass exercised `ask`, `map`, `eval`,
`mcp serve`, credential handling and server-side validation against the live API, about
185 requests in all: `ask`, `map`, `eval`, `doctor --live`, `auth status`, credential
errors, server-side validation, and all five `jev mcp serve` tools. `auth login` against
a real credential store was not exercised. What it found that the documentation does not say is recorded in
the next section; the bugs it found are in `CHANGELOG.md`.

## What only the live API tells you

Facts a reader cannot get from the documentation, recorded here because `jev`'s
behaviour depends on them. The first three were observed on 2026-09-20 and captured in
`crates/jev-client/tests/fixtures/response-models-live.json`; the rest on 2026-09-23.

| Fact | Why it matters |
| --- | --- |
| `release_date` is a full RFC-3339 **timestamp** with microseconds (`2026-09-10T18:38:01.391457+00:00`), not the `YYYY-MM-DD` the Python SDK's `ModelMetadata` example shows | Anything parsing it as a date breaks. `jev` treats it as an opaque string and prints it verbatim, so it does not. |
| `GET /v1/models` returns only the **aliases** — `jev-latest` and `jev-preview`. `jev-1.13.0` is a valid `model` value and is **not** listed | The endpoint is not an enumeration of what may be sent. `jev models` says so, and `jev` never validates a model id against this list. |
| An unknown model is rejected with **HTTP 400** (`Unknown model: …`), not 422 | Both already map to exit `2`, and neither is retried. |
| A state over the token budget is rejected with **HTTP 400** and the body `{"detail":{"error_type":"max_tokens_exceeded"}}` — no `message` | `jev` reads `error_type` so the error names the cause instead of printing raw JSON. Observed at 93 KB and 312 KB of state. |
| A rejected credential is **HTTP 401** `Cannot authenticate with the server. Please check your API key and try again.` | Exit `3`, not retried. |
| `jev-preview` currently resolves to `jev-1.13.0`, the same version as `jev-latest` | An alias is not a promise of a different model. `model` in the output says which one answered. |
| A Choice with 255 options, a Score with two identical level names, and question ids such as `a b`, `ümlaut`, `../etc` or 300 characters are all **accepted** | `jev` enforces the 255-option limit locally and rejects duplicate Choice options; it does not reject duplicate Score levels, and neither does the API. |
| Latency varies from about 0.2 s to 9 s for the same small request, and slow responses were not confined to the first request of a run | The 10 s default timeout is close to that ceiling. See "Retries" above. |
| Probabilities come back rounded to two decimals and sum to 1 within 0.01. `score` is the probability-weighted mean of the levels, computed from the unrounded probabilities: one answer reported `score: 0.09` where the rounded values give `0.08` | Do not expect to reproduce `score` exactly from the displayed distribution. |

## Known deviations

Every place `jev` deliberately differs from the official SDKs, with the reason. In the
style of `typesafe-ai-rs`'s `docs/PARITY.md`: a deviation is acceptable, an *unrecorded*
deviation is not.

| Deviation | The SDK does | `jev` does | Why |
| --- | --- | --- | --- |
| HTTP-date `Retry-After` | Parses it via `parsedate_to_datetime` | Ignores it and falls back to backoff | Parsing it needs a wall clock, which would make retry behaviour non-deterministic and untestable. The API's own SDKs send the numeric form. |
| Retry-hint ceiling | None | Clamps any hint to 60 s | A hostile or misconfigured endpoint can ask a client to sleep for a year. |
| Unknown answer `type` | Dropped, with a log warning | Kept, flagged `"unrecognized": true` | Strictly more useful to a script, and neither fails the response. |
| Error-message truncation | 200 chars, **only** for the raw-body fallback; an extracted message passes through whole | 200 chars, always | A hostile endpoint must not be able to use an error message as an unbounded output channel. |
| `usage` object absent | Fails validation — the object is required in both the schema and the public type, even though both token counts are optional | Decodes, with both counts `None` | Forward compatibility. A response that omits a field `jev` only reports is not worth failing over. **Note:** the token counts being optional is *not* the reason; the object itself is required, and this is a deliberate loosening. |
| 408 Request Timeout | No specific class | Grouped with 429/529 as "throttled" | Retry behaviour is identical and the grouping keeps the retry policy in one place. The message is imprecise for a 408; that is the cost. |
| `TYPESAFE_BASE_URL`, `TYPESAFE_DEFAULT_MODEL`, `TYPESAFE_LOG_LEVEL` | Read | **Not read** | The endpoint is a security boundary (ADR-0008): it must not be settable by an ambient variable a CI template might export. `TYPESAFE_API_KEY` *is* honoured. |
| `null` `instructions` / `null` Noul criteria sides | Accepted | Not expressible | `Content` models `string \| object \| array`. An explicit `null` instruction carries no information, so nothing is lost; recorded because it is a real difference. |
