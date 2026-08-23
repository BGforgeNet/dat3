/**
 * Splits an InstallShield self-extracting installer into its member files.
 *
 * Everything after the last PE section is a flat sequence of records - four
 * NUL-terminated strings (name, stored path, version, decimal length) followed
 * by that many raw bytes. Nothing is compressed, so this is a copy rather than
 * a decoder, which is why it can live here instead of in a C helper the suite
 * would have to fetch and build. Run by toee-demo.sh; typechecked by
 * `npm run typecheck`.
 */

import { closeSync, fstatSync, mkdirSync, openSync, readSync, writeSync } from "node:fs";
import path from "node:path";

const [installer, outDir] = process.argv.slice(2);
if (installer === undefined || outDir === undefined) {
	console.error("usage: iss_extract.ts <installer.exe> <output-dir>");
	process.exit(2);
}

const fd = openSync(installer, "r");
const installerSize = fstatSync(fd).size;

function readAt(length: number, position: number): Buffer {
	const buffer = Buffer.alloc(length);
	return buffer.subarray(0, readSync(fd, buffer, 0, length, position));
}

/** End of the last raw PE section, which is where the member records start. */
function payloadStart(): number {
	const dos = readAt(64, 0);
	if (dos.toString("latin1", 0, 2) !== "MZ") {
		throw new Error(`${installer}: not a PE executable`);
	}
	const peOffset = dos.readUInt32LE(0x3c);
	const coff = readAt(24, peOffset);
	if (coff.toString("latin1", 0, 4) !== "PE\0\0") {
		throw new Error(`${installer}: missing PE header`);
	}
	const sectionCount = coff.readUInt16LE(6);
	const table = readAt(sectionCount * 40, peOffset + 24 + coff.readUInt16LE(20));

	let end = 0;
	for (let i = 0; i < sectionCount; i++) {
		const base = i * 40;
		end = Math.max(end, table.readUInt32LE(base + 16) + table.readUInt32LE(base + 20));
	}
	return end;
}

let position = payloadStart();

function readString(): string {
	const parts: Buffer[] = [];
	for (;;) {
		const chunk = readAt(256, position);
		if (chunk.length === 0) {
			throw new Error(`${installer}: truncated record header`);
		}
		const nul = chunk.indexOf(0);
		if (nul === -1) {
			parts.push(chunk);
			position += chunk.length;
			continue;
		}
		parts.push(chunk.subarray(0, nul));
		position += nul + 1;
		return Buffer.concat(parts).toString("latin1");
	}
}

mkdirSync(outDir, { recursive: true });
const copyBuffer = Buffer.alloc(1 << 20);

while (position < installerSize) {
	const name = readString();
	if (name === "") {
		break;
	}
	readString(); // stored install path, which the volumes do not need
	readString(); // file version, unused
	const length = Number(readString());
	if (!Number.isSafeInteger(length) || length < 0) {
		throw new Error(`${installer}: bad length for member ${name}`);
	}

	// basename, so a crafted member name cannot write outside outDir.
	const out = openSync(path.join(outDir, path.basename(name)), "w");
	try {
		let remaining = length;
		while (remaining > 0) {
			const got = readSync(fd, copyBuffer, 0, Math.min(remaining, copyBuffer.length), position);
			if (got === 0) {
				throw new Error(`${installer}: truncated member ${name}`);
			}
			writeSync(out, copyBuffer, 0, got);
			position += got;
			remaining -= got;
		}
	} finally {
		closeSync(out);
	}
	console.log(`${name} (${length} bytes)`);
}

closeSync(fd);
