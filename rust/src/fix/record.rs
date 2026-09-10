//! One generic record read as a bounded stream of messages.
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
//! instead and a schema-only build keeps [`FixCodec::parse_text_record`] - the
//! same split the Avro codec already draws between its scalar and record
//! surfaces. Both surfaces end in [`parse_bytes`], which is where a row's own
//! columns are applied, so a row read one way reads exactly as it reads the
//! other - and the row's own dialect and version travel resolved into the one
//! build every reader funnels into, so no row ever costs a copy of the codec.

use crate::{Error, Result, Scalar, Version};

use crate::Field;

use super::build::{Fill, RowExtras};
use super::codec::FixCodec;
use super::msg::FixMsg;
use super::{FixBranch, FixMessages};

/// The column a payload is read from when nothing names another.
pub const DEFAULT_PAYLOAD_COLUMN: &str = "body";

/// The columns a record supplies as parameters rather than as payload.
///
/// Each names an argument the byte readers already take, so a record carrying
/// only a payload behaves exactly as the byte reader behaves - which is what
/// makes this an entry point rather than a second contract.
pub(super) const BEGINSTRING_COLUMN: &str = "beginstring";
pub(super) const SEPARATOR_COLUMN: &str = "sep";
/// The column a row states its own clock in, which stamps the message.
pub(super) const CLOCK_COLUMN: &str = super::TIMESTAMP_NAME;
/// The column a row states its direction in, read by the batch reader.
pub(super) const DIRECTION_COLUMN: &str = "direction";
/// The column naming the plugin that logged the row's line.
///
/// Read twice over, from one cell: as the fill of the crate's own
/// [`pluginid`](super::PLUGINID_TAG) column, by name like any other column,
/// and as the dialect the row is read under, through
/// [`FixCodec::dialect_of`]. It is deliberately no parameter: a parameter is
/// consumed by the read, and this column is carried into the row it names.
pub(super) const PLUGINID_COLUMN: &str = "pluginid";

/// Whether a column is one the readers take as a parameter or the payload,
/// rather than one that could fill a field by its name.
pub(super) fn is_parameter(name: &str, payload: &str) -> bool {
    [
        payload,
        BEGINSTRING_COLUMN,
        SEPARATOR_COLUMN,
        CLOCK_COLUMN,
        DIRECTION_COLUMN,
    ]
    .iter()
    .any(|held| crate::types::folds_equal(held, name))
}

/// The version a `beginstring` column states, as `BeginString` spells it.
///
/// `FIX.4.2` and `4.2` are one version; `FIXT.1.1` is none, because the
/// session layer says nothing about the application version, and neither
/// is anything else that is not a version.
pub(super) fn version_of(beginstring: &str) -> Option<Version> {
    beginstring
        .strip_prefix("FIX.")
        .unwrap_or(beginstring)
        .parse::<Version>()
        .ok()
}

/// What one row states for itself beside its payload.
///
/// The dialect and the version travel resolved - the branch the row's
/// `pluginid` reached, the version its `beginstring` parsed to - because
/// resolving them is the reader's boundary work and the build only reads
/// under them. The separator is the text it holds; the clock and the fills
/// are the values their columns carry. Absent is silence - a column that
/// was not there, was null, held no text, or named nothing the dictionary
/// declares - and silence is never an instruction and never an error.
#[derive(Clone, Copy, Default)]
pub(super) struct RowParameters<'row> {
    /// The dialect the row is read under, where its `pluginid` named one.
    pub(super) branch: Option<&'row FixBranch>,
    /// The version the row is read at, where its `beginstring` stated one.
    pub(super) version: Option<Version>,
    /// The separator, whose first byte splits the payload as a numeric frame.
    pub(super) separator: Option<&'row str>,
    /// The row's own clock, which stamps the message.
    pub(super) clock: Option<&'row Scalar>,
    /// The row's other columns, resolved to the fields they fill.
    pub(super) fills: &'row [Fill<'row>],
}

impl<'row> RowParameters<'row> {
    /// What the builder applies beside the pairs.
    fn extras(&self) -> RowExtras<'row> {
        RowExtras {
            branch: self.branch,
            version: self.version,
            clock: self.clock,
            fills: self.fills,
        }
    }
}

/// A row nobody could read, which is still a row - dated and versioned as
/// every row is, by what the row itself stated.
pub(super) fn empty(reader: &FixCodec, extras: RowExtras<'_>) -> FixMsg {
    reader
        .build_pairs_with(&[], extras)
        .expect("an empty message builds")
}

