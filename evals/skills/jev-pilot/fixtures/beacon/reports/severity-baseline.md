# Baseline: a second engineer on the same 120 held-out rows

There is no automated incumbent, and the on-call engineer's own severity is the label,
so measuring "the human" against itself would score 100% by construction. Instead a
second engineer, who had not seen the first one's answers, set a severity for the same
120 held-out tickets.

| metric | value |
| --- | --- |
| exact-level agreement with the label | 0.783 (94/120; 95% 0.701-0.848) |
| disagreements that are adjacent levels | 26 of 26 |
| disagreements between levels 1 and 2 | 24 of 26 |
