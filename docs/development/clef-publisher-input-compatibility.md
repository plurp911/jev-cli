# Local publisher input compatibility

The 2026-10-04 final capability audit found a gap between the project's local
bridge validation and Cloudflare's published Python implementation. Both model
cards describe state as any string or JSON value. Their pinned
`joint_schema_model.py` files share Git blob digest
`6a3ac5fc030a3f33f7fa860a1271886b64a6606d`.

Primary authority is the [Clef publisher implementation](https://huggingface.co/Cloudflare/clef/blob/2f3de3dd85f379784083b0814d997ab627200f0c/joint_schema_model.py)
and the [Flash implementation](https://huggingface.co/Cloudflare/clef-flash/blob/17f0b0ad64efb65d273590632833508766b2aae6/joint_schema_model.py).
The audit also checked the current publisher source in this session. Its
`render`, `encode_record`, `question_options`, and `systemone_answer` establish:

- Present state accepts numbers, booleans, null, and blank strings alongside
  ordinary strings, objects, and arrays. Missing state remains invalid.
- Missing instructions, null, and exactly `""` fall back to the question ID.
  Whitespace strings remain whitespace; other JSON values render as JSON.
- Missing Noul criteria, overall null, and an empty object use default descriptions.
  An explicitly null side suppresses that side's default description.
- Choice descriptions may be null. Score descriptions and returned legends
  preserve JSON scalars and blank strings as well as structured values.
- The Python head enumerates options dynamically. Score26 and Score255 were
  encoded and answered by the actual publisher helpers in the offline audit.
  The bridge's previous ten-level restriction inherited TypeSafe's limit. Its
  updated 255-level bound matches this client's bounded Choice surface; it is
  not a publisher head or API limit. The minimum of two options/levels avoids
  billing a model for a question with one possible answer. The full fixed schema
  must still fit `max_length`.

These observations were also checked against the actual pinned publisher functions
using a fake tokenizer, without importing the model or executing inference. New
offline regressions first failed on scalar and blank states before the bridge fix.

This is a separately reviewed compatibility decision under `AGENTS.md` §2 and §9:
the bridge's earlier test asserting that present scalar and blank states must fail
encoded an unnecessarily narrow project restriction. That assertion was replaced
with positive exact-type/value preservation checks and a missing-state rejection
check. Request-size, finite-number, duplicate-key, nesting, media, credential, and
provider-boundary checks remain required.

The expansion belongs to the explicitly selected Hugging Face bridge. Cloudflare's
[hosted input schema](https://developers.cloudflare.com/workers-ai/models/clef/schema-input.json)
still lists string/object/array content and nonempty required instructions.
Ollama's documented generic System One schema also remains narrower, although its
Clef-specific encoder currently accepts more forms. That source/documentation
discrepancy is not a reason to broaden the generic provider contract. TypeSafe,
Cloudflare, and the other local providers keep their existing validation.
