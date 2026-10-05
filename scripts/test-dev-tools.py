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


class MediaVerification(unittest.TestCase):
    def helper(self,source,name):
        start=source.index(name+'() {')
        end=source.index('\n}\n',start)+3
        return source[start:end]

    def test_missing_and_partial_processor_imports_skip_normally_but_fail_push(self):
        source=(ROOT/'scripts/verify.sh').read_text()
        helpers=self.helper(source,'run')+'\n'+self.helper(source,'optional_module')
        for state in ['missing','partial']:
            for mode in ['full','push']:
                with self.subTest(state=state,mode=mode),tempfile.TemporaryDirectory() as name:
                    python=Path(name)/'python3'
                    # A partial install would pass find_spec('transformers') but
                    # fails the actual processor import that the real gate uses.
                    python.write_text('#!/bin/sh\n'+('exit 1\n' if state=='missing' else
                        'case "$2" in *"from transformers import Qwen3VLVideoProcessor"*) exit 1;; *) exit 0;; esac\n'))
                    python.chmod(0o700)
                    script='set -Eeuo pipefail\nMODE='+mode+'\nFAILED=();SKIPPED=();PASSED=();BOLD="";OFF="";RED=""\n'+helpers+"\noptional_module processor transformers install-hint true\nprintf '%s,%s,%s\\n' \"${#FAILED[@]}\" \"${#SKIPPED[@]}\" \"${#PASSED[@]}\"\n"
                    result=subprocess.run([shutil.which('bash'),'-c',script],
                        env={'PATH':name},capture_output=True,text=True,check=False)
                    self.assertEqual(result.returncode,0,result.stderr)
                    self.assertEqual(result.stdout.strip(),'1,0,0' if mode=='push' else '0,1,0')

    def test_explicit_media_interpreter_failures_are_not_optionalized(self):
        source=(ROOT/'scripts/verify.sh').read_text()
        start=source.index('  if [ -n "${JEV_CLEF_PYTHON:-}" ]; then')
        block=source[start:source.index('\n  fi',start)+5]
        with tempfile.TemporaryDirectory() as name:
            log=Path(name)/'args';interpreter=Path(name)/'python'
            interpreter.write_text("#!/bin/sh\nprintf '%s\\n' \"$*\" >> "+shlex.quote(str(log))+'\nexit 1\n')
            interpreter.chmod(0o700)
            script='set -Eeuo pipefail\nMODE=push\nFAILED=();SKIPPED=();PASSED=();BOLD="";OFF="";RED=""\n'+self.helper(source,'run')+'\n'+block+"\nprintf '%s,%s,%s\\n' \"${#FAILED[@]}\" \"${#SKIPPED[@]}\" \"${#PASSED[@]}\"\n"
            result=subprocess.run([shutil.which('bash'),'-c',script],
                env={'JEV_CLEF_PYTHON':str(interpreter),'PATH':os.environ.get('PATH','')},capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertEqual(result.stdout.splitlines()[-1],'2,0,0')
            calls=log.read_text().splitlines()
            self.assertEqual(calls,['scripts/test-clef-server.py --real-pillow',
                                    'scripts/test-clef-server.py --real-processor --real-pillow'])

    def test_lazy_missing_backend_class_is_not_an_available_processor(self):
        source = (ROOT / 'scripts/verify.sh').read_text()
        helpers = self.helper(source, 'run') + '\n' + self.helper(source, 'optional_module')
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            (directory / 'transformers.py').write_text(
                'class Qwen3VLVideoProcessor:\n'
                '    def __init__(self):\n'
                '        raise ImportError("synthetic missing backend")\n')
            (directory / 'PIL').mkdir()
            (directory / 'PIL/__init__.py').write_text('')
            (directory / 'PIL/Image.py').write_text('')
            python = directory / 'python3'
            python.write_text('#!/bin/sh\nexec ' + shlex.quote(sys.executable) + ' "$@"\n')
            python.chmod(0o700)
            for mode in ['full', 'push']:
                script = ('set -Eeuo pipefail\nMODE=' + mode +
                          '\nFAILED=();SKIPPED=();PASSED=();BOLD="";OFF="";RED=""\n' + helpers +
                          "\noptional_module processor transformers install-hint true\n" +
                          "printf '%s,%s,%s\\n' \"${#FAILED[@]}\" \"${#SKIPPED[@]}\" \"${#PASSED[@]}\"\n")
                result = subprocess.run([shutil.which('bash'), '-c', script],
                    env={'PATH': name, 'PYTHONPATH': name}, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.strip(), '1,0,0' if mode == 'push' else '0,1,0')


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

    def test_clef_media_diagnosis_uses_selected_interpreter_and_actual_imports(self):
        with patch.dict(os.environ,{'JEV_CLEF_PYTHON':'/explicit/python'},clear=True), \
             patch.object(setup.shutil,'which',return_value='/tool'), \
             patch.object(setup,'command',side_effect=self.metadata) as probe:
            checks=setup.diagnose()
        rows=[row for row in checks if row['name'].startswith('Clef media ')]
        self.assertEqual(len(rows),2)
        self.assertTrue(all(row['required'] for row in rows))
        calls=[call.args for call in probe.call_args_list if call.args[0]=='/explicit/python']
        self.assertEqual(len(calls),2)
        self.assertTrue(any('from transformers import Qwen3VLVideoProcessor' in call[3] for call in calls))
        self.assertTrue(any('from PIL import Image' in call[3] for call in calls))
        self.assertTrue(any('Qwen3VLVideoProcessor()' in call[3] for call in calls))

    def test_partial_clef_media_install_is_required_and_actionable(self):
        def metadata(*args):
            if len(args)>1 and args[1]=='-B':
                return subprocess.CompletedProcess(args,1,'','synthetic missing processor import')
            return self.metadata(*args)
        with patch.dict(os.environ,{'JEV_CLEF_PYTHON':'/explicit/python'},clear=True), \
             patch.object(setup.shutil,'which',return_value='/tool'), \
             patch.object(setup,'command',side_effect=metadata):
            checks=setup.diagnose()
        rows=[row for row in checks if row['name'].startswith('Clef media ')]
        self.assertEqual(len(rows),2)
        self.assertTrue(all(row['status']=='missing' and row['required'] for row in rows))
        self.assertTrue(all('JEV_CLEF_PYTHON' in row['remedy'] and 'clef-live-testing.md' in row['remedy'] for row in rows))

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


class ReleaseManifestPaths(unittest.TestCase):
    def rewrite(self, values, *, link_escape=False, origin="alias"):
        source = (ROOT / "scripts/release-dry-run.sh").read_text()
        marker = 'python3 - "$MANIFEST" "$SOURCE_TREE/target" "$CARGO_TARGET_DIR" <<\'PYTHON\'\n'
        program = source.split(marker, 1)[1].split('\nPYTHON\n', 1)[0]
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            destination = directory / "host-target"
            destination.mkdir()
            tree = directory / "source"
            tree.mkdir()
            alias = tree / "target"
            alias.symlink_to(destination, target_is_directory=True)
            if link_escape:
                outside = directory / "outside"
                outside.mkdir()
                (destination / "escaped").symlink_to(outside, target_is_directory=True)
            manifest = directory / "manifest.json"
            bases = {"alias": alias, "resolved": destination, "relative": Path("target"),
                     "outside": directory / "outside"}
            paths = [str(bases[origin] / value) for value in values]
            original = {"artifacts": {"archive": {"path": paths[0]}}, "upload_files": paths}
            manifest.write_text(json.dumps(original))
            result = subprocess.run([sys.executable, "-B", "-", str(manifest), str(alias), str(destination)],
                                    input=program, text=True, capture_output=True, check=False, cwd=tree)
            return result, json.loads(manifest.read_text()), original, str(destination)

    def test_owned_archive_and_upload_paths_remain_usable_after_capture_cleanup(self):
        result, actual, _, destination = self.rewrite(["dist/archive.tar.xz", "dist/checksum"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(actual["artifacts"]["archive"]["path"], str(Path(destination) / "dist/archive.tar.xz"))
        self.assertEqual(actual["upload_files"], [str(Path(destination) / "dist" / name)
                                                 for name in ["archive.tar.xz", "checksum"]])

    def test_parent_components_fail_before_manifest_is_replaced(self):
        for path in ["../source.rs", "foo/../../outside", "dist/../archive.tar.xz"]:
            with self.subTest(path=path):
                result, actual, original, _ = self.rewrite([path])
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(actual, original)

    def test_symlink_escape_fails_before_manifest_is_replaced(self):
        result, actual, original, _ = self.rewrite(["escaped/archive.tar.xz"], link_escape=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(actual, original)

    def test_absolute_outside_paths_fail_before_manifest_is_replaced(self):
        result, actual, original, _ = self.rewrite(["archive.tar.xz"], origin="outside")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(actual, original)

    def test_resolved_target_symlink_escape_is_refused(self):
        result, actual, original, _ = self.rewrite(["escaped/archive.tar.xz"],
                                                  origin="resolved", link_escape=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(actual, original)

    def test_relative_parent_escape_is_refused(self):
        result, actual, original, _ = self.rewrite(["../source.rs"], origin="relative")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(actual, original)

    def test_relative_owned_paths_are_canonicalized(self):
        result, actual, _, destination = self.rewrite(["dist/archive.tar.xz"], origin="relative")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(actual["upload_files"], [str(Path(destination) / "dist/archive.tar.xz")])

    def test_resolved_owned_paths_remain_usable(self):
        result, actual, _, destination = self.rewrite(["dist/archive.tar.xz"], origin="resolved")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(actual["upload_files"], [str(Path(destination) / "dist/archive.tar.xz")])


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

    def test_codex_read_only_tools_cannot_lose_their_offline_gate(self):
        path = self.root / "scripts/verify.sh"
        original = path.read_text()
        command = "python3 scripts/test-skill-eval-tools.py"
        self.assertIn(command, original)
        path.write_text("\n".join("# " + line if command in line else line
                                  for line in original.splitlines()) + "\n")
        self.assertTrue(any("no longer runs test-skill-eval-tools.py" in error
                            for error in readiness.check(self.root)))

    def test_clef_proof_scripts_cannot_be_removed_from_the_gate(self):
        path = self.root / "scripts/verify.sh"
        original = path.read_text()
        for script in ["test-source-snapshot.py", "test-clef-live.py", "test-clef-quality.py",
                       "test-clef-python-profile.py",
                       "test-clef-model-manifest.py", "test-clef-server.py"]:
            with self.subTest(script=script):
                path.write_text("\n".join("# " + line if "scripts/" + script in line else line
                                          for line in original.splitlines()) + "\n")
                self.assertTrue(any("no longer runs " + script in error for error in readiness.check(self.root)))

    def test_real_media_checks_require_their_flags_and_both_processor_interpreters(self):
        path = self.root / "scripts/verify.sh"
        original = path.read_text()
        for before, after in [("--real-processor", ""), ("--real-pillow", ""),
                              ('"$JEV_CLEF_PYTHON" scripts/test-clef-server.py',
                               'python3 scripts/test-clef-server.py'),
                              ('python3 scripts/test-clef-server.py --real-processor --real-pillow',
                               'python3 scripts/test-clef-server.py --real-pillow')]:
            with self.subTest(mutation=before):
                path.write_text(original.replace(before, after))
                self.assertTrue(any("real media gate" in error for error in readiness.check(self.root)))

    def test_optional_media_checks_do_not_replace_the_unconditional_bridge_suite(self):
        path = self.root / "scripts/verify.sh"
        lines = path.read_text().replace("\\\n", " ").splitlines()
        removed = [line for line in lines
                   if line.strip().startswith('run "local Clef bridge tests"')]
        self.assertEqual(1, len(removed))
        path.write_text("\n".join(line for line in lines if line not in removed) + "\n")
        self.assertTrue(any("no longer runs test-clef-server.py" in error
                            for error in readiness.check(self.root)))

    def test_real_processor_checks_do_not_replace_the_pillow_only_decoder_suite(self):
        path = self.root / "scripts/verify.sh"
        lines = path.read_text().replace("\\\n", " ").splitlines()
        removed = [line for line in lines
                   if line.strip().endswith("python3 scripts/test-clef-server.py --real-pillow")]
        self.assertEqual(1, len(removed))
        path.write_text("\n".join(line for line in lines if line not in removed) + "\n")
        self.assertTrue(any("real media gate python3 scripts/test-clef-server.py --real-pillow" in error
                            for error in readiness.check(self.root)))

    def test_echoing_a_script_command_does_not_execute_the_gate(self):
        path = self.root / "scripts/verify.sh"
        original = path.read_text()
        for script in ["test-clef-live.py", "test-clef-server.py"]:
            with self.subTest(script=script):
                path.write_text(original.replace("python3 scripts/" + script,
                                                 "echo python3 scripts/" + script))
                self.assertTrue(any("no longer runs " + script in error for error in readiness.check(self.root)))

    def test_codex_eval_harness_cannot_lose_its_offline_gate(self):
        path = self.root / "scripts/verify.sh"
        original = path.read_text()
        path.write_text("\n".join("# " + line if "scripts/test-skill-eval-codex.py" in line else line
                                  for line in original.splitlines()) + "\n")
        self.assertTrue(any("no longer runs test-skill-eval-codex.py" in error
                            for error in readiness.check(self.root)))

    def test_codex_eval_dependencies_remain_required_without_document_markers(self):
        document = self.root / readiness.MAP
        document.write_text(re.sub(r"<!-- readiness: scripts/(?:skill-eval-codex|test-skill-eval-codex)\.py -->\n?",
                                  "", document.read_text()))
        for dependency in ["scripts/skill-eval-codex.py", "scripts/test-skill-eval-codex.py"]:
            with self.subTest(dependency=dependency):
                path = self.root / dependency
                contents = path.read_bytes() if path.is_file() else (ROOT / dependency).read_bytes()
                path.unlink(missing_ok=True)
                self.assertTrue(any(dependency in error for error in readiness.check(self.root)))
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(contents)

    def test_clef_dependencies_remain_required_without_document_markers(self):
        document = self.root / readiness.MAP
        # Deleting a navigation marker must not also disable the dependency check.
        document.write_text(re.sub(r"<!-- readiness: scripts/(?:source-snapshot|test-source-snapshot|clef[^ >]*|test-clef[^ >]*)[^>]* -->\n?",
                                  "", document.read_text()))
        for dependency in ["scripts/source-snapshot.py", "scripts/test-source-snapshot.py",
                           "scripts/clef-live.py", "scripts/test-clef-live.py",
                           "scripts/clef-quality.py", "scripts/test-clef-quality.py",
                           "scripts/clef-python-profile.py", "scripts/test-clef-python-profile.py",
                           "scripts/clef-model-manifest.py", "scripts/test-clef-model-manifest.py",
                           "scripts/clef-server.py", "scripts/test-clef-server.py",
                           "scripts/clef-local/clef-manifest.json", "scripts/clef-local/clef-flash-manifest.json",
                           "scripts/clef-local/requirements.txt", "scripts/clef-local/requirements-linux-cpu.lock",
                           "scripts/clef-local/requirements-linux-cpu.hashes.lock",
                           "scripts/clef-local/requirements-linux-cpu.download.lock"]:
            with self.subTest(dependency=dependency):
                path = self.root / dependency
                contents = path.read_bytes() if path.is_file() else (ROOT / dependency).read_bytes()
                path.unlink(missing_ok=True)
                self.assertTrue(any(dependency in error for error in readiness.check(self.root)))
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(contents)

    def test_canonical_skills_cannot_resolve_outside_the_checkout(self):
        path = self.root / ".claude/skills/api-compat/SKILL.md"
        contents = path.read_bytes()
        path.unlink()
        with tempfile.TemporaryDirectory(prefix="jev-external-skill-") as work:
            external = Path(work) / "SKILL.md"
            external.write_bytes(contents)
            path.symlink_to(external)
            self.assertTrue(any("external canonical development skill api-compat" in error
                                for error in readiness.check(self.root)))

    def test_generated_skill_drift_is_reported_without_mutating_the_copy(self):
        path = self.root / ".agents/skills/api-compat/SKILL.md"
        path.parent.mkdir(parents=True)
        canonical = (self.root / ".claude/skills/api-compat/SKILL.md").read_text()
        stale = canonical.replace(".claude/", ".Codex/")
        path.write_text(stale)
        errors = readiness.check_adapters(self.root)
        self.assertTrue(any(".agents/skills/api-compat/SKILL.md" in error and "canonical" in error for error in errors))
        self.assertEqual(stale, path.read_text())
        self.assertEqual([], readiness.check(self.root))
        path.write_text(canonical)
        self.assertEqual([], readiness.check_adapters(self.root))

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
