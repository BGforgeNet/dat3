/*!
# The Temple of Elemental Evil DAT Archive Format

ToEE uses Troika's hierarchical DAT layout. It shares Arcanum's zlib payloads
and newer `1TAD` footer, but its entry table is a tree: names are single path
components and every record has parent, first-child, and next-sibling indices.

## File layout

1. File data (all files concatenated, raw or zlib streams)
2. Table marker: u32 absolute offset of the entry table
3. Entry table: u32 entry count, then hierarchical file/directory entries
4. Footer: either the original 12-byte `DAT ` variant, or the 28-byte `DAT1`
   variant with a 16-byte GUID; both end with total name bytes and table distance
*/

use anyhow::{Context, Result, bail};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read, Write};
use std::path::Path;

use crate::common::{self, CompressionLevel, ExtractionMode, FileEntry, ListFormat, utils};

const FOOTER_SIZE: usize = 28;
const MAGIC: [u8; 4] = *b"1TAD";
const V0_FOOTER_SIZE: usize = 12;
const V0_MAGIC: [u8; 4] = *b" TAD";
/// Longest stored name, including its NUL. Matches the bound OpenTemple's
/// reader enforces per entry, which is what shipped archives are built against.
const MAX_COMPONENT_BYTES: usize = 260;

/// Longest full path: a backstop on parser memory rather than a format limit,
/// since a path is materialized per entry and unbounded depth turned a 2.9 MB
/// archive into 14.3 GB of strings. Shipped archives peak at 111 bytes.
const MAX_PATH_BYTES: usize = 1024;

const FLAG_RAW: u32 = 0x1;
const FLAG_ZLIB: u32 = 0x2;
const FLAG_DIR: u32 = 0x400;

/// Bytes per table entry excluding the variable-length name bytes.
/// name_len plus eight metadata words.
const ENTRY_FIXED_BYTES: u64 = 9 * 4;

#[derive(Debug)]
struct ToeeFileEntry {
    name: String,
    flags: u32,
    real_size: u32,
    packed_size: u32,
    offset: u32,
    parent: i32,
    first_child: i32,
    next_sibling: i32,
}

impl ToeeFileEntry {
    fn is_dir(&self) -> bool {
        self.flags & FLAG_DIR != 0
    }
}

#[derive(Debug)]
struct SaveNode {
    full_name: String,
    name: String,
    file_index: Option<usize>,
    parent: Option<usize>,
    first_child: Option<usize>,
    next_sibling: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ToeeVersion {
    V0,
    V1,
}

/// ToEE archive handler.
#[derive(Debug)]
pub struct ToeeArchive {
    files: Vec<FileEntry>,
    /// Directory entries as stored, so ones holding no files still round-trip.
    /// Shipped archives carry them and `save` cannot re-derive them from paths.
    dirs: Vec<String>,
    data: Vec<u8>,
    guid: [u8; 16],
    version: ToeeVersion,
}

/// Distinguish ToEE's hierarchical table from Arcanum's flat table.
///
/// ToEE V0 has its own signature. ToEE V1 and Arcanum use the same footer, but
/// their exact table sizes differ by three link fields per entry, so entry
/// count plus the footer's name-byte total distinguishes non-empty archives.
pub fn is_toee_format(data: &[u8]) -> bool {
    let Some(trailer) = data.get(data.len().saturating_sub(V0_FOOTER_SIZE)..) else {
        return false;
    };
    if trailer.len() != V0_FOOTER_SIZE {
        return false;
    }

    let magic: [u8; 4] = trailer[..4].try_into().unwrap();
    let footer_size = match magic {
        V0_MAGIC => V0_FOOTER_SIZE,
        MAGIC => FOOTER_SIZE,
        _ => return false,
    };
    if data.len() < footer_size + 4 {
        return false;
    }

    let filename_bytes = u32::from_le_bytes(trailer[4..8].try_into().unwrap()) as u64;
    let table_from_end = u32::from_le_bytes(trailer[8..12].try_into().unwrap()) as usize;
    if table_from_end < footer_size + 4 || table_from_end > data.len() {
        return false;
    }

    let table_start = data.len() - table_from_end;
    let Some(count_bytes) = data.get(table_start..table_start + 4) else {
        return false;
    };
    let entry_count = u32::from_le_bytes(count_bytes.try_into().unwrap()) as u64;
    if entry_count == 0 && magic == MAGIC {
        // An empty ToEE table is byte-for-byte indistinguishable from an empty
        // Arcanum table, so retain the established Arcanum interpretation. The
        // cost is that deleting the last entry of a v1 archive reopens it as
        // Arcanum; nothing in the format can tell the two apart, and the v0
        // footer, which has its own signature, is unaffected.
        return false;
    }

    let expected = (footer_size as u64)
        .checked_add(4)
        .and_then(|n| n.checked_add(filename_bytes))
        .and_then(|n| n.checked_add(entry_count.checked_mul(ENTRY_FIXED_BYTES)?));
    expected == Some(table_from_end as u64)
}

impl ToeeArchive {
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            dirs: Vec::new(),
            data: Vec::new(),
            guid: [0; 16],
            version: ToeeVersion::V1,
        }
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        if data.len() < V0_FOOTER_SIZE + 4 {
            bail!("ToEE DAT file too small");
        }

