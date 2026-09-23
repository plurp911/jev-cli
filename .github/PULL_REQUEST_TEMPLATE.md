## What this changes

<!-- One paragraph. What behaviour is different after this PR, and why. -->

## How it was verified

<!-- Required. Paste the command you ran and its result. "It compiles" is not
     verification; neither is "CI will catch it". -->

```
$ scripts/verify.sh --push
```

## Checklist

- [ ] `scripts/verify.sh --push` passes locally before pushing, or the failure is
      explained above.
- [ ] Tests were added or changed to cover the new behaviour.
- [ ] No test was deleted, `#[ignore]`d, loosened, or made non-blocking. If one was,
      it is called out above with a justification.
- [ ] No lint or local verification check was relaxed to make this pass.
- [ ] No credential, API key, token, or real endpoint appears in the diff — including
      in tests, fixtures, comments, and commit messages.
- [ ] Any change to stdout, exit codes, or the JSON output shape follows
      `docs/adr/0003-cli-compatibility.md` and is noted in `CHANGELOG.md`.
- [ ] Any new dependency is justified below and satisfies
      `docs/adr/0004-dependency-policy.md`.
- [ ] Any claim about Jev/System One API behaviour is backed by a link to the official
      TypeSafe documentation, not by recollection.

## API behaviour claims

<!-- If this PR changes how the CLI talks to TypeSafe, link the official docs page
     that establishes the behaviour. Delete this section if it does not apply. -->

## New dependencies

<!-- Name, purpose, why no existing dependency or std covers it, and a note on the
     crate's maintenance and security track record. Delete if none. -->

## Authored by

<!-- State whether this change was written by a human, by an AI agent, or both.
     This is for reviewer calibration, not a judgement. -->
