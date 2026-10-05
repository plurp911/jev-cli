---
type: llm
focus: last_message
---

PASS only if the response gives --provider cloudflare --cloudflare-account-id 0123456789abcdef0123456789abcdef --model clef-flash with both explicitly named --image files, a bounded noul question, --state '' (or equivalent empty text argument), and machine output. It identifies PNG/JPEG/WebP, at most four images, 4 MiB each and 8 MiB aggregate, explains directory discovery and remote image URLs are not accepted, and says the named images go to Cloudflare. FAIL if it scans files, claims omitted state is equivalent to explicit empty state, uses TypeSafe auth, or invents an automatic screenshot/upload path.
