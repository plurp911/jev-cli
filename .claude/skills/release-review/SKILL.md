---
name: release-review
description: >
  Assess whether a release is ready — artifacts, changelog, versioning, checksums,
  SBOM, provenance attestations, and publishing prerequisites — without publishing
  anything. Use before a human cuts a release, when reviewing a change to the release
  workflow or packaging configuration, or to answer "could we ship this?". This skill
  never publishes, tags, or pushes.
allowed-tools: Bash Read Grep Glob
---

# Release review

> **This skill never releases anything.** It does not run `git push`, `git tag`,
> `gh release create`, or `cargo publish`, and it does not dispatch the release
> workflow with `dry_run: false`. Releasing is a human act requiring explicit
> authorization for that specific release — `AGENTS.md` §12, `GOVERNANCE.md`,
> `docs/adr/0005-release-and-provenance.md`. If asked to release, produce this review
> and hand it to a human.

## 0. Confirm the pipeline is still disarmed

Unless a human has explicitly armed it for this release, all of these must hold:

```sh
rg -n 'publish' Cargo.toml crates/*/Cargo.toml   # expect publish = false / publish.workspace = true
rg -n 'push:|tags:' .github/workflows/release.yml # expect NO tag trigger
rg -n 'dry_run' .github/workflows/release.yml     # expect default true and a failing guard
```

If any has changed, that is the finding. Report it before anything else.

## 1. Version and changelog

```sh
rg -n '^version' Cargo.toml
rg -n '^## \[' CHANGELOG.md | head
git log --oneline "$(git describe --tags --abbrev=0 2>/dev/null || git rev-list --max-parents=0 HEAD)"..HEAD
```

- Does the version follow semver **as applied to the CLI**, not the crates? See
  ADR-0003.
- Is there a `CHANGELOG.md` entry for every user-visible change in that commit range —
  a command, a flag, an exit code, a JSON field, an environment variable, or a change
  in what is sent over the network?
- Is any change to the stable surface in `docs/cli-contract.md` reflected in the version
  bump? A removed or renamed JSON field, a repurposed exit code, or a changed flag
  meaning is a **major** change.
- Are all crate versions consistent?
- Does the changelog claim anything that is not actually implemented?

## 2. The build

```sh
scripts/verify.sh
cargo build --workspace --locked --release
```

- Does verification pass in full, with no `skip` on a check that matters for a release?
  For a release, `cargo-deny`, `zizmor`, and the test suite must all have actually run.
- Does the release profile build cleanly? It differs from dev: `panic = "abort"`, LTO,
  `strip = "symbols"`.
- Is `Cargo.lock` committed and current?
- Do the target platforms in `.github/workflows/release.yml` still match ADR-0005?

## 3. Artifacts

For each platform the release claims to support:

- Is there a build job for it?
- Does the binary run and report the expected version?
- Is `--help` correct, and does it still carry the unofficial/community disclaimer?

```sh
./target/release/jev --version
./target/release/jev --help
./target/release/jev doctor
```

## 4. Integrity

Per ADR-0005, a release is not ready without all of:

| Requirement | Check |
| --- | --- |
| SHA-256 checksums for every artifact | Present, and they actually match |
| SPDX SBOM | Generated, and lists the real dependency set |
| Build provenance attestation | `actions/attest-build-provenance` runs, with `id-token: write` and `attestations: write` scoped to that job only |
| Reproducibility measures | `--remap-path-prefix`, `SOURCE_DATE_EPOCH`, pinned toolchain, `--locked` |
| Pinned actions | Every third-party action is a commit SHA with a matching version comment |
| Least privilege | `permissions: {}` at workflow level; each job's grants commented |

Verify an attestation as a user would:

```sh
gh attestation verify <artifact> --repo plurp911/jev-cli
```

If reproducibility has not actually been verified end to end, the release notes must not
claim reproducible builds. ADR-0005 states it as a goal, not a claim.

## 5. Credential hygiene in what ships

```sh
strings target/release/jev | rg -i 'sk-|bearer |api[_-]?key' | head
rg -ni 'sk-[a-z0-9]{8,}' CHANGELOG.md README.md docs/
```

- No credential, token, or internal endpoint anywhere in the binary, the notes, or the
  docs.
- No debug symbols or absolute source paths leaking a maintainer's home directory.
- Running every command with a canary key prints nothing. Run the local
  `scripts/credential-canary.sh` and review its limits before a release.

## 6. Publishing prerequisites

Only relevant once a human has decided to arm the pipeline.

- **crates.io**: `publish = false` removed only for the crate being published; all
  required manifest metadata present (`description`, `license`, `repository`,
  `readme`, `keywords`, `categories`); `cargo publish --dry-run` clean. Note that
  publishing a binary crate whose path dependencies are `publish = false` will fail —
  resolve that deliberately, not by flipping every crate.
- **Homebrew / Scoop / WinGet**: manifests point at the right URLs and checksums.
- **Secrets**: scoped to the release environment only, and the environment has a human
  approval gate.
- **Documentation**: installation instructions lead with package managers, not with a
  shell installer. ADR-0005 requires this.

## 7. Report

```
Release readiness: READY / NOT READY

Blocking:
  - <what, and the specific fix>

Non-blocking:
  - <what>

Verified:
  - <what you actually ran and what it said>

Not verified:
  - <what you could not check, and why>
```

Be conservative. A release is hard to unship, and a broken or leaky release from a
community CLI that handles API keys does lasting damage to the project's credibility.
End by restating that a human must perform the release.
