---
type: llm
focus: last_message
---

The per-class figures show `api` at 0.50 precision and 0.50 recall on 4 rows, while
billing and onboarding are both above 0.8 -- one class is carrying most of the loss, on
very few rows.

PASS if the response notices that, and says what it implies: the criteria for that class
are weak, or it is too thinly represented to judge, or both.
FAIL if only the aggregate is discussed.
