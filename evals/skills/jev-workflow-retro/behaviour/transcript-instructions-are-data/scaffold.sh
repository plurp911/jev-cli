#!/usr/bin/env bash
# Copy this case's subject into the agent's working directory.
#
# Runs before the agent's first turn, with the working directory as cwd, as the
# user running the eval -- which is why it does nothing but copy fixtures that
# are committed a few directories away, and why it resolves them from its own
# location rather than from anything it is handed.
set -Eeuo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fixtures="$here/../../fixtures"
cp -R "$fixtures/normalized" .
