I've finished reading everything in scope. Here is the review.

# Final review of source-final → source-complete corrections

**Verdict:** the code and evidence are sound, on one condition. The final same-source full gate has not been retained yet: `final-complete-gate-artifacts/` does not exist in the workspace. I make no claim that it passed. Root must finish it and update the receipt before claiming the work is complete.

## 1. The `exclusiveMinimum` guard extension: justified

This adds a feature-compatibility keyword and tightens the checks. It does not silence a test.

- **The red run was real and narrow.** In `failed-final-gate-artifacts/verify.log`, the only failure in both the default run (lines 929–946) and the no-keychain run (lines 1928–1945) is `every_schema_is_an_object_rooted_portable_subset`. It failed on exactly `media_kwargs.fps`, `videos[].metadata.duration` and `videos[].metadata.fps` using `exclusiveMinimum`.
- **An independent check passed in the same failed gate.** The MCP Inspector `tools/list --strict` reported no portability warnings (line 2794).
- **The change is right.** JSON Schema 2020-12 §6.2.3 makes `exclusiveMinimum` a number, and MCP 2025-11-25 tools use 2020-12 by default.
- **The guard is stronger, not weaker.**
  - `server.rs:388-404` adds only that one keyword. The unknown-keyword rejection and the single-string `type` rule are unchanged.
  - `server.rs:417-419` rejects the draft-4 boolean form.
  - The new test at `:437-459` checks the accepted case, the boolean rejection and the unknown-keyword rejection. Each assertion is exact.
  - The all-schema root test is byte-identical.
  - The independent zero/positive check (`check-request-schema.py:83-130`, log line 2232) and the record-level `mcp.rs:1504-1514` assertion remain.
- **Green proof:** `portable-guard-green.txt` passes both tests. That is a targeted run, not the full gate.
- **Nit, optional:** `minimum`/`maximum` still have no numeric-type check. That asymmetry predates this change.

## 2. Doc-residue corrections: verified

- **MCP `map` budget docs match the code.** `mcp.md:282-289`, `threat-model.md:420-426` and the ADR addendum all describe what `tools.rs:825-846` does: serialized states plus decoded media bytes, with template media counted once. The CLI JSONL/`--lines` difference and the exclusion of `--image`/`--video-frame` files are stated accurately.
- **ADR 0012** labels its original bounds as the 2026-09-23 decision and adds a dated addendum. The historical text is preserved.
- **Recipient wording:**
  - `INSTRUCTIONS` now says "images and videos", and a new lifecycle assertion checks it (`mcp.rs:309-314`).
  - The `mcp.md` and threat-model residue now names the selected recipient and includes the loopback/offload caveat.
- **The `is-jev-useful-here` restoration is text-identical** to `source/skills/is-jev-useful-here/SKILL.md:244-247`, including "one sentence, not a section". The before/after hashes in the doc-residue proof match the `independent_six` → `fit_restoration_three` epoch change.
- **Nit:** in `SKILL.md:246-247`, "It is the item most often dropped" now follows the provider-reference link sentence, so "It" reads as if it means the link. Consider "Privacy is the item…". This is wording only; the fit-restoration run passed.

## 3. Fresh eval evidence: arithmetic reconciles exactly

I summed all five phases in `skill-evaluations.json` and checked them against `all207-fresh-independent-audit.json`:

- **Attempts and errors:** 207 attempted, 197 valid, 10 errors. All 10 errors are baseline `max_turns` errors (7 + 3).
- **Candidate records:** 108 attempted and valid, 102 strict passes, 108/108 tool contract, 63/69 body.
- **Usage:**
  - 363 judge observations: 3 per LLM-graded record. That works out to 61 + 18 + 9 + 15 + 18 body records.
  - Measured tokens 5,882,073; cached 3,746,432; judge tokens 2,177,209.
  - Partial error tokens 432,674. The individual minima sum correctly. For example, the sparse without-1 record's last total of 46,019 equals its recorded minimum.
- **Residual records:** all six candidate residual hashes match the audit. All phases show `configuration_mismatches: []`.

**The six candidate failures, read in full (responses and every vote):**
- **`credentials-with-1` (2–1 fail):** the answer never says `JEV_API_KEY` is for TypeSafe. That is an explanatory omission, and the grade is fair.
- **`privacy-with-3` (2–1 fail):** the answer says "no cloud key" instead of "no credential or dummy key", and doesn't explicitly require agreement for a remote host. Also an omission, and fair.
- **Parcel with-1 and with-2 (2–1 each):** no CPU timeout policy.
- **Sparse with-1 and with-2 (3–0 each):** no 600-second timeout.

