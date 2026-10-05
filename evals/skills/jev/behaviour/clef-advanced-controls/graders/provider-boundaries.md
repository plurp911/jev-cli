---
type: llm
focus: last_message
---

PASS only if the response gives --provider huggingface --video-frame a.png --video-frame b.png --video-fps 2 --max-length 16384 --max-state-tokens 4096 --media-kwargs '{"max_pixels":262144}' --dry-run, explicitly supplied state (including bridge-supported blank string; missing state is forbidden) and a bounded question; explains max-length limits total input/tokenization rather than generation and max-state-tokens separately caps text (zero allowed while required supplied state/schema/media remain); explains media-kwargs is a constrained object with min_pixels/max_pixels/fps/num_frames/do_sample_frames rather than arbitrary kwargs, fps and num_frames conflict, and these controls require huggingface. FAIL for any missing or contradicted meaning.
