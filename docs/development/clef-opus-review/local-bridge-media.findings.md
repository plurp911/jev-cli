I've reviewed all 18 assigned files. I found one medium-severity behaviour defect and one medium-severity resource and usability defect, both in the Hugging Face bridge path, plus several low-severity issues. None of them leak credentials or let a request break out of loopback. I could not execute anything, so every reproduction below is reasoned from the code and, where noted, from the repo's own tests.

## Findings, by priority

### F1 [Medium, confirmed] Video `max_pixels` limits the whole clip, but the bridge and docs treat it as a per-frame limit
- **Where:** `scripts/clef-server.py:253-271`, `docs/clef.md:203-207`, `crates/jev-client/src/client.rs:587-616`.
- **Trigger:** the bridge works out a per-frame budget (`per_frame_budget`, 64M pixels divided by the frame count) and passes it as the video processor's `size.longest_edge`. The Qwen3-VL video processor applies that number to the whole clip (frames × height × width), not to each frame.
- **Proof from the repo's own test:** `test-clef-server.py:516-522` sends two 64×64 frames with `max_pixels: 4096` and asserts `video_grid_thw == [[1,2,2]]`, which means each frame came out at 32×32.
  - If the limit were per frame, nothing would shrink: 64×64 is exactly 4096, so the grid would be `[1,4,4]`.
  - The asserted `[1,2,2]` only happens if 4096 is the budget for both frames together.
- **Impact:** videos silently get lower resolution than the docs and the publisher's defaults suggest.
  - 32 VGA frames (about 9.8M pixels) are capped at 2M pixels in total, roughly 290×218 per frame. The publisher default (25,165,824 total) would not shrink them at all.
  - The documented example `{"max_pixels":262144}` with two frames gives each frame half of that.
  - An explicit `min_pixels` is also applied to the whole clip for video.
  - This makes inputs smaller, so it does not weaken the memory bound.
