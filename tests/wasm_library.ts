/**
 * Integration test for the dat3-wasm npm package: dat3-core for Node and Electron.
 *
 * Installs the packed tarball into a scratch project, as an application would, and
 * checks the library against the native dat3 on the fixture archives the other
 * tests fetch. Run by wasm_library.sh after the package build; typechecked by
 * `npm run typecheck`.
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";

const TESTS_DIR = import.meta.dirname;
const WORK_DIR = path.join(TESTS_DIR, "test_wasm_library");
const TARBALL = path.join(TESTS_DIR, "..", "target", "dat3-wasm.tgz");
const DAT3 = path.join(TESTS_DIR, "..", "target", "x86_64-unknown-linux-musl", "release", "dat3");

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

/** One object in a `dat3 l --json` array */
interface ListingEntry {
	name: string;
	size: number;
	packed_size: number;
	compressed: boolean;
}

function run(command: string, args: string[]): string {
	const result = spawnSync(command, args, {
		cwd: WORK_DIR,
		encoding: "utf8",
		timeout: SPAWN_TIMEOUT_MS,
		maxBuffer: SPAWN_MAX_BUFFER,
	});
	if (result.error) {
		throw result.error;
	}
	assert.equal(result.status, 0, `${command} ${args.join(" ")} failed:\n${result.stderr}`);
	return result.stdout;
}

/** The native listing, in the library's field names */
function nativeEntries(archive: string): Entry[] {
	// Unchecked here: json_listing.ts verifies this document's shape field by field
	const listing = JSON.parse(run(DAT3, ["l", "--json", "--case-sensitive", archive])) as ListingEntry[];
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
				.filter((entry) => Buffer.compare(archive.read(entry.name), readFileSync(path.join(outDir, entry.name))) !== 0)
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
