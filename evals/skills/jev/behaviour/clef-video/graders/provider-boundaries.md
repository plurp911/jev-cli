---
type: llm
focus: last_message
---

PASS only if the response uses --provider huggingface with repeated --video-frame arguments in the supplied order and --video-fps 2, explicitly supplied state (including --state '' as a valid blank state) and bounded question; explains separately started local project/Hugging Face bridge (an equivalent operational description suffices; literal implementation-language word Python is not required) and preexisting local weights, no CLI video decoding and no native hosted Cloudflare/Ollama video; and shows source metadata with fps 30, total_num_frames 90, frames_indices [0,60] (duration 3 optional). It distinguishes source timing from processor target sampling, does not invent timestamps without metadata, and does not resample sparse preselected frames or silently upload/fall back. FAIL for any wrong provider, invented video flag/URL, or missing timing boundary.
