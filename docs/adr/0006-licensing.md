# ADR-0006: Dual `MIT OR Apache-2.0`

* Status: Accepted
* Date: 2026-09-19

## Context

The licence has to suit a developer CLI that we want adopted widely — installed by
individuals, vendored into corporate CI, and possibly packaged by distributions. It also
has to suit the Rust ecosystem, whose norms shape what contributors and legal reviewers
expect.

## Options considered

### MIT only

Short, universally understood, maximally permissive, and trivially approved by corporate
legal teams. Its gap is patents: MIT grants no express patent licence. A contributor
holding a patent on something they contributed is not clearly estopped from asserting it.
For a small tool the practical risk is low, but the ambiguity is real and some corporate
reviewers flag it.

### Apache-2.0 only

Includes an express patent grant and a defensive patent-termination clause, plus explicit
contribution and trademark terms. Its gap is compatibility: Apache-2.0 is incompatible
with GPL-2.0-only, which would block some downstream reuse. It is also longer, and some
reviewers treat it as heavier than a small tool warrants.

### Dual `MIT OR Apache-2.0`

The Rust ecosystem standard — used by the Rust project itself and by the overwhelming
majority of crates, including every dependency here. The user picks either licence, so
they get MIT's compatibility *or* Apache-2.0's patent grant, whichever they need.

The cost is real but small: two licence files, a slightly longer notice, and a
contribution clause.

### Copyleft (MPL-2.0, GPL-3.0)

Not considered seriously. Copyleft on a developer CLI reduces adoption, complicates
vendoring into corporate CI, and serves no goal this project has.

## Decision

**Dual-licensed `MIT OR Apache-2.0`**, at the user's option.

- `LICENSE-MIT` and `LICENSE-APACHE` at the repository root.
- `license = "MIT OR Apache-2.0"` in every crate manifest.
- `README.md` carries the standard contribution clause: contributions are dual-licensed
  under the same terms unless stated otherwise.
- `CONTRIBUTING.md` states the same, and forbids adding files under other licences or
  removing existing headers and attribution.

No CLA. It raises the barrier to contribution for a project this size, and the
dual-licence contribution clause covers what a CLA would.

## Consequences

- Maximum downstream compatibility, including with GPL-2.0-only projects via the MIT
  arm, and an express patent grant available via the Apache-2.0 arm.
- Matches every dependency's licence, so the combined tree has no friction, and matches
  what Rust contributors already expect.
- Two files and a slightly longer notice. Accepted.
- **The licence covers the code, not the name.** "TypeSafe", "System One", and "Jev" are
  not ours. They are used descriptively to say what this tool talks to. Nothing in this
  repository may imply endorsement; see `README.md` and `GOVERNANCE.md`.

## Revisit if

- TypeSafe raises a trademark concern about the binary name or the repository name. That
  is a naming question, not a licensing one, but it would be decided here.
- A downstream packager reports a concrete licensing obstacle.
