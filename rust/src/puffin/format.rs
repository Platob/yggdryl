//! Puffin file framing: the magic, the footer, its flags, and its payload.
//!
//! The file is `Magic Blob… Footer`, and the footer is
//! `Magic FooterPayload FooterPayloadSize Flags Magic` - the payload size a
//! 4-byte little-endian signed integer, the flags 4 bytes with bit 0 of byte 0
//! marking an LZ4-compressed payload. The payload itself is one UTF-8 JSON
//! `FileMetadata` object - a `blobs` list and optional string `properties` -
//! decoded through [`crate::json`] exactly the way the Iceberg module decodes
//! its metadata documents. An LZ4-compressed payload is refused by name: this
//! build takes no LZ4 dependency, and a silently skipped footer would be a
//! silently invisible file.

use smol_str::{SmolStr, format_smolstr};

use crate::io::IOBase;
use crate::{Result, Value};

use super::bitmap::{codec, invalid};
use super::blob::BlobMetadata;

/// The four magic bytes at the head of the file and both ends of the footer.
pub(crate) const MAGIC: [u8; 4] = [0x50, 0x46, 0x41, 0x31];

/// Bytes after the payload: `FooterPayloadSize`, `Flags`, and closing magic.
const FOOTER_TAIL: u64 = 12;

/// Bit 0 of flag byte 0: the footer payload is LZ4-compressed.
const FLAG_FOOTER_COMPRESSED: u8 = 0x01;

/// The decoded footer payload: what the file holds and who wrote it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileMetadata {
    /// Every blob in the file, in footer order.
    pub blobs: Vec<BlobMetadata>,
    /// File-level string properties, such as `created-by`.
    pub properties: Vec<(SmolStr, SmolStr)>,
}

impl FileMetadata {
    /// Return one file property by key.
    pub fn get_property(&self, key: &str) -> Option<&str> {
        self.properties
            .iter()
            .find_map(|(name, value)| (name == key).then(|| value.as_str()))
    }

    /// Read the `FileMetadata` object a footer payload holds.
    ///
    /// # Errors
    ///
    /// Returns an error when the document is not an object with a `blobs`
    /// list, or when one blob entry breaks the spec, located by its index.
    pub fn from_json(document: &Value) -> Result<Self> {
        let blobs_json = document
            .get_key_str("blobs")
            .and_then(Value::as_sequence)
            .ok_or_else(|| {
                invalid(SmolStr::new_static(
                    "expected a \"blobs\" list in the footer payload",
                ))
            })?;
        let mut blobs = Vec::with_capacity(blobs_json.len());
        for (index, blob) in blobs_json.iter().enumerate() {
            blobs.push(BlobMetadata::from_json(blob).map_err(|error| locate(error, index))?);
        }
        let properties = match document.get_key_str("properties") {
            None => Vec::new(),
            Some(properties) => string_entries(properties, "footer \"properties\"")?,
        };
        Ok(Self { blobs, properties })
    }

    /// Write this metadata as the footer payload object.
    ///
    /// # Errors
    ///
    /// Returns an error when a blob's location exceeds a JSON long.
    pub fn to_json(&self) -> Result<Value> {
        let mut entries = vec![(
            Value::from("blobs"),
            Value::from_sequence(
                self.blobs
                    .iter()
                    .map(BlobMetadata::to_json)
                    .collect::<Result<Vec<_>>>()?,
            ),
        )];
        if !self.properties.is_empty() {
            entries.push((
                Value::from("properties"),
                Value::from_mapping(
                    self.properties
                        .iter()
                        .map(|(key, value)| (Value::from(key.clone()), Value::from(value.clone()))),
                )?,
            ));
        }
        Value::from_mapping(entries)
    }
}

/// Read a JSON object of string values into ordered entries.
pub(crate) fn string_entries(document: &Value, what: &str) -> Result<Vec<(SmolStr, SmolStr)>> {
    let mapping = document
        .as_mapping()
        .ok_or_else(|| invalid(format_smolstr!("expected a {what} object")))?;
    let mut entries = Vec::with_capacity(mapping.len());
    for (key, value) in mapping {
        let (Some(key), Some(value)) = (key.as_str(), value.as_str()) else {
            return Err(invalid(format_smolstr!(
                "expected string {what} entries, got a non-string entry"
            )));
        };
        entries.push((SmolStr::new(key), SmolStr::new(value)));
    }
    Ok(entries)
}

/// Locate a blob metadata error at its index in the footer's list.
fn locate(error: crate::Error, index: usize) -> crate::Error {
    match error {
        crate::Error::Codec {
            format,
            position,
            reason,
        } => crate::Error::Codec {
            format,
            position,
            reason: format_smolstr!("blobs[{index}]: {reason}"),
        },
        other => other,
    }
}

