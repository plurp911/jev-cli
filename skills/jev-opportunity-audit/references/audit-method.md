# How to search a repository, surface by surface

The method behind the nine surfaces in `SKILL.md`: what to look for, what the tell is,
and what to read once you have a hit.

Two rules before any of it. **Read the architecture first** — README, entry points,
dependency manifest, directory names, the shape of the request path — because a
grep-first audit produces findings about files nobody calls. And **searching is for
locating, not for concluding**: a hit is a place to read, never a finding on its own.

---

## 1. Existing model calls whose output is bounded

Usually the highest-value surface. Start from the dependency manifest — an SDK in
`requirements.txt`, `package.json`, `go.mod`, `Cargo.toml` tells you a model is in here
somewhere — then find the call sites.

Search for the client, then for the prompt text: `messages=`, `completion`, `system=`,
`response_format`, `json_schema`, `tool_choice`, `temperature`, `max_tokens`, and the
string fragments prompts are made of — "respond with", "reply with only", "return JSON",
"one of the following", "answer yes or no", "rate from 1".

**The tell is at the call site, not in the prompt.** Read what the caller does with the
response. If it parses one field and drops the rest, or matches the first word, or
compares against a fixed set of strings, the code is paying for prose to move one token
of information. Signals that make it stronger:

- A parse-failure branch, a fallback value, or a counter for unparsable responses. Each
  one is a cost of not having a typed answer.
- A comment complaining the model is chatty, adds reasoning, or ignores the format.
- A retry, a batch size, or a serialised loop that exists because of a rate limit.
- A named bill, a spend comment, or a downgrade that was tried and reverted.

**Prompts are often not in the code.** A repository whose prompts live in `prompts/*.txt`,
in YAML, in a database, or in a chain or graph definition, loaded by one generic
`run_prompt()`, produces almost no hits on the patterns above. If the manifest names a
model SDK and your search finds one call site, go looking for where the text is kept.

**The near-miss:** a call whose output is *parsed* as structure but whose content is free
text a person reads. A draft, a summary, an explanation. Not a finding.

## 2. Heuristics standing in for a fuzzy concept

Look for a module-level constant that is a list of words or phrases, a dictionary of
weights, a scoring function that sums them and compares to a magic number, and a long
chain of regexes with semantic names.

Search: `KEYWORDS`, `PHRASES`, `PATTERNS`, `_TERMS`, `SPAM_`, `URGENT`, `BLOCK`, `re.compile`
near words like `urgent`, `angry`, `risk`, `intent`, `toxic`, `relevant`, and threshold
constants named `*_THRESHOLD`, `*_CUTOFF`, `MIN_SCORE`.

**The evidence is usually the maintainer's own comment.** A `TODO` about people rewording
things, a note that the list is wrong more often than they would like, a line saying "do
not add more phrases to fix one case". Quote it. It is a request for semantic
understanding, written out longhand, and it is the strongest evidence an audit can carry
because it is the team's own words.

**The near-miss:** a keyword list matching *identifiers* rather than prose — error codes,
SKUs, HTTP methods, file extensions, status enums. Exact matching over a closed
vocabulary is correct and must stay.

## 3. Enum columns and status fields

Read `schema.sql`, the migrations directory, or the ORM model definitions. A column named
`status`, `category`, `queue`, `priority`, `reason`, `risk_level`, `sentiment` or `tag`,
constrained to a small set, is a bounded decision that physically exists in the database.
Then find what writes it: a form a human fills in, a rule, a trigger, or a model call you
have already found. A column populated by hand at volume, or by a rule someone keeps
amending, is a strong lead.

**The near-miss:** a status column driven by the system's own lifecycle — `pending`,
`processing`, `failed`, `archived`. Those are state transitions the code makes, not
judgments about content.

## 4. Routing

Search for dispatch: a dict of handlers, a registry, a `match`/`switch` on a string, a
`route_to`, `select_model`, `choose_agent`, `pick_tool`, `assign_queue`, `for_team`.

Then ask what the branch is keyed on. **Keyed on a field the system already knows** —
plan tier, region, account type, a status enum, an HTTP path — is a lookup table, and a
long ugly one is still a lookup table. **Keyed on what a human wrote** is a judgment.

The false positives on this surface are the most common in any audit, and they all look
identical to real routing: feature flags and entitlement checks (every arm traces to a
contract or a regulation), locale and i18n dispatch, content-type and MIME dispatch,
HTTP verb or path dispatch, and log-level routing. All keyed on something the system
already knows. All must stay exact.

## 5. Retrieval

Search for the vector store or search client, then read forward from the query. You are
looking for: a candidate limit much larger than the final limit, a rerank step, a
relevance filter, a model call between retrieval and use, a truncation to `top_k` in
arbitrary order, and a citation or grounding check.

The shape worth reporting: *N candidates in, a model call, M survivors, and the code
keeps only ids*. Note whether the current step returns a set or a ranking — a set that
then gets truncated arbitrarily is discarding the ordering the system needs.

## 6. Agent harnesses

Read the loop. You are looking for judgments that happen *per turn*:

- The whole tool list shipped every turn when a handful could be selected first.
- A small yes/no call inside the loop — "should a human take over", "is this done", "is
  this relevant", "did that tool call do what was asked". Check how its answer is
  parsed: `.startswith("yes")` with an else-branch that treats anything unexpected as one
  of the two answers is a gate that fails in a fixed direction, and that belongs in the
  Risk line.
- Context assembly that decides what to keep.

