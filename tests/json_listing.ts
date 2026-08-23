/**
 * Integration test for the machine-readable listing: `dat3 l --json`.
 *
 * Written in TypeScript rather than shell because the assertions are about a
 * parsed data structure, and a real parser is the only thing that can check the
 * promise the flag makes. Run by test.sh; typechecked by `npm run typecheck`.
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import path from "node:path";

/** One object in a `l --json` array. */
interface ListingEntry {
	name: string;
	size: number;
	packed_size: number;
	compressed: boolean;
}

/** The archive formats dat3 can create, all of which share one listing path. */
const FORMATS = ["dat2", "dat1", "arcanum", "toee"] as const;
type Format = (typeof FORMATS)[number];

const TESTS_DIR = import.meta.dirname;
const WORK_DIR = path.join(TESTS_DIR, "test_json_listing");
const SRC_DIR = path.join(WORK_DIR, "src");

// test.sh exports DAT3; the fallback is the same static build its common.sh points at.
const DAT3 =
	process.env["DAT3"] ??
	path.join(TESTS_DIR, "..", "target", "x86_64-unknown-linux-musl", "release", "dat3");

interface RunResult {
	stdout: string;
	stderr: string;
	status: number;
}

/** Run dat3 inside the work directory. */
function dat3(...args: string[]): RunResult {
	const result = spawnSync(DAT3, args, { cwd: WORK_DIR, encoding: "utf8" });
	if (result.error) {
		throw result.error;
	}
	return { stdout: result.stdout, stderr: result.stderr, status: result.status ?? -1 };
}

/** Run dat3 and fail the test if it did not succeed. */
function dat3Ok(...args: string[]): RunResult {
	const result = dat3(...args);
	assert.equal(result.status, 0, `dat3 ${args.join(" ")} failed:\n${result.stderr}`);
	return result;
}

/**
 * Parse a listing and check every entry's shape.
 *
 * The shape check lives here rather than in one test so that every use of a
 * listing gets it, and so a malformed document fails where it is read.
 */
function parseListing(json: string): ListingEntry[] {
	const parsed: unknown = JSON.parse(json);
	assert.ok(Array.isArray(parsed), `listing must be an array: ${json}`);
	return parsed.map((raw: unknown): ListingEntry => {
		assert.ok(typeof raw === "object" && raw !== null, `entry must be an object: ${json}`);
		const entry = raw as Record<string, unknown>;
		assert.deepEqual(
			Object.keys(entry).sort(),
			["compressed", "name", "packed_size", "size"],
			`unexpected field set: ${JSON.stringify(entry)}`,
		);
		const { name, size, packed_size, compressed } = entry;
		// assert.ok carries an assertion signature, so each check narrows the
		// value for the return below; assert.equal does not, which is what the
		// casts here used to work around.
		assert.ok(typeof name === "string", `name must be a string: ${JSON.stringify(entry)}`);
		assert.ok(typeof size === "number", `size must be a number: ${JSON.stringify(entry)}`);
		assert.ok(
			typeof packed_size === "number",
			`packed_size must be a number: ${JSON.stringify(entry)}`,
		);
		assert.ok(
			typeof compressed === "boolean",
			`compressed must be a boolean: ${JSON.stringify(entry)}`,
		);
		return { name, size, packed_size, compressed };
	});
}

/** The entry with the given name, or a test failure naming what was there. */
function entryNamed(entries: ListingEntry[], name: string): ListingEntry {
	const found = entries.find((entry) => entry.name === name);
	assert.ok(found, `no entry named ${name} in ${JSON.stringify(entries.map((e) => e.name))}`);
	return found;
}

/**
 * The names from the aligned text listing: drop the header and its rule, then
 * take everything past the last column.
 */
function namesFromText(listing: string): string[] {
	return listing
		.split("\n")
		.slice(2)
		.filter((line) => line.length > 0)
		.map((line) => {
			const match = /^ *\d+ +\d+ +(?:Yes|No) +(.*)$/.exec(line);
			assert.ok(match?.[1] !== undefined, `unparseable listing line: ${JSON.stringify(line)}`);
			return match[1];
		});
}

