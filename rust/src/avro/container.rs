//! The Avro object container: magic, header, blocks, and sync markers.

use smol_str::{SmolStr, format_smolstr};

use crate::io::IOBase;
use crate::{Codec, Level, Result, Value};

use super::datum::{Cursor, block_count, codec, decode, encode, invalid, put_bytes, put_long};
use super::schema::Schema;

/// The four bytes that open every Avro object container.
const MAGIC: [u8; 4] = [b'O', b'b', b'j', 1];

/// The header key naming the writer schema.
const SCHEMA_KEY: &str = "avro.schema";

/// The header key naming the block compression codec.
const CODEC_KEY: &str = "avro.codec";

/// Length of the synchronization marker that closes every block.
const SYNC_LEN: usize = 16;

/// One decoded Avro object container.
#[derive(Debug)]
pub struct Container {
    /// The header's key/value metadata, minus the reserved schema and codec.
    pub metadata: Vec<(SmolStr, SmolStr)>,
    /// Every decoded row, in file order.
    pub rows: Vec<Value>,
}

impl Container {
    /// Return one metadata value by key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.metadata
            .iter()
            .find_map(|(name, value)| (name == key).then(|| value.as_str()))
    }
}

/// Read every row of the Avro object container a handle holds.
///
/// A small self-describing file - an Iceberg manifest describes files rather
/// than rows - is read whole; the streaming that matters is one level up, over
/// the data files a manifest points at.
///
/// # Errors
///
/// Returns an error when the bytes are not an Avro object container, when the
/// codec is one this build does not implement, or when a row does not decode.
pub fn read_container<H: IOBase + ?Sized>(handle: &H) -> Result<Container> {
    let bytes = handle.read_all()?;
    let mut cursor = Cursor::new(&bytes);

    let magic = cursor.take(MAGIC.len())?;
    if magic != MAGIC {
        return Err(invalid(format_smolstr!(
            "expected an Avro object container starting with {MAGIC:?}, got {magic:?}"
        )));
    }

    let mut header = Vec::new();
    loop {
        let count = block_count(&mut cursor)?;
        if count == 0 {
            break;
        }
        for _ in 0..count {
            let key = SmolStr::new(std::str::from_utf8(cursor.bytes()?).map_err(|error| {
                codec(
                    cursor.position,
                    format_smolstr!("expected UTF-8 in an Avro header key, got {error}"),
                )
            })?);
            let value = cursor.bytes()?.to_vec();
            header.push((key, value));
        }
    }
    let sync: [u8; SYNC_LEN] = cursor.take(SYNC_LEN)?.try_into().map_err(|_| {
        codec(
            cursor.position,
            SmolStr::new_static("expected a sixteen-byte Avro synchronization marker"),
        )
    })?;

    let lookup = |key: &str| -> Option<&[u8]> {
        header
            .iter()
            .find_map(|(name, value)| (name == key).then_some(value.as_slice()))
    };
    let schema_bytes = lookup(SCHEMA_KEY).ok_or_else(|| {
        invalid(format_smolstr!(
            "expected an Avro header carrying {SCHEMA_KEY:?}"
        ))
    })?;
    let schema_json = crate::json::from_slice(schema_bytes)?;
    let schema = Schema::from_json(&schema_json)?;
    let codec_name = lookup(CODEC_KEY)
        .map(|value| String::from_utf8_lossy(value).into_owned())
        .unwrap_or_else(|| "null".to_owned());
    let block_codec = block_codec(&codec_name)?;

    let mut rows = Vec::new();
    while !cursor.is_exhausted() {
        let count = cursor.long()?;
        let count = u64::try_from(count).map_err(|_| {
            codec(
                cursor.position,
                format_smolstr!("expected a non-negative Avro block count, got {count}"),
            )
        })?;
        let payload = cursor.bytes()?;
        let marker = cursor.take(SYNC_LEN)?;
        if marker != sync {
            return Err(codec(
                cursor.position,
                SmolStr::new_static(
                    "expected the header's synchronization marker after an Avro block",
                ),
            ));
        }
        let decoded = block_codec.load(payload)?;
        let mut block = Cursor::new(&decoded);
        for _ in 0..count {
            rows.push(decode(&schema, &mut block)?);
        }
    }

    let metadata = header
        .into_iter()
        .filter(|(key, _)| key != SCHEMA_KEY && key != CODEC_KEY)
        .map(|(key, value)| (key, SmolStr::new(String::from_utf8_lossy(&value))))
        .collect();

    Ok(Container { metadata, rows })
}

