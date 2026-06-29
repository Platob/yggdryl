//! Node wrapper for the in-memory binary buffer [`yggdryl_core::Binary`].

use std::collections::{BTreeMap, HashMap};

use napi::bindgen_prelude::Buffer;
use napi_derive::napi;
use yggdryl_core::{Binary as CoreBinary, BinaryType as CoreBinaryType, Io};

use crate::{to_napi_err, BinaryType, Whence};

/// A growable, in-memory binary buffer that also implements the IO surface
/// (`read`/`write`/`seek`/`pread`/`pwrite`/`resize`).
///
/// `read`/`pread` hand back zero-copy `Binary` views and writes copy-on-write, so
/// views stay valid.
#[napi]
pub struct Binary {
    pub(crate) inner: CoreBinary,
}

#[napi]
impl Binary {
    #[napi(constructor)]
    pub fn new(data: Option<Buffer>, large: Option<bool>) -> Self {
        let mut inner = match data {
            Some(bytes) => CoreBinary::from_bytes(bytes.as_ref()),
            None => CoreBinary::new(),
        };
        if large.unwrap_or(false) {
            inner = inner.with_data_type(CoreBinaryType::large());
        }
        Binary { inner }
    }

    /// The buffer's data type (always a `BinaryType`).
    #[napi(getter)]
    pub fn data_type(&self) -> BinaryType {
        BinaryType {
            inner: self.inner.binary_type(),
        }
    }

    /// Returns a copy carrying the given `binary` type variant.
    #[napi]
    pub fn with_data_type(&self, data_type: &BinaryType) -> Binary {
        Binary {
            inner: self.inner.with_data_type(data_type.inner),
        }
    }

    /// The buffer's raw bytes.
    #[napi]
    pub fn to_bytes(&self) -> Buffer {
        self.inner.to_bytes().into()
    }

    /// A `binary` buffer holding a copy of `data`.
    #[napi(factory)]
    pub fn from_bytes(data: Buffer) -> Binary {
        Binary {
            inner: CoreBinary::from_bytes(data.as_ref()),
        }
    }

    /// The component map (`type`, plus `value` as hex).
    #[napi]
    pub fn to_mapping(&self) -> HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }

    /// Reconstructs a buffer from its component map.
    #[napi(factory)]
    pub fn from_mapping(mapping: HashMap<String, String>) -> napi::Result<Binary> {
        let mapping: BTreeMap<String, String> = mapping.into_iter().collect();
        CoreBinary::from_mapping(&mapping)
            .map(|inner| Binary { inner })
            .map_err(to_napi_err)
    }

    /// The JSON value (used by `JSON.stringify`).
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.inner).expect("Binary serializes to JSON")
    }

    /// Reconstructs a buffer from its JSON value.
    #[napi(js_name = "fromJSON", factory)]
    pub fn from_json(value: serde_json::Value) -> napi::Result<Binary> {
        serde_json::from_value(value)
            .map(|inner| Binary { inner })
            .map_err(to_napi_err)
    }

    // --- IO surface ---

    /// The number of valid bytes.
    #[napi(getter)]
    pub fn size(&self) -> f64 {
        self.inner.size() as f64
    }

    /// The number of bytes (alias of `size`).
    #[napi(getter)]
    pub fn length(&self) -> f64 {
        self.inner.len() as f64
    }

    /// The allocated capacity in bytes.
    #[napi(getter)]
    pub fn capacity(&self) -> f64 {
        self.inner.capacity() as f64
    }

    /// The current cursor position.
    #[napi]
    pub fn tell(&self) -> f64 {
        self.inner.tell() as f64
    }

    /// Moves the cursor; returns the new position.
    #[napi]
    pub fn seek(&mut self, offset: f64, whence: Option<Whence>) -> napi::Result<f64> {
        self.inner
            .seek(offset as i64, whence.unwrap_or(Whence::Start).into())
            .map(|pos| pos as f64)
            .map_err(to_napi_err)
    }

    /// Positional read of up to `length` bytes at `offset` (a zero-copy view).
    #[napi]
    pub fn pread(&self, offset: f64, length: f64) -> napi::Result<Binary> {
        self.inner
            .pread(offset as u64, length as usize)
            .map(|inner| Binary { inner })
            .map_err(to_napi_err)
    }

    /// Cursor read of up to `length` bytes; advances the cursor.
    #[napi]
    pub fn read(&mut self, length: f64) -> napi::Result<Binary> {
        self.inner
            .read(length as usize)
            .map(|inner| Binary { inner })
            .map_err(to_napi_err)
    }

    /// Positional write at `offset`, growing the buffer if needed.
    #[napi]
    pub fn pwrite(&mut self, offset: f64, data: Buffer) -> napi::Result<f64> {
        self.inner
            .pwrite(offset as u64, data.as_ref())
            .map(|written| written as f64)
            .map_err(to_napi_err)
    }

    /// Cursor write; advances the cursor.
    #[napi]
    pub fn write(&mut self, data: Buffer) -> napi::Result<f64> {
        self.inner
            .write(data.as_ref())
            .map(|written| written as f64)
            .map_err(to_napi_err)
    }

    /// Sets the allocated capacity.
    #[napi]
    pub fn set_capacity(&mut self, capacity: f64) -> napi::Result<()> {
        self.inner
            .set_capacity(capacity as u64)
            .map_err(to_napi_err)
    }

    /// Resizes the logical length, filling new bytes with `fill` (default 0).
    #[napi]
    pub fn resize(&mut self, new_size: f64, fill: Option<u8>) -> napi::Result<()> {
        self.inner
            .resize(new_size as u64, fill.unwrap_or(0))
            .map_err(to_napi_err)
    }

    /// Structural equality (content + type) with another `Binary`.
    #[napi]
    pub fn equals(&self, other: &Binary) -> bool {
        self.inner == other.inner
    }
}
