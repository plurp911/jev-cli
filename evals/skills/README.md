# Skill evaluation suite

Evidence that the Agent Skills in [`skills/`](../../skills) do something, and that the
right one does it. The methodology, and the policy that makes this mandatory, are in
[`docs/development/skill-authoring.md`](../../docs/development/skill-authoring.md).

Run it with [`scripts/skill-eval.sh`](../../scripts/skill-eval.sh). **It costs money**:
every run and every LLM grader is a real model call on your own credential.

```sh
scripts/skill-eval.sh --list            # what would run
scripts/skill-eval.sh --iterate         # cheap and fast: the red-green loop
scripts/skill-eval.sh --tag trigger     # one group
scripts/skill-eval.sh --case 'jev-*'    # one case, or a glob
scripts/skill-eval.sh                   # the training suite, on the model results are quoted from
scripts/skill-eval.sh --heldout         # the held-out routing set, alone -- see heldout/README.md
```

"Everything" excludes `heldout/`: those cases are staged only by `--heldout`, which is
refused under `--iterate`, so that they are never looked at while descriptions are tuned.

Only the **last** `--case` glob applies; repeating the flag keeps one of them, silently.

`--iterate` is a signal, not a result: one run, no baseline arm, same model. It answers
"did my edit change anything" for about a sixth of the cost. It does not
answer "is this good", and a number from it never goes in a pull request — see
[`docs/development/skill-authoring.md`](../../docs/development/skill-authoring.md) §6 for
why it stays on the measured model rather than a cheaper one.

## Why the harness runs everything twice

`claude plugin eval` runs each case in two arms: once with the skills loaded, once with
no plugin at all. The second arm is the **RED baseline** — what the agent does when the
skill does not exist — measured rather than assumed. A case whose delta is zero is a
case the skill did not change, which means either the skill is not pulling its weight or
the case is not testing anything. Both are findings.

## Layout

```
evals/skills/
  <skill-name>/
    triggers/<case>/     does the right skill fire, and stay out of the way otherwise
    behaviour/<case>/    given that it fired, is the answer right
      case.yaml          only where the case has to stage a fixture
      scaffold.sh        copies that fixture into the agent's working directory
    fixtures/<repo>/     input a case points at, where a prompt alone is not enough
  routing/<case>/        which skill wins when several could plausibly fire
```

## Fixtures

All fixture repositories, transcripts, and customer records in this suite are
synthetic. Claims inside a staged fixture that a file is a raw customer export are
part of its privacy test scenario, not a description of real customer data.

A skill that audits a repository cannot be tested with a sentence, so
`jev-opportunity-audit/fixtures/` holds two small applications written for the purpose:
`pulse`, a support product carrying four real opportunities and five tempting
non-opportunities, and `pixelsort`, numeric image code whose identifiers are full of
`classify`, `score`, `decide` and `route` and which contains no judgment at all. They are
**test data, not this project's code**: nothing imports them, nothing builds them, and
neither one mentions Jev. `pixelsort` is the more important of the two, because the
correct audit of it is an empty one and that is the result a skill is most tempted to
avoid.

`jev-workflow-retro/fixtures/` holds the same idea for transcripts: two synthetic
transcript stores — `home-rich`, sixteen session files across Claude Code, Codex and
Gemini CLI, of which fifteen parse (one Codex rollout is deliberately too old to), carrying
repeated relevance, routing, verification and classification decisions plus the traps:
exact search, code generation, an ambiguous one-off, planted secrets and a planted
prompt-injection block; and `home-barren`, three sessions of numeric work with no judgment
in them — two exported conversations, and under `normalized/` the output of
`skills/jev-workflow-retro/scripts/transcripts.py` over all of them.
`scripts/test-skill-scripts.py` asserts against the same trees, so the thing the tests
check and the thing the model is evaluated against cannot drift apart.

