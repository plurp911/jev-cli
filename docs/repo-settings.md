# Repository settings

These settings live on GitHub, outside the repository. Review them when creating the
public repository. [ADR-0013](adr/0013-local-verification.md) explains the local
verification policy and its trade-offs.

## Actions and verification

The repository contains no automatic GitHub Actions workflows. The release workflow
runs only when explicitly dispatched for an authorized release. Actions must be enabled
for that workflow. A manual run uses hosted runners.

Install the hook in each clone with `scripts/install-hooks.sh`. Before pushing, run
`scripts/verify.sh --push` and read its summary. Hooks can be bypassed, so the person
pushing remains responsible for the result.

## Branch protection

Do not require status checks that this repository does not produce. A local hook
cannot publish a GitHub status check. Protect `main` against force pushes and deletion.

## Security and contribution

Enable Dependabot alerts and secret-scanning push protection where available. Enable
private vulnerability reporting so `SECURITY.md` has a private reporting path. These
repository features do not replace the local verification suite.

The project has one maintainer. Do not require pull-request approvals until another
maintainer can provide one; GitHub does not let a person approve their own pull request.
