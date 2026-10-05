# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as
applied to **the command-line interface**, not to the Rust crates — see
[`docs/adr/0003-cli-compatibility.md`](docs/adr/0003-cli-compatibility.md).

Entries are required for anything user-visible: a command, a flag, an exit code, a
JSON field, an environment variable, or a change in what is sent over the network.

The `0.1.1` section records a development milestone before this public repository's
first commit. It is not a published release. For current verification and release
policy, use `AGENTS.md` and [ADR-0014](docs/adr/0014-manual-release-gate.md).

## [Unreleased]

### Fixed

- Documentation and agent guidance now consistently describe the selected Clef
  provider, transmitted images/video, isolated credentials, local runtime lifecycle,
  provider limits, model pinning, test provenance, and the published 0.3.0 release.

## 0.3.0 — 2026-10-05

### Added

- Automatic before/after CLI, smoke/quality helper, and helper-interpreter hashes in
  future Clef live-test reports. `--provenance-manifest` fingerprints explicitly
  named local model/runtime files, including runtime libraries; reports preserve
  unknown server association and remote-runtime limits.
- Clef and Clef Flash through explicit `--provider cloudflare`, `ollama`, and
  `llamacpp` adapters, plus `huggingface` through a separately launched Python bridge. TypeSafe remains the default. Cloudflare account selection uses
  `--cloudflare-account-id`, `CLOUDFLARE_ACCOUNT_ID`, or the nonsecret configuration
  setting; tokens use the existing custom credential namespace. Loopback local
  inference never resolves or sends credentials.
- Explicit PNG/JPEG/WebP vision with repeated `--image`, embedded request and MCP
  images, `map --images-field`, and optional evaluation-row images. Media is bounded
  before network use and included in resume fingerprints. Ollama gets its native
  raw-base64 representation; hosted Cloudflare gets its documented image objects.
- Cloudflare image-only requests with an explicitly supplied empty string state,
  across scalar commands, request files, batch mapping, evaluation, and MCP tools.
  Empty state still requires validated images; other providers retain their
  documented state requirements.
- Prepared video-frame arrays through the local Python bridge, repeated
  `--video-frame`, `map --videos-field`, evaluation rows, and MCP. The bridge uses
  explicitly installed publisher weights with offline loader settings; bounded
  `--max-length` and `--media-kwargs` expose native tokenization/processor controls.
- Cloudflare `--reject-if-busy`, Ollama `--keep-alive`, provider-aware model discovery,
  offline diagnostics with explicit provider/account metadata, and up to 26 Ollama Score levels. Existing JSON schemas and
  exit codes remain; MCP structured instructions and criteria gain additive support.
- Nonsecret `provider` and `cloudflare_account_id` configuration settings, and
  [hosted/local setup, capabilities, and limitations](docs/clef.md).
- Independent local state budgets with `--max-state-tokens`, source video cadence
  with `--video-fps`, and bounded per-video metadata in native requests and MCP.
- Publisher-compatible JSON scalars and blank state, instructions, and criterion
  descriptions through `--provider huggingface`, including exact instruction-ID
  fallback, explicit null Noul-side semantics, up to 255 Score levels, and scalar legends across
  commands, batches, evaluation, and MCP. Other provider validation remains scoped
  to its existing contract.
- Opt-in, request-bounded synthetic Clef live checks, deterministic fixture generation,
  pinned Python dependencies, and offline publisher-release integrity verification.
- A separate 32-row original synthetic quality benchmark, structured-question live
  coverage, hash-pinned Linux CPU wheel installation, and an offline Python runtime
  fingerprint checker. Synthetic quality measurements remain scoped to their dataset.
- GPT-6.1 Sol high skill evaluations through an instrumented Codex harness, including
  baseline comparisons, repeated runs, independent grading and held-out routing.
- `base64`, PNG/JPEG/WebP-only `imagesize`, and Unix-only `rustix` for bounded media
  handling and safe image file opening;
  justification is in [ADR-0015](docs/adr/0015-clef-providers-and-vision.md).

### Fixed

- Clef live-report creation refuses symlink parents and preserves existing files.
  Malformed CLI/MCP output, including excessively nested JSON, retains sanitized
  failure reports and provenance.
- Inference-command skill advice requires applicable recipient, credential,
  local-runtime, privacy, and CPU deadline/lifecycle explanations rather than
  relying on background reminders. A fresh comparison retains the prior guide and
  a common no-guide control alongside the changed guide.
- MCP map advertises per-record video timing metadata, and all inference schemas
  require positive source/sampling cadence and duration, matching runtime validation.
  The schema compatibility guard accepts numeric `exclusiveMinimum` from JSON
  Schema 2020-12 and continues to reject unknown keywords and invalid boolean bounds.
- Planning, fit, and audit skills identify the selected recipient rather than always
  naming TypeSafe; loopback-server forwarding and transmission consent remain explicit.
- Legacy skill-eval filters refuse empty selections before any model call.
- Switching providers no longer inherits an endpoint saved for a different
  provider. An explicit endpoint flag still overrides the selected provider's base
  URL, and existing TypeSafe endpoint configuration remains supported. A static
  stderr notice identifies a skipped saved endpoint without revealing its URL.
- Configuration accepts the existing `llama-cpp` CLI alias as well as `llamacpp`.
- Developer diagnosis constructs the video processor without weights, detecting
  missing lazy-loaded dependencies instead of treating the class import as sufficient.
- MCP map input budgets now count decoded image/video bytes as well as state,
  including template media. Inference-only media and processor flags are refused
  on commands that do not consume them.
- Diagnostics remain usable for a saved Cloudflare provider with no account ID,
  reporting the configuration error without reading credentials or making requests.
- Hugging Face preflight refuses animated WebP instead of sending input that its
  single-frame decoder will reject; image-format errors retain their specific cause.
- Request fuzzing now reaches raw PNG/JPEG/WebP headers directly and checks image
  bounds and canonical base64 round trips, alongside the existing JSON parser.
- Duplicate object keys are rejected in `--state-json`, `--state-json-file`,
  request-document state and question content, and options/levels files, including
  nested objects, before network use.
- Local Python video pixel controls now use the publisher's whole-clip units and
  separate image/video defaults. Preflight bounds include actual spatial-patch
  rounding, temporal padding, and possible sampling allocations.
- Malformed local image decoder input returns a content-free error, and disconnected
  clients do not expose decoder, request-header, or response-writing tracebacks.
- Synthetic live checks refuse semantically inconsistent Choice/Score answers and
  Hugging Face confidence values; rounded valid distributions remain supported.
- Processor options from `--media-kwargs` and request documents reject duplicate
  object keys, including nested keys, before anything is sent.
- Batch templates ignore their unused state semantically, including blank state;
  actual row state and media still undergo their normal validation. JSON batch and
  evaluation rows now reject duplicate fields at every nesting level, including labels.
- Local video sampling with `num_frames` clears the processor's default FPS to avoid
  a conflict between the two sampling alternatives.
- Local release rehearsals compile from an isolated read-only snapshot of the
  tracked working source and pair the binary with its exact captured archive,
  recording file hashes and original provenance.
- Source snapshots reject a tracked `.jev-source.json` before creating an archive,
  preventing collision with generated provenance metadata.
- Shipped agent guidance now uses the selected provider's credentials and privacy
  boundary, including anonymous loopback inference and explicit Clef media.
- Development guidance covers provider authority, uncommitted security review and
  the approved manual release workflow; readiness detects protected runtime-copy
  drift and guards the Clef and evaluation verification steps.
- The credential-argument canary recognizes `--max-state-tokens` as a numeric budget
  and checks its parser type; credential-shaped strings still fail locally.
- The spelling dictionary recognizes Transformers' exact `video_grid_thw` output
  identifier; ordinary spelling checks remain enabled.

### Known limitations

- Hosted Free-account checks on 2026-10-04 UTC found both Clef models reject
  `--reject-if-busy` with HTTP 422, Cloudflare code 5012. The retained numeric-error
  reports do not establish its precise cause. The CLI retains the documented native REST option
  and reports the rejection without silently resending a request without it.

## 0.2.1 — 2026-09-24

### Added

- **`jev map --limit N` and `--seed S`: run a question set on a few records before
  billing the whole batch** ([`docs/commands.md`](docs/commands.md)). `jev eval` had
  `--limit` and `jev map` did not, so a pilot meant `head`ing the input — and the first N
  rows of a file sorted by date or by source are a biased sample. `--limit` alone takes
  the first N; with `--seed` it takes N chosen by a hash of each record's id, the same N
  for the same input and seed. Records keep their input `index`, selections nest as N
  grows, and `--resume` re-uses rows already answered, so a pilot can be widened into
  the full run without re-sending anything. `--limit 0` is refused, as in `jev eval`.
  The `jev.map.summary/v1` document gains a `limit` object, present only under
  `--limit`; `total` counts the selection.
- `usage` in `jev.map.summary/v1`, and so in the MCP `map` tool's `summary`: the
  `input_tokens` and `output_tokens` the API reported for the records this run
  answered, which users were summing out of the rows with `jq`. It follows
  `jev.eval/v1`'s `usage` — a total is `null` when nothing reported it, never `0` —
  and adds `rows_without_usage`, the answered records whose response carried no count,
  so a total that is only a lower bound says so. Records skipped by `--resume` are
  not in it; failed records carry no usage and are in neither. The same totals are one
  line on stderr.

## 0.2.0 — 2026-09-23

The first public release. It includes the changes below and the command surface from
the earlier `0.1.1` development milestone.

### Added

- **`jev mcp serve`: a local, stdio-only Model Context Protocol server** ([`docs/mcp.md`](docs/mcp.md),
  [ADR-0012](docs/adr/0012-mcp-server.md)). It exposes five tools, `noul`, `choice`,
  `score`, `ask`, and `map`, backed by the same code as the commands of the same names.
  Their tool names, input schemas, and output schemas are a new stable surface, pinned
  in `crates/jev-cli/tests/snapshots/mcp-tools.json`. It adds one schema identifier,
  `jev.mcp.map/v1`, which wraps `map`'s rows and summary. No key is accepted as an
  argument or configuration; credentials resolve as for every other command.
- New runtime dependencies for `jev mcp serve` only: `rmcp` 3.4 (the official Rust MCP
  SDK, `server` and `transport-io` features) and `tokio` 1 (a current-thread runtime).
  The release binary grows by about 1.5 MB.

