#!/usr/bin/env python3
"""Assert that every source file the build needs is actually committed.

This exists because the failure it catches is invisible locally. `.gitignore` carried
`credentials.*` in its secrets section — unanchored, so git applied it at every depth —
and it matched `crates/jev-config/src/credentials.rs`. The file sat on disk, so every
local build, every test, and `scripts/verify.sh` passed. It was simply never committed.
The former CI job, which checked out what was actually pushed, failed with:

    Error writing files: failed to resolve mod `credentials`: …/credentials.rs does not exist

A working tree is not the repository. This compares the two, so the next time an ignore
rule swallows a source file it fails in one second here instead of in a release archive
built from a clean checkout.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Directories whose contents must be committed in full, with the suffixes that matter.
REQUIRED = {
    "crates": {".rs", ".toml"},
    "fuzz/fuzz_targets": {".rs"},
    "scripts": {".sh", ".py"},
    ".githooks": {""},
    ".github/workflows": {".yml"},
}

# Build output and the regenerable fuzz corpus are correctly ignored.
SKIP_PARTS = {"target", "corpus", "artifacts", "coverage"}


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout


def main() -> int:
    tracked = set(git("ls-files").splitlines())

    problems: list[str] = []
    checked = 0

    for directory, suffixes in REQUIRED.items():
        base = ROOT / directory
        if not base.is_dir():
            problems.append(f"{directory}/ is missing entirely")
            continue
        for path in sorted(base.rglob("*")):
            if not path.is_file() or path.suffix not in suffixes:
                continue
            relative = path.relative_to(ROOT)
            if SKIP_PARTS & set(relative.parts):
                continue
            checked += 1
            if relative.as_posix() in tracked:
                continue
            # Say *why* it is untracked: an ignore rule is a different problem from a
            # file someone simply forgot to `git add`.
            rule = subprocess.run(
                ["git", "check-ignore", "-v", "--no-index", relative.as_posix()],
                cwd=ROOT,
                capture_output=True,
                text=True,
            ).stdout.strip()
            # `check-ignore -v` reports the last rule that matched, which may be a
            # negation -- meaning the file is *not* ignored and was simply never added.
            # Reporting a negation as the cause would send the reader to fix the one
            # line that is already correct.
            source, _, pattern = rule.partition("\t")[0].rpartition(":")
            ignored = bool(rule) and not pattern.startswith("!")
            if ignored:
                problems.append(
                    f"{relative}: NOT COMMITTED, because an ignore rule matches it "
                    f"({source}: {pattern}). A clean checkout will not build."
                )
            else:
                problems.append(f"{relative}: not committed (never `git add`ed)")

    for problem in problems:
        print(f"::error::{problem}", file=sys.stderr)
    if problems:
        print(
            "\nA working tree is not the repository. Commit these, or narrow the "
            "ignore rule that swallowed them.",
            file=sys.stderr,
        )
        return 1

    print(f"{checked} source file(s) present in the working tree are all committed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
