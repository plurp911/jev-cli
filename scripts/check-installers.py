#!/usr/bin/env python3
"""Assert that a `dist build --artifacts=global` run produced installers that verify
what they download, and that nothing self-updating crept in.

This exists because the failure it catches is silent. `dist build --artifacts=global`
learns each archive's checksum from the `*-dist-manifest.json` files the local build
jobs leave in `target/distrib`. If those are missing — and the `.tar.xz.sha256` files
alone are *not* enough — `dist` still succeeds, but emits:

  * a shell installer whose `_checksum_value` is never assigned, so it takes the
    "no checksums to verify" branch and installs whatever it was served; and
  * a Homebrew formula with a `url` and no `sha256`, which `brew` installs unverified.

Both look completely normal. Run this against the output directory.

Usage: scripts/check-installers.py <dir> [--targets a,b,c]
"""

from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path

# `dist` 0.32.0 emits no checksum verification at all in the PowerShell installer: no
# `Get-FileHash`, no embedded digest. `powershell` is therefore not in `installers` in
# dist-workspace.toml, and finding a `.ps1` here means someone put it back -- which is a
# build failure, not a note, because it would be the one install path that verifies
# nothing. Revisit if a future `dist` adds verification.
POWERSHELL_VERIFIES = False


def declared_targets(root: Path) -> list[str]:
    config = tomllib.loads((root / "dist-workspace.toml").read_text(encoding="utf-8"))
    return list(config["dist"]["targets"])


def target_of(name: str) -> str | None:
    """The target triple embedded in an archive filename, if there is one."""
    match = re.search(r"-((?:x86_64|aarch64)-[a-z0-9._-]+?)\.(?:tar\.xz|zip)", name)
    return match.group(1) if match else None


def check_shell_installer(path: Path, required: set[str]) -> list[str]:
    """Every case arm for a required target must carry a sha256 digest.

    The shell installer selects an archive, then falls through a `case` whose arms set
    `_checksum_style` and `_checksum_value`. When `dist` has no manifest for a target,
    the arm simply omits both, and the download path prints "no checksums to verify"
    and installs anyway. So the check has to be per-arm: counting digests anywhere in
    the file would pass a build in which only one target was verified.
    """
    text = path.read_text(encoding="utf-8")
    problems = []

    arms = re.findall(
        r'"(jev-cli-[^"]+?\.(?:tar\.xz|zip))"\)(.*?)\n\s*;;', text, re.DOTALL
    )
    if not arms:
        return [f"{path.name}: no archive selection arms found; the format changed"]

    seen: set[str] = set()
    for archive, arm in arms:
        target = target_of(archive)
        if target is None:
            problems.append(f"{path.name}: cannot tell which target {archive} is for")
            continue
        seen.add(target)
        if target not in required:
            continue
        digest = re.search(r'_checksum_value="([0-9a-f]{64})"', arm)
        style = re.search(r'_checksum_style="(\w+)"', arm)
        if digest is None:
            problems.append(
                f"{path.name}: the {target} arm assigns no `_checksum_value`, so that "
                f"install takes the 'no checksums to verify' branch. The build job for "
                f"{target} almost certainly did not upload its `*-dist-manifest.json`."
            )
        elif style is None or style.group(1) != "sha256":
            problems.append(
                f"{path.name}: the {target} arm has a digest but checksum style "
                f"{style.group(1) if style else 'unset'!r}"
            )

    for target in sorted(required - seen):
        if "windows" in target:
            continue  # Windows is served by the PowerShell installer, not this one.
        problems.append(f"{path.name}: no arm at all for {target}")

    # `install-updater = false` in dist-workspace.toml, and AGENTS.md 3.3, forbid a
    # self-update mechanism. The installer always carries the *code path*; what must
    # not appear is an updater artifact for it to fetch.
    named = [n for n in re.findall(r'_updater_name="([^"]*)"', text) if n]
    if named:
        problems.append(f"{path.name}: references updater artifact(s) {named}")

    return problems


def check_formula(path: Path, required: set[str]) -> list[str]:
    """Each `url` in the formula must be followed by its `sha256`.

    `brew` will happily install from a `url` with no `sha256`; `dist` emits exactly
    that when the target's manifest is missing. Pairing them positionally is what
    catches a formula that verifies three of its four platforms.
    """
    problems = []
    pending: str | None = None
    paired: dict[str, bool] = {}

    for line in path.read_text(encoding="utf-8").splitlines():
        url = re.match(r'\s*url "([^"]+)"', line)
        if url:
            if pending is not None:
                paired[pending] = False
            pending = url.group(1)
            continue
        if pending is not None and re.match(r'\s*sha256 "[0-9a-f]{64}"', line):
            paired[pending] = True
            pending = None
    if pending is not None:
        paired[pending] = False

    if not paired:
        return [f"{path.name}: no download url at all"]

    for url, verified in paired.items():
        target = target_of(url.rsplit("/", 1)[-1])
        if target is not None and target not in required:
            continue
        if not verified:
            problems.append(
                f"{path.name}: `url {url}` has no `sha256`; `brew install` would "
                f"fetch it unverified"
            )
    return problems


def main() -> int:
    arguments = sys.argv[1:]
    only: set[str] | None = None
    if "--only" in arguments:
        index = arguments.index("--only")
        only = {t for t in arguments[index + 1].split(",") if t}
        del arguments[index : index + 2]
    if len(arguments) != 1:
        print(__doc__, file=sys.stderr)
        return 2

    directory = Path(arguments[0])
    root = Path(__file__).resolve().parent.parent
    declared = set(declared_targets(root))
    required = declared if only is None else only

    unknown = required - declared
    if unknown:
        print(
            f"::error::--only names target(s) not in dist-workspace.toml: "
            f"{sorted(unknown)}",
            file=sys.stderr,
        )
        return 2

    problems: list[str] = []
    checked = 0

    for installer in sorted(directory.glob("*installer.sh")):
        problems += check_shell_installer(installer, required)
        checked += 1
    for formula in sorted(directory.glob("*.rb")):
        problems += check_formula(formula, required)
        checked += 1

    for path in sorted(directory.glob("*installer.ps1")):
        if not POWERSHELL_VERIFIES:
            problems.append(
                f"{path.name}: `dist` 0.32.0's PowerShell installer performs no "
                f"checksum verification whatsoever. `powershell` was removed from "
                f"`installers` in dist-workspace.toml for that reason; if it is back, "
                f"either revert that or confirm this `dist` version verifies and set "
                f"POWERSHELL_VERIFIES."
            )
        checked += 1

    if checked == 0:
        print(f"::error::no installer or formula found in {directory}", file=sys.stderr)
        return 1

    for problem in problems:
        print(f"::error::{problem}", file=sys.stderr)
    if problems:
        return 1

    print(
        f"{checked} global artifact(s) verify what they download "
        f"({len(required)} target(s) required)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
