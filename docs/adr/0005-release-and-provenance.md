# ADR-0005: Release, packaging, and provenance strategy

* Status: Accepted; release authorization superseded by [ADR-0014](0014-manual-release-gate.md)
* Date: 2026-09-19

The packaging strategy remains current. This ADR's unarmed release design is
historical; the current manual release gate is in
[ADR-0014](0014-manual-release-gate.md).

## Current-status amendment (2026-10-05)

The decision text below preserves the original 2026-09-19 design, including
distribution aspirations. [ADR-0009](0009-dist-as-a-builder-not-a-workflow-generator.md)
supersedes workflow generation, and [ADR-0014](0014-manual-release-gate.md)
supersedes the unarmed authorization posture. The current workflow can publish
an explicitly authorized GitHub release while every crate remains `publish = false`.
v0.3.0 was published through that guarded manual path. Current direct downloads
use `sha256.sum`, per-archive SHA-256 files and binary provenance attestations;
the source archive and SBOM are not attested, and binaries are not code-signed
or notarized. Homebrew, Scoop, WinGet and crates.io are not active installation
channels. Release notes come from the annotated tag. See
[release verification](../release-verification.md) for the published evidence.

## Context

`jev` will be installed on developer machines and in CI, where it runs with a live
TypeSafe API key in its environment. A compromised release artifact is therefore a
credential-harvesting vector across every machine that installs it. Distribution is a
security surface, not a packaging chore.

It also has to be genuinely easy to install on Linux, macOS, and Windows, or people will
not use it — and "easy" must not mean `curl | sh` as the only option, which asks users
to execute an unverified script fetched over the network as their primary install path.

## Options considered

### Hand-written release workflow

Full control, no tool dependency. Rejected as the starting point: cross-compiling six
targets, producing archives, generating checksums, wiring attestations, and maintaining
Homebrew and Scoop manifests by hand is a large amount of security-sensitive YAML to get
right and keep right.

### `dist` (formerly `cargo-dist`)

Purpose-built for exactly this: it plans a release, cross-builds a matrix, produces
tarballs, checksums, symbols, MSI and pkg installers, Homebrew formulae, and npm
packages, and generates its own CI workflow. It supports GitHub Attestations and Windows
signing under its supply-chain-security features.

Maintenance status was checked rather than assumed: `v0.33.0` was released on
2026-09-11 and the repository was pushed to within the last week. It is alive.

The cost is that it generates the release workflow, so the workflow becomes tool output
that must still be reviewed and SHA-pinned like anything else. That cost was measured
during implementation and turned out to be thirty hand edits per regeneration;
[ADR-0009](0009-dist-as-a-builder-not-a-workflow-generator.md) narrows this decision to
`dist` as a builder only, with the workflow hand-written.

### Chosen

`dist` for the build and packaging matrix, with the generated workflow reviewed,
pinned, and committed — plus attestations and SBOM generation wired explicitly, rather
than assumed to be handled.

## Decision

### Safety posture first

**The release pipeline is prepared but not armed.**

- `.github/workflows/release.yml` has **no tag trigger**. It is `workflow_dispatch`
  only, takes a `dry_run` input defaulting to `true`, and its first job *fails* if
  `dry_run` is not `true`.
- All crates are `publish = false`. Publishing requires removing that, which is a
  visible diff in a CODEOWNERS-protected file.
- Publishing steps exist in the workflow as reviewed, commented-out intent, so arming
  the pipeline is a small auditable diff rather than a rewrite.
- No agent may release. `AGENTS.md` §12 and `GOVERNANCE.md` both state that a release
  requires explicit human authorization for that specific act.

Arming requires: this ADR accepted by a human maintainer; `publish` flipped for the
intended crate; a regenerated and reviewed `dist` workflow; and repository secrets
provisioned and scoped.

### Target platforms

| Target | Priority |
| --- | --- |
| `x86_64-unknown-linux-gnu` | Tier 1 |
| `aarch64-unknown-linux-gnu` | Tier 1 |
| `aarch64-apple-darwin` | Tier 1 |
| `x86_64-apple-darwin` | Tier 1 while Intel Macs remain in use |
| `x86_64-pc-windows-msvc` | Tier 1 |
| `aarch64-pc-windows-msvc` | Tier 2 — ship when the runner and toolchain support is reliable |
| `*-unknown-linux-musl` | Tier 2 — static binaries for Alpine and distroless containers |

### Distribution channels, in order of preference

1. **Package managers** — Homebrew (macOS, Linux), Scoop and WinGet (Windows).
   First-class, because the user's package manager already handles verification and
   updates.
2. **Signed GitHub release artifacts** — tarballs and zips with `SHA256SUMS`, SBOMs,
   and build provenance attestations. Directly verifiable.
3. **`cargo install jev-cli`** — for users who would rather build from source. Requires
   flipping `publish`.
4. **A shell installer** — provided for convenience, documented alongside its checksum
   and attestation verification steps, and **never presented as the only or primary
   path**. The README will show `brew install` before it shows any `curl`.

### Integrity

Every release artifact gets:

- **SHA-256 checksums**, published alongside the artifacts.
- **GitHub build provenance attestations** via `actions/attest-build-provenance`, so a
  user can verify with `gh attestation verify` that a binary came from this repository's
  workflow at a specific commit.
- **An SPDX SBOM**, so downstream consumers can scan what is inside.
- **Reproducibility measures**: pinned toolchain, `--remap-path-prefix` to strip
  absolute paths, `SOURCE_DATE_EPOCH`, `codegen-units = 1`, and a locked dependency
  graph. Bit-for-bit reproducibility is a goal, not yet a claim; it will not be claimed
  until it is verified.

Windows code signing and macOS notarization are desirable and deferred: both need
organizational identity and paid certificates that this project does not have. Their
absence is documented rather than papered over.

### Process

1. Update `CHANGELOG.md`; the release notes come from it.
2. A human runs `scripts/verify.sh` and `/release-review`.
3. A human bumps the version and tags.
4. A human dispatches the release workflow.
5. A human verifies checksums and attestations on the published artifacts before
   updating package manifests.

## Consequences

- No release can happen by accident, by merge, or by an agent. That is the primary
  design goal of this ADR.
- `dist` becomes a build-time dependency and its generated workflow must be reviewed
  and re-pinned on every regeneration.
- Homebrew, Scoop, and WinGet manifests are ongoing maintenance.
- Users on unsupported platforms build from source.

## Revisit if

- `dist` stops being maintained, or its generated workflow stops meeting the pinning and
  least-privilege requirements in the threat model.
- The project acquires a signing identity, which would allow Windows signing and macOS
  notarization.
- Reproducible builds are verified end to end, at which point the claim can be made
  explicitly.
