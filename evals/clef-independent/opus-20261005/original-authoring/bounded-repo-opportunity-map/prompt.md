---
name: opus-blind-clef-bounded-repo-opportunity-map
description: Repository opportunity discovery from supplied excerpts should trigger the audit skill, even without permission to inspect files.
tags: [opus-blind-clef]
runs: 3
max_turns: 8
---

Audit this small supplied repository map for possible Jev/Clef judgment opportunities. Do not inspect the actual checkout, change files, or run a pilot. I want a ranked shortlist, including reasons to leave some code alone.

src/catalog.rs: reads a caller-selected image and asks a reviewer to choose one of four visible product-condition grades; exports the reviewer grade as JSON. We do not know inter-reviewer agreement yet.
src/ids.rs: validates item identifiers with a published checksum algorithm; existing property tests cover the checksum.
src/frames.rs: accepts an explicit list of prepared title-card JPEG frames and displays them for a human readability decision; it has no source-video decoder.
src/auth.rs: retrieves service credentials from the OS store.

There is no permission to upload anything externally or to discover additional files. A local HF bridge is available, but its offload behavior is unknown. Identify where a bounded probabilistic judgment might fit, what evidence is missing, and where deterministic code is the appropriate tool. This is an opportunity audit rather than a request to operate the CLI or grade these particular images.
