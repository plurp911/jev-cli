I found one new Medium defect, three Low ones, and two places where the earlier reviewers got something wrong. Everything comes from reading the frozen source, patches and captured evidence; I ran nothing, so every regression test below is a proposal. All findings are against the original source, not the separate fix tree.

## Findings, in priority order

### 1. [Medium] Three shipped skills still say every state is sent to TypeSafe
- **Where:**
  - `skills/jev-pilot/SKILL.md:95`: "Whatever goes in `state` is transmitted to TypeSafe."
  - `skills/is-jev-useful-here/SKILL.md:240-244`: "name TypeSafe as the recipient".
  - `skills/jev-opportunity-audit/SKILL.md:201-203`: "adopting it sends that content to TypeSafe".
  - A case-insensitive search of these three files finds no mention of a provider, Cloudflare, Clef or Ollama.
- **Trigger:** a user who has chosen `--provider cloudflare`, as every clef-* eval prompt sets up, asks for a pilot, a fit check or an audit.
- **Impact:** the skill asks the data owner to approve TypeSafe as the recipient when the data actually goes to Cloudflare. For loopback providers it overstates that the data leaves the machine.
  - This exact text was loaded in the blind heldout runs (`final-heldout-20261005T002830Z/heldout-urgent-csv-vs-keyword-with-1/result.json`, the jev-pilot load in the trace). The jev-pilot skill was edited this session (patch 222) without fixing it.
  - The `jev` skill and `providers.md` already handle destinations per provider; these three skills do not.
- **Minimal fix:** make the sentence depend on the provider (TypeSafe by default, Cloudflare for `--provider cloudflare`, the local server for loopback) and point to `skills/jev/references/providers.md`.
- **Regression:** a readiness lint that rejects an unconditional "to TypeSafe" recipient sentence in the shipped skills, plus one training eval: a pilot with Cloudflare must name Cloudflare as the recipient.

### 2. [Low–Medium] The MCP `map` schema blocks per-record video `metadata` that the server accepts
- **Where:** `crates/jev-cli/src/mcp/schema/map.input.json`, `records[].videos[]` (patch 027, lines 233-279; snapshot 040, lines 1828-1875). It allows only `frames`, with `additionalProperties:false`.
- **Server side:**
  - `RecordArgs.videos: Vec<jev_core::EmbeddedVideo>` (`mcp/tools.rs:361`), and the core deserializer accepts `metadata` (`jev-core/src/media.rs:215-225`).
  - The CLI's `--videos-field` path uses the same type (`commands/map.rs:712`).
  - The top-level `videos[]` schema does include `metadata`.
- **Trigger:** an MCP host that validates against the schema sends a per-record video carrying its source `fps`.
- **Impact:** the call is refused before it reaches the server, or the agent concludes timing isn't supported. Per-record videos then fall back to sequential default timestamps.
- **Why tests miss it:** `tests/mcp.rs` only sends top-level video metadata (patch 037, line 65).
- **Minimal fix:** add the same `metadata` subschema to the record-level videos, then update the snapshot.
- **Regression:** an MCP `map` call whose record carries `videos[0].metadata.fps = 30` must produce a request body containing it. Also add a test that validates every test call's arguments against the advertised `inputSchema`.

### 3. [Low] Schema lower bounds disagree with server validation
- In all five input schemas, `fps`, `duration` and `media_kwargs.fps` use `"minimum": 0`, while the descriptions and server require strictly positive values ("0 < fps", `jev-core/src/media.rs:78`).
- **Fix:** use `exclusiveMinimum: 0`. **Regression:** a snapshot assertion on that keyword.

### 4. [Low] `legacy_preflight` treats an empty selection as success
- **Where:** `scripts/skill-eval-codex.py:1001-1008` returns 0 when no case matches (a typo, or the wrong partition). `scripts/skill-eval.sh:165` then stages and execs `claude plugin eval` with the operator's filter.
- **Impact:** if the Claude harness matches `--case` differently from fnmatch on the frontmatter name, a `skill_order` case can run without a grader for it.
- **History:** Grok v1 and v2 named this mechanism, but the v3 fix changed only the test (`after-test.py:70-76`), not the function.
- **Fix:** return 2 when nothing is selected. **Regression:** `legacy_preflight(['--case','no-such-case']) == 2`.

