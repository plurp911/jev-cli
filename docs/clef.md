# Clef and Clef Flash

`jev` supports Cloudflare's hosted Clef models and local Clef servers. The default
remains TypeSafe's `jev-latest`; select another provider explicitly. This CLI is an
independent community project, with no Cloudflare or TypeSafe endorsement.

The provider contracts and CLI behavior below were rechecked on 2026-10-05.
Dated research and test receipts record their own execution epochs; they do not
automatically become verification of later binaries or documentation changes.

## Choose a provider

| Provider | Default base URL | Default model | Authentication | Images |
| --- | --- | --- | --- | --- |
| `typesafe` | `https://api.typesafe.ai` | `jev-latest` | Existing TypeSafe sources | Unsupported |
| `cloudflare` | `https://api.cloudflare.com` | `clef` | `JEV_CUSTOM_API_KEY` or `JEV_CUSTOM_API_KEY_FILE` | PNG, JPEG, WebP |
| `ollama` | `http://127.0.0.1:11434` | `clef` | None on loopback | PNG, JPEG, WebP; vision weights required |
| `huggingface` | `http://127.0.0.1:8787` | `clef` | None on loopback | Images and prepared video frames |
| `llamacpp` | `http://127.0.0.1:8080` | `clef` | None on loopback | Clef images unsupported upstream |

`--model clef-flash` selects Flash. Ollama also accepts installed model tags; a
llama.cpp model name must match the server's loaded model or configured alias.
Cloudflare accepts `clef`, `clef-flash`, and their full
`@cf/cloudflare/…` identifiers, and sends the required short selector in the body.
`--endpoint` overrides the base URL without changing the selected protocol. Plain
HTTP is permitted only on unambiguous loopback; redirects are never followed.
The saved `endpoint` applies only to its saved provider; an omitted saved provider
means TypeSafe. Switching to a different provider uses that provider's default
endpoint unless `--endpoint` is supplied explicitly.
An explicitly configured remote local-provider server uses the custom key
sources. Local loopback calls never read or send any credential, even if key
variables or key files are configured.
Loopback identifies the server receiving the request. Whether inference stays on
this machine also depends on that server: a forwarding proxy or cloud-offloaded
model can send the content elsewhere.

The five values are also available as the nonsecret `provider` configuration
setting. Flags override configuration. Account resolution is
`--cloudflare-account-id`, then `CLOUDFLARE_ACCOUNT_ID`, then the
`cloudflare_account_id` configuration setting. Account IDs must be 32 hexadecimal
characters. The account environment variable alone never changes providers.
Configuration commands remain usable while an account has not yet been supplied.
For an incomplete saved Cloudflare provider, `jev doctor` reports a
`configuration_error` and `live.checked: false` without contacting a server;
`jev auth status` reports the error with `effective_source: null` and no credential
lookup. Supply the account ID to finish configuring that provider.

## Cloudflare

