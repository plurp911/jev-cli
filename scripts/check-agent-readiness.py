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
CANONICAL_SKILLS = ["verify", "security-review", "api-compat", "release-review", "typesafe-ai"]
PROOF_DEPENDENCIES = [
    "scripts/source-snapshot.py", "scripts/test-source-snapshot.py",
    "scripts/clef-live.py", "scripts/test-clef-live.py",
    "scripts/clef-quality.py", "scripts/test-clef-quality.py",
    "scripts/clef-python-profile.py", "scripts/test-clef-python-profile.py",
    "scripts/clef-model-manifest.py", "scripts/test-clef-model-manifest.py",
    "scripts/clef-server.py", "scripts/test-clef-server.py",
    "scripts/clef-local/clef-manifest.json", "scripts/clef-local/clef-flash-manifest.json",
    "scripts/clef-local/requirements.txt", "scripts/clef-local/requirements-linux-cpu.lock",
    "scripts/clef-local/requirements-linux-cpu.hashes.lock",
    "scripts/clef-local/requirements-linux-cpu.download.lock",
    "scripts/skill-eval-codex.py", "scripts/test-skill-eval-codex.py",
    "scripts/test-skill-eval-tools.py",
]


def check_adapters(root: Path) -> list[str]:
    """Diagnose ignored runtime copies without writing environment-owned files."""
    errors = []
    for directory in [".agents/skills", ".codex/skills", ".Codex/skills"]:
        for name in CANONICAL_SKILLS:
            relative = f"{directory}/{name}/SKILL.md"
            generated = root / relative
            canonical = root / f".claude/skills/{name}/SKILL.md"
            if not generated.exists():
                continue
            if (not generated.is_file() or not generated.resolve().is_relative_to(root.resolve())
                    or not canonical.is_file() or generated.read_bytes() != canonical.read_bytes()):
                errors.append(f"{relative} diverges from canonical .claude/skills/{name}/SKILL.md; "
                              "read the canonical skill directly and ask the environment owner to repair discovery")
    return errors


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
    for path in sorted(set(re.findall(r"<!-- readiness: ([^>]+) -->", text)) | set(PROOF_DEPENDENCIES)):
        target = root / path
        if not target.is_file() or not target.resolve().is_relative_to(root.resolve()):
            errors.append(f"missing or external readiness dependency {path}")
    for name in CANONICAL_SKILLS:
        target = root / ".claude/skills" / name / "SKILL.md"
        if not target.is_file() or not target.resolve().is_relative_to(root.resolve()):
            errors.append(f"missing or external canonical development skill {name}")
    gate = (root / "scripts/verify.sh").read_text(encoding="utf-8")
    invocations = set()
    unconditional_invocations = set()
    media_invocations = set()
    for line in gate.replace("\\\n", " ").splitlines():
        if not re.match(r"^\s*(?:run|optional|optional_module)\s", line):
            continue
        tokens = shlex.split(line, comments=True)
        # The helpers execute argv after their metadata fields. A mentioned
        # command (for example, echo python3 ...) is not an executed proof.
        command = tokens[3 if tokens[0] == "run" else 4:]
        if len(command) >= 2:
            executable, argument = command[:2]
            if executable == "python3":
                invocations.add(argument)
                if tokens[0] == "run":
                    unconditional_invocations.add(argument)
            if argument == "scripts/test-clef-server.py":
                media_invocations.add((executable, tuple(command[2:])))
    for path in ["check-agent-readiness.py", "check-architecture.py", "test-check-architecture.py",
                 "test-dev-tools.py", "test-benchmark.py", "check-request-schema.py",
                 "test-source-snapshot.py", "test-clef-live.py", "test-clef-quality.py",
                 "test-clef-python-profile.py", "test-clef-model-manifest.py",
                 "test-clef-server.py", "test-skill-eval-codex.py", "test-skill-eval-tools.py"]:
        # The schema validator is intentionally conditional on jsonschema. The
        # offline proof suites must run even when real media modules are absent.
        available = invocations if path == "check-request-schema.py" else unconditional_invocations
        if "scripts/" + path not in available:
            errors.append(f"scripts/verify.sh no longer runs {path}")
    # Offline stand-ins do not prove the real decoder/processor behavior. Preserve
    # both default and explicitly selected Python environments and their flags.
    for interpreter, flags in [("python3", ("--real-pillow",)),
                               ("python3", ("--real-processor", "--real-pillow")),
                               ("$JEV_CLEF_PYTHON", ("--real-processor", "--real-pillow"))]:
        # The decoder-only suite remains runnable without Transformers; a
        # processor invocation containing --real-pillow cannot stand in for it.
        if not any(command == interpreter and set(flags) == set(arguments)
                   for command, arguments in media_invocations):
            errors.append(f"scripts/verify.sh no longer runs real media gate {interpreter} "
                          f"scripts/test-clef-server.py {' '.join(flags)}")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check-adapters", action="store_true",
                        help="also fail on divergent ignored runtime skill copies; never repairs them")
    args = parser.parse_args()
    try:
        errors = check(ROOT)
        adapters = check_adapters(ROOT)
    except (OSError, UnicodeError, ValueError):
        print("cannot read readiness sources; restore mapped files or repair the capability map", file=sys.stderr)
        return 1
    if args.check_adapters:
        errors.extend(adapters)
    else:
        for error in adapters:
            print("warning: environment-owned skill copy: " + error, file=sys.stderr)
    for error in errors:
        print(error, file=sys.stderr)
    if errors:
        print("inspect live code/tests before updating the map; do not weaken product checks", file=sys.stderr)
        return 1
    print("agent readiness: all CLI commands map to existing behavior proofs; setup, skills, exemplars, and local gates exist")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
