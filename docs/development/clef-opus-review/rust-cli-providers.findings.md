# Review: Rust/CLI/providers scope (62 assigned paths)

I found no Critical or High defects. The credential split holds:
- TypeSafe keys only reach the official SystemOne endpoint (`endpoint.rs:228`).
- Cloudflare and remote local servers use the custom key namespace.
- Local servers on loopback skip credential lookup and the authorization header, enforced twice (`commands/mod.rs`, `client.rs` `send`, `http.rs`).

The findings below come from reading the code only. I ran nothing, so every regression test listed is a proposal.

## Findings, in priority order

### 1. [Medium] A configured `endpoint` setting silently applies to every `--provider` — confirmed by reading `crates/jev-cli/src/context.rs:521-575`
- **Code:** `resolve_endpoint` takes the endpoint from `--endpoint` or the config file, then uses it for every provider. It wraps it with `with_cloudflare_account`, `with_ollama`, `with_huggingface` or `with_llama_cpp` regardless of which provider was chosen.
- **Trigger:** `jev config set endpoint https://corp-proxy.example` (set up for TypeSafe), then `jev --provider ollama noul …` or `jev --provider cloudflare --cloudflare-account-id X noul …`.
- **Impact:**
  - "Ollama" traffic goes to the remote proxy instead of `127.0.0.1:11434`, together with `JEV_CUSTOM_API_KEY`.
  - Cloudflare traffic goes to `https://corp-proxy/client/v4/accounts/X/ai/run/…` with whatever custom key is set. If that key is a Cloudflare token, the proxy receives it.
  - The guide (`docs/clef.md:13-25`) says local providers default to loopback and that only "`--endpoint` overrides the base URL". The config-file path is not mentioned there or in `cli-contract.md`.
  - The usual stderr warning is shown, so this does not technically break AGENTS §4 (the user wrote the file). It does defeat the privacy expectation of choosing a local provider.
- **Why tests miss it:** `provider_cli.rs` covers a config-file `provider` and a flag `--endpoint`, but never a config-file `endpoint` combined with a provider chosen by flag.
- **Minimal fix:** only inherit `settings.endpoint` when the provider itself came from the config file (or the provider is `typesafe`). Otherwise use the provider's default, or refuse with a usage error that asks for an explicit `--endpoint`. Document whichever rule you pick.
- **Regression test:** write `config.toml` with `endpoint = "https://proxy.example.com"`. Run `jev --provider ollama noul ? --state x --dry-run -o json`. Assert `url == "http://127.0.0.1:11434/v1/systemone"` and `credential.source == "anonymous"`, or assert exit 2. Add the same test for Cloudflare.

### 2. [Low–Medium] The MCP `map` input limit no longer counts media, unlike the CLI — confirmed by reading `crates/jev-cli/src/mcp/tools.rs:820-835`
- **Code:** `records()` adds only `check_size(&record.state)` to `total`. The comment says it matches the CLI's whole-input limit.
- **Contrast:** the CLI's `jev map` reads JSONL through the byte-capped reader, so embedded base64 counts toward `--max-input-bytes`.
- **Impact:** 100 records × 8 MiB of images (about 800 MiB decoded) pass the MCP limit check, and up to 100 large hosted requests can follow. Each image is still bounded, so memory stays proportional to the input; this is an unbounded aggregate, not amplification.
- **Fix:** add each record's media size (`images` and `videos` `byte_len`) plus the template media once to `total`, or set a separate aggregate MCP media cap. Then update the comment.
- **Regression test:** an MCP `map` call with 3 records of about 3 MiB images each and `--max-input-bytes 4194304` should be refused with a usage error and zero mock hits.

### 3. [Low, performance — the cost estimate needs measuring] Preflight rebuilds every row's full request body before anything is sent
- **Where:** `commands/map.rs:560-580`, `commands/eval.rs:84-92`, `mcp/tools.rs:750-760`.
- **Code:** for each row, `preflight` calls `build_evaluation_request`, which base64-encodes and serializes the template images in full.
- **Trigger:** `jev --provider cloudflare --image 4MiB.png map --lines --input` with about 100k short lines. That means about 100k × 5.6 MB of serialization before the first request, then the same work again while sending.
- **Fix:** run the full preflight once on the template. Per row, check only what varies (state, row media, and the body size, computed or bounded from the template body length).
- **Proof needed:** a benchmark or timing test with a large template image and many rows.

### 4. [Low] The CHANGELOG omits stricter input handling that TypeSafe users will hit — confirmed
- **Code:**
  - `input.rs:295` (`json_state`) now uses `parse_unambiguous_value`.
  - `ordered.rs` `Field::to_value` now rejects duplicate keys at any depth inside question content.
  - `request.rs:725` does the same for levels files.
- **Impact:** `jev noul ? --state-json '{"a":1,"a":2}'` used to send `{"a":2}`; now it exits 2. `docs/commands.md:64-66` documents this, but the CHANGELOG "Fixed" entry only mentions media kwargs, batch/eval rows and labels. AGENTS §10 asks for an entry when a stable input behaviour changes.
- **Fix:** add a CHANGELOG line covering `--state-json`, `--state-json-file`, request-document state and question content, and options/levels files.

### 5. [Low] Media and provider flags are silently ignored on commands that never use them — confirmed
- **Code:** `validate_provider_flags` only checks provider compatibility.
- **Trigger:** `jev --provider cloudflare --image x.png --reject-if-busy models` exits 0, and nothing reads the image or sends the option. The same happens with `doctor`, `auth` and `config`.
- **Why it matters:** this goes against the project's own "refused rather than ignored" reasoning (`reject_unknown_top_level_keys`). MCP serve is the only command that refuses these flags.
- **Fix:** reject the media and option flags for commands other than inference and `mcp serve`.

