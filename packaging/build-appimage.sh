#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

if ! command -v pnpm >/dev/null 2>&1; then
  echo "error: pnpm is required" >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo is required" >&2
  exit 1
fi

echo "[1/3] Building frontend production bundle..."
pnpm run build

echo "[2/3] Building CLI + FUSE binaries..."
cargo build --release -p handshaker-proto -p handshaker-fuse

echo "[3/3] Building Tauri desktop AppImage..."
if ! pnpm tauri build --bundles appimage; then
  cat >&2 <<'EOF'
hint: AppImage bundling requires linuxdeploy / appimagetool (which Tauri normally caches in ~/.cache/tauri
when online). You can alternatively build a standalone release binary with:
  cargo build --release -p handshaker-desktop --features custom-protocol
EOF
  exit 1
fi
