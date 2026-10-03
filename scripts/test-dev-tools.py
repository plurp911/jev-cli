#!/usr/bin/env python3
"""Behavior tests for environment diagnosis, hook setup, and map drift detection."""

from __future__ import annotations

import importlib.util
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent


def load(name: str):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


setup = load("dev-setup")
readiness = load("check-agent-readiness")


class Diagnosis(unittest.TestCase):
    def metadata(self, *args):
        if args[0] == "dist":
            output = "cargo-dist 0.32.0"
        elif args[:3] == ("rustup", "component", "list"):
            output = "rustfmt-host\nclippy-host\nrust-docs-host\nrust-src"
        elif args[0] == "rustup":
            pinned = setup.tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
            msrv = setup.tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["rust-version"]
            output = f"{pinned}-host\n{msrv}-host\nnightly-host"
        else:
            output = ".githooks"
        return subprocess.CompletedProcess(args, 0, output, "")

    def test_installed_nightly_alias_is_accepted(self):
        with patch.object(setup.shutil, "which", return_value="/tool"), patch.object(setup, "command", side_effect=self.metadata):
            checks = setup.diagnose()
        self.assertTrue(all(item["status"] == "ok" or not item["required"] for item in checks))

    def test_wrong_dist_version_is_not_ready(self):
        def metadata(*args):
            if args[0] == "dist":
                return subprocess.CompletedProcess(args, 0, "cargo-dist 0.31.0", "")
            return self.metadata(*args)

        with patch.object(setup.shutil, "which", return_value="/tool"), patch.object(setup, "command", side_effect=metadata):
            checks = setup.diagnose()
        row = next(item for item in checks if item["name"] == "pinned dist version")
        self.assertEqual("missing", row["status"])
        self.assertTrue(row["required"])
        self.assertIn("0.32.0", row["remedy"])

    def test_missing_pinned_components_are_not_ready(self):
        for missing in ["rustfmt", "clippy"]:
            def metadata(*args):
                result = self.metadata(*args)
                if args[:3] == ("rustup", "component", "list"):
                    result.stdout = "\n".join(line for line in result.stdout.splitlines() if not line.startswith(missing))
                return result

            with self.subTest(component=missing), patch.object(setup.shutil, "which", return_value="/tool"), patch.object(setup, "command", side_effect=metadata):
                checks = setup.diagnose()
            row = next(item for item in checks if item["name"] == f"pinned component {missing}")
            self.assertEqual("missing", row["status"])
            self.assertTrue(row["required"])

    def test_a_dated_nightly_does_not_satisfy_the_fuzz_alias(self):
        def metadata(*args):
            output = "nightly-2026-09-01-x86_64-unknown-linux-gnu" if args[0] == "rustup" else ".githooks"
            return subprocess.CompletedProcess(args, 0, output, "")

        with patch.object(setup.shutil, "which", return_value="/tool"), patch.object(setup, "command", side_effect=metadata):
            row = next(item for item in setup.diagnose() if item["name"] == "fuzz nightly toolchain")
        self.assertEqual("missing", row["status"])
        self.assertTrue(row["required"])
        self.assertEqual("rustup toolchain install nightly", row["remedy"])

        # The actual fuzz command must reject the same incomplete environment before
        # building or mutating a corpus, rather than letting cargo download a toolchain.
        with tempfile.TemporaryDirectory(prefix="jev-nightly-test-") as work:
            for name, body in [("cargo-fuzz", "exit 0"), ("rustup", "printf '%s\\n' nightly-2026-09-01-x86_64-unknown-linux-gnu")]:
                path = Path(work) / name
                path.write_text("#!/bin/sh\n" + body + "\n")
                path.chmod(0o700)
            result = subprocess.run([shutil.which("bash"), str(ROOT / "scripts/fuzz-smoke.sh")],
                                    env={"PATH": work + os.pathsep + os.environ.get("PATH", "")},
                                    capture_output=True, text=True, check=False)
        self.assertEqual(127, result.returncode)
        self.assertIn("rustup toolchain install nightly", result.stderr)

    def test_missing_fuzz_or_dist_prevents_push_readiness(self):
        pinned = setup.tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
        msrv = setup.tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["rust-version"]

        def metadata(*args):
            output = f"{pinned}-host\n{msrv}-host\nnightly-host" if args[0] == "rustup" else ".githooks"
            return subprocess.CompletedProcess(args, 0, output, "")

        for missing in ["cargo-fuzz", "dist"]:
            with self.subTest(tool=missing), patch.object(setup.shutil, "which", side_effect=lambda tool: None if tool == missing else "/tool"), patch.object(setup, "command", side_effect=metadata):
                checks = setup.diagnose()
            row = next(item for item in checks if item["name"] == missing)
            self.assertTrue(row["required"])
            self.assertEqual("missing", row["status"])
            self.assertFalse(all(item["status"] == "ok" or not item["required"] for item in checks))

    def test_missing_required_tool_is_actionable_without_execution(self):
        with patch.object(setup.shutil, "which", return_value=None), patch.object(setup, "command") as probe:
            checks = setup.diagnose()
        probe.assert_not_called()
        cargo = next(item for item in checks if item["name"] == "cargo")
        self.assertEqual("missing", cargo["status"])
        self.assertTrue(cargo["required"])
        self.assertIn("pinned", cargo["remedy"])

    def test_missing_schema_validator_never_reports_success(self):
        result = subprocess.run([sys.executable, "-S", str(ROOT / "scripts/check-request-schema.py")],
                                capture_output=True, text=True, check=False)
        self.assertEqual(77, result.returncode)
        self.assertIn("jsonschema is not installed", result.stdout)

    def test_doctor_json_matches_exit_status_and_exposes_no_environment_values(self):
        # An empty search path deliberately makes Git/Cargo unavailable. The doctor
        # must remain usable and report missing prerequisites without a traceback.
        result = subprocess.run([sys.executable, str(ROOT / "scripts/dev-setup.py"), "doctor", "--json"],
                                env={"PATH": "", "JEV_API_KEY": "doctor-credential-canary"},
                                capture_output=True, text=True, check=False)
        self.assertEqual(1, result.returncode)
        document = json.loads(result.stdout)
        self.assertFalse(document["ok"])
        self.assertNotIn("doctor-credential-canary", result.stdout + result.stderr)
        self.assertTrue(any(row["name"] == "cargo" and row["status"] == "missing" for row in document["checks"]))


