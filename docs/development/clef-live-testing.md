# Opt-in Clef verification

The ordinary verification gate runs the offline Python harness tests. It does not
load weights, contact an inference provider, or reuse the ignored TypeSafe live
tests. Actual inference is a separate, explicitly invoked action.

## Synthetic smoke matrix

[`scripts/clef-live.py`](../../scripts/clef-live.py) creates original 256×256
images: a red square, a blue circle, and two frames moving the square rightward.
PNG generation uses only the standard library. `--with-pillow` also requests JPEG
and WebP; unavailable encoders appear in `skipped`, rather than passing silently.
Pillow encoding is reproducible within the pinned encoder environment; compressed
JPEG/WebP bytes are not promised identical across encoder versions.

Inspect the cases without starting the CLI, reading credentials, or making requests:

```sh
python3 -B scripts/clef-live.py plan --provider cloudflare --with-pillow
python3 -B scripts/clef-live.py fixtures --output-directory /tmp/clef-fixtures
python3 -B scripts/test-clef-live.py
python3 -B scripts/test-clef-model-manifest.py
```

`fixtures` requires a new explicitly named directory. `run` generates temporary
fixtures, removes its temporary files afterward, and writes only the explicitly
named, new report file. Existing reports are refused.

The matrix checks Noul, Choice, Score, mixed `ask`, text/object/array state,
object/array instructions and criteria for all three primitives,
supported image formats, multiple images, image mapping/evaluation, unchanged
resume output, and all five MCP tools. Cloudflare additionally checks empty
explicit state with images; the Python bridge additionally checks ordered video.
llama.cpp's unsupported media cases are reported as skipped. Every subprocess uses
`--no-config`, an explicit provider/model, and `--retries 0`. The entire matrix's
conservative request budget must fit `--max-requests` before the first process
starts. Resume reserves two requests even though a correct resume sends none, so
an accidental resend remains within the cap. The cap is per run, not a billing or
token limit. The maximum accepted cap is 100.

Start a local provider separately, explicitly install its model, and then run:

```sh
python3 -B scripts/clef-live.py run --jev target/release/jev \
  --provider huggingface --model clef-flash --endpoint http://127.0.0.1:8787 \
  --max-requests 30 --timeout 300 --with-pillow --report /tmp/clef-local-smoke.json
```

