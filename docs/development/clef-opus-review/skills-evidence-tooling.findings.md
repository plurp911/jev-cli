I've finished the review. I found 3 medium-severity defects and 11 lower-severity items. The most important is a broken rule, not a code bug: a held-out grader was edited after its results were seen. No security issue in the eval harness or release tooling was confirmed; three items (#4, #9, #10) are suspicions that need someone to reproduce them. I made no edits and ran nothing. Nothing below was executed, so every regression test is a proposal.

## Prioritized findings

### Confirmed defects

**1. [Medium] A held-out case was edited after its results were seen, which the repo's own rules forbid**
- **Where:** `evals/skills/heldout/urgent-csv-vs-keyword/graders/jev-did-not-fire.md:1-10`. The grader changed from "jev must never fire" to a `skill_order` check after the blind run.
- **Rules it breaks:**
  - `docs/development/skill-authoring.md:296-297`: "never edit a held-out case after seeing its result — retire it".
  - `evals/skills/heldout/README.md:17-18` and `evals/skills/README.md:296-298`.
  - `evals/skills/jev/CLEF.md:44` restates the rule as only "edit a held-out **prompt**", which is narrower and conflicts with the above.
- **Impact:**
  - The original 85/88 result and the separate after-the-fact 88/88 are both honestly labelled. But the repository's held-out set now contains a case whose grading was shaped by seeing its results, so any future "blind held-out" run includes a contaminated case.
  - The file name `jev-did-not-fire` now describes a check that allows `jev` to fire.
  - The receipt field `model_or_source_tuning_on_heldout: false` (`clef-skill-verification.json:145`) is literally true but misleading.
- **Why tests miss it:** nothing pins held-out file hashes once results have been viewed.
- **Minimal fix:**
  - Retire the case: restore the original grader under a `retired/` marker, or move the case to `routing/`.
  - Commission a replacement written blind.
  - Change the held-out count from 30 to 29 in receipts and docs, and align the CLEF.md wording with the repo rule.
- **Proposed regression test:** commit a held-out manifest frozen at first admission. `test-skill-eval-codex.py` asserts every file under `evals/skills/heldout/` matches it or is listed as retired. Editing this grader fails that test.

**2. [Medium] Clef eval graders require a command the CLI rejects**
- **Where:**
  - `evals/skills/jev/behaviour/clef-image/graders/provider-boundaries.md:6` requires `--cloudflare-account-id demo-account`.
  - The prompts at `clef-image/prompt.md:9`, `clef-credentials/prompt.md:9` and `triggers/clef-hosted-image/prompt.md:9` supply `demo-account`.
  - `crates/jev-client/src/endpoint.rs:243-246` rejects any account ID that isn't exactly 32 hex characters (`InvalidCloudflareAccount`, which surfaces as a usage error). The config path does the same at `jev-config/src/settings.rs:302-307`.
- **Impact:**
  - Passing answers recommend a command that fails at once.
  - A correct answer that points out the invalid ID could be marked down.
  - `skills/jev/references/providers.md:14` never states the 32-hex format, so the skill teaches nothing here.
  - The recorded passes for these cases validate the rubric, not runnable commands.
- **Why tests miss it:** these are LLM rubrics, and nothing executes or validates the recommended commands.
- **Minimal fix:** use a valid synthetic 32-hex ID in the prompts and graders. Add the format to `providers.md`. Rerun only the affected cases, keeping the original results.
- **Proposed regression test:** an offline test that pulls every `--cloudflare-account-id` value out of `evals/skills/**` and checks it against the same rule as the client (or runs `jev ... doctor --provider cloudflare` with it).

**3. [Medium] `scripts/verify.sh --push` now fails unless Pillow and Transformers are installed, and this is undocumented**
- **Where:** `scripts/verify.sh` (the patch adds `optional_module` real-decoder and real-processor checks). `optional_module` at lines 74-86 turns a missing module into a FAIL in `--push` mode.
- **Impact:**
  - The required pre-push hook (AGENTS.md §2 rule 7) now blocks every contributor who lacks Pillow and Transformers, or `JEV_CLEF_PYTHON`.
  - Neither `CONTRIBUTING.md` nor `scripts/dev-setup.py` mentions this.
  - `find_spec('transformers')` doesn't check torch or the video processor's other dependencies, so a partial install fails the check instead of skipping it.
