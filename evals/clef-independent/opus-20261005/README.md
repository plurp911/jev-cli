# Independent Clef generalization batch

This separate six-case dataset contains three behavior cases, two positive trigger
cases and one negative case covering the five shipped skills. It is outside
`evals/skills/` and is not part of the historical 30-case or prospective 29-case
routing corpus.

`original-authoring/` preserves all 19 case files and the original author note and
admission manifest. The author reports reading only the supplied purposes and
public primary sources, without shipped guides, prior cases or measured answers.
`admitted/` is the runnable copy. Before any model answers, ten unsupported
`expected` fields became exact skill-input checks, five negative checks received
explicit minimum zero, and one sparse-video input-token limit was corrected from
generation-length terminology. The limits, intended skills and other criteria
remain unchanged. The exact before/after hashes are in
`pre-inference-format-delta.json` and `provenance.json`.

The separately frozen local runner stages only `admitted/` as a six-case heldout
dataset with a compatible hash manifest. It uses the unchanged Codex app-server
evaluation main, three paired repeats and GPT-6.1 Sol high for measurements and
judges. Trigger verdicts and behavioral scores are reported separately. Source
and orchestration hashes, commands, errors, usage and results belong to the local
experiment receipt under `target/clef-local/opus-corrections-skill-evals/` and
ignored `evals/skills/results/` directories. No prompt, grader or guide was tuned to answers during the original
`0eb5...` inference phase. Prompts, graders and grades remain unchanged.

The input-budget correction follows the publisher's
[encode_record implementation](https://huggingface.co/Cloudflare/clef/raw/main/joint_schema_model.py):
`max_length` bounds input schema, media and state tokens. It is not generation
length. The admission manifest is an integrity record before inference; it does
not attest backend model weights or turn-level compute.

After the original batch completed, Claude's review identified that its CPU-timeout
criteria were absent from the evaluated provider guide (the sparse prompt also did
not request the exact 600-second value). The resulting general CPU deadline, retry
and cancellation guidance is explicitly informed by these blind results. Its
separate training evaluations have a new source binding; they do not turn the
original four failing responses into passes. A later replay of these six cases
would no longer be blind. The original authoring/admission files remain unchanged.
