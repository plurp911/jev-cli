# Urgency pilot — success criteria

Written 2026-09-10, before the second labelling batch was exported or any request was
sent against it. Agreed by the on-call lead.

**Bar: precision of at least 0.90 on held-out rows, with the lower end of its 95%
interval at or above 0.90.** A ticket wrongly marked urgent pages someone at night; a
missed one waits until morning. Recall is reported, not gated.

**Must not regress:** the eleven "payments failing" tickets from the 2026-08 incident
are all in the reported rows and must be marked urgent.

## History

An exploratory run on the first 60 rows, from `data/urgency-v1-export.jsonl`
(`reports/urgent-noul-heldout.json`), used
`--target 0.95` as a stretch figure, before any bar was agreed. It found the request was
leaking the `Resolution:` field into the state. That run is not evidence for or against
this bar: the request was changed (v2 drops the field) and the data is a disjoint batch.
