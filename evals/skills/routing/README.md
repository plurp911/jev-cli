# Cross-skill routing

A skill that fires correctly on its own can still be wrong once its neighbours exist.
The five Jev skills are full of adjacent intents:

| A user asks | The skill that should win |
| --- | --- |
| "How do I run a Choice?" | `jev` (operating the CLI) |
| "Would Jev help this workflow?" | `is-jev-useful-here` |
| "Find Jev opportunities in this repo" | `jev-opportunity-audit` |
| "Look at my last month of Claude work and find Jev opportunities" | `jev-workflow-retro` |
| "Test whether replacing this classifier with Jev actually works" | `jev-pilot` |

Every one of those sentences mentions Jev. Descriptions tested one at a time will all
look fine and still collide, because the description that wins is decided against the
others in the list, not against nothing.

**A near-miss that was tried and withdrawn.** `near-miss-what-is-a-noul` briefly asserted
that the `jev` skill must not fire on "what does Noul mean, and why no confidence field?".
It fired, and that is correct: the `jev` skill owns the fact that a Noul carries no
`confidence`, because it decides whether a `--require` path is evaluable. The assertion
was removed, not tuned for. Adding a negative is a claim about where a question belongs,
and it can be wrong.

## The procedure

1. **Only after the behaviour is right.** Routing is a property of the descriptions, and
   optimising a description for a skill that answers badly optimises the wrong thing.
2. **Every new skill adds a case here**, and the case asserts two things: the intended
   skill fired, and each *adjacent* skill did not.
3. **Run the whole suite**, not the new case. A description change that fixes the new
   case by stealing traffic from an existing one is a regression, and only the full run
   shows it.
4. **Change one description at a time.** Two edits and a moved number tell you nothing
   about which edit moved it.

## A routing case

One case per contested phrasing, with one positive grader and one negative grader per
adjacent skill. `TEMPLATE.md` in this directory is the shape to copy; it is not itself a
case, because the harness discovers cases by `prompt.md` and `case.yaml`.

Write the prompt the way the collision actually arrives — a real sentence with a real
repository, a real file, a real deadline. The near misses have to be genuinely close.
"Write me a fibonacci function" proves nothing about a description; "would it be worth
pointing Jev at this triage step?" does.

## The cases here now

Generated from actual graders with `python3 scripts/skill-eval-codex.py --routing-table`.
`test-skill-eval-codex.py` checks that this table matches the grading contract.

| Case | Intended winner | Asserted not to fire | Ordered consultation |
| --- | --- | --- | --- |
| `after-the-audit-prove-one` | `jev-pilot` | `is-jev-useful-here`, `jev-opportunity-audit` | none |
| `an-export-is-still-history` | `jev-workflow-retro` | `jev-opportunity-audit`, `jev-pilot` | none |
| `calibrate-a-threshold-is-the-cli` | `jev` | `is-jev-useful-here`, `jev-pilot` | none |
| `clef-image-command` | `jev` | `is-jev-useful-here`, `jev-opportunity-audit`, `jev-pilot`, `jev-workflow-retro` | none |
| `clef-video-command` | `jev` | `is-jev-useful-here`, `jev-opportunity-audit`, `jev-pilot`, `jev-workflow-retro` | none |
| `does-it-hold-up-is-a-pilot` | `jev-pilot` | `is-jev-useful-here` | `jev-pilot` → optional `jev` |
| `find-them-before-testing-any` | `jev-opportunity-audit` | `is-jev-useful-here`, `jev-pilot`, `jev-workflow-retro` | none |
| `from-my-history-now-prove-it` | `jev-pilot` | `is-jev-useful-here`, `jev-workflow-retro` | none |
| `generation-is-nobodys` | none | `is-jev-useful-here`, `jev`, `jev-opportunity-audit`, `jev-pilot`, `jev-workflow-retro` | none |
| `history-but-not-a-jev-question` | none | `is-jev-useful-here`, `jev`, `jev-opportunity-audit`, `jev-pilot`, `jev-workflow-retro` | none |
| `my-history-not-my-repo` | `jev-workflow-retro` | `is-jev-useful-here`, `jev`, `jev-opportunity-audit` | none |
| `my-repo-not-my-history` | `jev-opportunity-audit` | `is-jev-useful-here`, `jev-workflow-retro` | none |
| `one-step-named-not-the-repo` | `is-jev-useful-here` | `jev`, `jev-opportunity-audit`, `jev-pilot` | none |
| `point-jev-at-the-whole-app` | `jev-opportunity-audit` | `is-jev-useful-here`, `jev` | none |
| `pricing-is-nobodys` | none | `is-jev-useful-here`, `jev`, `jev-opportunity-audit`, `jev-pilot`, `jev-workflow-retro` | none |
| `prove-it-not-weigh-it` | `jev-pilot` | `is-jev-useful-here`, `jev-opportunity-audit` | none |
| `score-over-this-json-is-the-cli` | `jev` | `is-jev-useful-here`, `jev-pilot`, `jev-workflow-retro` | none |
| `show-me-the-command-for-this-judgment` | `jev` | `jev-opportunity-audit`, `is-jev-useful-here` | none |
| `urgent-csv-pilot-before-cli` | `jev-pilot` | `is-jev-useful-here` | `jev-pilot` → optional `jev` |
| `weigh-it-not-prove-it` | `is-jev-useful-here` | `jev-pilot`, `jev-workflow-retro` | none |
| `worth-pointing-jev-at-this-step` | `is-jev-useful-here` | `jev`, `jev-opportunity-audit` | none |

Every one of these names Jev, or describes work about it. **Four axes separate them**,
and each pair in the table above turns on exactly one.

**Code, or behaviour?** `my-repo-not-my-history` points at a checkout; `my-history-not-my-repo`
says "I don't mean the code, I mean what I've been doing". An export
(`an-export-is-still-history`) is still behaviour, not a repository.

**Weighing it, or proving it?** `weigh-it-not-prove-it` asks whether Jev belongs there at
all ("or is that overkill"). `prove-it-not-weigh-it` opens by closing that question
("I don't need convincing") and asks for evidence against the incumbent.

**A command, or a study?** `calibrate-a-threshold-is-the-cli` already has the request file
and the labels and wants the invocation. `does-it-hold-up-is-a-pilot` wants to know
whether to believe the result, "and be honest if the answer is no".

**Is it a Jev question at all?** `history-but-not-a-jev-question` reads a transcript store
for a completely different purpose, and `pricing-is-nobodys` is a documentation lookup.
Both must reach none of the five. These are the cases a description tuned on the positives
loses first.

Two axes separated the original four.

**Has the user already decided?** `worth-pointing-jev-at-this-step` is weighing a
heuristic against a model and says so ("or is that overkill"). `show-me-the-command-for-this-judgment`
opens by ruling that out of scope ("I've already decided") and asks for a command.

**One named step, or the whole tree?** `one-step-named-not-the-repo` gives a file and a
function and then closes the door explicitly ("I'm not asking about the rest of the app").
`point-jev-at-the-whole-app` says the opposite in the user's own words ("I honestly don't
know where"). That second sentence is the entire signal, and it is the one a description
tuned on the first pair will lose.

They are a set, not independent cases. A description edit that wins one by claiming
another's traffic is a regression, and only the full run shows it.
