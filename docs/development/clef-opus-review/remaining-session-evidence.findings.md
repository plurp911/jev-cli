I found three Low defects, three evidence or claim gaps, and confirmed the rest of the claims you listed. The most important gap is that no retained evidence supports "new 21 evals, 61/63 with two omissions", and no retained full gate covers `source-final`. This was read-only review: I ran nothing and hashed nothing, so every hash comparison relies on the receipts.

## Findings

### 1. [Medium, evidence] The 21-eval result and a `source-final` full gate have no retained evidence
- **The eval claim:** "61/63" appears only in my prompt. No result directory, summary, hash or per-case omission record exists in the workspace.
- **The current receipt says otherwise:** `source-final/docs/development/clef-opus-review-verification.json:3` says `"corrections_and_verification_in_progress"`, and lines 124–126 list `full_gate`, `fresh_corrective_skill_evals` and `followup_review` as `"pending"`.
- **The only retained post-review gate predates the late fixes:**
  - `first-gate-artifacts/retention.json:2` says "later fixes require final rerun".
  - Its packaged `CHANGELOG.md:110` still carries the old "because their input…" 5012 wording, which `source-final/CHANGELOG.md:119` replaced.
  - That gate passed every stage (893 default and 892 no-keychain tests, 11 skipped, fuzz clean, MCP Inspector ok, artifacts match `retention.json`), but on an earlier tree.
  - The 45 later paths in `late-corrections-index.json` have no retained full gate, including `clef-server.py`, `verify.sh`, `skill-eval-codex.py`, the MCP schemas and three skills.
- **Fix:** before claiming either result, record in the receipt the result path, candidate and evaluation hashes, model and effort, per-case scores, the two omissions verbatim, and `forced_retry:false` / `grader_edit:false`. Then run and retain a full gate on `source-final` and set those three fields from evidence.

### 2. [Low, docs] `clef.md` names a publisher revision nothing else pins
- `docs/clef.md:146` (`source-final:157`) says the inspected publisher implementation was revision `a20e258b`.
- That revision appears nowhere else. Every manifest and receipt pins Clef to `2f3de3dd…` and Flash to `17f0b0ad…` (`clef-live-testing.md:242-243`, `clef-publisher-input-compatibility.md:9-10`, `providers.md:66`, `clef-manifest.json:4`, `model-runtime-proof.json:142,418`).
- **Fix:** cite the pinned revisions or link the manifest table. A docs-drift test could assert that every HF revision in `docs/` and `skills/` appears in a `scripts/clef-local/*-manifest.json`.

### 3. [Low, evidence] "Original live receipts retain old helper hashes" is only partly true
- The claim is at `clef-residuals-verification.json:312`.
- Only the HF phase files and `native-flash-cpu-proof.json:32-45` record helper or CLI hashes.
- These contain no `sha256`, helper, binary or revision field at all:
  - `residuals-cloudflare-{clef,flash}-{smoke,quality}.json`
  - `residuals-ollama-{clef,flash}-{smoke,quality}.json`
  - `residuals-native-clef-*.json` and `native-clef-cpu-proof.json`
- So those 10 runs aren't tied to an executed binary or helper anywhere in retained evidence. The receipt is frozen, so scope the statement in the opus-review supplement.

### 4. [Low, evidence] The runtime proof cites a file that doesn't exist, and its `llama-server` hash may be a launcher
- `model-runtime-proof.json:443,452` cite `runtime-releases.json` as release evidence. No such file exists in `source`, `source-fixed`, `source-fixed-v2` or `source-final`.
- The `llama-server` hash at `:445-446` (`d7cedb07…`) covers only 17,864 bytes. This is a suspicion I couldn't check: a file that small is probably a launcher whose code lives in shared libraries that weren't hashed. If so, "fresh-streaming-sha256" doesn't attest the code that ran inference.
- **Fix:** commit or mark `local_only` the release evidence, and hash the loaded `libllama`/`libggml*` libraries too.

### 5. [Low] The unestablished 5012 cause survives in a test fixture and an unmarked receipt
- `source-final` docs (`clef.md:71-72`, `clef-live-testing.md:370-372`, `CHANGELOG.md:119`) now correctly say the cause isn't established.
- But `client.rs:1619` still contains a fabricated Cloudflare body, `fieldErrors.options: "Extra inputs are not permitted"`. Readers can mistake it for an observed response. Use a neutral body or add a "synthetic, cause not observed" comment.
- `clef-residuals-verification.json:157` ("upstream behavior prevents…") has no `historical`/`superseded` marker in the file itself. Only the supplement lists it as unchanged history.

