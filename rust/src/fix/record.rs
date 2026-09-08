//! One generic record read as one message.
//!
//! A record is a name-to-value map - one `Scalar` variant - and every
//! row-oriented reader in the crate answers with one, so this is the shape a
//! message is read from when it did not arrive as bare bytes. The payload
//! column carries those bytes; every other named column states, for that one
//! row, a fact the codec otherwise holds for the whole run.
//!
//! This is not the Arrow surface. [`super::batch`] reads a column of payloads
//! and writes a column of rows, and it is gated on `arrow` because it speaks
//! Arrow's types. Reading one record speaks none of them, so it lives here
//! instead and a schema-only build keeps [`FixCodec::transform_record`] - the same
//! split the Avro codec already draws between its scalar and record surfaces.

use smol_str::SmolStr;

use crate::{Error, Result, Scalar, Version};

use super::FixBranch;
use super::codec::FixCodec;
use super::msg::FixMsg;

/// The column a payload is read from when nothing names another.
pub const DEFAULT_PAYLOAD_COLUMN: &str = "body";

/// The columns a record supplies as parameters rather than as payload.
///
/// Each names an argument the byte readers already take, so a record carrying
/// only a payload behaves exactly as the byte reader behaves - which is what
/// makes this an entry point rather than a second contract.
const BRANCH_COLUMN: &str = "branch";
const BEGINSTRING_COLUMN: &str = "beginstring";
const SEPARATOR_COLUMN: &str = "sep";

/// One column by name, absent when it states nothing.
pub(super) fn column<'row>(record: &'row [(SmolStr, Scalar)], name: &str) -> Option<&'row Scalar> {
    record
        .iter()
        .find(|(held, _)| crate::types::folds_equal(held, name))
        .map(|(_, value)| value)
        .filter(|held| !held.is_null())
}

/// One column's bytes, however the column is typed.
pub(super) fn column_bytes(record: &[(SmolStr, Scalar)], name: &str) -> Option<Vec<u8>> {
    let held = column(record, name)?;
    held.as_bytes()
        .map(<[u8]>::to_vec)
        .or_else(|| held.as_str().map(|text| text.as_bytes().to_vec()))
}

/// One column's text.
pub(super) fn column_text(record: &[(SmolStr, Scalar)], name: &str) -> Option<String> {
    let held = column(record, name)?;
    held.as_str().map(ToOwned::to_owned)
}

/// A row nobody could read, which is still a row.
pub(super) fn empty(reader: &FixCodec) -> FixMsg {
    reader
        .transform_pairs(std::iter::empty::<(&[u8], &[u8])>(), false)
        .expect("an empty message builds")
}

/// One record read against one codec, the payload taken from `payload`.
///
/// [`FixCodec::transform_record`] is the door; this is where the row's own
/// columns are applied, beside the option-driven path the batch reader takes.
pub(super) fn transform_record_with(
    reader: &FixCodec,
    record: &Scalar,
    payload: &str,
    enrich: bool,
) -> Result<FixMsg> {
    let Some(held) = record.as_record() else {
        return Err(Error::Parse {
            target: "fix record",
            position: 0,
            reason: crate::text::expected_got("a record", "another value"),
        });
    };
    let row: Vec<(SmolStr, Scalar)> = held
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let bytes = column_bytes(&row, payload).unwrap_or_default();
    transform_record(reader, &row, &bytes, enrich)
}

/// Reads one record through the byte readers, per-row columns applied.
pub(super) fn transform_record(
    reader: &FixCodec,
    record: &[(SmolStr, Scalar)],
    bytes: &[u8],
    enrich: bool,
) -> Result<FixMsg> {
    // A column is the caller speaking per row and an option is the caller
    // speaking per stream, so both outrank the inference the readers fall back
    // on - and the column outranks the option, because it is the more specific
    // statement. A column absent, null or empty is silence, never an
    // instruction, and never an error.
    let mut reader = reader.clone();
    if let Some(name) = column_text(record, BRANCH_COLUMN) {
        if let Ok(branch) = FixBranch::from_str(&name) {
            reader = reader.with_branch(&branch);
        }
    }
    if let Some(held) = column_text(record, BEGINSTRING_COLUMN) {
        let spelling = held.strip_prefix("FIX.").unwrap_or(&held);
        if let Ok(version) = spelling.parse::<Version>() {
            reader = reader.with_version(version);
        }
    }
    if bytes.is_empty() {
        return Ok(empty(&reader));
    }
    // A stated separator is read as a numeric frame with that separator; with
    // none stated the reader picks its own dialect from the frame, which is
    // what a record carrying only a payload has to do.
    let stated = column_text(record, SEPARATOR_COLUMN).and_then(|held| held.bytes().next());
    let built = match stated {
        Some(separator) => reader
            .clone()
            .with_separator(separator)
            .transform_fix_line(bytes, enrich),
        None => reader.transform_line(bytes, enrich),
    };
    Ok(built.unwrap_or_else(|_| empty(&reader)))
}