- **Why tests miss it:** every recorded gate ran in default mode with `JEV_CLEF_PYTHON` set, never `--push` on a clean host.
- **Minimal fix:** gate the real-media checks on `JEV_CLEF_PYTHON` or an explicit flag, or record the requirement in ADR-0013 and the bootstrap/doctor tooling. Probe the actual processor imports.
- **Proposed regression test:** a `test-dev-tools.py` case that runs verify's helper in push mode with a stub `python3` lacking the modules, and asserts the documented outcome.

**4. [Low–Medium] The shipped `jev` skill says loopback inference always stays on the machine**
- **Where:** `skills/jev/SKILL.md:119, 345, 399` ("A loopback provider stays local"). The privacy grader at `clef-privacy/graders/provider-boundaries.md:6` reinforces it.
- **Impact:** the CLI can only check that the endpoint is loopback, not what the local server does next. An Ollama cloud-offloaded model or a local proxy forwards the content off the machine, and an agent following this guidance could send sensitive data.
- **Status:** the wording overclaim is confirmed. Whether Ollama's System One route can reach cloud models is a suspicion that needs checking.
- **Minimal fix:** say "goes to the local server; it stays on this machine only if that server runs the model locally (not a cloud-offloaded model or proxy)".

**5. [Low] The `clef-video` grader still requires a nonempty state**
- **Where:** `clef-video/graders/provider-boundaries.md:6` says "a nonempty state". The bridge accepts an explicit blank state (`providers.md:53-54`).
- **Impact:** the same stale requirement was already fixed in the `clef-advanced-controls` grader (`:6`) but not here, so a correct `--state ''` answer fails.
- **Fix:** align the wording as a separately reviewed contract change, keeping the original results.

**6. [Low] The routing README's "generated" table is stale, and no generator exists**
- **Where:** `evals/skills/routing/README.md:50,59` still lists `jev` as "asserted not to fire" for `does-it-hold-up-is-a-pilot`. The grader is now `skill_order`.
- **Why it persists:** a grep of `scripts/` found no generator behind the "regenerate it" instruction.
- **Fix:** regenerate the table with a column for ordered dependencies, and add a test that compares the table against the graders.

**7. [Low] Committed receipts point at evidence that isn't in the repository**
- **Where:**
  - `clef-skill-verification.json:108`: the held-out command is `python3 /tmp/jev-final-heldout-reviewed-contract.py`.
  - `clef-residuals-verification.json:117,134,143,174`: `/tmp` gate and review logs.
  - Many `evals/skills/results/...` paths, which are gitignored, and `target/...` paths.
- **Impact:** the source archive ships receipts whose evidence can't be audited, and the held-out run can't be reproduced from the repo.
- **Fix:** commit the wrapper and hashed summaries, or mark each path `local_only`. Add a test that every evidence path is in the repo or explicitly marked.

**8. [Low] `docs/development/clef-research.md:3-5` is stale.** It still says "No live inference request was made and no weights were downloaded", which later sessions contradicted. Qualify it as of the research date.

### Suspicions needing reproduction

**9. [Low] The eval harness treats every turn-attributed `error` notification as fatal**
- **Where:** `scripts/skill-eval-codex.py:763-766, 812-813`. It doesn't check for a retry flag (`willRetry`).
- **Risk:** if Codex emits errors it then retries, the harness counts them as run errors. A 429 would also set `evaluation_blocked` and cancel the whole suite.
- **Fix:** continue when the notification says it will retry.
- **Proposed test:** feed an error with `willRetry: true` followed by a normal completion, and expect a valid result.

**10. [Low] The app-server environment scrub uses a prefix denylist**
- **Where:** `skill-eval-codex.py:684` strips only `JEV_`, `TYPESAFE_`, `OPENAI_`, `AZURE_`, `ANTHROPIC_`, `CLAUDE_` and `CURSOR_`. `GITHUB_TOKEN`, `HF_TOKEN`, `AWS_*`, `CLOUDFLARE_*` and `CODEX_*` pass through, while `code_mode_host` and `unified_exec` are enabled (`:36-37`).
- **Risk:** isolation depends on Codex never exposing execution. The item whitelist only fails after an item has started.
- **Fix:** an allowlist environment (PATH, HOME, CODEX_HOME, locale variables, TMPDIR, certificate variables), plus a test asserting the `Popen` env keys are a subset of it.

**11. [Low] A malformed judge reply excludes the whole run instead of failing it**
- **Where:** `skill-eval-codex.py:886-905`.
- **Risk:** this could inflate pass rates if malformed judge output correlates with weak answers. No candidate judge errors appear in the final evidence, so there is no observed impact.