For Ollama, select `--provider ollama` and `http://127.0.0.1:11434`; for llama.cpp,
select `--provider llamacpp` and the server's explicit loopback endpoint with
`--max-requests 21`. Its text-only matrix includes mapping, labelled evaluation,
and both resume cases; vision and video remain explicitly skipped. Local
smoke runs are anonymous and refuse remote endpoints or a credential file.
CPU inference can be slow; `--timeout` is the explicitly chosen per-request
timeout, between 1 and 3,600 seconds. Failure stops the matrix immediately.
For llama.cpp, download the maintainer's [Clef Flash GGUF](https://huggingface.co/ggml-org/Clef-Flash-GGUF)
or [Clef GGUF](https://huggingface.co/ggml-org/Clef-GGUF), using a build containing
[the merged Clef support](https://github.com/ggml-org/llama.cpp/pull/29831).
An Ollama blob is not a substitute for that conversion. Keep the chosen model
revision, file hash, runtime release and quantization with the smoke report.
The Python bridge matrix includes `--max-state-tokens`, ordered frames with
`--video-fps`, explicit two-frame sampling, and a native video carrying source
`fps`, `total_num_frames`, `frames_indices`, and `duration`. It reserves 28
requests without optional encoders, or 30 with both JPEG and WebP. Cloudflare
reserves 25 or 27 respectively. `plan` reports the selected environment's count.

Cloudflare requires an explicit account ID and a private token file:

```sh
python3 -B scripts/clef-live.py run --jev target/release/jev \
  --provider cloudflare --model clef-flash --account "$CLOUDFLARE_ACCOUNT_ID" \
  --key-file /explicit/private/token-file --max-requests 27 --timeout 120 \
  --with-pillow --report /tmp/clef-hosted-smoke.json
```

The harness passes only the file path through `JEV_CUSTOM_API_KEY_FILE`; only
`jev` opens the file. It strips inherited Jev/TypeSafe/Cloudflare settings and does
not inspect credential contents. Reports omit endpoint/account/credential paths,
raw request/response content, and subprocess diagnostics. Missing configuration
is an error, never a silently passed live test. Provider or quota errors stop the
run; no fallback request is sent. The known hosted `rejectIfBusy` rejection is
recorded separately in [the prior hosted evidence](clef-hosted-verification.json).

`passed` describes integration: schemas, IDs, distributions, outcomes, and stream
behavior. `quality_observations` and `quality_evaluation` separately record red
square agreement using an explicitly reported 0.5 observation cut. Disagreement
does not become a transport failure. Two synthetic labelled examples cannot
establish accuracy, calibration, or a production threshold.

## Execution provenance

Every new smoke and quality `run` report contains `provenance`, with SHA-256 and
byte size captured before and after execution for the resolved CLI target, both
test helpers, their shared provenance module, and the helper's Python interpreter.
An executable name on `PATH` is resolved once and that same absolute target is
passed to every subprocess. Reports distinguish unchanged, changed, missing, or
refused files; changed or unavailable fingerprints fail a real integration run.
Injected test runners are explicitly unobserved execution.

To include local model and runtime files, pass `--provenance-manifest PATH` to
either helper. This is an explicitly named JSON file, not a directory search:

```json
{
  "schema": "jev.clef.provenance-inputs/v1",
  "artifacts": [
    {"role": "model", "path": "weights/model-00001-of-00003.safetensors"},
    {"role": "model", "path": "weights/joint_head.safetensors"},
    {"role": "runtime", "path": "runtime/llama-server"},
    {"role": "runtime-library", "path": "runtime/libllama-server-impl.so"}
  ]
}
```

Paths resolve relative to the manifest's directory unless absolute. Supply all
relevant shards, heads, configuration/processor files, executable/module files,
and runtime libraries for your selected server; the example is an incomplete
illustration, not a model/runtime inventory. Each entry may additionally provide
an expected lowercase 64-character `sha256` and integer `size`; mismatches refuse
the run before CLI invocation. Only named regular files are read, no model code
is imported, and manifest/artifact paths are omitted from the report. Reports
identify entries by role and manifest order. The limits are a 1 MiB manifest,
512 entries, 64 GiB per file, and 128 GiB per before/after capture. Hashing large
weight inventories reads those bytes twice and can take time.

Provenance preflight requires descriptor-relative, no-follow file opening.
Report creation also uses an exclusive, descriptor-relative, no-follow open;
replacing a report parent with a symlink cannot redirect the write. Platforms
without those filesystem primitives refuse before inference and may be unable to
persist the refusal report. Native Windows/macOS execution of these Python
provenance helpers and model inference remains unverified. The later v0.3.0
release workflow separately built all five CLI targets and ran native CLI smoke
tests on Linux x86-64/ARM64, macOS ARM64 and Windows x86-64; that scope does not
exercise these helpers or model inference. See
[release verification](../release-verification.md).

These are disk fingerprints, not proof of the serving process's loaded files or
of a complete dependency inventory. The helper interpreter hash does not cover
its Python packages; a CLI launcher hash does not cover its interpreter or loaded
libraries. Before/after snapshots cannot prove which bytes executed between them.
Remote weights/runtime remain unobserved, and a model identifier is not a hash.
Without a manifest, local model/runtime files also remain unobserved. Historical
reports retain their original provenance gaps; current files cannot fill them
retroactively.

## Larger original synthetic benchmark

[`scripts/clef-quality.py`](../../scripts/clef-quality.py) adds a separate, reproducible
32-row labelled benchmark: square/circle × red/blue × four positions × two sizes.
All image bytes are generated locally; none are copied from an external dataset.
Eight examples contain a red square and 24 do not. llama.cpp receives the explicitly
generated text descriptions instead. Those sentences explicitly state the color and
shape, so that arm is a literal-reading sanity check. Its agreement, Brier score and
log loss do not measure vision and must not be compared with image-run metrics.
New reports identify `evaluation_task` and mark cross-input comparison unsupported.

```sh
python3 -B scripts/clef-quality.py plan
python3 -B scripts/test-clef-quality.py
python3 -B scripts/clef-quality.py run --jev target/release/jev \
  --provider ollama --model clef-flash:9b-q8_0 --endpoint http://127.0.0.1:18788 \
  --max-requests 32 --timeout 600 --report /tmp/clef-flash-quality.json
```

Cloudflare uses the same explicit account/token-file arguments as the smoke matrix.
The full 32-request reservation must fit the chosen cap before anything is sent.
Runs use concurrency one, fail fast, and never retry or switch providers. Reports
record agreement at 0.5, Brier score and log loss (clipped at 1e-15); disagreement
is a measurement, not an integration failure. Partial, reordered or incorrectly
labelled observations fail verification. Reports omit media, states, credential
paths and raw diagnostics. This shape family tests a narrow synthetic task; it
does not establish production accuracy, transferable calibration, or a production
decision threshold. Add explicitly authorized representative labelled data through
`jev eval` before making those claims.

## Explicit Python environment and pinned weights

Use Python 3.11 or 3.12 in a dedicated environment. The publisher reports testing
[PyTorch 2.11 and Transformers 5.10.2](https://huggingface.co/Cloudflare/clef).
The [direct requirements](../../scripts/clef-local/requirements.txt) pin those
versions plus the matching torchvision and loader/media dependencies. In official
[torchvision 0.26.0 metadata](https://pypi.org/pypi/torchvision/0.26.0/json), the
required torch version is exactly 2.11.0. Accelerate is installed for the
publisher's `device_map` loader, and torchvision is installed for the video
processor. No package here is a dependency of the `jev` binary.

For the recorded Linux CPU profile, the
[version inventory](../../scripts/clef-local/requirements-linux-cpu.lock)
contains all 37 packages. The
[selected wheel manifest](../../scripts/clef-local/python-linux-cpu-wheels.json)
records the official source, filename and SHA-256 of one compatible artifact per
package. Both
[download requirements](../../scripts/clef-local/requirements-linux-cpu.download.lock)
and [offline installation requirements](../../scripts/clef-local/requirements-linux-cpu.hashes.lock)
require those hashes. CPU torch and torchvision wheels come from the official
PyTorch index; the other wheels come from official PyPI release metadata.
All 37 selected artifacts, totaling about 251 MB, were downloaded and checked.
A fresh environment installed entirely from that wheelhouse passed dependency,
runtime-profile, real decoder and video-processor checks on 2026-10-04.

Use a **new** dedicated environment directory: already-installed package versions
do not prove agreement with wheel bytes. Python 3.12.3 and pip 24.0 were used for
this installation. pip's [explicit interpreter option](https://pip.pypa.io/en/stable/topics/python-option/)
works even when the target environment has no pip, as with the recorded uv-created
venv. The first pip command performs the explicitly requested downloads; installation
then uses only the local wheelhouse and
[hash-checking mode](https://pip.pypa.io/en/stable/topics/secure-installs/).

```sh
# Choose another new directory if this path already exists.
test ! -e target/clef-local/repro-venv
python3.12 -m venv --without-pip target/clef-local/repro-venv
python3 -m pip --isolated --disable-pip-version-check download \
  --no-cache-dir --no-deps --only-binary=:all: --require-hashes \
  --dest target/clef-local/wheelhouse \
  -r scripts/clef-local/requirements-linux-cpu.download.lock
python3 -m pip --python target/clef-local/repro-venv/bin/python --isolated \
  --disable-pip-version-check install \
  --no-cache-dir --no-index --only-binary=:all: --require-hashes \
  --find-links target/clef-local/wheelhouse \
  -r scripts/clef-local/requirements-linux-cpu.hashes.lock
python3 -m pip --python target/clef-local/repro-venv/bin/python --isolated \
  --no-cache-dir check
target/clef-local/repro-venv/bin/python -B scripts/clef-python-profile.py \
  --expected scripts/clef-local/python-linux-cpu-profile.json \
  --report target/clef-local/reproduced-python-profile.json
JEV_CLEF_PYTHON=target/clef-local/repro-venv/bin/python scripts/verify.sh
```

The [recorded runtime profile](../../scripts/clef-local/python-linux-cpu-profile.json)
checks CPython version, interpreter bytes, ABI, architecture, libc version, the
verifier process's mapped system/standard-library files, and the exact installed
package inventory. Output omits absolute paths and environment values; a mismatched
profile fails before writing a passing report. OS package updates can legitimately
change library hashes. Review such a difference and repeat validation before
recording a new profile; do not silently accept the old inference evidence.

This is artifact pinning and an exact comparison with the recorded Linux runtime,
not a Python/OS provisioning mechanism or a complete operating-system image lock.
The profile does not inventory every library later loaded by inference, the kernel,
hardware, drivers or installer bytes. The helper performs no network request,
publisher-code execution or model loading. The selected wheels are for Linux
x86-64 CPython 3.12; they are not a CUDA profile or native macOS/Windows claim.

For another platform, use the direct requirements and its selected PyTorch wheel
profile, then resolve, hash and record its own transitive artifacts and runtime.
On Windows, create the venv with `py -3.12 -m venv target/clef-local/venv` and use
`target\clef-local\venv\Scripts\python.exe` for the Python commands and
`Scripts\hf.exe` for the download. For CUDA, choose the matching 2.11.0/0.26.0
wheel index from [PyTorch's versioned installation instructions](https://pytorch.org/get-started/previous-versions/)
instead of the CPU profile. Native macOS/Windows hardware compatibility requires
its own run. The publisher's test claim covers its stated torch/Transformers pair,
not every dependency/hardware combination. Latest `huggingface-hub` 2.x is outside
Transformers 5.10.2's `<2.0` requirement.

`JEV_CLEF_PYTHON` explicitly selects the interpreter for the verifier's
`test-clef-server.py --real-processor --real-pillow` check. That check exercises
the installed Transformers video processor and Pillow decoder with original
synthetic data and no loaded model. Without that environment selection, the gate
uses its normal interpreter and reports unavailable optional dependencies.
The full pre-push gate requires these imports; it fails when they are missing.
Set `JEV_CLEF_PYTHON` to the explicitly prepared environment above to run both
real-media checks with that interpreter. No weights or live requests are required.
The processor probe imports Pillow and constructs `Qwen3VLVideoProcessor` without
weights. A lazy placeholder class for a missing backend therefore does not count
as an available processor.

The explicitly invoked download below transfers about 19.08 GB for Clef Flash;
the separate Clef manifest describes about 54.99 GB. Allow additional space for
download bookkeeping, packages, and runtime allocations. The pinned Clef
configuration has 27,356,728,560 backbone parameters: at two bytes each, BF16
backbone weights alone require about 51 GiB, before the decision head,
activations, media processing, or loader overhead. That unquantized CPU path
cannot fit a 32 GiB host. This is a derived lower bound, not a measured peak or
a guarantee that a larger machine suffices; quantized Ollama/GGUF models have
different requirements. See the [configuration check](clef-live-results/clef-architecture-check.json).
Downloading is separate from ordinary CLI use. Review the selected publisher
Python code before running the bridge.

```sh
target/clef-local/venv/bin/hf download Cloudflare/clef-flash \
  --revision 17f0b0ad64efb65d273590632833508766b2aae6 \
  --local-dir target/clef-local/clef-flash
python3 -B scripts/clef-model-manifest.py \
  --model-path target/clef-local/clef-flash \
  --manifest scripts/clef-local/clef-flash-manifest.json \
  --report target/clef-local/snapshot-verification.json
target/clef-local/venv/bin/python scripts/clef-server.py \
  --model-path target/clef-local/clef-flash --model-name clef-flash \
  --device cpu --dtype bfloat16
```

Hugging Face supports [full commit revisions and explicit local directories](https://huggingface.co/docs/huggingface_hub/guides/download).
The recorded manifests come from its official metadata API at immutable revisions:

| Release | Revision | Manifest |
| --- | --- | --- |
| Clef Flash | `17f0b0ad64efb65d273590632833508766b2aae6` | [clef-flash-manifest.json](../../scripts/clef-local/clef-flash-manifest.json) |
| Clef | `2f3de3dd85f379784083b0814d997ab627200f0c` | [clef-manifest.json](../../scripts/clef-local/clef-manifest.json) |

The offline verifier hashes every recorded file, checks its size, rejects missing
files, extra model files and symlinks, and never executes publisher code. LFS
files use publisher SHA-256; ordinary files use Git blob SHA-1 including the
`blob <length>\0` prefix. Hugging Face's `.cache/huggingface` download bookkeeping
is excluded from the model-file inventory. These checks prove agreement with the
recorded publisher snapshot, not that executing that code is safe.

## Local-provider troubleshooting

As inspected on 2026-10-04, [Ollama issue #18769](https://github.com/ollama/ollama/issues/18769)
reports Clef Flash System One failures on Windows with Ollama 0.35.1, including
CUDA and CPU. [PR #18777](https://github.com/ollama/ollama/pull/18777), then open,
proposes a Windows large-file seek fix. A chat-completions success does not prove
the decision head works. Check the issue and release status before choosing a
Windows version, then test `/v1/systemone` through this harness. Do not hide the
failure by switching endpoints or weakening assertions.

The bounded Linux GPU experiment used the original Flash Q8_0 model on an 8 GiB
RTX 4070 Laptop GPU. Default loading exceeded VRAM. Reserving 4 GiB with the
official `OLLAMA_GPU_OVERHEAD=4294967296` setting allowed the runtime to place
5 of 33 layers on CUDA and the vision projector on CPU after its own startup
fallback. One synthetic PNG with mixed Noul/Choice/Score questions passed through
the CLI. This is partial GPU execution, not full GPU residency or a performance
benchmark; another model, context length, image workload, or occupied GPU may need
different resources. See the [GPU proof](clef-live-results/ollama-flash-gpu-partial.json)
for both allocation failures, successful offload measurements, and cleanup. No
driver or global runtime installation was made.

## Recorded results

The [residuals verification receipt](clef-residuals-verification.json) records that
phase's finish condition, repository gate, independent reviews, cleanup, and limits.
Its reports retain their recorded source bindings. The later
[v0.3.0 publication inspection](../release-verification.md#recorded-v030-release)
verifies the published CLI artifacts separately and ran no provider inference.

The expanded matrix adds a real structured-question request to every provider.
The following recorded runs passed with no retries:

| Provider and model | Passed cases | Reserved requests | Evidence |
| --- | --- | --- | --- |
| Cloudflare Clef | 22 | 27 | [Report](clef-live-results/residuals-cloudflare-clef-smoke.json) |
| Cloudflare Clef Flash | 22 | 27 | [Report](clef-live-results/residuals-cloudflare-flash-smoke.json) |
| Ollama Clef 27B Q4_K_M | 21 | 26 | [Report](clef-live-results/residuals-ollama-clef-smoke.json) |
| Ollama Clef Flash Q8_0 | 21 | 26 | [Report](clef-live-results/residuals-ollama-flash-smoke.json) |
| Python Clef Flash, before publisher input addendum | 25 | 30 | [Report](clef-live-results/residuals-hf-smoke.json) |
| llama.cpp Clef 27B Q4_K_M, text only | 16 | 21 | [Report](clef-live-results/residuals-native-clef-smoke.json) |
| llama.cpp Clef Flash Q4_K_M, text only | 16 | 21 | [Report](clef-live-results/residuals-native-flash-smoke.json) |

Each of the five image runs also agreed with all 32 original synthetic labels:
[Cloudflare Clef](clef-live-results/residuals-cloudflare-clef-quality.json),
[Cloudflare Flash](clef-live-results/residuals-cloudflare-flash-quality.json),
[Ollama Clef](clef-live-results/residuals-ollama-clef-quality.json),
[Ollama Flash](clef-live-results/residuals-ollama-flash-quality.json), and
[Python Flash](clef-live-results/residuals-hf-quality.json).
The [Python phase binding](clef-live-results/residuals-hf-quality-phase.json) records
the exact original loaded bridge and spawned CLI/helper hashes; the new publisher
input support has a separate corrected-bridge addendum.
The [publisher-input phase proof](clef-live-results/residuals-hf-publisher-input-final-phase.json)
assembles 19 validated cases across three explicitly bound phases: arbitrary JSON
states and descriptions, options/levels files, mapping, labelled evaluation, and
all five MCP tools. It preserves the original numeric 255-level request that
exceeded its 600-second CPU deadline. A distinct compact null-description
255-level request passed in about 15 minutes with a 3,600-second deadline.
The original map controller also expected the wrong default row IDs; the
[saved-output reanalysis](clef-live-results/residuals-hf-map-reanalysis.json)
checks the documented IDs and exact state hashes without resending requests.
This is assembled coverage, not one fresh 19-case run or a performance claim.
The [cleanup proof](clef-live-results/residuals-hf-cleanup-settled.json) confirms
the owned Python server stopped and its temporary weights were removed.
The [native 27B proof](clef-live-results/native-clef-cpu-proof.json) records its
separate text sanity check, physical-batch configuration failure and correction,
exact model/runtime identity, and cleanup. No image accuracy is implied by its
text metrics. These synthetic families do not establish production calibration.
The [native Flash proof](clef-live-results/native-flash-cpu-proof.json) records its
matching expanded matrix and [32-row text sanity check](clef-live-results/residuals-native-flash-quality.json),
verified model/runtime hashes, exact CLI/helper phase binding, and cleanup.
Both native models agreed with all 32 literal text labels. These text results
cannot be compared with the five image runs. Concurrent local CPU tests also mean
their elapsed times are not performance benchmarks.

These live reports retain their originally recorded validation scope. Explicit
executed CLI/helper hashes are available for the Hugging Face phases and native
Flash proof; the Cloudflare, Ollama and native Clef residual reports do not record
those hashes. Their executed CLI/helper versions cannot be attested retrospectively.
Subsequent smoke-harness regressions added selected-dataset checks for labels,
types, order, and predictions across all three primitives. That stricter validator
is proven by offline tests; it is not retroactively attributed to the earlier live
runs. Their provider answer-shape checks and the Rust evaluation tests remain
separate evidence.

The [wheel/runtime proof](clef-live-results/python-reproducibility-proof.json)
records fresh hash-checked downloads and offline installation. The
[discovery repair](clef-live-results/skill-discovery-alias-proof.json) records the
three specifically approved local environment aliases and clean strict readiness.

The following earlier synthetic integration matrices passed on 2026-10-04,
before the structured-question case was added. They are historical receipts,
not evidence for the current expanded matrix. Local runs
used Linux x86-64 CPU inference. Request reservations include two resume requests
that a correct implementation does not send; these are not billing totals.

| Provider and model | Tested runtime/model | Passed cases | Reserved requests | Evidence |
| --- | --- | --- | --- | --- |
| Cloudflare Clef | Hosted service | 21 | 26 | [Report](clef-live-results/cloudflare-clef-final.json) |
| Cloudflare Clef Flash | Hosted service | 21 | 26 | [Report](clef-live-results/cloudflare-clef-flash-final.json) |
| Python Clef Flash | torch 2.11.0+cpu, Transformers 5.10.2; pinned publisher weights | 24 | 29 | [Report](clef-live-results/huggingface-flash-final.json) |
| Ollama Clef | 0.35.1; `clef:27b-q4_k_m` | 20 | 25 | [Report](clef-live-results/ollama-clef-final.json) |
| Ollama Clef Flash | 0.35.1; `clef-flash:9b-q8_0` | 20 | 25 | [Report](clef-live-results/ollama-flash-final.json) |
| llama.cpp Clef Flash | b11392; maintainer Q4_K_M conversion; text only | 15 | 20 | [Report](clef-live-results/llama-flash-final.json) |

The [runtime and model proof](clef-live-results/model-runtime-proof.json) records
package versions, immutable revisions, runtime hashes, and fresh streaming hash
checks of the Ollama blobs and native Flash GGUF during verification. The earlier Flash
publisher snapshot check is recorded as historical: its 19 GB weights were
removed after successful inference, so a fresh file hash cannot be claimed.
The separate native Flash test weight was also removed after its hash and
inference checks to restore disk space. Subsequent testing removed the task-owned
Ollama 27B download to make space for native 27B verification; the Flash tag remains.
These reports establish the listed integration paths, not accuracy, calibration,
GPU compatibility, or every quantization and runtime combination.

The [27B Python configuration check](clef-live-results/clef-architecture-check.json)
constructed the pinned backbone on PyTorch's meta device and checked dimensions;
it loaded no weights and ran no inference. The unquantized 27B download exceeded
this machine's available disk space. The
[cross-platform checks](clef-live-results/cross-platform-checks.json) passed for
core/configuration on Windows and macOS targets; whole-workspace checks stopped
at missing native compiler/SDK requirements. Neither OS was executed in that
local inference phase.
The subsequent [security-helper type check](clef-opus-review/cross-platform-security-typecheck.json)
compiled the exact production macOS and Windows file-opening helpers against
their real APIs and the workspace's dependency versions. This narrower proof
does not establish an entire CLI build or native execution on either platform.
The subsequent v0.3.0 release builds and native CLI smoke tests supply separate
CLI evidence, while native model inference on those operating systems remains
unverified. macOS x86-64 was built but had no native smoke test.

The [hosted capacity recheck](clef-live-results/cloudflare-capacity-recheck.json)
observed HTTP 422 / Cloudflare code 5012 for `rejectIfBusy` on both models, with
empty stdout and no fallback. The retained reports do not establish the precise
cause. Successful capacity-rejection coverage remains blocked until the provider
accepts the documented option. Historical hosted observations remain in the separate
[verification receipt](clef-hosted-verification.json). Use a new report for each
new worktree or runtime.

The [Claude Code review and corrections](clef-opus-review-verification.json)
record the later Opus 5.5 medium review of this session, its finding dispositions,
regression checks and fresh GPT-6.1 Sol high skill evaluations. These corrections
have separate source bindings; earlier live reports retain their original recorded
provenance and coverage. No further provider inference was performed in the review
phase. Raw review streams and evaluation answers are retained locally, as the
receipt identifies, rather than included in the source archive.

The later [focused skill follow-up](clef-followup-skill-evaluations.json) records
60 fresh valid attempts across four new cases: the old guide passed 16/20,
the updated guide 20/20, and the common no-guide control 5/20. This disclosed,
frozen comparison used five repetitions per case and variant; its updated-guide
run was development-only, not a standalone paired confirmation. It establishes
neither native skill discovery nor broad reliability. The original 225 attempts,
including 12 execution errors and nine candidate criterion failures, remain
unchanged historical evidence; the follow-up does not regrade them or bind
earlier inference to the updated guide.
