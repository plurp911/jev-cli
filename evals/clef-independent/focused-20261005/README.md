# Focused Clef privacy and lifecycle cases

Four fresh cases were independently authored using public CLI/runtime facts, without
reading the shipped guides, prior prompts, responses, or grades. Each case declares
five repetitions and twelve turns before inference. The coordinator reviewed and
admitted these cases before model calls; this is fresh independent case authoring,
not a model-blind evaluation.

`original-authoring/` preserves all original prompts and graders. `admitted/` applies
the recorded pre-inference adapter-format corrections and requires an explicit state
source for the Hugging Face image command, including an empty string. Both manifests
and the exact delta are retained. The four cases stay outside the historical/default
skill corpus and do not change its counts or held-out integrity manifest.

The comparison uses one common no-guide control, the frozen prior guide, and one
focused changed guide. Measurements and three independent judge votes use GPT 6.1 Sol
high. Source hashes and all outcomes, including errors or failed criteria, belong in
[the new phase receipt](../../../docs/development/clef-followup-skill-evaluations.json).
The earlier 225-attempt receipt and original responses/grades remain unchanged.
