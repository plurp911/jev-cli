"""Bounded, offline fingerprints of explicitly implicated or named execution files.

Disk snapshots qualify receipts; they do not attest loaded code, server association,
or a complete runtime inventory. No model imports, traversal, or credential reads.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import sys

sys.dont_write_bytecode = True
MAX_MANIFEST_BYTES = 1024 * 1024
MAX_FILES = 512
MAX_FILE_BYTES = 64 * 1024 ** 3
MAX_TOTAL_BYTES = 128 * 1024 ** 3
MAX_JSON_DEPTH = 16
CHUNK_BYTES = 1024 * 1024


class Refused(ValueError):
    """Opaque failure: never interpolate caller paths or underlying errors."""


def require(condition):
    if not condition:
        raise Refused("check the explicitly named provenance manifest and files")


def named_path(value, base=None):
    require(type(value) is str and 0 < len(value) <= 4096
            and not any(ord(char) < 32 for char in value))
    path = Path(value)
    if not path.is_absolute():
        path = (base or Path.cwd()) / path
    require(".." not in path.parts)
    path = Path(os.path.abspath(path))
    # Optional inputs never follow links, including directory components.
    require(all(not parent.is_symlink() for parent in (path, *path.parents)))
    return path


def identity(info):
    return (info.st_dev, info.st_ino, info.st_mode, info.st_size, info.st_mtime_ns, info.st_ctime_ns)


def open_parent(path):
    """Return the named parent's descriptor, without following directory links.

    Fail closed where no-follow descriptor-relative opens are unavailable. Checking
    symlinks by pathname alone leaves parent replacement races during a long hash.
    """
    require(os.name == "posix" and hasattr(os, "O_NOFOLLOW") and hasattr(os, "O_DIRECTORY"))
    flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_DIRECTORY
    directory = os.open(path.anchor, flags)
    try:
        for part in path.parts[1:-1]:
            child = os.open(part, flags, dir_fd=directory)
            os.close(directory)
            directory = child
        return directory
    except BaseException:
        os.close(directory)
        raise


def open_named(path):
    directory = open_parent(path)
    try:
        return os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | getattr(os, "O_NONBLOCK", 0),
                       dir_fd=directory)
    finally:
        os.close(directory)


def snapshot(path, budget, limit=MAX_FILE_BYTES, return_bytes=False, blocked_ids=()):
    """Stream bounded regular files; reject changed open/path identities and links."""
    try:
        path = named_path(str(path))
        info = path.lstat()
        require(stat.S_ISREG(info.st_mode) and info.st_size <= min(limit, MAX_FILE_BYTES))
        require((info.st_dev, info.st_ino) not in blocked_ids)
        require(info.st_size <= budget[0])
        descriptor = open_named(path)
        with os.fdopen(descriptor, "rb") as stream:
            opened = os.fstat(stream.fileno())
            require(stat.S_ISREG(opened.st_mode) and identity(info) == identity(opened))
            require((opened.st_dev, opened.st_ino) not in blocked_ids)
            digest, count, content = hashlib.sha256(), 0, bytearray()
            while True:
                raw = stream.read(min(CHUNK_BYTES, limit - count + 1, budget[0] - count + 1))
                if not raw:
                    break
                count += len(raw)
                require(count <= min(limit, MAX_FILE_BYTES, budget[0]))
                digest.update(raw)
                if return_bytes:
                    content.extend(raw)
            require(count == opened.st_size and identity(opened) == identity(os.fstat(stream.fileno()))
                    and identity(opened) == identity(path.lstat()))
        budget[0] -= count
        value = {"sha256": digest.hexdigest(), "size": count}
        return value, identity(opened), bytes(content) if return_bytes else None
    except FileNotFoundError:
        return {"status": "missing"}, None, None
    except (OSError, ValueError, OverflowError, NotImplementedError, TypeError):
        return {"status": "refused"}, None, None


def manifest_entries(raw, base):
    # Bound nesting before JSON decoding can recurse or allocate nested containers.
    depth, quoted, escaped = 0, False, False
    text = raw.decode("utf-8")
    for char in text:
        if quoted:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                quoted = False
        elif char == '"':
            quoted = True
        elif char in "[{":
            depth += 1
            require(depth <= MAX_JSON_DEPTH)
        elif char in "]}":
            depth -= 1
    def unique(pairs):
        value = {}
        for key, item in pairs:
            require(key not in value)
            value[key] = item
        return value
    document = json.loads(text, object_pairs_hook=unique)
    require(type(document) is dict and set(document) == {"schema", "artifacts"}
            and document["schema"] == "jev.clef.provenance-inputs/v1")
    entries = document["artifacts"]
    require(type(entries) is list and 1 <= len(entries) <= MAX_FILES)
    paths, result = set(), []
    for item in entries:
        require(type(item) is dict and {"role", "path"} <= set(item)
                and set(item) <= {"role", "path", "sha256", "size"})
        require(item["role"] in ("model", "runtime", "runtime-library"))
        path = named_path(item["path"], base)
        require(path not in paths)
        paths.add(path)
        if "sha256" in item:
            require(type(item["sha256"]) is str and re.fullmatch(r"[a-f0-9]{64}", item["sha256"]))
        if "size" in item:
            require(type(item["size"]) is int and 0 <= item["size"] <= MAX_FILE_BYTES)
        result.append((path, item))
    return result


class Session:
    """Capture once before calls; finish snapshots again after every outcome."""

    def __init__(self, config, execution_observed):
        self.execution_observed = execution_observed
        self.ready, self.stable, self.refused = True, False, False
        selected = str(config["jev"])
        try:
            require(0 < len(selected) <= 4096 and not any(ord(char) < 32 for char in selected))
            found = shutil.which(selected)
            self.executable = str(Path(found or selected).resolve())
            cli_path = Path(self.executable)
        except (OSError, ValueError, RuntimeError):
            self.executable, cli_path = "", None
            self.ready, self.refused = False, True
        self.records = []
        self.blocked_ids, self.blocked_paths = set(), set()
        # Only inspect known credential *path* settings, never key values or file
        # contents. The child clears these inherited settings, but an accidental
        # artifact alias must not turn provenance hashing into credential reading.
        candidates = [config.get("key_file"), os.environ.get("JEV_API_KEY_FILE"),
                      os.environ.get("JEV_CUSTOM_API_KEY_FILE")]
        self.secret_paths = []
        for value in candidates:
            if value and len(str(value)) <= 4096:
                self.secret_paths.append(Path(os.path.abspath(str(value))))
        self.refresh_protected()
        budget = [MAX_TOTAL_BYTES]
        directory = Path(__file__).resolve().parent
        automatic = [("cli", cli_path), ("smoke-helper", directory / "clef-live.py"),
                     ("quality-helper", directory / "clef-quality.py"),
                     ("provenance-helper", Path(__file__).resolve()),
                     ("helper-interpreter", Path(sys.executable).resolve())]
        for role, path in automatic:
            self.capture(role, path, budget)
        if execution_observed and "sha256" not in self.records[0]["before"]:
            self.ready = False
        if any("sha256" not in item["before"] for item in self.records[1:]):
            self.ready = False
        self.supplied = False
        if config.get("provenance_manifest"):
            try:
                path = named_path(str(config["provenance_manifest"]))
                require(path not in self.blocked_paths)
                raw = self.capture("input-manifest", path, budget, manifest=True)
                require(raw is not None)
                entries = manifest_entries(raw, path.parent)
                require(all(path not in self.blocked_paths for path, _ in entries))
                for path, item in entries:
                    self.capture(item["role"], path, budget, expected=item)
                self.supplied = True
            except (OSError, ValueError, TypeError, KeyError, RecursionError):
                self.ready, self.refused = False, True

    def capture(self, role, path, budget, expected=None, manifest=False):
        self.refresh_protected()
        blocked = path is None or path in self.blocked_paths
        before, token, raw = (({"status": "refused"}, None, None) if blocked else
                              snapshot(path, budget, MAX_MANIFEST_BYTES if manifest else MAX_FILE_BYTES,
                                       manifest, self.blocked_ids))
        record = {"role": role, "index": len(self.records), "before": before,
                  "path": path, "token": token, "blocked": blocked}
        self.records.append(record)
        if expected is not None:
            require("sha256" in before)
            require(all(before[key] == expected[key] for key in ("sha256", "size") if key in expected))
            record["expected_digest_verified"] = "sha256" in expected
        return raw

    def refresh_protected(self):
        """Stat only known secret-manager paths; retain old and rotated identities."""
        for path in self.secret_paths:
            self.blocked_paths.add(path)
            try:
                resolved = path.resolve()
                self.blocked_paths.add(resolved)
                info = resolved.stat()
                self.blocked_ids.add((info.st_dev, info.st_ino))
            except (OSError, ValueError, RuntimeError):
                pass

    def finish(self):
        artifacts, budget = [], [MAX_TOTAL_BYTES]
        for record in self.records:
            self.refresh_protected()
            blocked = record["blocked"] or record["path"] in self.blocked_paths
            after, token, _ = (({"status": "refused"}, None, None) if blocked else
                               snapshot(record["path"], budget,
                                        MAX_MANIFEST_BYTES if record["role"] == "input-manifest" else MAX_FILE_BYTES,
                                        blocked_ids=self.blocked_ids))
            before = record["before"]
            status = (after.get("status") or before.get("status") or
                      ("unchanged" if after == before and token == record["token"] else "changed"))
            item = {"role": record["role"], "index": record["index"],
                    "before": before, "after": after, "status": status}
            if "expected_digest_verified" in record:
                item["expected_digest_verified"] = record["expected_digest_verified"]
            artifacts.append(item)
        self.stable = all(item["status"] == "unchanged" for item in artifacts)
        roles = {item["role"] for item in artifacts}
        return {"schema": "jev.clef.execution-provenance/v1",
                "status": "refused" if self.refused or not self.ready else "stable" if self.stable else "incomplete_or_changed",
                "execution_boundary": "resolved_subprocess_target" if self.execution_observed else "injected_runner_unobserved",
                "artifacts": artifacts, "attests_exact_execution": False,
                "model_artifacts": "caller_supplied" if self.supplied and "model" in roles else "unobserved",
                "provider_runtime": "caller_supplied" if self.supplied and roles & {"runtime", "runtime-library"} else "unobserved",
                "runtime_inventory_completeness": "unverified",
                "platform_policy": "File capture requires POSIX no-follow descriptor-relative opens; unavailable primitives fail closed.",
                "limits": {"manifest_bytes": MAX_MANIFEST_BYTES, "named_files": MAX_FILES,
                           "file_bytes": MAX_FILE_BYTES, "aggregate_bytes_per_capture": MAX_TOTAL_BYTES},
                "limitations": [
                    "Before/after disk fingerprints do not attest in-memory helper code or exact bytes executed between snapshots.",
                    "CLI scripts or launchers do not fingerprint their interpreters or dynamically loaded libraries.",
                    "The helper interpreter fingerprint does not inventory Python packages or native libraries.",
                    "Caller-selected model/runtime files are not proof the serving process loaded them; association and inventory completeness are unverified.",
                    "Remote provider weights and serving runtime are unobserved; model identifiers are not fingerprints."]}