        let trailer = &data[data.len() - V0_FOOTER_SIZE..];
        let magic: [u8; 4] = trailer[..4].try_into().unwrap();
        let version = match magic {
            V0_MAGIC => ToeeVersion::V0,
            MAGIC => ToeeVersion::V1,
            _ => bail!("Not a ToEE DAT archive: missing DAT or DAT1 signature"),
        };
        let footer_size = match version {
            ToeeVersion::V0 => V0_FOOTER_SIZE,
            ToeeVersion::V1 => FOOTER_SIZE,
        };
        if data.len() < footer_size + 4 {
            bail!("ToEE DAT file too small for its footer variant");
        }

        let footer_start = data.len() - footer_size;
        let footer = &data[footer_start..];
        let mut guid = [0u8; 16];
        if version == ToeeVersion::V1 {
            guid.copy_from_slice(&footer[..16]);
        }
        let filename_total_bytes = u32::from_le_bytes(trailer[4..8].try_into().unwrap()) as u64;
        let table_from_end = u32::from_le_bytes(trailer[8..12].try_into().unwrap()) as usize;
        if table_from_end < footer_size + 4 || table_from_end > data.len() {
            bail!("Invalid ToEE footer: entry table offset out of range");
        }

        let table_start = data.len() - table_from_end;
        if table_start > 0 && table_start < 4 {
            bail!("Invalid ToEE archive: truncated entry table marker");
        }
        if table_start >= 4 {
            let marker = u32::from_le_bytes(data[table_start - 4..table_start].try_into().unwrap());
            if marker as usize != table_start {
                bail!("Invalid ToEE archive: entry table marker does not match footer");
            }
        }

        let table = &data[table_start..footer_start];
        let mut cursor = Cursor::new(table);
        let entry_count = cursor
            .read_u32::<LittleEndian>()
            .context("Failed to read ToEE entry count")?;
        let expected_table_size = 4u64
            .checked_add(filename_total_bytes)
            .and_then(|n| n.checked_add((entry_count as u64).checked_mul(ENTRY_FIXED_BYTES)?))
            .context("ToEE entry table size overflow")?;
        if expected_table_size != table.len() as u64 {
            bail!("Invalid ToEE footer: entry table size does not match entry count and names");
        }

        let mut entries = Vec::new();
        for i in 0..entry_count {
            entries.push(Self::read_entry(&mut cursor, i)?);
        }
        if cursor.position() != table.len() as u64 {
            bail!("Invalid ToEE entry table: trailing bytes after final entry");
        }

        Self::validate_links(&entries)?;

        let mut paths = vec![None; entries.len()];
        let mut visiting = vec![false; entries.len()];
        for i in 0..entries.len() {
            Self::build_full_path(i, &entries, &mut paths, &mut visiting)?;
        }

        let data_end = table_start.saturating_sub(4);
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        for (i, entry) in entries.iter().enumerate() {
            if entry.is_dir() {
                if entry.real_size != 0 || entry.packed_size != 0 || entry.offset != 0 {
                    bail!("Invalid ToEE directory entry {i}: directory carries file data");
                }
                dirs.push(
                    paths[i]
                        .clone()
                        .context("ToEE directory path was not resolved")?,
                );
                continue;
            }

            let end = (entry.offset as usize)
                .checked_add(entry.packed_size as usize)
                .with_context(|| format!("Invalid ToEE entry {i}: data range overflow"))?;
            if end > data_end {
                bail!("Invalid ToEE entry {i}: file data extends into entry table");
            }

            files.push(FileEntry {
                name: paths[i]
                    .clone()
                    .context("ToEE file path was not resolved")?,
                offset: entry.offset as u64,
                size: entry.real_size,
                packed_size: entry.packed_size,
                compressed: entry.flags & FLAG_ZLIB != 0,
                data: None,
            });
        }

