#!/usr/bin/env python3
"""Canonical local setup and read-only developer environment diagnosis."""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

try:
    import tomllib
except ImportError:
    raise SystemExit("developer setup requires Python 3.11 or newer") from None

ROOT = Path(__file__).resolve().parent.parent
REQUIRED_TOOLS = {
    "git": "install Git",
    "rustup": "install rustup from https://rustup.rs",
    "cargo": "install the toolchain pinned in rust-toolchain.toml",
    "cargo-deny": "from /tmp: cargo install cargo-deny --locked",
    "typos": "from /tmp: cargo install typos-cli --locked",
    "shellcheck": "install shellcheck using your system package manager",
    "zizmor": "uv tool install zizmor",
    "cargo-fuzz": "from /tmp: cargo install cargo-fuzz --locked; rustup toolchain install nightly",
    "dist": "install cargo-dist at the version in dist-workspace.toml from /tmp",
}
OPTIONAL_TOOLS = {
    "cargo-nextest": "from /tmp: cargo install cargo-nextest --locked (cargo test is the fallback)",
    "cargo-sbom": "from /tmp: cargo install cargo-sbom --locked",
    "claude": "optional client for shipped skill manifest validation and paid evals",
    "npx": "optional MCP Inspector client; see docs/mcp.md",
}


def command(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(args, cwd=ROOT, capture_output=True, text=True, timeout=30)


def diagnose() -> list[dict]:
    """Read tool metadata only; never read credentials or launch the product."""
    checks = []

    def add(name: str, ok: bool, required: bool, hint: str) -> None:
        checks.append({"name": name, "status": "ok" if ok else "missing",
                       "required": required, "remedy": "" if ok else hint})

    add("Python 3.11+", sys.version_info >= (3, 11), True, "install Python 3.11 or newer")
    for tool, hint in REQUIRED_TOOLS.items():
        add(tool, shutil.which(tool) is not None, True, hint)
    for tool, hint in OPTIONAL_TOOLS.items():
        add(tool, shutil.which(tool) is not None, False, hint)
    add("Python jsonschema", importlib.util.find_spec("jsonschema") is not None,
        True, "install jsonschema for this Python interpreter: python3 -m pip install jsonschema")
    if shutil.which("dist"):
        version = tomllib.loads((ROOT / "dist-workspace.toml").read_text(encoding="utf-8"))["dist"]["cargo-dist-version"]
        result = command("dist", "--version")
        add("pinned dist version", result.returncode == 0 and result.stdout.strip() == f"cargo-dist {version}",
            True, f"from /tmp: cargo install cargo-dist --version {version} --locked")
    if shutil.which("rustup"):
        result = command("rustup", "toolchain", "list")
        installed = result.stdout.splitlines() if result.returncode == 0 else []
        toolchain = tomllib.loads((ROOT / "rust-toolchain.toml").read_text(encoding="utf-8"))["toolchain"]
        pinned = toolchain["channel"]
        msrv = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]["package"]["rust-version"]
        pinned_installed = False
        for name, version in [("pinned toolchain", pinned), ("MSRV toolchain", msrv)]:
            present = any(line.startswith(version + "-") or line.startswith(version + " ") for line in installed)
            add(name, present, True, f"rustup toolchain install {version}")
            if name == "pinned toolchain":
                pinned_installed = present
        # Do not probe an absent toolchain: rustup may otherwise download it, violating
        # doctor's read-only contract. Installation alone does not prove fmt/clippy work.
        components = []
        if pinned_installed:
            result = command("rustup", "component", "list", "--toolchain", pinned, "--installed")
            if result.returncode == 0:
                components = result.stdout.splitlines()
        for component in toolchain.get("components", []):
            present = any(line == component or line.startswith(component + "-") for line in components)
            add(f"pinned component {component}", present, True,
                f"rustup component add --toolchain {pinned} {component}")
        add("fuzz nightly toolchain", any(re.match(r"^nightly(?:-[A-Za-z]|\s|$)", line) for line in installed),
            True, "rustup toolchain install nightly")
    if shutil.which("git"):
        hook = command("git", "config", "--local", "--get", "core.hooksPath").stdout.strip()
        path = Path(hook) if hook else Path(".git/hooks")
        if not path.is_absolute():
            path = ROOT / path
        add("tracked pre-push hook", path.resolve() == (ROOT / ".githooks").resolve(), True,
            "run scripts/install-hooks.sh; preserve a different hook manager if one is configured")
    return checks


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["doctor", "bootstrap"])
    parser.add_argument("--json", action="store_true", help="doctor output as JSON")
    args = parser.parse_args()
    if args.action == "bootstrap":
        if args.json:
            parser.error("--json applies only to doctor")
        # Tool installation remains explicit; this installs only the clone-local hook
        # and builds the existing locked workspace. It never reads a .env file.
        for argv in [["sh", "scripts/install-hooks.sh"],
                     ["cargo", "build", "--workspace", "--locked"]]:
            try:
                result = subprocess.run(argv, cwd=ROOT, check=False)
            except OSError:
                print(f"cannot run {argv[0]}; install Git and the pinned Rust toolchain, then rerun bootstrap", file=sys.stderr)
                return 1
            if result.returncode:
                return result.returncode
    try:
        checks = diagnose()
    except (OSError, subprocess.TimeoutExpired, KeyError, tomllib.TOMLDecodeError):
        print("cannot inspect developer environment; check Git, rustup, and repository manifests", file=sys.stderr)
        return 1
    ok = all(item["status"] == "ok" or not item["required"] for item in checks)
    if args.json:
        print(json.dumps({"schema_version": 1, "ok": ok, "checks": checks}, indent=2))
    else:
        for item in checks:
            label = "ok" if item["status"] == "ok" else "missing" if item["required"] else "optional"
            print(f"{label:8} {item['name']}" + (f" — {item['remedy']}" if item["remedy"] else ""))
        print("Next: scripts/verify.sh. Doctor diagnoses prerequisites; it does not verify the product.")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
