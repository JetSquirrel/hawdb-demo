#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

if [[ ! -f node_modules/binaryen/bin/wasm-opt ]]; then
  echo "Missing pinned Binaryen optimizer. Run npm ci first." >&2
  exit 1
fi

if [[ "$(uname -s)" == "Darwin" ]]; then
  host="$(rustc +nightly -vV | sed -n 's/^host: //p')"
  rust_llvm_ar="$(rustc +nightly --print sysroot)/lib/rustlib/$host/bin/llvm-ar"

  if [[ -z "${AR_wasm32_unknown_unknown:-}" ]]; then
    for candidate in "$rust_llvm_ar" /opt/homebrew/opt/llvm/bin/llvm-ar /usr/local/opt/llvm/bin/llvm-ar; do
      if [[ -x "$candidate" ]]; then
        export AR_wasm32_unknown_unknown="$candidate"
        break
      fi
    done
  fi

  if [[ -z "${CC_wasm32_unknown_unknown:-}" ]]; then
    for candidate in /opt/homebrew/opt/llvm/bin/clang /usr/local/opt/llvm/bin/clang; do
      if [[ -x "$candidate" ]] && "$candidate" --print-targets 2>/dev/null | grep -qi WebAssembly; then
        export CC_wasm32_unknown_unknown="$candidate"
        break
      fi
    done
  fi

  if command -v zig >/dev/null 2>&1; then
    if [[ -z "${CC_wasm32_unknown_unknown:-}" ]]; then
      chmod +x "$root/scripts/zig-wasm-cc.sh"
      export CC_wasm32_unknown_unknown="$root/scripts/zig-wasm-cc.sh"
    fi
    if [[ -z "${AR_wasm32_unknown_unknown:-}" ]]; then
      chmod +x "$root/scripts/zig-wasm-ar.sh"
      export AR_wasm32_unknown_unknown="$root/scripts/zig-wasm-ar.sh"
    fi
  fi

  if [[ -z "${CC_wasm32_unknown_unknown:-}" || -z "${AR_wasm32_unknown_unknown:-}" ]]; then
    echo "No C compiler and archiver suitable for WebAssembly were found." >&2
    echo "Install Homebrew LLVM (brew install llvm) or Zig (brew install zig)." >&2
    exit 1
  fi
fi

cargo +nightly build --locked --release --target wasm32-unknown-unknown
# Preserve the previous working bundle if binding generation fails.
staging="$(mktemp -d "$root/.pkg-build.XXXXXX")"
trap 'rm -rf "$staging"' EXIT
wasm-bindgen \
  --target web \
  --out-dir "$staging" \
  --out-name hawdb_wasm_demo \
  target/wasm32-unknown-unknown/release/hawdb_wasm_demo.wasm
node scripts/optimize-wasm.mjs "$staging/hawdb_wasm_demo_bg.wasm"
mkdir -p pkg
cp "$staging"/* pkg/

printf 'Built %s\n' "$root/pkg/hawdb_wasm_demo_bg.wasm"
