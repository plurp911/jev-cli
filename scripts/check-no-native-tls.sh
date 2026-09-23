#!/usr/bin/env bash
# Assert that no native TLS stack is compiled into `jev`.
#
# ADR-0004 bans `openssl` and `openssl-sys`: `jev` verifies with `rustls`, and a system
# OpenSSL would reintroduce a C dependency and break cross-platform reproducibility.
#
# This used to be `grep '^name = "openssl"' Cargo.lock`, which is wrong, and produced a
# false failure for four commits. `Cargo.lock` records a package for every *optional*
# dependency any crate in the graph declares, whether or not the feature that enables it
# is selected. `dbus-secret-service` declares `openssl` behind its `crypto-openssl`
# feature; this workspace selects `crypto-rust`, so openssl is never built -- but the
# lockfile lists it all the same.
#
# `cargo tree -i` resolves features, so it answers the question the lockfile cannot:
# is this crate actually in the graph? Note it exits 0 and prints "nothing to print"
# when there is no dependent, so the output has to be read rather than the status.
set -Eeuo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

BANNED=(openssl openssl-sys)
failed=0

for crate in "${BANNED[@]}"; do
  # `--target all` and `--all-features` deliberately: a crate compiled only on Windows,
  # or only behind a feature the local push gate enables, is still a crate we ship.
  output="$(cargo tree --workspace --all-features --target all --invert "$crate" 2>&1 || true)"
  if printf '%s' "$output" | grep -q 'nothing to print'; then
    printf '  ok    %s is not in the compiled dependency graph\n' "$crate"
    continue
  fi
  printf '::error::%s entered the dependency graph; see ADR-0004\n' "$crate" >&2
  printf '%s\n' "$output" >&2
  failed=1
done

if [ "$failed" -ne 0 ]; then
  exit 1
fi

# The lockfile may still *list* openssl as an unselected optional dependency. Say so, so
# that the next person to grep the lockfile does not re-add the check that was wrong.
if grep -qE '^name = "openssl(-sys)?"$' Cargo.lock; then
  printf '  note  Cargo.lock lists openssl as an unselected optional dependency of\n'
  printf '        dbus-secret-service (crypto-openssl). It is not compiled. Do not\n'
  printf '        "fix" this by grepping Cargo.lock; see the comment in this script.\n'
fi
