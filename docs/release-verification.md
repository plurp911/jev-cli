# Verifying a release

> **No public release artifact exists yet.** This document describes the verification steps that will
> apply when one does, so that the process is reviewable before it is used rather than
> after. The release pipeline runs only after a human dispatches it; see
> [ADR-0014](adr/0014-manual-release-gate.md).

`jev` runs on developer machines and can hold a live API key in its environment. A
compromised artifact is therefore a credential-harvesting vector across every machine
that installs it. That is why this page exists and why `curl | sh` will never be the
only install path.

## Prefer a package manager

Homebrew, Scoop, and WinGet already handle checksum verification, and they handle
updates. Use one if you can. The verification below is for a directly downloaded
artifact.

## 1. Checksums

Every release publishes two forms, both produced by `dist`:

- `sha256.sum` — one aggregate file covering every archive and the source tarball;
- `<archive>.sha256` — one file per archive.

```sh
base=https://github.com/plurp911/jev-cli/releases/download/vX.Y.Z
curl -LO "$base/jev-cli-x86_64-unknown-linux-gnu.tar.xz"
curl -LO "$base/sha256.sum"
sha256sum --check --ignore-missing sha256.sum
```

The archives are `.tar.xz` (and `.zip` on Windows), and the aggregate file is
`sha256.sum` — not `SHA256SUMS`, which is a common convention this pipeline does not
use.

A checksum proves the file was not corrupted or swapped in transit. It does **not**
prove who built it — the checksum file comes from the same place as the artifact. For
that, use the attestation.

### What the installers verify

| Install path | Verifies the download |
| --- | --- |
| `jev-cli-installer.sh` | **Yes**, sha256 against a digest embedded at build time |
| Homebrew formula | **Yes**, `sha256` per platform, checked by `brew` |
| Scoop / WinGet manifests | **Yes**, `hash` / `InstallerSha256` |
| `.zip` + `sha256.sum` | **You do**, with the command above |

**There is no PowerShell installer, deliberately.** `dist` 0.32.0 generates one with no
checksum verification at all — no `Get-FileHash`, no embedded digest, no verification
step; it downloads over HTTPS and runs whatever it is served. Shipping that as the
Windows install path for a tool whose premise is the paragraph at the top of this page
would have been incoherent, so `powershell` is not in `installers` in
`dist-workspace.toml`. On Windows, use Scoop or WinGet, or download the `.zip` and
verify it against `sha256.sum`.

The embedded digests are not automatic. `dist build --artifacts=global` learns them
from the `*-dist-manifest.json` files each per-target build leaves in `target/distrib`;
without those it still succeeds, and emits a shell installer that prints "no checksums
to verify" and a Homebrew formula with a `url` and no `sha256`. Nothing about the output
looks wrong. `scripts/check-installers.py` runs in the manual release workflow and
`scripts/release-dry-run.sh`, failing the build in that case, per target.

## 2. Build provenance attestation

The release workflow creates an attestation only when a human selects `attest: true`.
Publishing does not wait for that optional job. Check that an attestation exists for
the artifact before treating a release as provenance-backed. If none exists, verify
the checksum and consider building from source instead.

When selected, GitHub signs a statement about which workflow, in which repository, at
which commit, produced the artifact.

```sh
gh attestation verify jev-cli-x86_64-unknown-linux-gnu.tar.xz --repo plurp911/jev-cli
```

A successful verification tells you the binary was built by this repository's release
workflow from a specific commit — not by someone who obtained a release token, and not
on a maintainer's laptop. If it fails, **do not install the artifact**, and please open a
security report.

**Read the ref and the commit, not just the pass.** The release workflow can be run
manually with `attest: true` to rehearse this step, and a rehearsal attestation is
cryptographically indistinguishable from a real one: same repository, same workflow,
same signing identity. What distinguishes them is *what* was attested. So check that
the attested ref is the release tag you expect and the commit is the one the tag points
at:

```sh
gh attestation verify jev-cli-x86_64-unknown-linux-gnu.tar.xz \
  --repo plurp911/jev-cli --format json \
  | jq -r '.[].verificationResult.signature.certificate
           | .sourceRepositoryRef, .sourceRepositoryDigest'
git rev-parse vX.Y.Z^{commit}   # must equal the digest above
```

An attestation whose ref is `refs/heads/main` rather than `refs/tags/vX.Y.Z` is a
rehearsal, not a release.

Not one project in the Jev CLI landscape surveyed in
[`research/jev-cli-landscape.md`](research/jev-cli-landscape.md) publishes attestations,
and only one publishes checksums. Being verifiable is a deliberate differentiator.

## 3. SBOM

Each release carries an SPDX software bill of materials listing every crate compiled
in, with versions. Feed it to your own scanner:

```sh
grype sbom:jev.spdx.json     # or trivy, syft, osv-scanner, …
```

**The SBOM is not itself attested.** It is generated in its own job from the same
commit, but it carries no provenance statement, so treat it as a convenience for
scanning rather than as evidence about the binary. See the current release gate in
[ADR-0014](adr/0014-manual-release-gate.md).

## 4. What is not claimed

Stated plainly, because absent guarantees are the ones people assume:

- **Windows binaries are not code-signed**, and **macOS binaries are not notarized.**
  Both require an organizational identity and paid certificates this project does not
  have. Expect a SmartScreen or Gatekeeper prompt.
- **Builds are not yet verified reproducible.** The toolchain is pinned, paths are
  remapped, `SOURCE_DATE_EPOCH` is set, and the dependency graph is locked — but
  bit-for-bit reproducibility has not been demonstrated end to end, so it is not
  claimed. When it is verified, this page will say so and show how to check.

## Building from source instead

The most direct verification is to build it yourself:

```sh
git clone https://github.com/plurp911/jev-cli
cd jev-cli
git verify-tag vX.Y.Z      # once tags are signed
cargo build --release --locked
```

`--locked` is important: it builds the exact dependency graph in the committed
`Cargo.lock` rather than resolving fresh.

## Reporting a problem

If verification fails, or an artifact looks wrong, follow [`SECURITY.md`](../SECURITY.md).
Do not open a public issue with the details of a suspected compromised artifact.
