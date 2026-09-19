#!/usr/bin/env bash
# Linux cross-build; use a separate exported source tree from native builds.
set -euo pipefail
cd "$(dirname "$0")/.."
for tool in cargo cargo-xwin clang lld-link llvm-rc makensis node python3; do
  command -v "$tool" >/dev/null || { echo "Missing build tool: $tool" >&2; exit 1; }
done
target=x86_64-pc-windows-msvc
sidecars=(flamel_mcp git_mcp docs_mcp db_mcp browser_mcp memory_mcp)
args=()
for name in "${sidecars[@]}"; do args+=(--bin "$name"); done
python3 scripts/audit-source.py
cargo xwin build --locked --release --target "$target" -p flamel-engine "${args[@]}"
target_dir=$(cargo metadata --locked --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
mkdir -p desktop/resources/bin
# Refuse mixed platform payloads instead of silently shipping stale native tools.
for file in desktop/resources/bin/*; do
  [[ -e "$file" || -L "$file" ]] || continue
  if [[ -L "$file" || ! -f "$file" ]]; then
    echo "Unexpected resource entry: $file" >&2
    exit 1
  fi
  case "${file##*/}" in
    flamel_mcp.exe|git_mcp.exe|docs_mcp.exe|db_mcp.exe|browser_mcp.exe|memory_mcp.exe) ;;
    *) echo 'Use a clean Windows source export; unexpected bundled tool found.' >&2; exit 1 ;;
  esac
done
for name in "${sidecars[@]}"; do
  cp "$target_dir/$target/release/$name.exe" "desktop/resources/bin/$name.exe"
done
python3 scripts/dependency-notices.py desktop/resources/notices
npm run build:ui
cd desktop
cargo tauri build --ci --runner cargo-xwin --target "$target" \
  --config '{"build":{"beforeBuildCommand":""},"bundle":{"targets":["nsis"]}}' -- --locked --bin flamel