### 6. [Info] Tiny-label wording
- Behaviourally the answer declined a threshold, but the `refuses-to-hand-over-a-threshold` criterion **failed** 3/3 votes, because the model never tied the refusal to per-class sample size (`genuine-tiny-label-failure.json:37-57`).
- The receipt's `failed_criteria` (`clef-skill-verification.json:2148-2151`) is accurate. Avoid summaries that pair "safely refuses threshold" with 1/3 in a way that implies that criterion passed.

## Checked and consistent
- **Coverage totals:** 798 records, 399/397 candidate, 399/376 baseline, 374 pairs, 23+2 errors.
- **Training (provenance v6):** sources sum to 103 cases; 15 baseline errors across 11 rows give 294 valid; tiny-label is the only candidate row below 3/3, giving 308/309. No older-candidate row read a changed body; the newest pilot readers all use `a4a`.
- **Tiny-label record:** acknowledged `gpt-6.1-sol`/high, 3 judges, score 1/3. The "73 error-free rows" Wilson figure is arithmetically right. The enabled features match the receipt's allowlist exactly: 5 legacy flags plus `code_mode_host` and `unified_exec`.
- **Your framing holds in the evidence:** old 85/88 strict blind and separate 88/88 post hoc are kept apart; `source-final` retires the edited case to `routing/` and freezes 29 (`admission-manifest.json:4`); baseline zero on positive routing is structural, from the missing Skill tool (`clef-skill-verification.json:1039-1043`), not a quality gap.
- **Live receipts:**
  - Original and residual smoke runs reconcile: case counts plus reserved requests, retries 0.
  - All seven quality reports are 32 rows (8 positive / 24 negative) with agreement 1.0.
  - The score-255 rounding bound (1.6193) and delta (0.0254) are correct, and HF confidence equals the top probability.
  - The 15 HF publisher files plus 11,792 bytes give exactly 19,083,377,402.
- **`source-final` fixes:** the video `max_pixels` units fix is present (`clef.md:221-235`), and the "transmitted to TypeSafe" wording is gone from shipped skills.
- **Binary hashes are phase-specific and disclosed:**

  | Artifact | Bytes | SHA-256 |
  | --- | --- | --- |
  | Release binary | 6,644,216 | `94fdfd…` |
  | Native-Flash `jev-test` | 6,644,152 | `7f2b1b…` |
  | Gate-15 dist binary | 5,998,456 | `0bdb0c…` |
  | First-gate dist binary | 6,015,672 | `f8b09b…` |

## Coverage and limits
- **Read in full:**
  - Initial `clef-verification.json` (historical), `clef-residuals-verification.json`, all 2,353 lines of `clef-skill-verification.json`, `clef.md`.
  - `model-runtime-proof.json` in full, both Ollama finals, every residual smoke report, `residuals-hf-token-inspection.json`, both native CPU proofs.
  - `source-final`'s `clef-opus-review-verification.json`, `clef-hosted-verification.json` and `clef.md:150-239`.
  - The three initial findings and the gap findings, and both artifact proofs.
  - Every non-test section of `first-gate-artifacts/verify.log`, plus the head of the test run. I confirmed the PASS lines total exactly 1,785 = 893 + 892.
- **Partly read:**
  - Quality reports: all metric fields by grep; one read whole.
  - Score-255 null: lines 1–40, 280–309, 540–564.
  - Tiny-label failure: lines 1–311 and 1016–1095, 1200–1289, 1800–1848; the rest is duplicate protocol items, covered by grep.
  - Provenance v6: lines 1–300, 380–440, 1700–1860, plus whole-file grep tallies.
  - Gate-15 log: summary lines only.
- **Not read:**
  - The 265-message conversation (only targeted greps). The earlier reviewers read it in full.
  - Heldout traces beyond the prior tallies.
- **Not verifiable here:** whether the `8c6e38…` candidate's `providers.md` matches the final bytes. No retained file list exists for that candidate.
- **Out of scope:** nothing here proves every provider, GPU or CUDA path, OS, or production calibration, and I don't claim it does.
