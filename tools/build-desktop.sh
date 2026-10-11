#!/usr/bin/env bash
set -euo pipefail
repo="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
case "$(uname -s)" in
  Linux|Darwin) ;;
  *) echo 'This script builds Linux/macOS desktop sources only.' >&2; exit 1 ;;
esac
if [[ -z "${VOSK_LIBRARY_DIR:-}" || ! -d "$VOSK_LIBRARY_DIR" ]]; then
  echo 'Set VOSK_LIBRARY_DIR to the directory containing the native Vosk library for your OS/architecture.' >&2
  exit 1
fi
cd "$repo/frontend"
npm ci
npm run build
cd "$repo"
cargo build --locked --release -p jarvis-app -p jarvis-gui
if [[ -d "$repo/target/release/resources" ]]; then
  echo 'Existing release resources kept. Update them manually after backing up your configuration.'
else
  cp -R "$repo/resources" "$repo/target/release/resources"
fi
echo "Built in $repo/target/release. Native libraries/models remain separate prerequisites."
