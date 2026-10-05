#!/usr/bin/env python3
"""Capture or compare an explicitly selected local Clef Python runtime, offline.

Records interpreter and mapped system-library hashes alongside package versions.
This verifies an existing runtime; it does not provision Python or the operating
system, run publisher code, load weights, or make network requests.
"""

from __future__ import annotations

import argparse
import hashlib
from importlib import metadata
import json
import os
from pathlib import Path
import platform
import re
import sys
import sysconfig

MAX_PROFILE = 256 * 1024
MAX_LIBRARY = 128 * 1024 * 1024
MAX_DEPTH = 64


def file_digest(path):
    digest, total = hashlib.sha256(), 0
    with path.open("rb") as stream:
        while chunk := stream.read(1024 * 1024):
            total += len(chunk)
            if total > MAX_LIBRARY:
                raise ValueError("runtime file exceeds its size bound")
            digest.update(chunk)
    return {"filename":path.name, "sha256":digest.hexdigest(), "size_bytes":total}


def capture_profile():
    packages = {}
    for distribution in metadata.distributions():
        name, version = distribution.metadata["Name"], distribution.version
        if not isinstance(name, str) or not re.fullmatch(r"[A-Za-z0-9_.-]{1,128}", name) or not re.fullmatch(r"[A-Za-z0-9_.+!-]{1,128}", version):
            raise ValueError("invalid installed-package metadata")
        name = re.sub(r"[-_.]+", "-", name).lower()
        if name in packages or len(packages) >= 512:
            raise ValueError("duplicate or excessive installed packages")
        packages[name] = version
    # Hash the executable and libraries mapped by this verifier's process. Output
    # retains only basenames, never home paths or the command line/environment.
    raw = b""
    if platform.system() == "Linux":
        with Path("/proc/self/maps").open("rb") as stream:
            raw = stream.read(MAX_PROFILE + 1)
    if len(raw) > MAX_PROFILE:
        raise ValueError("runtime map exceeds its size bound")
    libraries = set()
    for line in raw.decode("utf-8").splitlines():
        fields = line.split(maxsplit=5)
        if len(fields) == 6 and fields[5].startswith("/"):
            path = Path(fields[5])
            if ".so" in path.name:
                libraries.add(path)
    return {"schema":"jev.clef.python-runtime-profile/v1",
            "python_version":platform.python_version(), "implementation":platform.python_implementation(),
            "soabi":sysconfig.get_config_var("SOABI"), "os":platform.system(),
            "architecture":platform.machine(), "libc":list(platform.libc_ver()),
            "interpreter":file_digest(Path(sys.executable).resolve()),
            "system_libraries":sorted((file_digest(path) for path in libraries), key=lambda item: (item["filename"],item["sha256"])),
            "packages":dict(sorted(packages.items()))}


def compare_profiles(actual, expected):
    if not isinstance(expected, dict) or expected.get("schema") != "jev.clef.python-runtime-profile/v1" or actual != expected:
        raise ValueError("Python runtime differs from the recorded CPU profile")


def read_profile(path):
    with path.open("rb") as stream:
        raw = stream.read(MAX_PROFILE + 1)
    if len(raw) > MAX_PROFILE:
        raise ValueError("runtime profile exceeds its size bound")
    text = raw.decode("utf-8")
    depth, quoted, escaped = 0, False, False
    for character in text:
        if quoted:
            if escaped:
                escaped = False
            elif character == "\\":
                escaped = True
            elif character == '"':
                quoted = False
        elif character == '"':
            quoted = True
        elif character in "{[":
            depth += 1
            if depth > MAX_DEPTH:
                raise ValueError("runtime profile exceeds its nesting bound")
        elif character in "}]":
            depth -= 1
    def unique_fields(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate runtime profile field")
            result[key] = value
        return result
    try:
        return json.loads(text, object_pairs_hook=unique_fields)
    except RecursionError:
        raise ValueError("runtime profile exceeds its nesting bound") from None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expected", type=Path, help="compare with an explicitly named recorded profile")
    parser.add_argument("--report", type=Path, help="create a new sanitized profile receipt")
    args = parser.parse_args()
    try:
        profile = capture_profile()
        if args.expected:
            compare_profiles(profile, read_profile(args.expected))
        encoded = json.dumps(profile, sort_keys=True, indent=2) + "\n"
        if args.report:
            descriptor = os.open(args.report, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
                stream.write(encoded)
        print(encoded, end="")
    except (OSError, ValueError, TypeError, KeyError, UnicodeError):
        print("Python profile check failed: use the recorded Linux CPU runtime and packages.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
