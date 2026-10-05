# Review of the implemented corrections from the three Opus 5.5 medium reports

I read the code only and ran nothing. I did not assume any result from the full `--push` gate or the GPT 6.1 Sol 21-case evals, which are still running. The diff is `source` → `source-fixed-v2`. I checked it against the current files for every Rust, Python, fuzz and eval-harness change, and they match.

**Bottom line:** almost every correction is real and, as far as I can tell from reading, correct. The whole-clip patch math in Rust and Python matches. I found one claim that is probably wrong (N1, the push-gate import check), two documentation gaps, and a few small leftovers. I found no regression in credential handling, loopback, or held-out evidence.

## New findings

**N1 [Low–Medium, likely; needs one run to confirm] The push-gate import check probably doesn't detect a partial Transformers install**
- **Where:** `scripts/verify.sh:78-86`, `scripts/dev-setup.py:62-71`, and the doc claims at `docs/development/clef-live-testing.md:208` and `CONTRIBUTING.md:50`.
- **Trigger:** Transformers is installed but torch or torchvision is missing. Transformers' lazy loader usually hands back a placeholder class when a backend is missing. If it does that here, `from transformers import Qwen3VLVideoProcessor` succeeds, and the error only appears when the class is used.
- **Impact:**
  - Default `verify.sh` would run the real-processor test and report FAIL instead of `skip`.
  - The developer doctor would report the processor as `ok`.
  - The docs say a partial install "does not count as an available processor", which would be untrue.
  - The new test (`test-dev-tools.py`, the `partial` stub) fakes the import itself failing, so it can't catch this.
- **Fix:** have the check actually construct `Qwen3VLVideoProcessor()` (no weights needed), or check torch/torchvision availability explicitly. Then rerun in a venv that has transformers but no torchvision.

**N2 [Low] The skill and MCP schemas don't explain the corrected video pixel units**
- **Where:** `skills/jev/references/providers.md:95-99` and the `media_kwargs` descriptions in `crates/jev-cli/src/mcp/schema/*.input.json` (for example `noul.input.json:266-277`).
- **Impact:** agents read these, not `docs/clef.md`. Neither says that video `max_pixels`/`min_pixels` cover the whole clip including temporal padding, or that explicit values are capped at 64M ÷ the number of media items.
  - Example: `{"max_pixels":262144}` with 32 frames gives about 8,192 pixels per frame. An agent would assume 262,144 per frame.
- **Fix:** add one sentence to each. This is a skill text change, so it should be reviewed separately and kept apart from the evals that are running now.

**N3 [Low] A saved endpoint is now ignored without any notice**
- **Where:** `crates/jev-cli/src/context.rs:533-541`.
- **Trigger:** an older config has `endpoint = "http://127.0.0.1:11500"` but no `provider`, and the user runs `--provider ollama`.
- **Impact:** that used to reach 11500; it now goes to the default 11434 with no message. The change is intended, is in the CHANGELOG, and is safe for credentials. It is still a silent change in behaviour.
- **Fix:** print a one-line stderr note when a saved endpoint is skipped because the provider differs.

**N4 [Low, left over from bridge F3] One traceback path remains**
- **Where:** `scripts/clef-server.py:346-442`. There is no `handle_error` override.
- **Trigger:** a client sends a reset while the bridge is still reading the request line or headers. The resulting `ConnectionResetError` reaches socketserver's default handler, which prints a traceback.
- **Impact:** a traceback on stderr, though no request bytes are echoed. The CHANGELOG wording ("decoder or response-writing tracebacks") is accurate because it is scoped to those paths.
- **Fix:** override `handle_error` to print nothing.

**N5 [Info] Renaming `llama-cpp` to `llamacpp` changes the map resume fingerprint**
- **Where:** `provider_fingerprint` in `crates/jev-cli/src/commands/map.rs:151` includes `endpoint.provider()`.
- **Impact:** `--resume` would refuse llama.cpp map runs started by an earlier build. The llama.cpp provider is still `[Unreleased]`, so this only affects development artifacts.
- **Related leftover:** the config file still doesn't accept the `llama-cpp` spelling, although the CLI flag alias does (part of Rust #6).

**N6 [Info / style]**
- **Different units in the input limit:** the MCP map limit counts decoded media bytes, plus template media. The CLI `jev map` counts base64 text in its JSONL input, which is about 4/3 larger, and doesn't count `--image` template files. The code comment says the two limits match; they don't quite.
- **Style:** `dev-setup.py:62-71` uses the compact single-quote one-liner style. The rest of that file is formatted in standard PEP 8 style, so the new code doesn't match.

## Items you asked me to check

- **Whole-clip limits:**
  - Python (`clef-server.py:287-309`) and Rust (`client.rs:654-723`) both enforce 16,000,000 pixels per frame and 64,000,000 in total.
  - Default clip maximum is 25,165,824; default image maximum is `min(16M, item budget)`, below 16,777,216.
  - Explicit controls are capped at 16M (`media_options` and the jev-core validation that `request.rs:144` describes) and at 64M ÷ the number of media items.
- **Patch math parity between Rust and Python:**
  - Both round with ties to even (`round` / `round_ties_even`).
  - Downscaling divides `w / divisor / 32` in the same order, which is the case the 680×1105 red-then-green test exercises.
  - Upscaling uses `sqrt(min / area)` with `ceil`.
  - Both pad odd frame counts to even and check every count from 2 to `max(len, num_frames or 32)` when sampling. The numbers in the floats are well below 2⁵³, so f64 matches Python ints exactly.
  - I hand-checked the four Rust allocation cases and the three real-processor grids.
  - My reasoning about the processor's even rounding of `num_frames` suggests the range check never underestimates. That is not executed.