/// Replace a handle's bytes with an Avro object container holding `rows`.
///
/// The schema is an Avro schema as its JSON [`Value`], and its JSON spelling
/// is written into the header verbatim, so attributes this implementation does
/// not model - Iceberg's `field-id` among them - survive byte for byte. Every
/// row is written as one block, compressed with raw deflate, which is what the
/// `deflate` codec name means and what the reference implementations write by
/// default.
///
/// # Errors
///
/// Returns an error when the schema JSON is not a schema, when a row does not
/// fit it, or when the write fails.
pub fn write_container<H: IOBase + ?Sized>(
    handle: &mut H,
    schema_json: &Value,
    metadata: &[(&str, &str)],
    rows: &[Value],
) -> Result<()> {
    let schema = Schema::from_json(schema_json)?;
    let encoded_schema = crate::json::to_vec(schema_json)?;
    let sync = sync_marker();

    let mut payload = Vec::new();
    for row in rows {
        encode(&schema, row, &mut payload)?;
    }
    let compressed = Codec::Deflate.dump_with_level(&payload, Level::DEFAULT)?;

    let mut output = Vec::with_capacity(compressed.len() + 512);
    output.extend_from_slice(&MAGIC);
    put_long(&mut output, metadata.len() as i64 + 2);
    put_bytes(&mut output, SCHEMA_KEY.as_bytes());
    put_bytes(&mut output, &encoded_schema);
    put_bytes(&mut output, CODEC_KEY.as_bytes());
    put_bytes(&mut output, b"deflate");
    for (key, value) in metadata {
        put_bytes(&mut output, key.as_bytes());
        put_bytes(&mut output, value.as_bytes());
    }
    put_long(&mut output, 0);
    output.extend_from_slice(&sync);

    if !rows.is_empty() {
        put_long(&mut output, rows.len() as i64);
        put_bytes(&mut output, &compressed);
        output.extend_from_slice(&sync);
    }

    handle.write_all_bytes(&output)
}

/// Return the content coding one Avro codec name selects.
fn block_codec(name: &str) -> Result<Codec> {
    match name {
        "null" => Ok(Codec::Identity),
        // Avro's "deflate" is the raw stream, with no zlib wrapper.
        "deflate" => Ok(Codec::Deflate),
        "zstandard" => Ok(Codec::Zstd),
        other => Err(invalid(format_smolstr!(
            "expected an Avro block codec this build implements (null, deflate, zstandard), got \
             {other:?}"
        ))),
    }
}

/// Produce a synchronization marker unlikely to occur inside a block.
///
/// The marker only has to be constant within one file and improbable in its
/// data, so hashing process-seeded state is enough and avoids a dependency
/// whose only job would be sixteen bytes.
fn sync_marker() -> [u8; SYNC_LEN] {
    use std::hash::{BuildHasher, Hasher};

    let state = std::collections::hash_map::RandomState::new();
    let mut marker = [0_u8; SYNC_LEN];
    for (half, chunk) in marker.chunks_mut(8).enumerate() {
        let mut hasher = state.build_hasher();
        hasher.write_usize(half);
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default(),
        );
        chunk.copy_from_slice(&hasher.finish().to_le_bytes()[..chunk.len()]);
    }
    marker
}
