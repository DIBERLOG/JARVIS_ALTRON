#!/usr/bin/env bash
set -euo pipefail
repo="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
build="$repo/target/release"
export JARVIS_RESOURCE_ROOT="${JARVIS_RESOURCE_ROOT:-$build}"
if [[ -n "${VOSK_LIBRARY_DIR:-}" ]]; then
  export LD_LIBRARY_PATH="$VOSK_LIBRARY_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  export DYLD_FALLBACK_LIBRARY_PATH="$VOSK_LIBRARY_DIR${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}"
fi
[[ -x "$build/jarvis-app-sync-v2-lab" && -x "$build/jarvis-gui-sync-v2-lab" ]] || { echo 'Build both executables first.' >&2; exit 1; }
cd "$build"
"$build/jarvis-app-sync-v2-lab" &
core_pid=$!
trap 'kill "$core_pid" 2>/dev/null || true' EXIT
"$build/jarvis-gui-sync-v2-lab"
