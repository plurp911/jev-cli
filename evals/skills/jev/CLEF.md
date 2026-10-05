# Clef skill evaluation contract

These synthetic cases exercise CLI recommendations, not live inference. They use
only read-only tools; no key material, user data, model downloads, or provider
requests are part of the cases. Their declared three repeats and strict graders
remain the normal confirmation contract.

Inputs are concrete user requests for provider-specific commands, authentication,
local runtime setup, explicit image/prepared-video input, processor controls, and
MCP startup. Outputs are usable commands plus the relevant endpoint, privacy,
credential, and capability boundaries. The skill does not install runtimes,
decode video, determine current account prices, or replace deterministic image
metadata inspection.

The 15 added cases cover eight behaviors (`clef-credentials`, `clef-privacy`,
`clef-local-setup`, `clef-image`, `clef-video`, `clef-advanced-controls`,
`clef-mcp`, `clef-publisher-inputs`), two positive triggers, three nearby negative triggers, and two
cross-skill routing cases. The negative cases deliberately mention installed
Clef/jev while asking only for exact image dimensions, video decoding, or account
pricing. Naming the product must not override the task boundary.

An observed local-setup failure preceded the reference correction: the loaded
skill response said, "The skill doesn't give the bridge's setup path," and could
not identify the Windows System One issue or the appropriate llama.cpp GGUF
conversion. A separate no-plugin credential arm could not supply the custom
credential boundary; the loaded skill arm did. Exact model, commands, quotes,
run counts, and costs are recorded in
[the evidence receipt](../../../docs/development/clef-skill-verification.json).
These one-repeat pilots establish a failure; they are not confirmation results.

Run the full training set and routing cases, then the untouched held-out set
once the final candidate is fixed:

```sh
scripts/skill-eval.sh -- --concurrency 8
scripts/skill-eval.sh --heldout -- --concurrency 8
```

Both commands retain baseline-versus-skill arms and three repeats, with GPT-6.1 Sol
at high effort for every measured and independent judge session. They use existing
ChatGPT login and refuse API-key auth. Token usage is reported; dollar billing is
unavailable. A transport error or missing runtime is incomplete evidence, never a pass.
Inspect every failed rubric and its transcript before changing the candidate.
Never weaken an existing case or edit a held-out case (prompt, grader, fixture, or metadata) after viewing its result.

For representative native Codex behavior, use fresh isolated read-only sessions
with the same synthetic prompts and the shipped skills under `.agents/skills`.
Compare against a workspace without that catalog and record actual skill-file
reads. These pairs exercise native discovery for those requests; the full corpus
uses the instrumented app-server catalog. Claude has no fresh behavioral coverage
under the requested model policy. Keep model and effort explicit
and within the calling agent's capability/effort ceiling. Record exact commands
and outputs in the receipt; do not reuse the historical Grok result as fresh proof.
If a runtime cannot select the required GPT-6.1 Sol high model, report that limitation
instead of substituting a different model. The current Cursor model list lacks it.

One existing batching check was separately reviewed and corrected: the PR prompt
still asks for four checks, but only its three semantic checks belong in the model
batch. Issue-number detection must use an exact pattern in code. The original
rubric contradicted the skill's existing deterministic-only boundary; its original
results and the reviewed correction remain in the receipt.

The publisher-input baseline used an explicitly staged `compatibility.json` and
could not establish the bridge's scalar/null, instruction fallback, or 26-level
Score contract. The corrected reference documents those pinned publisher details
and the project's explicit 255-level client resource bound. The incomplete first
pilot, whose named file was missing, remains recorded separately.

One routing check was separately reviewed as inconsistent: the pilot skill requires
exact CLI syntax and delegates that syntax to `jev`, while the original rubric
forbade every `jev` load. Its three original failures remain preserved. The replacement
requires a successful pilot load before any secondary `jev` consultation, retaining
the separate pilot-positive and fit-negative checks. No prompt or case count changed.


The historical 30-case held-out run and original 85/88 checks remain unchanged in
the receipt. Its urgent CSV grader was changed after observing results; that case
is now retired from heldout and retained as `routing-urgent-csv-pilot-before-cli`
with the corrected pilot-before-CLI contract. Current heldout runs use 29 cases
frozen in `heldout/admission-manifest.json`. The post-hoc 88/88 regrade is not blind
held-out evidence. None of these older routing cases measures blind Clef behavior;
a future Clef held-out batch requires independent authorship before evaluation.
