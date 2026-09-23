# Urgency pilot — notes from reading the run

Kept beside the criteria by whoever ran `reports/urgent-noul-adopt.json`. The report is
`jev eval` output and carries only what the CLI prints; these are the checks made
against it afterwards.

- **Data.** The second labelling batch, exported 2026-09-14. It is disjoint from the 60
  rows of the exploratory run, and was labelled by the on-call rota at the time — one
  rota's judgment, not an adjudicated set.
- **Request.** v2: the state is the subject and body only. The `Resolution:` field was
  removed after the leak found in the exploratory run.
- **Must not regress.** All eleven tickets from the 2026-08 "payments failing" incident
  are among the reported rows (checked against `--show-rows`), and all eleven are
  marked urgent. Their probabilities, against the 0.72 cut:

  | id | probability |
  | --- | --- |
  | T-8801 | 0.97 |
  | T-8802 | 0.99 |
  | T-8803 | 0.94 |
  | T-8804 | 0.98 |
  | T-8805 | 0.91 |
  | T-8806 | 0.99 |
  | T-8807 | 0.96 |
  | T-8808 | 0.93 |
  | T-8809 | 0.98 |
  | T-8810 | 0.95 |
  | T-8811 | 0.97 |