**12. [Low] `source-snapshot.py:155` runs `git status` without `GIT_OPTIONAL_LOCKS=0`.** It may write to the user's `.git/index` and contend for `index.lock` with concurrent git use. Repo-local filter drivers still run; that config is trusted.

### Methodology limits

- No held-out case covers any Clef behaviour or trigger. The 30 held-out cases are older routing prompts with 68 tool-use checks and no LLM graders.
- The skill reference copies the exact eval scenario:
  - `providers.md:78` contains the exact clef-video values (`fps 30 / 90 / [0,60]`).
  - `providers.md:53-62` is close to verbatim with the publisher-inputs grader.
  - The training passes therefore measure recall of the reference text rather than generalization. Change the examples or add a blind Clef held-out batch, and qualify the claims.
- The grader correction to `baseline-beats-jev` (the calibration target) holds up against its fixture (`team-choice-heldout.json` target 0.9, no criteria file). The `pulse/ingest/worker.py` fixture repair is disclosed (`clef-skill-verification.json:1142`).

### Separately classified
- **`.gitignore`:** a pre-existing user change that ignores local `.claude/skills/{engineering-mode,…}`. Not attributed to this session and not to be reverted. It does mean the workflow skills used in the session aren't reproducible from the repo.
- **External limits, correctly documented:** `rejectIfBusy` HTTP 422, no Windows/macOS/full-GPU/HF 27B runs, synthetic-only accuracy. None are claimed as passing.

## Invariants confirmed by reading the code
- **Eval harness configuration:** it checks the model, effort and provider the server acknowledges; `account.type == chatgpt`; skill inventory, MCP, features and project docs fail closed; events need exact thread and turn IDs; answers must be nonblank with positive usage; cleanup kills the process group; snapshots are hashed before and after each run; the baseline has no Skill tool.
- **Read/Glob/Grep tools:** `..` is refused; symlinks are refused per path component; containment is checked after resolving; git-ignore runs with isolated git config; ripgrep runs with `--no-config` and size and time bounds.
- **`skill_order` grading:** names match exactly, and failed secondary attempts count.
- **Source snapshot:** no-follow opens through directory handles; bounded reads; the reserved receipt name is refused; the build tree is read-only and indexed with `--no-filters`; hooks and global config are disabled.
- **Release rehearsal:** `GIT_*` variables are unset, and manifest paths are canonicalized and contained.
- **Other:** the typos exception is a narrow regex. The `release.yml` change only deletes an obsolete commented block. A grep found no Cloudflare account ID in the receipts.

## Coverage
- **Read in full:**
  - Session context: the full conversation (all 265 messages), session brief, extraction proof, partition proof.
  - Patches 000-006, 009, 075, 127, 130-137, 139, 140, 164, 177, 190, 192, 206, 208, 219-224, 226.
  - Current sources: `skill-eval-codex.py`, `source-snapshot.py`, `providers.md`, ADR-0015, `clef-research.md`, `clef-publisher-input-compatibility.md`, `CLEF.md`, routing README, `skill-discovery-alias-proof.json`, all Clef trigger prompts and graders, and the clef-image/video/privacy/mcp/advanced-controls/publisher-inputs prompts and graders.
- **Read in part:**
  - `release-dry-run.sh` (lines 110-234 plus the patch), `skill-eval.sh` (70-258 plus the patch), `verify.sh` (55-94 plus the patch), `release.yml` (384-413 plus greps).
  - `clef-skill-verification.json` (1-240 plus greps), `clef-residuals-verification.json` (1-185 plus greps), evals README (180-299 plus the patch).
  - `test-skill-eval-codex.py` (1-100 plus all test names); test names only for `test-skill-eval-tools.py`, `test-source-snapshot.py` and `test-dev-tools.py`.
  - Eval evidence: scoped-training-summary (1-892 of 3753), provenance v6 (1-80), tiny-label failure (1-161).
  - Code: `context.rs` 535-564, `endpoint.rs` 235-256, publisher `joint_schema_model.py` 30-139.
- **Not read:**
  - Patches 070-074, 076-126 (the live-result receipts, except the alias proof), 128, 129, and most small grader and prompt patches in 141-189 beyond those listed.
  - Test bodies in 215-218.
  - The 25 prior review logs, `original-v2-review.txt`, `verification.txt`, `before-test.py`/`after-test.py`, the per-record held-out results, the nonblank audit, and the bodies of `docs/clef.md`, `clef-live-testing.md` and `commands.md` (checked by grep only).

These unread items are the remaining risk. No live receipt was checked against its raw run.
