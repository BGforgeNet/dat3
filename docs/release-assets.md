# Release assets

Each [release](https://github.com/BGforgeNet/dat3/releases) ships these files:

| File | What it is |
| --- | --- |
| `dat3` | Linux x86_64, statically linked |
| `dat3-arm64` | Linux arm64, statically linked |
| `dat3-macos` | macOS universal binary, native on Intel and Apple Silicon |
| `dat3.exe` | Windows x86_64 |
| `dat3-win32.exe` | Windows 32-bit |
| `dat3.wasm` | The same command line as WebAssembly, for systems with no native build ([below](#dat3wasm)) |
| `dat3-wasm.tgz` | npm package of the library for Node.js and Electron, not a command line ([docs/api.md](api.md)) |
| `SHA256SUMS` | Checksums of every file above ([verifying a release](#verifying-a-release)) |

`dat3-macos` is not notarized, so a copy downloaded in a browser has to be allowed in System Settings, or cleared
with `xattr -d com.apple.quarantine dat3-macos`, before its first run.

## dat3.wasm

The native builds cover the common systems. `dat3.wasm` is for everything else, such as FreeBSD and the other BSDs,
or Linux on a CPU other than x86_64 and arm64: it is the same program compiled to WebAssembly (WASI), and runs
wherever a WASI runtime does. [wasmtime](https://wasmtime.dev) is one; on FreeBSD it is the `www/wasmtime` port.

A WASI program sees only the directories it is given, so grant the ones holding the archive and the files it
reads or writes with `--dir`:

```bash
wasmtime run --dir . dat3.wasm l patch000.dat
wasmtime run --dir . dat3.wasm x patch000.dat -o out
```

Every path must lie inside a granted directory. A path outside them fails with "failed to find a pre-opened file
descriptor". To extract somewhere else, grant that directory too:

```bash
wasmtime run --dir . --dir /path/to/output dat3.wasm x patch000.dat -o /path/to/output
```

WebAssembly has no threads here, so extraction runs on one core and a large archive takes longer than with a
native build.

## Verifying a release

Download `SHA256SUMS` alongside the assets and check them. `--ignore-missing` checks only the files you
downloaded:

```bash
sha256sum -c --ignore-missing SHA256SUMS
```

On macOS:

```bash
shasum -a 256 -c --ignore-missing SHA256SUMS
```