        Ok(Self {
            files,
            dirs,
            data,
            guid,
            version,
        })
    }

    fn read_entry(cursor: &mut Cursor<&[u8]>, index: u32) -> Result<ToeeFileEntry> {
        let name_len = cursor
            .read_u32::<LittleEndian>()
            .with_context(|| format!("Failed to read ToEE entry {index} name length"))?
            as usize;
        if name_len == 0 || name_len > MAX_COMPONENT_BYTES {
            bail!(
                "Invalid ToEE entry {index}: name length {name_len} is outside 1..={MAX_COMPONENT_BYTES}"
            );
        }

        let mut name_bytes = vec![0; name_len];
        cursor
            .read_exact(&mut name_bytes)
            .with_context(|| format!("Failed to read ToEE entry {index} name"))?;
        if name_bytes.last() != Some(&0) {
            bail!("Invalid ToEE entry {index}: name is not NUL-terminated");
        }
        let name = utils::decode_filename(&name_bytes)
            .with_context(|| format!("Failed to decode filename for ToEE entry {index}"))?;
        if name.is_empty() || name.contains(['/', '\\']) {
            bail!("Invalid ToEE entry {index}: name is not a single path component");
        }

        let read_u32 = |cursor: &mut Cursor<&[u8]>, field: &str| {
            cursor
                .read_u32::<LittleEndian>()
                .with_context(|| format!("Failed to read ToEE entry {index} {field}"))
        };
        let read_i32 = |cursor: &mut Cursor<&[u8]>, field: &str| {
            cursor
                .read_i32::<LittleEndian>()
                .with_context(|| format!("Failed to read ToEE entry {index} {field}"))
        };

        let _unknown = read_u32(cursor, "name pointer")?;
        Ok(ToeeFileEntry {
            name,
            flags: read_u32(cursor, "flags")?,
            real_size: read_u32(cursor, "real size")?,
            packed_size: read_u32(cursor, "packed size")?,
            offset: read_u32(cursor, "data offset")?,
            parent: read_i32(cursor, "parent index")?,
            first_child: read_i32(cursor, "first child index")?,
            next_sibling: read_i32(cursor, "next sibling index")?,
        })
    }

    fn link_index(
        link: i32,
        entry_count: usize,
        label: &str,
        owner: usize,
    ) -> Result<Option<usize>> {
        if link == -1 {
            return Ok(None);
        }
        let index = usize::try_from(link).with_context(|| {
            format!("Invalid ToEE entry {owner}: negative {label} index {link}")
        })?;
        if index >= entry_count {
            bail!("Invalid ToEE entry {owner}: {label} index {link} is out of range");
        }
        Ok(Some(index))
    }

    fn validate_links(entries: &[ToeeFileEntry]) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        for (i, entry) in entries.iter().enumerate() {
            if let Some(parent) = Self::link_index(entry.parent, entries.len(), "parent", i)? {
                if !entries[parent].is_dir() {
                    bail!("Invalid ToEE entry {i}: parent {parent} is not a directory");
                }
            }
            Self::link_index(entry.next_sibling, entries.len(), "sibling", i)?;
            let child = Self::link_index(entry.first_child, entries.len(), "child", i)?;
            if child.is_some() && !entry.is_dir() {
                bail!("Invalid ToEE entry {i}: file has a child link");
            }
        }

        let mut linked = vec![false; entries.len()];
        Self::validate_sibling_chain(entries, Some(0), None, &mut linked)?;
        for (i, entry) in entries.iter().enumerate().filter(|(_, e)| e.is_dir()) {
            let first = Self::link_index(entry.first_child, entries.len(), "child", i)?;
            Self::validate_sibling_chain(entries, first, Some(i), &mut linked)?;
        }
        if let Some(index) = linked.iter().position(|seen| !seen) {
            bail!("Invalid ToEE tree: entry {index} is not linked from its parent");
        }
        Ok(())
    }

    fn validate_sibling_chain(
        entries: &[ToeeFileEntry],
        mut current: Option<usize>,
        expected_parent: Option<usize>,
        linked: &mut [bool],
    ) -> Result<()> {
        let mut chain_seen = HashSet::new();
        while let Some(index) = current {
            if !chain_seen.insert(index) {
                bail!("Invalid ToEE tree: sibling cycle at entry {index}");
            }
            if linked[index] {
                bail!("Invalid ToEE tree: entry {index} appears in multiple sibling chains");
            }
            let actual_parent =
                Self::link_index(entries[index].parent, entries.len(), "parent", index)?;
            if actual_parent != expected_parent {
                bail!("Invalid ToEE tree: entry {index} is linked from the wrong parent");
            }
            linked[index] = true;
            current =
                Self::link_index(entries[index].next_sibling, entries.len(), "sibling", index)?;
        }
        Ok(())
    }

    /// Resolve `index` to its full path, memoizing every ancestor on the way.
    ///
    /// The climb is iterative on purpose: parent depth is bounded only by the
    /// entry count, so recursing once per level overflows the stack on a
    /// crafted archive long before any other check can reject it.
    fn build_full_path(
        index: usize,
        entries: &[ToeeFileEntry],
        paths: &mut [Option<String>],
        visiting: &mut [bool],
    ) -> Result<()> {
        // Climb to the first ancestor that already has a path (or to a root),
        // recording the chain so it can be filled in downward afterwards.
        let mut chain = Vec::new();
        let mut current = Some(index);
        while let Some(i) = current {
            if paths[i].is_some() {
                break;
            }
            if visiting[i] {
                bail!("Invalid ToEE tree: parent cycle at entry {i}");
            }
            visiting[i] = true;
            chain.push(i);
            current = Self::link_index(entries[i].parent, entries.len(), "parent", i)?;
        }

        for &i in chain.iter().rev() {
            let path = match Self::link_index(entries[i].parent, entries.len(), "parent", i)? {
                Some(parent) => {
                    let parent_path = paths[parent]
                        .as_deref()
                        .context("ToEE parent path resolved out of order")?;
                    format!("{parent_path}\\{}", entries[i].name)
                }
                None => entries[i].name.clone(),
            };
            if path.len() > MAX_PATH_BYTES {
                bail!("Invalid ToEE tree: path at entry {i} exceeds {MAX_PATH_BYTES} bytes");
            }
            visiting[i] = false;
            paths[i] = Some(path);
        }
        Ok(())
    }

    pub fn list(&self, files: &[String], format: ListFormat) -> Result<()> {
        let all_files: Vec<&FileEntry> = self.files.iter().collect();
        common::list_files_filtered(&all_files, files, format)
    }

    pub fn extract(&self, output_dir: &Path, files: &[String], mode: ExtractionMode) -> Result<()> {
        let files_to_extract = common::filter_files_by_patterns(&self.files, files)?;
        common::extract_archive_parallel(
            &self.data,
            &files_to_extract,
            output_dir,
            mode,
            common::decompress_zlib,
        )
    }

    fn read_file_data(&self, file: &FileEntry) -> Result<Vec<u8>> {
        utils::read_file_slice(&self.data, file)
    }

    pub fn add_file(
        &mut self,
        file_path: &Path,
        compression: CompressionLevel,
        target_dir: Option<&str>,
        source_root: Option<&Path>,
    ) -> Result<()> {
        common::add_files_zlib(
            &mut self.files,
            file_path,
            compression,
            target_dir,
            source_root,
        )
    }

    pub fn delete_file(&mut self, file_name: &str) -> Result<()> {
        common::delete_file_from_list(&mut self.files, file_name)
    }

    /// Add `raw` and each of its ancestors to the node set.
    ///
    /// `file_index` is `None` for a directory path, which creates the same
    /// nodes but leaves the leaf marked as a directory.
    fn insert_path(
        nodes: &mut Vec<SaveNode>,
        keys: &mut HashMap<String, usize>,
        raw: &str,
        file_index: Option<usize>,
    ) -> Result<()> {
        let normalized = utils::normalize_path_for_archive(raw);
        utils::validate_archive_path(&normalized)?;
        let parts: Vec<&str> = normalized.split('\\').collect();
        if parts.iter().any(|part| part.is_empty()) {
            bail!("Invalid ToEE archive path: {raw}");
        }

        let mut full_name = String::new();
        for (part_index, part) in parts.iter().enumerate() {
            if part.len() + 1 > MAX_COMPONENT_BYTES {
                bail!(
                    "ToEE path component is longer than {} bytes: {part}",
                    MAX_COMPONENT_BYTES - 1
                );
            }
            if !full_name.is_empty() {
                full_name.push('\\');
            }
            full_name.push_str(part);
            if full_name.len() > MAX_PATH_BYTES {
                bail!(
                    "ToEE archive path is longer than {MAX_PATH_BYTES} bytes: {}",
                    utils::normalize_path_for_display(&full_name)
                );
            }
            let key = full_name.to_lowercase();
            // Only the last component is the thing being inserted; the rest are
            // the directories that have to exist above it.
            let leaf_file = if part_index + 1 == parts.len() {
                file_index
            } else {
                None
            };

            if let Some(existing) = keys.get(&key).copied() {
                let existing_is_file = nodes[existing].file_index.is_some();
                if leaf_file.is_some() || existing_is_file {
                    let existing_name = nodes[existing].full_name.clone();
                    if existing_name != full_name {
                        bail!(
                            "ToEE archive paths are case-insensitive: {} collides with {}",
                            utils::normalize_path_for_display(&full_name),
                            utils::normalize_path_for_display(&existing_name)
                        );
                    }
                    bail!(
                        "Conflicting ToEE archive path: {}",
                        utils::normalize_path_for_display(&full_name)
                    );
                }
                continue;
            }

            keys.insert(key, nodes.len());
            nodes.push(SaveNode {
                full_name: full_name.clone(),
                name: (*part).to_string(),
                file_index: leaf_file,
                parent: None,
                first_child: None,
                next_sibling: None,
            });
        }
        Ok(())
    }

    fn build_save_tree(&self) -> Result<Vec<SaveNode>> {
        let mut nodes: Vec<SaveNode> = Vec::new();
        let mut keys = HashMap::<String, usize>::new();

        // Stored directories go in first so that ones with no files under them
        // survive; the file pass then reuses them as ancestors.
        for dir in &self.dirs {
            Self::insert_path(&mut nodes, &mut keys, dir, None)?;
        }
        for (file_index, file) in self.files.iter().enumerate() {
            Self::insert_path(&mut nodes, &mut keys, &file.name, Some(file_index))?;
        }

        nodes.sort_by_cached_key(|node| node.full_name.to_lowercase());
        let index_by_name: HashMap<String, usize> = nodes
            .iter()
            .enumerate()
            .map(|(i, node)| (node.full_name.to_lowercase(), i))
            .collect();

        for node in &mut nodes {
            node.parent = node
                .full_name
                .rsplit_once('\\')
                .map(|(parent, _)| index_by_name[&parent.to_lowercase()]);
        }

        let mut first_by_parent = HashMap::<Option<usize>, usize>::new();
        let mut last_by_parent = HashMap::<Option<usize>, usize>::new();
        for i in 0..nodes.len() {
            let parent = nodes[i].parent;
            first_by_parent.entry(parent).or_insert(i);
            if let Some(previous) = last_by_parent.insert(parent, i) {
                nodes[previous].next_sibling = Some(i);
            }
        }
        for (parent, first_child) in first_by_parent {
            if let Some(parent) = parent {
                nodes[parent].first_child = Some(first_child);
            }
        }

        Ok(nodes)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let total_payload: u64 = self.files.iter().map(|f| f.packed_size as u64).sum();
        if total_payload > u32::MAX as u64 - 4 {
            bail!("ToEE archive would exceed the format's 4 GiB offset limit");
        }

        let nodes = self.build_save_tree()?;
        if nodes.len() > i32::MAX as usize {
            bail!("ToEE archive has too many entries for signed tree indices");
        }

        utils::write_atomically(path, |out| {
            let mut file_offsets = vec![0u32; self.files.len()];
            let mut current_offset = 0u32;
            for node in &nodes {
                let Some(file_index) = node.file_index else {
                    continue;
                };
                let file = &self.files[file_index];
                let owned;
                let bytes: &[u8] = match file.data {
                    Some(ref data) => data,
                    None => {
                        owned = self.read_file_data(file)?;
                        &owned
                    }
                };
                if bytes.len() != file.packed_size as usize {
                    bail!("Stored size does not match data for {}", file.name);
                }
                file_offsets[file_index] = current_offset;
                out.write_all(bytes)?;
                current_offset += file.packed_size;
            }

            out.write_u32::<LittleEndian>(current_offset + 4)?;
            out.write_u32::<LittleEndian>(u32::try_from(nodes.len()).unwrap())?;
            let mut table_size = 4u64;
            let mut names_len = 0u64;

            let link = |index: Option<usize>| -> i32 {
                index.map(|i| i32::try_from(i).unwrap()).unwrap_or(-1)
            };
            for node in &nodes {
                let mut name_bytes = node.name.as_bytes().to_vec();
                name_bytes.push(0);
                out.write_u32::<LittleEndian>(u32::try_from(name_bytes.len()).unwrap())?;
                out.write_all(&name_bytes)?;
                out.write_u32::<LittleEndian>(0)?; // original tools wrote an in-memory pointer

                if let Some(file_index) = node.file_index {
                    let file = &self.files[file_index];
                    out.write_u32::<LittleEndian>(if file.compressed {
                        FLAG_ZLIB
                    } else {
                        FLAG_RAW
                    })?;
                    out.write_u32::<LittleEndian>(file.size)?;
                    out.write_u32::<LittleEndian>(file.packed_size)?;
                    out.write_u32::<LittleEndian>(file_offsets[file_index])?;
                } else {
                    out.write_u32::<LittleEndian>(FLAG_DIR)?;
                    out.write_u32::<LittleEndian>(0)?;
                    out.write_u32::<LittleEndian>(0)?;
                    out.write_u32::<LittleEndian>(0)?;
                }
                out.write_i32::<LittleEndian>(link(node.parent))?;
                out.write_i32::<LittleEndian>(link(node.first_child))?;
                out.write_i32::<LittleEndian>(link(node.next_sibling))?;

                names_len += name_bytes.len() as u64;
                table_size += ENTRY_FIXED_BYTES + name_bytes.len() as u64;
            }

            let footer_size = match self.version {
                ToeeVersion::V0 => {
                    out.write_all(&V0_MAGIC)?;
                    V0_FOOTER_SIZE
                }
                ToeeVersion::V1 => {
                    out.write_all(&self.guid)?;
                    out.write_all(&MAGIC)?;
                    FOOTER_SIZE
                }
            };
            out.write_u32::<LittleEndian>(
                u32::try_from(names_len).context("ToEE archive filenames exceed the u32 limit")?,
            )?;
            out.write_u32::<LittleEndian>(
                u32::try_from(table_size + footer_size as u64)
                    .context("ToEE entry table exceeds the u32 limit")?,
            )?;
            Ok(())
        })
        .context("Failed to write ToEE DAT file")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScratchPath;
    use proptest::prelude::*;

    fn raw_file(name: &str, data: &[u8]) -> FileEntry {
        let mut file = FileEntry::with_data(name.to_string(), data.to_vec(), false);
        file.size = data.len() as u32;
        file
    }

    #[test]
    fn save_writes_hierarchical_reference_layout() {
        let mut archive = ToeeArchive::new();
        archive.files.push(raw_file("b\\y.txt", b"yy"));
        archive.files.push(raw_file("A\\sub\\x.txt", b"xxx"));

        let path = ScratchPath::new("toee_layout");
        archive.save(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(is_toee_format(&bytes));

        let footer_start = bytes.len() - FOOTER_SIZE;
        let distance = u32::from_le_bytes(bytes[bytes.len() - 4..].try_into().unwrap()) as usize;
        let table_start = bytes.len() - distance;
        assert_eq!(&bytes[..5], b"xxxyy");
        assert_eq!(
            &bytes[table_start - 4..table_start],
            &(table_start as u32).to_le_bytes()
        );

        let mut cursor = Cursor::new(&bytes[table_start..footer_start]);
        assert_eq!(cursor.read_u32::<LittleEndian>().unwrap(), 5);
        let mut seen = Vec::new();
        for _ in 0..5 {
            let len = cursor.read_u32::<LittleEndian>().unwrap() as usize;
            let mut name = vec![0; len];
            cursor.read_exact(&mut name).unwrap();
            let name = utils::decode_filename(&name).unwrap();
            let unknown = cursor.read_u32::<LittleEndian>().unwrap();
            let flags = cursor.read_u32::<LittleEndian>().unwrap();
            let real = cursor.read_u32::<LittleEndian>().unwrap();
            let packed = cursor.read_u32::<LittleEndian>().unwrap();
            let offset = cursor.read_u32::<LittleEndian>().unwrap();
            let parent = cursor.read_i32::<LittleEndian>().unwrap();
            let child = cursor.read_i32::<LittleEndian>().unwrap();
            let sibling = cursor.read_i32::<LittleEndian>().unwrap();
            seen.push((
                name, unknown, flags, real, packed, offset, parent, child, sibling,
            ));
        }

        assert_eq!(
            seen.iter()
                .map(|entry| entry.0.as_str())
                .collect::<Vec<_>>(),
            vec!["A", "sub", "x.txt", "b", "y.txt"]
        );
        assert_eq!(seen[0], ("A".to_string(), 0, FLAG_DIR, 0, 0, 0, -1, 1, 3));
        assert_eq!(seen[1], ("sub".to_string(), 0, FLAG_DIR, 0, 0, 0, 0, 2, -1));
        assert_eq!(
            seen[2],
            ("x.txt".to_string(), 0, FLAG_RAW, 3, 3, 0, 1, -1, -1)
        );
        assert_eq!(seen[3], ("b".to_string(), 0, FLAG_DIR, 0, 0, 0, -1, 4, -1));
        assert_eq!(
            seen[4],
            ("y.txt".to_string(), 0, FLAG_RAW, 2, 2, 3, 3, -1, -1)
        );
    }

    #[test]
    fn save_parse_extract_round_trip() {
        let payload = b"compress this ToEE payload ".repeat(30);
        let mut archive = ToeeArchive::new();
        archive.files.push(raw_file("root.txt", b"root"));
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(9));
        encoder.write_all(&payload).unwrap();
        let compressed = encoder.finish().unwrap();
        archive.files.push(FileEntry::with_compression_data(
            "dir\\packed.bin".to_string(),
            payload.clone(),
            compressed,
        ));

        let path = ScratchPath::new("toee_roundtrip");
        archive.save(&path).unwrap();
        let parsed = ToeeArchive::from_bytes(std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(parsed.files.len(), 2);

        let out = ScratchPath::new("toee_extract");
        parsed
            .extract(&out, &[], ExtractionMode::PreserveStructure)
            .unwrap();
        assert_eq!(std::fs::read(out.join("root.txt")).unwrap(), b"root");
        assert_eq!(std::fs::read(out.join("dir/packed.bin")).unwrap(), payload);
    }

    #[test]
    fn preserves_guid_across_resave() {
        let mut archive = ToeeArchive::new();
        archive.guid = std::array::from_fn(|i| i as u8 + 1);
        archive.files.push(raw_file("A.TXT", b"a"));
        let first = ScratchPath::new("toee_guid1");
        archive.save(&first).unwrap();
        let parsed = ToeeArchive::from_bytes(std::fs::read(&first).unwrap()).unwrap();
        let second = ScratchPath::new("toee_guid2");
        parsed.save(&second).unwrap();
        let bytes = std::fs::read(&second).unwrap();
        assert_eq!(
            &bytes[bytes.len() - FOOTER_SIZE..bytes.len() - 12],
            archive.guid
        );
    }

    #[test]
    fn detector_rejects_arcanum_table_shape() {
        let mut archive = ToeeArchive::new();
        archive.files.push(raw_file("A.TXT", b"a"));
        let path = ScratchPath::new("toee_detect");
        archive.save(&path).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        assert!(is_toee_format(&bytes));

        let count = 1u32;
        let names = 6u32;
        let arcanum_distance = FOOTER_SIZE as u32 + 4 + names + count * 24;
        let len = bytes.len();
        bytes[len - 8..len - 4].copy_from_slice(&names.to_le_bytes());
        bytes[len - 4..].copy_from_slice(&arcanum_distance.to_le_bytes());
        assert!(!is_toee_format(&bytes));
    }

    #[test]
    fn opens_original_v0_footer_and_preserves_it_on_save() {
        // OpenTemple's empty V0 reference fixture: count, "DAT " signature
        // (stored little-endian as " TAD"), filename bytes, table distance.
        let mut empty = Vec::new();
        empty.extend_from_slice(&0u32.to_le_bytes());
        empty.extend_from_slice(&V0_MAGIC);
        empty.extend_from_slice(&0u32.to_le_bytes());
        empty.extend_from_slice(&16u32.to_le_bytes());
        assert!(is_toee_format(&empty));
        let open_path = ScratchPath::new("toee_v0_open");
        std::fs::write(&open_path, &empty).unwrap();
        assert!(matches!(
            crate::common::DatArchive::open(&open_path).unwrap(),
            crate::common::DatArchive::Toee(_)
        ));

        let parsed = ToeeArchive::from_bytes(empty).unwrap();
        assert_eq!(parsed.version, ToeeVersion::V0);
        let path = ScratchPath::new("toee_v0");
        parsed.save(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[bytes.len() - 12..bytes.len() - 8], &V0_MAGIC);
        assert_eq!(
            ToeeArchive::from_bytes(bytes).unwrap().version,
            ToeeVersion::V0
        );
    }

    #[test]
    fn parser_rejects_broken_tree_link() {
        let mut archive = ToeeArchive::new();
        archive.files.push(raw_file("A\\B.TXT", b"b"));
        let path = ScratchPath::new("toee_badlink");
        archive.save(&path).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        let distance = u32::from_le_bytes(bytes[bytes.len() - 4..].try_into().unwrap()) as usize;
        let table = bytes.len() - distance;
        let first_name_len =
            u32::from_le_bytes(bytes[table + 4..table + 8].try_into().unwrap()) as usize;
        let first_child = table + 8 + first_name_len + 24;
        bytes[first_child..first_child + 4].copy_from_slice(&9999i32.to_le_bytes());
        assert!(ToeeArchive::from_bytes(bytes).is_err());
    }

    /// A valid archive whose parent chain is `depth` levels deep, laid out so
    /// that resolving entry 1 has to climb the whole chain in one go.
    fn deep_chain_archive(depth: usize) -> Vec<u8> {
        let mut table = (depth as u32).to_le_bytes().to_vec();
        for i in 0..depth {
            let (name, flags, parent, child) = match i {
                0 => ("r", FLAG_DIR, -1i32, (depth - 1) as i32),
                1 => ("f", FLAG_RAW, 2i32, -1i32),
                _ if i == depth - 1 => ("d", FLAG_DIR, 0i32, (i - 1) as i32),
                _ => ("d", FLAG_DIR, (i + 1) as i32, (i - 1) as i32),
            };
            let mut name_bytes = name.as_bytes().to_vec();
            name_bytes.push(0);
            table.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
            table.extend_from_slice(&name_bytes);
            for word in [0u32, flags, 0, 0, 0] {
                table.extend_from_slice(&word.to_le_bytes());
            }
            for link in [parent, child, -1i32] {
                table.extend_from_slice(&link.to_le_bytes());
            }
        }

        let mut out = 4u32.to_le_bytes().to_vec();
        out.extend_from_slice(&table);
        out.extend_from_slice(&[0u8; 16]);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&((depth * 2) as u32).to_le_bytes());
        out.extend_from_slice(&((table.len() + FOOTER_SIZE) as u32).to_le_bytes());
        out
    }

    #[test]
    fn rejects_a_deep_chain_without_overflowing_the_stack() {
        // Run on a deliberately small stack. A recursive walk descends the whole
        // 4000-level chain before building any path, so it dies on the stack
        // before the length cap can speak; the iterative walk climbs in a loop
        // and reports the over-long path as an ordinary error.
        let bytes = deep_chain_archive(4000);
        let error = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || ToeeArchive::from_bytes(bytes))
            .unwrap()
            .join()
            .unwrap()
            .unwrap_err()
            .to_string();
        assert!(error.contains(&MAX_PATH_BYTES.to_string()), "{error}");
    }

    #[test]
    fn parses_a_chain_that_stays_under_the_path_cap() {
        let bytes = deep_chain_archive(400);
        let parsed = ToeeArchive::from_bytes(bytes).unwrap();
        assert_eq!(parsed.files.len(), 1);
        assert_eq!(parsed.files[0].name.matches('\\').count(), 399);
    }

    #[test]
    fn refuses_to_write_a_path_over_the_cap() {
        let mut archive = ToeeArchive::new();
        let deep = vec!["dir"; MAX_PATH_BYTES / 4 + 1].join("\\");
        archive
            .files
            .push(raw_file(&format!("{deep}\\x.txt"), b"x"));
        let path = ScratchPath::new("toee_long_path");
        let error = archive.save(&path).unwrap_err().to_string();
        assert!(error.contains(&MAX_PATH_BYTES.to_string()), "{error}");
    }

    #[test]
    fn keeps_directories_that_no_longer_hold_files() {
        let mut archive = ToeeArchive::new();
        archive.files.push(raw_file("keep\\a.txt", b"a"));
        archive.files.push(raw_file("gone\\b.txt", b"b"));
        let first = ScratchPath::new("toee_dirs1");
        archive.save(&first).unwrap();

        let mut parsed = ToeeArchive::from_bytes(std::fs::read(&first).unwrap()).unwrap();
        assert_eq!(parsed.dirs, vec!["gone".to_string(), "keep".to_string()]);

        parsed.delete_file("gone\\b.txt").unwrap();
        let second = ScratchPath::new("toee_dirs2");
        parsed.save(&second).unwrap();

        let reparsed = ToeeArchive::from_bytes(std::fs::read(&second).unwrap()).unwrap();
        assert_eq!(reparsed.files.len(), 1);
        assert!(
            reparsed.dirs.contains(&"gone".to_string()),
            "{:?}",
            reparsed.dirs
        );
    }

    #[test]
    fn names_a_case_collision_as_the_cause() {
        let mut archive = ToeeArchive::new();
        archive.files.push(raw_file("A.TXT", b"a"));
        archive.files.push(raw_file("a.txt", b"b"));
        let path = ScratchPath::new("toee_case");
        let error = archive.save(&path).unwrap_err().to_string();
        assert!(error.contains("case-insensitive"), "{error}");
    }

    #[test]
    fn round_trips_a_v0_archive_holding_files() {
        let mut archive = ToeeArchive::new();
        archive.version = ToeeVersion::V0;
        archive.files.push(raw_file("dir\\v0.txt", b"v0"));
        let path = ScratchPath::new("toee_v0_files");
        archive.save(&path).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert!(is_toee_format(&bytes));
        assert_eq!(&bytes[bytes.len() - 12..bytes.len() - 8], &V0_MAGIC);
        let parsed = ToeeArchive::from_bytes(bytes).unwrap();
        assert_eq!(parsed.version, ToeeVersion::V0);
        assert_eq!(parsed.files.len(), 1);
        assert_eq!(
            utils::read_file_slice(&parsed.data, &parsed.files[0]).unwrap(),
            b"v0"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn save_then_parse_round_trips(
            payloads in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..256), 1..8)
        ) {
            let mut archive = ToeeArchive::new();
            for (i, data) in payloads.iter().enumerate() {
                let name = if i % 2 == 0 { format!("F{i}.BIN") } else { format!("SUB\\F{i}.BIN") };
                archive.files.push(raw_file(&name, data));
            }
            let path = ScratchPath::new("prop_toee");
            archive.save(&path).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            prop_assert!(is_toee_format(&bytes));
            let parsed = ToeeArchive::from_bytes(bytes).unwrap();
            prop_assert_eq!(parsed.files.len(), payloads.len());
            for (i, data) in payloads.iter().enumerate() {
                let name = if i % 2 == 0 { format!("F{i}.BIN") } else { format!("SUB\\F{i}.BIN") };
                let file = parsed.files.iter().find(|file| file.name == name).unwrap();
                prop_assert_eq!(utils::read_file_slice(&parsed.data, file).unwrap(), data.clone());
            }
        }

        #[test]
        fn from_bytes_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
            let _ = ToeeArchive::from_bytes(bytes);
        }
    }
}