- **Exact-sizing preflight:**
  - Sizing serializes `""` placeholders and adds `ceil(n/3)*4` per image.
  - This is exact because serde_json doesn't escape `/` or `+`, and the image field order (`content_type`, `base64`) matches `EmbeddedImage::serialize`.
  - The bytes actually sent are unchanged.
  - The body-size cap is applied for Cloudflare, Hugging Face and Ollama, with tests at exactly the cap and at cap + 1 that make zero attempts.
  - Every preflight caller goes through `media::preflight`.
  - The benchmark source lives outside the repo (under `/home/...`), so I could not inspect or reproduce it.
- **Fuzz target:**
  - The API compiles as far as reading shows (`EmbeddedImage` has a hand-written `Debug`, and `MAX_IMAGE_*` is re-exported).
  - The three seeds are byte-identical copies of real fixtures; all three display as valid images.
  - `fuzz-smoke.sh` copies the seeds into the corpus.
  - Honest scope: the real test is that the `imagesize` header parsers never panic. The bound and round-trip assertions mostly repeat what the constructor already checks.
  - It does not reach the data-URL and MIME paths, the CLI's native-image parser, or the bridge's Python `dimensions()`.
  - No fuzz build was run after root added this path.
- **Endpoint configuration:** same provider and missing-provider-means-TypeSafe behave correctly; tests cover all five providers. Doctor exits 0 and auth status exits 3. The new path reloads only `Settings` and never touches credentials or sockets. Explicit provider errors still exit 2.
- **MCP aggregate counting:** the 78-byte fixture against a 150-byte limit proves template media is counted (without it the total is about 87 bytes).
- **Inference flags on admin commands:** refused before the configuration loads.
- **Error causes and animated WebP:** the JSON-kind visitor keeps specific error causes. The animated-WebP check uses the same byte offset (20) as the bridge.
- **Eval harness:**
  - `willRetry` is checked only for errors tied to the current turn.
  - The environment allowlist test matches exactly.
  - A malformed judge reply is now a distinct error with usage kept, the confirmation fails, and expected/attempted/valid/error counts are reported.
  - `GIT_OPTIONAL_LOCKS=0` is set on every git call.
  - An explicitly selected interpreter that fails is never turned into a skip.
- **Skills and held-out set:**
  - The 29-case manifest is validated before discovery, by an offline test on the real directory.
  - The case moved to routing with byte-identical graders.
  - The historical receipts, including 85/88 strict and the separate 88/88 post-hoc, are not in the change set.
  - Where the README and the verification receipt describe the manifest, they call it prospective, not retroactive proof of blind authorship.
  - The routing table is generated, and a test checks it matches the graders.
- **Documentation:**
  - `--timeout 600 --retries 0` is documented, and so is the fact that an inference already running can't be cancelled.
  - There is no EOF-based cancellation, and the write-half-closed test proves those clients still get their response.
  - Clef research is now dated to the initial research phase.
  - The `/home` symlink case is documented as an intentional no-follow boundary with a relative-path workaround.
  - The new `clef-opus-review` supplement marks the historical raw evidence paths (`/tmp`, `target/`, `evals/skills/results`) as `local_only`.

## Initial findings: accepted or rejected

- **Bridge (local-bridge-media):**
  - **F1:** accepted and fixed by documenting whole-clip units. The alternative, keeping `max_pixels` per frame, was reasonably not chosen.
  - **F2:** accepted as documentation. The peek-and-cancel idea is correctly rejected.
  - **F3:** fixed, except N4.
  - **F4, F5, F6:** fixed.
  - **F7:** documented as intentional.
- **Rust (rust-cli-providers):**
  - **#1–#5 and #8:** fixed.
  - **#6:** mostly fixed (see N5).
  - **#7:** the content-type mismatch case is fixed. The "oversized data URL reported as invalid base64" claim is correctly rejected: the old raw-string path checked length first and already reported 4 MiB.
- **Skills (skills-evidence-tooling):**
  - **#1, #2, #4–#9, #11, #12:** fixed.
  - **#3:** fixed, but see N1.
  - **#10:** fixed. Unverified risk: on Linux, Codex keyring auth may need `DBUS_SESSION_BUS_ADDRESS`/`XDG_RUNTIME_DIR`, which aren't allowlisted. That would fail closed (an auth error, not a leak).

## Coverage

- **Read in full:**
  - `corrections-index.json` and every `corrections.diff` hunk, except the three archived findings copies, which I only spot-checked against `outputs/`.
  - The three initial findings files.
  - All five receipts and proofs.
  - The bridge red and final-green logs.
  - Current `clef-server.py`.
  - Current `client.rs:360-840`, `context.rs:270-610`, `commands/mod.rs:300-450`.
  - jev-core `media.rs` (`EmbeddedImage` section).
  - The fuzz target, `fuzz/Cargo.toml` and `fuzz-smoke.sh`.
  - `skill-eval-codex.py` `run`, `evaluate_run`, `report` and `legacy_preflight`.
  - `verify.sh:40-96`, `dev-setup.py:20-79`, `clef-live.py:90-159`, and `providers.md:40-110`.
- **Not read:**
  - The full bodies of the test files outside the diff hunks.
  - The full `clef-live-testing.md` and `CONTRIBUTING.md`.
  - `outputs/{connection-probe,coverage-gaps}*`, `bridge-red.log`, `bridge-green-first.log`, `bridge-green-second.log` and `bridge-stdlib-green.log` (I read the HTTP-red, rounding-red and final-green logs).
  - The eval-evidence trees.
- **Not checked:** any hash (I can't execute). Whether the root's six new blind cases exist outside the source; none are in it, so they couldn't have been changed here.
