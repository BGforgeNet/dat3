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

/// Decompress LZSS-encoded data from a DAT1 archive.
///
/// ## Block structure
///
/// The data consists of alternating blocks:
/// - 16-bit big-endian length `N`
/// - If `N == 0`: end of stream
/// - If `N < 0`: `|N|` raw (uncompressed) bytes follow
/// - If `N > 0`: `N` LZSS-compressed bytes follow
///
/// Each compressed block resets the dictionary (filled with spaces, position 4078).
/// A flag byte controls whether subsequent data is a literal byte or a
/// 2-byte dictionary reference (position + length).
pub fn decompress(compressed_data: &[u8]) -> Result<Vec<u8>> {
    if compressed_data.is_empty() {
        return Ok(Vec::new());
    }

    let mut cursor = Cursor::new(compressed_data);
    let mut output = Vec::new();
    let mut dictionary = vec![0u8; DICT_SIZE];
    let mut dict_write_pos;

    while let Ok(block_size) = cursor.read_i16::<BigEndian>() {
        if block_size == 0 {
            break;
        }

        if block_size < 0 {
            // Raw block: read |block_size| bytes directly
            let bytes_to_read = (-block_size) as usize;
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
            let bytes_to_process = block_size as usize;
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
                    match cursor.read_u8() {
                        Ok(c) => {
                            flags = (c as u16) | 0xff00;
                            bytes_read += 1;
                            if bytes_read > bytes_to_process {
                                break;
                            }
                        }
                        Err(_) => break,
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
