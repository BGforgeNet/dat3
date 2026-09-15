/**
 * The `dat3 l --json` document: its entry shape, and the one parser every test reading it goes through.
 *
 * The shape check lives here rather than in one test so that every use of a listing gets it, and so a
 * malformed document fails where it is read.
 */

import assert from "node:assert/strict";

/** One object in a `dat3 l --json` array */
export interface ListingEntry {
    name: string;
    size: number;
    packed_size: number;
    compressed: boolean;
}

/** Parse a listing and check every entry's shape */
export function parseListing(json: string): ListingEntry[] {
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
        assert.ok(typeof packed_size === "number", `packed_size must be a number: ${JSON.stringify(entry)}`);
        assert.ok(typeof compressed === "boolean", `compressed must be a boolean: ${JSON.stringify(entry)}`);
        return { name, size, packed_size, compressed };
    });
}