class HookSetup(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory(prefix="jev-hook-test-")
        self.root = Path(self.work.name)
        self.environment = os.environ.copy()
        # A linked-worktree hook exports GIT_DIR/GIT_COMMON_DIR. Leaving them set
        # redirects fixture init/config into the real repository, even with cwd set.
        selectors = subprocess.run(["git", "rev-parse", "--local-env-vars"],
                                   capture_output=True, text=True, check=True).stdout.splitlines()
        for name in selectors:
            self.environment.pop(name, None)
        self.environment.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
        subprocess.run(["git", "init", "--quiet", str(self.root)], env=self.environment, check=True)
        (self.root / ".githooks").mkdir()

    def tearDown(self):
        self.work.cleanup()

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.root, env=self.environment,
                              capture_output=True, text=True, check=True).stdout.strip()

    def install(self):
        return subprocess.run(["sh", str(ROOT / "scripts/install-hooks.sh")], cwd=self.root,
                              env=self.environment, capture_output=True, text=True, check=False)

    def test_absolute_equivalent_hook_path_and_rerun_converge(self):
        self.git("config", "core.hooksPath", str(self.root / ".githooks"))
        self.assertEqual(0, self.install().returncode)
        self.assertEqual(0, self.install().returncode)
        self.assertEqual(".githooks", self.git("config", "--get", "core.hooksPath"))

    def test_other_hook_manager_is_preserved(self):
        self.git("config", "core.hooksPath", "other-hooks")
        result = self.install()
        self.assertEqual(1, result.returncode)
        self.assertIn("preserve", result.stderr)
        self.assertEqual("other-hooks", self.git("config", "--get", "core.hooksPath"))

    def test_equivalent_spellings_converge_without_false_foreign_manager_errors(self):
        for path in ["./.githooks", ".githooks/", str(self.root / ".githooks") + "/",
                     str(self.root / ".githooks" / ".." / ".githooks")]:
            with self.subTest(path=path):
                self.git("config", "core.hooksPath", path)
                self.assertEqual(0, self.install().returncode)
                self.assertEqual(".githooks", self.git("config", "--get", "core.hooksPath"))


