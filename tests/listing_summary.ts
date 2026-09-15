/**
 * Prints `entries|compressed|unpackedBytes` for a `dat3 l --json` listing.
 *
 * A separate file rather than a `node -e` string so that `npm run typecheck`
 * sees it and the ListingEntry shape stays checked. Run by toee-demo.sh.
 */

import { readFileSync } from "node:fs";
import { parseListing } from "./listing.ts";

const [listing] = process.argv.slice(2);
if (listing === undefined) {
    console.error("usage: listing_summary.ts <listing.json>");
    process.exit(2);
}

const entries = parseListing(readFileSync(listing, "utf8"));
const compressed = entries.filter((entry) => entry.compressed).length;
const unpacked = entries.reduce((sum, entry) => sum + entry.size, 0);
process.stdout.write(`${entries.length}|${compressed}|${unpacked}`);
