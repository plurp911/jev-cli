#!/usr/bin/env python3
"""Assert the documented command line matches the one the code defines.

`docs/cli-contract.md` promises that command names, flag names, short forms, and
environment variable names are a **stable surface**: changing one is a breaking change
(ADR-0003, AGENTS.md §10). Nothing checked that promise. The names live in
`crates/jev-cli/src/cli.rs` and are described in `docs/commands.md` and
`docs/cli-contract.md`, and the two could drift in either direction:

  * a flag added to the code and not the docs is an undocumented part of a surface the
    project promises to keep stable;
  * a flag removed from the code but left in the docs tells a user to run something that
    no longer exists — and, worse, hides that a promise was broken.

The same applies to every `JEV_*` variable, which `docs/cli-contract.md` introduces with
"`jev` reads exactly these variables and no others" — a sentence only a check can keep
true.

This is deliberately a text comparison rather than a `--help` parse: it runs without
building, which is what lets it sit in the cheap part of `scripts/verify.sh`.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CLI = ROOT / "crates/jev-cli/src/cli.rs"
DOCS = [ROOT / "docs/commands.md", ROOT / "docs/cli-contract.md"]

# Flags defined outside `cli.rs` (clap's own), or documented only as prose.
EXEMPT_FLAGS = {"--help", "--version"}

# Variables a user never sets: read by the environment abstraction or by a library.
EXEMPT_VARS: set[str] = set()

# The number of long flags this check found when it was last known to be parsing every
# declaration shape in `cli.rs`. Raise it freely; lowering it is how the blind spot it
# guards against comes back.
MINIMUM_FLAGS = 37


def long_flags(source: str) -> set[str]:
    """Every `--flag` the clap definition declares."""
    flags = set()
    # `#[arg(long)]` on a field named `foo_bar` is `--foo-bar`.
    #
    # Two shapes this has to survive, both of which made a real flag invisible:
    #
    # * The attribute body is matched as "anything up to `)]`" rather than "anything with
    #   no `]`". The latter cannot match an attribute that itself contains a bracket, so
    #   `conflicts_with_all = [...]` and `value_name = "NAME[=DESCRIPTION]"` made five
    #   real flags -- --state, --state-file, --state-json, --state-json-file and --option
    #   -- invisible. They happened to be documented, so nothing failed.
    # * The visibility is `pub(...)` with any restriction, not just `pub(crate)`. A field
    #   declared `pub(super)` slipped past and was neither checked nor counted, so the
    #   floor below could not see it either.
    for match in re.finditer(
        r"#\[arg\((.*?)\)\]\s*(?:pub(?:\([^)]*\))?\s+)?(\w+)\s*:", source, re.S
    ):
        attrs, field = match.group(1), match.group(2)
        rename = re.search(r'long\s*=\s*"([^"]+)"', attrs)
        if rename:
            flags.add(f"--{rename.group(1)}")
        elif re.search(r"\blong\b", attrs):
            flags.add("--" + field.replace("_", "-"))
    return flags


def documented(texts: list[str], name: str) -> bool:
    return any(name in text for text in texts)


def main() -> int:
    source = CLI.read_text(encoding="utf-8")
    texts = [path.read_text(encoding="utf-8") for path in DOCS]
    where = ", ".join(path.name for path in DOCS)
    problems: list[str] = []

    flags = long_flags(source) - EXEMPT_FLAGS
    if not flags:
        problems.append(
            "parsed no flags out of cli.rs, so this check is asserting nothing; "
            "the clap definition's shape probably changed"
        )
    # A floor, not an equality: adding a flag must not need this number edited, but
    # losing a batch of them must fail. The attribute regex silently skipped five flags
    # because "no flags at all" was the only shape that tripped the guard above, and
    # 32 found out of 37 looks exactly like success.
    elif len(flags) < MINIMUM_FLAGS:
        problems.append(
            f"parsed only {len(flags)} flag(s) out of cli.rs, fewer than the "
            f"{MINIMUM_FLAGS} this check has seen before; the attribute regex has "
            "probably stopped matching some declaration shape. Do not lower the number "
            "to make this pass"
        )
    for flag in sorted(flags):
        if not documented(texts, flag):
            problems.append(f"`{flag}` is defined in cli.rs but documented in neither {where}")

    # Every `JEV_*` variable the workspace reads, against the contract's table.
    variables = set()
    for path in (ROOT / "crates").rglob("*.rs"):
        if "/tests/" in path.as_posix():
            continue
        variables.update(re.findall(r'"(JEV_[A-Z_]+)"', path.read_text(encoding="utf-8")))
    contract = (ROOT / "docs/cli-contract.md").read_text(encoding="utf-8")
    for variable in sorted(variables - EXEMPT_VARS):
        if variable not in contract:
            problems.append(
                f"`{variable}` is read by the code but is not in cli-contract.md's "
                "environment table, which says jev reads exactly the variables it lists"
            )

    if problems:
        print("the documented command line does not match the code:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1

    print(f"{len(flags)} flag(s) and {len(variables)} environment variable(s) documented")
    return 0


if __name__ == "__main__":
    sys.exit(main())
