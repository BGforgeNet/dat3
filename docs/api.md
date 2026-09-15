# Using dat3-core as a library

The archive code is the `dat3-core` crate in this repository. It is not published on crates.io; depend on it
through git, pinned to a release tag. Its errors are `anyhow::Error`:

```toml
[dependencies]
dat3-core = { git = "https://github.com/BGforgeNet/dat3", tag = "<release tag>" }
anyhow = "1"
```

Its API may still change between releases. The optional `clap` feature derives `clap::ValueEnum` on
`ArchiveFormat`. The API documentation, which includes this page, builds with
`cargo doc --no-deps --package dat3-core --open` in a checkout, or with `cargo doc --open` in a project that depends
on it.

## From Rust

Files on disk work as the `dat3` command line does, and print what it prints. The in-memory methods work on bytes,
touch no filesystem and print nothing:

```rust no_run
use dat3_core::common::{CaseMode, CompressionLevel, ExtractionMode, MissingFiles, Selection};
use dat3_core::{ArchiveFormat, DatArchive};

fn main() -> anyhow::Result<()> {
    // Build a Fallout 2 archive from the `art` directory
    let mut archive = DatArchive::new(ArchiveFormat::Dat2);
    archive.add_file("art", CompressionLevel::new(9)?, None, None, CaseMode::Insensitive)?;
    archive.save("patch000.dat")?;

    // Open it again and extract the FRM files, keeping their directories
    let archive = DatArchive::open("patch000.dat")?;
    let patterns = ["*.frm".to_string()];
    let selection = Selection {
        patterns: &patterns,
        on_missing: MissingFiles::Fail,
        case: CaseMode::Insensitive,
    };
    archive.extract("out", ExtractionMode::PreserveStructure, &selection)?;

    // The same archive in memory
    let mut archive = DatArchive::from_bytes(std::fs::read("patch000.dat")?)?;
    println!("{}", archive.format().arg_name()); // "dat2", detected from the bytes
    for entry in archive.entries() {
        // names as stored, with "\" separators
        println!("{} {} {} {}", entry.name, entry.size, entry.packed_size, entry.compressed);
    }
    let frm = archive.read("art/critters/haenroaa.frm", CaseMode::Insensitive)?; // "/" or "\"
    println!("{} bytes", frm.len());
    let msg = std::fs::read("new.msg")?;
    archive.insert("text/english/game/new.msg", msg, CompressionLevel::new(9)?, CaseMode::Insensitive)?;
    let removed: bool = archive.remove("data/old.txt", CaseMode::Insensitive)?;
    println!("removed: {removed}");
    std::fs::write("patch000.dat", archive.to_bytes()?)?;
    Ok(())
}
```

`CaseMode::Insensitive` matches names regardless of case, as the games do, preferring an exact match;
`CaseMode::Sensitive` uses names exactly as stored.

## From Node.js and Electron

Releases also ship `dat3-wasm.tgz`, an npm package of the same library compiled to WebAssembly. It runs in Node and in
Electron's main process and workers on every platform, with TypeScript types included. Install it from the release:

```bash
npm install https://github.com/BGforgeNet/dat3/releases/download/<release tag>/dat3-wasm.tgz
```

It works on bytes, so the application reads and writes the files:

```ts
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { Archive, type Entry } from "dat3-wasm";

// Open an archive. `using` frees its memory at the end of the block; where `using` is unavailable, call
// archive.free() when done, or the memory is released only at garbage collection.
{
  using archive = Archive.fromBytes(readFileSync("patch000.dat"));
  console.log(archive.format); // "dat2", detected from the bytes

  const entries: Entry[] = archive.entries();
  for (const entry of entries) {
    console.log(entry.name, entry.size, entry.packedSize, entry.compressed); // names use "/"
  }

  // Extract every file, keeping its directories. Names come from the archive, so refuse any that would land
  // outside the output directory.
  const outDir = path.resolve("out");
  for (const { name } of entries) {
    const target = path.resolve(outDir, name);
    if (!target.startsWith(outDir + path.sep)) {
      throw new Error(`refusing to extract ${name} outside ${outDir}`);
    }
    mkdirSync(path.dirname(target), { recursive: true });
    writeFileSync(target, archive.read(name));
  }

  const frm: Uint8Array = archive.read("art/critters/haenroaa.frm"); // any letter case
  console.log(frm.length);
  archive.insert("text/english/game/new.msg", readFileSync("new.msg"), 9); // compression 0-9
  const removed: boolean = archive.remove("data/old.txt");
  console.log(removed);
  writeFileSync("patch000.dat", archive.toBytes());
}

// Build a new archive from a directory tree
{
  using built = new Archive("arcanum"); // "dat1", "dat2", "arcanum" or "toee"
  for (const file of readdirSync("mod", { recursive: true, withFileTypes: true })) {
    if (file.isFile()) {
      const source = path.join(file.parentPath, file.name);
      built.insert(path.relative("mod", source), readFileSync(source), 9); // "/" or "\" separators
    }
  }
  writeFileSync("mod.dat", built.toBytes());
}

// Failures throw an Error carrying the reason
try {
  Archive.fromBytes(new Uint8Array([1, 2, 3]));
} catch (error) {
  console.error(error instanceof Error ? error.message : error);
}
```

Names are looked up regardless of case, preferring an exact match. Calls run on the calling thread, so run long
operations on large archives in a worker to keep a window responsive.
