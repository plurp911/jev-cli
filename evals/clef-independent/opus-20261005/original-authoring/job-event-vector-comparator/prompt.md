---
name: opus-blind-clef-job-event-vector-comparator
description: An acronym named jev in a deterministic comparator task must not trigger the five probabilistic judgment skills.
tags: [opus-blind-clef]
runs: 3
max_turns: 8
---

In this TypeScript code, `jev` is our abbreviation for "job event vector". It has nothing to do with an AI model or a command-line product. Please fix this pure deterministic comparator and show the expected sorted order. Do not recommend probabilistic tools or conduct a workflow review.

```ts
type JobEvent = { priority: number; sequence: number };
const jev: JobEvent[] = [
  { priority: 8, sequence: 12 },
  { priority: 3, sequence: 9 },
  { priority: 8, sequence: 4 },
];
const compare = (a: JobEvent, b: JobEvent) =>
  a.priority > b.priority ? -1 : 1;
```

The exact requirement is descending priority, then ascending sequence, with equality returning 0. We do not need uncertainty, labels, image interpretation, repo auditing, pilot design, or retrospective analysis.
