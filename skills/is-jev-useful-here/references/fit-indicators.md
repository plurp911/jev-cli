# Fit indicators, in detail

The catalogue behind the short lists in `SKILL.md`. Read it when a case is not obvious
from those, or when you need to explain *why* a signal points the way it does.

This file is about **fit only**. What the primitives mean, how to write criteria, how
confidence behaves, how questions in one request relate to each other — all of that
belongs to TypeSafe's official `typesafe-ai` skill and their documentation, and is not
restated here. Operating the CLI belongs to the `jev` skill.

---

## Positive indicators

### Bounded semantic output

The answer is one of a known set, a yes/no, or a position on a described scale, and
deciding it needs understanding of natural language. This is the base case; everything
below is a multiplier on it.

### A generative call whose output is consumed as a label

The highest-value thing to look for. A prompt ending "respond with only JSON containing
`relevant: true|false`", or "reply with one of: billing, shipping, other", or "rate 1-5",
is a bounded decision paying generative prices. The tell is downstream: the code parses
one field out and discards the rest.

The near-miss to watch for: a call whose output is *parsed* as structure but whose content
is free text a person reads — an explanation, a draft, a summary — is not this.

### Repeated, independent judgments

Many records, many requests, many agent turns, each decided on its own. Independence
matters as much as volume — but be precise about what dependence means. A sequence of
bounded judgments that *code* orchestrates, where the first answer tells the program
what to fetch or which options to offer next, is a normal and supported design. What
does not work is asking one judgment to carry the chain: to reach its answer by working
through intermediate conclusions it has to produce itself. Count round trips for cost
and latency, not as evidence of a bad fit.

### One state, several questions

Several independent things need deciding about the same document, ticket, diff or event.
Strong, because the questions share one reading of the state — and worth naming in the
assessment, because the naive implementation of this case is one call per question.

### Classification, routing, filtering, relevance, prioritisation

The canonical family. Ticket triage, model or agent or tool selection, candidate
filtering, reranking retrieved passages, deciding which of N things a person should look
at first, deciding whether a piece of context is worth including.

### Semantic gates and bounded verification

"Does this output contradict the source?", "does this changelog entry describe the
change?", "does this citation support the claim?", "does this tool call match what was
asked?". Checking one bounded property of something another system produced — checking,
not repairing.

**Caveat, and it is not small:** if the thing being checked is written by someone who
benefits from passing the check, the judgment is adversarial, and a judgment that can be
edited against is not a control on its own. See the negative indicator below.

### Latency-sensitive judgments

A decision on a request path, in a UI, or inside an agent loop, where a generative model
is currently the thing being waited on.

### High volume

Enough calls that unit cost is a line item rather than a rounding error. Volume alone is
not a reason — a high volume of judgments that code could make is a reason to write the
code — but it converts a marginal fit into a worthwhile one.

### Uncertainty the workflow can use

There is somewhere for "not sure" to go: a human queue, a fallback to a more expensive
model, a second pass, a wider net. A calibrated probability is worth far more to a
workflow with an escalation path than to one that must answer every case.

---

## Negative indicators

### Open-ended generation

Writing, summarising, rewriting, translating, explaining, drafting, or producing code.
Jev returns typed answers, not text. Not a weak fit — not possible.

### Multi-step reasoning

A single judgment that can only be reached by working through intermediate conclusions,
exploring, or backtracking. Several round trips that *code* drives — using each answer to
fetch evidence, build new state, or decide the next options — are a supported pattern, not
a warning sign; the official documentation covers when a second request is warranted. The
negative here is the judgment that has to reason, not the workflow that has several steps.
Many round trips do cost latency and tokens, which is a sizing question, not a fit one.

### Deterministic computation

Arithmetic, totals, percentages, counting, sorting, deduplication by key. A model can be
wrong about 2+2.

### Dates, intervals and counting

"Is this more than 30 days old", "which of these is most recent", "how many happened this
week". These feel semantic and are not: parse and compare in code. The semantic part, if
there is one, is usually *which* of several dates on the document is the one that matters
— and that is a selection over candidates the code found.

### Exact lexical matching, and standard parsing

Finding a literal string, an identifier, a SKU, an error code, a path — that is what
search tools are for. Reading JSON, XML, CSV, a log format, an AST, a specified file
format — that is what parsers are for. A model that usually parses correctly is a worse
parser.

### Tiny one-off workloads

Forty rows, once. Usually WEAK rather than NO: it would work, and the integration, the
credential and the validation cost more than doing it by hand. That changes the moment it
becomes a recurring job.

### The model as a security boundary

Authorisation, access control, spend approval, deletion, anything irreversible. A
probability is evidence; the gate has to be deterministic — a limit, an allowlist, a
signature, a person. Jev can narrow what reaches the gate.

Then ask the follow-up question, because it decides the verdict: once the deterministic
gate is in place, **is there a real judgment left?** If yes, CONDITIONAL on that gate. If
the gate is doing all the work, NO. Where it applies, the safe asymmetry is to automate
the permissive direction on the clean path and escalate everything else, rather than
automating both directions off one threshold.

### Adversarial, attacker-controlled state

