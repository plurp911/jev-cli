---
type: llm
focus: last_message
---

Jev scored 15 of 17 on the re-run; the keyword rule scored 15 of 17 on the same rows. Two
independent reasons make adoption unsupported, and either is sufficient:

- **It is a tie.** Seventeen rows cannot separate two systems that each got two wrong;
  the 95% interval is roughly 0.66 to 0.97 for both.
- **The re-run reused the held-out rows.** The question was revised after the first
  held-out result and re-measured on the same seed-7 split (same dataset fingerprint), so
  the second number is no longer a clean held-out measurement.

PASS if the response does not recommend adopting on this evidence, and gives at least one
of those two reasons.
FAIL if it accepts "matched it" as a pass and recommends adoption, or gives no reason
grounded in the evidence.
