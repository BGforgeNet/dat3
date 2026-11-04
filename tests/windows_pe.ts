/**
 * What the Windows loader asks of the cross-built dat3.exe files before running a line of them, checked from their PE
 * headers on any platform: they must load on Windows 7 SP1.
 *
 * The test jobs run on current Windows, which loads a binary importing a function Windows 7 lacks, so only the
 * import table shows it. Run by test.sh against build.sh's output; typechecked by `npm run typecheck`.
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";

const TARGET_DIR = path.join(import.meta.dirname, "..", "target");

/** IMAGE_FILE_LARGE_ADDRESS_AWARE: a 32-bit process gets 4 GiB of address space on 64-bit Windows, not 2 GiB. */
const LARGE_ADDRESS_AWARE = 0x20;

/** Windows 7 is NT 6.1; the loader refuses an image whose subsystem version is higher than the system's. */
const WINDOWS_7 = { major: 6, minor: 1 };

/**
 * Every DLL the builds may import, each shipped by Windows 7 SP1 itself. msvcrt.dll rather than the Universal C
 * Runtime's api-ms-win-crt-* sets, which Windows 7 has only after an optional update. A new import fails the test
 * until it is checked against Windows 7 and added here.
 */
const WINDOWS_7_DLLS = new Set(["kernel32.dll", "ntdll.dll", "advapi32.dll", "msvcrt.dll"]);

/**
 * Functions added after Windows 7 that a Rust or MinGW build has been known to import. Not every newer function: the
 * DLL list above is the broad check, and this one catches what reaches Windows 7's own DLLs by name.
 */
const AFTER_WINDOWS_7 = new Set([
    "GetSystemTimePreciseAsFileTime",
    "WaitOnAddress",
    "WakeByAddressSingle",
    "WakeByAddressAll",
    "ProcessPrng",
    "CreateFile2",
    "CopyFile2",
    "GetCurrentThreadStackLimits",
    "GetOverlappedResultEx",
    "SetThreadDescription",
    "GetThreadDescription",
]);

interface Build {
    exe: string;
    machine: number;
    /** Only a 32-bit image's address space depends on the flag */
    is32Bit: boolean;
}

const BUILDS: Build[] = [
    // IMAGE_FILE_MACHINE_AMD64
    { exe: "x86_64-win7-windows-gnu", machine: 0x8664, is32Bit: false },
    // IMAGE_FILE_MACHINE_I386
    { exe: "i686-win7-windows-gnu", machine: 0x14c, is32Bit: true },
];

interface PeImage {
    machine: number;
    characteristics: number;
    subsystemVersion: { major: number; minor: number };
    /** Imported function names by lowercased DLL name; ordinal imports appear as `#<ordinal>` */
    imports: Map<string, string[]>;
}

/** The headers and import table of a PE image, enough to judge whether a given Windows loads it. */
function readPe(file: string): PeImage {
    const buf = readFileSync(file);
    assert.equal(buf.toString("latin1", 0, 2), "MZ", `${file}: no DOS header`);
    const pe = buf.readUInt32LE(0x3c);
    assert.equal(buf.toString("latin1", pe, pe + 4), "PE\0\0", `${file}: no PE signature`);

    const coff = pe + 4;
    const machine = buf.readUInt16LE(coff);
    const sectionCount = buf.readUInt16LE(coff + 2);
    const optionalSize = buf.readUInt16LE(coff + 16);
    const characteristics = buf.readUInt16LE(coff + 18);

    const opt = coff + 20;
    const magic = buf.readUInt16LE(opt);
    assert.ok(magic === 0x10b || magic === 0x20b, `${file}: unknown optional header magic 0x${magic.toString(16)}`);
    const pe32Plus = magic === 0x20b;
    const subsystemVersion = { major: buf.readUInt16LE(opt + 48), minor: buf.readUInt16LE(opt + 50) };
    // Data directory 1 is the import table
    const importRva = buf.readUInt32LE(opt + (pe32Plus ? 112 : 96) + 8);

    const sections = Array.from({ length: sectionCount }, (_, i) => {
        const at = opt + optionalSize + i * 40;
        return {
            rva: buf.readUInt32LE(at + 12),
            size: Math.max(buf.readUInt32LE(at + 8), buf.readUInt32LE(at + 16)),
            offset: buf.readUInt32LE(at + 20),
        };
    });
    const fileOffset = (rva: number): number => {
        const section = sections.find((s) => rva >= s.rva && rva < s.rva + s.size);
        assert.ok(section, `${file}: RVA 0x${rva.toString(16)} is in no section`);
        return rva - section.rva + section.offset;
    };
    const cString = (at: number): string => buf.toString("latin1", at, buf.indexOf(0, at));

    const imports = new Map<string, string[]>();
    for (let desc = fileOffset(importRva); ; desc += 20) {
        const lookupRva = buf.readUInt32LE(desc) || buf.readUInt32LE(desc + 16);
        const nameRva = buf.readUInt32LE(desc + 12);
        if (lookupRva === 0 && nameRva === 0) {
            break;
        }
        const names: string[] = [];
        const thunkSize = pe32Plus ? 8 : 4;
        for (let thunk = fileOffset(lookupRva); ; thunk += thunkSize) {
            const value = pe32Plus ? buf.readBigUInt64LE(thunk) : BigInt(buf.readUInt32LE(thunk));
            if (value === 0n) {
                break;
            }
            const ordinalFlag = pe32Plus ? 1n << 63n : 1n << 31n;
            names.push(
                value & ordinalFlag ? `#${value & 0xffffn}` : cString(fileOffset(Number(value & 0x7fffffffn)) + 2),
            );
        }
        imports.set(cString(fileOffset(nameRva)).toLowerCase(), names);
    }
    return { machine, characteristics, subsystemVersion, imports };
}

for (const build of BUILDS) {
    const exe = path.join(TARGET_DIR, build.exe, "release", "dat3.exe");
    const image = readPe(exe);

    test(`${build.exe}: built for its architecture`, () => {
        assert.equal(image.machine, build.machine);
    });

    test(`${build.exe}: imports only DLLs Windows 7 ships`, () => {
        assert.ok(image.imports.has("kernel32.dll"), `no kernel32.dll import: ${[...image.imports.keys()].join(", ")}`);
        const foreign = [...image.imports.keys()].filter((dll) => !WINDOWS_7_DLLS.has(dll));
        assert.deepEqual(foreign, []);
    });

    test(`${build.exe}: imports no function added after Windows 7`, () => {
        const newer = [...image.imports].flatMap(([dll, names]) =>
            names.filter((name) => AFTER_WINDOWS_7.has(name)).map((name) => `${dll}!${name}`),
        );
        assert.deepEqual(newer, []);
    });

    test(`${build.exe}: subsystem version no higher than Windows 7's`, () => {
        const { major, minor } = image.subsystemVersion;
        assert.ok(
            major < WINDOWS_7.major || (major === WINDOWS_7.major && minor <= WINDOWS_7.minor),
            `subsystem version ${major}.${minor}`,
        );
    });

    if (build.is32Bit) {
        test(`${build.exe}: large address aware`, () => {
            assert.equal(image.characteristics & LARGE_ADDRESS_AWARE, LARGE_ADDRESS_AWARE);
        });
    }
}