**New finding A [Low–Medium, evidence qualification and skill gap].** The four independent failures turn on a 600-second value that is in neither the prompt nor the evaluated guide.
- The sparse prompt only says "One slow CPU request is fine; retries are not". The parcel prompt has no timeout at all.
- The candidate `0eb5…` skill bytes are the only thing the measured model could read (the trace's `Glob **/*` returned empty). They contain no `--timeout`, `--retries` or 600. The only 600 in `skills/` is an unrelated `DEFAULT_MAX_CHARS` constant.
- That guidance exists only in `docs/clef.md:168-184`.
- Both failing sparse answers explicitly say the timeout and retry flag syntax could not be verified. The passing rep 3 offered 600 as an example.
- So 4 of the 14/18 are better described as a rubric expectation that neither the prompt nor the evaluated guide stated, which also exposes a real gap: `providers.md` doesn't tell agents to use `--timeout 600 --retries 0` with a CPU bridge. They are not evidence that the model ignored the guide.
- **Actions:**
  - Add that cause note to the receipt's interpretation, with no regrade.
  - If you add the flags to `providers.md`, label it as informed by blind answers. Any later rerun of those cases is then no longer blind.

**Qualification B.** On `independent_six` body criteria, the baseline passed 5/6 valid records and the candidate 5/9. The baseline's strict 3/15 is low mainly because the baseline arm has no Skill tool, so its trigger checks fail by construction. Don't present the six-case batch as a body-quality gain.

**Stale receipt fields (must fix before any completion claim):**
- `clef-opus-review-verification.json:195` (`fresh_corrective_skill_evals: "pending"`) and `:198` (`"(pending)"`) contradict `skill-evaluations.json` "All admitted phases complete".
- `:215` still says "Final independent generalization pending".
- I accept that these stay pending until root compiles the final proofs.

**Scope statements are right:** per-phase source bindings, the warning that the combined counts don't attest one source, and "no measured gain" for privacy 27 (all 9 records in each arm pass). Historical 30 / prospective 29 counts are untouched.

## 4. Native runtime inventory

- **The arithmetic is right.** It has 39 entries, which sum to 71,257,904 bytes, and `llama-server` (17,864 B) needs `libllama-server-impl.so`. Your correction stands: `native-clef-cpu-proof` does record model and runtime hashes, and only CLI/helper hashes are missing.
- **Qualification C.** Ten of the 39 entries are symlinks. 71,257,904 B counts the targets of the 10 symlinks again. There are 29 regular files totalling 43,118,224 unique bytes. Say "39 entries (29 files)" at `verification.json:278`.

## 5. Publisher links, the CF5012 label, the lazy-backend probe and the benchmark

- **Publisher links:** the manifests that `docs/clef.md` now links to exist.
- **CF5012:** the mock at `client.rs:1617-1618` is explicitly labelled synthetic.
- **Lazy-backend probe:** the script, version (Transformers 5.10.2) and result agree. Scope: it hides the backends by patching `find_spec` rather than uninstalling them, and the result JSON says so. The final doctor run shows the processor check `ok`.
- **Benchmark addendum:** stderr, sub-millisecond resolution and the missing baseline log are all disclosed.

## Coverage

- **Read in full:**
  - The index and every hunk of `completion-corrections.diff`. The `original-authoring/` copies were checked against `admitted/` through the manifest and author-note hashes, not line by line.
  - The guard decision, its diff and green log.
  - `server.rs:375-472`; `tools.rs:819-860`; `mcp.rs:1490-1535`.
  - The doc-residue proof, all-207 audit and skill-evaluations receipt.
  - The probe script and result, the benchmark addendum and the doctor output.
  - The independent-six prompts and graders for sparse and parcel.
  - All six candidate failure verdicts and votes, plus the four independent answers and the credentials and privacy answers.
- **Partial:**
  - The remaining 201 raw records: hashes via the audit, two error/pass records spot-checked by grep.
  - `verify.log` from the failed gate: grep only.
  - The current `docs/clef.md`, `mcp.md` and `threat-model.md` outside the changed hunks.
- **Unread:**
  - `fresh-skill-evaluations-before-fit-restoration.json`, `all207-final-skill-evaluations.json`, `runtime-releases.json`, and the full `native-runtime-inventory.json` in session context. I relied on the committed copy in the diff.
  - Unchanged earlier implementation, as you instructed.
- **Not possible here:** any hash recomputation or execution.
