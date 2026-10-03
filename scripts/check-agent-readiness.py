#!/usr/bin/env python3
"""Check the operational capability map and canonical setup paths for drift."""

from __future__ import annotations

import re
import argparse
import shlex
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MAP = "docs/development/agent-workflows.md"


def check(root: Path) -> list[str]:
    errors = []
    document = root / MAP
    if not document.is_file():
        return [f"missing {MAP}"]
    text = document.read_text(encoding="utf-8")
    rows = []
    for line in text.splitlines():
        if not line.startswith("| `"):
            continue
        cells = [part.strip().strip("`") for part in line.strip("|").split("|")]
        if len(cells) != 5:
            errors.append("capability map rows require command, source, test, proof, and outcome")
            continue
        name, source, test, proof, outcome = cells
        rows.append(name)
        for path in [source, test]:
            target = root / path
            if not target.is_file() or not target.resolve().is_relative_to(root.resolve()):
                errors.append(f"{name}: missing or external mapped path {path}")
        target = root / test
        if target.is_file() and target.resolve().is_relative_to(root.resolve()):
            # These are standard Rust integration tests. Require a runnable #[test],
            # not a same-named helper, ignored test, or conditionally disabled proof.
            source = re.sub(r"/\*.*?\*/|//[^\n]*", "", target.read_text(encoding="utf-8"), flags=re.S)
            match = re.search(r"((?:^[ \t]*#\[[^\n]+\][ \t]*\n)*)^[ \t]*fn\s+"
                              + re.escape(proof) + r"\s*\(", source, re.M)
            attributes = match[1] if match else ""
            if not re.search(r"#\[test\]", attributes) or re.search(r"#\[(?:ignore|cfg|cfg_attr)\b", attributes):
                errors.append(f"{name}: missing behavior test {proof}")
        if not outcome:
            errors.append(f"{name}: missing consumer outcome")
    cli = (root / "crates/jev-cli/src/cli.rs").read_text(encoding="utf-8")
    match = re.search(r"pub enum Command\s*\{(.*?)\n\}", cli, re.S)
    if not match:
        errors.append("cannot discover Command enum; update the readiness checker for the new CLI definition")
    else:
        commands = {name.lower() for name in re.findall(r"^    ([A-Z]\w*)\b", match[1], re.M)}
        if commands != set(rows):
            errors.append(f"capability map differs from CLI commands: missing {sorted(commands-set(rows))}, obsolete {sorted(set(rows)-commands)}")
        if len(rows) != len(set(rows)):
            errors.append("duplicate command rows in capability map")
    for path in re.findall(r"<!-- readiness: ([^>]+) -->", text):
        target = root / path
        if not target.is_file() or not target.resolve().is_relative_to(root.resolve()):
            errors.append(f"missing or external readiness dependency {path}")
    for name in ["verify", "security-review", "api-compat", "release-review", "typesafe-ai"]:
        if not (root / ".claude/skills" / name / "SKILL.md").is_file():
            errors.append(f"missing canonical development skill {name}")
    gate = (root / "scripts/verify.sh").read_text(encoding="utf-8")
    invocations = set()
    for line in gate.replace("\\\n", " ").splitlines():
        if not re.match(r"^\s*(?:run|optional|optional_module)\s", line):
            continue
        tokens = shlex.split(line, comments=True)
        for executable, argument in zip(tokens, tokens[1:]):
            if executable == "python3":
                invocations.add(argument)
    for path in ["check-agent-readiness.py", "check-architecture.py", "test-check-architecture.py",
                 "test-dev-tools.py", "test-benchmark.py", "check-request-schema.py"]:
        if "scripts/" + path not in invocations:
            errors.append(f"scripts/verify.sh no longer runs {path}")
    return errors


def main() -> int:
    argparse.ArgumentParser(description=__doc__).parse_args()
    try:
        errors = check(ROOT)
    except (OSError, UnicodeError, ValueError):
        print("cannot read readiness sources; restore mapped files or repair the capability map", file=sys.stderr)
        return 1
    for error in errors:
        print(error, file=sys.stderr)
    if errors:
        print("inspect live code/tests before updating the map; do not weaken product checks", file=sys.stderr)
        return 1
    print("agent readiness: all CLI commands map to existing behavior proofs; setup, skills, exemplars, and local gates exist")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
