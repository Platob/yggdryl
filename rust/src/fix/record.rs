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
//! Both surfaces end in [`transform_bytes`], which is where a row's own
//! columns are applied, so a row read one way reads exactly as it reads the
//! other.

use crate::{Error, Result, Scalar, Version};

use super::FixBranch;
use super::build::RowExtras;
use super::codec::FixCodec;
use super::msg::FixMsg;

/// The column a payload is read from when nothing names another.
pub const DEFAULT_PAYLOAD_COLUMN: &str = "body";

/// The columns a record supplies as parameters rather than as payload.
///
/// Each names an argument the byte readers already take, so a record carrying
/// only a payload behaves exactly as the byte reader behaves - which is what
/// makes this an entry point rather than a second contract.
pub(super) const BRANCH_COLUMN: &str = "branch";
pub(super) const BEGINSTRING_COLUMN: &str = "beginstring";
pub(super) const SEPARATOR_COLUMN: &str = "sep";
/// The column a row states its own clock in, which stamps the message.
pub(super) const CLOCK_COLUMN: &str = super::TIMESTAMP_NAME;
/// The column a row states its direction in, read by the batch reader.
pub(super) const DIRECTION_COLUMN: &str = "direction";

/// Whether a column is one the readers take as a parameter or the payload,
/// rather than one that could fill a field by its name.
pub(super) fn is_parameter(name: &str, payload: &str) -> bool {
    [
        payload,
        BRANCH_COLUMN,
        BEGINSTRING_COLUMN,
        SEPARATOR_COLUMN,
        CLOCK_COLUMN,
        DIRECTION_COLUMN,
    ]
    .iter()
    .any(|held| crate::types::folds_equal(held, name))
}

/// What one row states for itself beside its payload.
///
/// The three parameters are the text they hold; the clock and the fills are
/// the values their columns carry. Absent is silence - a column that was not
/// there, was null, or held no text - and silence is never an instruction
/// and never an error.
#[derive(Clone, Copy, Default)]
pub(super) struct RowParameters<'row> {
    /// The dialect the row is read under.
    pub(super) branch: Option<&'row str>,
    /// The version the row is read at, as `BeginString` spells it.
    pub(super) beginstring: Option<&'row str>,
    /// The separator, whose first byte splits the payload as a numeric frame.
    pub(super) separator: Option<&'row str>,
    /// The row's own clock, which stamps the message.
    pub(super) clock: Option<&'row Scalar>,
    /// The row's other columns, filling the fields their names reach.
    pub(super) fills: &'row [(&'row str, &'row Scalar)],
}

impl<'row> RowParameters<'row> {
    /// What the builder applies beside the pairs.
    fn extras(&self) -> RowExtras<'row> {
        RowExtras {
            clock: self.clock,
            fills: self.fills,
        }
    }
}

/// A row nobody could read, which is still a row - dated and versioned as
/// every row is, by what the row itself stated.
pub(super) fn empty(reader: &FixCodec, extras: RowExtras<'_>) -> FixMsg {
    reader
        .build_pairs_with(&[], extras, false)
        .expect("an empty message builds")
}

/// One record read against one codec, the payload taken from `payload`.
///
/// [`FixCodec::transform_record`] is the door; this reads the row's own
/// columns out of the record and hands them on, beside the option-driven path
/// the batch reader takes to the same place. Nothing is copied: the payload
/// is read where the record holds it, and a parameter is the text it is.
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
    // One column by name, absent when it states nothing.
    let column = |name: &str| {
        held.iter()
            .find(|(known, _)| crate::types::folds_equal(known, name))
            .map(|(_, value)| value)
            .filter(|value| !value.is_null())
    };
    let bytes = column(payload)
        .and_then(|held| held.as_bytes().or_else(|| held.as_str().map(str::as_bytes)))
        .unwrap_or_default();
    // Every other column the record carries is offered as a fill: one named
    // after a field the dictionary knows lands on it, the rest are silence.
    let fills: Vec<(&str, &Scalar)> = held
        .iter()
        .filter(|(name, value)| !is_parameter(name, payload) && !value.is_null())
        .map(|(name, value)| (name.as_str(), value))
        .collect();
    let parameters = RowParameters {
        branch: column(BRANCH_COLUMN).and_then(Scalar::as_str),
        beginstring: column(BEGINSTRING_COLUMN).and_then(Scalar::as_str),
        separator: column(SEPARATOR_COLUMN).and_then(Scalar::as_str),
        clock: column(CLOCK_COLUMN),
        fills: &fills,
    };
    transform_bytes(reader, parameters, bytes, enrich)
}

/// Reads one payload through the byte readers, the row's own parameters
/// applied.
///
/// A column is the caller speaking per row and an option is the caller
/// speaking per stream, so both outrank the inference the readers fall back
/// on - and the column outranks the option, because it is the more specific
/// statement. The codec is the run's, and a row stating nothing reads under
/// it as it stands; a copy carrying the row's own pins is made only for a
/// row that states one.
pub(super) fn transform_bytes(
    reader: &FixCodec,
    parameters: RowParameters<'_>,
    bytes: &[u8],
    enrich: bool,
) -> Result<FixMsg> {
    let branch = parameters
        .branch
        .and_then(|name| FixBranch::from_str(name).ok());
    let version = parameters.beginstring.and_then(|held| {
        held.strip_prefix("FIX.")
            .unwrap_or(held)
            .parse::<Version>()
            .ok()
    });
    let pinned;
    let reader = if branch.is_some() || version.is_some() {
        let mut copy = reader.clone();
        if let Some(branch) = &branch {
            copy = copy.with_branch(branch);
        }
        if let Some(version) = version {
            copy = copy.with_version(version);
        }
        pinned = copy;
        &pinned
    } else {
        reader
    };
    let extras = parameters.extras();
    if bytes.is_empty() {
        return Ok(empty(reader, extras));
    }
    // A stated separator is read as a numeric frame with that separator; with
    // none stated the reader picks its own dialect from the frame, which is
    // what a record carrying only a payload has to do.
    let built = match parameters.separator.and_then(|held| held.bytes().next()) {
        Some(separator) => reader.split_fix_with(bytes, separator, extras, enrich),
        None => reader.transform_line_with(bytes, extras, enrich),
    };
    Ok(built.unwrap_or_else(|_| empty(reader, extras)))
}
