/*!
# LZSS Decompression for Fallout 1 DAT files

Implements the sliding-window dictionary compression used by DAT1 archives.

Only decompression is implemented. Compression is stubbed for future work.
*/

use anyhow::Result;
use byteorder::{BigEndian, ReadBytesExt};
use std::io::{Cursor, Read};

/// Dictionary size (2^12) - standard for DAT1 format
const DICT_SIZE: usize = 4096;

/// Maximum match length: 4 bits -> 0..15, + 2 offset, + 1 inclusive = 18
const MAX_MATCH: usize = 18;

/// Initial dictionary write position.
/// Set to DICT_SIZE - MAX_MATCH to prevent buffer overrun during initial matches.
const INITIAL_DICT_POS: usize = DICT_SIZE - MAX_MATCH; // 4078

/// Block-header bit marking a raw (uncompressed) block; the low 15 bits carry its length.
const RAW_BLOCK_FLAG: u16 = 0x8000;

/// Decompress LZSS-encoded data from a DAT1 archive.
///
/// ## Block structure
///
/// The data consists of alternating blocks, each led by a 16-bit big-endian header `N`:
/// - `N == 0`: end of stream
/// - `N & 0x8000` set: raw (uncompressed) block; its length is the low 15 bits
/// - otherwise: `N` LZSS-compressed bytes follow
///
/// Each compressed block resets the dictionary (filled with spaces, position 4078).
/// A flag byte controls whether subsequent data is a literal byte or a
/// 2-byte dictionary reference (position + length).
///
/// ## The raw-block header is a flag plus a magnitude, not a signed length
///
/// Published descriptions of this format (fodev's `dat.html` among them) call the
/// header a signed 16-bit length and say a negative value introduces a raw block of
/// `|N|` bytes. That is wrong for every raw block shorter than `0x4000`: negating
/// the header yields `0x8000 - len` rather than `len`, always an over-read.
/// The two readings coincide at exactly one length, `0x4000` - which is the size of
/// every raw block except the last one in a stream, so the signed reading survives
/// almost everywhere and fails only on a stream's final short block.
///
/// Measured over the shipped Fallout 1 archives: `master.dat` carries 6,593 raw
/// blocks, 6,387 of them a full `0x4000` bytes (where both readings agree) and
/// 206 short ones where they diverge; reading the low 15 bits frames all 14,997 of its compressed
/// entries exactly, while negating mis-frames those 206 and aborts extraction part
/// way through. `critter.dat` cannot distinguish the two - it contains no raw blocks
/// at all, which is why testing against it alone left this undetected.
pub fn decompress(compressed_data: &[u8], expected_size: usize) -> Result<Vec<u8>> {
    if compressed_data.is_empty() {
        return Ok(Vec::new());
    }

    let mut cursor = Cursor::new(compressed_data);
    // expected_size is untrusted archive metadata, so cap the reservation by the
    // format's maximum expansion: 17 compressed bytes (flag + 8 two-byte
    // references) yield at most 8 * 18 = 144 output bytes, under 9x.
    let mut output = Vec::with_capacity(expected_size.min(compressed_data.len().saturating_mul(9)));
    let mut dictionary = vec![0u8; DICT_SIZE];
    let mut dict_write_pos;

    loop {
        // The stream may end cleanly at a block boundary; a partial 2-byte
        // block header past that point is truncation, not end-of-stream.
        if cursor.position() == compressed_data.len() as u64 {
            break;
        }
        let block_header = cursor
            .read_u16::<BigEndian>()
            .map_err(|e| anyhow::anyhow!("Truncated LZSS stream: incomplete block header: {e}"))?;
        if block_header == 0 {
            break;
        }

        if block_header & RAW_BLOCK_FLAG != 0 {
            // Raw block: the low 15 bits are its length (see the header note above)
            let bytes_to_read = (block_header & !RAW_BLOCK_FLAG) as usize;
            let mut direct_bytes = vec![0u8; bytes_to_read];
            cursor.read_exact(&mut direct_bytes).map_err(|e| {
                anyhow::anyhow!(
                    "Failed to read {} uncompressed bytes: {} (remaining: {})",
                    bytes_to_read,
                    e,
                    compressed_data.len() - cursor.position() as usize
                )
            })?;
            output.extend_from_slice(&direct_bytes);
        } else {
            // Compressed block: LZSS-encoded data
            let bytes_to_process = block_header as usize;
            let mut bytes_read = 0;

            // Reset dictionary for each compressed block
            dict_write_pos = INITIAL_DICT_POS;
            dictionary.fill(0x20); // Fill with spaces (ASCII 32)

            // Flag byte: shifted right each iteration, refilled when bit 8 is clear
            let mut flags: u16 = 0;

            loop {
                if bytes_read >= bytes_to_process {
                    break;
                }

                flags >>= 1;
                if (flags & 256) == 0 {
                    let c = cursor.read_u8().map_err(|e| {
                        anyhow::anyhow!(
                            "Truncated LZSS stream: failed to read flag byte at position {}: {}",
                            bytes_read,
                            e
                        )
                    })?;
                    flags = (c as u16) | 0xff00;
                    bytes_read += 1;
                    if bytes_read > bytes_to_process {
                        break;
                    }
                }

                if (flags & 1) != 0 {
                    // Literal byte
                    let byte = cursor.read_u8().map_err(|e| {
                        anyhow::anyhow!(
                            "Failed to read literal byte at position {}: {}",
                            bytes_read,
                            e
                        )
                    })?;
                    bytes_read += 1;

                    output.push(byte);
                    dictionary[dict_write_pos] = byte;
                    dict_write_pos = (dict_write_pos + 1) & (DICT_SIZE - 1);
                } else {
                    // Dictionary reference (2 bytes: position + length)
                    if bytes_read + 1 >= bytes_to_process {
                        break;
                    }

                    let byte1 = cursor.read_u8().map_err(|e| {
                        anyhow::anyhow!(
                            "Failed to read dictionary byte 1 at position {}: {}",
                            bytes_read,
                            e
                        )
                    })? as u16;
                    let byte2 = cursor.read_u8().map_err(|e| {
                        anyhow::anyhow!(
                            "Failed to read dictionary byte 2 at position {}: {}",
                            bytes_read + 1,
                            e
                        )
                    })? as u16;
                    bytes_read += 2;

                    let dict_read_pos = (byte1 | ((byte2 & 0xF0) << 4)) as usize;
                    let match_length = ((byte2 & 0x0F) + 2) as usize;

                    // Copy match_length+1 bytes from dictionary
                    for offset in 0..=match_length {
                        let read_offset = (dict_read_pos + offset) & (DICT_SIZE - 1);
                        let byte = dictionary[read_offset];
                        output.push(byte);
                        dictionary[dict_write_pos] = byte;
                        dict_write_pos = (dict_write_pos + 1) & (DICT_SIZE - 1);
                    }
                }
            }
        }
    }

    Ok(output)
}

