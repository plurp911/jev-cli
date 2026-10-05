# Clef compatibility research

Research date: 2026-10-03. Sources below were read during this implementation.
The API contract comes from provider documentation and provider-owned source, not
community clients. During this initial research phase, no live inference request
was made and no weights were downloaded. Later hosted and local runs are recorded
separately in [live testing](clef-live-testing.md) and its verification receipts.

## Cloudflare-hosted contract

The [Clef model page](https://developers.cloudflare.com/workers-ai/models/clef/)
defines the following request shape:

```json
{
  "model": "clef",
  "state": "Context for the decision",
  "questions": {
    "visible": {
      "type": "noul",
      "instructions": "Is the relevant information visible?"
    }
  },
  "images": [{"content_type": "image/png", "base64": "..."}]
}
```

| Field | Contract |
| --- | --- |
| `model` | `clef` or `clef-flash`, matching the selected endpoint |
| `state` | Required string, object, or array |
| `questions` | Required map containing 1–64 questions |
| Question identifiers | Letters, digits, underscore, period, hyphen; maximum 100 characters |
| `instructions` | Required nonempty string, object, or array |
| `choice.criteria` | 2–255 named options; descriptions may be strings, objects, arrays, or null |
| `score.criteria` | 2–10 ordered string, object, or array descriptions |
| `noul.criteria` | Optional `true` and/or `false` descriptions |

The [Clef Flash model page](https://developers.cloudflare.com/workers-ai/models/clef-flash/)
publishes the same question and media contract. Images are optional, precede the
state, and are shared by the questions. Each image accepts either a data URL or an
object with required `content_type` and `base64` fields. Supported content types
are `image/png`, `image/jpeg`, and `image/webp`. Ordinary remote URLs are refused.

| Media limit | Hosted maximum |
| --- | --- |
| Number of images | 4 |
| Decoded bytes per image | 4 MiB |
| Pixels per image | 16 megapixels |
| Total decoded image bytes | 8 MiB |
| Entire JSON request body | 13 MiB |
| Model context | 65,536 tokens for either model |

The limit on decoded bytes refers to decoding base64 into the encoded image file,
not expanding the image into an RGB pixel buffer. The hosted schema contains no
`videos` or `media_kwargs` field. Its introductory description of video capability
does not establish a hosted video request format. Long text state may be truncated
by the hosted service to fit its token budget.

## Authentication, routing, and response envelope

The [official REST setup](https://developers.cloudflare.com/workers-ai/get-started/rest-api/)
requires a Cloudflare account ID and Workers AI API token. Its token setup names
Workers AI Read and Edit permissions. Use Bearer authorization and JSON content.

```text
POST https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/run/@cf/cloudflare/clef
POST https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/run/@cf/cloudflare/clef-flash
```

REST responses wrap the model response in `result`, alongside `success`, `errors`,
and `messages`. Workers bindings return the model result directly; treating the
REST envelope as a System One answer would therefore fail.

The model result has `model`, `answers`, and `usage`. Answers retain the three
[System One types](https://docs.typesafe.ai/api): `noul` contains a yes probability;
`choice` contains the selected option, probabilities, and confidence; `score`
contains the expected zero-based level, legend, probabilities, and confidence.
Usage contains integer `input_tokens` and `output_tokens`. Structured instructions
and criteria already belong to the TypeSafe API, so they are not Clef-only features.

## Other hosted functionality

The [native REST capacity option](https://developers.cloudflare.com/workers-ai/features/reject-if-busy/)
is a platform option outside the model schema: add
`"options": {"rejectIfBusy": true}` to reject unavailable capacity instead of
waiting in the service queue. Rejection is HTTP 429, internal code 3040.

[Workers AI errors](https://developers.cloudflare.com/workers-ai/platform/errors/)
document HTTP 400 for malformed data or model/task selection, 403 for account,
model, or plan restrictions, 404 for invalid model IDs, 405 for unsupported
operations, 408 for timeout or abortion, 413 for excessive body size, and 429 for
allocation or capacity limits. These are not TypeSafe's 422/529 conventions.

[Asynchronous batch support](https://developers.cloudflare.com/workers-ai/features/batch-api/)
is identified by the model catalog's Batch capability. Neither Clef page advertises
Batch support. Client-side processing of JSONL records remains distinct from this
queued service API. Neither page advertises function calling or LoRA inference.

The [launch announcement](https://blog.cloudflare.com/clef-decision-models/)
describes fine-tuning as an FDE-assisted service, with a self-service platform still
planned. It provides no public Clef training endpoint to implement in this CLI.
The typed models do not emit free-form explanations or reasoning text.

[AI Gateway's Workers AI integration](https://developers.cloudflare.com/ai-gateway/usage/providers/workersai/)
is a separate platform feature. Current documentation uses the
`cf-aig-gateway-id` header for REST routing and advertises logging, analytics, and
caching. Gateway use must remain an explicit user choice; model selection alone
must not enable it.

## Local implementations and their differences

The [Cloudflare-owned Clef model card](https://huggingface.co/Cloudflare/clef)
and [Clef Flash model card](https://huggingface.co/Cloudflare/clef-flash) release
Apache-2.0 weights and a custom joint schema head. The
[official Python implementation](https://huggingface.co/Cloudflare/clef/blob/main/joint_schema_model.py)
provides `encode_record`, `collate_records`, `load_release_model`, and `systemone`.
Local records accept PIL images and video frame arrays, plus processor-specific
`media_kwargs`. Its default `max_length` is 16,384 tokens. Local instructions may
be omitted, in which case the question identifier supplies them. These are Python
object interfaces, not a documented HTTP video encoding.

The local `systemone` helper returns a System One body directly, reports zero
output tokens, and rounds probabilities to four decimal places. Its Choice
confidence is the maximum option probability; its Score confidence is likewise the
maximum level probability. Do not recompute either value using another provider's
confidence interpretation.

The automatically generated Hugging Face chat/vLLM examples do not show how to
load the custom joint schema head. The publisher's explicit loader and decision
helper are the relevant examples for actual Clef decisions.

| Serving implementation | Officially documented difference |
| --- | --- |
| [Ollama decision guide](https://docs.ollama.com/capabilities/decision) | Local Clef requires Ollama 0.35.1 or later; no API key; PNG/JPEG/WebP images are raw base64 strings, not data URLs or Cloudflare objects |
| [Ollama System One endpoint](https://docs.ollama.com/api/systemone) | `POST /v1/systemone`; optional `keep_alive` duration string or seconds; 64 KiB body without images, 32 MiB with images; input must fit context and is never truncated; no streaming, video, tools, or generation controls |
| [Ollama Clef library entry](https://ollama.com/library/clef) | 1–64 questions; Choice and Score each use 2–26 options/levels; local context is configured by the server |
| [llama.cpp server](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md#post-v1systemone-typesafe-compatible-system-one-api) | Clef uses `/v1/systemone`, Choice permits 255 options, Score permits 2–10 levels; its current documentation explicitly says Clef images are unsupported |

There is no single universal local Clef media contract. In particular, a Cloudflare
image object must not be sent unchanged to Ollama. Local adapters require their own
provider selection, validation, and serialization.

Ollama's [Clef encoder source](https://github.com/ollama/ollama/blob/main/decision/clef.go)
confirms structured descriptions and instructions are accepted for Clef, and that
omitted instructions fall back to the question identifier. It sorts Choice options
by identifier and enforces the documented 26-candidate limit for both Choice and
Score. Its
[shared request compiler](https://github.com/ollama/ollama/blob/main/decision/systemone.go)
rejects videos and enforces 1–64 questions. Restrictions in that file's generic
Nimble/Tev question encoder must not be mistaken for Clef restrictions: Clef takes
its own encoding path.

## Existing integration code inspected

The publisher-owned Python implementation above was read in full. Additional
community integrations were examined only for discovery and design comparisons:

- [node-decision-model](https://github.com/fraserxu/node-decision-model) documents
  provider adapters and its published package documents Cloudflare embedded images.
- [decision-model-eval](https://github.com/relevan-dev/decision-model-eval) documents
  a text-only local wrapper around the publisher's `systemone` helper and a separate
  hosted Workers AI adapter.
- [A Clef SDK adapter example](https://flaviocopes.com/clef/) explicitly unwraps
  Cloudflare's REST envelope before exposing the response to a TypeSafe SDK.
- [zero-shot-ie-bench's Clef client](https://github.com/umstek/zero-shot-ie-bench/blob/main/engines/clef_client.py)
  was read for its explicit model-to-endpoint mapping. Its automatic `.env` loading
  and credential discovery are incompatible with this repository's privacy rules
  and are not implementation candidates.

These projects do not establish provider behavior, and none of their code was
copied. The implementation must be judged against the official sources above.
