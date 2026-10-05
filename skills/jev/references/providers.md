# Provider selection and explicit media

Run `jev doctor` with the intended provider and account flags before inference.
TypeSafe remains the default. Model selection alone does not switch providers.

| Provider | Credentials | Supported media |
| --- | --- | --- |
| `typesafe` | `JEV_API_KEY`, `JEV_API_KEY_FILE`, SDK convention, or OS store | Text/structured state |
| `cloudflare` | Custom key namespace plus Cloudflare account ID | PNG/JPEG/WebP |
| `ollama` | None on loopback; custom namespace for explicit remote HTTPS | PNG/JPEG/WebP with Clef vision weights and Ollama 0.35.1+ |
| `llamacpp` | None on loopback; custom namespace for explicit remote HTTPS | Clef text; upstream Clef vision is unavailable |
| `huggingface` | None on loopback; custom namespace for explicit remote HTTPS | Images and prepared video through the separately started Python bridge |

For Cloudflare, use `--provider cloudflare --cloudflare-account-id ACCOUNT_ID
--model clef-flash`. `ACCOUNT_ID` must be exactly 32 hexadecimal characters
(for example, synthetic `0123456789abcdef0123456789abcdef`). The credential belongs in `JEV_CUSTOM_API_KEY` or a private
secret-manager file named by `JEV_CUSTOM_API_KEY_FILE`. Do not ask for or print it.
`jev auth login` stores a TypeSafe credential and does not authenticate Cloudflare.

For local Ollama, use `--provider ollama --model clef-flash`. It defaults to
`http://127.0.0.1:11434`. No dummy key is needed. The CLI does not install the server
or download weights. Local model availability is the server's responsibility. Loopback identifies the
server receiving the request, not its downstream behavior. For content that must
stay on this machine, confirm the server runs the model locally without cloud
offload or proxy forwarding before sending it.

