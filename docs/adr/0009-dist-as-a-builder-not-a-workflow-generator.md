# ADR-0009: `dist` builds the artifacts; it does not own the workflow

* Status: Accepted
* Date: 2026-09-19
* Refines: [ADR-0005](0005-release-and-provenance.md), which chose `dist` and accepted
  "the workflow becomes tool output that must still be reviewed and SHA-pinned like
  anything else" as its cost. This ADR records what that cost turned out to be, and
  narrows the decision accordingly.

The `dist`-as-builder decision remains current. The release authorization examples
below describe the original unarmed workflow; [ADR-0014](0014-manual-release-gate.md)
records the current manual publish gate.

## Context

ADR-0005 chose `dist` for the build and packaging matrix, with its generated workflow
"reviewed, pinned, and committed". Implementing that revealed how large the review is.

`dist 0.32.0` was installed and run against this repository. `dist generate` produces a
296-line workflow. Measured against this repository's own rules:

| Requirement | `dist`-generated workflow |
| --- | --- |
| No tag trigger until the pipeline is armed (ADR-0005) | Has `push: tags:` **and** `pull_request` |
| Every third-party action pinned to a commit SHA (threat model T11) | 16 actions on mutable tags |
| Least privilege per job, each grant commented | `permissions: contents: write` at workflow level |
| `zizmor --persona=pedantic` clean (CI gate) | 32 findings: 19 high, 3 medium, 2 low, 8 informational |
| A concurrency group | Absent |

Every one of those would have to be fixed by hand **after every regeneration** — roughly
thirty edits, of which sixteen are commit SHAs that must be looked up individually. That
is not a review; it is a rewrite performed repeatedly, and the failure mode is silent:
a pin that does not get reapplied looks exactly like one that did.

Meanwhile the part of `dist` that is genuinely hard to replace — cross-platform
archives, per-artifact checksums, a Homebrew formula, shell and PowerShell installers,
a source tarball, and consistent naming — works perfectly well on its own.

## Options considered

1. **Use the generated workflow, patch it after each regeneration.** ADR-0005's literal
   reading. Rejected: thirty manual edits per regeneration, sixteen of them SHA lookups,
   with a silent failure mode. The cost lands on exactly the file where a mistake is
   most expensive.
2. **Drop `dist` and hand-roll packaging.** Rejected: archives, checksums, installers,
   and a Homebrew formula across five targets is a large amount of security-sensitive
   shell to write and keep right, which is the reason ADR-0005 chose a tool.
3. **Fork or vendor `dist`'s template.** Rejected: it decouples from upstream at the
   first version bump and leaves us maintaining someone else's YAML generator.
4. **Use `dist` as a builder, keep the workflow hand-written.** Chosen.

## Decision

**`dist` is invoked as a command; it never generates or manages a workflow.**

- `dist-workspace.toml` sets `allow-dirty = ["ci"]`. That is the mechanism, not a
  comment: with it, `dist build` works and `dist` stops asserting ownership of
  `.github/workflows/release.yml`.
- `dist generate` is not run by any workflow, any script, or `scripts/verify.sh`.
- `.github/workflows/release.yml` stays hand-written: no tag trigger, dispatch-only,
  a `guard` job that fails unless `dry_run` is true, every action SHA-pinned, per-job
  permissions with a comment on each grant, and zero `zizmor --persona=pedantic`
  findings.
- The workflow calls `dist plan` and `dist build --artifacts=local --target …` per
  runner, then verifies the checksum `dist` wrote rather than trusting it.
- `install-updater = false`, which is the machine enforcement of `AGENTS.md` §3.3's ban
  on a self-update mechanism. With it true, `dist` bundles `axoupdater` into the
  installers.
- Scoop and WinGet manifests are hand-written templates in `packaging/`, because `dist`
  does not produce them and because both channels require a pull request against
  somebody else's repository — a human act by definition.
- `scripts/release-dry-run.sh` rehearses the whole thing locally for the host target:
  build, verify the checksum, generate the SBOM, unpack the archive, run the binary from
  it, and confirm the licences are inside.

### What this costs

`dist`'s own release orchestration — creating the GitHub release, uploading, announcing,
updating the Homebrew tap — is not used. Those steps have to be written into the
workflow when the pipeline is armed. That is a one-time cost on a file that is reviewed
line by line anyway, and it is smaller than thirty edits per regeneration forever.

### What this keeps

Upgrading `dist` remains a single version bump in `dist-workspace.toml` with no workflow
churn, because the workflow does not depend on `dist`'s template — only on its
command-line interface.

## Consequences

- The release workflow is `zizmor --persona=pedantic` clean and stays that way, because
  nothing regenerates it.
- Bumping `cargo-dist-version` is a reviewed one-line change.
- MSI is not produced. `dist` requires WiX GUIDs and a Windows toolchain for it, and
  ADR-0005's Windows channels are Scoop and WinGet, neither of which needs one. A
  portable `.zip` is what both consume.
- Artifacts are named `jev-cli-<target>` after the Cargo package, while the binary
  inside is `jev`. Renaming the package is a crates.io decision for a human to make when
  publishing is armed; until then the mismatch is documented rather than worked around.

## Revisit if

- `dist` gains a way to emit a workflow that is SHA-pinned, least-privilege, and
  trigger-free, at which point option 1 becomes cheap.
- The release workflow grows enough orchestration that `dist`'s own is clearly less
  work, even counting the hardening.