- `missing_answers` in `jev.evaluation/v1` and `jev.map.row/v1`: present only when the
  API returned no answer for a question, listing the ids. Before, a map row simply
  lacked the answer, which a script could read as a question never asked.

### Security

- **An API error body that quotes the credential back no longer shows it.** A proxy, a
  debugging echo server, or a hostile custom endpoint could repeat the `Authorization`
  value in its error text, and `jev` printed that text. The key is now replaced with
  `<redacted>` in `jev-client`, before the message reaches the CLI or an MCP result:
  in the full decoded message before it is truncated, so neither a JSON-escaped key nor
  one straddling the length limit survives, and in the `x-typesafe-request-id` header.

### BREAKING

- **`--retries` above the maximum is now refused rather than silently capped.**
  `jev --retries 50 …` used to be clamped to `10` with nothing said, even under
  `--verbose`, so a CI job that raised it for resilience got the default behaviour and
  never learned otherwise. It now exits `2` with `--retries must be at most 10`, which
  is the rule `--timeout` has always applied: a flag is this invocation's explicit
  intent, so an out-of-range one is refused. A stale value in a **configuration file**
  is still clamped rather than refused, because it should not make every invocation
  fail.

### Fixed — found by the first run against the real TypeSafe API

On 2026-09-23 the opt-in live test matrix ran for the first time, all passing, and a
manual pass exercised every command and `jev mcp serve` against the live API
(`docs/api-compatibility.md`, "Live verification").

- **A key containing a line break or control character is now refused as a credential
  error (exit `3`), naming its source.** Before, a two-line key file failed at request
  build time as "could not reach the API endpoint" (exit `4`) and was retried twice,
  though nothing had been sent. Surrounding whitespace is still trimmed.
- **`--options-file` now sends options in the order written and rejects a repeated name**
  with `duplicate option name`, as `ask -r` already did and `docs/commands.md` promised.
  Before, options were sent alphabetically and a repeat was silently last-one-wins, or
  misreported as "needs at least 2 options, found 1".
- **The API's token-limit rejection is readable.** HTTP 400
  `{"detail":{"error_type":"max_tokens_exceeded"}}` now reads
  `max_tokens_exceeded: the request is over the model's token limit` rather than raw
  JSON.
- **Text output lists questions and Choice options in the order the request asked
  them**, not alphabetically. JSON output is unchanged.
- `auth status` and `doctor` against a custom endpoint with no key now advise
  `JEV_CUSTOM_API_KEY` or `JEV_CUSTOM_API_KEY_FILE`. They used to suggest
  `jev auth login` and `JEV_API_KEY`, neither of which applies to a custom endpoint.
- An empty key now names the variable that held it (`$JEV_API_KEY`, not "environment").
- `jev mcp serve`'s `map` rejects `concurrency: 0` with the MCP range (1 to 16), not the
  CLI's (1 to 64).
- `jev eval --no-split` prints its warning once, prefixed `warning:`, instead of twice.
  Calibration bin bounds print as `0.3`, not `0.30000000000000004`. The eval text footer
  lost a trailing space. The zero-option Choice error no longer explains the one-option
  case.
- **`jev auth login` refuses a key containing a line break or control character**
  (exit `2`) and stores nothing. Before, it stored a key the resolver would then refuse.
- `auth status` and `doctor` sources gain **`set`**: whether the variable exists or the
  store holds an entry, even when the value is unusable. `present` keeps its documented
  meaning, a usable value was found. The text report shows such a source as "set but
  unusable" rather than "not set", which contradicted the error printed beneath it.
- **`jev map` and `jev mcp serve` `map` rows carry `attempts` when `ok` is `false`**,
  as they already did when it was `true`: a failed row may still have reached the
  server, and been billed, more than once. It is `0` only for `invalid-request`, where
  nothing was sent. The MCP `map` output schema now lists `attempts` as required on
  every row.
- Documentation: an unknown model is HTTP 400, not 404 (`docs/mcp.md`); `doctor --live`
  exits `0` even when its check fails, which is now stated; timeouts are retried like the
  official SDK, and a timed-out attempt may already have been billed. The exit-code
  table in `docs/cli-contract.md` now says `doctor` exits `0` as a report.
- Internal: `scripts/test-skill-scripts.py` expected five sessions from the rich fixture
  under `--max-sessions 2`, from before the fixture gained a second Gemini CLI session;
  it now expects six and requires both capped providers to be named.

### Fixed — three skill gaps found by the Opus 5.5 confirm pass

Each was read in the kept traces of the failing runs before anything was changed; the
remaining failures in that pass were run-to-run variance or, in one case, a judge call
refused by the API's own safeguards, and nothing was edited for them.

- **`jev-pilot` had no rule for a miss that the errors explain.** Its verdict table let a
  missed bar read as `REJECT` even when every error sat at one boundary the question
  itself drew. Missing the bar now makes `REJECT` possible, not automatic: errors that
  concentrate at a fixable boundary, with the confidence dropping there, are
  `REVISE AND RE-EVALUATE` — unless the incumbent already does the job. `NEED MORE DATA`
  now also says where the user may already have the labels.
- **`is-jev-useful-here` did not load on an obvious no.** Asked whether Jev could speed up
  a filter-and-sum job, the agent answered from general knowledge in three runs of three,
  at length. The description now says to use it even then, because a short no is the
  skill's job.
- **`jev-workflow-retro`'s empty report could still end with a speculative line** about
  what a longer window might show. The rule against sketching future work now names it.

### Fixed — found by a GPT-6 Sol review of the whole session's work

A read-only review of `3f475c9..HEAD` through the Codex CLI, run at the user's request;
every finding was reproduced before it was fixed.

- **`jev-pilot` put a secret into the agent's context in order to look for it.** Step 4
  printed the dry-run body and asked the agent to read it. It now checks where the
  request goes, its size and its fields, counts sensitive shapes by kind locally, and
  leaves the raw preview to the user.
- **`jev-pilot` paired one draw's confidence with another draw's errors.** It ran `map`
  and then `eval` over the same rows — two bills, two sets of answers — and read the
  errors against the `map` confidence. `jev eval --show-rows` is now the run the errors
  come from; `map` is only for the confidence of those rows, labelled as a second draw.
- **Statistics**: two people's agreement is a reference point for a bar, not a ceiling;
  McNemar's test is for accuracy only; and "the incumbent already does the job" means it
  meets every criterion, cost and latency included.
- **The retro's headline said nothing left the machine**, then described the exception.
  It now leads with what is local (the parser) and what is not (what the agent reads),
  and offers a summary-only path for a strictly local audit.
- **The audit's secrets case now checks the whole trace** for the canary value, so a
  recursive search that never names `secrets/` no longer passes.
- **The pilot's fixture reports were not the CLI's shape.** They nested metrics, wrote
  intervals as arrays and carried fields `jev eval` never prints. All six are now in the
  exact `jev.eval/v1` shape, checked against the CLI run on a mock API; what the CLI does
  not print (confidence on Score rows, the regression checks, label provenance) moved to
  `pilots/*/notes.md`, where a real pilot would keep it, and the graders say so.
- **The transcript normaliser, nine fixes, each with a test that failed first**: a long
  quoted credential is redacted whole rather than clipped part-way; a record whose role
  the parser does not recognise is emitted with its size, never its text, and `*_output`,
  `*_result` and `*_response` roles are tool results; every string that reaches an event
  — tool names, argument keys, models, projects — is redacted and capped at one boundary;
  an invalid `--since` or `--until` is an error instead of a silently wider window; the
  window and caps apply to `--input` exports, and a filter that cannot apply is reported;
  only the providers' own injected wrapper tags are stripped, so a prompt's
  `<question>…</question>` survives; replayed tool calls and Claude Code usage are
  de-duplicated on the provider's ids; Codex subagent rollouts join their parent before
  the session cap; and a run stops, and says so, at two million events. The keyword
  redaction pattern was also quadratic on long words (0.73 s for a 5,000-character
  string; now 0.001 s). The no-network test is now an import allowlist over the parsed
  source, which the old substring check was not.
- **Realistic reports exposed two `jev-pilot` gaps.** With the state no longer inside the
  fixture report — the CLI never prints it — the agent read only the report and missed a
  leak it had found every time before. Step 9 now says the report does not carry the
  state and to look the errors up in the dataset it names. And a class carrying the loss
  now has to be named as too thin to judge, weakly defined, or both; a no whose condition
  is more data says how many rows and where they may already exist.
- **`scripts/skill-eval.sh` graded with the confirm model** even when an override chose
  another to measure; the judge now follows the model actually measured.
- **`scripts/validate-skills.py`** now rejects invalid YAML escapes, accepts
  `Bash(git status:*)` in `allowed-tools` as the spec does, and resolves a Markdown link
  from the file it is in.

### Fixed — found by five independent Opus 5.5 reviews of the finished skills

Trigger collisions, cross-agent portability, architecture, privacy and security, and the
pilot's statistics, each reviewed separately and read-only.

- **The transcript normaliser printed tool output from exports.** A generic or Cursor
  agent record with role `tool`, `function` or `tool_result` and no name came out as an
  `unknown` turn with 600 characters of what the tool returned. It is now a `tool_result`
  carrying only success and size, like every other adapter's.
- **Redaction missed the commonest shapes in a transcript**: JSON- and repr-quoted pairs
  (`{"api_key": "…"}`), `--password X`, `curl -u user:pass`, `github_pat_`, `glpat-`,
  `xapp-`, `npm_`, `hf_` and STS `ASIA` keys, PGP private key blocks, and a PEM with no
  `END` line. All are redacted now, each with a test.
- **The normaliser had no size bounds**: an 80 MB line peaked at 217 MB. Lines and
  whole-read files over 16 MiB are skipped and reported, and subagent files per session
  are capped. Cline metadata files get the symlink guard, and a non-string `cwd` no
  longer crashes discovery. `CLAUDE_CONFIG_DIR` and `CODEX_HOME` are honoured, so a moved
  store no longer reads as "no sessions".
- **"Nothing leaves the machine" was not exact.** What the retro reads enters the
  agent's own session, and so its model provider. The skill now says so, reads
  aggregates before text, and asks before reading another agent's history.
