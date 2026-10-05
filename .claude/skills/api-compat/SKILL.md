---
name: api-compat
description: >
  Check what jev sends to and expects from its selected provider against that
  provider's authoritative contract, and keep compatibility fixtures honest. Use before
  changing any request shape, response decoding, question type, model identifier, error
  mapping, or retry behaviour — and whenever you are about to state how the Jev API
  or a Clef provider behaves.
allowed-tools: Bash Read Edit Grep Glob WebFetch
---

# API compatibility

**Use the selected provider's primary sources. Your recollection is not authority.**

This repository is a community project. It has no special knowledge of the API, and a
confidently wrong wire format is worse than no implementation at all. Every claim about
API behaviour must be traceable to an official source read in the current session.

## TypeSafe authority order

1. <https://docs.typesafe.ai> — start from <https://docs.typesafe.ai/llms.txt>.
   Mintlify serves Markdown by appending `.md` to a page path, e.g.
   <https://docs.typesafe.ai/api.md>. Resolve relative links against
   `https://docs.typesafe.ai`.
2. The official agent skill, vendored at `.claude/skills/typesafe-ai/SKILL.md`. Invoke
   it with `/typesafe-ai` for design guidance.
3. The official TypeSafe SDK repositories and their typed definitions.

**Never** authoritative: this repository's comments, another community project, an
"awesome-" list, a blog post, or a conference talk. Those may be read for inspiration;
they may not be cited and their code may not be copied.

If the docs are unreachable, **say so and stop**. Do not fill the gap from memory.

## Additional provider authority

Select authority from the explicit provider, as required by `AGENTS.md` §6 and
ADR-0015. Shared primitive names do not establish identical routes, media formats,
authentication, limits, or response envelopes.

| Provider or surface | Authority to read in this session |
| --- | --- |
| Cloudflare Workers AI | [Clef schema](https://developers.cloudflare.com/workers-ai/models/clef/) or [Clef Flash schema](https://developers.cloudflare.com/workers-ai/models/clef-flash/) for the selected model |
| Ollama | [System One endpoint](https://docs.ollama.com/api/systemone) |
| llama.cpp | [Official server contract](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md) for the selected runtime revision |
| Explicit local Python bridge HTTP contract | Project-owned [docs/clef.md](../../../docs/clef.md), approved in ADR-0015 |
| Python bridge model loader and media processing | [Publisher implementation](https://huggingface.co/Cloudflare/clef/blob/main/joint_schema_model.py), selected model revision, and the selected processor's official source |

The project-owned bridge HTTP contract is authoritative only for that bridge. It
does not establish upstream provider behavior. Read the selected official source
before changing any adapter; if it is unavailable, stop the adapter change and
report the missing evidence. Hosted and local capabilities must be checked
independently. Research notes and captured live reports help locate evidence but
do not replace a current official contract.

## Procedure

### 1. Read the relevant official page, now

Do not rely on what this file or the codebase says. Fetch the selected provider's
source above in this session. For TypeSafe, use this page map:

| Changing | Read |
| --- | --- |
| Request or response shape, auth, errors | `/api.md` |
| Which primitive to use | `/primitives.md`, then `/primitives/choice.md`, `/score.md`, or `/noul.md` |
| What to send as state | `/concepts/state.md` |
| Confidence or probability handling | `/confidence.md` |
| Model identifiers | `/models.md` |
| Multiple questions in one request | `/cookbooks/parallel_questions.md` |
| Retry and backoff | `/api.md`, plus the SDK retry pages |

### 2. Record what you found

For every fact the change depends on, write down the URL and the exact wording. These go
in the pull request. A fact without a citation does not go into the code.

### 3. Compare against the codebase

```sh
rg -n 'systemone|jev-latest|api\.typesafe\.ai' crates/
rg -n 'DEFAULT_MODEL|DEFAULT_BASE_URL|SYSTEM_ONE_PATH' crates/
rg -n 'Provider|Protocol|cloudflare|ollama|llamacpp|huggingface|clef' crates/ scripts/clef-server.py
```

Check each against the page you just read:

- **Endpoint and method.**
- **Auth header** name and scheme.
- **Required request fields** and their types.
- **Per-primitive question fields**, including which are required and which optional.
- **Answer shapes** for each primitive, and the exact field names.
- **Usage fields.**
- **Error status codes** and the error body shape.
- **Documented limits** — question count, state size, rate limits.

Anything present in the code but absent from the docs is a fabrication until proven
otherwise. Anything in the docs but absent from the code is either a gap or a
deliberate omission, and deliberate omissions get a comment saying so.

### 4. Semantics, not just shape

Wire compatibility is the easy half. These are the errors that produce plausible,
wrong output:

- **A `Noul` is a probability of yes, not a confidence and not an intensity.** A value
  near 0.5 means "roughly as likely as not", not "moderately".
- **`Choice` and `Score` confidence describes distribution concentration**, not whether
  the workflow is correct and not permission to act.
- **Probability spread across several acceptable options is not a failure.** Low
  confidence on a harmless preference may be fine.
- **A `Score` is a probability-weighted position on ordered levels**, not a free
  numeric rating.
- **Typed output guarantees the interface, not the truth.** Never describe a System One
  answer as a fact.
- **Thresholds in cookbooks are examples**, not universal constants. Never hardcode one
  as if it were a documented rule.

If the change touches question design or confidence handling, invoke `/typesafe-ai`.

### 5. Fixtures

Compatibility fixtures represent the selected authoritative contract, replayed
through `MockTransport` so decoding is tested without a network. Distinguish a
captured response from a synthetic schema example in its provenance; neither
establishes another provider's behavior.

When adding one:

- **Redact every credential.** Use an obviously fake key such as
  `sk-fixture-not-a-real-key`.
- **Redact any real user content.** Fixtures are public forever.
- Record the source URL or the date and circumstances of capture in a sibling comment
  or README.
- Add the malformed variants too: truncated JSON, wrong types, an out-of-range
  probability, an unknown primitive type, an empty answers map, a gigantic body. The
  happy path is the least interesting case.

Test with the mock, never a live call:

```rust
let transport = MockTransport::new().with_response(200, include_bytes!("fixtures/choice.json").to_vec());
```

### 6. Compatibility rules

- **Unknown fields in a response are ignored, not fatal.** The API will add fields.
- **A missing required field is an error**, reported as
  `ClientError::MalformedResponse` with a reason that does not embed the whole body.
- **An out-of-range value is rejected at the boundary.** `Probability` already does
  this; follow the pattern.
- **Never send a field the docs do not define.**
- **Map errors by the selected provider's documented status code**, not by guessing
  at message text. TypeSafe's documented status mapping does not establish another
  provider's error contract.

## Report

State, in the pull request:

1. Which official pages you read, with URLs.
2. Exactly what each one established.
3. What changed in the code as a result.
4. Anything the docs did not answer — and that you therefore did not implement.

Point 4 matters most. A gap stated honestly is fine; a gap filled from memory is how a
community CLI becomes untrustworthy.
