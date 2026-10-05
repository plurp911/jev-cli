I found no defect in N1–N6 that would block shipping. What remains is three small doc/skill items and some limits on the evidence.

This was read-only static review. I ran nothing and checked no hashes. The final full gate and the GPT-6.1 Sol high evals are still running, so I make no claim about their outcomes. My comparison of source-fixed-v2 to source-final relies on the supplied `late-corrections.diff` (all 1,595 lines read), checked against the current files.

## Findings

**1. [Low] User docs still describe the old MCP `map` limit.**
- `docs/mcp.md:282` says the limit is "Total state across records … The same ceiling `jev map` applies to a whole input".
- `docs/threat-model.md:420-422` and `docs/adr/0012-mcp-server.md:126-127` say "states summing to at most `--max-input-bytes`".
- The code now counts states plus decoded image/video bytes, with template media counted once (`tools.rs:825-846`). The corrected N6 comment says the CLI's units differ.
- **Fix:** update the `mcp.md` table row and the threat model. Mark the ADR as historical or amend it.

**2. [Low] Docs outside the three skills still name TypeSafe as the only recipient.**
- `docs/mcp.md:337-338` says the server instructions mean "state goes to TypeSafe". The actual `INSTRUCTIONS` text (`mcp/server.rs:28-32`) names the configured endpoint.
- `docs/threat-model.md:434` still says "Every state is transmitted to TypeSafe."
- This is the same kind of problem as coverage-gaps finding 1, but the new lint (`test-skill-scripts.py:1473-1494`) only checks three skill paragraphs.
- Nit: `INSTRUCTIONS` mentions "State and explicitly supplied images" but not videos.

**3. [Low, worth deciding now] The `is-jev-useful-here` edit goes beyond naming the recipient** (`SKILL.md:240-248`).
- It removed "It is the item most often dropped…".
- It loosened "On a NO or WEAK verdict this is one sentence, not a section" to "can be brief".
- Both can affect the existing `definite-no-stays-brief/stays-short` and `privacy-flagged` graders. Decide on intent now, not from the pending eval answers.

**Info:**
- The three skills link `providers.md` by an absolute GitHub `main` URL (`is-jev-useful-here:246`, `jev-pilot:102`, `jev-opportunity-audit:208`). That page won't have this content until it is published, and those eval cases only allow Read/Glob/Grep. This matches the existing style in `providers.md:38-40`.
- The `check-request-schema.py` docstring (lines 2-15) still describes only the request schema.
- `client.rs:1619` is a synthetic mock reply containing "options: Extra inputs are not permitted". It is not evidence of the CF5012 cause.

## N1–N6

- **N1 (lazy processor):**
  - `verify.sh:85` and `dev-setup.py:65-66` now construct `Qwen3VLVideoProcessor()`. The red log fails as expected and the green log passes 46/46.
  - The actual Transformers probe JSON shows torch and torchvision hidden, the import succeeding, and construction raising `ImportError`. That supports the fix.
  - Limits:
    - Only the result JSON was kept. There is no probe script, Transformers version, or record of how the backends were hidden.
    - The regression test uses a synthetic stub class (`test-dev-tools.py:74-97`), not Transformers' real placeholder class.
    - No kept run exercises the new top-level import-and-construct path in the complete environment. Indirect evidence: the real-processor suite constructs the same class with no arguments (`test-clef-server.py:648`) and passed 42/42.
- **N2 (whole-clip text):** the schema and `providers.md` wording matches both enforcement paths: the bridge (`clef-server.py:285-309`) and the Rust preflight (`client.rs:654-723`).
- **N3 (provider switch notice):**
  - The flag is set from a saved endpoint plus a resolved default (`context.rs:361-362`). The note is static text sent through `warn` (`commands/mod.rs:168-170`), so it never includes the URL and `--quiet` suppresses it.
  - Tests cover a switch to another provider, a switch back to TypeSafe, `--quiet`, an explicit `--endpoint`, and a matching provider.
  - Only a summary receipt was kept; there are no raw red/green logs.
