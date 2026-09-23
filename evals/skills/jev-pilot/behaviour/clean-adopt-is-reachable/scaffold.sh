#!/usr/bin/env bash
# Copy this case's subject into the agent's working directory.
set -Eeuo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cp -R "$here/../../fixtures/beacon" .
