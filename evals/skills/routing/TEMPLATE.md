# Routing case template

Copy this into `routing/<contested-phrasing>/` as `prompt.md` plus one file per grader
under `graders/`. This file is not a case: the harness discovers cases by `prompt.md`
and `case.yaml`, so a template named `TEMPLATE.md` is inert.

## `prompt.md`

```markdown
---
name: routing-<short-description-of-the-collision>
tags: [routing]
runs: 3
max_turns: 6
allowed_tools: [Read, Glob, Grep, Skill]
---

<The sentence, written the way it actually arrives. Concrete: a repository, a file, a
person, a deadline. If a reader cannot tell which skill ought to win without being told,
the prompt is too abstract to be a routing test.>
```

## `graders/intended-skill-fired.md`

```markdown
---
type: tool_used
tool: Skill
input_match: '"skill"\s*:\s*"(?:[\w-]+:)?<intended-skill>"'
---
```

## `graders/<adjacent-skill>-did-not-fire.md`

One of these per adjacent skill. Listing only the nearest neighbour is how a collision
gets missed: add every skill whose description a reasonable person could argue for.

```markdown
---
type: tool_used
tool: Skill
input_match: '"skill"\s*:\s*"(?:[\w-]+:)?<adjacent-skill>"'
min: 0
max: 0
---
```

## Reading the result

The no-plugin arm passes every negative grader for free — no skills are loaded, so none
of them fire. Read the **with** column for routing, and treat the delta as information
about the positive grader only.