const archiveFor = (format: Format): string => `test_${format}.dat`;

// A compressible file and a file too small to be worth compressing, so the
// listing carries both compressed states, plus a subdirectory for the separators.
rmSync(WORK_DIR, { recursive: true, force: true });
mkdirSync(path.join(SRC_DIR, "sub"), { recursive: true });
const BIG = Array.from({ length: 2000 }, (_, i) => String(i + 1)).join("\n") + "\n";
writeFileSync(path.join(SRC_DIR, "big.txt"), BIG);
writeFileSync(path.join(SRC_DIR, "sub", "tiny.txt"), "hi\n");

for (const format of FORMATS) {
	dat3Ok("a", archiveFor(format), "--format", format, "-c", "9", "-C", "src", "big.txt", "sub/tiny.txt");
}

for (const format of FORMATS) {
	test(`${format}: the JSON listing describes the same entries as the text listing`, () => {
		const entries = parseListing(dat3Ok("l", archiveFor(format), "--json").stdout);
		const textNames = namesFromText(dat3Ok("l", archiveFor(format)).stdout);
		assert.deepEqual(
			entries.map((entry) => entry.name),
			textNames,
		);
	});

	test(`${format}: sizes match the files that went in`, () => {
		const entries = parseListing(dat3Ok("l", archiveFor(format), "--json").stdout);
		assert.equal(entries.length, 2);
		assert.equal(entryNamed(entries, "big.txt").size, statSync(path.join(SRC_DIR, "big.txt")).size);
		assert.equal(
			entryNamed(entries, "sub/tiny.txt").size,
			statSync(path.join(SRC_DIR, "sub", "tiny.txt")).size,
		);
	});

	test(`${format}: separators are forward slashes on every platform`, () => {
		const entries = parseListing(dat3Ok("l", archiveFor(format), "--json").stdout);
		assert.ok(entryNamed(entries, "sub/tiny.txt"));
		for (const entry of entries) {
			assert.ok(!entry.name.includes("\\"), `backslash in name: ${entry.name}`);
		}
	});

	test(`${format}: the names it prints are names extraction accepts back`, () => {
		const entries = parseListing(dat3Ok("l", archiveFor(format), "--json").stdout);
		const outDir = `out_${format}`;
		rmSync(path.join(WORK_DIR, outDir), { recursive: true, force: true });
		for (const entry of entries) {
			dat3Ok("x", archiveFor(format), "-o", outDir, entry.name);
		}
		const diff = spawnSync("diff", ["-r", "src", outDir], { cwd: WORK_DIR, encoding: "utf8" });
		assert.equal(diff.status, 0, diff.stdout + diff.stderr);
	});
}

// DAT1 stores uncompressed, so only the zlib formats report a compressed entry.
for (const [format, expected] of [
	["dat2", true],
	["arcanum", true],
	["toee", true],
	["dat1", false],
] as const) {
	test(`${format}: reports compression as ${expected}`, () => {
		const big = entryNamed(parseListing(dat3Ok("l", archiveFor(format), "--json").stdout), "big.txt");
		assert.equal(big.compressed, expected);
		if (expected) {
			assert.ok(big.packed_size < big.size, JSON.stringify(big));
		} else {
			assert.equal(big.packed_size, big.size);
		}
	});
}

test("a filter narrows the array", () => {
	const entries = parseListing(dat3Ok("l", "test_dat2.dat", "--json", "big.txt").stdout);
	assert.deepEqual(
		entries.map((entry) => entry.name),
		["big.txt"],
	);
});

test("a name that is not in the archive still fails, as the text listing does", () => {
	const result = dat3("l", "test_dat2.dat", "--json", "nope.txt");
	assert.notEqual(result.status, 0, "should have failed");
	assert.match(result.stderr, /nope\.txt/);
	// stdout stays a valid document, so a consumer parsing it sees no entries
	assert.deepEqual(parseListing(result.stdout), []);
});