- **Why tests miss it:** the test asserts the shrunken grid without checking it against the documented meaning. `test_resize_controls_cannot_amplify…` (line 199) also bakes in the per-frame model.
- **Minimal fix:** compute a separate total budget for video `size` (for example the number of padded frames × the per-frame value, capped by the remaining 64M budget and by the publisher's 25,165,824). Then either keep the user's `max_pixels` as a per-frame value and scale it, or document it as a whole-clip value. Mirror the change in Rust preflight.
- **Regression test:** in the real-processor suite, two 64×64 frames with a per-frame `max_pixels` of 4096 should give `[1,4,4]`; a default 32-frame 512×512 clip should not be shrunk.

### F2 [Medium, reproduction recommended] Retries and the default timeout pile up abandoned work on the single-threaded bridge
- **Where:** `clef-server.py:366-385`, `jev-cli/src/context.rs:36`, `retry.rs:94-104`.
- **Trigger:** the CLI defaults to a 10 s timeout and 2 retries, and a timeout counts as retryable (`retry.rs:492`). The bridge handles one request at a time, has no inference deadline, and never checks whether the client is still connected.
  - On CPU (about 60 s per image request in this session), the example at `docs/clef.md:156` times out after about 32 s.
  - The bridge then still runs up to three queued copies. Each request's body is already in the socket buffer, so it gets read and processed even though the client has gone.
- **Impact:** several minutes of wasted CPU, and the bridge stays blocked for any later requests.
  - Each abandoned request also fails when the bridge tries to send its response (`BrokenPipeError`). The 500 fallback write fails too, so a stack trace is printed (see F3).
- **Why tests miss it:** every live harness run used `--retries 0` with a 120–3600 s timeout, and `clef.md` never mentions timeouts.
- **Minimal fix:** document `--timeout 600 --retries 0` for the bridge on CPU. Better: before running inference, peek the socket (`select` plus `recv(MSG_PEEK)`) and skip requests whose client has closed. Optionally, don't retry timeouts for local loopback providers.
- **Regression test:** a bridge test that opens a connection, sends a request, closes it before the handler runs, and asserts `infer` is never called.

### F3 [Low, confirmed by reading] Some error paths escape the bridge's handler and print stack traces
- **Where:** `clef-server.py:145-162`, `:295`, `:369`, `:382-385`.
- **Trigger 1:** decode errors are caught only as `(ValueError, OSError, RuntimeError)`. Pillow raises `SyntaxError("broken PNG file (chunk b'…')")` from `load()` when an IDAT is truncated and followed by an invalid chunk header.
  - The pre-decode header check (`dimensions()`) and `Image.open` both pass; the failure comes inside `load()` via `load_read` → `PngStream.read`. `SyntaxError` is not in the caught set.
- **Trigger 2:** the client disconnects before the response is written (F2), giving the `BrokenPipe` failure above.
- **Impact:** the client gets no HTTP response, which looks like a transport error and may trigger retries. socketserver prints a stack trace to stderr. That contradicts `docs/threat-model.md:34-35` ("errors never echo … stack traces"), and the `SyntaxError` text includes 4 bytes from the request.
- **Why tests miss it:** the fake-PIL tests only raise the bomb error or a size mismatch, and the real-Pillow tests only cover the bomb and duplicate-header cases.
- **Fix:** in `decode_image`, convert any non-`BadRequest` exception into `BadRequest`. Wrap the `do_POST` body in a final catch-all that sends a 500 and ignores write errors. Override `handle_error` to print nothing.
- **Regression test (`--real-pillow`):** a 2×3 PNG whose IDAT is a partial zlib stream, followed by `b"\0"*8`. Expect HTTP 400 `{"error":"request rejected"}` and empty stderr.

### F4 [Low, confirmed] Running the harness CLIs without `--account` or `--endpoint` crashes with a traceback
- **Where:** `scripts/clef-live.py:249` and `:659`; `clef-quality.py` uses the same `configuration()`.
- **Trigger:** argparse stores `account=None`, so `config.get("account", "")` returns `None` and `re.fullmatch` raises `TypeError`. `main` only catches `(Failure, OSError, ValueError)`, so you get a traceback and exit 1 instead of the fixed refusal and exit 2.
- **Why tests miss it:** they pass dicts that leave the key out entirely.
- **Fix:** use `config.get("account") or ""` and catch `TypeError`.
- **Regression test:** run the script as a subprocess with `run --provider cloudflare` and no `--account`; expect exit 2 and no `Traceback`.

### F5 [Low] Live-harness answer checks would miss inconsistent answers
- **Where:** `clef-live.py:108-134`.
- **Gap:** nothing checks that `choice` is the most likely option, that `confidence` equals the highest probability, or that `score` equals the weighted sum of level × probability (the publisher defines all three). A renderer bug that swaps `choice` would still pass the smoke matrix.
- **Fix:** add those three checks, with a tolerance for the publisher's 4-decimal rounding.

### F6 [Low, suspicion] Rust and bridge validation disagree on animated WebP
- An animated WebP passes Rust preflight for `--provider huggingface` but the bridge rejects it (`clef-server.py:98`).
- The user then sees a generic "huggingface rejected the request; check server configuration" message (`client.rs:343-347`).
- **Fix:** refuse animated WebP in Rust for the bridge path.

### F7 [Low, cross-boundary, needs a repro on such a distro] Image paths under an OS-owned symlinked directory are refused on Linux
- `jev-cli/src/media.rs:324-361` opens every path component with no-follow, and the only exception is the macOS root aliases.
- On distros where `/home` is a system symlink (for example Fedora Silverblue's `/home → var/home`), every absolute image path under `$HOME` would be refused. Relative paths still work.
- Refusing symlinks is intentional and documented; the problem is that only the macOS exception exists.

### Hardening and information (no defect)
- **Installed-package check:** `clef-python-profile.py` compares package names and versions, not installed file bytes. The docs say so. Checking each package's install RECORD hashes would make "installed bytes match the locked wheels" a real guarantee.
- **Model-code integrity:** the bridge never re-verifies the model manifest at startup, so verification and loading are separate steps. The executed `joint_schema_model.py` is pinned only by its Git blob SHA-1. Second-preimage attacks on SHA-1 are infeasible, but an optional `--manifest` check at startup with a locally recorded SHA-256 would close the gap.
- **`rejectIfBusy`:** the client sends exactly the shape the cited Cloudflare page describes. Code 5012 is consistent with Clef's strict input schema rejecting a top-level `options` field. I couldn't check this externally, so it stays an upstream limit, not a client bug.
- **Slow 255-level Score case:** CPU compute limit, not a code bug. It does show F2's mechanics: the bridge kept computing after the client timed out.

## Invariants that hold
- **Loopback only:** loopback bind, a strict Host allow-list, any `Origin` header refused, `application/json` required, OPTIONS returns 405, and one `Content-Length` with no `Transfer-Encoding`.
- **Body parsing:** 13 MB body limit, bracket depth of 64 checked before parsing, duplicate keys and non-finite numbers rejected.
- **Media checks before decoding:** headers for every image and frame are checked before any decode. The decoder must agree on format and size before `load()`. Decoded metadata is cleared.
- **Video metadata:** Rust and Python agree on the metadata checks, the frame and video counts, and the pixel and byte totals.
- **Offline loading:** offline and telemetry environment variables are set before importing torch, and no `.pyc` files are written.
- **`max_state_tokens`:** the bridge's composition matches the publisher's `systemone` probability semantics.
- **Model manifest verifier:** rejects path traversal, symlinks and untracked files.
- **Harness reports:** they keep no CLI stderr and no key paths. The quality tool keeps disagreement as a measurement. Strict ID, label, type and order checks hold in both the smoke and quality harnesses.

## Not actually run (from the receipts and transcript)
- The Hugging Face bridge on CUDA. This is the script's **default** `--device cuda` path.
- `float16` and `float32` dtypes.
- Unquantized Clef 27B (BF16) on any device.
- Full-GPU Ollama or llama.cpp; only 5 of 33 Flash layers were offloaded, with the vision projector on CPU.
- llama.cpp vision (unsupported upstream).
- Native Windows or macOS execution.
- A successful Cloudflare `rejectIfBusy` (only mocked).
- A real-processor run at the maximum media limits (4 videos × 32 frames).
- `max_state_tokens` on 27B.
- The current `clef-live.py` validators (those were tightened after model inference finished). The earlier receipts were produced by the older helper.

## Coverage
- **Read in full:**
  - All 18 assigned patches. Each is a whole new file, and the line counts match the frozen source.
  - The current source of every assigned file: `clef-server.py`, `clef-live.py`, `clef-quality.py`, `clef-model-manifest.py`, `clef-python-profile.py`, the 4 lock files, the wheels and profile JSON, both manifests, and all 5 test files.
  - The session brief, the extraction proof, and all 265 conversation messages.
  - ADR-0015 and the compatibility doc.
  - The publisher's `joint_schema_model.py`, `processor_config.json` and `joint_head_config.json`.
  - `jev-core/src/media.rs` and `jev-cli/src/media.rs`.
- **Read in part:**
  - `client.rs` lines 150-668 and `retry.rs` lines 60-205.
  - `request.rs` lines 150-430.
  - `wire.rs` decoding (lines 42-122).
  - `threat-model.md` lines 1-60, `clef.md` lines 120-260, `clef-live-testing.md` lines 100-240, `clef-research.md` lines 60-110.
  - `clef-residuals-verification.json` lines 1-200.
  - `verify.sh` lines 178-197.
  - The addendum evidence JSON, plus bridge-hash grep results across the evidence files.
- **Not read:**
  - `source/AGENTS.md` as a file. I relied on the identical copy embedded in the transcript.
  - `clef-skill-verification.json` (one grep only).
  - The 25 prior Grok logs and the eval-evidence folder (outside my area).
  - `full-session.diff` as a whole; I used the per-file patches instead.
  - Other Rust CLI and MCP code beyond the media and provider mapping.
- **Limitation:** without running anything I couldn't hash the frozen `clef-server.py` to confirm it matches the live-tested bridge. Its SHA-256 is recorded as `3bfb653b…` (21,689 bytes) consistently across the evidence phases.