For Windows, check the current [Ollama Clef Flash System One issue](https://github.com/ollama/ollama/issues/18769)
and [proposed large-file seek fix](https://github.com/ollama/ollama/pull/18777)
before choosing a release. Failures were reported on Windows with Ollama 0.35.1;
a successful chat-completions call does not prove `/v1/systemone` works. Test
the decision endpoint on the chosen release; do not silently switch to cloud.

For llama.cpp, obtain its compatible [Clef GGUF](https://huggingface.co/ggml-org/Clef-GGUF)
or [Clef Flash GGUF](https://huggingface.co/ggml-org/Clef-Flash-GGUF) conversion.
Do not assume an Ollama model blob is interchangeable: the tested Clef Flash
blob was refused by llama.cpp with a tensor-count mismatch.

For `--provider huggingface`, use the project's [Python bridge and setup guide](https://github.com/plurp911/jev-cli/blob/main/docs/clef.md#publisher-python-weights-and-video)
and [reproducible local setup](https://github.com/plurp911/jev-cli/blob/main/docs/development/clef-live-testing.md).
The [bridge script](https://github.com/plurp911/jev-cli/blob/main/scripts/clef-server.py)
runs separately from the installed CLI. Prepare and review a pinned publisher
release and install its model-card dependencies in your chosen environment;
the bridge loads an existing local release and executes its explicitly named
publisher Python module. It downloads no weights or dependencies at startup.

```sh
python3 scripts/clef-server.py --model-path /path/to/pinned/clef-flash \
  --model-name clef-flash --device cuda --dtype bfloat16
jev --provider huggingface --model clef-flash doctor
```

The script path above is in the source checkout. The bridge defaults to
`http://127.0.0.1:8787`; local inference needs no dummy key. CPU mode, dtype,
and hardware compatibility depend on the selected model release. The CLI does
not install, download, or launch any local runtime and never falls back to cloud.

For slow CPU inference or timeout advice, include **deadline, retries, and server
lifecycle** in the answer:

- Choose and explain a finite per-attempt deadline, including the CLI maximum of
  3,600 seconds and the automatic retry count. For example, `--timeout 600 --retries 0`
  allows one attempt up to ten minutes without automatic duplicate attempts; 600 is
  illustrative, not a special CPU default. CPU inference can take minutes.
- Explain that a client timeout or disconnect stops waiting but does not cancel active
  PyTorch inference. The bridge serializes inference, so it can remain busy and later
  requests can wait behind it. Waiting lets the active work finish; client cleanup does
  not release that server-side inference slot. Manage a shutdown/restart through the
  separately started server when needed; the CLI does not launch or stop the runtime.

The selected project Hugging Face bridge accepts supplied state as any JSON value,
including scalar/null or a blank string; missing state is invalid. Native request
files preserve JSON scalar/null Choice/Score descriptions and Score legends.
Missing/null/exactly-empty instructions use the question ID; whitespace is preserved.
An absent Noul criteria side uses its default, but an explicitly null side suppresses
that description; overall null criteria uses defaults. Hugging Face Score accepts
2–255 levels as this client's resource bound, not a publisher head/API limit; the fixed
schema must still fit the total tokenizer budget. TypeSafe/Cloudflare retain 2–10 and
generic Ollama 2–26. Do not infer their state/instruction acceptance from the bridge.
These forms follow the [pinned publisher implementation](https://huggingface.co/Cloudflare/clef/blob/2f3de3dd85f379784083b0814d997ab627200f0c/joint_schema_model.py).

Only named `--image PATH` files are read. No directory search, glob expansion,
automatic screenshot capture, remote image URL, or path embedded in a row causes
an upload. Images are PNG/JPEG/WebP, at most four, 4 MiB and 16 million pixels each,
8 MiB total. `map --images-field FIELD` selects embedded row images explicitly.
For image-only Cloudflare decisions, explicitly supply `--state ''` with valid images.
TypeSafe, Ollama and llama.cpp retain their nonempty text-state requirement.

Use `--video-frame PATH` in order for the Hugging Face bridge, with optional
`--video-fps FPS` to establish source cadence. One CLI video has 1–32
equal-dimension prepared frames; the CLI does not decode video files.
When giving a video command, state its prerequisite: the project's Python bridge
must be started separately with preexisting local weights. An already-running bridge
does not mean the CLI supplies or launches it.
Native documents/MCP accept video objects with frames and optional bounded source
metadata, for example `{"fps":30,"total_num_frames":90,"frames_indices":[0,60],"duration":3}`
for two sparse supplied frames. Indices must increase, match the supplied frame
count, and stay below the total. Sparse preselected frames cannot be resampled.
For video advice, explicitly distinguish `--video-fps`/native metadata `fps` (source
timing) from supplied sparse-frame spacing and `media_kwargs.fps` (processor target
sampling when enabled). Target sampling does not replace the source rate, and sparse
preselected frames cannot be resampled. Do not infer timestamp accuracy when source
timing is absent; the installed processor supplies defaults.

`--max-length` limits total input tokenization (1–65,536, default 16,384),
not generated output. `--max-state-tokens` independently limits textual state
(0–65,536); zero omits text tokens while retaining required supplied state and
fixed schema/media. Explain both budgets when asked: total input tokenization and
the separate text-state cap, including zero's behavior. `--media-kwargs` is a constrained JSON object accepting
`min_pixels`, `max_pixels`, `fps`, `num_frames`, and `do_sample_frames`, not
arbitrary processor kwargs. Pixel limits are positive, at most 16 million,
minimum ≤ maximum; sampling FPS is positive and at most 120, and frame count
is 1–32. `fps` and `num_frames` are alternatives and cannot both be supplied.
Pixel targets apply per still image or to the whole video clip including temporal
padding, not to each video frame; explicit targets are also capped at
64,000,000 divided by the number of images plus clips. Actual patch-rounded
allocation must fit 16,000,000 pixels per frame and 64,000,000 in total.
Sampling defaults to false to preserve supplied frames. These controls and video
require `huggingface`; Cloudflare/Ollama do not accept native video here.

For an MCP host, configure the provider/account/model on `jev ... mcp serve`; tool
calls carry embedded media only. Forward custom credential environment variables by
name if the host filters its environment; never put credential values in host config.

The provider documentation is the authority for endpoint-specific behavior:
[Cloudflare Clef](https://developers.cloudflare.com/workers-ai/models/clef/),
[Ollama System One](https://docs.ollama.com/api/systemone), and
[publisher weights](https://huggingface.co/Cloudflare/clef).
