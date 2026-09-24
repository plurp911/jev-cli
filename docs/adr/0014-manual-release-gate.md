# ADR-0014: Manual release gate

* Status: Accepted
* Date: 2026-09-23; amended 2026-09-23 for the first public release
* Supersedes: [ADR-0005](0005-release-and-provenance.md) on release authorization

## Context

The original release design kept publishing steps disabled. The current workflow can
publish a GitHub release after a human takes several explicit steps. It still has no
tag or push trigger. Crates remain `publish = false`, so it cannot publish to crates.io.

## Decision

`.github/workflows/release.yml` runs only through `workflow_dispatch`. Its default is a
dry run, which publishes nothing. To publish, the operator must disable `dry_run`, enter
the exact version tag in `confirm_tag`, select `attest`, and dispatch the workflow at
the commit named by that existing tag. The guard job checks these conditions before
the publish job can run. `AGENTS.md` §12 still requires separate human authorization
for each tag and release.

The workflow builds archives, checksums, and an SPDX SBOM. A dry run can skip build
provenance. Publication waits for the attestation job to succeed. The release manager
still verifies the attestation on a downloaded artifact before claiming provenance:
the workflow gate proves that a statement was minted, while the download check proves
that it covers the published bytes.

## Consequences

- A merge or tag push alone cannot publish a release.
- A release artifact exists only after a human authorizes that specific release.
- Local verification remains the only routine gate; the manual workflow does not
  replace automatic CI for ordinary changes. See [ADR-0013](0013-local-verification.md).
