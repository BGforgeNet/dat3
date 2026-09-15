/**
 * Integration test for the dat3-wasm npm package: dat3-core for Node and Electron.
 *
 * Installs the packed tarball into a scratch project, as an application would, and
 * checks the library against the native dat3 on the fixture archives the other
 * tests fetch, and the Node example in docs/api.md against the package. Run by
 * wasm_library.sh after the package build; typechecked by `npm run typecheck`, and
 * needs that install (`npm ci`) to typecheck the example.
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync, type SpawnSyncReturns } from "node:child_process";
import { createRequire } from "node:module";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { parseListing } from "./listing.ts";

const TESTS_DIR = import.meta.dirname;
const WORK_DIR = path.join(TESTS_DIR, "test_wasm_library");
const TARBALL = path.join(TESTS_DIR, "..", "target", "dat3-wasm.tgz");
const DAT3 = path.join(TESTS_DIR, "..", "target", "x86_64-unknown-linux-musl", "release", "dat3");
const API_DOC = path.join(TESTS_DIR, "..", "docs", "api.md");
/** Run through this Node rather than a `tsc` looked up on PATH */
const TSC = path.join(TESTS_DIR, "..", "node_modules", "typescript", "bin", "tsc");

/** Bound on each child process: a sync spawn blocks the runner, so its own timeout is the only one */
const SPAWN_TIMEOUT_MS = 120_000;
/** Room for a fixture's JSON listing, which passes spawnSync's 1 MiB default */
const SPAWN_MAX_BUFFER = 64 * 1024 * 1024;

const FORMATS = ["dat1", "dat2", "arcanum", "toee"] as const;

/** Real archives of every format, one with compressed entries each, fetched by the shell tests */
const FIXTURES = [
    { file: "FalloutDemo.dat", format: "dat1" },
    { file: "rpu.dat", format: "dat2" },
    { file: "ArcanumDemo.dat", format: "arcanum" },
    { file: "tpgamefiles.dat", format: "toee" },
] as const;

/**
 * The package API as these tests use it. The generated dat3_wasm.d.ts is the real
 * declaration, but it exists only after a build, and the typecheck runs without one.
 */
interface Entry {
    name: string;
    size: number;
    packedSize: number;
    compressed: boolean;
}

interface Archive {
    readonly format: string;
    entries(): Entry[];
    read(name: string): Uint8Array;
    insert(name: string, data: Uint8Array, compression?: number | null): void;
    remove(name: string): boolean;
    toBytes(): Uint8Array;
    free(): void;
}

interface Dat3Wasm {
    Archive: {
        new (format: string): Archive;
        fromBytes(bytes: Uint8Array): Archive;
    };
}

/** Run a command, failing only if it could not be started or timed out */
function spawn(command: string, args: string[], cwd: string): SpawnSyncReturns<string> {
    const result = spawnSync(command, args, {
        cwd,
        encoding: "utf8",
        timeout: SPAWN_TIMEOUT_MS,
        maxBuffer: SPAWN_MAX_BUFFER,
    });
    if (result.error) {
        throw result.error;
    }
    return result;
}

/** Run a command that must succeed, returning its stdout */
function run(command: string, args: string[], cwd: string = WORK_DIR): string {
    const result = spawn(command, args, cwd);
    assert.equal(result.status, 0, `${command} ${args.join(" ")} failed:\n${result.stdout}${result.stderr}`);
    return result.stdout;
}

/** The native listing, in the library's field names */
function nativeEntries(archive: string): Entry[] {
    const listing = parseListing(run(DAT3, ["l", "--json", "--case-sensitive", archive]));
    return listing.map((entry) => ({
        name: entry.name,
        size: entry.size,
        packedSize: entry.packed_size,
        compressed: entry.compressed,
    }));
}

function isDat3Wasm(value: unknown): value is Dat3Wasm {
    if (typeof value !== "object" || value === null || !("Archive" in value)) {
        return false;
    }
    const archive: unknown = value.Archive;
    return typeof archive === "function" && "fromBytes" in archive && typeof archive.fromBytes === "function";
}

rmSync(WORK_DIR, { recursive: true, force: true });
mkdirSync(path.join(WORK_DIR, "src"), { recursive: true });
writeFileSync(path.join(WORK_DIR, "package.json"), JSON.stringify({ name: "consumer", private: true }));
// --offline: the package has no dependencies, so installing it must not need the network
run("npm", ["install", "--offline", "--no-audit", "--no-fund", TARBALL]);
const loaded: unknown = createRequire(path.join(WORK_DIR, "package.json"))("dat3-wasm");
assert.ok(isDat3Wasm(loaded), "dat3-wasm does not export the Archive class");
const { Archive } = loaded;

const BIG = Buffer.from("frame ".repeat(500));
const TINY = Buffer.from("hi\n");

