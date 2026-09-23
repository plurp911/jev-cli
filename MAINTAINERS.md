# Maintainers

This project is maintained by the community. It is not staffed by TypeSafe AI.

| Role | Responsibility |
| --- | --- |
| Maintainer | Reviews and merges changes; owns the roadmap and scope decisions. |
| Security responder | Triages private vulnerability reports; owns `SECURITY.md`, `deny.toml`, and the workflow permissions. |
| Release manager | Runs the release process defined in `docs/adr/0014-manual-release-gate.md`. Releases require a human; no agent may release. |

## Current maintainers

| Name | Role | GitHub |
| --- | --- | --- |
| plurp911 | Maintainer, security responder, release manager | [@plurp911](https://github.com/plurp911) |

This project currently has **one** maintainer, who holds all three roles. Two
consequences are worth stating plainly rather than pretending otherwise:

- **There is no second reviewer.** [`.github/CODEOWNERS`](.github/CODEOWNERS) routes
  security-sensitive paths to the same person as everything else. Its value today is
  that it marks a change as touching a protected surface, not that it adds independent
  review. Split those paths onto a dedicated security reviewer as soon as there is a
  second maintainer.
- **"Maintainer consensus" in [`GOVERNANCE.md`](GOVERNANCE.md) means one person's
  decision** until that changes. The written-ADR requirement still applies, and matters
  more rather than less: with no second reviewer, the record is the only thing that
  makes a decision auditable later.

This matters more than usual here, because the code is written by AI agents and the
human review step is a single point of failure. The maintainer reviews the local
verification result before pushing; GitHub does not provide an independent CI check.
See `AGENTS.md` §11 and `docs/threat-model.md` T14.

## Becoming a maintainer

Sustained, high-quality contribution over time, plus demonstrated judgement about what
this project should *not* do. Existing maintainers decide by consensus. See
[`GOVERNANCE.md`](GOVERNANCE.md).
