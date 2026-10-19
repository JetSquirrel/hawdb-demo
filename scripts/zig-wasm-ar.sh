#!/usr/bin/env bash
set -euo pipefail
exec "${ZIG:-$(command -v zig)}" ar "$@"