for (const { file, format } of FIXTURES) {
    test(`${format}: ${file} lists and reads exactly as the native dat3 extracts it`, () => {
        const archivePath = path.join(TESTS_DIR, file);
        const archive = Archive.fromBytes(readFileSync(archivePath));
        try {
            assert.equal(archive.format, format);
            const entries = archive.entries();
            assert.deepEqual(entries, nativeEntries(archivePath));
            assert.ok(
                entries.some((entry) => entry.compressed),
                `${file} has no compressed entry, so decompression goes untested`,
            );

            const outDir = path.join(WORK_DIR, `native_${format}`);
            run(DAT3, ["x", "--case-sensitive", archivePath, "-o", outDir]);
            const differing = entries
                .filter((entry) => {
                    const native = readFileSync(path.join(outDir, entry.name));
                    return Buffer.compare(archive.read(entry.name), native) !== 0;
                })
                .map((entry) => entry.name);
            assert.deepEqual(differing, []);
        } finally {
            archive.free();
        }
    });
}

for (const format of FORMATS) {
    test(`${format}: an archive built in JS reads back, and the native dat3 reads it too`, () => {
        const archive = new Archive(format);
        archive.insert("art/Hero.FRM", BIG, 9);
        archive.insert("sub\\tiny.txt", TINY);
        const bytes = archive.toBytes();
        archive.free();

        const file = path.join(WORK_DIR, `built_${format}.dat`);
        writeFileSync(file, bytes);
        assert.deepEqual(
            nativeEntries(file)
                .map((entry) => entry.name)
                .sort(),
            ["art/Hero.FRM", "sub/tiny.txt"],
        );

        const reopened = Archive.fromBytes(bytes);
        assert.deepEqual(Buffer.from(reopened.read("ART/hero.frm")), BIG);
        assert.deepEqual(Buffer.from(reopened.read("sub/tiny.txt")), TINY);
        reopened.free();
    });
}

test("entries are plain objects that survive JSON and structured cloning", () => {
    const archive = new Archive("dat2");
    archive.insert("a.txt", TINY);
    const entries = archive.entries();
    archive.free();
    const expected = [{ name: "a.txt", size: TINY.length, packedSize: TINY.length, compressed: false }];
    assert.deepEqual(entries, expected);
    assert.deepEqual(JSON.parse(JSON.stringify(entries)), expected);
    assert.deepEqual(structuredClone(entries), expected);
});

test("insert replaces a name in any case, and remove reports whether it removed", () => {
    const archive = new Archive("dat2");
    archive.insert("Data/A.txt", Buffer.from("old"));
    archive.insert("data/a.TXT", Buffer.from("new"));
    assert.deepEqual(
        archive.entries().map((entry) => entry.name),
        ["data/a.TXT"],
    );
    assert.equal(Buffer.from(archive.read("DATA/A.TXT")).toString(), "new");
    assert.equal(archive.remove("data/a.txt"), true);
    assert.equal(archive.remove("data/a.txt"), false);
    assert.deepEqual(archive.entries(), []);
    archive.free();
});

test("names differing only in case stay reachable by their exact spelling", () => {
    writeFileSync(path.join(WORK_DIR, "src", "README.TXT"), "upper");
    writeFileSync(path.join(WORK_DIR, "src", "readme.txt"), "lower");
    run(DAT3, ["a", "--case-sensitive", "twins.dat", "-C", "src", "README.TXT", "readme.txt"]);

    const archive = Archive.fromBytes(readFileSync(path.join(WORK_DIR, "twins.dat")));
    assert.equal(Buffer.from(archive.read("README.TXT")).toString(), "upper");
    assert.equal(Buffer.from(archive.read("readme.txt")).toString(), "lower");
    assert.throws(() => archive.read("Readme.txt"), {
        name: "Error",
        message: "Readme.txt matches entries that differ only in case; name one exactly: README.TXT / readme.txt",
    });
    archive.free();
});

test("failures throw an Error carrying the reason", () => {
    assert.throws(() => new Archive("zip"), {
        name: "Error",
        message: 'unsupported archive format "zip" (expected dat1, dat2, arcanum, or toee)',
    });
    assert.throws(() => Archive.fromBytes(new Uint8Array([1, 2, 3])), {
        name: "Error",
        message: "DAT2 file too small",
    });

    const archive = new Archive("dat2");
    assert.throws(() => archive.read("missing.txt"), { name: "Error", message: "File not found: missing.txt" });
    assert.throws(() => archive.insert("../escape.txt", TINY), {
        name: "Error",
        message: /^Invalid archive path for add operation \('\.\.' component\)/,
    });
    assert.throws(() => archive.insert("a.txt", TINY, 10), {
        name: "Error",
        message: "Compression level must be 0-9, got 10",
    });
    assert.deepEqual(archive.entries(), []);
    archive.free();
});

