#!/usr/bin/env python3
"""Snapshot explicitly Git-tracked working files for a local release rehearsal.

Capture bounded tracked-file bytes once and retain their original Git provenance.
An optional isolated, read-only Git tree compiles those same bytes independently of
later edits to the original worktree. Build tools are trusted: read-only permissions
protect ordinary edits, not deliberate rewriting by the same owner. An uncommitted
rehearsal remains a local test, not a publishable release.
"""

from __future__ import annotations

import argparse
from contextlib import ExitStack, contextmanager
import errno
import gzip
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess
import sys
import tarfile
import tomllib

MAX_FILE = 50 * 1024 * 1024
MAX_SOURCE = 500 * 1024 * 1024
MAX_ENTRIES = 20001  # At most 20,000 source files and one generated receipt.
DIR_FD_OPEN = os.open in os.supports_dir_fd and hasattr(os, "O_NOFOLLOW") and hasattr(os, "O_DIRECTORY")


def git_environment():
    # Honor the explicitly named repository, with no user hooks, templates, or
    # external filters inherited into an isolated rehearsal checkout.
    environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    environment.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_SYSTEM=os.devnull,
                       GIT_CONFIG_NOSYSTEM="1", GIT_TEMPLATE_DIR="", GIT_OPTIONAL_LOCKS="0")
    return environment


def git(root, *args, input=None, env=None):
    return subprocess.run(["git", "-c", "core.hooksPath=" + os.devnull,
                           "-c", "core.fsmonitor=false", "-c", "commit.gpgsign=false", *args],
                          cwd=root, check=True, capture_output=True, input=input,
                          env=git_environment() if env is None else env).stdout


def path_metadata(root, relative):
    """Fallback identity checks for platforms without directory-relative opens."""
    checked = []
    path = root
    for part in relative.parts:
        path = path / part
        metadata = path.lstat()
        # Windows junctions and other reparse points can redirect parent traversal.
        reparse = getattr(metadata, "st_file_attributes", 0) & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0)
        if stat.S_ISLNK(metadata.st_mode) or reparse:
            raise ValueError("tracked source symlink refused")
        checked.append((metadata.st_dev, metadata.st_ino, metadata.st_mode))
    return checked


@contextmanager
def source_stream(root, relative, root_descriptor):
    """Pin each path component before opening a tracked regular file."""
    flags = os.O_RDONLY | getattr(os, "O_NONBLOCK", 0) | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_BINARY", 0)
    try:
        with ExitStack() as directories:
            if root_descriptor is not None:
                current = root_descriptor
                for part in relative.parts[:-1]:
                    current = os.open(part, flags | os.O_DIRECTORY, dir_fd=current)
                    directories.callback(os.close, current)
                descriptor = os.open(relative.name, flags, dir_fd=current)
            else:
                before = path_metadata(root, relative)
                if not stat.S_ISREG(before[-1][2]):
                    raise ValueError("unsupported tracked source file")
                descriptor = os.open(root / relative, flags)
            try:
                stream = os.fdopen(descriptor, "rb")
            except Exception:
                os.close(descriptor)
                raise
            with stream:
                if root_descriptor is None:
                    opened = os.fstat(stream.fileno())
                    if before != path_metadata(root, relative) or before[-1][:2] != (opened.st_dev, opened.st_ino):
                        raise ValueError("tracked source path changed before reading")
                yield stream
    except OSError as error:
        if error.errno in (errno.ELOOP, errno.ENOTDIR):
            raise ValueError("unsupported tracked source path or symlink refused") from None
        raise


def file_identity(metadata):
    return metadata.st_size, metadata.st_mode, metadata.st_mtime_ns, metadata.st_ctime_ns