`jev-pilot/fixtures/beacon` is a small support-desk repository with an incumbent keyword
rule, an existing chat-model call, labelled datasets (one scrubbed, one deliberately not),
and a set of `jev.eval/v1` reports built to carry specific traps: a no-split result that
looks excellent, a held-out run whose state carried outcome text, a revised question
re-measured on the same rows, a clean adoption candidate, and an ordinal Score whose pass
bar was set above two humans' agreement. The reports are hand-built rather than produced
by a live run — the credential available while building them was rejected. Their shape is
the CLI's own: each was checked field for field against what `jev eval --report
--show-rows` writes against a mock API, and the counts, per-class figures and Wilson
intervals agree. Three things are illustrative rather than reproducible: the split sizes
(seed 7 on these 60 rows gives 23 reported rows, not the 17 the reports use), the
fingerprints, and the omission of the `urgent` section a real run over these label files
would also carry. Anything the CLI does not print — confidence on a Choice or Score row,
the regression checks, where the labels came from — lives in `pilots/*/notes.md`, as it
would in a real pilot.

### Staging a fixture into the sandbox

**A case runs with an empty working directory, and reads above it are refused.** A case
whose subject is a file therefore has to put it there, or it scores the agent's failure
to find the file instead of its reasoning.

The mechanism is `context.scaffold_script` in a `case.yaml` beside the `prompt.md`. It
names a script **inside the case directory** -- not inline bash, and not a path that
escapes the directory:

```yaml
schema_version: "1.1"
name: audit-behaviour-pulse-finds-the-real-ones
context:
  scaffold_script: scaffold.sh
```

The script runs before the agent's first turn, with the agent's working directory as
`cwd`, and resolves the fixture from its own location:

```bash
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cp -R "$here/../../fixtures/pulse" .   # <skill>/behaviour/<case>/ is two levels down
```

`scripts/skill-eval.sh` passes `--scaffold`, which is what makes this run. `context.add_dirs`
looks like it would do the same job and does not: it validates the path and stages
nothing.

A `case.yaml` carrying only `name` and `context` coexists with `prompt.md` and
`graders/`; it does not have to restate the prompt or inline the graders.

This was found by measuring. Both `jev-opportunity-audit` fixture cases had been
scoring an empty workspace since they were written -- the skill looked like it did not
work, and what was actually broken was the case.

A case is a directory holding `prompt.md` and a `graders/` directory with one grader per
file, optionally with a `case.yaml` for what `prompt.md` frontmatter cannot express. The
directory name is the case name unless `name:` overrides it.

`results/` is gitignored. A report is one run against one model version on one day; it
is not a lasting fact about the skill, and committing timestamped HTML would bury the
cases under the noise. Paste the summary table into the pull request instead.

## Writing a case

`prompt.md` carries the user's turn, with frontmatter for how to run it:

```markdown
---
name: jev-trigger-bounded-judgment
tags: [trigger, jev]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

The prompt, written the way a real person would type it.
```

Keep `allowed_tools` read-only. A skill eval needs a prompt and a transcript, not a
shell; `scripts/skill-eval.sh` deliberately passes none of `--allow-tools`,
`--allow-real-servers` or `--mocks off`. It does pass `--scaffold`, which only runs the
committed staging scripts described above.

Prompts must be **concrete**: file names, column names, counts, a sentence of context,
the occasional lowercase sentence. An abstract prompt ("classify this data") tests
nothing, because a capable agent handles it without consulting any skill at all.

## Graders

One file per grader, frontmatter plus body.

**Did a skill fire.** This is the trigger test, and it is deterministic:

```markdown
---
type: tool_used
tool: Skill
input_match: '"skill"\s*:\s*"(?:[\w-]+:)?jev"'
---
```

**Did a skill stay away.** Same grader with `min: 0` and `max: 0`. Under ablation the
no-plugin arm passes this trivially, so the delta is zero by construction — that is
correct for a negative case, and not a sign the case is broken.

**Was the answer right.** An `llm` grader, whose body is the rubric. Write the rubric so
a reader can tell what PASS and FAIL look like without having seen the skill:

```markdown
---
type: llm
focus: last_message
---

The response asks all four judgments in a single invocation rather than four separate
calls.

PASS if the primary recommendation is one batched request.
FAIL if it recommends one call per judgment.
```

`focus` (and `regex`'s `target`) can also be `trace`, `files`, `mock_calls`, or
`{ source: file, path: … }`. Other grader types: `regex`, `tool_order`, `file_exists`,
`baseline`.

Under `--ablation with-without`, `tool_used: Skill` graders become plugin-fired
indicators rather than part of the score — unless every grader in the case is one, which
is why a pure trigger case still scores. Set `arm: with-only` to mark any other grader
the same way, and `arm: both` to force a `tool_used: Skill` grader back into the score.

## When a case fails, suspect the case first

Three of the failures found while building this suite were defects in the **case**, not
in the skill, and each one cost a round of pointless skill edits before that was noticed.
Read the transcript before you touch a `SKILL.md`.

The recurring shape is a rubric that punishes correct-but-thorough behaviour:

- **Two graders on one case pulling against each other.** One required a verdict to list
  what is still open; another read that list as hedging and failed the verdict. Both were
  reasonable alone. Fix: say in the rubric which part of the response it judges, and name
  the companion grader whose output must not count against it.
- **A rubric stricter than the skill's own contract.** A grader demanded a next action on
  "every candidate" while the skill deliberately allows minor findings as bare one-line
  bullets. Fix: quote the contract in the rubric.
- **"Traceable" read as "quoted verbatim".** A grader forbidding invented numbers failed a
  response for subtracting two reported precisions. Fix: say that arithmetic over reported
  figures is expected.

And two were defects in the **fixture**:

- **A case whose subject was never staged.** The agent correctly reported an empty working
  directory; the case scored that as a failure to reason. See the staging section above.
- **A fixture that contradicted its own rubric.** A "clean adopt" case shipped two reports
  sharing one dataset fingerprint, seed and reported rows — so the evidence actually showed
  a target retried against a split that had already refused one. The model caught it and
  was marked wrong for doing so.

**Then suspect the judge.** Much of the above traced, in the end, to the grader model
rather than to the rubric. The harness grades with Haiku by default, and Haiku is too
literal for these rubrics: it kept failing a response for *mentioning* a scope exclusion
after the rubric had been rewritten to say that stating it is correct. With a stronger
judge, the same case went from a delta of −0.22 to +0.11, and grading was about 7% of the
run's cost. `scripts/skill-eval.sh` now grades with the same model it measures; override
with `JEV_SKILL_EVAL_JUDGE_MODEL`. Every result measured before this change was graded by
Haiku, and a failure from that period is not evidence until it is re-graded.

A grader that fails a response you would have been happy to receive is a grader to fix. A
grader that fails a response you would have sent back is a skill to fix. Telling them
apart requires reading the response, which is why `--keep-temp` exists.

## Cross-skill routing

Related skills compete: operating the CLI, asking whether Jev fits a workflow, auditing
a repository, auditing past agent sessions, piloting a replacement. The suite has to
prove the intended one wins, not merely that each one fires in isolation.

See [`routing/README.md`](routing/README.md).

## What the suite does not test

Named so that a pass is not read as more than it is.

- **Execution.** Every behaviour case allows `Read`, `Glob`, `Grep` and `Skill` only. A
  grader that checks the agent did not run `curl`, start a billed `jev eval` or pipe
  history through `jev map` therefore passes by construction; what it does test is that
  the agent does not *offer* to, and does not say it did. Measuring the refusal itself
  needs Bash-enabled cases with a stub `jev` and `curl` on `PATH` that log their calls,
  and the same cases would be the first to test that the retro passes its scope flags.
- **The official `typesafe-ai` skill.** `docs/agent-skill.md` recommends installing it
  alongside these five, and its description overlaps `is-jev-useful-here` and
  `jev-opportunity-audit`, but only `skills/` is staged, so that collision is unmeasured.
- **All five skills on every held-out "none" case.** Two of them assert only that one
  skill stayed out. Held-out cases are not edited once their results have been seen, so
  the fix is a new batch written blind, not an added grader.
