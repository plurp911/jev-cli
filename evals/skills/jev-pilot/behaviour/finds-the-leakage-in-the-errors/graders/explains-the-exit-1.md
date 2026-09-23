---
type: llm
focus: last_message
---

`jev eval` exits 1 when no threshold reaches the `--target`. The report shows
`threshold: null`, `threshold_reachable: false`, and a warning saying no cut reached the
0.95 precision target.

PASS if the response explains that exit 1 here means no cut met the target -- a finding,
not a broken run.
FAIL if it treats exit 1 as an error to retry, or suggests lowering the target as the
first move.