/// Read and validate the footer of a non-empty Puffin file.
///
/// Returns the decoded payload and the offset of the footer's opening magic,
/// which is where the blob region ends and where an appending writer puts the
/// next blob's bytes.
///
/// # Errors
///
/// Returns an error naming the expected and actual bytes at their offset when
/// any magic, size, or flag rule is broken; an LZ4-compressed payload is
/// refused naming the codec.
pub(crate) fn read_footer<H: IOBase>(handle: &H) -> Result<(FileMetadata, u64)> {
    let size = handle.size();
    // Head magic, footer magic, empty payload, size, flags, tail magic.
    let minimum = 4 + 4 + FOOTER_TAIL;
    if size < minimum {
        return Err(invalid(format_smolstr!(
            "expected a Puffin file of at least {minimum} bytes, got {size}"
        )));
    }
    check_magic(handle, 0, "at the head of the file")?;
    let mut tail = [0_u8; 12];
    handle.pread_exact(size - FOOTER_TAIL, &mut tail)?;
    let payload_size = i32::from_le_bytes([tail[0], tail[1], tail[2], tail[3]]);
    let payload_size = u64::try_from(payload_size).map_err(|_| {
        codec(
            usize::try_from(size - FOOTER_TAIL).unwrap_or(usize::MAX),
            format_smolstr!("expected a non-negative footer payload size, got {payload_size}"),
        )
    })?;
    if tail[8..12] != MAGIC {
        return Err(magic_error(
            size - 4,
            "at the end of the footer",
            &tail[8..12],
        ));
    }
    let flags = &tail[4..8];
    if flags[0] & FLAG_FOOTER_COMPRESSED != 0 {
        return Err(invalid(SmolStr::new_static(
            "expected an uncompressed footer payload, got compression codec \"lz4\" (this build implements no LZ4)",
        )));
    }
    if flags[0] & !FLAG_FOOTER_COMPRESSED != 0 || flags[1..] != [0, 0, 0] {
        return Err(codec(
            usize::try_from(size - 8).unwrap_or(usize::MAX),
            format_smolstr!(
                "expected zero reserved footer flags, got {:02x}{:02x}{:02x}{:02x}",
                flags[0],
                flags[1],
                flags[2],
                flags[3]
            ),
        ));
    }
    let footer_offset = (size - FOOTER_TAIL)
        .checked_sub(payload_size + 4)
        .filter(|offset| *offset >= 4)
        .ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a footer payload fitting a {size}-byte file, got {payload_size} bytes"
            ))
        })?;
    check_magic(handle, footer_offset, "at the start of the footer")?;
    let payload = handle.read_range(
        footer_offset + 4,
        usize::try_from(payload_size).unwrap_or(usize::MAX),
    )?;
    let document = crate::json::from_slice(&payload)?;
    Ok((FileMetadata::from_json(&document)?, footer_offset))
}

/// Encode a complete footer: magic, payload, size, flags, magic.
///
/// The payload is written uncompressed, so the flag bytes are all zero; the
/// spec's only flag selects LZ4, which this build refuses rather than emits.
///
/// # Errors
///
/// Returns an error when a blob's location exceeds a JSON long or the payload
/// exceeds the 4-byte signed size field.
pub(crate) fn footer_bytes(metadata: &FileMetadata) -> Result<Vec<u8>> {
    let payload = crate::json::to_vec(&metadata.to_json()?)?;
    let payload_size = i32::try_from(payload.len()).map_err(|_| {
        invalid(format_smolstr!(
            "expected a footer payload of at most {} bytes, got {}",
            i32::MAX,
            payload.len()
        ))
    })?;
    let mut footer = Vec::with_capacity(4 + payload.len() + 12);
    footer.extend_from_slice(&MAGIC);
    footer.extend_from_slice(&payload);
    footer.extend_from_slice(&payload_size.to_le_bytes());
    footer.extend_from_slice(&[0, 0, 0, 0]);
    footer.extend_from_slice(&MAGIC);
    Ok(footer)
}

/// Check four magic bytes at an offset, naming the place on failure.
fn check_magic<H: IOBase>(handle: &H, offset: u64, place: &str) -> Result<()> {
    let mut bytes = [0_u8; 4];
    handle.pread_exact(offset, &mut bytes)?;
    if bytes == MAGIC {
        Ok(())
    } else {
        Err(magic_error(offset, place, &bytes))
    }
}

/// Report absent magic bytes as expected-versus-got at their offset.
fn magic_error(offset: u64, place: &str, actual: &[u8]) -> crate::Error {
    codec(
        usize::try_from(offset).unwrap_or(usize::MAX),
        format_smolstr!(
            "expected magic \"PFA1\" {place}, got {:02x}{:02x}{:02x}{:02x}",
            actual[0],
            actual[1],
            actual[2],
            actual[3]
        ),
    )
}
