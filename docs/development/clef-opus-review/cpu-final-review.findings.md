# Narrow review: CPU guidance correction (source-complete → source-cpu-final)

**Verdict:** I accept the correction, with conditions. The new CPU guidance is accurate against the source and fixes my earlier finding A. The new training case adds coverage without weakening any existing check, and the 107→108 / 18→19 count change is an honest corpus extension. **The work isn't complete yet:**
- `cpu-final-gate.log` and `cpu-final-gate-proof.json` are missing.
- `all225-final-skill-evaluations.json` and `cpu18-fresh-audit.json` are missing.

So I can't say the gate passed or reconcile the new 18 outcomes, and I'm not inferring success for either. Completion depends on root keeping a green same-source full gate and the new 18 receipt and audit. The earlier 899/898 gate predates these changes.

## Claims checked against the source

All of these hold:
- **Per-attempt timeout, maximum 3600:**
  - `context.rs:50` sets `MAX_TIMEOUT_SECONDS = 3600`.
  - `:330-332` refuses a larger `--timeout` with a usage error. The proof records 3601 → exit 2.
  - `cli.rs:118` documents "Per-attempt HTTP timeout … maximum: 3600".
  - `http.rs:68` applies `timeout_global` to each request.
- **`--retries 0` means one attempt:**
  - `context.rs:247-256` turns the flag into `max_retries`.
  - The total budget is `max(default, timeout × (retries+1))`, so a 600-second single attempt isn't cut short.
- **Serialized bridge, no active cancellation:**
  - `clef-server.py:23,348` uses a plain single-threaded `HTTPServer`.
  - `:449` has the comment saying one handler at a time serializes model access.
  - `:429` calls `infer(...)` synchronously and nothing checks the socket during it.
  - `:378-379` silently drops the write error if the client has disconnected.
  - So a client timeout stops the waiting but not the PyTorch work, and later requests queue behind it. This matches `providers.md:57-64` and `docs/clef.md:183-188`.
- **"600 is illustrative, not a CPU default":** correct. The default is 10 seconds (`context.rs:36`).
- **"The CLI does not launch or stop the runtime":** consistent with the existing `providers.md:54-55`.

## New case and count change

- **No existing files changed.** The `.jev-source.json` hunks only add the three `clef-cpu-timeout` files and update the hashes of the five files you named. No existing prompt or grader under `evals/` changed.
- **Turn limits:** the other clef cases still use `max_turns: 8`. The new case uses 12, which matches the existing `checks-before-it-sends`.
- **Test change is minimal:** only the two count assertions changed, with a comment. The `runs == 3` and non-empty-graders checks are still in place. The proof includes the red (108 != 107) and green runs.
- **The rubric is fair to the guide.** Every point it requires is in what the measured model can read:
  - the flags, per-attempt meaning, 3600 maximum, no cancellation, serialization, and no launching: `providers.md:52-64`
  - no dummy key: `providers.md:53`
  - the offload/proxy caveat: `SKILL.md:119-120` and `providers.md:25`
  
  The FAIL conditions target real hazards: invented lifecycle commands, claims that a disconnect cancels the work, cloud fallback, and key requests.
- **README disclosure:** clear. The guidance is described as informed by the blind results, the original four failures stay failures, any replay is not blind, and the authoring files are unchanged.

## Native runtime inventory: verified

I recomputed it from the committed entries:
- 39 entries, of which 10 are `"symlink": true`.
- The symlinks add 2 × (931,232 + 55,184 + 6,406,584 + 4,780,144 + 1,896,696) = 28,139,680 bytes.
- 71,257,904 − 28,139,680 = **43,118,224** unique bytes in the 29 regular files. ✓
- The limitations section still says no historical loaded-library attestation is claimed.

## Issues to fix

1. **Must fix before completion: stale "current" binding in the receipt.**
   - In `clef-opus-review-verification.json`, `source_phase_binding.current_production_and_guidance_files` still lists `skills/jev/references/providers.md` at `be16807c…`. The frozen source is now `6daaddb1…`.
   - It also doesn't bind the new case files or `test-skill-eval-codex.py`.
   - `full_gate` (exit 0, 899/898, `final-complete-gate-artifacts`) doesn't say it predates the CPU change. Only the `followup_review` text hints at that.
   - Fix: when the new gate lands, rebind to the cpu-final hashes, and label the old gate as pre-CPU or replace it.
2. **Low: one test claim in the proof can't be traced.**
   - `cpu-guidance-training-correction-proof.json` reports `"source_contract": "PASS CPU policyflags/bounds/cancel semantics/…"`.
   - No committed test refers to the new paragraph or the new case. I searched `scripts/test*`, `crates/**/tests` and the whole source tree; only `providers.md` and the manifest match.
   - So this was an uncommitted, one-off check, and the only lasting regression guard for the paragraph is the LLM case itself.
   - Fix: say that in the proof, or name the script and command. Committing a small text-pin test is optional.
3. **Optional nits:**
   - The rubric requires mentioning the 3600 maximum, which the prompt doesn't ask about. The guide does state it, so this is acceptable but strict.
   - `docs/clef.md:187-188` says the same flags apply to `jev mcp serve`, but the new skill paragraph omits that.

## Coverage

- **Read in full:**
  - The index and every hunk of `cpu-final-corrections.diff`, including the verification JSON, the committed copy of my completion review, the inventory and the README.
  - The three new case files and the CPU proof JSON.
  - `providers.md:35-79`.
  - `clef-server.py:346-450`.
  - `client.rs:120-319`.
  - The `context.rs` timeout and retry logic (via grep plus `:28-52`) and the `cli.rs` flag help.
  - All inventory entry sizes and symlink flags.
  - `ProviderPrivacyGuides`.
- **Partial:** `docs/clef.md` (grep only), the RetryPolicy stop behaviour at `max_retries=0` (not read line by line), and the `max_turns` values across the other behaviour cases.
- **Unread or absent:**
  - The cpu-final gate artifacts and the all225 and cpu18 receipts, which don't exist yet.
  - The 207 raw results, which are unchanged and were reviewed earlier.
  - Unchanged implementation outside the files above.
- **Not possible here:** hash recomputation, execution, or evaluating the new answers.