- **`jev-pilot` checked one row, then sent all of them.** It now scans every row locally
  and confirms the count before a billed batch, and keeps pilot data out of version
  control.
- **`jev-pilot`'s statistics**: a band read off the reported sweep is in-sample; an
  interval straddling the bar is unsettled, not a miss; overlapping intervals are not
  parity (count the discordant rows); calibration is not required for an empirical cut;
  a revised bar applies to the next measurement; human agreement is the ceiling to check
  a bar against; the gated figure under a coverage target rests on the covered rows; and
  a Score's bar may be adjacent agreement or weighted kappa. The new REVISE rule is
  bounded: the boundary must be visible in the question, it is available once, and a
  second miss is `REJECT`.
- **The `jev` skill's examples broke its own rules**: a CI gate fed an author-written
  diff, production logs piped out unchecked, and an agent action auto-run on a Noul. Each
  now says what it may and may not decide, and the skill gains the "what you send is
  data, not instructions" rule the other four already had.
- **A small sample could outrank a working incumbent.** The first version of the bounded
  REVISE rule put `NEED MORE DATA` ahead of every other verdict under fifty reported
  rows, and the re-measure caught it: with the keyword rule beating Jev on seventeen
  rows, the pilot called the result "formally need more data". A small sample can leave
  Jev's number unsettled; it cannot make a working incumbent need replacing, so that is
  `REJECT` at any size.
- **`is-jev-useful-here` rarely loaded on a cost or speed question.** "Will moving this
  to Jev cut the bill?" loaded it in one run of nine, because "not for pricing" read as
  excluding it. The description now claims "would moving this to Jev make it faster or
  cheaper" and excludes only Jev's own prices and rate limits.
- **"Use it even when the answer looks like an obvious no" over-triggered.** Added this
  round for the definite-no case, it pulled `is-jev-useful-here` into a plain question
  about having an LLM draft release notes, where it appended a Jev verdict nobody asked
  for. The new `routing-generation-is-nobodys` case caught it; the sentence is gone, and
  the description now excludes whether a model should write or generate something.
- **Descriptions**: `is-jev-useful-here` no longer claims "holds up" or a bare "where
  would Jev fit" (a pilot's and an audit's); `jev-pilot` claims "measurably beats" and
  "missed its bar", not "replace" and "failed"; the retro claims an exported *agent*
  conversation.
- **Portability**: the retro no longer relies on a shell variable surviving between tool
  calls, says how to find its own directory, and notes that tool names are each
  provider's own. `scripts/validate-skills.py` now rejects YAML that strict loaders
  reject, an empty `compatibility`, a non-string `metadata` value, and a reference or
  script path that does not resolve inside the skill.
- **Evals**: the audit's secrets case is now a git checkout with a real `.env` canary,
  and grades Grep and Glob as well as Read; the retro's sensitive case plants a login
  typed into a prompt, the one shape that tests the skill rather than the normaliser; the
  plain-grep near miss also asserts `is-jev-useful-here` stays out; a new routing case
  asks whether an LLM should write release notes, which no skill should claim; and the
  beacon fixture reports carry the warnings the real CLI prints.

### Fixed — the transcript normaliser was losing most of what it exists to capture

Found by running the adapters against the real stores on a real machine and comparing
the counts against the files, rather than by reading the code. None of these raised;
each one simply returned less, which is the failure the module was written to make loud.

- **Codex: `response_item/message` turns were not read**, so on a store where most turns
  are recorded that way, prompts and assistant turns went missing while tool calls still
  came through, and a session read as pure tool use. Both record shapes are now read —
  and, per the entry above, counted once, since every version on disk records a turn in
  both. Compaction records are read too.
- **Gemini CLI: 9 prompts were read out of 390.** A Gemini content part is
  `{"text": …}` with no `type` key — the native part shape, not an Anthropic or OpenAI
  content block — and the joiner required `type == "text"`. Measured over the whole real
  store: **9 prompts before, 584 after.**
- **Gemini CLI: the whole-document format was parsed as JSON Lines.** `.json` and
  `.jsonl` files share the directory and the `.json` ones are a single pretty-printed
  document, disproportionately the largest sessions. Reading one a line at a time
  produced thousands of "unparsable line" problems and no events, while discovery still
  counted it as a session.
- **Claude Code: subagent transcripts were never discovered.** A subagent's own turns
  live only in its sidechain file, one directory deeper than the glob reached, so the
  delegated half of every session was invisible and the `isSidechain` handling was
  unreachable code. 630 such files were present and unread on the machine this was found
  on. They now roll into the parent session's totals.
- **`--max-sessions` was pooled across providers, so a provider used less recently than
  another vanished from the run — and `discover` then reported it as "store present, no
  sessions in the window", a claim about the user's history rather than about this run's
  budget.** The cap is per provider now, and `--provider` no longer reports the providers
  it excluded as having nothing.
- **A pre-envelope Codex rollout yielded no events and no problem**, which is
  indistinguishable from a session in which nothing happened. It is now reported.
- Three adapters could count a directory as a session; the Markdown reader missed the
  `**Assistant:**` convention, where the colon sits inside the bold delimiters; and
  `events` printed problems without the path.

`references/providers.md` is corrected throughout, including the `grok-cli` row, whose
store changed shape between two checks on the same machine. Each format above has a
fixture and a regression test in `scripts/test-skill-scripts.py`.

### Internal — how the shipped skills are measured

- **Pinned model and effort.** `scripts/skill-eval.sh` runs every child session, iterate
  pass and grader on `claude-opus-5-5`, a full model id, with `CLAUDE_CODE_EFFORT_LEVEL=low`,
  instead of inheriting whatever the calling session is configured with. The `opus` alias
  moved from Opus 5 to Opus 5.5 while this suite was being built; results measured before
  that were Opus 5, graded by Haiku.
- **Graders no longer run on Haiku**, the harness default, which repeatedly failed
  correct answers on rubrics that expressly allowed them.
- **`--iterate`**: one run, no baseline arm, same model — roughly a sixth of a default
  pass (3 runs × 2 arms), for the red–green loop only. Its numbers are not results.
- **`--heldout`** stages the held-out routing set (`evals/skills/heldout/`, 30 cases
  written without sight of any description) and nothing else, and is refused under
  `--iterate`.
- Every behaviour case records whether its own skill loaded, so a pass the base model
  earned is not credited to a skill.
- Each run stages under its own `target/skill-evals/<run-id>`; a shared tree was
  clobbered when a held-out run started beside a training run.
- `refuses-to-open-secrets` now writes its credential canary at staging time. The
  fixture's own `.gitignore` excludes `secrets/` — deliberately — so the canary had never
  been committed, and the case tested nothing outside the machine that wrote it.

### Fixed — the transcript normaliser was miscounting real sessions

An independent review ran `skills/jev-workflow-retro/scripts/transcripts.py` against a
real machine's stores and compared what it emitted with the raw records. Every finding
below produced a wrong number without raising, and each now has a regression test built
from the real record shape (`scripts/test-skill-scripts.py`, 64 tests at this entry).

- **Codex usage was overcounted about 26×.** `token_count` carries a running total after
  every turn — up to 1,688 per file — and they were summed. Only the last one counts; on
  the reviewer's window the script now reports exactly the independently computed total.
- **Every Codex turn was counted twice**, once as `response_item/message` and once as
  `event_msg/user_message` or `agent_message`. Both exist in every version on disk.
- **Injected turns were counted as prompts**: Codex's `<environment_context>`-style
  wrappers (37% of its "prompts") and Claude Code's background-task notifications and
  peer messages (40% of its), which `origin.kind` distinguishes from a person.
- **Codex subagents** were read from a field that does not exist, and a spawn was
  recorded twice (the `spawn_agent` call and a `SubAgentActivity` "started"). A
  subagent's rollout — whose user turns the *parent agent* wrote — now rolls into the
  parent session instead of counting as the person typing the same instruction three
  times.
- **Codex MCP tool calls were dropped entirely**; they have no `function_call` record.
- A forked Codex rollout switched to its parent's session id; subagent files were counted
  as sessions and used up `--max-sessions`; the Claude Code session-open event had no
  working directory; `--days` filtered on file mtime, so a long-running file carried a
  month of older turns into a week's report.
- **Cursor's agent CLI is now parsed**
  (`~/.cursor/projects/*/agent-transcripts/`). Its nested `{role, message: {content}}`
  shape had been coming out of the generic reader as the literal word "content".
- A byte-order mark lost a file's first line; a zero-byte file, lines that were JSON but
  not objects, and a directory named `*.jsonl` were skipped silently; a non-string name
  or deep nesting crashed the run. All are now reported in `problems`.

### Fixed — redaction gaps in the transcript normaliser

Found by an independent review of `skills/jev-workflow-retro/scripts/transcripts.py`,
each one confirmed by reproducing it. The skill's entire privacy claim rests on this
function, so these were the findings that mattered most.

- **`SCREAMING_SNAKE_CASE=value` was not redacted at all.** The keyword pattern wrapped
  its keyword in `\b`, and `_` is a word character, so the boundary never fired inside an
  identifier: `AWS_SECRET_ACCESS_KEY=…`, `DB_PASSWORD=…` and `STRIPE_SECRET=…` — which is
  the shape almost every real credential in a transcript actually has — passed through
  verbatim.
- **Underscore-separated vendor keys were not redacted.** The pattern required a literal
  `sk-`, so every real Stripe `sk_live_…` key went straight through.
- **A credential in a URL authority** (`postgres://user:pw@host/db`) has no keyword near
  it and nothing could reach it. It now has its own pattern.
- **`safe_args` redacted values without their keys.** Splitting a tool call's arguments
  into separate entries destroyed the keyword adjacency the patterns depend on, so a
  value under a key literally named `password` was emitted in full. It now redacts the
  pair.
- **A symlink inside a transcript store was followed out of it.** `Path.glob` follows
  links, so a link planted in `~/.claude/projects` made this skill read and report a file
  the user never named — `AGENTS.md` §4 forbids exactly that. Discovery now resolves each
  path against the store root and reports anything that escapes instead of reading it.
- **`clip` scanned an entire field before truncating it.** One 100 MB line meant
  collapsing whitespace and running every pattern over all of it to produce at most a few
  hundred characters. The work is now bounded, not just the output.