/// One record read against one codec, the payload taken from `payload`.
///
/// [`FixCodec::parse_text_record`] is the door; this reads the row's own
/// columns out of the record and hands them on, beside the cell-driven path
/// the batch reader takes to the same place. Nothing is copied: the payload
/// is read where the record holds it, and a parameter is the text it is.
pub(super) fn parse_record_with(
    reader: &FixCodec,
    record: &Scalar,
    payload: &str,
) -> Result<FixMessages> {
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
    // The payload column is the one column a record must have: a record
    // without it, or holding something that is neither text nor bytes under
    // it, would parse nothing, and is refused naming the column rather than
    // answered as an empty message. A null under it is a row of nothing,
    // which is still a row.
    let stated = held
        .iter()
        .find(|(known, _)| crate::types::folds_equal(known, payload))
        .map(|(_, value)| value);
    let bytes = match stated {
        None => {
            let names: Vec<&str> = held.keys().map(|name| name.as_str()).collect();
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new(payload),
                reason: crate::text::expected_got(
                    format_args!("a text or binary column named {payload}"),
                    format_args!("a record carrying only [{}]", names.join(", ")),
                ),
            });
        }
        Some(value) if value.is_null() => &[][..],
        Some(value) => match value
            .as_bytes()
            .or_else(|| value.as_str().map(str::as_bytes))
        {
            Some(bytes) => bytes,
            None => {
                let actual = value.dtype().map_or_else(
                    |_| "another value".to_string(),
                    |dtype| format!("a value of {dtype}"),
                );
                return Err(Error::InvalidRecord {
                    path: smol_str::SmolStr::new(payload),
                    reason: crate::text::expected_got(
                        format_args!("a text or binary column named {payload}"),
                        actual,
                    ),
                });
            }
        },
    };
    // Every other column the record carries is offered as a fill: one named
    // after a field the dictionary knows lands on it, the rest are silence.
    // Resolved as the batch reader resolves its columns - under the codec's
    // own pin - so one record and one batch row fill the same fields.
    let resolved: Vec<(Field, i32, &Scalar)> = held
        .iter()
        .filter(|(name, value)| !is_parameter(name, payload) && !value.is_null())
        .filter_map(|(name, value)| {
            let (field, tag) = reader.fill_target(name)?;
            Some((field, tag, value))
        })
        .collect();
    let fills: Vec<Fill<'_>> = resolved
        .iter()
        .map(|(field, tag, value)| Fill {
            field,
            tag: *tag,
            value,
        })
        .collect();
    // The plugin that logged the line is one of those fills, and it is also
    // the dialect the line is read under where it names one the dictionary
    // declares; a payload column spelled `pluginid` is the payload alone.
    let parameters = RowParameters {
        branch: column(PLUGINID_COLUMN)
            .filter(|_| !is_parameter(PLUGINID_COLUMN, payload))
            .and_then(Scalar::as_str)
            .and_then(|plugin| reader.dialect_of(plugin)),
        version: column(BEGINSTRING_COLUMN)
            .and_then(Scalar::as_str)
            .and_then(version_of),
        separator: column(SEPARATOR_COLUMN).and_then(Scalar::as_str),
        clock: column(CLOCK_COLUMN),
        fills: &fills,
    };
    parse_bytes(reader, parameters, bytes)
}

/// Reads one payload through the byte readers, the row's own parameters
/// applied.
///
/// A column is the caller speaking per row and an option is the caller
/// speaking per stream, so both outrank the inference the readers fall back
/// on - and the column outranks the option, because it is the more specific
/// statement. The codec is the run's and is never copied: what the row
/// stated travels beside the payload into the one build every reader
/// funnels into, which reads under the row's dialect and version where the
/// row stated them and under the codec's where it did not.
pub(super) fn parse_bytes(
    reader: &FixCodec,
    parameters: RowParameters<'_>,
    bytes: &[u8],
) -> Result<FixMessages> {
    let extras = parameters.extras();
    if bytes.is_empty() {
        return Ok(FixMessages::one(empty(reader, extras)));
    }
    // A stated separator is read as a numeric frame with that separator; with
    // none stated the reader picks its own dialect from the frame, which is
    // what a record carrying only a payload has to do.
    let built = match parameters.separator.and_then(|held| held.bytes().next()) {
        Some(separator) => reader
            .split_fix_with(bytes, separator, extras)
            .map(FixMessages::one),
        None => reader.parse_line_with(bytes, extras),
    };
    Ok(built.unwrap_or_else(|_| FixMessages::one(empty(reader, extras))))
}
