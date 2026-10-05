#!/usr/bin/env python3
"""Offline snapshot-integrity tests; synthetic tiny files, no model downloads."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("clef-model-manifest.py")


class ManifestTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.module = None
        if SCRIPT.is_file():
            spec = importlib.util.spec_from_file_location("clef_manifest", SCRIPT)
            cls.module = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(cls.module)

    def setUp(self):
        self.assertIsNotNone(self.module, "offline model verifier is missing")
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.model = self.root / "model"
        self.model.mkdir()
        (self.model / "config.json").write_bytes(b"{}")
        (self.model / "model.safetensors").write_bytes(b"original synthetic weights")
        self.manifest = {"schema":"jev.clef.model-manifest/v1", "repository":"Cloudflare/clef-flash",
            "revision":"1" * 40, "files":[
                {"path":"config.json","size":2,"algorithm":"git-blob-sha1",
                 "digest":hashlib.sha1(b"blob 2\0{}").hexdigest()},
                {"path":"model.safetensors","size":26,"algorithm":"sha256",
                 "digest":hashlib.sha256(b"original synthetic weights").hexdigest()}]}

    def test_matching_pinned_files_are_verified_without_loading_code(self):
        result = self.module.verify(self.model, self.manifest)
        self.assertEqual(result["files_verified"], 2)
        self.assertEqual(result["revision"], "1" * 40)

    def test_tampering_missing_files_and_untracked_code_fail(self):
        (self.model / "config.json").write_bytes(b"[]")
        with self.assertRaises(self.module.Failure):
            self.module.verify(self.model, self.manifest)
        (self.model / "config.json").write_bytes(b"{}")
        (self.model / "model.safetensors").unlink()
        with self.assertRaises(self.module.Failure):
            self.module.verify(self.model, self.manifest)
        (self.model / "model.safetensors").write_bytes(b"original synthetic weights")
        (self.model / "extra.py").write_bytes(b"raise Exception()")
        with self.assertRaises(self.module.Failure):
            self.module.verify(self.model, self.manifest)

    def test_manifest_paths_cannot_escape_the_named_model_directory(self):
        self.manifest["files"][0]["path"] = "../config.json"
        with self.assertRaises(self.module.Failure):
            self.module.verify(self.model, self.manifest)

    def test_unpinned_revisions_and_symlinks_are_refused(self):
        self.manifest["revision"] = "main"
        with self.assertRaises(self.module.Failure):
            self.module.verify(self.model, self.manifest)
        self.manifest["revision"] = "1" * 40
        target = self.root / "outside.json"
        target.write_bytes(b"{}")
        (self.model / "config.json").unlink()
        try:
            (self.model / "config.json").symlink_to(target)
        except OSError as error:
            self.skipTest("host does not permit creating symlinks: " + str(error.errno))
        with self.assertRaises(self.module.Failure):
            self.module.verify(self.model, self.manifest)


if __name__ == "__main__":
    unittest.main()
