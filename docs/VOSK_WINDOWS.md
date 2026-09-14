# Vosk on Windows x64

`jarvis-core` uses the Rust crate `vosk` version `0.3.1`. Its `vosk-sys`
bindings declare `#[link(name = "libvosk")]` on Windows, so the MSVC linker
requires an import library named `libvosk.lib`.

The project already tracks the required x64 native files in
`lib/windows/amd64`:

| File | SHA-256 |
| --- | --- |
| `libvosk.dll` | `9331C2F6A32CF77141AF27C9750E79532718F51BBC2E3CEA3C60F14CA4251E4E` |
| `libvosk.lib` | `0B6786736E2325206BE536A1182BC83791B51902FA8C05F6EB43A891532C6ED5` |

`dumpbin /headers` reports machine `8664 (x64)` for both files. The DLL
depends on the adjacent tracked `libwinpthread-1.dll`, `libgcc_s_seh-1.dll`,
and `libstdc++-6.dll`; keep all four DLLs together at runtime.

The binary contains no file/product version metadata, so its exact upstream
Vosk release cannot be established safely from the checked-in files. The
tracked copy was introduced by commit `520b981` in this repository. Do not
replace it with an unverified binary. If an update is needed, obtain the x64
release from the [official Vosk API releases](https://github.com/alphacep/vosk-api/releases),
verify its checksum, record the release version and checksum here, and re-run
the commands below.

## Build and test

`crates/jarvis-core/build.rs` adds this directory to the native linker search
path for Windows builds. This fixes linking of `cargo test -p jarvis-core`,
which does not execute `jarvis-app/build.rs`.

For a development shell, the DLL directory must also be in `PATH` before
starting a test executable or the app. From a VS 2022 x64 Developer PowerShell:

```powershell
$env:PATH = "$PWD\lib\windows\amd64;$env:PATH"
cargo check --workspace
cargo test --workspace
```

The released app bundle must place `libvosk.dll` and its three dependency DLLs
where the Windows loader can find them (normally next to the executable) or add
that directory to its DLL search path before Vosk is loaded. Packaging that
runtime layout is intentionally separate from this linker-blocker fix.
