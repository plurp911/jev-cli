#!/usr/bin/env python3
"""Offline checks for the explicitly selected Clef Python runtime profile."""

import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("clef-python-profile.py")


class ProfileTests(unittest.TestCase):
    def module(self):
        self.assertTrue(SCRIPT.is_file(), "explicit runtime profile verifier is missing")
        spec = importlib.util.spec_from_file_location("clef_python_profile", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def profile(self):
        return {"schema":"jev.clef.python-runtime-profile/v1", "python_version":"3.12.3",
            "implementation":"CPython", "soabi":"cpython-312-x86_64-linux-gnu",
            "os":"Linux", "architecture":"x86_64", "libc":["glibc","2.39"],
            "interpreter":{"filename":"python3.12", "sha256":"a"*64,"size_bytes":10},
            "system_libraries":[{"filename":"libc.so.6","sha256":"b"*64,"size_bytes":20}],
            "packages":{"torch":"2.11.0+cpu"}}

    def test_exact_runtime_and_package_profile_is_accepted(self):
        module = self.module()
        module.compare_profiles(self.profile(), self.profile())

    def test_interpreter_platform_library_and_package_drift_are_refused(self):
        module = self.module()
        for field, replacement in [("python_version","3.12.4"),("os","Windows"),
                ("architecture","aarch64"),("libc",["glibc","2.40"]),
                ("interpreter",{"filename":"python3.12","sha256":"c"*64,"size_bytes":10}),
                ("system_libraries",[]),("packages",{"torch":"2.11.0"})]:
            with self.subTest(field=field):
                changed = copy.deepcopy(self.profile())
                changed[field] = replacement
                with self.assertRaises(ValueError):
                    module.compare_profiles(changed, self.profile())

    def test_extra_or_missing_packages_are_refused(self):
        module = self.module()
        for packages in [{}, {"torch":"2.11.0+cpu","unexpected":"1.0"}]:
            changed = self.profile()
            changed["packages"] = packages
            with self.assertRaises(ValueError):
                module.compare_profiles(changed, self.profile())

    def test_actual_profile_contains_hashes_and_no_absolute_paths(self):
        module = self.module()
        # Package metadata belongs to the explicitly selected venv, not the host
        # running this deterministic test. Keep real interpreter/library hashing.
        with patch.object(module.metadata, "distributions", return_value=[]):
            profile = module.capture_profile()
        self.assertEqual(profile["python_version"], ".".join(map(str, sys.version_info[:3])))
        self.assertRegex(profile["interpreter"]["sha256"], "^[a-f0-9]{64}$")
        self.assertNotIn(str(Path.home()), json.dumps(profile))
        self.assertTrue(all("/" not in library["filename"] for library in profile["system_libraries"]))

    def test_cli_rejects_drift_without_creating_a_passing_report(self):
        self.module()
        with tempfile.TemporaryDirectory(prefix="jev-python-profile-") as temporary:
            root = Path(temporary)
            expected, report = root / "expected.json", root / "report.json"
            # -S isolates this fixture from the host's unrelated distributions.
            baseline = subprocess.run([sys.executable,"-S","-B",str(SCRIPT)],capture_output=True,text=True)
            self.assertEqual(baseline.returncode, 0, baseline.stderr)
            profile = json.loads(baseline.stdout)
            profile["python_version"] = "0.0.0"
            expected.write_text(json.dumps(profile))
            result = subprocess.run([sys.executable,"-S","-B",str(SCRIPT),"--expected",str(expected),"--report",str(report)],capture_output=True,text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(report.exists())
            self.assertNotIn(str(root), result.stderr)

    def test_duplicate_fields_in_an_expected_profile_are_refused(self):
        module = self.module()
        with tempfile.TemporaryDirectory(prefix="jev-python-profile-") as temporary:
            path = Path(temporary) / "profile.json"
            path.write_text('{"python_version":"3.12.3","python_version":"3.12.3"}')
            with self.assertRaises(ValueError):
                module.read_profile(path)

    def test_runtime_files_and_expected_profiles_are_bounded(self):
        module = self.module()
        with tempfile.TemporaryDirectory(prefix="jev-python-profile-") as temporary:
            path = Path(temporary) / "bounded-file"
            path.write_bytes(b"x" * 64)
            with patch.object(module, "MAX_LIBRARY", 8):
                with self.assertRaises(ValueError):
                    module.file_digest(path)
            with patch.object(module, "MAX_PROFILE", 8):
                with self.assertRaises(ValueError):
                    module.read_profile(path)

    def test_excessively_nested_profile_is_an_opaque_validation_error(self):
        module = self.module()
        with tempfile.TemporaryDirectory(prefix="jev-python-profile-") as temporary:
            path = Path(temporary) / "nested.json"
            path.write_text("[" * 2000 + "0" + "]" * 2000)
            with self.assertRaises(ValueError):
                module.read_profile(path)

    def test_cli_never_overwrites_an_existing_receipt(self):
        self.module()
        with tempfile.TemporaryDirectory(prefix="jev-python-profile-") as temporary:
            report = Path(temporary) / "report.json"
            report.write_bytes(b"previous verification receipt")
            result = subprocess.run([sys.executable,"-S","-B",str(SCRIPT),"--report",str(report)],capture_output=True,text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(report.read_bytes(), b"previous verification receipt")
            self.assertNotIn(str(report), result.stderr)


if __name__ == "__main__":
    unittest.main()
