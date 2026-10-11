#!/usr/bin/env bash
# Native build only: run on the target OS and architecture, never as root.
set -euo pipefail
repo="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo"
if [[ -z "${VOSK_LIBRARY_DIR:-}" ]]; then
  # Download, but do not install Python packages or alter the system Python.
  version=0.3.45
  [[ "$(uname -s)" != Darwin ]] || version=0.3.42
  python3 -m pip download --no-deps --only-binary=:all: "vosk==$version" -d "target/vosk-wheel-$version"
  wheel="$(find "target/vosk-wheel-$version" -name '*.whl' -print -quit)"
  python3 -m zipfile -e "$wheel" "target/vosk-native-$version"
  export VOSK_LIBRARY_DIR="$repo/target/vosk-native-$version/vosk"
fi
[[ -d "$VOSK_LIBRARY_DIR" ]] || { echo 'VOSK_LIBRARY_DIR does not exist.' >&2; exit 1; }
export VOSK_LIBRARY_DIR="$(cd "$VOSK_LIBRARY_DIR" && pwd)"
host="$(rustc -vV | sed -n 's/^host: //p')"
case "$(uname -s)" in
  Linux)
    native=libvosk.so; bundles=deb,appimage
    export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-rpath,\$ORIGIN/../lib/jarvis-gui"
    export LD_LIBRARY_PATH="$VOSK_LIBRARY_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    ;;
  Darwin)
    native=libvosk.dylib; bundles=app,dmg
    # The universal2 Python wheel ships a Mach-O dylib named libvosk.so.
    if [[ ! -f "$VOSK_LIBRARY_DIR/$native" && -f "$VOSK_LIBRARY_DIR/libvosk.so" ]]; then
      file "$VOSK_LIBRARY_DIR/libvosk.so" | grep -q 'Mach-O' || { echo 'Vosk library is not a macOS Mach-O binary.' >&2; exit 1; }
      mkdir -p target/vosk-link
      cp "$VOSK_LIBRARY_DIR/libvosk.so" "target/vosk-link/$native"
      export VOSK_LIBRARY_DIR="$repo/target/vosk-link"
      install_name_tool -id '@rpath/libvosk.dylib' "$VOSK_LIBRARY_DIR/$native"
    fi
    export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-rpath,@executable_path/../Frameworks"
    export DYLD_FALLBACK_LIBRARY_PATH="$VOSK_LIBRARY_DIR${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}"
    ;;
  *) echo 'Linux/macOS native build required.' >&2; exit 1 ;;
esac
[[ -f "$VOSK_LIBRARY_DIR/$native" ]] || { echo "Missing $native for $host" >&2; exit 1; }
npm --prefix frontend ci
cargo build --locked --release -p jarvis-app
case "$host" in
  x86_64-unknown-linux-gnu) recorder_platform=linux/x86_64; recorder=libpv_recorder.so ;;
  x86_64-apple-darwin) recorder_platform=mac/x86_64; recorder=libpv_recorder.dylib ;;
  aarch64-apple-darwin) recorder_platform=mac/arm64; recorder=libpv_recorder.dylib ;;
  *) echo "Installer recipe not validated for $host" >&2; exit 1 ;;
esac
recorder_source="$(find target/release/build -path "*/out/lib/$recorder_platform/$recorder" -print -quit)"
[[ -n "$recorder_source" ]] || { echo 'Native PvRecorder missing from Cargo output.' >&2; exit 1; }
cp "$recorder_source" "target/release/$recorder"
cp "target/release/jarvis-app" "target/release/jarvis-app-$host"
cp "$VOSK_LIBRARY_DIR/$native" "target/release/$native"
if [[ "$native" == libvosk.dylib ]]; then
  # Avoid an absolute developer-machine dependency in the bundled core.
  dependency="$(otool -L target/release/jarvis-app | awk '/libvosk/{print $1; exit}')"
  [[ -n "$dependency" ]] || { echo 'Core does not link Vosk as expected.' >&2; exit 1; }
  install_name_tool -change "$dependency" '@rpath/libvosk.dylib' "target/release/jarvis-app-$host"
  install_name_tool -id '@rpath/libvosk.dylib' target/release/libvosk.dylib
fi
cd crates/jarvis-gui
../../frontend/node_modules/.bin/tauri build --bundles "$bundles"
echo "Packages: $repo/target/release/bundle"
