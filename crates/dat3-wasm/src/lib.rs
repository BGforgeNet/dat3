/*!
# dat3-wasm

dat3-core compiled to WebAssembly for Node.js and Electron's main process and
workers. `package.sh` turns the module into an npm package with wasm-bindgen's
`nodejs` glue.

Everything works on bytes: the JS side reads and writes files. Names use `/` in
both directions (`\` is accepted too), and are looked up regardless of case,
preferring an exact match.
*/

use dat3_core::common::{CaseMode, CompressionLevel};
use dat3_core::{ArchiveFormat, DatArchive};
use wasm_bindgen::prelude::*;

/// Level `insert` uses when none is given, the dat3 command line's default
const DEFAULT_COMPRESSION: u8 = 1;

/// Games look names up regardless of case; `find_stored_index` still prefers
/// the exact name, so names differing only in case stay reachable.
const CASE: CaseMode = CaseMode::Insensitive;

/// A JS `Error` carrying the whole context chain
fn js_error(error: impl std::fmt::Display) -> JsError {
    JsError::new(&format!("{error:#}"))
}

/// A Fallout (DAT1, DAT2) or Troika (Arcanum, ToEE) archive held in memory
#[wasm_bindgen]
#[derive(Debug)]
pub struct Archive {
    inner: DatArchive,
}

#[wasm_bindgen]
impl Archive {
    /// A new, empty archive in `format`: `"dat1"`, `"dat2"`, `"arcanum"` or `"toee"`
    #[wasm_bindgen(constructor)]
    pub fn new(format: &str) -> Result<Archive, JsError> {
        let format = ArchiveFormat::from_arg_name(format).ok_or_else(|| {
            js_error(format_args!(
                "unsupported archive format {format:?} (expected dat1, dat2, arcanum, or toee)"
            ))
        })?;
        Ok(Self {
            inner: DatArchive::new(format),
        })
    }

    /// Parse archive bytes, detecting the format
    #[wasm_bindgen(js_name = fromBytes)]
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Archive, JsError> {
        let inner = DatArchive::from_bytes(bytes).map_err(js_error)?;
        Ok(Self { inner })
    }

    /// The archive's format: `"dat1"`, `"dat2"`, `"arcanum"` or `"toee"`
    #[wasm_bindgen(getter)]
    pub fn format(&self) -> String {
        self.inner.format().arg_name().to_string()
    }

    /// Every file entry, names `/`-separated in their stored case.
    ///
    /// Plain objects rather than wasm-backed classes, so they survive
    /// `JSON.stringify` and `postMessage` between Electron processes.
    #[wasm_bindgen(unchecked_return_type = "Entry[]")]
    pub fn entries(&self) -> Result<js_sys::Array, JsValue> {
        let array = js_sys::Array::new();
        for entry in self.inner.entries() {
            let object = js_sys::Object::new();
            let fields: [(&str, JsValue); 4] = [
                ("name", entry.name.replace('\\', "/").into()),
                ("size", entry.size.into()),
                ("packedSize", entry.packed_size.into()),
                ("compressed", entry.compressed.into()),
            ];
            for (key, value) in fields {
                js_sys::Reflect::set(&object, &key.into(), &value)?;
            }
            array.push(&object);
        }
        Ok(array)
    }

    /// The decompressed contents of entry `name`
    pub fn read(&self, name: &str) -> Result<Vec<u8>, JsError> {
        self.inner.read(name, CASE).map_err(js_error)
    }

    /// Add `data` as entry `name`, replacing an entry of that name in any case.
    /// `compression` is 0-9 (default 1); DAT1 always stores uncompressed.
    pub fn insert(
        &mut self,
        name: &str,
        data: Vec<u8>,
        compression: Option<u8>,
    ) -> Result<(), JsError> {
        let level =
            CompressionLevel::new(compression.unwrap_or(DEFAULT_COMPRESSION)).map_err(js_error)?;
        self.inner.insert(name, data, level, CASE).map_err(js_error)
    }

    /// Remove entry `name`; returns whether there was one
    pub fn remove(&mut self, name: &str) -> Result<bool, JsError> {
        self.inner.remove(name, CASE).map_err(js_error)
    }

    /// The archive file's bytes, as dat3 would save it
    #[wasm_bindgen(js_name = toBytes)]
    pub fn to_bytes(&self) -> Result<Vec<u8>, JsError> {
        self.inner.to_bytes().map_err(js_error)
    }
}

#[wasm_bindgen(typescript_custom_section)]
const ENTRY_TYPE: &str = r#"
/** One file entry's name and sizes */
export interface Entry {
    /** `/`-separated name in its stored case */
    name: string;
    /** Size of the contents in bytes */
    size: number;
    /** Size as stored in the archive */
    packedSize: number;
    /** Whether the stored data is compressed */
    compressed: boolean;
}
"#;
