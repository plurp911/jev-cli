# Clef documentation and skill audit

The [new audit receipt](clef-documentation-skill-audit.json) records five shipped
skills, ten project-owned developer skills, the official TypeSafe skill, and five
pinned authoring skills. Existing historical receipts remain unchanged. The official
TypeSafe skill matches current upstream bytes; all pinned authoring copies passed
their integrity check and were preserved.

Corrections cover provider-specific vision eligibility, transmitted dataset media,
local credentials and billing, alias versus weight identity, calibration boundaries,
retrospective privacy, and Clef security/release/verification guidance. Skill
descriptions and routing remain unchanged. The open Agent Skills specification,
Anthropic authoring best practices, Superpowers writing-skills, and canonical local
authoring skills informed the changes; consulted sources and final hashes are in
the receipt.

The fresh GPT-6.1 Sol high confirmations select nine focused cases with three repeats
per arm: 54 measured sessions, guide 27/27 and no-guide 15/27, with zero selected
execution errors. All four negative trigger cases passed. Results are assembled from
four explicitly recorded candidate epochs, not one final-source evaluation run.
Every selected loaded skill/reference file matches final bytes; full candidate
snapshots, differences and per-attempt traces remain hash-bound locally. This is an
instrumented portable catalog evaluation, not native client discovery. The updater
authored the fresh prompts and rubrics; fresh independent judges graded responses,
so this is not blind independent case design.

The old-guide exploratory run was already correct on four cases; no new-versus-old
behavioral improvement is claimed. There were 92 attempted sessions: four initial
sandbox startup failures and 88 completed sessions. Of those 88, 18 used newly
authored, unsatisfiable negative criteria, leaving 70 sessions with usable grading;
54 are selected for the final comparison. All runs are retained. The corrected criteria
explicitly require zero target skill calls and were measured afresh; those original
invalid scores are excluded from the selected results.

Two review findings were fixed: parser-only network behavior is distinguished from
agent model context, and processor reproduction respects `JEV_CLEF_PYTHON`. Skill
validation reported 16 valid and five pinned-vendor skips verified separately;
readiness, pinned integrity and diff checks passed. The parent's final full
`scripts/verify.sh` run remains the completion gate and must be reported separately.
No provider inference or production accuracy claim follows from this audit.

The receipt identifies a local retention archive and manifest for 492 artifacts.
Those raw target artifacts are excluded from normal Git commits; the tracked receipt
preserves their hashes without publishing the raw transcripts.
