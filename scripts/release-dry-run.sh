#!/usr/bin/env bash
# Build the release artifacts for the host platform and prove they work.
#
# This is the local rehearsal of `.github/workflows/release.yml`. It builds exactly what
# a release would contain for *this* machine's target, generates the checksum and the
# SBOM, unpacks the archive into a clean directory, and runs the binary from there.
#
# It publishes nothing, tags nothing, and pushes nothing, and there is no flag that
# makes it do so. See AGENTS.md §12.
set -Eeuo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

# Child tools invoke Git too. Keep inherited repository redirects, configuration,
# and executable filters outside the entire rehearsal, not only source capture.
while IFS= read -r rehearsal_git_name; do
  unset "$rehearsal_git_name"
done < <(compgen -A variable GIT_ || true)
export GIT_CONFIG_GLOBAL=/dev/null
export GIT_CONFIG_SYSTEM=/dev/null
export GIT_CONFIG_NOSYSTEM=1
export GIT_TEMPLATE_DIR=""

REHEARSAL_ROOT="$(pwd -P)"
OUT="$REHEARSAL_ROOT/target/distrib"
export CARGO_TARGET_DIR="$REHEARSAL_ROOT/target"

missing() {
  printf '%s is not installed.\n  %s\n' "$1" "$2" >&2
  exit 127
}
command -v dist >/dev/null 2>&1 || missing dist "cargo install cargo-dist --locked"

printf '\n==> build local artifacts for this host\n'
# Only this machine's target. `dist` refuses to cross-compile to macOS, and the release
# workflow builds every target on its own runner; rehearsing that locally is not
# possible and pretending otherwise would make this script lie.
HOST_TARGET="$(rustc -vV | awk '/^host:/ {print $2}')"
printf '  host target: %s\n' "$HOST_TARGET"
# Start from an empty output directory. `dist` reads every `*-dist-manifest.json` it
# finds here, so a manifest left over from an earlier run would make this rehearsal
# report on artifacts that no longer exist.
rm -rf "$OUT"
mkdir -p "$OUT"

# Compile an isolated, read-only copy of the captured source. Main-worktree edits
# during compilation cannot change its inputs, even if they are later restored.
# Build tools are trusted not to deliberately rewrite the owner's read-only files.
# Keep source outside Cargo's cache so cache cleanup cannot remove the checkout.
SOURCE_REHEARSAL="$(mktemp -d /tmp/jev-source-rehearsal.XXXXXX)"
SOURCE_TREE="$SOURCE_REHEARSAL/tree"
MANIFEST=""
STAGE=""
cleanup() {
  if [ -d "$SOURCE_TREE" ]; then
    chmod -R u+w "$SOURCE_TREE"
  fi
  rm -rf "$SOURCE_REHEARSAL"
  if [ -n "$MANIFEST" ]; then rm -f "$MANIFEST"; fi
  if [ -n "$STAGE" ]; then rm -rf "$STAGE"; fi
}
trap cleanup EXIT
python3 scripts/source-snapshot.py --output "$SOURCE_REHEARSAL/source.tar.gz" --build-tree "$SOURCE_TREE"
# dist uses the workspace's target directory independently of CARGO_TARGET_DIR.
# Pin both tools to the same writable cache; captured source stays read-only.
chmod u+w "$SOURCE_TREE"
ln -s "$CARGO_TARGET_DIR" "$SOURCE_TREE/target"
chmod u-w "$SOURCE_TREE"
cd "$SOURCE_TREE"

printf '\n==> plan from captured source\n'
dist plan

# The per-target manifest is what `--artifacts=global` reads to learn each archive's
# checksum. Producing it here is what makes the installer check below meaningful, and
# it mirrors exactly what the release workflow's build job uploads. It is written
# outside "$OUT" first: `dist` scans that directory as it runs, and would try to parse
# the half-written file.
MANIFEST="$(mktemp)"
dist build --artifacts=local --target "$HOST_TARGET" --output-format=json > "$MANIFEST"
# These schema fields describe local artifacts. Keep them usable after the
# temporary source tree (including its target alias) is removed.
python3 - "$MANIFEST" "$SOURCE_TREE/target" "$CARGO_TARGET_DIR" <<'PYTHON'
import json
from pathlib import Path
import sys

path, alias, destination = map(Path, sys.argv[1:])
with path.open("rb") as stream:
    raw = stream.read(4 * 1024 * 1024 + 1)
if len(raw) > 4 * 1024 * 1024:
    raise SystemExit("release manifest exceeds its size bound")
manifest = json.loads(raw)
resolved_destination = destination.resolve(strict=True)

def canonical(value):
    if not isinstance(value, str):
        return value
    requested = Path(value)
    if ".." in requested.parts:
        raise SystemExit("release manifest path contains a parent component")
    if not requested.is_absolute():
        requested = alias.parent / requested
    try:
        relative = requested.resolve().relative_to(resolved_destination)
    except (ValueError, OSError, RuntimeError):
        raise SystemExit("release manifest path escapes the captured target") from None
    return str(resolved_destination / relative)

for artifact in manifest.get("artifacts", {}).values():
    if "path" in artifact:
        artifact["path"] = canonical(artifact["path"])
if "upload_files" in manifest:
    manifest["upload_files"] = [canonical(value) for value in manifest["upload_files"]]
path.write_text(json.dumps(manifest, indent=2) + "\n")
PYTHON
mv "$MANIFEST" "$OUT/$HOST_TARGET-dist-manifest.json"