Create a Workers AI API token and obtain your account ID using
[Cloudflare's REST setup](https://developers.cloudflare.com/workers-ai/get-started/rest-api/).
Supply the token through `JEV_CUSTOM_API_KEY`, or point
`JEV_CUSTOM_API_KEY_FILE` at a file your secret manager maintains. Never put a token
in command-line arguments or the configuration file. TypeSafe keys and the OS
keychain are never consulted for this provider.

```sh
export CLOUDFLARE_ACCOUNT_ID=0123456789abcdef0123456789abcdef
# Configure JEV_CUSTOM_API_KEY or JEV_CUSTOM_API_KEY_FILE through your secret manager.
jev --provider cloudflare --model clef-flash noul 'Is this urgent?' \
  --state 'Checkout is failing for every customer.' --output json
jev --provider cloudflare --image screenshot.png noul 'Is an error visible?' \
  --state 'Inspect this application screenshot.' --dry-run
```

Cloudflare inference uses
`POST /client/v4/accounts/{account}/ai/run/@cf/cloudflare/{model}`. The CLI unwraps
the Workers AI REST `result` envelope and presents the existing versioned Jev JSON
answer contract. `--reject-if-busy` sends `options.rejectIfBusy`, following
[Cloudflare's native REST capacity contract](https://developers.cloudflare.com/workers-ai/features/reject-if-busy/).
A documented capacity rejection (HTTP 429, code 3040) is returned without retrying
that rejection. Other retryable failures still use the configured bounded retry policy.

Hosted checks on 2026-10-04 UTC found that both Clef models on a Workers AI Free
account reject this option with HTTP 422, code 5012. The retained reports contain
the numeric error, so the precise cause is not established by that evidence.
Capacity rejection could therefore not be
verified for these models. The CLI reports the failure without removing the
option or resending a request that could wait in a capacity queue. Omit
`--reject-if-busy` to use ordinary hosted inference; the capacity option remains
unverified until Cloudflare accepts it for Clef.

`jev models` and `jev doctor --live` use the provider's model-listing endpoint;
they do not perform an inference. Cloudflare's published model-search schema does
not specify entry fields, so its normalized CLI catalog reports the two supported
Clef models after validating a successful search response. It is not an account
entitlement check or a complete Workers AI catalog; release dates are empty.

## Local open-source inference

Install an upstream server and download the weights yourself. The CLI does not
download weights, start processes, install a runtime, or silently switch to a cloud
provider. An unavailable local server produces an ordinary connection error.

Ollama's [decision guide](https://docs.ollama.com/capabilities/decision) requires
Ollama 0.35.1 or later for Clef and Clef Flash, including vision weights for images:

```sh
ollama pull clef
ollama pull clef-flash
jev --provider ollama --model clef-flash --image screenshot.webp \
  noul 'Does the screenshot show a failed checkout?' --state 'Checkout screenshot'
jev --provider ollama --keep-alive 10m choice 'Which team should handle this?' \
  --state 'My refund has not arrived.' --option billing --option technical
jev --provider ollama models --output json
```

`--keep-alive` is Ollama-only: a duration such as `5m`, seconds, zero to unload after
the call, or a negative value to keep the model loaded. The server interprets the
duration. The native request document also accepts `keep_alive` as a string or
integer. The CLI serializes images as raw base64 for Ollama, rather than sending
Cloudflare image objects. See the [System One endpoint](https://docs.ollama.com/api/systemone).

llama.cpp's [server documentation](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md#post-v1systemone-typesafe-compatible-system-one-api)
describes `/v1/systemone` and its Clef model loading requirements. Start a compatible
server with Clef GGUF weights and a model alias, then use the alias with `--model`:

Clef evaluates each complete prompt in one physical batch. Set `--batch-size` and
`--ubatch-size` large enough for the full prompt, including questions; a long
context setting alone is insufficient. The tested CPU configuration used
`--ctx-size 8192 --batch-size 8192 --ubatch-size 8192 --parallel 1`.

Use the llama.cpp maintainer's [Clef GGUF](https://huggingface.co/ggml-org/Clef-GGUF)
or [Clef Flash GGUF](https://huggingface.co/ggml-org/Clef-Flash-GGUF) conversion,
as linked by the [merged Clef support change](https://github.com/ggml-org/llama.cpp/pull/29831).
Ollama's model blobs are a different conversion: the tested llama.cpp build refused
an Ollama Clef Flash blob with a tensor-count mismatch. Do not assume GGUF files
from the two runtimes are interchangeable.

```sh
jev --provider llamacpp --model clef noul 'Is this urgent?' --state 'Checkout is down.'
```

### Publisher Python weights and video

The explicit `huggingface` provider uses this repository's separately implemented
[Python bridge](../scripts/clef-server.py), calling the publisher's
[release loader and System One function](https://huggingface.co/Cloudflare/clef/blob/main/joint_schema_model.py).
This bridge protocol belongs to this project; it is not a Cloudflare hosted API.
The bridge script is distributed in the source repository and runs separately;
the installed `jev` binary connects to it without embedding a Python runtime.

This provider preserves the publisher's wider JSON input forms: state may be a
number, boolean, null, or blank string as well as text, objects, and arrays. A state
source must still be explicitly supplied. Instructions omitted from a request,
null, or exactly `""` use the question ID; whitespace strings are preserved.
Criterion descriptions and Score legends also preserve JSON scalars and blank
strings. An explicitly null Noul side suppresses its default description. Score
supports up to 255 levels; the 255-option/level ceiling and minimum of two are
this client's resource and meaningful-question bounds, not limits of the publisher's
dynamic decision head. The fixed schema must still fit the selected token budget.
Other providers retain their own content requirements. See the
[compatibility decision and primary sources](development/clef-publisher-input-compatibility.md).

Download a pinned [Clef](https://huggingface.co/Cloudflare/clef) or
[Clef Flash](https://huggingface.co/Cloudflare/clef-flash) release yourself, including
its publisher Python implementation, weights, and processor files. Install its
model-card dependencies in your chosen environment (PyTorch, Transformers, Pillow,
NumPy, Hugging Face Hub, and safetensors). The publisher reports testing PyTorch
2.11 and Transformers 5.10.2. The verified publisher revisions and their file hashes
are recorded in the [Clef manifest](../scripts/clef-local/clef-manifest.json) and
[Clef Flash manifest](../scripts/clef-local/clef-flash-manifest.json).
Review the selected release's code: the bridge imports
that explicitly named local Python module and executes it at startup.
The [reproducible local setup and live-test guide](development/clef-live-testing.md)
provides pinned dependencies, exact release revisions, per-file integrity verification,
original synthetic fixtures, and explicit opt-in hosted/local smoke commands.

```sh
python3 scripts/clef-server.py --model-path /path/to/pinned/clef-flash \
  --model-name clef-flash --device cuda --dtype bfloat16
jev --provider huggingface --model clef-flash --timeout 600 --retries 0 --image screenshot.png \
  noul 'Is an error visible?' --state 'Application screenshot'
jev --provider huggingface --model clef-flash --timeout 600 --retries 0 \
  --video-frame frame-001.png --video-frame frame-002.png \
  --video-fps 2 --max-length 16384 --max-state-tokens 4096 \
  --media-kwargs '{"max_pixels":262144}' \
  noul 'Does the object move to the left?' --state 'Ordered video frames'
```

Repeat `--video-frame` for one ordered video, with 1–32 equal-dimension frames.
Prepare frames yourself; the CLI does not discover frames or decode video files.
Native request documents and MCP support up to four videos, each shaped as
`{"frames":[{"content_type":"image/png","base64":"…"}]}`. Image objects and
data URLs use the same input validation as still images.

CPU inference can take minutes. Choose an explicit deadline, such as
`--timeout 600 --retries 0`, to avoid automatic duplicate attempts after a short
timeout. The bridge serializes inference. A client timeout stops waiting for the
answer; it does not cancel an inference already running in PyTorch. Another
request can therefore wait behind that work. The same timeout and retry settings
apply when launching `jev mcp serve` for this bridge.

`--video-fps` supplies source cadence for the one CLI frame sequence, greater than
zero and at most 120. It differs from the processor's target sampling `fps`.
Native documents and MCP can attach a `metadata` object to each video:
`{"fps":30,"total_num_frames":90,"frames_indices":[0,60],"duration":3}` for
two supplied frames. `fps` is required; the other fields are optional. Frame indices
must be strictly increasing, below the total source-frame count, and match the
supplied frame count. The total is at most 10 million and at least the supplied
count; both duration and total/fps are bounded to one day. The bridge derives omitted
counts and indices from the supplied sequence. Model timestamps
follow source index/fps. Sparse preselected frames cannot be resampled; complete
sequences may use the bounded sampling controls. With no metadata, the installed
processor supplies its timing defaults.

`--max-length` sets the publisher's input/tokenization limit (1–65,536, default
16,384), not generated output length. The fixed question/media tokens must fit
inside that limit; excess state is truncated by the publisher encoder.
`--max-state-tokens` independently caps textual state at 0–65,536 tokens; zero omits
state tokens while retaining the required supplied state and fixed schema/media.
The total `--max-length` still applies. The bridge composes the publisher's public
encoding, model, and answer functions to support this separate budget.
Publisher
input/tokenization errors return HTTP 400, while runtime failures return HTTP 500. `--media-kwargs` accepts a JSON object with
`min_pixels` and `max_pixels` (positive, at most 16 million, minimum ≤ maximum),
`fps` (positive, at most 120), `num_frames` (1–32), and `do_sample_frames` (boolean).
The bridge translates these constrained controls into the processor's image/video
options. Frame sampling defaults to false to preserve the supplied order. `fps` and `num_frames` are alternatives; requesting both is rejected. Singleton
videos are padded with a duplicate frame for the native temporal patch; sampled
videos need at least two frames. Padding counts against the processor budget. Frame
sampling controls are interpreted by the installed publisher processor.
When `num_frames` is selected, the bridge clears the processor's default sampling
FPS so the two alternatives do not conflict.

The processor interprets `min_pixels` and `max_pixels` per still image and across
the entire video clip, including temporal padding. These are different units:
a video maximum is not a maximum for each frame. The bridge preserves the
publisher defaults of a 65,536-pixel image minimum and a 4,096-pixel clip minimum.
The default clip maximum is 25,165,824 pixels; the still-image maximum is capped
at the client's 16,000,000-pixel limit, below the publisher's 16,777,216 default.
Default maxima are reduced when needed to share the 64-million-pixel processor
budget between images and clips. Each minimum is clipped to its effective maximum;
an explicit minimum overrides both defaults. Explicit controls remain bounded to
16,000,000 pixels. Before decoding, the bridge and client account for 32-pixel
spatial patches, temporal padding, and possible sampling output, rejecting
allocations above 16,000,000 pixels per frame or 64,000,000 pixels in total.
Processor pixel settings are targets rounded to spatial and temporal patches;
rounding can exceed a nominal target. The actual allocation bounds above remain
hard limits.
These bridge bounds and the default of preserving supplied frames apply even when
the installed processor's own settings allow larger inputs or automatic sampling.

The bridge accepts 1–64 questions, with IDs at most 1,024 Unicode characters,
2–255 Choice options, and 2–255 Score levels. These are this bridge's client bounds.

The bridge serves `/v1/systemone` and `/v1/models` on `127.0.0.1:8787`. It loads
only an existing local release with offline/local-files-only loader settings;
it does not install dependencies or download weights. Only the loopback bind is
allowed. Browser Origin headers and non-loopback Host headers are refused, request
bodies are bounded, inference is serialized, and errors omit supplied content.
The bridge's model listing reports its loaded model. CPU mode and alternative
floating-point types are explicit startup options; hardware/runtime compatibility
remains a property of the selected model release.

## Vision inputs and bounds

Repeat `--image PATH` to embed explicitly named PNG, JPEG, or WebP files in order.
The file contents determine their format, not the extension. Images are shared by
all questions and sent before the state; an explicit state source is still required.
The Hugging Face bridge accepts single-frame images; animated WebP is refused
before a request is sent. Prepare individual frames explicitly for video input.
Cloudflare accepts an empty text state with validated images: pass `--state ''`,
or an empty string in a request document. Ollama requires nonempty text even with
images. Without a state flag, scalar commands still read piped text from stdin.
URLs, directory
discovery, symlinks, and paths found inside input records are not image inputs.
On macOS, the standard root aliases `/tmp`, `/var`, and `/etc` are accepted only
when their observed targets are the corresponding `/private` directories. All
components of the rewritten path still undergo no-follow checks.
Other symlinked directory aliases are refused, including a symlinked `/home` on
some Linux installations. Use an explicitly named path without symlink components
or a relative path from the real directory.

For request files and MCP, embed a base64 data URL or an object:

```json
{
  "model": "clef-flash",
  "state": "Inspect this screenshot",
  "images": [{"content_type": "image/png", "base64": "<base64 encoded PNG>"}],
  "questions": {
    "visible": {"type": "noul", "instructions": "Is an error message visible?"}
  }
}
```

The placeholder above must be replaced by actual encoded image bytes. An image
source in a request/template and `--image` cannot both be supplied. Templates and
per-record media are also alternatives, to avoid silently duplicating inputs.
`--dry-run` validates and prints the actual provider request, including embedded
image data, and never resolves a token or opens a network connection. Its output
contains the supplied content; treat it accordingly.

The CLI enforces these media bounds for both hosted and local vision: four images,
4 MiB of compressed image file bytes and 16 million pixels per image, and 8 MiB total file
bytes. Local video and still images share that 8 MiB total and a 64 million
pixel aggregate budget. The bridge also bounds processor resize/sampling allocations
against that pixel budget before decoding. These are Cloudflare's documented bounds and deliberate client bounds for
local inference. Base64 is decoded before checking bytes. The CLI validates the
format header and dimensions without decompressing image pixels; this is not full
image integrity validation. The inference server performs image decoding.

### Provider limits

| Request limit | Cloudflare | Ollama | llama.cpp | Python bridge |
| --- | --- | --- | --- | --- |
| Questions | 1–64 | 1–64 | Existing bounded input applies | 1–64 |
| Choice options | 2–255 | 2–26 | 2–255 | 2–255 |
| Score levels | 2–10 | 2–26 | 2–10 | 2–255 |
| Whole JSON body | 13 MiB | 64 KiB text; 32 MiB with images | Existing bounded input | 13 MiB |
| Cloudflare question IDs | At most 100 ASCII letters/digits/`_`/`.`/`-` | Not applied | Not applied | Not applied |

The existing 1 MiB text-source default remains; increase `--max-input-bytes` when
reading a request document or JSONL record containing larger embedded images.
Image files named with `--image` use the separate image bound.

## Batch, evaluation, and agents

Existing commands, JSON output, scalar output, gates, batch routing, cancellation,
and offline previews work with the selected provider. For `map`, explicitly select
the image field in records:

```sh
jev --provider ollama --model clef-flash map --request questions.json \
  --input records.jsonl --state-field state --images-field images
jev --provider cloudflare eval --request questions.json --dataset labelled.jsonl
jev --provider ollama mcp serve
```

For video records use `--provider huggingface --videos-field videos`.
Evaluation rows can contain optional `images` and `videos` arrays alongside `state` and
`labels`. Labels remain local. Media and provider options participate in request
and row fingerprints, so changing images cannot silently reuse a prior judgment.
Unchanged TypeSafe requests retain the existing resume fingerprints.

MCP uses embedded image/video arguments rather than host filesystem paths, and keeps the
provider/account fixed at server startup. All five tools use the same validation
and transport as their CLI counterparts. See [MCP](mcp.md).

## Pinning and verification limits

A model name is not a weight or runtime fingerprint. A Cloudflare result, an
Ollama tag, and a local server alias do not establish the same inference behavior.
Record the selected provider, reviewed weight revision or digest, runtime and
processor versions, and inference options when calibrating a deployment.
`jev eval` and `map --resume` detect changed supplied inputs and options; they cannot
detect weights replaced behind an unchanged model name. Recalibrate after such a
change. The [live-test helpers](development/clef-live-testing.md#execution-provenance)
can hash explicitly named files without claiming to attest the server's loaded memory.

The [recorded results](development/clef-live-testing.md#recorded-results) distinguish
offline adapter checks, actual hosted/local inference, synthetic quality measurements,
and skill evaluations. Release archive and native CLI smoke checks establish
packaging behavior, not successful inference with every model on every platform.
Hosted Clef and Clef Flash image inference passed the retained smoke matrices and
the original 32-image synthetic benchmark. Those tasks do not establish production
accuracy or calibration. Full unquantized Clef 27B Python inference, full GPU
residency/Python CUDA inference, native model inference across operating systems,
production-data quality, and successful Clef capacity rejection remain unverified.

## Capabilities the endpoints do not expose

Both Clef models return typed probabilities and decisions, with structured
instructions and criteria. The CLI preserves the returned confidence rather than
recomputing it; calibrated thresholds do not transfer automatically between models.
Cloudflare documents a 65,536-token context and may truncate long text state.
Ollama requires the complete input to fit its loaded context and does not truncate.

The hosted Clef schemas expose images, not video. Ollama's System One endpoint
explicitly excludes video, streaming, tools, and generation controls. llama.cpp
currently excludes Clef images. Unsupported fields are rejected before any
request is sent. Local video and processor controls are available only through
the explicitly selected Python bridge.
Cloudflare's launch describes assisted fine-tuning, but supplies no public Clef
training endpoint; the model catalog does not advertise asynchronous Batch or LoRA
support for Clef. Client-side JSONL processing remains available. AI Gateway
analytics/caching are not enabled automatically.

Provider source evidence and integration comparisons are recorded in
[Clef research](development/clef-research.md). Hosted contracts:
[Clef](https://developers.cloudflare.com/workers-ai/models/clef/),
[Clef Flash](https://developers.cloudflare.com/workers-ai/models/clef-flash/),
[input schema](https://developers.cloudflare.com/workers-ai/models/clef/schema-input.json),
[capacity option](https://developers.cloudflare.com/workers-ai/features/reject-if-busy/),
[errors](https://developers.cloudflare.com/workers-ai/platform/errors/).
