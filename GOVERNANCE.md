# Governance

## What this project is

`jev` is an independent, community-maintained CLI for TypeSafe AI's System One API,
with explicitly selected Cloudflare Clef and Clef Flash providers and separately
managed local inference. It is not a TypeSafe or Cloudflare product. The aspiration is to
become the CLI the community converges on; that is earned through reliability, not
claimed through branding.

If TypeSafe ever wishes to endorse, adopt, or take over this project, that is a
conversation to have openly and to record here and in `README.md`. Until such a
statement exists, every document in this repository states plainly that the project is
unofficial.

## How decisions are made

Most changes need one maintainer approval and a reviewed local verification result.
See `AGENTS.md` §11 for the required checks and their limits.

Some changes need an **Architecture Decision Record** in `docs/adr/` and consensus
among maintainers:

- Changing the implementation language, the crate structure, or the transport model.
- Changing credential storage or the security posture.
- Breaking the CLI contract defined in `docs/cli-contract.md`.
- Adding a runtime dependency with a large transitive tree, or relaxing `deny.toml`.
- Changing the release, signing, or provenance strategy.
- Adding anything listed as permanently out of scope in `AGENTS.md` §3.3 — telemetry,
  plugins, code execution, automatic file discovery, an async runtime, self-update.

Disagreement is resolved by discussion in the pull request or issue. If maintainers
cannot reach consensus, the status quo wins: this project's failure mode should be
"did not add it", not "added it and regretted it".

While the project has a single maintainer (see [`MAINTAINERS.md`](MAINTAINERS.md)),
"consensus" is one person's decision. The ADR requirement still applies, and matters
more rather than less: with no second reviewer, the written record is the only thing
that makes a decision auditable later.

## Scope discipline

The hardest ongoing job here is saying no. A feature request is evaluated against
whether it makes `jev` a better *interface to System One and the explicitly supported
Clef providers from a shell*, not against
whether it is useful in general. Adjacent functionality belongs in another tool that
pipes into this one.

## Releases

Each tag and release requires explicit human authorization for that specific act. An
authorized agent may carry out the reviewed steps; an automated merge cannot cut a
release, push a tag, or publish a package. The release workflow runs only when it is
dispatched, and publication requires disabling its default dry run and confirming an
existing tag.
See `.github/workflows/release.yml` and `docs/adr/0014-manual-release-gate.md`.

## AI-authored code

This repository is written largely by AI coding agents under human direction. The
governance consequence is that **the maintainer must review local verification before
pushing**. The rules in `AGENTS.md` are treated as source code: changing them requires the same review as
changing the security policy, and `.github/CODEOWNERS` reflects that.

A maintainer who merges an agent-authored change is accountable for it exactly as if
they had written it.

## Changing this document

By pull request, with maintainer consensus.