### 6. [Low] Stale or inconsistent user-facing text — confirmed
- `cli.rs:70`: the `--image` help says "(Cloudflare or Ollama)", but `huggingface` is also accepted.
- `dataset.rs:234`: the unknown-field error still lists only `schema`/`id`/`state`/`labels`, though `images` and `videos` are now accepted.
- `endpoint.rs:275` reports the provider as `"llama-cpp"`, which appears in `client.rs:345` errors. The CLI and the config file use `llamacpp`, and the config does not accept the `llama-cpp` spelling that the CLI flag accepts.

### 7. [Low] Ollama image errors lose their specific cause — confirmed by reading `media.rs:417`
- **Code:** the untagged `NativeImage` falls back from a typed image to a raw string.
- **Impact:**
  - An oversized but valid data URL is reported as "image base64 is invalid".
  - A typed object with a mismatched `content_type` gives serde's generic "did not match any variant".
- **Fix:** branch on the JSON kind (string or object) explicitly instead of using an untagged enum.

### 8. [Low, UX] An incomplete provider config blocks `jev doctor` and `jev auth status` — confirmed
- `commands/mod.rs:338` exempts only `config` from provider resolution. With `provider = "cloudflare"` saved and no account ID, the diagnostic commands fail before they can explain the problem. The error message itself is actionable.

### Design notes, not defects
- The resume fingerprint (`map.rs` `provider_fingerprint`) covers `options` and `keep_alive`. Toggling `--reject-if-busy` or `--keep-alive` therefore refuses `--resume`. The error message names "request option", so this is a deliberate choice; it is debatable for capacity and retention policies that don't change the answer.
- The MCP input schemas are static unions: `instructions` is optional and scalar `state` is advertised even when TypeSafe is configured. The server enforces the stricter rules correctly; agents will just get more rejected calls.

## Invariants I checked and found intact
- **Credentials:**
  - `is_official` needs both the official base URL and the SystemOne protocol (test-covered).
  - `Credential::anonymous` sends no header, while an empty key still sends `Bearer` (transport test).
  - `redact` handles an empty secret.
  - Cloudflare errors echo only the numeric code; local-provider errors are generic; neither echoes the response body.
- **Routing:**
  - The Cloudflare account must be 32 hex characters; the model name is normalized to `clef` or `clef-flash` before it goes into the URL, so nothing can be injected into the path.
  - Redirects are still not followed.
  - Plain HTTP is still allowed only on loopback.
- **Provider gating before any credential or socket:** publisher-style JSON (scalars, blank text) only for `huggingface`; videos, `max_length`, `max_state_tokens` and `media_kwargs` only for `huggingface`; `rejectIfBusy` only for Cloudflare; `keep_alive` only for Ollama; images not for TypeSafe or llama.cpp; Score/Choice limits are 10, 26 or 255 by provider; at most 64 questions; body-size caps of 13 MiB (Cloudflare/huggingface) and 64 KiB / 32 MiB (Ollama without / with images).
- **Media:**
  - Base64 length is bounded before decoding; the full PNG signature and IHDR are required.
  - Pixel counts use checked multiplication; per-image limits and the aggregate bytes/pixels limits are checked in both setter orders.
  - Video metadata is bounded with increasing indices, and resampling sparse indices is refused.
  - Unix file opens walk directory handles with `NOFOLLOW|NONBLOCK`.
- **Publisher compatibility:** I checked against the pinned `joint_schema_model.py`:
  - Missing, `null` or `""` instructions fall back to the question ID; whitespace is kept.
  - An empty Noul criteria object or `null` uses the defaults; an explicitly `null` side suppresses that side's default.
  - Score legends keep scalars.

  All of these match lines 46-57, 120-123 and 523-543.
- **Old TypeSafe behaviour:**
  - The legacy fingerprint is kept when no provider features are used.
  - The request body has the same keys in the same order.
  - New JSON output fields are only additions.
  - Exit codes are unchanged; mapping 413 to `InvalidRequest` still exits with the usage code.

## What I inspected
- **Full patch plus current source:** 007, 008, 010–023, 025, 026, 030–035, 039 (whole current file), 041–063, 068, 191.
- **Partially:**
  - 024 (first 80 lines).
  - 027, 028, 029: patches not read; I read the current `map`, `noul` and `score` schema files in part.
  - 036, 037, 038, 067: test function lists only.
  - 040 (snapshot): grep only.
- **Not inspected:** 064–066 (binary image fixtures).
- **Context read:**
  - The session brief, the extraction proof, and all 265 transcript messages.
  - The ADR-0015 source file and the threat-model patch.
  - `clef-residuals-verification.json`, `clef-research.md` and `clef-publisher-input-compatibility.md` in full; `clef-skill-verification.json` lines 1–60.
  - Parts of `clef.md`, `cli-contract.md` and `commands.md`.
  - The CHANGELOG patch and the publisher `joint_schema_model.py` (partly).
  - Current `errors.rs`, `models.rs`, `wire.rs` and `redact`.
- **Not read:** `full-session.diff` as a whole, the prior Grok logs, and the eval evidence. Those belong to the other reviewers' areas.

## Limits of this review
- Static reading only; nothing was executed.
- My claim that serde turns unknown fields into errors when `deny_unknown_fields` is combined with `flatten` (the MCP argument structs) comes from my knowledge of serde, not from running it. A test sending an unknown field to `noul` should confirm it.
- Windows and macOS file-open code is untested, as already documented.
- There is still no fuzz target that reaches valid-base64 image headers. The existing `request_document` fuzz target only partly exercises media parsing.
