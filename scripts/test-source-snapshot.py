#!/usr/bin/env python3
"""Exercise source rehearsal archives against isolated, real Git repositories."""

from pathlib import Path
import importlib.util
import io
import os
import shlex
import shutil
from unittest.mock import patch
import json
import subprocess
import sys
import tarfile
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("source-snapshot.py")


class UntrackedReadGuard:
    """Allow descriptor inspection and cleanup, but fail any content disclosure."""
    def __init__(self, stream): self.stream = stream
    def __enter__(self): return self
    def __exit__(self, *args): return self.stream.__exit__(*args)
    def fileno(self): return self.stream.fileno()
    def read(self, *args):
        raise AssertionError("an untracked symlink target was read")


class SourceSnapshotTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="jev-source-snapshot-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.git("init", "-q")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("config", "user.name", "Source fixture")
        (self.root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.0.1"\n')
        (self.root / "source.rs").write_text("old source\n")
        self.git("add", "Cargo.toml", "source.rs")
        self.git("commit", "-qm", "fixture")
        self.archive = self.root / "target" / "source.tar.gz"

    def git(self, *args):
        return subprocess.run(["git", "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false", *args], cwd=self.root, check=True,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    def run_script(self, *args):
        return subprocess.run([sys.executable, str(SCRIPT), "--root", str(self.root), *args],
                              text=True, capture_output=True)

    def build(self):
        result = self.run_script("--output", str(self.archive))
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_snapshot_never_refreshes_the_original_git_index(self):
        index=self.root/'.git/index'
        os.utime(index,ns=(946684800000000000,946684800000000000))
        os.utime(self.root/'source.rs',ns=(946684801000000000,946684801000000000))
        before=index.read_bytes();stamp=index.stat().st_mtime_ns
        self.build()
        self.assertEqual(index.read_bytes(),before)
        self.assertEqual(index.stat().st_mtime_ns,stamp)

    def test_archive_contains_dirty_and_new_staged_source_but_no_untracked_secret(self):
        (self.root / "source.rs").write_text("new source\n")
        (self.root / "bridge.py").write_text("explicit bridge source\n")
        self.git("add", "bridge.py")
        (self.root / "private-token").write_text("secret-canary-should-never-be-archived")
        self.build()
        with tarfile.open(self.archive) as archive:
            self.assertEqual(archive.extractfile("jev-cli-0.0.1/source.rs").read(), b"new source\n")
            self.assertEqual(archive.extractfile("jev-cli-0.0.1/bridge.py").read(), b"explicit bridge source\n")
            self.assertFalse(any("private-token" in entry.name for entry in archive))
            receipt = json.load(archive.extractfile("jev-cli-0.0.1/.jev-source.json"))
            self.assertTrue(receipt["working_tree_changes"])
        self.assertEqual(self.run_script("--check", str(self.archive)).returncode, 0)

    def test_tracked_receipt_name_is_refused_before_emitting_an_archive(self):
        reserved = self.root / ".jev-source.json"
        reserved.write_bytes(b"explicitly tracked source")
        self.git("add", ".jev-source.json")
        result = self.run_script("--output", str(self.archive))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("reserved", result.stderr)
        self.assertFalse(self.archive.exists())
        self.assertEqual(reserved.read_bytes(), b"explicitly tracked source")

    def test_source_change_after_snapshot_refuses_consistency_claim(self):
        self.build()
        (self.root / "source.rs").write_text("changed during binary build\n")
        result = self.run_script("--check", str(self.archive))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("source.rs", result.stderr)

    def test_adding_tracked_source_after_snapshot_is_detected(self):
        self.build()
        (self.root / "extra.rs").write_text("new module\n")
        self.git("add", "extra.rs")
        self.assertNotEqual(self.run_script("--check", str(self.archive)).returncode, 0)

    def test_adding_a_tracked_reserved_receipt_after_snapshot_is_refused(self):
        self.build()
        reserved = self.root / ".jev-source.json"
        reserved.write_bytes(b"new explicitly tracked source")
        self.git("add", ".jev-source.json")
        result = self.run_script("--check", str(self.archive))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("reserved", result.stderr)

    def test_build_tree_receipt_metadata_tampering_is_detected(self):
        for field, value in [("base_revision", "0" * 40), ("working_tree_changes", False), ("files", {})]:
            with self.subTest(field=field):
                (self.root / "source.rs").write_text("dirty captured source\n")
                tree = self.root / "target" / ("build-tree-" + field)
                result = self.run_script("--output", str(self.archive), "--build-tree", str(tree))
                self.assertEqual(result.returncode, 0, result.stderr)
                receipt_file = tree / ".jev-source.json"
                receipt = json.loads(receipt_file.read_text())
                receipt[field] = value
                receipt_file.chmod(0o644)
                receipt_file.write_text(json.dumps(receipt))
                result = subprocess.run([sys.executable, str(SCRIPT), "--root", str(tree), "--check-build-tree", "--check", str(self.archive)],
                                        capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0)

    def test_build_tree_receipt_mode_tampering_is_detected(self):
        tree = self.root / "target" / "build-tree"
        result = self.run_script("--output", str(self.archive), "--build-tree", str(tree))
        self.assertEqual(result.returncode, 0, result.stderr)
        (tree / ".jev-source.json").chmod(0o755)
        result = subprocess.run([sys.executable, str(SCRIPT), "--root", str(tree), "--check-build-tree", "--check", str(self.archive)],
                                capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)

    def test_explicit_build_tree_check_refuses_a_missing_generated_receipt(self):
        self.build()
        result = self.run_script("--check-build-tree", "--check", str(self.archive))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing", result.stderr)

    def test_build_tree_check_mode_requires_a_check_action(self):
        result = self.run_script("--check-build-tree", "--output", str(self.archive))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("requires --check", result.stderr)
        self.assertFalse(self.archive.exists())

    def module(self):
        spec = importlib.util.spec_from_file_location("source_snapshot", SCRIPT)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_snapshot_budget_includes_the_generated_receipt(self):
        module = self.module()
        source_size = sum(len(data) for data, _ in module.source_files(self.root).values())
        with patch.object(module, "MAX_SOURCE", source_size + 1):
            with self.assertRaises(ValueError):
                module.snapshot(self.root, self.archive)
        self.assertFalse(self.archive.exists())

    def test_snapshot_receipt_must_fit_the_per_file_bound(self):
        module = self.module()
        with patch.object(module, "MAX_FILE", 64):
            with self.assertRaises(ValueError):
                module.snapshot(self.root, self.archive)
        self.assertFalse(self.archive.exists())

    def test_snapshot_entry_budget_includes_the_generated_receipt(self):
        module = self.module()
        for number in range(4):
            (self.root / f"tiny-{number}").write_bytes(b"x")
        self.git("add", "tiny-0", "tiny-1", "tiny-2", "tiny-3")
        with patch.object(module, "MAX_ENTRIES", 6, create=True):
            with self.assertRaises(ValueError):
                module.snapshot(self.root, self.archive)
        self.assertFalse(self.archive.exists())

    def test_archive_exact_size_and_entry_bounds_round_trip(self):
        module = self.module()
        module.snapshot(self.root, self.archive)
        with tarfile.open(self.archive) as archive:
            entries = list(archive)
        with patch.object(module, "MAX_SOURCE", sum(entry.size for entry in entries)), \
             patch.object(module, "MAX_FILE", max(entry.size for entry in entries)), \
             patch.object(module, "MAX_ENTRIES", len(entries), create=True):
            module.snapshot(self.root, self.archive)
            module.check(self.root, self.archive)

    def test_archive_checker_counts_the_receipt_against_the_entry_bound(self):
        module = self.module()
        module.snapshot(self.root, self.archive)
        with patch.object(module, "MAX_ENTRIES", 2, create=True):
            with self.assertRaises(ValueError):
                module.check(self.root, self.archive)

    def test_file_growth_after_metadata_cannot_trigger_an_unbounded_read(self):
        module = self.module()
        source = self.root / "source.rs"
        inode = source.stat().st_ino
        real_open = io.open
        requests = []
        class GrowingStream:
            def __init__(self, stream): self.stream = stream
            def __enter__(self): return self
            def __exit__(self, *args): return self.stream.__exit__(*args)
            def fileno(self): return self.stream.fileno()
            def read(self, size=-1):
                requests.append(size)
                with real_open(source, "ab") as output:
                    output.write(b"x" * 1024)
                if size < 0:
                    raise AssertionError("tracked source was read without an allocation bound")
                return self.stream.read(size)
        def opened(path, *args, **kwargs):
            stream = real_open(path, *args, **kwargs)
            if os.fstat(stream.fileno()).st_ino == inode:
                return GrowingStream(stream)
            return stream
        with patch.object(module, "MAX_FILE", 64), patch.object(io, "open", opened):
            with self.assertRaises(ValueError):
                module.source_files(self.root)
        self.assertEqual(len(requests), 1)
        self.assertLessEqual(requests[0], 65)

    def test_final_symlink_swap_before_open_never_reads_the_untracked_target(self):
        module = self.module()
        source = self.root / "source.rs"
        private = self.root / "untracked-private"
        private.write_bytes(b"canary-leak")
        private_inode = private.stat().st_ino
        real_open, real_os_open = io.open, os.open
        swapped = []
        def swap(path):
            if not isinstance(path, int) and Path(path).name == "source.rs" and not swapped:
                source.unlink()
                source.symlink_to(private)
                swapped.append(True)
        def opened(path, *args, **kwargs):
            swap(path)
            stream = real_open(path, *args, **kwargs)
            return UntrackedReadGuard(stream) if os.fstat(stream.fileno()).st_ino == private_inode else stream
        def descriptor(path, *args, **kwargs):
            swap(path)
            return real_os_open(path, *args, **kwargs)
        with patch.object(io, "open", opened), patch.object(os, "open", descriptor):
            with self.assertRaises((ValueError, OSError)):
                module.source_files(self.root)
        self.assertTrue(swapped)

    def test_parent_directory_symlink_swap_before_open_is_refused(self):
        self.assert_parent_swap_refused(fallback=False)

    def test_portable_fallback_refuses_parent_swap_before_reading_contents(self):
        self.assert_parent_swap_refused(fallback=True)

    def assert_parent_swap_refused(self, *, fallback):
        module = self.module()
        if fallback:
            module.DIR_FD_OPEN = False
        directory = self.root / "module"
        directory.mkdir()
        (directory / "inner.rs").write_bytes(b"tracked source")
        self.git("add", "module/inner.rs")
        private = self.root / "untracked-module"
        private.mkdir()
        (private / "inner.rs").write_bytes(b"private canary")
        private_inode = (private / "inner.rs").stat().st_ino
        real_open, real_os_open = io.open, os.open
        swapped = []
        def swap(path):
            if not isinstance(path, int) and "module" in Path(path).parts and not swapped:
                directory.rename(self.root / "original-module")
                directory.symlink_to(private, target_is_directory=True)
                swapped.append(True)
        def opened(path, *args, **kwargs):
            swap(path)
            stream = real_open(path, *args, **kwargs)
            return UntrackedReadGuard(stream) if os.fstat(stream.fileno()).st_ino == private_inode else stream
        def descriptor(path, *args, **kwargs):
            swap(path)
            return real_os_open(path, *args, **kwargs)
        with patch.object(io, "open", opened), patch.object(os, "open", descriptor):
            with self.assertRaises((ValueError, OSError)):
                module.source_files(self.root)
        self.assertTrue(swapped)

    def test_build_tree_uses_frozen_dirty_and_staged_source_even_if_main_changes_and_restores(self):
        revision = self.git("rev-parse", "HEAD").stdout
        (self.root / "source.rs").write_bytes(b"captured build source\n")
        (self.root / "bridge.py").write_bytes(b"staged build source\n")
        self.git("add", "bridge.py")
        (self.root / "untracked-private").write_bytes(b"never snapshot this canary")
        tree = self.root / "target" / "build-tree"
        result = self.run_script("--output", str(self.archive), "--build-tree", str(tree))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.git("rev-parse", "HEAD").stdout, revision)
        self.assertEqual((tree / "bridge.py").read_bytes(), b"staged build source\n")
        self.assertFalse((tree / "untracked-private").exists())
        (self.root / "source.rs").write_bytes(b"transient wrong build source\n")
        compiled = subprocess.run([sys.executable, "-c", "from pathlib import Path; print(Path('source.rs').read_text(), end='')"],
                                  cwd=tree, capture_output=True, check=True).stdout
        (self.root / "source.rs").write_bytes(b"captured build source\n")
        self.assertEqual(compiled, b"captured build source\n")
        tracked = subprocess.run(["git", "show", "HEAD:source.rs"], cwd=tree, capture_output=True, check=True)
        self.assertEqual(tracked.stdout, compiled)
        receipt = json.loads((tree / ".jev-source.json").read_text())
        self.assertEqual(receipt["base_revision"], revision.decode().strip())
        self.assertTrue(receipt["working_tree_changes"])
        result = subprocess.run([sys.executable,str(SCRIPT),"--root",str(tree),"--check-build-tree","--check",str(self.archive)],
                                capture_output=True,text=True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_build_tree_preserves_crlf_bytes_and_does_not_run_inherited_git_hooks(self):
        (self.root / "source.rs").write_bytes(b"captured source\r\n")
        (self.root / ".gitattributes").write_text("source.rs text eol=lf\n")
        self.git("add", ".gitattributes")
        hooks = self.root / "untracked-hooks"
        hooks.mkdir()
        marker = self.root / "unexpected-hook"
        (hooks / "pre-commit").write_text("#!/bin/sh\nprintf 'ran' > " + str(marker) + "\n")
        (hooks / "pre-commit").chmod(0o755)
        config = self.root / "untracked-gitconfig"
        config.write_text("[core]\n hooksPath = " + str(hooks) + "\n[commit]\n gpgSign = true\n")
        tree = self.root / "target" / "build-tree"
        with patch.dict(os.environ,{"GIT_CONFIG_GLOBAL":str(config)}):
            result = self.run_script("--output",str(self.archive),"--build-tree",str(tree))
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertFalse(marker.exists())
        committed = subprocess.run(["git","show","HEAD:source.rs"],cwd=tree,capture_output=True,check=True).stdout
        self.assertEqual(committed,b"captured source\r\n")
        self.assertEqual((tree / "source.rs").read_bytes(),committed)

    def test_build_tree_git_history_survives_removal_of_original_objects(self):
        self.git("tag", "v0.0.1")
        revision = self.git("rev-parse", "HEAD").stdout.strip()
        tree = self.root / "target" / "build-tree"
        result = self.run_script("--output", str(self.archive), "--build-tree", str(tree))
        self.assertEqual(result.returncode, 0, result.stderr)
        objects = self.root / ".git/objects"
        objects.rename(self.root / ".git/saved-objects")
        parent = subprocess.run(["git", "cat-file", "-p", revision.decode()], cwd=tree, capture_output=True)
        self.assertEqual(parent.returncode, 0, parent.stderr)
        tag = subprocess.run(["git", "rev-parse", "v0.0.1"], cwd=tree, capture_output=True, check=True)
        self.assertEqual(tag.stdout.strip(), revision)
        self.assertFalse((tree / ".git/objects/info/alternates").exists())

    def test_build_tree_refuses_an_existing_destination_without_overwriting_it(self):
        tree = self.root / "target" / "build-tree"
        tree.mkdir(parents=True)
        canary = tree / "keep.txt"
        canary.write_bytes(b"existing destination")
        result = self.run_script("--output",str(self.archive),"--build-tree",str(tree))
        self.assertNotEqual(result.returncode,0)
        self.assertEqual(canary.read_bytes(),b"existing destination")

    def test_build_tree_refuses_a_dangling_symlink_destination_without_creating_its_target(self):
        tree = self.root / "target" / "build-tree"
        tree.parent.mkdir(parents=True)
        redirected = self.root / "untracked-redirected-tree"
        tree.symlink_to(redirected, target_is_directory=True)
        result = self.run_script("--output",str(self.archive),"--build-tree",str(tree))
        self.assertNotEqual(result.returncode,0)
        self.assertFalse(redirected.exists())

    def release_fixture(self, *, manifest=False, environment=None):
        scripts = self.root / "scripts"
        scripts.mkdir()
        shutil.copy2(SCRIPT, scripts / SCRIPT.name)
        shutil.copy2(SCRIPT.with_name("release-dry-run.sh"), scripts / "release-dry-run.sh")
        (self.root / "source.rs").write_bytes(b"captured release source\n")
        self.git("add", "scripts")
        tools = self.root / "untracked-tools"
        tools.mkdir()
        (tools / "rustc").write_text("#!/bin/sh\nprintf 'host: x86_64-unknown-linux-gnu\\n'\n")
        (tools / "rustc").chmod(0o755)
        (tools / "dist").write_text("#!" + sys.executable + "\n" + r'''import json, os, subprocess, sys
from pathlib import Path
if os.environ.get("SNAPSHOT_FIXTURE_GIT_PROBE") == "1":
    subprocess.run(["git", "status", "--porcelain"], check=True, capture_output=True)
    git_root = subprocess.run(["git", "rev-parse", "--show-toplevel"], check=True, capture_output=True, text=True).stdout.strip()
    original = Path(os.environ["SNAPSHOT_FIXTURE_MAIN"])
    (original / "target" / "observed-git.json").write_text(json.dumps({"cwd":os.getcwd(),"git_root":git_root}))
if "--artifacts=local" in sys.argv:
    original = Path(os.environ["SNAPSHOT_FIXTURE_MAIN"])
    source = original / "source.rs"
    captured = source.read_bytes()
    source.write_bytes(b"transient wrong release source\n")
    compiled = Path("source.rs").read_text()
    source.write_bytes(captured)
    (original / "target" / "observed-build.json").write_text(json.dumps({"cwd":os.getcwd(),"compiled":compiled,"target":str(Path("target").resolve())}))
    if os.environ.get("SNAPSHOT_FIXTURE_MANIFEST") == "1":
        artifact = Path("target/distrib/fixture.archive")
        artifact.write_bytes(b"local artifact")
        print(json.dumps({"artifacts":{"fixture":{"path":str(artifact.absolute())}},"upload_files":[str(artifact.absolute())]}))
        raise SystemExit(0)
    raise SystemExit(1)  # Stop after exercising the build input, before archive smoke checks.
print("{}")
''')
        (tools / "dist").chmod(0o755)
        result = subprocess.run(["bash", str(scripts / "release-dry-run.sh")], cwd=self.root,
            env={**os.environ,"PATH":str(tools)+os.pathsep+os.environ["PATH"],"SNAPSHOT_FIXTURE_MAIN":str(self.root),"SNAPSHOT_FIXTURE_MANIFEST":"1" if manifest else "0",**(environment or {})},
            capture_output=True,text=True)
        return result

    def test_release_child_git_cannot_execute_an_inherited_global_filter(self):
        (self.root / ".gitattributes").write_text("source.rs filter=canary\n")
        self.git("add", ".gitattributes")
        marker = self.root / "untracked-filter-executed"
        filter_script = self.root / "untracked-filter.py"
        filter_script.write_text("from pathlib import Path\nimport sys\n"
            f"Path({str(marker)!r}).write_text('executed')\n"
            "sys.stdout.buffer.write(sys.stdin.buffer.read())\n")
        config = self.root / "untracked-global-config"
        command = shlex.quote(sys.executable) + " " + shlex.quote(str(filter_script))
        config.write_text('[filter "canary"]\n\tclean = ' + command + '\n\trequired = true\n')
        result = self.release_fixture(environment={"GIT_CONFIG_GLOBAL":str(config),"SNAPSHOT_FIXTURE_GIT_PROBE":"1"})
        self.assertTrue((self.root / "target/observed-git.json").exists(), result.stderr)
        self.assertFalse(marker.exists(), "an inherited Git filter executed during release planning")

    def test_release_child_git_cannot_be_redirected_by_inherited_repository_variables(self):
        redirect = self.root / "untracked-redirect"
        redirect.mkdir()
        subprocess.run(["git", "init", "-q", str(redirect)], check=True, capture_output=True)
        result = self.release_fixture(environment={"GIT_DIR":str(redirect / ".git"),"GIT_WORK_TREE":str(redirect),"SNAPSHOT_FIXTURE_GIT_PROBE":"1"})
        self.assertTrue((self.root / "target/observed-git.json").exists(), result.stderr)
        observed = json.loads((self.root / "target/observed-git.json").read_text())
        self.assertEqual(observed["git_root"], observed["cwd"])
        self.assertNotEqual(Path(observed["git_root"]), redirect)

    def test_release_manifest_paths_survive_cleanup_of_the_frozen_build_tree(self):
        result = self.release_fixture(manifest=True)
        self.assertNotEqual(result.returncode,0)  # The fixture intentionally produces no installable archive.
        manifest = json.loads((self.root / "target/distrib/x86_64-unknown-linux-gnu-dist-manifest.json").read_text())
        expected = str(self.root / "target/distrib/fixture.archive")
        self.assertEqual(manifest["artifacts"]["fixture"]["path"],expected)
        self.assertEqual(manifest["upload_files"],[expected])
        self.assertTrue(Path(manifest["artifacts"]["fixture"]["path"]).is_file())

    def test_release_build_cannot_compile_transient_main_worktree_edits(self):
        result = self.release_fixture()
        self.assertNotEqual(result.returncode,0)
        self.assertTrue((self.root / "target" / "observed-build.json").exists(), result.stderr)
        observed = json.loads((self.root / "target" / "observed-build.json").read_text())
        self.assertEqual(observed["compiled"],"captured release source\n")
        self.assertNotEqual(Path(observed["cwd"]),self.root)
        self.assertEqual(Path(observed["target"]),self.root / "target")

    def test_release_source_tree_is_outside_the_shared_cargo_cache(self):
        result = self.release_fixture()
        self.assertTrue((self.root / "target/observed-build.json").exists(), result.stderr)
        observed = json.loads((self.root / "target/observed-build.json").read_text())
        self.assertFalse(Path(observed["cwd"]).is_relative_to(self.root / "target"))

    def test_snapshot_is_reproducible(self):
        self.build()
        first = self.archive.read_bytes()
        self.build()
        self.assertEqual(first, self.archive.read_bytes())

    def test_tracked_symlink_is_refused_without_reading_its_target(self):
        secret = self.root / "private-token"
        secret.write_text("secret-canary-never-read")
        (self.root / "linked.rs").symlink_to(secret)
        self.git("add", "linked.rs")
        result = self.run_script("--output", str(self.archive))
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("secret-canary-never-read", result.stderr)
        self.assertIn("symlink", result.stderr)


if __name__ == "__main__":
    unittest.main()
