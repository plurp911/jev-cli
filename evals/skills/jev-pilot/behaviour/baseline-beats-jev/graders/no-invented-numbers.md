---
type: llm
focus: last_message
---

Every quantity in the response must be traceable: either stated in the report files and
the repository, or derived from those by arithmetic.

**Arithmetic on reported figures is correct and expected**, and is not an invention:
subtracting two precisions to give the gap in points, turning a recall of 0.89 into
"about one in nine missed", converting a row count into a proportion. So is quoting a
fingerprint, a seed, a row count or an interval from a report.

PASS if every figure either appears in the fixtures or follows arithmetically from
figures that do.

That includes a confidence interval computed from reported counts — the skill tells the
reader to work one out when a report gives only a point estimate — provided the response
says it computed it rather than presenting it as part of the report.
FAIL if the response states a cost in currency, a latency, a throughput, a projected
accuracy on data that was not measured, or a saving -- any number whose source is neither
the fixtures nor arithmetic over them.
