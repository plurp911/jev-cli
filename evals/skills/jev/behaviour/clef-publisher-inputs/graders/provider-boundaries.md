---
type: llm
focus: last_message
---

PASS only if the response:
- Gives `jev --provider huggingface --model clef-flash ask -r compatibility.json --dry-run` (equivalent flag ordering and `--request` spelling accepted), keeps the named request's JSON types and supplied state, and does not call inference, install a runtime, obtain credentials, or fall back to cloud.
- Explains this project's selected Hugging Face bridge accepts present JSON boolean/scalar/null and blank state, plus JSON scalar/null descriptions and Score legends; missing state remains invalid. No coercion to text or arbitrary removal of the 26 supplied Score levels is required.
- Says missing/null/exactly-empty instructions use the question ID, while whitespace is preserved. Explicitly null Noul side criteria suppresses that side's default description; absent side uses its default, and overall null criteria uses defaults.
- Distinguishes this client's Hugging Face Score bound of 2–255 from the dynamic publisher head/API: 255 is a client resource bound, and the fixed schema must still fit the total tokenizer budget. TypeSafe/Cloudflare stay at 2–10 and generic Ollama at 2–26; do not infer their scalar-state/blank-instruction acceptance from the publisher bridge.
FAIL for any missing or contradicted requirement, invented provider capability, or silent payload coercion.
