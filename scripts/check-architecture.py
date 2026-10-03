#!/usr/bin/env python3
"""Check the workspace dependency direction recorded in ADR-0007.

Cargo resolves renamed, inherited, and target-specific manifest declarations for us.
Checking all dependency kinds keeps a test/build dependency from silently erasing a
crate boundary. This reads local metadata only; it neither builds nor uses a network.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ALLOWED = {
    "jev-cli": {"jev-client", "jev-config", "jev-core"},
    "jev-client": {"jev-core"},
    "jev-core": set(),
    "jev-config": set(),
}


def violations(metadata: dict) -> list[str]:
    """Return forbidden workspace edges, including dev/build and target edges."""
    members = set(metadata["workspace_members"])
    packages = [p for p in metadata["packages"] if p["id"] in members]
    names = {p["name"] for p in packages}
    problems = []
    if names != set(ALLOWED):
        problems.append(
            "workspace crates differ from ADR-0007: "
            f"expected {', '.join(sorted(ALLOWED))}; found {', '.join(sorted(names))}"
        )
    for package in packages:
        source = package["name"]
        for dependency in package["dependencies"]:
            destination = dependency["name"]
            if destination not in names or destination in ALLOWED.get(source, set()):
                continue
            kind = dependency["kind"] or "normal"
            target = dependency.get("target")
            scope = f"{kind}, {target}" if target else kind
            problems.append(f"forbidden workspace dependency: {source} -> {destination} ({scope})")
    return sorted(set(problems))


def main(root: Path = ROOT) -> int:
    try:
        result = subprocess.run(
            ["cargo", "metadata", "--no-deps", "--locked", "--offline", "--format-version", "1"],
            cwd=root,
            capture_output=True,
            text=True,
            check=False,
        )
    except OSError:
        print("cannot run cargo metadata; install the pinned Rust toolchain", file=sys.stderr)
        return 1
    if result.returncode:
        print(
            "cannot read offline workspace metadata; run cargo metadata --no-deps "
            "--locked --offline --format-version 1 to diagnose the manifest",
            file=sys.stderr,
        )
        return 1
    try:
        problems = violations(json.loads(result.stdout))
    except (ValueError, KeyError, TypeError):
        print("cargo returned invalid workspace metadata", file=sys.stderr)
        return 1
    for problem in problems:
        print(problem, file=sys.stderr)
    if problems:
        print("preserve ADR-0007's crate boundaries; a new boundary needs a reviewed decision", file=sys.stderr)
        return 1
    print("workspace architecture: four crates follow ADR-0007 dependency direction")
    return 0


if __name__ == "__main__":
    sys.exit(main())
