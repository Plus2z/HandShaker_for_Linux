#!/usr/bin/env bash
set -euo pipefail

# Work around WebKitGTK Wayland DMABUF/GBM blank-surface issues on proprietary NVIDIA drivers
# unless the user already configured WEBKIT_DISABLE_DMABUF_RENDERER explicitly.
if [[ -z "${WEBKIT_DISABLE_DMABUF_RENDERER+x}" ]]; then
  if [[ "${HANDSHAKER_DISABLE_DMABUF:-0}" == "1" ]] || [[ -e /proc/driver/nvidia/version ]]; then
    export WEBKIT_DISABLE_DMABUF_RENDERER=1
  fi
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -x "${SCRIPT_DIR}/handshaker-desktop" ]]; then
  exec "${SCRIPT_DIR}/handshaker-desktop" "$@"
elif command -v handshaker-desktop >/dev/null 2>&1; then
  exec handshaker-desktop "$@"
else
  echo "error: handshaker-desktop binary not found in ${SCRIPT_DIR} or PATH" >&2
  exit 127
fi