def source_files(root):
    names = sorted(set(os.fsdecode(name) for name in git(root, "ls-files", "-z").split(b"\0") if name))
    if len(names) > MAX_ENTRIES:
        raise ValueError("source snapshot exceeds its entry bound")
    files = {}
    total = 0
    with ExitStack() as descriptors:
        root_descriptor = None
        if DIR_FD_OPEN:
            root_descriptor = os.open(root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
            descriptors.callback(os.close, root_descriptor)
        for name in names:
            relative = PurePosixPath(name)
            native = Path(name)
            if relative.is_absolute() or native.drive or native.is_absolute() or not relative.parts or ".." in relative.parts or ".." in native.parts or any(part.casefold() == ".git" for part in relative.parts):
                raise ValueError("unsafe tracked source path")
            with source_stream(root, relative, root_descriptor) as stream:
                metadata = os.fstat(stream.fileno())
                if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > MAX_FILE:
                    raise ValueError(f"unsupported or oversized source: {name!r}")
                if metadata.st_size > MAX_SOURCE - total:
                    raise ValueError("source snapshot exceeds its size bound")
                # Bound the allocation independently of concurrent file growth. One
                # extra byte distinguishes a growing file from the expected content.
                data = stream.read(metadata.st_size + 1)
                after = os.fstat(stream.fileno())
                if len(data) != metadata.st_size or file_identity(metadata) != file_identity(after):
                    raise ValueError(f"source changed while being read: {name!r}")
            total += len(data)
            files[name] = (data, 0o755 if metadata.st_mode & 0o111 else 0o644)
    return files


def checksums(files):
    return {name: {"sha256": hashlib.sha256(data).hexdigest(), "mode": mode}
            for name, (data, mode) in files.items()}


def snapshot(root, output, build_tree=None):
    if build_tree is not None and (build_tree.exists() or build_tree.is_symlink()):
        raise ValueError("source build tree destination already exists")
    files = source_files(root)
    if ".jev-source.json" in files:
        raise ValueError("tracked source uses the reserved receipt name .jev-source.json")
    version = tomllib.loads(files["Cargo.toml"][0].decode())["workspace"]["package"]["version"]
    if not isinstance(version, str) or not re.fullmatch(r"[A-Za-z0-9_.+-]+", version):
        raise ValueError("invalid source archive version")
    prefix = f"jev-cli-{version}"
    epoch = int(git(root, "show", "-s", "--format=%ct", "HEAD"))
    receipt = {"schema": "jev.source-rehearsal/v1", "prefix": prefix,
               "base_revision": git(root, "rev-parse", "HEAD").decode().strip(),
               "working_tree_changes": bool(git(root, "status", "--porcelain", "--untracked-files=no")),
               "files": checksums(files)}
    files[".jev-source.json"] = ((json.dumps(receipt, sort_keys=True, indent=2) + "\n").encode(), 0o644)
    # The reader's limits cover every entry, including generated provenance.
    # Reject here before emitting an archive that its own checker cannot accept.
    if len(files) > MAX_ENTRIES or any(len(data) > MAX_FILE for data, _ in files.values()) or sum(len(data) for data, _ in files.values()) > MAX_SOURCE:
        raise ValueError("source snapshot exceeds its archive bounds")
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_name(output.name + ".tmp")
    try:
        with temporary.open("wb") as stream, gzip.GzipFile(filename="", mode="wb", fileobj=stream, mtime=epoch) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                for name, (data, mode) in sorted(files.items()):
                    entry = tarfile.TarInfo(f"{prefix}/{name}")
                    entry.size, entry.mode, entry.mtime = len(data), mode, epoch
                    archive.addfile(entry, io.BytesIO(data))
        os.replace(temporary, output)
    finally:
        temporary.unlink(missing_ok=True)
    if build_tree is not None:
        materialize(root, build_tree, files, epoch)


def materialize(root, tree, files, epoch):
    """Build only the captured bytes; the original worktree is never committed."""
    tree.parent.mkdir(parents=True, exist_ok=True)
    git(root, "-c", "init.templateDir=", "clone", "--no-hardlinks", "--no-checkout", "--local", "--", str(root), str(tree))
    index = []
    for name, (data, mode) in sorted(files.items()):
        path = tree / name
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("xb") as stream:
            stream.write(data)
        path.chmod(mode)
        # Git's normal add/checkout applies attributes and filters. Index the exact
        # snapshot bytes directly so CRLF normalization cannot change release source.
        digest = git(tree, "hash-object", "-w", "--no-filters", "--stdin", input=data).strip()
        index.append(f"{0o100000 | mode:o}".encode() + b" " + digest + b"\t" + os.fsencode(name) + b"\0")
    git(tree, "read-tree", "--empty")
    git(tree, "update-index", "-z", "--index-info", input=b"".join(index))
    environment = git_environment()
    environment.update(GIT_AUTHOR_DATE=f"{epoch} +0000", GIT_COMMITTER_DATE=f"{epoch} +0000")
    git(tree, "-c", "user.name=jev source rehearsal", "-c", "user.email=source-rehearsal@example.invalid",
        "commit", "-qm", "chore: isolate local source rehearsal", env=environment)
    # Protect against ordinary edits. Build tools run as the owner and are trusted
    # not to deliberately change permissions and rewrite captured source.
    directories = {tree}
    for name, (_, mode) in files.items():
        path = tree / name
        path.chmod(mode & ~0o222)
        directories.update(path.parents)
    for directory in directories:
        if directory == tree or tree in directory.parents:
            directory.chmod(0o555)


def check(root, path, *, build_tree=False):
    current = source_files(root)
    generated_receipt = None
    if build_tree:
        generated_receipt = current.pop(".jev-source.json", None)
        if generated_receipt is None:
            raise ValueError("isolated build tree is missing its generated source receipt")
    elif ".jev-source.json" in current:
        raise ValueError("tracked source uses the reserved receipt name .jev-source.json")
    expected = checksums(current)
    actual = {}
    receipt = None
    total = 0
    entries = 0
    with tarfile.open(path) as archive:
        for entry in archive:
            parts = PurePosixPath(entry.name).parts
            if len(parts) < 2 or not entry.isfile() or entry.size > MAX_FILE:
                raise ValueError("unsupported source archive entry")
            total += entry.size
            entries += 1
            if total > MAX_SOURCE or entries > MAX_ENTRIES:
                raise ValueError("source archive exceeds its size bound")
            name = PurePosixPath(*parts[1:]).as_posix()
            stream = archive.extractfile(entry)
            if stream is None:
                raise ValueError("missing source archive content")
            data = stream.read(MAX_FILE + 1)
            if len(data) != entry.size:
                raise ValueError("truncated source archive content")
            if name == ".jev-source.json":
                if receipt is not None:
                    raise ValueError("duplicate source receipt")
                if build_tree and generated_receipt != (data, entry.mode):
                    raise ValueError("isolated build tree source receipt changed during rehearsal")
                receipt = json.loads(data)
                continue
            if name in actual:
                raise ValueError("duplicate source archive entry")
            actual[name] = {"sha256": hashlib.sha256(data).hexdigest(), "mode": entry.mode}
    if receipt is None or receipt.get("schema") != "jev.source-rehearsal/v1" or receipt.get("files") != actual:
        raise ValueError("source receipt does not match the archive")
    changed = sorted(name for name in set(expected) | set(actual) if expected.get(name) != actual.get(name))
    if changed:
        raise ValueError("source changed during release rehearsal: " + repr(changed[:8]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument("--output", type=Path)
    action.add_argument("--check", type=Path)
    parser.add_argument("--build-tree", type=Path, help="new isolated read-only Git tree containing the captured source; requires --output")
    parser.add_argument("--check-build-tree", action="store_true", help="development-only: validate an isolated build tree and its generated receipt; requires --check")
    args = parser.parse_args()
    if args.build_tree is not None and args.output is None:
        parser.error("--build-tree requires --output")
    if args.check_build_tree and args.check is None:
        parser.error("--check-build-tree requires --check")
    try:
        if args.output:
            snapshot(args.root.resolve(), args.output, args.build_tree.absolute() if args.build_tree else None)
        else:
            check(args.root.resolve(), args.check, build_tree=args.check_build_tree)
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError, tarfile.TarError):
        # The exception text can contain input from tracked files; only our validation
        # errors are suitable as diagnostics, never a subprocess's captured output.
        error = sys.exception()
        message = str(error) if type(error) is ValueError else "cannot create or validate source snapshot"
        print(message, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
