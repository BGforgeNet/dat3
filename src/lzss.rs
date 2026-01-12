use anyhow::Result;
use byteorder::{BigEndian, ReadBytesExt};
use std::io::{Cursor, Read};

/// LZSS Dictionary size - standard for DAT1 format
/// This is a power of 2 (2^12) to allow efficient bitwise operations
const DICT_SIZE: usize = 4096;

/// Minimum match length for dictionary references
/// Matches shorter than this are stored as literals
/// Note: MIN_MATCH would be used in compression implementation
#[allow(dead_code)]
const MIN_MATCH: usize = 3;

/// Maximum match length that can be encoded in 4 bits (0x0F + 2 + 1 = 18)
/// This comes from the encoding format where length is stored as (actual_length - 2)
/// in 4 bits, giving us a range of 2-17, but the implementation uses 0..=match_length
/// which makes the effective maximum 18
const MAX_MATCH: usize = 18;

/// Initial dictionary write position
/// Set to DICT_SIZE - MAX_MATCH to prevent buffer overrun during initial matches
const INITIAL_DICT_POS: usize = DICT_SIZE - MAX_MATCH; // 4096 - 18 = 4078

/// LZSS decompression for Fallout 1 DAT files
///
/// Algorithm Overview:
///
/// 1. **File Structure**: DAT1 files contain alternating blocks:
///    - 16-bit big-endian length N
///    - If N == 0: End of file
///    - If N < 0: |N| uncompressed bytes follow
///    - If N > 0: N compressed bytes follow (LZSS encoded)
///
/// 2. **LZSS Compression**: Uses sliding window dictionary compression:
///    - Dictionary size: 4096 bytes (circular buffer)
///    - Each compressed block starts fresh: dict position reset to 4078, entire dict filled with spaces
///    - Flag byte controls literal vs dictionary reference
///    - Dictionary references: 2 bytes encode position + length
///
/// 3. **Critical Implementation Details**:
///    - Each compressed block MUST reset dict position to DICT_SIZE-MAX_MATCH (4078)
///    - Each compressed block MUST reinitialize entire dictionary with spaces (0x20)
///    - Flag processing: shift-then-test pattern with upper bit tracking
///    - Dictionary wraparound using bitwise AND mask
///
/// This implementation follows the standard LZSS decompression algorithm for DAT1 files.
pub fn decompress(compressed_data: &[u8]) -> Result<Vec<u8>> {
    // Handle empty input data to prevent cursor errors
    if compressed_data.is_empty() {
        return Ok(Vec::new());
    }

    let mut cursor = Cursor::new(compressed_data);
    let mut output = Vec::new();
    // Dictionary will be initialized for each compressed block
    let mut dictionary = vec![0u8; DICT_SIZE];
    let mut dict_write_pos; // Will be set for each compressed block

    while let Ok(block_size) = cursor.read_i16::<BigEndian>() {
        if block_size == 0 {
            break;
        }

        if block_size < 0 {
            // Negative block_size: read |block_size| bytes directly to output
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
            // Positive block_size: compressed data follows
            let bytes_to_process = block_size as usize;
            let mut bytes_read = 0;

            // For each compressed block, reset dictionary position and initialize with spaces
            dict_write_pos = INITIAL_DICT_POS;
            dictionary.fill(0x20); // Fill entire dictionary with spaces (ASCII 32)

            // Process flags with upper bit tracking
            let mut flags: u16 = 0;

            loop {
                if bytes_read >= bytes_to_process {
                    break;
                }

                // Shift flags and check if we need to read a new flag byte
                flags >>= 1; // First shift right by 1
                if (flags & 256) == 0 {
                    // Then test bit 8
                    match cursor.read_u8() {
                        Ok(c) => {
                            flags = (c as u16) | 0xff00; // Set upper 8 bits
                            bytes_read += 1;
                            if bytes_read > bytes_to_process {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }

                if (flags & 1) != 0 {
                    // Flag bit is 1: literal byte
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
                    // Flag bit is 0: dictionary reference
                    if bytes_read + 1 >= bytes_to_process {
                        break;
                    }

                    // Need at least 2 bytes for dictionary reference
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

                    // Decode dictionary offset and match length
                    let dict_read_pos = (byte1 | ((byte2 & 0xF0) << 4)) as usize;
                    let match_length = ((byte2 & 0x0F) + 2) as usize;

                    // Copy from dictionary
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

/// LZSS compression for Fallout 1 DAT files
///
/// **Note**: This function is not currently used because DAT1 files are stored uncompressed
/// in our implementation. It's kept for potential future implementation.
#[allow(dead_code)]
pub fn compress(_data: &[u8]) -> Result<Vec<u8>> {
    // TODO: Implement LZSS compression for DAT1 format
    // For now, DAT1 files are stored uncompressed
    todo!("LZSS compression not implemented - DAT1 files are stored uncompressed")
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_decompress() {
        // TODO: Add decompression tests when compression is implemented
    }
}
