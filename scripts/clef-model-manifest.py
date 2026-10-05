#!/usr/bin/env python3
"""Verify an explicitly named local snapshot against recorded publisher file hashes.

Offline only: does not download files, import publisher code, or initialize a model.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import sys

sys.dont_write_bytecode = True


class Failure(ValueError):
    """An invalid snapshot or manifest."""


def require(condition):
    if not condition:
        raise Failure("snapshot does not match the pinned publisher manifest")


def verify(directory, manifest):
    directory = Path(directory)
    require(directory.is_dir() and not directory.is_symlink())
    require(manifest.get("schema") == "jev.clef.model-manifest/v1"
            and manifest.get("repository") in ("Cloudflare/clef", "Cloudflare/clef-flash")
            and re.fullmatch(r"[a-f0-9]{40}", manifest.get("revision", "")))
    files = manifest.get("files")
    require(type(files) is list and files)
    expected, verified, total = set(), [], 0
    for entry in files:
        require(type(entry) is dict and type(entry.get("path")) is str)
        relative = PurePosixPath(entry["path"])
        require(not relative.is_absolute() and relative.parts and ".." not in relative.parts
                and "\\" not in entry["path"] and ":" not in entry["path"]
                and entry["path"] not in expected)
        expected.add(entry["path"])
        file = directory.joinpath(*relative.parts)
        require(all(not directory.joinpath(*relative.parts[:index]).is_symlink()
                    for index in range(1, len(relative.parts) + 1)))
        require(file.is_file() and type(entry.get("size")) is int and entry["size"] >= 0
                and file.stat().st_size == entry["size"])
        algorithm = entry.get("algorithm")
        require(algorithm in ("sha256", "git-blob-sha1"))
        require(type(entry.get("digest")) is str and re.fullmatch(
            r"[a-f0-9]{64}" if algorithm == "sha256" else r"[a-f0-9]{40}", entry["digest"]))
        hasher = hashlib.sha256() if algorithm == "sha256" else hashlib.sha1()
        if algorithm == "git-blob-sha1":
            hasher.update(("blob " + str(entry["size"]) + "\0").encode("ascii"))
        with file.open("rb") as stream:
            while chunk := stream.read(1024 * 1024):
                hasher.update(chunk)
        require(hasher.hexdigest() == entry["digest"])
        total += entry["size"]
        verified.append(entry["path"])
    for file in directory.rglob("*"):
        # hf --local-dir keeps download bookkeeping here. It is not model input.
        relative = file.relative_to(directory)
        if relative.parts[:2] == (".cache", "huggingface"):
            continue
        require(not file.is_symlink())
        if file.is_file():
            require(relative.as_posix() in expected)
    return {"schema": "jev.clef.model-verification/v1", "repository": manifest["repository"],
            "revision": manifest["revision"], "passed": True, "files_verified": len(verified),
            "bytes_verified": total}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model-path", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--report", type=Path, help="write a new explicitly named verification receipt")
    args = parser.parse_args()
    try:
        if args.report:
            require(not args.report.exists() and args.report.parent.is_dir())
        manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
        result = verify(args.model_path, manifest)
        if args.report:
            with args.report.open("x", encoding="utf-8") as stream:
                json.dump(result, stream, indent=2)
                stream.write("\n")
        print(json.dumps(result))
        return 0
    except (Failure, OSError, ValueError, TypeError, KeyError):
        print("Snapshot verification failed: use the exact pinned revision and manifest.", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
