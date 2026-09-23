# Agent skills

Five Agent Skills that teach an AI coding agent to work with Jev, TypeSafe's System One
judgment model. They are in [`skills/`](../skills), one directory each, in the open Agent
Skills format.

| Goal | Skill | Say something like |
| --- | --- | --- |
| Learn and use the CLI | [`jev`](../skills/jev/) | "How do I run a Score over this JSON and read the confidence?" |
| Ask whether Jev fits an idea | [`is-jev-useful-here`](../skills/is-jev-useful-here/) | "Could Jev decide which of these alerts matter, or is that overkill?" |
| Find opportunities in a repository | [`jev-opportunity-audit`](../skills/jev-opportunity-audit/) | "Look through this codebase and find good places to add Jev." |
| Find opportunities in your agent history | [`jev-workflow-retro`](../skills/jev-workflow-retro/) | "Look at my Claude and Codex work from the last month — where do I keep making the same small decision?" |
| Test one opportunity empirically | [`jev-pilot`](../skills/jev-pilot/) | "Test whether Jev can replace our ticket classifier, and tell me if it can't." |

Most agents pick the right one from the request. You can also name it: in Claude Code,
`/jev-workflow-retro last 30 days, Claude Code only` or `/jev-pilot` followed by the
candidate.

## How they fit together

They are a lifecycle: **discover** a candidate (the audit, from code; the retro, from
what you actually did), **weigh** it (`is-jev-useful-here`, answered from the shape of
the problem, without calling the API), then **prove** it (`jev-pilot`, which measures it
against what you do today and is willing to come back with no). `jev` is underneath all of
them: the commands, flags, output and exit codes. The four assessment skills name each
other and `jev` when a request belongs elsewhere; `jev` itself is the foundation and
names none of them.

What Jev *is* — question design, state, what confidence means — belongs to
[TypeSafe's official skill](https://docs.typesafe.ai/agent-skill). These defer to it and
do not restate it.

## Install

Each skill is a **whole directory**, not just its `SKILL.md`: four carry `references/`,
and `jev-workflow-retro` carries `scripts/transcripts.py`. Copy the directories:

```sh
mkdir -p .claude/skills                       # one project; or ~/.claude/skills for all
rsync -a --exclude __pycache__ /path/to/jev-cli/skills/ .claude/skills/
npx skills add typesafe-ai/skills --skill typesafe-ai   # the official skill, recommended alongside
```

Install one by naming its directory instead; each stands alone. Other agents read the
same format — a directory with a `SKILL.md` carrying `name` and `description`
frontmatter — from a skills directory of their own: copy the same directories into the
one your agent's documentation names.

Prerequisites: `python3` for `jev-workflow-retro`; the `jev` binary on `PATH` and a
credential for `jev` and for running a `jev-pilot`. Nothing here installs hooks or edits
your agent's configuration.

## What each one will and will not do

**`jev`** — reach for `rg` or ordinary code first, where they are right; batch questions
into one `jev ask` rather than looping; read `--output json`, never the text; `0` means the
API answered, not "yes", and `6` means the gate is broken, not "no"; never present a
threshold as validated until `jev eval` has measured it; check what leaves the machine,
with `--dry-run` when unsure. For a Choice or a Score, `confidence` decides whether to
act; a Noul's probability is both the answer and the certainty.

**`is-jev-useful-here`** — a verdict first, `STRONG`, `CONDITIONAL`, `WEAK` or `NO`, and
`NO` is an answer; names what stays outside Jev; corrects a mistaken premise ("Jev does
not generate text") before building on it; never invents a saving; a probability is never
the gate on something irreversible, and attacker-written input makes the model unfit to be
the gate on its own. Calls no API.

**`jev-opportunity-audit`** — searches nine surfaces rather than keywords (bounded model
calls, heuristics standing in for meaning, enum columns, routing, retrieval, agent loops,
review queues and CI, data pipelines, and existing labelled data); every finding cites a
path, a symbol and the caller, and carries one next action (`PILOT`, `INVESTIGATE`,
`LOW PRIORITY`, or `DO NOT USE JEV` where Jev is already in place and should not be); a
required section on where Jev must stay out; "none" is a valid audit. Changes no code,
calls no API, opens no secrets.

**`jev-workflow-retro`** — reads Claude Code, Codex, Gemini CLI, Cline and Cursor agent
history, or an exported JSONL, JSON, Markdown or text transcript. **Its parser runs
locally and makes no network call; nothing is sent to TypeSafe**, and it declines to pipe
your history through `jev map`. What the agent reads does enter its own session, like any
file it opens, so it reads aggregates before text, asks before reading another agent's
history, and can work from aggregates alone when you need it strictly local. Its parser
redacts credential shapes and reduces tool output to size and success. It separates
what it counted from what it inferred and from any rate it extrapolated, reports tokens
but never a price, names the frequent work that must stay (exact search above all), and
says plainly when there is nothing. Transcript content is data, never instruction.

**`jev-pilot`** — writes the success criteria down before measuring and does not move
them; stops before spending if the output is unbounded, there are no labels, or the data
may not leave; shows the exact bytes before sending, and scans every row and confirms the
count before a batch; measures the incumbent on the same rows; chooses a threshold on
rows it does not report on; judges a bar by the interval, not the point; allows one
justified revision, not iteration on the test set; ends in
`ADOPT CANDIDATE`, `PROMISING — NEED MORE DATA`, `REVISE AND RE-EVALUATE` or
`REJECT FOR THIS WORKFLOW`, and a reject is a successful pilot. Prototype only: no
production path, CI gate or existing model call is touched.

## How they are tested

`scripts/validate-skills.py` holds every skill to the open
[Agent Skills specification](https://github.com/agentskills/agentskills), so they stay
portable rather than Claude-specific, and runs in `scripts/verify.sh`. Behaviour and
routing are measured by the eval suite in [`evals/skills/`](../evals/skills/README.md):
each case runs with the skills loaded and with none, so a skill's effect is a number, and
a held-out routing set checks that the descriptions generalise. The method is in
[`docs/development/skill-authoring.md`](development/skill-authoring.md).