class HookEnvironment(unittest.TestCase):
    def test_hook_fixtures_preserve_a_linked_worktrees_shared_git_configuration(self):
        with tempfile.TemporaryDirectory(prefix="jev-hook-environment-") as work:
            parent = Path(work) / "parent"
            linked = Path(work) / "linked"
            # This outer fixture starts without Git's repository-local selectors;
            # the child then receives the same selectors a worktree hook inherits.
            environment = os.environ.copy()
            selectors = subprocess.run(["git", "rev-parse", "--local-env-vars"],
                                       capture_output=True, text=True, check=True).stdout.splitlines()
            for name in selectors:
                environment.pop(name, None)
            environment.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")

            def git(*args, cwd=parent):
                return subprocess.run(["git", *args], cwd=cwd, env=environment,
                                      capture_output=True, text=True, check=True).stdout.strip()

            git("init", "--quiet", str(parent), cwd=work)
            git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                "commit", "--allow-empty", "--quiet", "-m", "fixture")
            git("worktree", "add", "--quiet", "--detach", str(linked))
            git("config", "core.hooksPath", ".githooks")
            hook_environment = {**environment,
                                "GIT_DIR": git("rev-parse", "--path-format=absolute", "--git-dir", cwd=linked),
                                "GIT_COMMON_DIR": git("rev-parse", "--path-format=absolute", "--git-common-dir", cwd=linked)}
            result = subprocess.run([sys.executable, str(Path(__file__).resolve()), "HookSetup"],
                                    cwd=linked, env=hook_environment,
                                    capture_output=True, text=True, check=False)
            self.assertEqual(0, result.returncode, result.stderr)
            self.assertEqual("false", git("config", "--local", "--get", "core.bare"))
            self.assertEqual(".githooks", git("config", "--local", "--get", "core.hooksPath"))
            self.assertEqual("", git("status", "--porcelain"))


class ReleaseCredentialProbe(unittest.TestCase):
    def probe(self, diagnostic):
        # Exercise the actual smoke block, replacing only the unpacked executable.
        source = (ROOT / "scripts/release-dry-run.sh").read_text()
        block = "# Run it the way a user would" + source.split("# Run it the way a user would", 1)[1].split("# The archive must also carry", 1)[0]
        with tempfile.TemporaryDirectory(prefix="jev-release-probe-") as work:
            binary = Path(work) / "jev"
            binary.write_text("#!/bin/sh\ncase \"$1\" in --version|doctor) exit 0;; esac\n"
                              + "printf '%s\\n' " + shlex.quote(diagnostic) + " >&2\nexit 3\n")
            binary.chmod(0o700)
            result = subprocess.run([shutil.which("bash"), "-c", "set -Eeuo pipefail\n" + block],
                                    env={**os.environ, "STAGE": work, "binary": str(binary)},
                                    capture_output=True, text=True, check=False)
        return result

    def test_local_refusal_is_accepted(self):
        result = self.probe("error: no TypeSafe API key found; disabled by JEV_NO_KEYCHAIN; use $JEV_API_KEY")
        self.assertEqual(0, result.returncode, result.stderr)

    def test_api_authentication_exit_is_not_a_local_refusal_or_echoed(self):
        canary = "release-output-canary"
        result = self.probe("HTTP 401: rejected JEV_API_KEY " + canary)
        self.assertEqual(1, result.returncode)
        self.assertNotIn(canary, result.stdout + result.stderr)

    def test_http_error_cannot_masquerade_as_missing_key_remediation(self):
        result = self.probe("no TypeSafe API key found; disabled by JEV_NO_KEYCHAIN; JEV_API_KEY: HTTP 401")
        self.assertEqual(1, result.returncode)