Spam, abuse, moderation, fraud, jailbreak screening, checking an untrusted model's output.
The person writing the input wants a particular answer and can keep rewriting until they
get it. This does not make it a non-fit — these are real and common uses — but it changes
what the assessment has to say: the model is never the gate on its own, even when the
action is reversible. A person or a deterministic rule takes the action, and the model
orders, narrows or pre-screens what reaches them; the consequences of a successful
evasion stay bounded by limits the input cannot influence; and the error rate will not
stay where it was measured. Say that the change needs threat modelling by
whoever owns that in the user's project; if their environment has a security-review skill
or process, that is where it belongs.

### Consequential judgments about people

Hiring, credit, housing, insurance, benefits, immigration, discipline, medical triage,
anything with a legal or life consequence for an individual. These score well on every
positive indicator above — bounded, repeated, high volume, escalatable — which is exactly
why they need saying out loud. The fit question is not the question. Regulation,
auditability, contestability, bias testing and disclosure decide whether this is
permissible at all, and they are not an engineer's call to make in an assessment. Name
the kinds of obligation, name who has to rule, and stop — do not cite particular statutes
or jurisdictions from memory, for the same reason you do not quote an API fact from
memory. The configuration that draws the most scrutiny is the one where the model's
output removes a person from consideration without anyone seeing the case; a design that
ranks a queue a human still works through is a materially different proposition.

### Required context exceeds practical state

Answering honestly needs the whole repository, the entire conversation history, a large
corpus, or facts not available at decision time. Sometimes fixable by retrieving the
relevant slice first, which makes it CONDITIONAL with retrieval as the condition; sometimes
not.

### The path cannot tolerate a third-party dependency

An offline-capable path, a hot path with a hard latency budget, or a step that must keep
working when an external service is slow or unavailable. Adding a network call to it is a
legitimate NO, and the question "what does this do on a timeout, and can it?" belongs in
any assessment of a request-path judgment.

### The decision must stay reproducible

A judgment that has to be re-derivable years later — a regulated decision, a compliance
record, an audit trail. Model versions move. This is not automatically a NO, but it is a
constraint the assessment has to raise rather than discover later.

### Something the team already runs is the right answer

A fine-tuned small classifier, an embedding model plus a linear model, or an existing
rules engine that works. "Could Jev replace this classifier" is very often answered with
"keep the classifier". Check, rather than assume, the three things that usually decide
it: whether it was actually measured on their data, what it costs per call (a hosted
model is billed like any other), and whether it runs inside their environment. Where
those hold, they are hard to beat; where they do not, the incumbent is a weaker
alternative than it looks.

### The data cannot leave

A data agreement that forbids a new sub-processor, a jurisdiction that forbids the
transfer, a category of data the organisation does not send out. Assess the restriction
that is in force: that is NO, not a condition. Someone being in a position to seek a
change later does not make it conditional — say the verdict is NO today and worth
revisiting if the restriction actually changes.

### Modality mismatch

Images, audio, video, scanned documents. A scanned invoice is an OCR problem before it is
anything else, and an assessment that skips that step promises something that cannot be
delivered.

Numbers are **not** on this list. Structured state carrying numeric fields is ordinary
input, and a judgment can reason about what an amount or a count means. What is excluded
is arithmetic over them, and that is the deterministic-computation rule above, not a
question of modality.

### Nothing is actually wrong

The current approach works, nobody is complaining, and the interest is curiosity. A fine
reason to prototype and a poor reason to migrate. WEAK.

---

## Conditional patterns

These come up repeatedly and are neither yes nor no. Naming the condition is the value of
the assessment. Apply the test in `SKILL.md`: if satisfying the condition leaves no
judgment worth making, it is a NO.

| Pattern | The condition |
| --- | --- |
| One fused question doing five jobs | Decompose it into independent questions over the same state |
| "Extract a value from this document" | Find candidates in code and let Jev **select** among them; over-generate, because a value that was not offered cannot be chosen |
| Ranking a large set | Judge each item independently and sort in code — check what the loss of cross-item comparison does on your data |
| A decision with money or access attached | A deterministic gate behind it, and escalation rather than auto-denial |
| No labelled data anywhere | Find the labels that already exist, and see below |
| Sensitive data | An explicit decision that this content may be sent, and a state field carrying the minimum |
| Enormous context | Retrieve or slice first |
| Scanned or image input | OCR first; that is a separate decision with its own cost |

---

## The labels-from-the-incumbent trap

The most available source of labels and the most misleading one. Calibrating against the
output of the system being replaced measures agreement with that system, including
everywhere it is wrong — and the cases where the incumbent is wrong are usually the cases
that motivated the change. It is a legitimate, cheap first signal. A held-out set judged
by a person is what turns it into ground truth. Say this whenever the recommended labels
come from the incumbent.

---

## What never goes in an assessment

- A cost, latency, throughput or accuracy figure for the user's workflow.
- A published TypeSafe benchmark presented as a prediction about the user's case, or any
  such figure quoted from memory rather than from the page.
- A probability threshold presented as validated.
- The options of a Choice, the wording of criteria, the levels of a Score, a request
  document, a pipeline, or an implementation. That is the next request.
- A `jev` command line. Naming the command is enough; the flags, the file format and the
  output contract belong to the `jev` skill, and an invocation invented from memory is one
  the user will paste and watch fail.
- A claim about pricing, quotas, rate limits, model identifiers or wire format. Those
  belong to the official documentation; if one matters, point at
  <https://docs.typesafe.ai/llms.txt> rather than constructing a page path from memory.
