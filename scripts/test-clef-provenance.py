#!/usr/bin/env python3
"""Offline provenance boundaries: actual bytes, explicit files, no model loading."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("clef_provenance.py")


class ProvenanceTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue(SCRIPT.is_file(), "automatic execution provenance is missing")
        spec = importlib.util.spec_from_file_location("clef_provenance_test", SCRIPT)
        self.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.module)
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.cli = self.root / "cli"
        self.cli.write_bytes(b"abc")
        self.cli.chmod(0o700)
        self.config = {"jev": self.cli, "provider": "ollama"}

    def session(self):
        return self.module.Session(self.config, execution_observed=True)

    def manifest(self, artifacts):
        path = self.root / "manifest.json"
        path.write_text(json.dumps({"schema": "jev.clef.provenance-inputs/v1", "artifacts": artifacts}))
        self.config["provenance_manifest"] = path
        return path

    def test_cli_and_both_helpers_have_actual_sha256_and_unknown_provider(self):
        session = self.session()
        receipt = session.finish()
        self.assertTrue(session.ready)
        cli = receipt["artifacts"][0]
        self.assertEqual(cli["before"]["sha256"], "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        self.assertEqual(cli["status"], "unchanged")
        helpers = {item["role"]: item for item in receipt["artifacts"]}
        for role, name in (("smoke-helper", "clef-live.py"), ("quality-helper", "clef-quality.py"),
                           ("provenance-helper", "clef_provenance.py")):
            self.assertEqual(helpers[role]["before"]["sha256"], hashlib.sha256(SCRIPT.with_name(name).read_bytes()).hexdigest())
        self.assertEqual(receipt["model_artifacts"], "unobserved")
        self.assertEqual(receipt["provider_runtime"], "unobserved")
        self.assertFalse(receipt["attests_exact_execution"])

    def test_byte_change_with_same_size_and_restored_mtime_is_detected(self):
        session = self.session()
        before = self.cli.stat()
        self.cli.write_bytes(b"xyz")
        os.utime(self.cli, ns=(before.st_atime_ns, before.st_mtime_ns))
        receipt = session.finish()
        self.assertEqual(receipt["artifacts"][0]["status"], "changed")
        self.assertFalse(session.stable)

    def test_missing_after_capture_is_not_stable(self):
        session = self.session()
        self.cli.unlink()
        self.assertEqual(session.finish()["artifacts"][0]["status"], "missing")
        self.assertFalse(session.stable)

    def test_explicit_model_and_runtime_libraries_verified_without_paths(self):
        entries = []
        for role in ("model", "runtime", "runtime-library"):
            path = self.root / (role + "-private-canary")
            path.write_bytes(b"abc")
            entries.append({"role": role, "path": str(path), "size": 3,
                            "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"})
        self.manifest(entries)
        session = self.session()
        receipt = session.finish()
        self.assertTrue(session.ready)
        self.assertEqual(receipt["model_artifacts"], "caller_supplied")
        self.assertEqual(receipt["provider_runtime"], "caller_supplied")
        self.assertEqual(receipt["runtime_inventory_completeness"], "unverified")
        self.assertNotIn("private-canary", json.dumps(receipt))
        self.assertNotIn(str(self.root), json.dumps(receipt))

    def test_invalid_named_inputs_refuse_atomically(self):
        directory = self.root / "directory"
        directory.mkdir()
        symlink = self.root / "symlink"
        symlink.symlink_to(self.cli)
        linked_dir = self.root / "linked-dir"
        linked_dir.symlink_to(self.root, target_is_directory=True)
        invalid = [
            {"role": "model", "path": str(directory)},
            {"role": "model", "path": str(symlink)},
            {"role": "model", "path": str(linked_dir / "cli")},
            {"role": "model", "path": str(self.root / "missing")},
            {"role": "model", "path": str(self.cli), "sha256": "bad"},
            {"role": "model", "path": str(self.cli), "sha256": "0" * 64},
            {"role": "model", "path": str(self.cli), "size": True},
            {"role": "credentials", "path": str(self.cli)},
            {"role": "model", "path": str(self.cli), "extra": "private-canary"},
        ]
        for entry in invalid:
            with self.subTest(entry=entry):
                self.manifest([entry])
                session = self.session()
                receipt = session.finish()
                self.assertFalse(session.ready)
                self.assertEqual(receipt["status"], "refused")
                self.assertNotIn(str(self.root), json.dumps(receipt))

    def test_limits_deep_json_duplicate_paths_and_invalid_utf8_refuse(self):
        path = self.manifest([{"role": "model", "path": str(self.cli)}] * 2)
        self.assertFalse(self.session().ready)
        path.write_bytes(b"\xff")
        self.assertFalse(self.session().ready)
        path.write_text("[" * 100 + "]" * 100)
        self.assertFalse(self.session().ready)
        path.write_bytes(b"x" * (self.module.MAX_MANIFEST_BYTES + 1))
        self.assertFalse(self.session().ready)
        self.manifest([{"role": "model", "path": str(self.cli)}])
        with patch.object(self.module, "MAX_FILE_BYTES", 2):
            self.assertFalse(self.session().ready)
        with patch.object(self.module, "MAX_TOTAL_BYTES", 2):
            self.assertFalse(self.session().ready)

    def test_path_alias_resolves_once_to_actual_argv_file(self):
        alias = self.root / "alias"
        alias.symlink_to(self.cli)
        self.config["jev"] = "alias"
        with patch.dict(os.environ, {"PATH": str(self.root)}):
            session = self.session()
        self.assertEqual(session.executable, str(self.cli))
        self.assertTrue(session.ready)

    def test_missing_cli_real_execution_refused_injected_boundary_unobserved(self):
        self.config["jev"] = str(self.root / "missing")
        self.assertFalse(self.session().ready)
        session = self.module.Session(self.config, execution_observed=False)
        self.assertTrue(session.ready)
        self.assertEqual(session.finish()["execution_boundary"], "injected_runner_unobserved")

    def test_explicit_auth_file_cannot_be_hashed_as_manifest_or_artifact(self):
        auth = self.root / "auth-canary"
        auth.write_bytes(b"private-canary")
        self.config["key_file"] = auth
        original = self.module.os.open
        def guarded(path, *args, **kwargs):
            self.assertNotEqual(Path(path), auth, "provenance must never open the auth input")
            return original(path, *args, **kwargs)
        with patch.object(self.module.os, "open", side_effect=guarded):
            self.config["provenance_manifest"] = auth
            self.assertFalse(self.session().ready)
            self.manifest([{"role": "runtime", "path": str(auth)}])
            session = self.session()
            self.assertFalse(session.ready)
            self.assertNotIn("private-canary", json.dumps(session.finish()))

    def test_model_library_and_manifest_changes_invalidate_stability(self):
        library = self.root / "library"
        library.write_bytes(b"abc")
        manifest = self.manifest([{"role": "runtime-library", "path": str(library)}])
        session = self.session()
        library.write_bytes(b"xyz")
        manifest.write_text("{}")
        receipt = session.finish()
        statuses = {item["role"]: item["status"] for item in receipt["artifacts"]}
        self.assertEqual(statuses["runtime-library"], "changed")
        self.assertEqual(statuses["input-manifest"], "changed")
        self.assertFalse(session.stable)

    def test_fifo_and_too_many_files_refuse_without_opening_special_file(self):
        if hasattr(os, "mkfifo"):
            path = self.root / "fifo"
            os.mkfifo(path)
            self.manifest([{"role": "model", "path": str(path)}])
            self.assertFalse(self.session().ready)
        self.manifest([{"role": "model", "path": str(self.cli)}] * (self.module.MAX_FILES + 1))
        self.assertFalse(self.session().ready)

    def test_metadata_change_during_hash_cannot_be_reported_as_a_fingerprint(self):
        original = self.module.identity
        seen = 0
        def changing(info):
            nonlocal seen
            seen += 1
            if seen == 3:
                self.cli.write_bytes(b"xyz")
            return original(info)
        with patch.object(self.module, "identity", side_effect=changing):
            before, _, _ = self.module.snapshot(self.cli, [100])
        self.assertEqual(before, {"status": "refused"})

    def test_hardlinked_auth_file_never_opens_even_as_cli(self):
        auth = self.root / "auth"
        auth.write_bytes(b"private-canary")
        alias = self.root / "auth-alias"
        os.link(auth, alias)
        self.config.update(key_file=auth, jev=alias)
        original = self.module.os.open
        def guarded(path, *args, **kwargs):
            self.assertNotEqual(str(path), str(alias), "hardlinked auth alias must not open")
            self.assertNotEqual(str(path), alias.name, "hardlinked auth alias must not open")
            return original(path, *args, **kwargs)
        with patch.object(self.module.os, "open", side_effect=guarded):
            session = self.session()
            self.assertFalse(session.ready)
            self.assertNotIn("private-canary", json.dumps(session.finish()))

    def test_parent_symlink_swap_during_open_never_hashes_other_bytes(self):
        tree = self.root / "tree"
        tree.mkdir()
        file = tree / "model"
        file.write_bytes(b"abc")
        other = self.root / "other"
        other.mkdir()
        (other / "model").write_bytes(b"private-canary")
        original = self.module.os.open
        swapped = False
        def swap(path, *args, **kwargs):
            nonlocal swapped
            if not swapped and str(path) in (str(file), "tree"):
                swapped = True
                tree.rename(self.root / "original-tree")
                tree.symlink_to(other, target_is_directory=True)
            return original(path, *args, **kwargs)
        sha256 = self.module.hashlib.sha256
        test = self
        class GuardedDigest:
            def __init__(self):
                self.digest = sha256()
            def update(self, raw):
                test.assertNotIn(b"private-canary", raw, "swapped directory bytes must never enter hashing")
                self.digest.update(raw)
            def hexdigest(self):
                return self.digest.hexdigest()
        with patch.object(self.module.os, "open", side_effect=swap), \
                patch.object(self.module.hashlib, "sha256", GuardedDigest):
            before, _, _ = self.module.snapshot(file, [100])
        self.assertTrue(swapped)
        self.assertEqual(before, {"status": "refused"})

    def test_helper_source_change_and_permission_change_are_reported(self):
        clone = self.root / "helpers"
        clone.mkdir()
        for name in ("clef-live.py", "clef-quality.py", "clef_provenance.py"):
            (clone / name).write_bytes(SCRIPT.with_name(name).read_bytes())
        spec = importlib.util.spec_from_file_location("cloned_provenance", clone / SCRIPT.name)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        session = module.Session(self.config, execution_observed=True)
        (clone / "clef-live.py").write_bytes(b"different helper source")
        self.cli.chmod(0o600)
        receipt = session.finish()
        statuses = {item["role"]: item["status"] for item in receipt["artifacts"]}
        self.assertEqual(statuses["smoke-helper"], "changed")
        self.assertEqual(statuses["cli"], "changed")
        self.assertFalse(session.stable)

    def test_missing_no_follow_primitive_fails_closed(self):
        if not hasattr(self.module.os, "O_NOFOLLOW"):
            self.skipTest("no-follow primitive already unavailable")
        with patch.object(self.module.os, "O_NOFOLLOW", None):
            value, _, _ = self.module.snapshot(self.cli, [100])
        self.assertEqual(value, {"status": "refused"})

    def test_inherited_credential_file_alias_and_rotation_never_open(self):
        auth = self.root / "auth"
        auth.write_bytes(b"private-canary")
        alias = self.root / "auth-alias"
        os.link(auth, alias)
        self.manifest([{"role": "model", "path": str(alias)}])
        with patch.dict(os.environ, {"JEV_API_KEY_FILE": str(auth)}):
            self.assertFalse(self.session().ready)
        library = self.root / "library"
        library.write_bytes(b"abc")
        self.manifest([{"role": "runtime-library", "path": str(library)}])
        with patch.dict(os.environ, {"JEV_CUSTOM_API_KEY_FILE": str(auth)}):
            session = self.session()
            auth.unlink()
            library.unlink()
            auth.write_bytes(b"rotated-private-canary")
            os.link(auth, library)
            original = self.module.open_named
            def guarded(path):
                self.assertNotEqual(path, library, "rotated credential inode must not open")
                return original(path)
            with patch.object(self.module, "open_named", side_effect=guarded):
                receipt = session.finish()
        self.assertEqual(receipt["artifacts"][-1]["status"], "refused")
        self.assertFalse(session.stable)
        self.assertNotIn("private-canary", json.dumps(receipt))


if __name__ == "__main__":
    unittest.main()
