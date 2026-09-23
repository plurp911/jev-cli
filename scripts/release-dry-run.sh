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

OUT=target/distrib

missing() {
  printf '%s is not installed.\n  %s\n' "$1" "$2" >&2
  exit 127
}
command -v dist >/dev/null 2>&1 || missing dist "cargo install cargo-dist --locked"

printf '==> plan\n'
dist plan

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

# The per-target manifest is what `--artifacts=global` reads to learn each archive's
# checksum. Producing it here is what makes the installer check below meaningful, and
# it mirrors exactly what the release workflow's build job uploads. It is written
# outside "$OUT" first: `dist` scans that directory as it runs, and would try to parse
# the half-written file.
MANIFEST="$(mktemp)"
dist build --artifacts=local --target "$HOST_TARGET" --output-format=json > "$MANIFEST"
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
trap 'rm -rf "$STAGE"' EXIT
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
env -i "JEV_CONFIG_DIR=$STAGE/config" "PATH=/usr/bin:/bin" "$binary" --version
env -i "JEV_CONFIG_DIR=$STAGE/config" "PATH=/usr/bin:/bin" "$binary" doctor >/dev/null
printf '  ok  the unpacked binary runs and reports its configuration\n'

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
