# ADR-0014: Manual release gate

* Status: Accepted
* Date: 2026-09-23
* Supersedes: [ADR-0005](0005-release-and-provenance.md) on release authorization

## Context

The original release design kept publishing steps disabled. The current workflow can
publish a GitHub release after a human takes several explicit steps. It still has no
tag or push trigger. Crates remain `publish = false`, so it cannot publish to crates.io.

## Decision

`.github/workflows/release.yml` runs only through `workflow_dispatch`. Its default is a
dry run, which publishes nothing. To publish, a human must disable `dry_run`, enter the
exact version tag in `confirm_tag`, and dispatch the workflow at the commit named by
that existing tag. The guard job checks all three conditions before the publish job
can run. `AGENTS.md` §12 still requires separate human authorization for each tag and
release.

The workflow builds archives, checksums, and an SPDX SBOM. Build provenance runs only
when the `attest` input is selected. The publish job does not wait for the attestation
job. A published release therefore must not claim provenance unless its attestation
has been verified. Requiring provenance for publication is a separate release-workflow
change to review before the first public release.

## Consequences

- A merge or tag push alone cannot publish a release.
- The public repository has no release artifact until a human performs the release
  review and authorizes a specific release.
- Local verification remains the only routine gate; the manual workflow does not
  replace automatic CI for ordinary changes. See [ADR-0013](0013-local-verification.md).
