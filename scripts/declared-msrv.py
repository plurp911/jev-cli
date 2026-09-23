#!/usr/bin/env python3
"""Print the workspace's declared minimum supported Rust version.

Local verification reads the MSRV from the manifests rather than hardcoding it, so the
push gate cannot drift from `rust-version` in `Cargo.toml`. Every workspace member
inherits the same value; disagreement is itself a bug, so this exits non-zero rather
than guessing.
"""

from __future__ import annotations

import json
import subprocess
import sys


def main() -> int:
    metadata = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            capture_output=True,
            check=True,
            text=True,
        ).stdout
    )

    declared = {package["name"]: package.get("rust_version") for package in metadata["packages"]}

    missing = sorted(name for name, version in declared.items() if version is None)
    if missing:
        print(f"packages do not declare rust-version: {missing}", file=sys.stderr)
        return 1

    versions = set(declared.values())
    if len(versions) != 1:
        print(f"workspace members declare inconsistent rust-version: {declared}", file=sys.stderr)
        return 1

    print(versions.pop())
    return 0


if __name__ == "__main__":
    sys.exit(main())