### 5. [Low, evidence] The stated cause of Cloudflare error 5012 isn't backed by retained evidence
- `docs/clef.md:60-62` says "the `options` field is not permitted by their input validation", and `clef-live-testing.md:365-366` says "Cloudflare must resolve that model-validator limitation".
- The receipts record only HTTP 422 and code 5012 (`cloudflare-capacity-recheck.json`, `residuals-capacity.json`), and the client echoes only the numeric code. `residuals-capacity.json` mentions two "diagnostic_requests", but what they returned is not retained.
- **Fix:** state the observation without the cause, or keep a sanitized provider message.

### 6. [Low] The Windows and macOS `jev-cli` file-open code has never compiled
- `jev-cli/src/media.rs:302-320` (macOS aliases) and `:363-400` (Windows reparse-point walk) are covered by no build: `cross-platform-checks.json` type-checked only `jev-core` and `jev-config`.
- The first compile would happen in the `release.yml` matrix (lines 171-173). The docs are accurate ("core/configuration"). This adds that the security-relevant open path is not even type-checked, not just unexecuted.

## Corrections to the earlier reports
- **Local-bridge F5 should not be adopted as written.** Asserting `confidence == max(probabilities)` for every provider would wrongly fail Ollama.
  - In `ollama-flash-gpu-partial.json:88-108`, Choice confidence is 0.9209 while the top probability is 0.9903; Score confidence is 0.6603 while the top probability is 0.9369.
  - Only the publisher's HF bridge defines Score confidence as the maximum (`joint_schema_model.py:540`, observed in `residuals-hf-score255-null.json:33` and `:292`).
  - Gate any such check on `huggingface`. The weighted-score tolerance must grow with the number of levels: up to 1.619 for 255 levels, per `residuals-hf-score255-null.json:25`. The docs already handle this correctly (`output-schema.md:82`, `clef-research.md:121-125`).
- **Local-bridge's "bridge `3bfb653b…` is consistent across phases" is wrong for the HF 32-image quality phase.**
  - That phase loaded bridge `91ba413e…` (21,630 B), CLI `1eafe029…`, `clef-live.py` `ad5bbdf3…` and `clef-quality.py` `84c59f23…` (`residuals-hf-quality-phase.json:21-55`). This is disclosed in `clef-residuals-verification.json:65` and `clef-live-testing.md:291-293`.
  - The frozen snapshot receipt confirms the frozen bridge is `3bfb653b…` (`.jev-source.json:3760-3762`), which closes that reviewer's hashing limitation, provided the receipt itself is accurate.
  - The frozen `clef-live.py` is `14744f34…` (`.jev-source.json:3712-3714`), which matches no live phase's helper. So `"same_live_helper_still_on_disk": true` (`residuals-hf-publisher-input-final-phase.json:312`) was only true at the time of that receipt.

## Additional impact of the heldout grader edit (distinct from the retirement under way)
- The edit also blocks the whole-heldout legacy run: `legacy_preflight(['--heldout'])` now returns 2 (`test-skill-eval-codex.py:86`).
- `test_existing_corpus_is_read_without_changing_cases` (patch 216, lines 286-296) only counts cases (`len(heldout) == 30`) and never pins file bytes. Retirement will need to update it, and it would not catch another post-hoc edit.
- `skill_order` passes an empty trace (patch 216, line 327), so the separate pilot-fired grader must stay.

## Methodology note
- The baseline arm has no Skill tool (`clef-skill-verification.json:1031-1043`). All 15 baseline heldout passes are stay-away cases: typesafe-invoice, refactor-billing, jev-pricing-free-tier, find-old-postgres-fix and checkout-ab-test, three repetitions each.
- Baselines score zero on positive routing cases by construction. Don't read candidate-versus-baseline as a quality difference; the receipts don't claim one.