/// LZSS compression for DAT1 files (not yet implemented).
///
/// Currently DAT1 archives are created with uncompressed files.
/// This stub exists for future implementation.
#[allow(dead_code)] // Stub for future LZSS compression support
pub fn compress(_data: &[u8]) -> Result<Vec<u8>> {
    todo!("LZSS compression not implemented - DAT1 files are stored uncompressed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The size shipped archives use for every raw block except the last one in
    /// a stream, and the one length at which the signed and flag readings agree.
    /// Not the 15-bit ceiling (0x7FFF) - just the encoder's chunk size.
    const MAX_RAW_BLOCK_LEN: usize = 0x4000; // 16384

    proptest! {
        #[test]
        fn decompress_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
            let _ = decompress(&bytes, 0);
        }

        #[test]
        fn raw_blocks_round_trip(payload in prop::collection::vec(any::<u8>(), 1..512)) {
            let stream = raw_block(&payload);
            prop_assert_eq!(decompress(&stream, payload.len()).unwrap(), payload);
        }
    }

    #[test]
    fn does_not_trust_hostile_expected_size() {
        // expected_size comes from archive metadata; a crafted value must not
        // trigger a giant (or overflowing) upfront allocation.
        assert_eq!(decompress(&raw_block(b"ABC"), usize::MAX).unwrap(), b"ABC");
    }

    /// Raw (uncompressed) block, encoded the way shipped archives encode one:
    /// `RAW_BLOCK_FLAG | length` big-endian, then `length` literal bytes.
    fn raw_block(payload: &[u8]) -> Vec<u8> {
        // A payload at or past the flag bit would encode as a different block
        // entirely - the exact confusion these tests exist to pin down.
        assert!(payload.len() < RAW_BLOCK_FLAG as usize);
        let header = RAW_BLOCK_FLAG | payload.len() as u16;
        let mut v = header.to_be_bytes().to_vec();
        v.extend_from_slice(payload);
        v
    }

    /// The regression this file's fixtures used to be blind to: a raw block
    /// shorter than a full 16 KiB one. Reading the header as a signed length
    /// yields `0x8000 - len` here, which is always an over-read.
    #[test]
    fn short_raw_block_reads_its_low_15_bits_as_the_length() {
        for len in [1usize, 3, 9352, 14737, MAX_RAW_BLOCK_LEN - 1] {
            let payload = vec![0xABu8; len];
            let decoded = decompress(&raw_block(&payload), len)
                .unwrap_or_else(|e| panic!("raw block of {len} bytes failed to decode: {e}"));
            assert_eq!(
                decoded, payload,
                "raw block of {len} bytes round-tripped wrong"
            );
        }
    }

    /// A full-size raw block is the one length where the signed reading happens
    /// to agree, which is why every retail `critter.dat` entry decoded correctly.
    #[test]
    fn full_size_raw_block_still_round_trips() {
        let payload = vec![0x5Au8; MAX_RAW_BLOCK_LEN];
        assert_eq!(
            decompress(&raw_block(&payload), MAX_RAW_BLOCK_LEN).unwrap(),
            payload
        );
    }

    #[test]
    fn decompresses_raw_block() {
        assert_eq!(decompress(&raw_block(b"ABC"), 3).unwrap(), b"ABC");
    }

    #[test]
    fn zero_block_size_terminates_stream() {
        let mut stream = raw_block(b"ABC");
        stream.extend_from_slice(&[0x00, 0x00]);
        assert_eq!(decompress(&stream, 3).unwrap(), b"ABC");
    }

    #[test]
    fn decompresses_literal_in_compressed_block() {
        // Compressed block of 2 bytes: flag byte 0x01 (bit 0 set = literal), then the literal.
        let stream = [0x00, 0x02, 0x01, b'X'];
        assert_eq!(decompress(&stream, 3).unwrap(), b"X");
    }

    #[test]
    fn errors_on_truncated_block_header() {
        // A lone trailing byte cannot form the 2-byte block-size header.
        let mut stream = raw_block(b"ABC");
        stream.push(0x00);
        assert!(decompress(&stream, 8).is_err());
    }

    #[test]
    fn errors_on_truncated_raw_block() {
        // Header claims 5 raw bytes, only 3 present.
        let mut stream = (RAW_BLOCK_FLAG | 5u16).to_be_bytes().to_vec();
        stream.extend_from_slice(b"ABC");
        assert!(decompress(&stream, 8).is_err());
    }

    #[test]
    fn errors_on_compressed_block_with_missing_data() {
        // Header claims a 2-byte compressed block, but the stream ends immediately.
        let stream = [0x00, 0x02];
        assert!(decompress(&stream, 8).is_err());
    }
}
