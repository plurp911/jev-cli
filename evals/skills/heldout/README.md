# Held-out routing set

Twenty-nine active prompts (originally thirty) for checking that a description change generalises, rather than merely
fitting the trigger and routing cases it was tuned on.

**How they were written.** By a separate agent that was shown only a one-line purpose for
each of the five skills and was forbidden to read any `SKILL.md`, eval case or document in
this repository. It chose each prompt's intended winner and the plausible wrong answers
from those purposes alone. At least one near-miss for each confusable pair, five prompts
that should reach none of the skills (two of them naming Jev), and a third of them
lowercase, casual, or typo'd — the way the requests actually arrive.

**How they are used.** `scripts/skill-eval.sh --heldout` stages these and nothing else, and
refuses to run under `--iterate`. Everything else excludes them. Run them once per final
candidate description, at full depth, and quote that run.

**What never happens to them.** A held-out case is not edited after its result has been
seen. If one turns out to be wrong, it is retired and a new one is written — by someone who
has not read the descriptions — because a case adjusted to agree with the descriptions is
no longer held out.


## Retirement and admission

`heldout-urgent-csv-vs-keyword` was retired on 2026-10-04 because its grader changed
after results were viewed. The corrected case remains in training at
`routing/urgent-csv-pilot-before-cli`; it is not a blind held-out case. Historical
30-case receipts, including the original 85/88 checks and labelled post-hoc regrade,
retain their original counts. Current and future runs contain 29 active cases until
a separately authored blind replacement is admitted.

`admission-manifest.json` freezes every active case file, including prompts, graders,
metadata and fixtures. The adapter validates hashes before discovery or inference;
edits, additions and removals fail. The manifest was introduced after historical
runs and is a prospective integrity control, not retroactive proof of blind authorship.
After results are seen, retire a flawed case into routing with its decision recorded
in the manifest; never update its active hashes to accept a scoring correction.
Admit new cases only before results are viewed, with independent authorship and
review of the admission-manifest diff. Keep retired decisions and old receipts.