- **N4 (bridge header reset):**
  - `QuietHTTPServer.handle_error` swallows only `Exception`. Python's `socketserver` re-raises anything else after shutting the request down, so KeyboardInterrupt and SystemExit still propagate. A real HTTP test proves this.
  - The red log (`stderr` not empty) and green logs (42 and 32 tests) are consistent.
- **N5 (config alias):** `settings.rs:294` and `context.rs:437` both accept `llama-cpp`, and the endpoint match uses the normalised provider. Only summary evidence was kept.
- **N6 (budget comment):** the comment is accurate. Two nits: the CLI cap applies to the whole input in `--lines` mode too, and `--video-frame` files are also excluded. `dev-setup.py` now follows PEP 8.

## Schemas and tests

- All five input schemas use `exclusiveMinimum: 0` for source `fps`, `duration` and sampling `fps`. They have no `$schema`, so the latest draft applies. Runtime agrees (`jev-core/media.rs:78,140-146`).
- `map` record-level `metadata` is identical to the template-level copy (`map.input.json:304-338`).
- The new `mcp.rs` test checks that the two copies are equal, that `exclusiveMinimum` is present, and that the metadata reaches the request body. Only a green log exists for it; the record-level red is covered by the schema-script red log (line 16).
- `check-request-schema.py:83-128` validates the good case before the zero-value cases, so a rejection can't be caused by some unrelated error. Not implemented: the suggestion to validate every MCP test call's arguments against the advertised `inputSchema`. The script's synthetic cases cover this only partly.

## Benchmark (previously unread, now closed)

- `preflight-benchmark.rs` is sound, and exact sizing is independently pinned by `client.rs:1077-1193`.
- Issues with the output file:
  - The `after_stdout` label is wrong, because `eprintln!` writes to stderr.
  - "0 ms" at millisecond resolution only means under 1 ms.
  - The baseline has no raw log or source revision.
  - The kept source copy isn't hashed against the path recorded in the JSON.

## Cross-platform check

- The extracted helpers in `lib.rs:2-123` match `media.rs:279-400` line for line.
- Rustix features match the workspace `Cargo.toml:62`, and all seven locked versions and checksums match the workspace `Cargo.lock`. The macOS log ("Locking 7 packages") shows the harness lock was resolved fresh rather than copied.
- On Windows, rustix is a Unix-only dependency, so only the crate itself was checked.
- As you said, this proves only that the extracted production helpers compile. The callers, the full CLI and any execution are unproven, and the JSON says so.

## Coverage-gaps findings (all six)

1. **Skills recipient:** addressed with three skill edits, a lint, and three training cases whose results are pending. Docs residue remains (findings 1–2 above).
2. **Record-level metadata:** addressed.
3. **Strict lower bounds:** addressed in all five schemas.
4. **Empty legacy selection:** addressed (`skill-eval-codex.py:1058-1060`, with a test).
5. **CF5012 cause:** addressed in `clef.md:71-73`, `clef-live-testing.md:370-372` and `CHANGELOG:119-120`. The historical receipts are not in the change set. Old wording survives only in earlier review reports.
6. **Windows/macOS file-open code:** honestly bounded by the helper type-check.

Initial gate (`verify.log`): 893 and 892 nextest passes, and six fuzz targets built and run for 10 s each ("All fuzz targets ran clean"). The image-header fuzz path is inside `request_document`. This gate predates the late corrections.

## Not read, or only partly read

- **Rust tests:** `tests/mcp.rs` and `provider_cli.rs` beyond the new tests and their helper names; the four non-`map` schemas beyond grepping their bounds and the `noul` header; the snapshot file (relied on its green log).
- **Python tests:** the full `test-skill-scripts.py` and `test-skill-eval-codex.py`.
- **Evidence and logs:** `final-corrections.jsonl` and `remaining-session-evidence.jsonl` (grep only); the eval-evidence trees; the middle of `verify.log` (grep only); the initial `connection-probe` and `coverage-gaps` outputs contained only `REVIEW_READY` and an API error.
- **Docs:** the full `clef.md`, `clef-live-testing.md` and `CONTRIBUTING.md` beyond the changed hunks.
