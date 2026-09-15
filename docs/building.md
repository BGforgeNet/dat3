# Building dat3

## Requirements

- Rust 1.87 or newer
- Target-specific toolchains (install as needed)
- `./install-tools.sh` for the pinned tooling, including [Zig](https://ziglang.org/), which the aarch64
  target needs: mimalloc is C, and no aarch64-musl C compiler is packaged for common distros. It also links the
  macOS build
- Node 24 or newer, to run the integration suite (`./test.sh`): its helpers under `tests/` are TypeScript, run
  by Node's own type stripping. `npm ci && npm run typecheck` typechecks them. Neither is needed to build dat3

## Build

```bash
./build.sh
```

Builds are static, except for macOS, where every program links the system library dynamically.

Binaries will be at:

```bash
target/x86_64-unknown-linux-musl/release/dat3
target/aarch64-unknown-linux-musl/release/dat3
target/universal2-apple-darwin/release/dat3
target/x86_64-pc-windows-gnu/release/dat3.exe
target/i686-pc-windows-gnu/release/dat3.exe
target/wasm32-wasip1/release/dat3.wasm
```

The npm package for Node and Electron is at `target/dat3-wasm.tgz`; `crates/dat3-wasm/package.sh` builds it alone.