// The Node example in docs/api.md, checked so it cannot drift from the package it documents

const FRM = Buffer.from("frm bytes\n");
const NEW_MSG = Buffer.from("{100}{}{New message}\n");

/** The one `ts` code block in docs/api.md, verbatim */
function apiDocExample(): string {
    const blocks = [...readFileSync(API_DOC, "utf8").matchAll(/^```ts\n([\s\S]*?)^```$/gm)];
    assert.equal(blocks.length, 1, "docs/api.md should hold exactly one ts code block");
    const example = blocks[0]?.[1];
    assert.ok(example !== undefined);
    return example;
}

/** Archive bytes holding `entries`, built with the library itself */
function dat2With(entries: Record<string, Buffer>): Uint8Array {
    const archive = new Archive("dat2");
    for (const [name, data] of Object.entries(entries)) {
        archive.insert(name, data);
    }
    const bytes = archive.toBytes();
    archive.free();
    return bytes;
}

/**
 * A directory holding the example and every file it names, with `patch` as its patch000.dat. Its
 * package.json makes the example an ES module; dat3-wasm resolves from WORK_DIR's node_modules.
 */
function docExampleDir(name: string, patch: Uint8Array): string {
    const dir = path.join(WORK_DIR, name);
    rmSync(dir, { recursive: true, force: true });
    mkdirSync(path.join(dir, "mod", "sub"), { recursive: true });
    writeFileSync(path.join(dir, "package.json"), JSON.stringify({ private: true, type: "module" }));
    writeFileSync(path.join(dir, "example.ts"), apiDocExample());
    writeFileSync(path.join(dir, "patch000.dat"), patch);
    writeFileSync(path.join(dir, "new.msg"), NEW_MSG);
    writeFileSync(path.join(dir, "mod", "a.txt"), "one\n");
    writeFileSync(path.join(dir, "mod", "sub", "b.txt"), "two\n");
    return dir;
}

test("the docs/api.md Node example typechecks against the package's own declarations", () => {
    assert.ok(existsSync(TSC), `${TSC} is missing; npm ci installs it`);
    const dir = docExampleDir("doc_typecheck", new Uint8Array());
    writeFileSync(
        path.join(dir, "tsconfig.json"),
        JSON.stringify({ extends: path.join(TESTS_DIR, "tsconfig.json"), include: ["example.ts"] }),
    );
    run(process.execPath, [TSC, "--noEmit", "-p", "tsconfig.json"], dir);
});

test("the docs/api.md Node example does what its comments say", () => {
    const patch = dat2With({ "art/critters/haenroaa.frm": FRM, "data/old.txt": TINY });
    const dir = docExampleDir("doc_example", patch);
    const result = spawn(process.execPath, ["example.ts"], dir);
    assert.equal(result.status, 0, result.stdout + result.stderr);
    assert.match(result.stdout, /^dat2$/m);
    assert.match(result.stderr, /DAT2 file too small/);

    assert.deepEqual(readFileSync(path.join(dir, "out", "art", "critters", "haenroaa.frm")), FRM);
    assert.deepEqual(readFileSync(path.join(dir, "out", "data", "old.txt")), TINY);

    const saved = Archive.fromBytes(readFileSync(path.join(dir, "patch000.dat")));
    assert.deepEqual(
        saved.entries().map((entry) => entry.name),
        ["art/critters/haenroaa.frm", "text/english/game/new.msg"],
    );
    assert.deepEqual(Buffer.from(saved.read("text/english/game/new.msg")), NEW_MSG);
    saved.free();

    const built = Archive.fromBytes(readFileSync(path.join(dir, "mod.dat")));
    assert.equal(built.format, "arcanum");
    assert.deepEqual(built.entries().map((entry) => entry.name).sort(), ["a.txt", "sub/b.txt"]);
    built.free();
});

test("the docs/api.md Node example refuses an entry name that leaves its output directory", () => {
    // insert refuses a ".." name, so the stored name of an ordinary entry is patched into one
    const bytes = Buffer.from(dat2With({ "aa/evil.txt": TINY }));
    const at = bytes.indexOf("aa\\evil.txt");
    assert.ok(at >= 0, "stored name not found in the archive bytes");
    bytes.write("..", at);
    const hostile = Archive.fromBytes(bytes);
    assert.deepEqual(
        hostile.entries().map((entry) => entry.name),
        ["../evil.txt"],
        "the library reads the name as is, so only the example's check stands in the way",
    );
    hostile.free();

    const dir = docExampleDir("doc_example_hostile", bytes);
    const result = spawn(process.execPath, ["example.ts"], dir);
    assert.notEqual(result.status, 0, "the example should have refused ../evil.txt");
    assert.match(result.stderr, /refusing to extract \.\.\/evil\.txt outside /);
    assert.ok(!existsSync(path.join(dir, "evil.txt")), "evil.txt was written outside out/");
});