class MapDrift(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory(prefix="jev-map-test-")
        self.root = Path(self.work.name)
        text = (ROOT / readiness.MAP).read_text()
        paths = {readiness.MAP, "crates/jev-cli/src/cli.rs", "scripts/verify.sh"}
        for line in text.splitlines():
            if line.startswith("| `"):
                cells = [part.strip().strip("`") for part in line.strip("|").split("|")]
                paths.update(cells[1:3])
        paths.update(re.findall(r"<!-- readiness: ([^>]+) -->", text))
        paths.update(f".claude/skills/{name}/SKILL.md" for name in
                     ["verify", "security-review", "api-compat", "release-review", "typesafe-ai"])
        for path in paths:
            target = self.root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / path, target)

    def tearDown(self):
        self.work.cleanup()

    def test_current_map_is_clean(self):
        self.assertEqual([], readiness.check(self.root))

    def test_new_command_requires_a_proof_row(self):
        path = self.root / "crates/jev-cli/src/cli.rs"
        path.write_text(path.read_text().replace("pub enum Command {", "pub enum Command {\n    Unmapped,"))
        self.assertTrue(any("unmapped" in error for error in readiness.check(self.root)))

    def test_deleted_behavior_test_is_detected(self):
        path = self.root / "crates/jev-cli/tests/cli.rs"
        path.write_text(path.read_text().replace("fn doctor_makes_no_request_by_default(", "fn renamed_doctor_test("))
        self.assertTrue(any("missing behavior test doctor_makes_no_request_by_default" in error
                            for error in readiness.check(self.root)))

    def test_deleted_exemplar_is_detected(self):
        (self.root / "crates/jev-core/src/probability.rs").unlink()
        self.assertTrue(any("probability.rs" in error for error in readiness.check(self.root)))

    def test_comments_cannot_replace_required_gate_invocations(self):
        path = self.root / "scripts/verify.sh"
        original = path.read_text()
        for script in ["check-agent-readiness.py", "check-architecture.py", "test-check-architecture.py",
                       "test-dev-tools.py", "test-benchmark.py", "check-request-schema.py"]:
            with self.subTest(script=script):
                lines = ["# " + line if "scripts/" + script in line else line for line in original.splitlines()]
                path.write_text("\n".join(lines) + "\n")
                self.assertTrue(any("no longer runs " + script in error for error in readiness.check(self.root)))

    def test_exemplar_markers_cannot_point_outside_the_repository(self):
        path = self.root / readiness.MAP
        original = path.read_text()
        with tempfile.TemporaryDirectory(prefix="jev-external-exemplar-") as work:
            external = Path(work) / "exemplar.rs"
            external.write_text("// outside the checkout\n")
            for replacement in [str(external), os.path.relpath(external, self.root)]:
                with self.subTest(path=replacement):
                    path.write_text(original.replace("<!-- readiness: crates/jev-core/src/probability.rs -->",
                                                     "<!-- readiness: " + replacement + " -->"))
                    self.assertTrue(any("external readiness dependency" in error for error in readiness.check(self.root)))

    def test_helpers_ignored_and_disabled_functions_are_not_behavior_proofs(self):
        path = self.root / "crates/jev-cli/tests/cli.rs"
        original = path.read_text()
        name = "doctor_makes_no_request_by_default"
        needle = "#[test]\nfn " + name + "("
        self.assertIn(needle, original)
        for attributes in ["", "#[test]\n#[ignore]\n", "#[test]\n#[cfg(any())]\n"]:
            with self.subTest(attributes=attributes):
                path.write_text(original.replace(needle, attributes + "fn " + name + "("))
                self.assertTrue(any("missing behavior test " + name in error for error in readiness.check(self.root)))


if __name__ == "__main__":
    unittest.main(verbosity=2)
