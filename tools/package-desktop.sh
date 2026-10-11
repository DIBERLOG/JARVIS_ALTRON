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
    export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-rpath,\$ORIGIN/../lib/jarvis-altron"
    export LD_LIBRARY_PATH="$VOSK_LIBRARY_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    ;;
  Darwin)
    native=libvosk.dylib; bundles=app,dmg
    # Vosk 0.3.42's universal2 wheel names its Mach-O library libvosk.dyld.
    if [[ ! -f "$VOSK_LIBRARY_DIR/$native" && -f "$VOSK_LIBRARY_DIR/libvosk.dyld" ]]; then
      file "$VOSK_LIBRARY_DIR/libvosk.dyld" | grep -q 'Mach-O' || { echo 'Vosk library is not a macOS Mach-O binary.' >&2; exit 1; }
      mkdir -p target/vosk-link
      cp "$VOSK_LIBRARY_DIR/libvosk.dyld" "target/vosk-link/$native"
      export VOSK_LIBRARY_DIR="$repo/target/vosk-link"
      install_name_tool -id '@rpath/libvosk.dylib' "$VOSK_LIBRARY_DIR/$native"
    fi
    export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=-Wl,-rpath,@executable_path/../Frameworks"
    export DYLD_FALLBACK_LIBRARY_PATH="$VOSK_LIBRARY_DIR${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}"
    ;;
  *) echo 'Linux/macOS native build required.' >&2; exit 1 ;;
esac
[[ -f "$VOSK_LIBRARY_DIR/$native" ]] || { echo "Missing $native for $host" >&2; exit 1; }
# ort's bundled archive excludes Intel Macs; use Microsoft's matching native runtime.
extra_config=()
if [[ "$host" == x86_64-apple-darwin ]]; then
  runtime=onnxruntime-osx-x86_64-1.23.2
  mkdir -p target/onnxruntime target/release
  curl --fail --location --retry 3 "https://github.com/microsoft/onnxruntime/releases/download/v1.23.2/$runtime.tgz" -o "target/onnxruntime/$runtime.tgz"
  echo "d10359e16347b57d9959f7e80a225a5b4a66ed7d7e007274a15cae86836485a6  target/onnxruntime/$runtime.tgz" | shasum -a 256 -c -
  tar -xzf "target/onnxruntime/$runtime.tgz" -C target/onnxruntime
  export ORT_LIB_LOCATION="$repo/target/onnxruntime/$runtime/lib"
  export ORT_PREFER_DYNAMIC_LINK=1
  install_name_tool -id '@rpath/libonnxruntime.1.23.2.dylib' "$ORT_LIB_LOCATION/libonnxruntime.1.23.2.dylib"
  export DYLD_FALLBACK_LIBRARY_PATH="$ORT_LIB_LOCATION:$DYLD_FALLBACK_LIBRARY_PATH"
  cp -L "$ORT_LIB_LOCATION/libonnxruntime.1.23.2.dylib" target/release/libonnxruntime.1.23.2.dylib
  extra_config=(--config '{"bundle":{"resources":{"../../target/onnxruntime/onnxruntime-osx-x86_64-1.23.2/LICENSE":"third_party/licenses/ONNX-Runtime-LICENSE.txt","../../target/onnxruntime/onnxruntime-osx-x86_64-1.23.2/ThirdPartyNotices.txt":"third_party/licenses/ONNX-Runtime-ThirdPartyNotices.txt"},"macOS":{"frameworks":["../../target/release/libvosk.dylib","../../target/release/libonnxruntime.1.23.2.dylib"]}}}')
fi
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
../../frontend/node_modules/.bin/tauri build --bundles "$bundles" "${extra_config[@]}"
echo "Packages: $repo/target/release/bundle"
