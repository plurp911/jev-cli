# Packaging

Where each distribution channel's manifest comes from, and who updates it.

The Scoop and WinGet files here are unpublished templates. The release workflow
publishes GitHub archives and generates a Homebrew formula and shell installer as
release assets. Each release requires explicit human authorization; see
[`AGENTS.md`](../AGENTS.md) §12 and [ADR-0014](../docs/adr/0014-manual-release-gate.md).

## Channels

| Channel | Manifest | Generated or written? | Updated by |
| --- | --- | --- | --- |
| GitHub release archives | — | `dist build` | the release workflow |
| Homebrew | `jev-cli.rb` | `dist build --artifacts=global` | a human, after verifying the published artifacts |
| Shell installer | `jev-cli-installer.sh` | `dist build --artifacts=global` | as above |
| Scoop | [`scoop/jev.json`](scoop/jev.json) | hand-written template | a human |
| WinGet | [`winget/`](winget/) | hand-written template | a human |
| crates.io | — | `cargo publish` | a human, once `publish = false` is removed |

`dist` generates the first three. The Scoop and WinGet manifests are templates here
because `dist` does not produce them and because both channels want a pull request
against someone else's repository, which is a human act by definition.

**There is no PowerShell installer.** `dist` 0.32.0 generates one that verifies no
checksum at all, so `powershell` is not in `installers` in `dist-workspace.toml` and
`scripts/check-installers.py` fails the build if a `.ps1` reappears. Scoop and WinGet
are the Windows channels; both carry a hash, which is the whole reason they are worth
the pull request.

## Why a human fills in the checksums

Every template below has `PLACEHOLDER` where a version and a SHA-256 belong. That is
deliberate. The intended order is:

1. The release workflow builds and publishes the artifacts.
2. A human downloads them and verifies the checksums **and** the build provenance
   attestation — see [`docs/release-verification.md`](../docs/release-verification.md).
3. Only then do the manifests get the verified values.

A script that copies a checksum straight out of the build it is packaging proves
nothing: it attests that a file matches itself. The verification step has to happen
between the two, and a person has to do it.

## Install-path ordering

The README shows package managers before any download, and any download before a shell
installer. `curl | sh` is offered because people expect it, never as the headline. See
ADR-0005.

## What the installers must not do

`install-updater = false` in `dist-workspace.toml`, and it stays false.
[`AGENTS.md`](../AGENTS.md) §3.3 forbids a self-update mechanism: updates come from the
package manager the user chose, and a binary that rewrites itself is a supply-chain
surface nobody asked for. If a generated installer ever grows self-update behaviour,
that is a bug to fix, not a feature to document.