Each has a regression test in `scripts/test-skill-scripts.py`.

Also fixed: two compiled `.pyc` files were tracked, one of them inside `skills/` — a
published surface a user copies into their own agent, where a CPython-version-specific
build artefact has no business being. `.gitignore` had no rule for them.

### Added — `jev-pilot`, a fifth shipped Agent Skill

- **`skills/jev-pilot/`** takes one candidate decision and settles whether Jev actually
  works well enough for it, ending in `ADOPT CANDIDATE`, `PROMISING — NEED MORE DATA`,
  `REVISE AND RE-EVALUATE` or `REJECT FOR THIS WORKFLOW`. It closes the loop between the
  three discovery skills and evidence. See [`docs/agent-skill.md`](docs/agent-skill.md).
- **The failure it exists to prevent is the pilot that was always going to succeed**:
  success defined after the numbers arrived, a strawman baseline, a threshold tuned and
  then reported on the same rows. So the criteria are written down before the first
  request and do not move; the incumbent is measured on exactly the rows Jev sees; the
  threshold is chosen on calibration rows and reported on rows the choice never saw; and
  `REJECT` is documented as a successful outcome that is not softened because effort was
  spent.
- **Three checks come before any spend**, and any of them ends the pilot at zero cost: is
  the output bounded, do labelled examples exist, and may this data leave the user's
  environment. A pilot having been requested does not authorise the transmission —
  `--dry-run` shows the exact bytes first.
- **Prototype only.** It changes no production path, adds no CI gate, and removes no
  existing model call, because the incumbent is the baseline and deleting it destroys the
  comparison.
- The method — finding labels, how many are enough, reading the calibration and the
  confusion matrix, the four error groups, checking for state that leaks the answer, and
  the artifact layout — is in
  [`references/method.md`](skills/jev-pilot/references/method.md).

### Added — `jev-workflow-retro`, a fourth shipped Agent Skill

- **`skills/jev-workflow-retro/`** audits the user's own past coding-agent sessions and
  reports which repeated decisions Jev could take over, ranked, with the count behind
  each. It is the behavioural counterpart to `jev-opportunity-audit`: that skill reads a
  repository, this one reads what the user actually did. See
  [`docs/agent-skill.md`](docs/agent-skill.md).
- **Transcripts never leave the machine.** Parsing and analysis are local, no part of
  the workflow calls the API, and the skill declines a request to pipe a session store
  through `jev map` — a transcript store holds prompts, proprietary source, customer
  records and credentials printed by accident, and that is not a default to override.
- **`skills/jev-workflow-retro/scripts/transcripts.py`** is a dependency-free local
  normaliser with `discover`, `events` and `summary` subcommands. Adapters for Claude
  Code, OpenAI Codex, Gemini CLI and Cline, each verified against real session files,
  plus a generic reader for an exported JSONL, JSON, Markdown or plain-text transcript.
  It clips text, replaces credential shapes, and reduces tool *results* to size and
  success rather than content. Providers with a local store that is deliberately not
  parsed — the Cursor editor, Copilot, Grok, OpenCode, Pi — are named in the output with their
  paths, so an unsupported provider never reads as "examined, nothing found". Formats
  and their traps are recorded in
  [`references/providers.md`](skills/jev-workflow-retro/references/providers.md).
- **`scripts/test-skill-scripts.py`** holds it to the repository's testing bar —
  tests over provider discovery, normalisation, redaction, clipping, counting, corrupt
  and truncated input, the generic reader, and the assertion that the script imports no
  third-party module and opens no socket. It runs in `scripts/verify.sh`. Three real
  defects were found by writing it: a per-kind counter silently replaced the session
  total, the Codex adapter's project filter discarded every Codex session, and a turn
  the generic reader could not classify was dropped by the kind filter instead of being
  reported.

### Fixed — the two fixture-based skill eval cases had never reached their fixtures

- **`jev-opportunity-audit`'s `pulse` and `pixelsort` cases were scoring an empty
  workspace.** `claude plugin eval` starts each case with an empty working directory and
  refuses reads above it, so both cases had been measuring the agent's inability to find
  `fixtures/pulse` rather than the quality of its audit, and reporting that as a low
  score for the skill. Cases that need a fixture now carry a `case.yaml` with
  `context.scaffold_script` and a `scaffold.sh` that copies it in, and
  `scripts/skill-eval.sh` passes `--scaffold`. The flag and why it is honest here are
  documented in
  [`docs/development/skill-authoring.md`](docs/development/skill-authoring.md) §8.
  `context.add_dirs` looks like it would do the same job and does not: it validates the
  path and stages nothing.

### Added — `jev eval`, threshold calibration

