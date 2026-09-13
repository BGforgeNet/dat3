# Using dat3-core as a library

The archive code is the `dat3-core` crate in this repository. It is not published on crates.io; depend on it
through git, pinned to a release tag:

```toml
[dependencies]
dat3-core = { git = "https://github.com/BGforgeNet/dat3", tag = "<release tag>" }
```

Its API may still change between releases. The optional `clap` feature derives `clap::ValueEnum` on
`ArchiveFormat`. The API documentation, with an example, builds with `cargo doc --no-deps --package dat3-core --open`
in a checkout, or with `cargo doc --open` in a project that depends on it.

## From Node.js and Electron

Releases also ship `dat3-wasm.tgz`, an npm package of the same library compiled to WebAssembly. It runs in Node and in
Electron's main process and workers on every platform, with TypeScript types included. Install it from the release:

```bash
npm install https://github.com/BGforgeNet/dat3/releases/download/<release tag>/dat3-wasm.tgz
```

It works on bytes, so the application reads and writes the files:

```ts
import { readFileSync, writeFileSync } from "node:fs";
import { Archive, type Entry } from "dat3-wasm";

const archive = Archive.fromBytes(readFileSync("patch000.dat"));
const entries: Entry[] = archive.entries();
for (const entry of entries) {
  console.log(entry.name, entry.size, entry.packedSize, entry.compressed); // names use "/"
}
const frm: Uint8Array = archive.read("art/critters/haenroaa.frm"); // any letter case
archive.insert("text/english/game/new.msg", readFileSync("new.msg"), 9); // compression 0-9
archive.remove("data/old.txt");
writeFileSync("patch000.dat", archive.toBytes());
archive.free(); // releases the archive's memory now rather than at garbage collection
```

`new Archive("dat2")` starts an empty archive (`"dat1"`, `"dat2"`, `"arcanum"` or `"toee"`). Names are looked up
regardless of case, preferring an exact match. Failures throw an `Error` with the reason. Calls run on the calling
thread, so run long operations on large archives in a worker to keep a window responsive.
