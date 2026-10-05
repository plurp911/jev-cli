# Architecture Decision Records

Each ADR records one decision: the context, the options considered, what was chosen,
and what it costs. They are written so that a future contributor — human or agent — can
tell whether a decision still holds, instead of re-litigating it from scratch.

An ADR is never edited to change its decision. It is superseded by a new one.

| # | Decision | Status |
| --- | --- | --- |
| [0001](0001-implementation-language.md) | Rust as the implementation language | Accepted |
| [0002](0002-security-and-credentials.md) | Credential storage: secure storage or environment, no plaintext fallback | Accepted; ordering superseded by [0008](0008-credential-precedence-and-endpoint-isolation.md) |
| [0003](0003-cli-compatibility.md) | The CLI is the product; the crates promise nothing | Accepted |
| [0004](0004-dependency-policy.md) | Minimal dependencies, enforced by `cargo-deny` | Accepted |
| [0005](0005-release-and-provenance.md) | Release and packaging strategy | Accepted; release authorization superseded by [0014](0014-manual-release-gate.md), builder choice refined by [0009](0009-dist-as-a-builder-not-a-workflow-generator.md) |
| [0006](0006-licensing.md) | Dual `MIT OR Apache-2.0` | Accepted |
| [0007](0007-workspace-architecture.md) | Four crates; blocking transport behind a trait | Accepted |
| [0008](0008-credential-precedence-and-endpoint-isolation.md) | Environment before keychain; a separate credential namespace per endpoint | Accepted |
| [0009](0009-dist-as-a-builder-not-a-workflow-generator.md) | `dist` builds the artifacts; it does not own the workflow | Accepted; release authorization updated by [0014](0014-manual-release-gate.md) |
| [0010](0010-batch-evaluation-and-semantic-routing.md) | JSONL batch framing, semantic routing, and no bulk-content prompt | Accepted |
| [0011](0011-threshold-calibration.md) | `jev eval`: held-out threshold calibration, and the statistics this CLI refuses | Accepted |
| [0012](0012-mcp-server.md) | A local, stdio-only MCP server, and the one async runtime it needs | Accepted |
| [0013](0013-local-verification.md) | Local verification before push; no automatic GitHub Actions | Accepted |
| [0014](0014-manual-release-gate.md) | Manual release gate; provenance required to publish, optional in rehearsals | Accepted |
| [0015](0015-clef-providers-and-vision.md) | Explicit hosted/local Clef providers and bounded vision | Accepted |

## When an ADR is required

See [`GOVERNANCE.md`](../../GOVERNANCE.md). In short: language and structure, security
posture, CLI contract breaks, heavyweight dependencies, release strategy, and anything
listed as permanently out of scope in `AGENTS.md` §3.3.

## Template

```markdown
# ADR-NNNN: Title

* Status: Proposed | Accepted | Superseded by ADR-MMMM
* Date: YYYY-MM-DD

## Context
## Options considered
## Decision
## Consequences
## Revisit if
```