## Checked and found consistent
- **Cloudflare smoke receipts:** 21 or 22 cases reconcile with 26 or 27 reserved requests in both the original and residual matrices. Case counts for every other provider also match `clef-live.py:179-237`.
- **HF publisher-input assembly:** 19 validated cases, 40 successful plus 1 failed request (41 reserved), 103 in total including smoke and quality. The chain of prior-report hashes links across all three phases.
- **Map reanalysis:** per-row input tokens sum to the reported 1,139.
- **Publisher manifest:** 17 files in the manifest versus 15 in the proof; the gap is exactly `.gitattributes` plus `README.md` (11,792 bytes).
- **Cleanup:** only about 243 MB showed as freed right after deletion; the settled receipt (`residuals-hf-cleanup-settled.json`) records 30.19 GB free once the filesystem reclaimed the blocks.
- **Heldout tallies, from all 180 records:** 88 valid candidates with 85 passes (the 3 non-passes are exactly `urgent-csv-vs-keyword-with-1..3`), 82 valid baselines with 15 passes, and 2 + 8 errors matching the budget-error audit.
- **Training summary:** 309/308 candidates, 294/72 baselines.
- **Final gate log:** 879 and 878 nextest passes with 11 skipped, 5 third-party skill skips, 50, 21 and 22 harness tests, 26 real-decoder and 28 real-processor tests.
- **v3 partition test:** sound. Exit 2 cannot come from an empty selection.
- Earlier Grok defects (bridge invocation drift, `.jev-source.json` check, eval label binding, blank-answer guard, unknown features, quota substring matching) are closed by the test bodies I read.

## Coverage audit

**Read in full:**
- **Rust:** patches 024, 027, 028, 029, 036, 037, 038, 040 and 067. Current `tools.rs:300-389` plus targeted greps of `jev-core/src/media.rs`, `jev-cli/src/media.rs:280-430` and `map.rs`.
- **Receipts:**
  - Full: 070, 071, 073, 074, 076–083, 085–088, 091–094, 098–107, 120.
  - The `final-phase` receipt in full.
  - Model-runtime proof: lines 1–185 plus a hash grep.
  - Score-255 receipt: head, tail and a grep of every probability.
  - `residuals-hf-smoke`: head and tail.
  - Remaining residual smoke and quality receipts: key fields by grep.
- **Docs:** patch 128, 129. `clef-live-testing.md` 40-80 and 240-369, `clef.md` 50-69 and 220-249, `clef-research.md` 78-134, `clef-residuals-verification.json` 1-130, `clef-skill-verification.json` 104-163 and 1030-1054 plus greps.
- **Eval cases:** prompts and graders for clef-local-setup, clef-credentials, clef-publisher-inputs (`case.yaml`, `compatibility.json`, `scaffold.sh`), every clef trigger, and the clef-image-command and clef-video-command routing cases.
- **Tooling test bodies:** 215, 216, 217, 218. `skill-eval.sh` 50-258; `legacy_preflight`.
- **Review history:** all 25 Grok logs, `original-v2-review.txt`, `verification.txt`, `partition-regression-proof.json`, and the before/after tests.
- **Heldout and training evidence:** `budget-error-audit.json`, one full heldout record, grep tallies over all 180 records, and the headers of the nonblank audit and training summary.

**Limitations:**
- **Binary fixtures 064–066:** I could only see the git binary-patch sizes (632, 78 and 62 bytes) and a few tests. `media.rs` asserts 2×3 pixels and MIME type for all three; the fixture README says they were generated with Pillow. No visual or byte-level inspection was possible.
- **Before/after tests:** I compared lines 68-89. The 546/547 line counts suggest the rest is identical; I did not check that line by line.
- **Not read or only partly read:**
  - patch 126 (`clef-verification.json`);
  - lines 185-468 of the model-runtime proof (grep only);
  - patches 089/090 (Ollama finals, via the proof summary only), 095–097 and 108–119 (greps only);
  - the full body of `clef.md` (patch 072);
  - most of the 2,359-line skill verification (patch 125);
  - the provenance v6 file and the tiny-label failure;
  - the middle sections of the final gate log;
  - every heldout trace beyond the tallies.
  - I did not re-read the 265-message conversation, which the earlier reviewers read in full.
- **Hashes:** I could not hash any file, so every hash comparison relies on the retained receipts, including `.jev-source.json`.
- **Spilled output:** one search spilled its output into a home-directory file, which I deliberately did not open.
