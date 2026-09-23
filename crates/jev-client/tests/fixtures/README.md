# Compatibility fixtures

Recorded request and response documents used to prove that `jev` still speaks the
protocol the official TypeSafe sources describe.

## Provenance

Every fixture is transcribed verbatim from an official source, and each file records
which one in its `_source` field. No fixture is invented, and none comes from a
community project (`AGENTS.md` §6, §8).

The live pages changed during the session in which these were written — see
`docs/api-compatibility.md`, "Verification status". Every fixture below was
**re-transcribed from the live page on 2026-09-19**, which is what the tests now assert
against; the local snapshot under `references/05-typesafe-docs/pages/` is the older
record and no longer matches for the pages marked so.

| Source | Transcribed from live | Local snapshot |
| --- | --- | --- |
| <https://docs.typesafe.ai/api> | 2026-09-19 | **differs** — examples re-run against `jev-1.13.0` |
| <https://docs.typesafe.ai/primitives/choice> | 2026-09-19 | **differs** |
| <https://docs.typesafe.ai/primitives/score> | 2026-09-19 | **differs** |
| <https://docs.typesafe.ai/primitives/noul> | 2026-09-19 | **differs** |
| <https://docs.typesafe.ai/models> | 2026-09-19 | identical |
| `typesafe-ai/typesafe-sdk-python` `2ce5c65` | 2026-09-19 | `_schemas/models.py`, generated from `https://api.typesafe.ai/openapi.json` |
| **live** `GET https://api.typesafe.ai/v1/models` | 2026-09-20 | recorded as `response-models-live.json` |

`response-models-live.json` is the one fixture recorded from the API rather than
transcribed from a document, because it is the only source that gives `release_date` a
concrete value — and it gives a full RFC-3339 timestamp, not the plain date the SDK
example shows. See `docs/api-compatibility.md`, "What only the live API tells you".

`response-models.json` carries only the rows its two sources actually document. It
deliberately does **not** list every model the live `/models` endpoint may return: a
fixture is a transcription, and a row nobody documents would be an invention. The live
shape is kept separately, in `response-models-live.json`.

## What the tests assert

`crates/jev-client/tests/compatibility.rs` checks both directions:

* a request `jev` builds encodes to exactly the documented body, field for field;
* a documented response body decodes into the domain model without loss, and every
  probability, confidence, legend entry, and usage count survives.

A fixture that stops matching is a compatibility signal, not a test to adjust. Re-fetch
the official page, decide whether the API changed or `jev` did, and record the outcome
in `docs/api-compatibility.md`.

## Updating

1. Re-fetch the page and diff it against `references/05-typesafe-docs/pages/`. A
   difference is expected — record it rather than assuming the snapshot is current.
2. Copy the example verbatim, including field order where the source shows one.
3. Update `_source` and the table above.
4. Note the change in `CHANGELOG.md` if it alters what `jev` sends or accepts.
