# ADR-0013: Verify locally before pushing

* Status: Accepted
* Date: 2026-09-23

## Current-status amendment (2026-10-05)

The original consequence about GitHub no longer checking platforms below applies
to routine change verification. The separately authorized manual v0.3.0 release
run passed five target builds and four native CLI smoke tests on hosted runners.
It supplies no automatic check for ordinary pushes or pull requests, and ran no
model inference. The local verification decision remains in effect. See
[release verification](../release-verification.md) for the target coverage.

## Context

The repository previously ran automatic CI, security, dependency, and Scorecard
workflows. The maintainer moved verification to the contributor's machine before each
push. This reduces reliance on hosted runners, but removes independent checks on
GitHub. The trade-off must remain visible to contributors.

## Options considered

1. Keep automatic GitHub CI. Deferred: it supplies independent checks, but requires
   hosted runs for every change.
2. Use self-hosted runners. Deferred: the maintainer would need to operate and secure
   the runners.
3. Run the checks locally with a pre-push hook. Chosen.

## Decision

The four automatic workflows are removed. The manual release workflow remains and is
never dispatched without separate authorization. Contributors run
`scripts/install-hooks.sh` once per clone. Its pre-push hook runs
`scripts/verify.sh --push`, which fails if a required local check cannot run.

`scripts/verify.sh` remains the full verification command for an individual change.
The push mode adds a stricter treatment of missing local tools. Neither the hook nor
repository files can force another clone to install the hook. `git push --no-verify`
can bypass it. The hook requires a clean working tree so it verifies committed files.
The maintainer must review the verification result before a merge.

## Consequences

- Ordinary pushes, pull requests, and scheduled events run no GitHub Actions jobs.
- GitHub no longer checks Linux, macOS, and Windows independently. Contributors must
  test the platforms they claim to support, especially before releases.
- Scheduled advisory checks, independent secret scanning, coverage artifacts, and
  Scorecard reports no longer run on GitHub. The credential canary runs locally outside
  the Rust tests. Other missing independent checks remain a risk until local replacements
  exist.
- A local hook is a convenience and a prompt, not a remote enforcement mechanism.
  Branch protection must not require the retired status checks.

## Revisit if

- Independent hosted checks become practical and maintainers choose to restore them.
- A maintained self-hosted runner becomes available.
- More contributors make a local-only gate too easy to miss.
