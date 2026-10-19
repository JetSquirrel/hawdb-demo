#!/usr/bin/env bash
set -euo pipefail

zig="${ZIG:-$(command -v zig)}"
args=()
for argument in "$@"; do
  if [[ "$argument" == "--target=wasm32-unknown-unknown" ]]; then
    args+=("-target" "wasm32-freestanding")
  else
    args+=("$argument")
  fi
done
exec "$zig" cc "${args[@]}"
