#!/usr/bin/env bash
# Stage the exact synthetic request named by the prompt.
set -Eeuo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cp "$here/compatibility.json" .
