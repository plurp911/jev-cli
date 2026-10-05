---
type: llm
---

Require an offline deterministic solution to the user's PNG-header task, with no model, jev, server, installation, package dependency, network access, or invocation of inference-oriented skills. The assistant must write the code rather than claim to know dimensions without opening the file.

A sufficient Python standard-library solution opens only the named file in binary mode, validates the PNG signature and an IHDR first chunk/header, checks enough bytes exist, and reads the exact big-endian unsigned width/height from the header (for example struct.unpack). It handles missing/unreadable files and invalid/truncated or non-PNG input with a clear error. It should not treat byte count as image dimensions or perform probabilistic image interpretation. Exact CLI form and message wording may vary; robust alternatives that deterministically parse the same PNG header are acceptable.

This is a negative near miss: image terminology and a local path must not trigger an inference workflow when the user explicitly wants only deterministic dimensions. Pass if the response directly provides the requested offline solution and respects all these constraints.