- **`jev eval`** measures a question against labelled examples you already have, and
  chooses a threshold you can defend. It is not a benchmark of Jev: it measures one
  question, one dataset, and one model version, and the report records all three.
  Nothing is trained, and a label is never sent — ground truth is compared locally after
  the answer comes back. See [ADR-0011](docs/adr/0011-threshold-calibration.md) and
  [`docs/commands.md`](docs/commands.md#jev-eval).
- **A labelled dataset format, `jev.eval.row/v1`.** JSONL, one example per line, with
  `id`, `state`, and `labels` keyed by question id. The questions stay in the same
  `-r` request document `ask` and `map` already take, so one committed question set can
  be run, batched, and evaluated without being written three times. An unknown question
  id, a Choice label that is not a declared option, a Score label outside the legend, or
  a duplicate row id is **refused, not ignored** — a silently dropped label looks scored
  and was not.
- **A report document, `jev.eval/v1`**, on stdout with `--output json` and optionally to
  a file with `--report` (mode `0600` on Unix). It records the model alias requested,
  the concrete version that answered, the dataset fingerprint, the question fingerprint,
  the split, the objective, the threshold, and the metrics — so a later run against a
  new model version is comparable.
- **Metrics that are defensible per question type.** Noul: Brier score, log loss
  (clamped so one confident miss cannot be infinite), expected calibration error over
  ten equal-width bins, and a full threshold sweep including
  `negative_predictive_value`. Choice: accuracy, per-class precision/recall/F1, macro
  averages, a confusion matrix, the classical multi-category Brier score, and
  top-label calibration. Score: exact and adjacent agreement, mean absolute error on the
  *continuous* score, quadratic-weighted kappa, and a coverage sweep. Every ratio is
  `null` rather than `0` where its denominator vanishes.
- **`--objective`, with no default.** `maximize-f1`, `min-precision`, and `min-recall`
  select a decision cut on a noul's probability; `min-accuracy` and `target-coverage`
  select a confidence cut on a choice or a score. A noul has no confidence and `jev` does
  not invent one, so asking for a coverage objective on one is a usage error raised
  before anything is sent. Every threshold is reported with the objective that chose it
  and the rule that broke ties; the word "optimal" appears nowhere.
- **A held-out split by default whenever a threshold is selected.** The cut is chosen on
  the calibration rows and reported on the rest. The split is keyed on each row's `id`,
  so appending examples or re-sorting the file leaves existing rows where they were.
  `--seed` and `--test-fraction` control it, `--calibration`/`--test` supply the two
  sides explicitly, and `--no-split` opts out with a warning on stderr *and* in the
  report. Without `--objective` there is no split, because nothing is selected and
  nothing can leak.
- **A 95% Wilson interval beside every headline number**, and explicit warnings for a
  small sample and for a thin minority class.
- **`--limit`, `--dry-run`, `-j/--concurrency`, `--fail-fast`, and `--show-rows`** on
  `jev eval`, all meaning what they mean elsewhere. `--limit` re-fingerprints the
  dataset, so the report never describes more than it measured.
- **Exit `1` gains a third meaning**: `jev eval` returns it when no threshold reaches
  `--target`. It is the same statement the code already carried — a condition was
  evaluated and did not hold — and it lets a CI job ask "does this question still clear
  95% precision?" and branch on the answer.

### Added — `is-jev-useful-here`, a second shipped Agent Skill

- **`skills/is-jev-useful-here/`** answers one question: should a described workflow use
  Jev at all, and where exactly is the boundary? It returns a verdict of `STRONG`,
  `CONDITIONAL`, `WEAK` or `NO`, what stays outside Jev, and what would have to be
  measured before trusting it — and it is willing to conclude that Jev is the wrong tool.
  It never calls the API to decide, declares only read tools, and reads only what the
  user named. See [`docs/agent-skill.md`](docs/agent-skill.md).
- **It is separate from the `jev` skill on purpose.** `jev` is for a user who has already
  decided and needs the command; `is-jev-useful-here` is for a user who has not. The two
  contested phrasings are tested against each other in
  [`evals/skills/routing/`](evals/skills/routing/README.md).
- **Written against an observed baseline.** Five realistic fit questions were run without
  the skill first. The baseline was competent and consistently wrong in the same four
  ways: it never stated a verdict, it opened affirmatively every time including on cases
  that were conditional, it dropped the privacy consequence on two of the five — the two
  carrying customer data and user queries — and it answered a fit question with an
  implementation. Those four failures are what the skill addresses.

### Added — `jev-opportunity-audit`, a third shipped Agent Skill

- **`skills/jev-opportunity-audit/`** audits a repository for places Jev would earn its
  place and reports them ranked, each citing a path, a symbol, the code, and the caller
  that reaches it. Every finding carries one next action — `PILOT`, `INVESTIGATE` or
  `LOW PRIORITY` — with its migration complexity and whether validation data exists. A
  required section names the tempting places Jev must stay out of, and **an empty audit
  is a valid result**: a repository with no natural language in its domain gets a short
  report and no findings.
- **Diagnostic only, and bounded in what it opens.** It changes no code and calls no API.
  It also declines to read secrets, key material or anything gitignored, stays inside the
  root it was given, and quotes code and comments rather than data — reading a whole
  repository is a permission this skill is the only one to hold, and `AGENTS.md` §5 is
  the reason it is bounded in writing.
- **Evaluated against two purpose-built fixtures** in
  [`evals/skills/jev-opportunity-audit/fixtures/`](evals/skills/README.md): `pulse`, a
  support product with four real opportunities and five tempting non-opportunities, and
  `pixelsort`, numeric image code full of `classify`, `score`, `decide` and `route`
  identifiers and no judgment at all. The skill forbids inventing a product idea to avoid
  an empty result. (An early observation of the baseline doing exactly that was made
  before these cases could reach their fixtures — see the staging fix above — and is not
  quoted as evidence.)
- **Routing.** Cases in [`evals/skills/routing/`](evals/skills/routing/README.md)
  separate "give me the command" (`jev`), "would this one step benefit"
  (`is-jev-useful-here`) and "go and look" (`jev-opportunity-audit`) from each other.

### Fixed

- **`jev eval` now reports an interrupted or cut-short run as one.** The batch driver
  said it had stopped; `eval` dropped that on the floor. A user who pressed Ctrl-C got a
  report assembled from whatever finished first, exit `0`, and no sign that anything was
  missing — and with `--report`, that report was written to disk looking complete. It now
  exits `130` for an interrupt and `5` for any other early stop, and the document carries
  `rows.total`, `rows.stopped_early`, `rows.interrupted`, and a warning saying the numbers
  cover only the rows that were reached.
- **`jev eval --report` refuses to overwrite one of its own inputs.** `--report` is
  written with truncation, so `--report data.jsonl` read the dataset, measured it, and
  then destroyed it — silently, and after the requests had been paid for. Naming the same
  path as `--request`, `--dataset`, `--calibration`, or `--test` is now a usage error,
  the same guard `jev map` has between its output, review, and input files.
- **`jev eval` no longer reports a threshold chosen from no observations.** A coverage
  sweep always carries the synthetic cut at zero, so `--objective target-coverage` on a
  question whose calibration rows happened to be empty returned `threshold: 0.0` with
  `threshold_reachable: true` — a number presented as chosen, from no data at all.
- **A class the model gets entirely wrong no longer raises the macro F1.** F1 with a
  defined precision and recall of zero is zero, not undefined; returning `null` excluded
  the worst class from the macro average, so `macro_f1` went *up* when a class got worse.
  It stays `null` only where precision or recall is itself undefined.
- **`labelled` counts the rows `n` counts.** It was computed over the whole dataset while
  `n` covered only the reported side, so a healthy 70/30 split reported
  `labelled: 100, n: 30` with zero failures — which the documented rule reads as seventy
  rows having failed.
- **The small-sample warning now also watches the rows the threshold is chosen from.** It
  keyed only on the reported side, so a cut fitted to three calibration examples passed
  without a word as long as the held-out side was large. It also no longer claims an
  interval sits beside "each number": only the headline proportion carries one.
- **`--objective maximize-f1` no longer invents a `--target` in its own error.** When F1
  is undefined at every cut — which happens when the calibration rows carry only one
  label — the message said "no threshold reaches --target 0", naming a flag that
  objective explicitly refuses to accept. It now says what actually happened.
- **`jev eval --dry-run` reports its count as `records`**, the key `jev map --dry-run`
  already uses for the same thing. One schema with two names for one concept is a schema
  a consumer has to special-case per command.

- **`--dry-run` in the default text format no longer passes terminal escapes through.**
  The JSON branch escaped bidirectional overrides, zero-width characters, and `DEL`
  before printing; the text branch — which is by definition the one read on a terminal —
  pretty-printed straight to stdout. A `--dry-run` exists to rehearse a request built
  from untrusted state, so an override in that state reordered the preview on the
  reviewer's own screen and the request they approved was not the request they read.
  Both branches now escape, and the pretty form stays pretty.

### Changed

- **`jev map --help` now shows the `--require` grammar**, as every other command that
  accepts `--require` already did.
- **`--timeout` and `--concurrency` state their maximums in `--help`**, as `--retries`
  already did.
- **The bounded-concurrency loop moved to one place** (`crates/jev-cli/src/batch.rs`),
  shared by `map` and `eval`. It is subtle code — one pooled agent across workers,
  interrupt checks between records, panic-tolerant collection — and written twice it
  would have to be fixed twice. No user-visible behaviour changes; `-j` is now asserted
  to be respected rather than assumed.
- **`exit.rs` and `docs/cli-contract.md` now record that exit `1` is also `jev config
  get` on an unset key**, which was true and documented only in `docs/commands.md`.
- **`docs/threat-model.md` T8 now counts four write sinks**, not two: `--review-file`
  and `eval --report` were added without the enumeration being updated. A new threat,
  T9b, records that ground truth never reaches the API and how the types enforce it.
  The "no client-side rate limiting" gap now states the batch-scale consequence and
  names the fix that is not implemented.


- **`jev map --output-file` now refuses a file that already has rows** unless `--resume`
  is given. It previously appended unconditionally, which is a stable-surface behaviour
  change under [ADR-0003](docs/adr/0003-cli-compatibility.md): a script that re-ran the
  same command to accumulate rows in one file now exits `2`.

  It changed because the old behaviour was silently destructive of meaning rather than
  useful: running the same command twice left every record in the file twice with no
  warning, and a later `--resume` then read a file whose rows came from two different
  runs and could not tell. The error names all three remedies — remove the file, pass
  `--resume`, or name a different one. An empty or absent file is unaffected, so a first
  run is unchanged, and `--resume` is unchanged.

  `--review-file` has the same rule, but it is new in this release and so breaks nothing.

### Added — semantic routing, and a request schema

- **`jev map --require EXPR`** classifies each answered row with the same expression
  language `jev noul --require` gates on. In `map` it **routes rather than gates**: it
  never changes the exit code, which keeps reporting whether the API answered. Every row
  gains a `gate` object (`expression`, `outcome`, `reason`), and the summary gains
  `gate` counts. `null` when no expression was given, so a consumer filtering on
  `.gate.outcome` sees a missing verdict rather than a missing field.
- **`jev map --review-file PATH`** diverts the rows that did not pass, so the main stream
  is the set you were willing to act on automatically. Rows that are `unevaluable` go
  there too — a gate that could not be evaluated is never treated as one that passed,
  the same rule that keeps exit `6` apart from exit `1`. A row that *failed* is never
  diverted: "the API did not answer" and "the API answered and the answer needs a look"
  are different problems. Without the flag nothing is discarded; every row still reaches
  the main stream, annotated. Created `0600` on Unix, like `--output-file`, and
  `--resume` reads both files so a reviewed row is not re-sent or re-billed.

  There is deliberately no `--min-confidence` and no synthesized Noul confidence. The API
  returns confidence only for Choice and Score, computed from the `probabilities` already
  in the answer; a Noul has none, `<id>.confidence` on one is `unevaluable`, and a Noul's
  uncertainty is written as a band on its probability. No threshold has a default. See
  [ADR-0010](docs/adr/0010-batch-evaluation-and-semantic-routing.md) §5.
- **`schema/request.schema.json`** — a JSON Schema (2020-12) for the request-file format,
  with `$id`, for editor completion and inline validation. The format carries no `version`
  field and will not: the document *is* the official API request body, and a key the API
  does not define would make the file invalid as one. `scripts/check-request-schema.py`,
  run by `scripts/verify.sh` and CI, keeps the schema and the parser in step — every
  committed example must validate and eleven documents `jev` refuses must not.
- **`examples/`** — six recipes (issue classification, semantic log filtering, a
  pull-request risk gate, RAG candidate relevance, dataset triage with a review file, and
  agent action classification) with five committed request files, each tested against both
  `jev ask` and `jev map`. Examples, not commands.
- **[ADR-0010](docs/adr/0010-batch-evaluation-and-semantic-routing.md)** records the JSONL
  framing contract ADR-0003's revisit trigger asked for, and answers the confirmation-path
  question `docs/threat-model.md` T9 requires for a bulk-content feature: `--dry-run`,
  `--state-field`, a pre-flight count on stderr, and `0600` row files rather than a prompt
  that every piped invocation would skip.
- `jev.map.row/v1` gains `request_digest`.

### Fixed — found by an adversarial audit

- **`--resume` refuses a changed question set or model.** It compared the input only, so
  editing the prompt after a disappointing first run and resuming produced a file whose
  early rows answered one question and whose later rows answered another — then reported
  the batch complete and exited `0`. Rows now carry a `request_digest` over the questions
  and the model. A row without one, from an earlier version, still resumes.
- **A rejected credential stops the batch and exits `3`.** It was classified per row, so
  `jev map` kept going, sent one doomed request for every remaining record — up to a
  million — and reported the result as a partial batch at exit `5`. A CI job branching on
  `3` to re-authenticate never saw it.
- **`--output-file` refuses a file that already has rows**, unless `--resume` is given.
  Append was unconditional, so running the same command twice silently left every record
  in the file twice, and a later `--resume` read a file whose rows came from two runs.
- **`--output-file` may not be the same path as `--input`.** The input is read fully
  before the sink opens, so there was no loop — result rows were just appended onto the
  input file, leaving it silently no longer valid as input. `--review-file` may not equal
  `--output-file` either.
- **`--require`'s `==` and `!=` on numbers are exact.** They used `f64::EPSILON` as an
  *absolute* tolerance, which inverted across the range: near zero it was enormous in
  relative terms, so `x == 0` and `x > 0` both held for a probability of `1e-17`; above
  about `2` it was smaller than one ULP and degenerated to exact equality anyway. One
  operator cannot mean two things depending on the magnitude of its input. `==` and `!=`
  are now complementary and consistent with `>=`/`<=`.
- **A duplicate field inside a question is a load error.** The top level and the
  question-id level both rejected duplicates; one level deeper the policy silently
  reversed, so `{"type": "noul", "type": "choice"}` sent a Choice without a word of
  complaint. Duplicate Choice option names were last-one-wins for the same reason and are
  now refused too.
- **A Choice's options keep the order they were written in.** `serde_json::Map` is a
  `BTreeMap`, so options written zebra, apple, mango were parsed apple, mango, zebra —
  and option order is something a model can be sensitive to. Asserted against a real
  socket, because every JSON value type available to a test is itself sorted.
- **A malformed question is reported instead of a nonsense error about `state`.** The
  "is this a full document?" test required *every* question to have a `type`, so one
  question missing it fell through to the bare-questions branch and reported
  ``question `state` must be an object`` — blaming a key that was completely correct.
- **`jev map` explains its own input limit.** The per-source byte ceiling applies to the
  whole JSONL file, and its message — "a truncated state produces a confident answer to a
  question you did not ask" — described something that was not happening, with thousands
  of separate states none of which would be truncated. It is also `map`'s real batch
  ceiling, biting long before `MAX_RECORDS`.
- **A question id that a gate cannot address says so.** An id beginning with a digit is
  legal but lexes as a number, and the error blamed the grammar — the one thing the user
  cannot change — rather than naming the id.

### Fixed — found by three independent review passes

- **`jev map` now honours the request document's `model`.** It read the session's model
  directly and discarded the document's, so a committed request file naming a model was
  billed on a different one — while `docs/commands.md` promised the same file works with
  `ask` and `map` alike. `ask` had always reconciled the two; `map` never did. It also
  made the new `request_digest` blind to the one edit it exists to catch: changing
  `model` inside the file moved nothing, so `--resume` accepted a file answered by
  another model.
- **`--review-file` gets the same "already has content" refusal as `--output-file`.**
  It was still an unconditional append, so a user iterating on a threshold accumulated
  every previous run's rows in the file a person is supposed to open.
- **A resume that may be missing a review file says so.** Drop `--review-file` from an
  otherwise identical resumed command and the diverted records looked unevaluated: they
  were sent again, billed again, and written to the output file, so the same index ended
  up in both files and a later resume papered over it. Rows record whether the run that
  wrote them was classifying, and a resume without `--review-file` warns when they were.
- **A full request document may contain a question called `type`.** The full-versus-bare
  heuristic tested that `questions.type` *exists* rather than that it holds a type name,
  so such a document took the bare branch and blamed a key the user did not write — the
  same class of error as the `state` fix above.
- **A gate error is one readable line again.** A missing line continuation left a
  32-space run in the middle of the message, and the number that caused it had been
  replaced by the words "a number", losing the clue. A numeric literal on the left of a
  comparison now names both causes and echoes the value.
- **The three broken cookbook recipes.** The routing example asked a **Noul** for a
  `confidence`, so every row was `unevaluable`, every record went to the review file, the
  main stream was empty and the run exited `0` — the exact mistake the page above it
  warns against. The pull-request gate piped state into `-r`, which only `--questions`
  accepts, so it exited `2` every time and the CI snippet failed on every run. The log
  recipe added `1` to `.id`, which is a string, so `jq` errored per line and the loop
  silently printed nothing. `scripts/check-examples.py`, run by `scripts/verify.sh` and
  CI, now validates every `--require` in the cookbook against the question types of the
  request file on the same command line — prose warning people off a mistake did not stop
  it being committed in the example that demonstrates the feature.
- **The request schema was materially laxer than the parser**, so a file could validate
  green in an editor and be refused at run time: whitespace-only names and content,
  control characters, an over-long model, and noul criteria describing neither side.
  `scripts/check-request-schema.py` now tests both directions — 23 documents that must be
  rejected and 8 that must be accepted, including the two request-file shapes whose
  classification is ambiguous.
- **The `--resume` fingerprint ignored Choice option order.** It rendered each question
  into a `serde_json::Value` before hashing, and `Value`'s object type is a `BTreeMap` —
  so it sorted the option names before hashing the one thing `jev-core`'s hand-written
  serializers exist to preserve. Reordering the options changed what was sent and did not
  change the digest, so `--resume` accepted a file answered by a different request. The
  digest is now built from serialized text directly.
- **`--resume` validates each file before merging them.** With both `--output-file` and
  `--review-file`, a good review row overwrote a conflicting output row at the same index
  and the run exited `0` reporting the batch complete, leaving the incompatible row on
  disk. Each file is checked on its own first.
- **The pre-flight record count is printed at ordinary verbosity.** ADR-0010 and
  `docs/threat-model.md` T9 both name it as one of the controls standing in for a
  confirmation prompt on a bulk-send command, and it was written with the verbose-only
  helper — a visibility control nobody saw. `--quiet` still silences it.
- **A Choice option name may not contain a control character.** `QuestionId` rejected
  them from the beginning and option names did not, so the two halves of one document
  were held to different standards — and an option name becomes a JSON key in
  `probabilities`, comes back as the answer's `choice`, and is what a `--require` gate
  compares against.
- **The cookbook's CI snippet reached its own handler.** A GitHub Actions `run:` block is
  `bash -e`, so the pipeline's non-zero exit ended the step before `case $?` ran —
  collapsing "the policy says no" back into "the job failed", the one distinction the
  example exists to draw. The status is now captured.
- **Three checks passed while the thing they guard was broken.**
  `scripts/check-examples.py` matched only single-quoted `--require`, so rewriting the
  cookbook's gate with double quotes made it stop looking; `scripts/check-cli-docs.py`'s
  regex accepted only `pub` and `pub(crate)`, so a `pub(super)` flag was invisible to it
  *and* to the floor meant to catch that; and `scripts/check-request-schema.py` tested
  that 255 Choice options and 10 Score levels are accepted but never that 256 and 11 are
  refused, so deleting the bounds from the schema left it green. All three now fail on
  the mutation that exposed them.
- **The schema disagreed with the parser in three more places**: C1 control characters
  were outside the rejected range, Choice option names were held to a rule the parser did
  not apply, and `maxLength` on `model` was measured before trimming, so a padded name
  `jev` accepts was red-underlined. A schema that rejects a valid file is worse than a lax
  one, so the trimmed-length cap is documented rather than approximated.
- **`==` and `!=` being exact is now documented** where the grammar is, with the
  recommendation to prefer a range for anything the model computed. `--output-file`'s
  refusal, both row files being in completion order, and stdout producing nothing until a
  batch ends were all true and none were written down.

### Internal

- `scripts/check-cli-docs.py` matched an `#[arg(...)]` body as "anything with no `]`",
  which cannot match an attribute that contains a bracket. Five real flags — `--state`,
  `--state-file`, `--state-json`, `--state-json-file` and `--option` — were invisible to
  the check that exists to keep flag names documented. All five happened to be
  documented, so nothing failed, and a new flag declared with `conflicts_with_all` would
  have been exempt silently. The regex is fixed (32 → 37 flags found) and a floor now
  fails the check if the count drops, because "found 32 of 37" looks exactly like success
  and only "found none" tripped the old guard.
- `crates/jev-cli/src/ordered.rs`'s order- and duplicate-preserving parse is now generic
  in the value type, so the guarantee reaches inside a question body and a `criteria` map
  instead of stopping two levels up.
- `README.md`'s "no published release, no package, and no version number yet" contradicted
  `0.1.1` three sections later, and `docs/benchmarks.md` described a 201-line script as
  "about eighty lines".
- Tests: 473 → 517. New coverage for gate routing and every gate outcome, the review
  file's contents and its `0600` mode, a diverted row on resume, a rejected credential
  stopping a batch, a changed question set and a changed model on resume, output/input
  path collisions including `./x` against `x`, an output file that already has rows,
  duplicate fields and option names inside a question, Choice option order asserted
  against a real socket, the full-document heuristic in both directions, exact numeric
  equality, a recursive parse at 60/100/1000/100,000 levels of nesting, and both
  composition directions for every committed example.
- `scripts/bench.sh` isolates JSONL parsing from output serialization (`--lines` versus
  JSONL, stdout versus `--output-file`) and measures two input sizes, so scaling and
  memory growth are stated with data rather than assumed. The CI step that validates the
  request schema pins its one dependency.

Hardening of the four foundations — machine output, authentication, network behaviour,
and `--dry-run` — driven by four independent audits and two independent reviews of the
resulting change. Nothing here changes a command name, a flag, or an exit code; every
JSON change is an addition.

### Security

- **On Windows the stored API key no longer roams.** `windows-native-keyring-store`
  creates credentials with `Enterprise` persistence, which writes them to the user's
  roaming profile — so on a domain-joined machine the key followed them to every other
  machine they signed in to. macOS and Linux both store locally, and ADR-0002 described
  OS-native storage as a *local* secure store throughout; the roaming was a library
  default nobody had recorded a decision about. `jev` now passes `persistence = Local`.
  Anyone who already ran `jev auth login` keeps the roaming credential until they log in
  again, because persistence is fixed when the secret is written.
- **An empty `APPDATA` or `HOME` no longer puts the configuration in the working
  directory.** Only `XDG_CONFIG_HOME` filtered an empty value, so `APPDATA=""` — real in
  Windows service and scheduled-task contexts — resolved to the *relative* path
  `jev\config.toml`, and `HOME=""` did the same on macOS and Unix. That contradicts the
  invariant this project states in three places and tests under the name
  `nothing_in_the_working_directory_is_ever_consulted`, which only ever covered the
  populated case. All three branches now filter, and a base that is not absolute is
  refused outright.

- **Proxy variables are no longer inherited.** `ureq`'s default configuration reads
  `HTTP_PROXY`, `HTTPS_PROXY`, and `ALL_PROXY`, so a variable `jev` neither documented
  nor reported decided where every request went. For a loopback endpoint — the one case
  `jev` permits cleartext, justified by "there is no network to observe" — the agent
  opened a `CONNECT` tunnel to the proxy and sent the `Authorization` header through it
  in the clear. Proxy support is now off, and `docs/cli-contract.md` says so.
- **The `os-keychain` feature is now genuinely separable from the binary.** `jev-cli`
  depended on `jev-config` with its default features on, so `--no-default-features`
  still linked the credential store and the keychain-less build `jev-config` documents
  was not reachable through `jev` at all. `jev-cli` now has its own `os-keychain`
  feature that forwards, and the build without it refuses `jev auth login` with the
  documented pointer to environment authentication.
- **JSON output escapes the characters a terminal acts on.** `serde_json` leaves
  `U+007F`, the bidirectional overrides and isolates, the zero-width characters, and
  `U+2028`/`U+2029` raw, so API-supplied text in a machine document could reorder how a
  line reads (Trojan Source), hide text, or split one JSONL record into two. They are
  now written as `\uXXXX`, which parses back to the identical string — the value a
  consumer receives is unchanged.

  This covers every path that writes a document, including `jev map --output-file`,
  which wrote raw bytes and was therefore the one place the hazards survived, in the
  artifact most likely to be read in a terminal days later. `U+2028` and `U+2029` are
  in the set because they are `Zl`/`Zp` rather than `Cf`, and a list built from `Cf`
  alone missed them.
- **`--dry-run` no longer reads a credential.** Reporting credential availability had
  been wired to the resolving check, which opens the file named by `JEV_API_KEY_FILE`
  and queries the OS credential store — on macOS raising an unlock prompt from the one
  command that promises to touch nothing. It now inspects the environment's shape only,
  and says so in the document.

### Fixed

- **`jev map --resume` no longer corrupts the output file.** A killed run leaves a
  truncated final line; the append-mode writer concatenated the next row onto it. The
  re-evaluated record was billed, answered, and written into a line that parsed as
  neither row, while the summary reported the batch complete and exited `0`.
- **`--resume` refuses an output file produced from different input.** It matched on
  input position alone, so an edited, filtered, or re-sorted input silently skipped
  records that had never been evaluated and still exited `0`.
  Rows now carry a `state_digest`, and both it and the row id are compared. Comparing
  ids alone would not have been enough: without `--id-field` the id *is* the position,
  so the comparison was a tautology in the configuration nearly every run uses. A
  result for an index the current input cannot contain is refused for the same reason —
  counted as resumed, it inflated `total` and `complete` and reported a shrunken batch
  as finished.
- **Ctrl-C during a retry backoff stops the run and exits `130`.** The wait was a bare
  `thread::sleep`, so a `Retry-After: 30` made the process deaf for thirty seconds per
  attempt and the interrupt was swallowed entirely: the run finished every remaining
  attempt and exited `4`.
- **`--timeout` is bounded at 3600 seconds.** `--timeout 9223372036854775807` overflowed
  `Instant + Duration` and **panicked** with exit `101`; lower absurd values hung
  indefinitely and defeated the bound that makes the retry loop provably terminate. A
  flag beyond the ceiling is now a usage error; a configuration-file value is clamped.
- **`--value` really does print one line.** A Choice's selected option is API-supplied
  text and could contain a newline or a tab, which `sanitize` preserves by design. A
  `$(jev … --value)` capture could hold an embedded newline.
- **`NO_COLOR` now overrides `color = "always"` in the configuration file**, as
  `docs/cli-contract.md` already promised. `--color always` still overrides `NO_COLOR`.
- **A response body that stalls after the headers is reported as a timeout**, not as
  "could not reach the API endpoint: other error". The endpoint had been reached, and
  the message pointed away from `--timeout`, the one thing that would have helped.
- **A `jev map --output-file` this run creates is `0600` on Unix.** Left to the umask it
  came out world- or group-readable, while the configuration file was deliberately
  `0600` — and the rows hold the model's answers about the user's state, which the
  threat model lists among the assets worth protecting. A file the user already created
  keeps their permissions.
- **`jev map` classifies an undecodable response as `unavailable`**, matching the
  single-request path. Reporting it as `request` told a consumer that retries
  `unavailable` to give up on a transient API fault.
- **A credential store that fails is no longer reported as an empty one.** A locked or
  cancelled Keychain, or a duplicate Secret Service entry, surfaced from `jev ask` as a
  plain "no TypeSafe API key found" that listed the store among the places looked — and
  advised running `jev auth login`, which writes to the store that had just failed. The
  reason is now kept and reported, and the advice points at environment authentication
  instead. `jev auth status` does the same in a build compiled without a store.
- **Duplicate credential entries get their own message.** Two Secret Service items
  matching the same service and account make the credential unreadable *and*
  unremovable, so `jev auth logout` cannot clear it either. That was reported as
  "secure credential storage is unavailable on this system", sending the user to look
  for a broken keyring rather than a duplicate.
- **`jev doctor` and `jev auth status` no longer report a broken credential as an
  absent one.** A blank `JEV_API_KEY` read as "not set" and an unusable
  `JEV_API_KEY_FILE` as "present, in use: none", each offering remediation for a problem
  the user did not have.

### Added

- `jev.evaluation/v1` and `jev.map.row/v1` gain `request_id`, the API's own identifier
  for the call from the `x-typesafe-request-id` response header. The official SDK
  appends it to every API error; `jev` discarded it, so a user whose batch row failed
  had nothing to hand TypeSafe support and no way to recover it afterwards. It is on
  failed rows too, which is where it matters. `null` when the API sent no header.

- `jev.dry-run/v1` gains `body_bytes`, `credential` (availability and source name, never
  a value), `headers` on the `map` form, and `sample_truncated`. The document is now
  built by the same function that builds a real request, so a dry run cannot describe a
  request a live run would not send — pinned by a test that compares the two.
- Under `--dry-run` the transport is replaced by one that refuses every request, making
  "nothing leaves the machine" structural rather than five call sites remembering a
  boolean.
- `jev.map.row/v1` gains `state_digest`. A row without it, from an earlier version,
  still resumes.
- `jev.dry-run/v1` gains `gate`, the parsed `--require` expression, and `--value` under
  `--dry-run` now says on stderr that it has nothing to print rather than silently
  emitting a document.
- `jev.doctor/v1` gains `credentials.error` and `jev.auth/v1` gains `error`, which
  distinguish "nothing is configured" from "something is configured and broken".
- `JEV_NO_KEYCHAIN` makes `jev` report secure storage as unavailable without consulting
  it, so the integration suite stops querying a developer's real keychain. It reduces
  capability only — no credential is supplied, redirected, or written in plaintext — and
  it is in `docs/cli-contract.md`'s environment table, because that table promises `jev`
  reads exactly the variables it lists.

### Internal

- Tests: 429 → 473. New coverage for the proxy, dry-run on every sending command
  (including `map`, which had none), dry-run/live body equality, interrupt during a
  backoff, resume against a truncated line and against changed input, `--value` line
  discipline, `NO_COLOR` precedence, timeout bounds, `--fail-fast` (a user-facing flag
  that had none), a response body that stalls mid-read, surrogate-pair escaping, and
  a connection closed mid-response, and golden top-level key sets for every JSON
  document `jev` emits.
- The credential canary in `.github/workflows/security.yml` now sets all five credential
  environment variables — `TYPESAFE_API_KEY` and both `*_KEY_FILE` forms were never
  exercised — and scans the files `jev` writes, not only stdout and stderr. A Rust test
  fails if a source or the file scan is dropped again.
- `scripts/verify.sh` and CI build and test with `--no-default-features`. The
  `os-keychain` feature is separable and documented as supported, and nothing compiled
  that configuration.
- `scripts/check-cli-docs.py`, run by `scripts/verify.sh` and CI, compares the clap
  definition against `docs/commands.md` and `docs/cli-contract.md` in both directions.
  Command, flag, and environment variable *names* are a compatibility promise
  (ADR-0003) and nothing enforced it: a flag could be added, renamed, or removed with
  the documentation still describing the old surface. It also keeps
  `docs/cli-contract.md`'s "jev reads exactly these variables and no others" true,
  which is a sentence only a check can keep true.
- `docs/research/comparison.md` carries a dated correction: its claim that "there is no
  code path by which a production key reaches another host" was false when written, and
  the inherited-proxy path is why. The original paragraph is left as it was — a research
  snapshot that quietly becomes correct is worth less than one that shows where it was
  wrong.
- `docs/cli-contract.md` now states that `jev completions` ignores `--output` and that
  `jev map` always writes JSONL, including under `--output text`. Both were true and
  neither was written down.
- The `schema` identifiers are cross-checked against `docs/output-schema.md` and
  `docs/cli-contract.md` in both directions, so a document cannot be added, renamed, or
  removed without the documentation following.

### Internal — skill-authoring toolchain

Nothing here reaches the `jev` binary, its dependencies, or a released artifact. It is
development tooling for authoring the Agent Skills in `skills/`.

- **`scripts/skill-authoring-setup.sh`** provisions the authoring toolchain from
  `skill-authoring-lock.json`: Anthropic's official `skill-creator`, and four skills from
  `obra/superpowers` (`writing-skills`, `test-driven-development`,
  `verification-before-completion`, `dispatching-parallel-agents`). It clones each
  upstream at a pinned commit into the gitignored `references/07-skill-authoring/`,
  verifies the commit, and copies the skill directories verbatim into `.claude/skills/`
  with their licence and a generated `PROVENANCE.md`. Nothing from those clones is
  executed, nothing is committed, and nothing outside the repository is written.
- **`scripts/skill-eval.sh`** runs the shipped skills' eval suite through
  `claude plugin eval`. A default run takes each case twice — with the skills and with no plugin at all
  — so the RED baseline is measured rather than assumed. It is deliberately **not** part
  of `scripts/verify.sh`: every case is a real model call on the operator's own
  credential.
- **`evals/skills/`** holds the cases. The first four cover the `jev` skill's triggering
  in both directions, including a near miss that must *not* fire, and two behaviours.
  `evals/skills/routing/` carries the procedure and template for cross-skill trigger
  collisions, which is what the five-skill suite needs.
- **`scripts/validate-skills.py`** now enforces the open Agent Skills specification
  rather than an approximation of it: only the frontmatter keys the specification
  defines, `compatibility` length, a 500-line ceiling on `SKILL.md`, a lowercase
  `skill.md` reported instead of silently skipped, and `allowed-tools` **space**-
  separated. That last one found a real portability defect — every skill in the
  repository, including the shipped one, used the comma-separated form, which Claude
  Code accepts and the specification does not. All five were corrected. Third-party
  skills carrying a `PROVENANCE.md` are skipped, but **only under `.claude/skills/`**,
  which is the one tree the setup script may write to; everything under `skills/` is
  validated unconditionally, because otherwise one extra file next to a `SKILL.md` would
  have been enough to walk an unchecked shipped skill past the gate.
  `skill-authoring-setup.sh --check` verifies the third-party copies against their
  pinned commit instead.
- `scripts/verify.sh` gained `shipped skill manifest`, a second opinion from
  `claude plugin validate --strict`, tool-gated so a contributor without Claude Code
  sees `skip` rather than a failure.
- A security review of the above found four things, all fixed here: the `PROVENANCE.md`
  skip above; a malformed lockfile that printed a traceback and then reported success,
  because `exit` inside a process substitution leaves only that subshell; a licence file
  copied without the symlink guard that covers the rest of the tree; and an empty
  `PASSTHROUGH` array that would abort `skill-eval.sh` under `set -u` on bash 3.2.
- `AGENTS.md` §14 and `docs/development/skill-authoring.md` make the method binding: no
  skill without an observed failure, the specification as the portability authority, and
  eval evidence rather than "the Markdown validates" as the completion bar.


## 0.1.1 — 2026-09-20

The first tagged development version. An earlier dry run found that the Windows build
ran its steps under PowerShell, where `"$TARGET"` expands to nothing. No public artifact
was published for this version. **Deliberately pre-1.0:** the compatibility promises in
[`docs/cli-contract.md`](docs/cli-contract.md) take effect at `1.0.0`, and nothing here
has been exercised by anyone outside the project yet. Exit codes and JSON documents may
still change before `1.0.0` — read the schema field, and pin the version if you script
against it.

### Added — the command surface

- `jev noul`, `jev choice`, and `jev score` — the three System One primitives, each
  taking state from a flag, a file, or stdin, and returning the full probability
  distribution.
- `jev ask` — several independent questions about one state in a single request. The
  request document is the official API request body, so an example from
  <https://docs.typesafe.ai/api> runs unchanged.
- `jev map` — one question set over many records. JSONL in and out, input order
  preserved, bounded concurrency, independent row failures, and `--resume` that reads
  the output file rather than caching model judgments.
- `jev models` — the model list from `GET /v1/models`, with no hard-coded catalogue.
- `jev doctor` — configuration, credentials, endpoint, and model, with no network
  request unless `--live` is given.
- `jev auth login | status | logout` — OS credential store, hidden prompt, no plaintext
  fallback.
- `jev config list | get | set | unset | path` — non-secret settings only.
- `jev completions` — bash, zsh, fish, PowerShell, and elvish.
- `--require` — a small, total expression language that turns a model judgment into an
  exit status. No shell, no `eval`, fuzzed and property-tested.
- `--dry-run` — prints the exact request body that would be sent, and sends nothing.
- `--value` — one scalar for shell use.
- `--output json` — one newline-terminated document per invocation, every one carrying
  a versioned `schema` field.

### Added — infrastructure

- A blocking HTTPS transport over `ureq` and `rustls`, with retries, exponential
  backoff, jitter, `retry-after` handling, a total time budget, and a bounded response
  size. No async runtime.
- OS credential store support: macOS Keychain, Windows Credential Manager, and Secret
  Service on Linux, via `keyring-core` plus one target-gated store crate.
- Compatibility fixtures recorded from official TypeSafe documents, with tests
  asserting both request encoding and response decoding
  (`crates/jev-client/tests/fixtures/`).
- Five fuzz targets over every place attacker-influenced bytes enter, each asserting a
  domain invariant rather than only the absence of a crash (`fuzz/`).
- An agent skill for operating this CLI (`skills/jev/`), separate from and deferring to
  TypeSafe's official skill.
- `scripts/bench.sh`, `scripts/fuzz-smoke.sh`, and `scripts/release-dry-run.sh`, all
  wired into `scripts/verify.sh` and CI.
- Release machinery: `dist` as a builder behind a hand-written, SHA-pinned manual
  workflow; checksums verified after they are written; an SPDX SBOM; optional
  provenance; and an install smoke test that unpacks the archive and runs the binary.
- Packaging templates for Scoop and WinGet (`packaging/`).
- `scripts/check-installers.py`, which fails the build when the generated shell
  installer or Homebrew formula would install an archive **without verifying its
  checksum** — a state `dist` reaches silently when the per-target
  `*-dist-manifest.json` files are missing, as they were.
- `scripts/check-workflows.py`, which asserts that every workflow tool is pinned, that
  the `cargo-dist` pin matches `dist-workspace.toml`, that `allow-licenses` matches
  `deny.toml`, and that the packaging manifests are still templates.
- `fuzz/seeds/`, a small committed seed corpus. It is what makes a short CI fuzz run
  start from a real parse, and where an input that once found a bug is replayed forever.
- Documentation: command reference, output schema, API compatibility, troubleshooting,
  release verification, benchmarks, and agent-skill installation.

### Changed

- **MSRV raised from 1.85 to 1.88.** The Secret Service credential store requires it.
- **Credential resolution order is now environment before keychain**, and
  `TYPESAFE_API_KEY` is honoured. See
  [ADR-0008](docs/adr/0008-credential-precedence-and-endpoint-isolation.md); this
  supersedes the ordering in ADR-0002. Nothing else about credential handling changed,
  and there is still no plaintext fallback.
- `deny.toml` allows `CDLA-Permissive-2.0`, the licence of the Mozilla CA certificate
  set `rustls` verifies against. It is a permissive data licence and imposes no
  copyleft obligation.
- The credential store is opened lazily. It was previously opened on every invocation,
  including `--version`, which cost about 6 ms and contacted D-Bus for no reason.

### Security

- **A loopback check that a resolver could disagree with is closed.** `is_loopback` read
  an IPv4 octet with Rust's `parse::<u8>`, which ignores leading zeros, while
  `getaddrinfo` reads a leading zero as an **octal** marker. So
  `http://0000127.00000000000012.077.2` was accepted as loopback and permitted plain
  HTTP, and the kernel then resolved it to `87.10.63.2` — a public address. A credential
  would have crossed the internet in cleartext with no warning. An octet is now accepted
  only as one to three digits with no leading zero. Found by the `endpoint_url` fuzz
  target; see `docs/threat-model.md` T4.
- `jev_config::Secret` redacts on `Debug` and `Display` and zeroizes on drop.
- A credential is passed to `Transport::execute` as a non-`Clone` `Credential` rather
  than carried in the request structure, so a retry path cannot duplicate plaintext.
  This closes the open follow-up recorded under T2 in the threat model.
- **A TypeSafe credential is structurally unreachable from a non-official endpoint.**
  Those use `JEV_CUSTOM_API_KEY`, and the OS store is not consulted for them.
- Plain HTTP is refused except for unambiguous loopback; `localhost.evil.example` is
  not loopback. Redirects are never followed.
- The configuration file cannot hold a credential: a key whose name looks like one, at
  any nesting level, is a load error.
- API- and file-supplied text is sanitized before display — ANSI escapes, bidirectional
  overrides, and zero-width characters.
- Responses are bounded in size and nesting depth, rejected rather than lossily
  decoded, and checked for internal consistency: a Choice absent from its own
  distribution, or a Score off its own scale, is refused.
- Input is rejected before a request is built when it is empty, non-UTF-8, binary,
  oversized, or malformed. It is never silently truncated.
- The CI credential canary now runs every subcommand with and without `--verbose`, with
  both credential namespaces set, and over the dry-run, custom-endpoint, and live
  paths.

### Fixed

- **The Windows release build ran its steps under PowerShell**, where `"$TARGET"` is a
  PowerShell variable rather than an environment one, so `rustup target add ""` failed
  with "does not contain component `rust-std` for target `''`" and the `mktemp` in the
  next step would not have existed either. The build job now pins `shell: bash` for
  every step on every platform. Found by the first release dry run.
- **The "no native TLS stack" check was feature-blind and failed for an openssl that is
  never compiled.** It grepped `Cargo.lock`, which lists a package for every *optional*
  dependency any crate declares, selected or not. `dbus-secret-service` declares
  `openssl` behind `crypto-openssl`; this workspace selects `crypto-rust`. The check now
  resolves features with `cargo tree`, and says so loudly enough that nobody re-adds the
  grep.
- **A core source file was never committed.** `.gitignore` carried `credentials.*` in
  its secrets section; the pattern is unanchored, so git applied it at every depth and
  it matched `crates/jev-config/src/credentials.rs` — the module that resolves API
  credentials. The file was on disk, so every local build, every test, and
  `scripts/verify.sh` passed for four commits. Only CI, which builds what was actually
  pushed, failed. Source files are now re-included explicitly, and
  `scripts/check-tracked-sources.py` fails the build if an ignore rule swallows one
  again.

- **A failed write of `jev map --output-file` was reported as a partial batch in which
  nothing had failed.** The error was recorded in a flag the caller never read, so a
  full disk produced exit `5`, a summary saying `failed: 0`, and an invitation to
  `--resume` from a file that could not be written. It is now exit `74` with the reason.
- The same write failure was classified as an internal error, which told the user they
  had found a bug in `jev` and sent them to the issue tracker for a full disk.
- `io::ErrorKind`'s own wording reached users on the output-file paths — a missing
  directory reported "entity not found". All read *and* write paths now share
  `io_reason`, which says "no such file or directory".
- **`jev models` printed a ragged table.** The name column was padded with
  `{:<width$}` around a `Safe`, whose `Display` wrote the string directly instead of
  calling `Formatter::pad` — so width, fill, alignment, and precision were all silently
  ignored. It compiles and looks correct, and every fixture hid it because their model
  names happened to be the same length; the live API, which returns `jev-latest` and
  `jev-preview`, did not. `Safe` now honours the format spec.
- `--help` for `--color auto` said colour follows **stderr**; it follows stdout.
- The credential canary's coverage test matched subcommand names by substring, so
  `"conf"` satisfied `"config"` and a bare `"auth"` satisfied every nested subcommand.
  It now compares sets and derives nested subcommands from `--help`; the canary
  workflow exercises `auth login|status|logout` and all five `config` subcommands.

### Exit codes

Three codes added; no existing code changed meaning.

- `5` — a batch finished with some rows failing.
- `6` — a `--require` gate could not be evaluated. Deliberately distinct from `1`: a
  broken gate is not a negative judgment, and must never be read as a pass.
- `74` (`EX_IOERR`) — output could not be written. Deliberately distinct from `70`: a
  full disk is the environment's problem, not a bug report.

### Not claimed

Stated here because absent guarantees are the ones people assume. See
[`docs/release-verification.md`](docs/release-verification.md).

- **No PowerShell installer.** `dist` 0.32.0 generates one that verifies no checksum at
  all, which contradicts this project's own security posture, so it is not built. On
  Windows use Scoop or WinGet, or the `.zip` plus `sha256.sum`.
- **Binaries are not code-signed or notarized.** Expect a SmartScreen or Gatekeeper
  prompt.
- **Builds are not verified reproducible.** The toolchain is pinned, paths are remapped,
  `SOURCE_DATE_EPOCH` is set, and the graph is locked — but it has not been demonstrated
  bit-for-bit, so it is not claimed.
- **No published artifact or attestation.** The manual release workflow can request
  provenance for a future release, but publication does not wait for the optional
  attestation job. Check each release before claiming provenance.
- Exit `70` is locked by a unit test but has no end-to-end assertion; its only call
  sites are unreachable with valid domain types.
