#!/usr/bin/env python3
"""Assert that the manual release workflow agrees with its source configuration.

A workflow that restates a policy defined elsewhere is a drift trap: both copies look
right in isolation, and the disagreement only shows up at release time. This catches:

  * An unpinned `tool: cargo-dist` in a workflow. `taiki-e/install-action` would then
    install whatever is newest, so what a release contains could change without a
    commit — which contradicts `dist-workspace.toml`'s own comment and ADR-0009.
  * A pin that drifts from `cargo-dist-version` in `dist-workspace.toml`. `dist`
    refuses to run when the two disagree, but it only finds out at release time.

  * A packaging template in `packaging/` that is no longer a template. These carry
    `PLACEHOLDER_*` tokens a human replaces *after* verifying a published artifact; a
    manifest committed with a real version or digest in it either ships a stale hash or
    ships a literal "PLACEHOLDER" to users.

Every tool named in a `tool:` input must carry an explicit `@version`.
"""

from __future__ import annotations

import re
import stat
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = ROOT / ".github" / "workflows"


def check_packaging_templates() -> list[str]:
    """Packaging manifests in this repository must still be templates.

    They are filled in by a human after verifying a published artifact's checksum and
    attestation (`packaging/README.md`). Both halves of that going wrong are silent: a
    half-filled manifest ships a stale digest, and an unnoticed `PLACEHOLDER_VERSION`
    reaches a package index as a literal string.
    """
    packaging = ROOT / "packaging"
    if not packaging.is_dir():
        return ["packaging/ is missing"]

    problems = []
    for path in sorted(packaging.rglob("*")):
        if not path.is_file() or path.name == "README.md":
            continue
        text = path.read_text(encoding="utf-8")
        where = path.relative_to(ROOT)

        placeholders = set(re.findall(r"PLACEHOLDER_[A-Z0-9_]+", text))
        if not placeholders:
            problems.append(
                f"{where}: a packaging manifest with no PLACEHOLDER token. Either it "
                f"was filled in and committed, or it is missing the fields a release "
                f"has to substitute."
            )
            continue

        # A real digest sitting next to an unfilled placeholder is the half-filled case.
        for digest in re.findall(r"\b[0-9a-f]{64}\b", text):
            problems.append(
                f"{where}: contains a literal sha256 ({digest[:12]}…) alongside "
                f"{sorted(placeholders)}; a partially filled template is worse than "
                f"either state"
            )
        if re.search(r"^\s*(?:version|PackageVersion):\s*[\"\']?\d", text, re.M):
            problems.append(f"{where}: carries a concrete version, not a placeholder")

    return problems


def main() -> int:
    declared = tomllib.loads(
        (ROOT / "dist-workspace.toml").read_text(encoding="utf-8")
    )["dist"]["cargo-dist-version"]

    problems: list[str] = []
    pinned = 0

    active = {
        path.name
        for path in WORKFLOWS.iterdir()
        if path.suffix in {".yml", ".yaml"}
    }
    if active != {"release.yml"}:
        problems.append(
            f"only the manual release workflow may be active; found {sorted(active)}"
        )
    release_text = (WORKFLOWS / "release.yml").read_text(encoding="utf-8")
    trigger_block = re.search(r"(?m)^on:\n((?:^[ \t].*\n|^\n)*)", release_text)
    triggers = (
        re.findall(r"(?m)^  ([\w-]+):", trigger_block.group(1))
        if trigger_block
        else []
    )
    if triggers != ["workflow_dispatch"]:
        problems.append(f"release.yml must be manual-only; found triggers {triggers}")

    hook = ROOT / ".githooks" / "pre-push"
    if not hook.is_file() or not hook.stat().st_mode & stat.S_IXUSR:
        problems.append("the tracked pre-push hook must exist and be executable")

    for workflow in sorted(WORKFLOWS.glob("*.yml")):
        text = workflow.read_text(encoding="utf-8")
        for line_number, line in enumerate(text.splitlines(), start=1):
            match = re.match(r"\s*tool:\s*(\S.*?)\s*$", line)
            if not match:
                continue
            for tool in (t.strip() for t in match.group(1).split(",")):
                where = f"{workflow.relative_to(ROOT)}:{line_number}"
                if "@" not in tool:
                    problems.append(
                        f"{where}: `{tool}` is not pinned. Add `@<version>`, or a "
                        f"newer release of it can change this build without a commit."
                    )
                    continue
                pinned += 1
                name, version = tool.split("@", 1)
                if name == "cargo-dist" and version != declared:
                    problems.append(
                        f"{where}: pinned to cargo-dist@{version}, but "
                        f"dist-workspace.toml declares cargo-dist-version = "
                        f'"{declared}". `dist` refuses to run when these disagree.'
                    )

    problems += check_packaging_templates()

    for problem in problems:
        print(f"::error::{problem}", file=sys.stderr)
    if problems:
        return 1

    print(f"{pinned} workflow tool pin(s), all explicit and in step")
    print("only the manual release workflow is active")
    print("the tracked pre-push hook is executable")
    print("every packaging manifest is still a template")
    return 0


if __name__ == "__main__":
    sys.exit(main())