The orchestration itself — multi-step tool use, planning, writing the response — stays
with the frontier model. Do not propose moving the loop.

## 7. Human review queues, and CI

Read `.github/workflows/`, the hooks, the lint configuration, the scripts directory.
Look for checks that approximate a semantic policy with a pattern: a PR-title regex, a
description length check, a banned-words list, a "does the changelog mention this" grep.
Also look for what a human does every time that nobody automated — a review checklist, a
triage rota, a risk label applied by hand.

The same shape exists in the product, not only in CI: a moderation queue, a flagging UI,
a "needs review" state, an inbox someone works through in order. Find what decides
whether something lands in the queue and how it is ordered once it is there. Both are
bounded judgments, and the queue is also where the labels are, because the humans
working it have been producing them all along.

A model in CI is a real design decision with its own failure modes; note that the check
must fail in a direction the team can live with when the service is unavailable.

## 8. Data pipelines

Batch jobs, cron entries, queue consumers, ETL steps, migration scripts. Look for a stage
that classifies, triages, prioritises what a human reviews, or filters a large set on
something fuzzy. Volume is usually easy to evidence here — a schedule, a batch size, a row
count, a table name.

## 9. Labelled data that already exists

Not an opportunity, but look for it on purpose rather than hoping, because it is what
separates `PILOT` from `INVESTIGATE`. An `evals/`, `golden/`, `fixtures/` or `benchmarks/`
directory; a file with `labelled`, `annotated` or `ground_truth` in its name; an audit or
decision log; a corrections table or an override column; a queue where a human overrules
the system and the overrule is stored.

Say per finding whether such a set exists for *that* decision. A labelled set for the
router does not validate the urgency scorer.

---

## Deciding, once you have a hit

The five-item bar and the two gates behind it live in `SKILL.md`; they are not repeated
here. Run every hit through them before it becomes a finding. The one that catches the
most hits is bar 2 — **name the caller** — because a search finds vendored code,
generated clients, test doubles, fixtures, dead code and permanently flagged-off paths
just as readily as it finds the live path, and none of those is a finding.

Two negatives worth carrying in your head while you read, because they look like
opportunities right up to the moment you check:

- **A model the team already runs.** A fine-tuned small classifier, or embeddings plus a
  linear model, that works. Frequently the right answer to "could Jev replace this
  classifier" — but check the three things that decide it rather than assuming them:
  whether it was measured on their data, whether it is billed per call (a hosted model
  is), and whether it runs inside their environment. The repository usually shows all
  three.
- **Labels that come from the incumbent.** The output of the system being replaced is the
  most available validation data in any repository and the most misleading: it measures
  agreement with that system, including everywhere it is wrong, and the cases where it is
  wrong are usually the ones that motivated the change. It is a cheap first signal, not
  ground truth, and a finding resting on it is `INVESTIGATE`, not `PILOT`, until a
  human-judged held-out set exists.

The `is-jev-useful-here` skill, if it is installed alongside this one, carries the long
form of the fit reasoning in its own reference file. This audit does not depend on it.

## Identifiers that mean nothing

Names lie in both directions, and a keyword-matching audit fails on exactly these.
`classify`, `score`, `decide`, `route`, `filter`, `threshold`, `rank`, `match`, `judge`
appear constantly in numeric and control-flow code: a luminance score, a retry decision, a
routing table, a filter over a slice, a numeric threshold. Read the function before
believing the name.

The reverse is just as common. The strongest opportunity in a repository is often called
`_tag`, `_bucket`, `handle`, `process`, `check`, or nothing suggestive at all.

---

## Evidence that makes a finding rankable

Collect these as you go. They are in the code far more often than an audit bothers to
look.

| Looking for | Found in |
| --- | --- |
| Volume | metrics counters, a docstring, a cron schedule, a queue name, a batch size, a row count |
| Cost | a comment naming a bill, a model choice, a `max_tokens`, a downgrade that was reverted |
| Latency pressure | a timeout, a latency histogram, "on the request path", a cache added to avoid a call |
| Rate-limit pain | a serialised loop, a semaphore, a backoff, a comment about a TPM ceiling |
| Existing labels | a fixtures or evaluation directory, a `labelled`/`golden` file, an audit log, a human-corrections table |
| Privacy gates | a region check, a data-processing flag, a consent field, a redaction helper |
| Blast radius | how many call sites a function has, whether its return type is public |

---

## What never goes in an audit

- A finding about a file you did not read.
- A cost, latency or accuracy figure the repository does not evidence.
- A published TypeSafe benchmark presented as a prediction about this repository.
- A new product capability dressed as a finding.
- A Choice's options, a Score's levels, criteria wording, a request document, or a `jev`
  command line. Name the decision and the primitive; the syntax belongs to the `jev`
  skill and the question design to the official `typesafe-ai` skill.
- A threshold presented as validated.
- A finding requiring a modality the selected provider cannot accept. Cloudflare/Ollama
  Clef support bounded image judgments; the project Hugging Face bridge also accepts
  prepared frames. TypeSafe and llama.cpp Clef remain text-only here; audio is unsupported.
  OCR/transcription is a separate option when extracted text is needed. Check the
  [provider reference](https://github.com/plurp911/jev-cli/blob/main/skills/jev/references/providers.md),
  and do not discover or upload media while auditing. Numeric fields in structured state
  are ordinary input; arithmetic over them is excluded by bar 4, not modality.
- A proposal to bound the core output of a product whose job *is* generation.
- A finding at a place the repository already calls TypeSafe or `jev`. Say it is already
  done.
- Any edit to the repository.