printf '\n==> checksums\n'
shopt -s nullglob
archives=("$OUT"/*.tar.xz "$OUT"/*.zip)
if [ "${#archives[@]}" -eq 0 ]; then
  printf 'no archive was produced\n' >&2
  exit 1
fi
for archive in "${archives[@]}"; do
  sum_file="$archive.sha256"
  if [ ! -f "$sum_file" ]; then
    printf 'no checksum for %s\n' "$archive" >&2
    exit 1
  fi
  # Verify it rather than trusting that it was written.
  expected="$(cut -d' ' -f1 < "$sum_file")"
  actual="$(sha256sum "$archive" | cut -d' ' -f1)"
  if [ "$expected" != "$actual" ]; then
    printf 'checksum mismatch for %s\n' "$archive" >&2
    exit 1
  fi
  printf '  ok  %s\n' "$(basename "$archive")"
done

printf '\n==> installers and formula\n'
# Only this host's target has a real manifest, so the other arms of the installer
# legitimately have no digest. `--only` tells the checker to require a digest for
# this target alone; a full release build has every target and runs without that flag.
dist build --artifacts=global >/dev/null
python3 scripts/check-installers.py "$OUT" --only "$HOST_TARGET"

printf '\n==> source matches the binary build\n'
python3 scripts/source-snapshot.py --check-build-tree --check "$SOURCE_REHEARSAL/source.tar.gz"
cp "$SOURCE_REHEARSAL/source.tar.gz" "$OUT/source.tar.gz"
python3 - "$OUT" <<'PY'
import hashlib
from pathlib import Path
import sys

directory = Path(sys.argv[1])
digest = hashlib.sha256((directory / "source.tar.gz").read_bytes()).hexdigest()
(directory / "source.tar.gz.sha256").write_text(f"{digest} *source.tar.gz\n")
aggregate = directory / "sha256.sum"
lines = [line for line in aggregate.read_text().splitlines() if not line.endswith(" *source.tar.gz")]
lines.append(f"{digest} *source.tar.gz")
aggregate.write_text("\n".join(lines) + "\n")
PY
printf '  ok  source archive includes the exact isolated build files and original-worktree provenance\n'

printf '\n==> SBOM\n'
if command -v cargo-sbom >/dev/null 2>&1; then
  cargo sbom --output-format spdx_json_2_3 > "$OUT/jev.spdx.json"
  packages="$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1]))["packages"]))' \
      "$OUT/jev.spdx.json")"
  printf '  ok  jev.spdx.json (%s packages)\n' "$packages"
else
  printf '  skip  cargo-sbom not installed (cargo install cargo-sbom --locked)\n'
fi

printf '\n==> install smoke test\n'
STAGE="$(mktemp -d)"
for archive in "${archives[@]}"; do
  case "$archive" in
    *.tar.xz) tar -xJf "$archive" -C "$STAGE" ;;
    *.zip)    unzip -q "$archive" -d "$STAGE" ;;
  esac
done

binary="$(find "$STAGE" -type f -name 'jev' -o -type f -name 'jev.exe' | head -1)"
if [ -z "$binary" ]; then
  printf 'the archive does not contain a jev binary\n' >&2
  exit 1
fi
chmod +x "$binary"

# Run it the way a user would, from an unpacked archive, with nothing inherited.
env -i "JEV_CONFIG_DIR=$STAGE/config" "JEV_NO_KEYCHAIN=1" "PATH=/usr/bin:/bin" "$binary" --version
env -i "JEV_CONFIG_DIR=$STAGE/config" "JEV_NO_KEYCHAIN=1" "PATH=/usr/bin:/bin" "$binary" doctor >/dev/null
# A missing credential must fail locally, before the artifact can contact the API.
# Disabling the OS store makes this independent of the releaser's real keychain.
set +e
printf 'state' | env -i "JEV_CONFIG_DIR=$STAGE/config" "JEV_NO_KEYCHAIN=1" \
  "PATH=/usr/bin:/bin" "$binary" noul "is this urgent?" >"$STAGE/no-key.stdout" 2>"$STAGE/no-key.stderr"
code=$?
set -e
# Exit 3 also covers an HTTP authentication rejection. Check the local reason and
# disabled store explicitly; never echo an unexpected diagnostic into a release log.
if [ "$code" != 3 ] || [ -s "$STAGE/no-key.stdout" ] || \
  ! grep -Fq 'no TypeSafe API key found' "$STAGE/no-key.stderr" || \
  ! grep -Fq 'disabled by JEV_NO_KEYCHAIN' "$STAGE/no-key.stderr" || \
  ! grep -Fq 'JEV_API_KEY' "$STAGE/no-key.stderr" || \
  grep -Fiq 'HTTP' "$STAGE/no-key.stderr"; then
  printf 'expected a local missing-credential refusal with the OS store disabled (exit 3), got exit %s\n' "$code" >&2
  exit 1
fi
printf '  ok  the unpacked binary runs, reports configuration, and refuses missing credentials\n'

# The archive must also carry the licences and the changelog, because a binary
# distributed without its licence text is a licence violation.
for required in LICENSE-MIT LICENSE-APACHE README.md CHANGELOG.md; do
  if ! find "$STAGE" -name "$required" | grep -q .; then
    printf 'the archive is missing %s\n' "$required" >&2
    exit 1
  fi
  printf '  ok  %s is in the archive\n' "$required"
done

printf '\n==> what was built\n'
# `find -printf` is a GNU extension: BSD find, which is what macOS ships, fails with
# "unknown primary or operator" and took the whole script down at its last line. `wc -c`
# is POSIX and agrees on both.
find "$OUT" -maxdepth 1 -type f | sort | while IFS= read -r file; do
  printf '  %-52s %s bytes\n' "$(basename "$file")" "$(wc -c < "$file")"
done

printf '\nNothing was published. Releasing is a human decision; see AGENTS.md §12.\n'
