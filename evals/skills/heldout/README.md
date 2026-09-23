# Held-out routing set

Thirty prompts for checking that a description change generalises, rather than merely
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
