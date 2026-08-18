//! Single-object encoding: one datum framed by its schema fingerprint.
//!
//! A message system that cannot afford a whole container header per record
//! frames each datum as `C3 01`, the writer schema's 64-bit Rabin fingerprint
//! in little-endian order, and the encoded body. The fingerprint is how a
//! receiver picks the writer schema out of a store - and the same fingerprint
//! is the natural cache key for a resolution plan.

use smol_str::format_smolstr;

use crate::{Limits, Result, Value};

use super::datum::{Cursor, DatumCodec, invalid};
use super::schema::Schema;

/// The two bytes that open every single-object encoding.
const SINGLE_MAGIC: [u8; 2] = [0xC3, 0x01];

/// Encode one value in the single-object framing.
///
/// # Errors
///
/// Returns an error when the value does not fit the schema, naming both.
pub fn to_single_object_vec(schema: &Schema, value: &Value) -> Result<Vec<u8>> {
    let datum = DatumCodec {
        names: &schema.names,
        limits: Limits::default(),
    };
    let mut output = Vec::with_capacity(32);
    output.extend_from_slice(&SINGLE_MAGIC);
    output.extend_from_slice(&schema.fingerprint().to_le_bytes());
    datum.encode(&schema.node, value, &mut output, 0)?;
    Ok(output)
}

/// Decode one single-object framed value written with `writer`.
///
/// # Errors
///
/// Returns an error when the framing is not single-object encoding, when the
/// fingerprint is not the writer schema's, or when the body does not decode.
pub fn from_single_object_slice(input: &[u8], writer: &Schema) -> Result<Value> {
    from_single_object_slice_with_limits(input, writer, Limits::default())
}

/// [`from_single_object_slice`] with explicit limits.
///
/// # Errors
///
/// Returns an error when the framing, fingerprint, body, or a limit fails.
pub fn from_single_object_slice_with_limits(
    input: &[u8],
    writer: &Schema,
    limits: Limits,
) -> Result<Value> {
    if input.len() > limits.max_input_bytes() {
        return Err(invalid(format_smolstr!(
            "expected a single-object datum of at most {} bytes, got {}",
            limits.max_input_bytes(),
            input.len()
        )));
    }
    let mut cursor = Cursor::new(input);
    let magic = cursor.take(SINGLE_MAGIC.len())?;
    if magic != SINGLE_MAGIC {
        return Err(invalid(format_smolstr!(
            "expected a single-object datum starting with {SINGLE_MAGIC:02X?}, got {magic:02X?}"
        )));
    }
    let declared = u64::from_le_bytes(cursor.take(8)?.try_into().unwrap_or_default());
    let expected = writer.fingerprint();
    if declared != expected {
        return Err(invalid(format_smolstr!(
            "expected the writer schema fingerprint {expected:016x}, got {declared:016x}"
        )));
    }
    let datum = DatumCodec {
        names: &writer.names,
        limits,
    };
    let mut budget = datum.budget();
    datum.decode(&writer.node, &mut cursor, 0, &mut budget)
}
