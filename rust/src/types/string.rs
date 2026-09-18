//! The string datatype family: one layout, one charset, one length bound.
//!
//! Arrow has three string layouts and no way to say what a string is *in*: a
//! `Utf8` array declares UTF-8 and nothing else declares anything, and there
//! is no `varchar(n)` or `char(n)` anywhere in the format. This family is the
//! one place this crate answers all three questions - which layout, which
//! charset, how long - and every string the crate has is one member of it.
//!
//! [`StringLayout`] names the five layouts. [`StringType`] is a layout
//! beside the charset its bytes are written in and the bound its values are
//! held to. [`crate::DataType::string`] builds the one string datatype,
//! [`crate::DataType::String`], from them; `utf8`, `ascii`, `varchar(32)`
//! and `char(8)` are spellings of it, never datatypes of their own.
//! [`Str`] is the one string value, and the ten registered codes beside it
//! are identities over a published registry rather than strings with a
//! charset: each stores as the ASCII text it is, under its own Arrow
//! extension name and held to its own standard's width.
//!
//! ```
//! use yggdryl::types::{StringLayout, StringType};
//! use yggdryl::{Charset, DataType};
//!
//! # fn main() -> yggdryl::Result<()> {
//! // Every spelling is one datatype, and it reads back as itself.
//! assert_eq!(DataType::from_str("string")?, DataType::utf8());
//! assert_eq!(DataType::from_str("largestringview")?.to_string(), "large_utf8_view");
//! assert_eq!(DataType::from_str("fixed_string(us-ascii,4)")?.to_string(), "fixed_ascii(4)");
//!
//! // The layout, the charset and the bound are what a string declares.
//! let latin = DataType::from_str("string(windows-1252,32)")?;
//! let parameters = latin.string_parameters().expect("a string datatype");
//! assert_eq!(parameters.layout(), StringLayout::String);
//! assert_eq!(parameters.charset(), Charset::Cp1252);
//! assert_eq!(parameters.max(), Some(32));
//! assert_eq!(latin.to_string(), "string(windows-1252,32)");
//! # Ok(())
//! # }
//! ```

use std::collections::BTreeMap;

use std::fmt;

use std::num::NonZeroU32;

pub(crate) use arrow::{
    arrow_storage, describes_storage, from_arrow_storage, is_text_storage, needs_extension,
};

pub(crate) use scalars::str_from_value;

pub use scalars::{INLINE_CAPACITY, Str};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use smol_str::{SmolStr, format_smolstr};

use crate::metadata::{FIELD_ENUM_KEY, parse_string_enum};

use crate::types::parser::Parser;
use crate::types::{BLOOMBERG_WIDTH, CFI_WIDTH, COUNTRY_WIDTH, CURRENCY_WIDTH, CUSIP_WIDTH, ISIN_WIDTH, MIC_WIDTH, SEDOL_WIDTH, SIDE_WIDTH, STATE_WIDTH, TIMEINFORCE_WIDTH};

use crate::types::parser;


use crate::{Charset, DataType, DataTypeId, Error, Field, Result, Scalar};

/// What a string datatype lays out in Arrow, and what it reads back from.
///
/// Arrow declares a layout and, for its three string layouts, UTF-8. It
/// declares no charset, no length bound, and it has one view layout where
/// this crate has two. So the projection is: text this crate stores as UTF-8
/// or US-ASCII rides Arrow's own string layouts - ASCII bytes are UTF-8, and
/// Arrow is told the truth about the bytes - text in any other charset rides
/// the matching *binary* layout, because the bytes are not UTF-8 and saying
/// they are would be a lie a reader acts on, and everything Arrow cannot say
/// rides the `yggdryl.string` extension document beside it.
mod arrow {
    use arrow_schema::DataType as ArrowDataType;

    use super::{StringLayout, StringType};
    use crate::{Charset, Error, Result};

    /// Whether a string's bytes ride Arrow's text layouts rather than its binary
    /// ones.
    ///
    /// The one owner of that fact: the projection below, the column writer, the
    /// cell reader, the digest feed and the cast planner all ask here. UTF-8 is
    /// Arrow's own text, and US-ASCII is a subset of it.
    pub(crate) const fn is_text_storage(parameters: StringType) -> bool {
        matches!(parameters.charset(), Charset::Utf8 | Charset::Ascii)
    }

    /// Whether a string field needs the `yggdryl.string` document beside its
    /// storage.
    ///
    /// Only where Arrow cannot say what the string declares: a charset other
    /// than UTF-8, a bound, or the large view layout, which Arrow projects onto
    /// its one view. Plain `utf8`, `large_utf8` and `utf8_view` are Arrow's own
    /// datatypes and cross bare.
    pub(crate) const fn needs_extension(parameters: StringType) -> bool {
        !parameters.charset().is_utf8()
            || parameters.is_bounded()
            || matches!(parameters.layout(), StringLayout::LargeStringView)
    }

    /// The Arrow storage one string datatype lays out.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] when a fixed width is outside `i32`,
    /// which is as wide as Arrow's own fixed binary counts.
    pub(crate) fn arrow_storage(parameters: StringType) -> Result<ArrowDataType> {
        // The variant is public, so a fixed string can arrive here without the
        // width that makes it fixed. A boundary is where that stops.
        parameters.validate()?;
        // A fixed width is one layout in Arrow whatever the charset: Arrow has no
        // fixed-width string, so the bytes ride its fixed binary and the charset
        // travels beside them.
        if let Some(width) = parameters.fixed() {
            let width = i32::try_from(width).map_err(|_| Error::InvalidDataType {
                kind: "string",
                reason: smol_str::format_smolstr!(
                    "fixed width {width} is outside the i32 range Arrow counts in"
                ),
            })?;
            return Ok(ArrowDataType::FixedSizeBinary(width));
        }
        let text = is_text_storage(parameters);
        Ok(match (parameters.layout(), text) {
            (StringLayout::String, true) => ArrowDataType::Utf8,
            (StringLayout::String, false) => ArrowDataType::Binary,
            (StringLayout::LargeString, true) => ArrowDataType::LargeUtf8,
            (StringLayout::LargeString, false) => ArrowDataType::LargeBinary,
            // Arrow has one view layout, so both of this crate's project onto it
            // and the `large` half of the distinction rides the metadata.
            (StringLayout::StringView | StringLayout::LargeStringView, true) => ArrowDataType::Utf8View,
            (StringLayout::StringView | StringLayout::LargeStringView, false) => {
                ArrowDataType::BinaryView
            }
            // The fixed layout answered above: `validate` gave it a width and
            // the width gave it its storage.
            (StringLayout::FixedString, _) => ArrowDataType::Binary,
        })
    }

    /// Whether one Arrow storage is what these parameters lay out.
    ///
    /// The import side asks this rather than re-deriving: a `yggdryl.string`
    /// document over a storage it does not describe is a foreign field wearing
    /// our name, and it imports as its storage instead.
    pub(crate) fn describes_storage(
        parameters: StringType,
        storage: &ArrowDataType,
    ) -> Result<bool> {
        Ok(arrow_storage(parameters)? == *storage)
    }

    /// The string datatype one Arrow text storage imports as.
    ///
    /// A charset and a bound are a `yggdryl.string` document on the field, so a
    /// bare storage imports as the plain UTF-8 layout it is; the field level
    /// puts the document's parameters back when it is there.
    ///
    /// # Errors
    ///
    /// Returns an error when the storage belongs to another family.
    pub(crate) fn from_arrow_storage(value: &ArrowDataType) -> Result<crate::DataType> {
        match value {
            ArrowDataType::Utf8 => Ok(crate::DataType::utf8()),
            ArrowDataType::LargeUtf8 => Ok(crate::DataType::large_utf8()),
            ArrowDataType::Utf8View => Ok(crate::DataType::utf8_view()),
            other => Err(crate::types::invalid(
                "string",
                smol_str::format_smolstr!("expected a text storage, got {other}"),
            )),
        }
    }
}

/// Arrow casts owned by this datatype family.
pub(crate) mod casts {
    use std::borrow::Cow;
    use std::sync::Arc;

    use arrow_array::builder::{
        BinaryBuilder, BinaryViewBuilder, LargeBinaryBuilder, LargeStringBuilder, StringBuilder,
        StringViewBuilder,
    };
    use arrow_array::{Array, ArrayRef, BinaryArray, FixedSizeBinaryArray, StringArray};
    use arrow_buffer::{BooleanBuffer, BooleanBufferBuilder};
    use arrow_schema::DataType as ArrowDataType;

    use crate::arrow::{Error, Result};
    use crate::types::budget::{MaterializationBudget, reserve_vec_bytes};
    use crate::types::bytes::casts::variable_binary_source;
    use crate::types::cast::arrow_cast_exposed;
    use crate::types::cast::{downcast, internal_target_error, named_cell};
    use crate::types::cast::columns::is_exposed;
    use crate::types::string::arrow_storage;
    use crate::types::{
        Str, StringType, code_cell_text, code_text, trim_padding, uuid_parse, uuid_text,
    };
    use crate::{Charset, DataType, Field};

    /// What the planner learned about the column a string reads from.
    ///
    /// A recognized source declares how its bytes are read; a bare one is read
    /// as text where Arrow calls it text, and as bytes already in the target's
    /// charset everywhere else.
    #[derive(Clone, Debug)]
    pub(crate) enum StringSource {
        /// A `yggdryl.string` column: every cell is read under its own
        /// parameters before it is restated under the target's.
        String(StringType),
        /// A registered code: validated ASCII, at most the code's own width.
        Code(DataType),
        /// A UUID: sixteen stored bytes, read as the canonical spelling they are.
        Uuid,
        /// Storage with no extension identity.
        Bare,
    }

    /// One cell of the temporary every string source is read through.
    #[derive(Clone, Copy)]
    enum Cell<'a> {
        Text(&'a str),
        Bytes(&'a [u8]),
        /// One cell of a fixed-width slot, whose trailing NUL is the slot's
        /// padding rather than the value's bytes.
        ///
        /// Which sources it is padding *for* is the reading's own question: a
        /// declared width and a code both trim it where they read, and a UUID
        /// fills its sixteen bytes, so only bare storage trims here.
        Slot(&'a [u8]),
    }

    /// Validates every exposed, non-null value entering a string datatype and
    /// stores it in the target's own storage.
    ///
    /// Every source is read through one of three temporaries - text, variable
    /// bytes, or the fixed binary it already is - so the per-cell reading is
    /// stated once per source kind rather than once per Arrow layout. A failing
    /// cell is null under `safe` and an error naming the row otherwise.
    pub(crate) fn ingest_string_array(
        array: &ArrayRef,
        source: &StringSource,
        safe: bool,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        let Some(target) = field.dtype().string_parameters() else {
            return Err(internal_target_error("string"));
        };
        let read = |cell: Cell<'_>| -> crate::Result<Str> {
            match (source, cell) {
                (StringSource::String(parameters), Cell::Text(text)) => {
                    Str::from_storage(text, *parameters).try_with_parameters(target)
                }
                // A declared width trims its own padding where it reads.
                (StringSource::String(parameters), Cell::Bytes(bytes) | Cell::Slot(bytes)) => {
                    Str::from_bytes(bytes, *parameters)?.try_with_parameters(target)
                }
                // So does a code, at the width its standard fixes.
                (StringSource::Code(code), Cell::Bytes(bytes) | Cell::Slot(bytes)) => {
                    Str::from_storage(code_cell_text(code, bytes)?, StringType::default())
                        .try_with_parameters(target)
                }
                // A UUID is sixteen bytes of identity, every one of which can be
                // NUL, and its value is the canonical spelling they name.
                (StringSource::Uuid, Cell::Bytes(bytes) | Cell::Slot(bytes)) => {
                    Str::from(uuid_text(&uuid_parse(bytes)?)).try_with_parameters(target)
                }
                (StringSource::Uuid, Cell::Text(text)) => {
                    Str::from(uuid_text(&uuid_parse(text.as_bytes())?)).try_with_parameters(target)
                }
                (StringSource::Code(_) | StringSource::Bare, Cell::Text(text)) => {
                    Str::from_storage(text, StringType::default()).try_with_parameters(target)
                }
                (StringSource::Bare, Cell::Slot(bytes)) => Str::from_bytes(trim_padding(bytes), target),
                (StringSource::Bare, Cell::Bytes(bytes)) => Str::from_bytes(bytes, target),
            }
        };
        if let ArrowDataType::FixedSizeBinary(_) = array.data_type() {
            let cells = downcast::<FixedSizeBinaryArray>(array.as_ref())?;
            return string_storage(
                target,
                field,
                cells.len(),
                safe,
                exposure,
                budget,
                |index| {
                    cells
                        .is_valid(index)
                        .then(|| read(Cell::Slot(cells.value(index))))
                },
            );
        }
        if let Some(bytes) = variable_binary_source(array, field, exposure, budget)? {
            let cells = downcast::<BinaryArray>(bytes.as_ref())?;
            return string_storage(
                target,
                field,
                cells.len(),
                safe,
                exposure,
                budget,
                |index| {
                    cells
                        .is_valid(index)
                        .then(|| read(Cell::Bytes(cells.value(index))))
                },
            );
        }
        let text = if array.data_type() == &ArrowDataType::Utf8 {
            Arc::clone(array)
        } else {
            // The temporary is nullable text: the kernel's masked path fills
            // nothing, and the target's own null policy runs after the reading.
            arrow_cast_exposed(
                array,
                &ArrowDataType::Utf8,
                safe,
                exposure,
                &Field::new(field.name(), DataType::utf8(), true),
                budget,
            )?
        };
        let cells = downcast::<StringArray>(text.as_ref())?;
        string_storage(
            target,
            field,
            cells.len(),
            safe,
            exposure,
            budget,
            |index| {
                cells
                    .is_valid(index)
                    .then(|| read(Cell::Text(cells.value(index))))
            },
        )
    }

    /// Builds the storage of one string datatype from one read cell per row.
    ///
    /// Unexposed rows are null: an ancestor hides them, so their bytes are
    /// neither validated nor copied. Every value is held before a byte of the
    /// output exists, so the payload is measured once and the buffer sized to
    /// it. The repertoire was settled when the value was restated - US-ASCII is
    /// judged there, UTF-8 is what a value holds - so text storage takes the
    /// characters as they are; every other charset writes its bytes here, and a
    /// scalar it cannot spell fails at the write, as in every other writer.
    fn string_storage(
        target: StringType,
        field: &Field,
        rows: usize,
        safe: bool,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
        cell: impl Fn(usize) -> Option<crate::Result<Str>>,
    ) -> Result<ArrayRef> {
        budget.add_array(field.dtype(), rows)?;
        reserve_vec_bytes::<Option<Str>>(budget, rows)?;
        let mut values = Vec::new();
        values.try_reserve_exact(rows).map_err(|error| {
            Error::IncompatibleSchema(format!("string output allocation failed: {error}"))
        })?;
        let mut payload = 0_usize;
        for index in 0..rows {
            let value = match is_exposed(exposure, index).then(|| cell(index)).flatten() {
                Some(Ok(value)) => Some(value),
                Some(Err(_)) if safe => None,
                Some(read @ Err(_)) => Some(named_cell(field, index, read)?),
                None => None,
            };
            payload = payload.saturating_add(value.as_ref().map_or(0, Str::encoded_len));
            values.push(value);
        }
        budget.add_bytes(payload)?;
        let charset = target.charset();
        macro_rules! filled {
            ($builder:expr, $stored:expr) => {{
                let mut builder = $builder;
                for (index, value) in values.iter().enumerate() {
                    match value.as_ref().map($stored) {
                        Some(Ok(stored)) => builder.append_value(stored),
                        Some(Err(_)) if safe => builder.append_null(),
                        Some(Err(error)) => return named_cell(field, index, Err(error)),
                        None => builder.append_null(),
                    }
                }
                Arc::new(builder.finish()) as ArrayRef
            }};
        }
        Ok(match arrow_storage(target)? {
            ArrowDataType::FixedSizeBinary(width) => {
                let slot = usize::try_from(width).map_err(|_| internal_target_error("string"))?;
                // The reservation bounds `rows * slot`, so the product cannot overflow.
                let mut padded = vec![0_u8; rows * slot];
                let mut validity = BooleanBufferBuilder::new(rows);
                for (index, value) in values.iter().enumerate() {
                    let present = match value.as_ref().map(|value| encoded(charset, value)) {
                        Some(Ok(encoded)) => {
                            padded[index * slot..][..encoded.len()].copy_from_slice(&encoded);
                            true
                        }
                        Some(Err(_)) if safe => false,
                        Some(Err(error)) => return named_cell(field, index, Err(error)),
                        None => false,
                    };
                    validity.append(present);
                }
                let nulls = arrow_buffer::NullBuffer::new(validity.finish());
                Arc::new(FixedSizeBinaryArray::try_new(
                    width,
                    arrow_buffer::Buffer::from(padded),
                    (nulls.null_count() != 0).then_some(nulls),
                )?)
            }
            ArrowDataType::Utf8 => filled!(StringBuilder::with_capacity(rows, payload), characters),
            ArrowDataType::LargeUtf8 => {
                filled!(LargeStringBuilder::with_capacity(rows, payload), characters)
            }
            // Arrow's view layout carries a prefix per cell rather than offsets,
            // so it takes the row count and grows its own payload blocks.
            ArrowDataType::Utf8View => filled!(StringViewBuilder::with_capacity(rows), characters),
            ArrowDataType::Binary => filled!(BinaryBuilder::with_capacity(rows, payload), |value| {
                encoded(charset, value)
            }),
            ArrowDataType::LargeBinary => {
                filled!(LargeBinaryBuilder::with_capacity(rows, payload), |value| {
                    encoded(charset, value)
                })
            }
            ArrowDataType::BinaryView => filled!(BinaryViewBuilder::with_capacity(rows), |value| {
                encoded(charset, value)
            }),
            _ => return Err(internal_target_error("string")),
        })
    }

    /// What text storage stores: the characters themselves.
    fn characters(value: &Str) -> crate::Result<&str> {
        Ok(value.as_str())
    }

    /// What binary storage stores: the characters written in the charset.
    fn encoded(charset: Charset, value: &Str) -> crate::Result<Cow<'_, [u8]>> {
        charset.encode(value.as_str())
    }

    /// Validates every exposed, non-null value entering a registered code and
    /// stores it as the text it is.
    ///
    /// A Utf8 column of valid values is the target's own storage, so it is shared
    /// rather than rebuilt; a fixed binary is trimmed of the padding its slot
    /// wrote, and anything else first renders as Utf8 through Arrow's kernel.
    /// What the constant width buys is the inner loop: the length check is
    /// fixed-size, so a currency column ingests three bytes a row with no width
    /// to read.
    pub(crate) fn ingest_code_array<const WIDTH: usize>(
        array: &ArrayRef,
        safe: bool,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        if let ArrowDataType::FixedSizeBinary(_) = array.data_type() {
            let source = downcast::<FixedSizeBinaryArray>(array.as_ref())?;
            return code_text_array::<WIDTH>(field, source.len(), safe, exposure, budget, |index| {
                source.is_valid(index).then(|| source.value(index))
            });
        }
        if let Some(bytes) = variable_binary_source(array, field, exposure, budget)? {
            let source = downcast::<BinaryArray>(bytes.as_ref())?;
            return code_text_array::<WIDTH>(field, source.len(), safe, exposure, budget, |index| {
                source.is_valid(index).then(|| source.value(index))
            });
        }
        let text = if array.data_type() == &ArrowDataType::Utf8 {
            Arc::clone(array)
        } else {
            arrow_cast_exposed(
                array,
                &ArrowDataType::Utf8,
                safe,
                exposure,
                &Field::new(field.name(), DataType::utf8(), true),
                budget,
            )?
        };
        let source = downcast::<StringArray>(text.as_ref())?;
        // The column is the target's own storage once every cell passes; a cell
        // that fails is an error when strict, and under `safe` a null the
        // rebuild below writes. Nothing is copied when the column already holds
        // what the code promises, which is what a column written as this code
        // always does.
        let mut every_cell_passes = true;
        for index in 0..source.len() {
            if is_exposed(exposure, index)
                && source.is_valid(index)
                && code_cell::<WIDTH>(field, index, source.value(index).as_bytes(), safe)?.is_none()
            {
                every_cell_passes = false;
                break;
            }
        }
        if every_cell_passes {
            return Ok(text);
        }
        code_text_array::<WIDTH>(field, source.len(), safe, exposure, budget, |index| {
            source
                .is_valid(index)
                .then(|| source.value(index).as_bytes())
        })
    }

    /// Builds the Utf8 storage of one registered code from one cell per row.
    ///
    /// Unexposed rows are null, and so is a refused cell under `safe`, exactly
    /// as a refused string cell is: the two families answer one cast the same
    /// way. A cell never outgrows `WIDTH`, so the payload is bounded before a
    /// byte of it is copied.
    fn code_text_array<'a, const WIDTH: usize>(
        field: &Field,
        rows: usize,
        safe: bool,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
        cell: impl Fn(usize) -> Option<&'a [u8]>,
    ) -> Result<ArrayRef> {
        budget.add_array(field.dtype(), rows)?;
        let mut builder = StringBuilder::with_capacity(rows, rows.saturating_mul(WIDTH));
        for index in 0..rows {
            let value = cell(index)
                .filter(|_| is_exposed(exposure, index))
                .map(|raw| code_cell::<WIDTH>(field, index, raw, safe))
                .transpose()?
                .flatten();
            match value {
                Some(text) => builder.append_value(text),
                None => builder.append_null(),
            }
        }
        Ok(Arc::new(builder.finish()))
    }

    /// Validates one code cell at the code's constant width.
    ///
    /// `None` is a refused cell under `safe`, which the caller stores as null;
    /// strict, the refusal names the row and the column.
    fn code_cell<'a, const WIDTH: usize>(
        field: &Field,
        index: usize,
        bytes: &'a [u8],
        safe: bool,
    ) -> Result<Option<&'a str>> {
        let refused = |reason: String| match safe {
            true => Ok(None),
            false => Err(Error::IncompatibleSchema(format!(
                "row {index} of column {name}: {reason}",
                name = field.name()
            ))),
        };
        let text = match code_text::<WIDTH>(bytes) {
            Ok(text) => text,
            Err(error) => return refused(error.to_string()),
        };
        // A securities identifier carries its own check, and a column of them
        // holds the canonical spelling: what a cast lets in is what a read
        // answers, so the check digit and the case are settled here rather
        // than on every read of the cell.
        let canonical = match field.dtype() {
            DataType::Isin => crate::types::Isin::is_canonical(text),
            DataType::Cusip => crate::types::Cusip::is_canonical(text),
            DataType::Sedol => crate::types::Sedol::is_canonical(text),
            DataType::Bloomberg => crate::types::Bloomberg::is_canonical(text),
            _ => true,
        };
        if !canonical {
            return refused(format!(
                "expected a securities identifier in its canonical spelling, got {text:?}"
            ));
        }
        Ok(Some(text))
    }
}

// The ten registered codes' values.
// ------------------------------------------------------------------------
// What a [`Cfi`] code means: ISO 10962, six characters.
//
// The [code registry](super::codes) already owns a CFI's width and its
// ASCII validity, which is all a *storage* layer needs. This is the rest of
// it - what the six characters say - and it lives here rather than in any
// protocol module because a CFI is a value, not a message: FIX's
// `CFICode(461)`, an ISIN registry's classification and a lake column all
// read the same six characters the same way, and a protocol that owned the
// reading would be a second one.
//
// # Six positions, and only four of them take `X`
//
// Position 1 is the **category** and position 2 the **group** within it.
// Positions 3 to 6 are four **attributes** whose meaning depends on the
// `(category, group)` pair - position 5 is "payment status" for `ES`,
// "income" for `EP`, "assets" for `CI` and not applicable at all for `SE` -
// so nothing here reads an attribute by position alone.
//
// [`Cfi::UNKNOWN`] means "not applicable or unknown" and is valid **only in
// positions 3 to 6**. There is no valid category `X` and no valid group `X`,
// which is what stops a caller filling a code it does not have: a value that
// knows no category is absent, never `XXXXXX`.
//
// Where a category is known and its group is not, [`Cfi::coarse`] answers
// that category's **Others** group rather than an `X`, because that is how
// the standard itself spells "this kind of thing, kind unspecified": `EM` is
// Equities/Others, `DM` Debt/Others, `CM` CIVs/Others.
//
// # Merging two statements
//
// [`Cfi::merged`] folds two codes for one instrument position by position,
// and only when they agree on what the instrument *is*: same category, same
// group. A stated attribute fills an unknown one, so `ESXXXX` merged with
// `ESVUFR` is `ESVUFR`. Two different stated attributes are a conflict, and
// this crate has one answer for two voices that disagree - ambiguity answers
// nothing - so that position answers `X` rather than picking a winner. Two
// different categories or groups are not a merge at all: they are two
// statements about two different instruments, and the answer is `None`.
//
// # Provenance
//
// The tables are the ISO 10962:2021 code list published by SIX Financial
// Information, the standard's appointed Maintenance Agency, as
// `cfi-20210507-current`. The 2021 edition moved the code list out of the
// standard document and publishes it as a free external list, so this is the
// list itself rather than a reading of it.
// ------------------------------------------------------------------------

impl DataType {
    /// Every registered code, with its canonical name and width.
    ///
    /// The one listing: the parser, the Arrow extension table and every
    /// binding read the codes from here rather than repeating four arms.
    pub const CODES: &'static [(&'static str, DataType, usize)] = &[

        ("country", DataType::Country, COUNTRY_WIDTH),

        ("currency", DataType::Currency, CURRENCY_WIDTH),

        ("mic", DataType::Mic, MIC_WIDTH),

        ("cfi", DataType::Cfi, CFI_WIDTH),

        ("isin", DataType::Isin, ISIN_WIDTH),

        ("cusip", DataType::Cusip, CUSIP_WIDTH),

        ("sedol", DataType::Sedol, SEDOL_WIDTH),

        ("side", DataType::Side, SIDE_WIDTH),

        ("state", DataType::State, STATE_WIDTH),

        ("timeinforce", DataType::TimeInForce, TIMEINFORCE_WIDTH),

        ("bloomberg", DataType::Bloomberg, BLOOMBERG_WIDTH),

    ];
}

impl DataType {
    /// The integer an ASCII value packs into: its bytes padded with trailing
    /// NUL to the width, big-endian.
    ///
    /// The padding is the packing's, never a column's - a code stores as the
    /// text it is - and it is what makes the integer order exactly as the
    /// text does and be the same integer in every process, which is what a
    /// stable hash and a portable enum member both need. An ASCII byte is at
    /// most `0x7F`, so the sign bit is never set and the value is never
    /// negative: four bytes fill an `i32`, eight an `i64`, and sixteen the
    /// whole `i128`.
    ///
    /// Only a fixed US-ASCII width and a registered code have one. A variable
    /// string takes a value of any length, so there is no integer its bytes
    /// always fit; another charset can set the sign bit; and a width above
    /// sixteen bytes outgrows the widest integer this crate carries. All
    /// three are refused rather than truncated.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// // `USD` packs into `USD\0` under `fixed_ascii(4)`, which is that
    /// // big-endian `i32`; under `currency` it is the three bytes alone.
    /// let ccy = DataType::fixed_ascii(4)?;
    /// assert_eq!(ccy.ascii_packed(b"USD")?, 0x5553_4400);
    /// assert_eq!(ccy.ascii_packed(b"USD\0")?, 0x5553_4400);
    /// assert_eq!(ccy.ascii_value(0x5553_4400)?, "USD");
    /// assert_eq!(DataType::Currency.ascii_packed(b"USD")?, 0x0055_5344);
    ///
    /// // The order of the integers is the order of the text.
    /// assert!(ccy.ascii_packed(b"EUR")? < ccy.ascii_packed(b"USD")?);
    ///
    /// // Twelve bytes need 96 bits, and sixteen the whole `i128`.
    /// let twelve = DataType::fixed_ascii(12)?;
    /// let isin = twelve.ascii_packed(b"US0378331005")?;
    /// assert_eq!(twelve.ascii_value(isin)?, "US0378331005");
    /// assert!(isin > i128::from(u64::MAX));
    ///
    /// assert!(ccy.ascii_packed(b"EURO!").is_err());
    /// assert!(DataType::ascii().ascii_packed(b"USD").is_err());
    /// assert!(DataType::fixed_ascii(17)?.ascii_packed(b"USD").is_err());
    /// assert!(DataType::fixed_utf8(4)?.ascii_packed(b"USD").is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the accepted datatypes when this has no
    /// US-ASCII width or its width outgrows an `i128`, and one naming the
    /// width when `value` is not ASCII text that fits it.
    pub fn ascii_packed(&self, value: &[u8]) -> Result<i128> {
        let width = self.packed_width()?;
        let text = ascii_text(width, value)?;
        let mut slot = [0_u8; 16];
        // The text fits the width, and the width fits the slot.
        slot[..text.len()].copy_from_slice(text.as_bytes());
        Ok(i128::from_be_bytes(slot) >> (8 * (16 - width as i128)))
    }

    /// The ASCII value a packed integer carries, without its padding.
    ///
    /// The inverse of [`Self::ascii_packed`], and it refuses exactly what that
    /// refuses: an integer wider than the width, a negative one, and one whose
    /// bytes are not the padded storage of an ASCII value.
    ///
    /// # Errors
    ///
    /// Returns an error naming the accepted widths when this is not one, and
    /// one naming the width when `packed` is not the storage of an ASCII value
    /// of it.
    pub fn ascii_value(&self, packed: i128) -> Result<Str> {
        let width = self.packed_width()?;
        let bytes = packed.to_be_bytes();
        let (above, stored) = bytes.split_at(bytes.len() - width);
        if above.iter().any(|byte| *byte != 0) {
            return Err(ascii_refusal(
                Some(width),
                format_smolstr!("the integer {packed}, which is wider than the width"),
            ));
        }
        let text = ascii_text(width, stored)?;
        Str::new(text).try_with_parameters(match self {
            Self::String(parameters) => *parameters,
            // A code is US-ASCII bounded at the width its standard fixes.
            _ => super::StringType::ascii(super::StringLayout::String)
                .try_with_bound(width as u32)?,
        })
    }

    /// The width a packed integer may be built from, refusing the rest.
    ///
    /// A code packs into the width its standard fixes; a string packs into
    /// the width that makes it fixed, and only in US-ASCII, where no byte
    /// sets the sign bit.
    fn packed_width(&self) -> Result<usize> {
        let width = match self.code_width() {
            Some(width) => Some(width),
            None if self
                .string_parameters()
                .is_some_and(|parameters| parameters.charset() == crate::Charset::Ascii) =>
            {
                self.fixed_byte_width()
            }
            None => None,
        };
        match width {
            Some(width) if width <= PACKED_LIMIT => Ok(width),
            _ => Err(ascii_values_refusal(self)),
        }
    }
}

/// The widest fixed ASCII storage one `i128` holds, in bytes.
const PACKED_LIMIT: usize = 16;

/// Validates bytes as an ASCII value of at most `width` bytes and trims the
/// trailing NUL padding.
///
/// The one validator every arm calls: field validation and canonicalization,
/// Arrow ingest, and casts all answer the same trimmed text or the same
/// refusal naming the width.
///
/// # Errors
///
/// Returns an error naming the width when the trimmed bytes hold a NUL, a
/// non-ASCII byte, or more than `width` bytes.
pub(crate) fn ascii_text(width: usize, bytes: &[u8]) -> Result<&str> {
    ascii_text_sized(Some(width), bytes)
}

/// [`ascii_text`] over the width the caller already holds, if there is one.
///
/// The one body every shape runs: a fixed width passes its length, and a
/// registered code passes its constant, which lets the length check fold at
/// each code's call site.
#[inline]
pub(crate) fn ascii_text_sized(width: Option<usize>, bytes: &[u8]) -> Result<&str> {
    let text = crate::types::string::trim_padding(bytes);
    if let Some(position) = text.iter().position(|byte| *byte == 0) {
        return Err(ascii_refusal(
            width,
            format_smolstr!("a NUL byte at {position}"),
        ));
    }
    // The byte class is one fact and [`crate::Charset`] owns it: this is that
    // charset's own scan, which reads a machine word at a time, rather than a
    // second one written here.
    let position = crate::charset::ascii_len(text);
    if position < text.len() {
        return Err(ascii_refusal(
            width,
            format_smolstr!("a non-ASCII byte 0x{:02X} at {position}", text[position]),
        ));
    }
    if width.is_some_and(|width| text.len() > width) {
        return Err(ascii_refusal(
            width,
            format_smolstr!("{} bytes", text.len()),
        ));
    }
    // Every byte is ASCII, so the slice is UTF-8 by construction.
    std::str::from_utf8(text).map_err(|error| ascii_refusal(width, format_smolstr!("{error}")))
}

/// The bytes a code value carries, in any accepted spelling.
pub(crate) fn ascii_bytes(value: &Scalar) -> Option<&[u8]> {
    match value {
        Scalar::String(text) => Some(text.as_str().as_bytes()),
        code if code.is_code() => code.as_str().map(str::as_bytes),
        Scalar::Bytes(bytes) => Some(bytes.as_bytes()),
        _ => None,
    }
}

fn ascii_refusal(width: Option<usize>, actual: SmolStr) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: match width {
            Some(width) => crate::text::expected_got(
                format_args!("ASCII text of at most {width} bytes"),
                actual,
            ),
            None => crate::text::expected_got(format_args!("ASCII text"), actual),
        },
    }
}

/// Refuses a datatype that has no packed integer.
fn ascii_values_refusal(values: &DataType) -> Error {
    Error::InvalidDataType {
        kind: "ascii",
        reason: crate::text::expected_got(
            format_args!(
                "a fixed US-ASCII string of at most {PACKED_LIMIT} bytes, or a registered code"
            ),
            format_args!("{values}"),
        ),
    }
}

// ------------------------------------------------------------------------
// Named member dictionaries for string fields.
// ------------------------------------------------------------------------

/// The enum a string field's values name: one value per member name.
///
/// This is the vocabulary a declaration named itself, and it is what a
/// [`crate::Field`] stores under `field:enum` so the enum crosses Arrow, a
/// file, and another runtime intact.
///
/// The width lives in the field's datatype and is never copied here: a
/// member's code is [`DataType::ascii_packed`] of its value under that width,
/// so every reader of one enum answers the same integers. Members are held by
/// name, which is what makes the rendered document deterministic - the order a
/// declaration happened to use is not part of a member's identity once the
/// code is the value's own bytes.
///
/// ```
/// use yggdryl::{DataType, StringEnum};
///
/// # fn main() -> yggdryl::Result<()> {
/// let side = StringEnum::from_members("Side", [("BUY", "B"), ("SELL", "S")])?;
/// assert_eq!(side.get("BUY"), Some("B"));
/// assert_eq!(side.get_member("S"), Some("SELL"));
/// assert_eq!(
///     side.into_members(&DataType::fixed_ascii(4)?)?,
///     [("BUY".into(), 0x4200_0000), ("SELL".into(), 0x5300_0000)]
/// );
/// assert_eq!(
///     side.into_json(),
///     r#"{"members":{"BUY":"B","SELL":"S"},"name":"Side"}"#
/// );
/// assert_eq!(StringEnum::from_json(&side.into_json())?, side);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StringEnum {
    /// The enum's own name, which is not the field's name.
    name: Str,
    /// Member name to string value, ordered by name so the document is one text.
    members: BTreeMap<Str, Str>,
}

impl StringEnum {
    /// Creates an enum of no members under one name.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` is empty or holds a control character.
    pub fn new(name: impl Into<Str>) -> Result<Self> {
        let name = name.into();
        validate_enum_text("enum name", &name)?;
        Ok(Self {
            name,
            members: BTreeMap::new(),
        })
    }

    /// Creates an enum from its members, one string value per member name.
    ///
    /// A repeated member name keeps the last value, exactly as [`Self::insert`]
    /// would; two members may share a value, because two spellings of one code
    /// is what an alias is.
    ///
    /// # Errors
    ///
    /// Returns an error when the enum name or a member name is empty or holds
    /// a control character.
    pub fn from_members<I, N, V>(name: impl Into<Str>, members: I) -> Result<Self>
    where
        I: IntoIterator<Item = (N, V)>,
        N: Into<Str>,
        V: Into<Str>,
    {
        let mut enumeration = Self::new(name)?;
        for (member, value) in members {
            enumeration.insert(member, value)?;
        }
        Ok(enumeration)
    }

    /// Parses the `field:enum` document.
    ///
    /// # Errors
    ///
    /// Returns an error when the document is not a JSON object of a string
    /// `"name"` and an object `"members"` of strings, and one naming the part
    /// when a name is empty or holds a control character.
    pub fn from_json(document: &str) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_str(document.trim()).map_err(|error| {
            enum_document_refusal(format_smolstr!(
                "expected an enum JSON document, got unparsable JSON: {error}"
            ))
        })?;
        let Some(object) = value.as_object() else {
            return Err(enum_document_refusal(format_smolstr!(
                "expected an enum JSON object, got {}",
                crate::text::elide_display(&value)
            )));
        };
        let Some(serde_json::Value::String(name)) = object.get("name") else {
            return Err(enum_document_refusal(SmolStr::new_static(
                "expected a JSON string \"name\"",
            )));
        };
        let empty = serde_json::Map::new();
        let members = match object.get("members") {
            None | Some(serde_json::Value::Null) => &empty,
            Some(serde_json::Value::Object(members)) => members,
            Some(other) => {
                return Err(enum_document_refusal(format_smolstr!(
                    "expected a JSON object \"members\", got {}",
                    crate::text::elide_display(other)
                )));
            }
        }
        .iter()
        .map(|(member, value)| match value {
            serde_json::Value::String(value) => Ok((member.as_str(), value.as_str())),
            other => Err(enum_document_refusal(format_smolstr!(
                "expected a JSON string for the member {member:?}, got {}",
                crate::text::elide_display(other)
            ))),
        })
        .collect::<Result<Vec<_>>>()?;
        Self::from_members(name.as_str(), members)
    }

    /// Renders the `field:enum` document: every name in order, so one enum
    /// is one text however it was built.
    pub fn into_json(&self) -> String {
        let members = self
            .members
            .iter()
            .map(|(member, value)| {
                (
                    member.as_str().to_owned(),
                    serde_json::Value::String(value.as_str().to_owned()),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        serde_json::Value::Object(serde_json::Map::from_iter([
            (
                "name".to_owned(),
                serde_json::Value::String(self.name.as_str().to_owned()),
            ),
            ("members".to_owned(), serde_json::Value::Object(members)),
        ]))
        .to_string()
    }

    /// The enum's own name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The value one member names, or `None` for a member it has not.
    pub fn get(&self, member: &str) -> Option<&str> {
        self.members.get(member).map(Str::as_str)
    }

    /// The first member naming one value, or `None` when none does.
    ///
    /// Two members may share a value; the first by name answers, so an alias
    /// never changes which member a stored value reads back as.
    pub fn get_member(&self, value: &str) -> Option<&str> {
        self.members
            .iter()
            .find(|(_, held)| held.as_str() == value)
            .map(|(member, _)| member.as_str())
    }

    /// Names one value and returns the value the member had.
    ///
    /// # Errors
    ///
    /// Returns an error when `member` is empty or holds a control character.
    pub fn insert(&mut self, member: impl Into<Str>, value: impl Into<Str>) -> Result<Option<Str>> {
        let member = member.into();
        validate_enum_text("member name", &member)?;
        Ok(self.members.insert(member, value.into()))
    }

    /// Removes one member and returns the value it named.
    pub fn remove(&mut self, member: &str) -> Option<Str> {
        self.members.remove(member)
    }

    /// The number of members.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Returns whether this enum names nothing.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The members by name, each with the value it names.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.members
            .iter()
            .map(|(member, value)| (member.as_str(), value.as_str()))
    }

    /// The enum member name one value takes.
    ///
    /// An ASCII letter is kept uppercased, a digit is kept, every other byte
    /// becomes `_`, a leading digit takes a `_` in front, and a name that both
    /// opens and closes with `_` drops its trailing underscores - that shape
    /// is what Python reserves for `_sunder_` and `__dunder__` names, where a
    /// member is refused or silently dropped.
    ///
    /// The rule belongs to the vocabulary rather than to one width, so an enum
    /// that registers one value at a time names it exactly as generating a
    /// whole listing at once would.
    ///
    /// ```
    /// use yggdryl::StringEnum;
    ///
    /// assert_eq!(StringEnum::member_name("USD").as_str(), "USD");
    /// assert_eq!(StringEnum::member_name("n/a").as_str(), "N_A");
    /// assert_eq!(StringEnum::member_name("-a-").as_str(), "_A");
    /// assert_eq!(StringEnum::member_name("").as_str(), "_");
    /// ```
    #[must_use]
    pub fn member_name(value: &str) -> Str {
        let mut name = String::with_capacity(value.len() + 1);
        // A packed value is ASCII, so one byte is one character. Any other
        // byte is not ASCII alphanumeric, so it becomes `_` like the rest of
        // what the rule replaces.
        for byte in value.bytes() {
            name.push(if byte.is_ascii_alphanumeric() {
                char::from(byte.to_ascii_uppercase())
            } else {
                '_'
            });
        }
        if name.starts_with(|first: char| first.is_ascii_digit()) {
            name.insert(0, '_');
        }
        // A name that both opens and closes with `_` carries the shape Python
        // reserves for `_sunder_` and `__dunder__`, where a member is refused or
        // silently dropped; a name of nothing but `_` has no other spelling.
        let named = name.trim_end_matches('_').len();
        if name.starts_with('_') && named > 0 {
            name.truncate(named);
        }
        if name.is_empty() {
            return Str::new_static("_");
        }
        Str::from(name)
    }

    /// The members paired with their packed codes under one fixed US-ASCII
    /// width.
    ///
    /// # Errors
    ///
    /// Returns an error naming the accepted widths when `width` is not one,
    /// and one naming the width when a value does not fit it.
    pub fn into_members(&self, width: &DataType) -> Result<Vec<(Str, i128)>> {
        self.members
            .iter()
            .map(|(member, value)| Ok((member.clone(), width.ascii_packed(value.as_bytes())?)))
            .collect()
    }
}

fn enum_document_refusal(reason: SmolStr) -> Error {
    Error::InvalidDataType {
        kind: "string-enum",
        reason,
    }
}

/// Refuses the two spellings a stored document could not carry back.
fn validate_enum_text(part: &'static str, value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(enum_document_refusal(format_smolstr!(
            "expected a non-empty {part}"
        )));
    }
    if let Some(position) = value.chars().position(char::is_control) {
        return Err(enum_document_refusal(format_smolstr!(
            "expected a {part} with no control character, got one at {position}"
        )));
    }
    Ok(())
}

// ------------------------------------------------------------------------
// The one door into the string family, and the questions every string answers.
// ------------------------------------------------------------------------
impl DataType {
    /// The string datatype these parameters name.
    ///
    /// This is the family's one constructor. Every string is
    /// [`DataType::String`]; what differs is what it declares, and the
    /// parameters say all of it - a layout, a charset, and a bound.
    ///
    /// ```
    /// use yggdryl::types::{StringLayout, StringType};
    /// use yggdryl::{Charset, DataType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// // Plain UTF-8 renders under Arrow's own name.
    /// let plain = StringType::utf8(StringLayout::LargeString);
    /// assert_eq!(DataType::string(plain)?, DataType::large_utf8());
    /// assert_eq!(DataType::large_utf8().to_string(), "large_utf8");
    ///
    /// // A charset or a bound is what a string declares.
    /// let bounded = StringType::utf8(StringLayout::String).try_with_bound(32)?;
    /// assert_eq!(DataType::string(bounded)?.to_string(), "utf8(32)");
    /// assert_eq!(DataType::string(Charset::Cp1252)?.to_string(), "string(windows-1252)");
    /// assert_eq!(DataType::fixed_ascii(4)?.to_string(), "fixed_ascii(4)");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] for a fixed layout with no width:
    /// the width is what makes it fixed.
    pub fn string(parameters: impl Into<StringType>) -> Result<Self> {
        let parameters = parameters.into();
        parameters.validate()?;
        Ok(Self::String(parameters))
    }

    /// Unbounded UTF-8 with 32-bit offsets - Arrow's `Utf8`.
    #[must_use]
    pub const fn utf8() -> Self {
        Self::String(StringType::utf8(StringLayout::String))
    }

    /// Unbounded UTF-8 with 64-bit offsets - Arrow's `LargeUtf8`.
    #[must_use]
    pub const fn large_utf8() -> Self {
        Self::String(StringType::utf8(StringLayout::LargeString))
    }

    /// Unbounded UTF-8 in the view layout - Arrow's `Utf8View`.
    #[must_use]
    pub const fn utf8_view() -> Self {
        Self::String(StringType::utf8(StringLayout::StringView))
    }

    /// Unbounded US-ASCII with 32-bit offsets.
    #[must_use]
    pub const fn ascii() -> Self {
        Self::String(StringType::ascii(StringLayout::String))
    }

    /// UTF-8 of exactly `width` stored bytes, padded with trailing NUL.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] for a width of zero.
    pub fn fixed_utf8(width: u32) -> Result<Self> {
        Self::string(StringType::utf8(StringLayout::FixedString).try_with_bound(width)?)
    }

    /// US-ASCII of exactly `width` stored bytes, padded with trailing NUL.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(DataType::fixed_ascii(4)?.to_string(), "fixed_ascii(4)");
    /// assert_eq!(DataType::fixed_ascii(4)?.fixed_byte_width(), Some(4));
    /// assert_eq!(DataType::ascii().to_string(), "ascii");
    /// assert_eq!(DataType::ascii().fixed_byte_width(), None);
    /// assert!(DataType::fixed_ascii(0).is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] for a width of zero.
    pub fn fixed_ascii(width: u32) -> Result<Self> {
        Self::string(StringType::ascii(StringLayout::FixedString).try_with_bound(width)?)
    }

    /// The parameters a string datatype declares, `None` for every other.
    ///
    /// The registered codes are deliberately not here. A currency is an
    /// identity over ISO 4217 the way a URL is one over RFC 3986 - both store
    /// as text, and neither is a string with a charset - so a code answers
    /// [`DataType::code_width`] and [`DataType::is_code`] instead, and its
    /// Arrow extension name is what keeps a column of one from importing as
    /// the plain text beside it.
    ///
    /// ```
    /// use yggdryl::types::StringLayout;
    /// use yggdryl::{Charset, DataType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let utf8 = DataType::utf8().string_parameters().expect("a string datatype");
    /// assert_eq!(utf8.layout(), StringLayout::String);
    /// assert_eq!(utf8.charset(), Charset::Utf8);
    ///
    /// let ascii = DataType::fixed_ascii(3)?.string_parameters().expect("a string datatype");
    /// assert_eq!(ascii.charset(), Charset::Ascii);
    /// assert_eq!(ascii.fixed(), Some(3));
    ///
    /// assert!(DataType::Currency.string_parameters().is_none());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn string_parameters(&self) -> Option<StringType> {
        match self {
            Self::String(parameters) => Some(*parameters),
            _ => None,
        }
    }

    /// Return whether this datatype is a string.
    #[must_use]
    pub const fn is_string(&self) -> bool {
        matches!(self, Self::String(_))
    }

    /// The charset a string column's bytes are written in.
    ///
    /// `None` is a datatype that is not a string; every string has one,
    /// because UTF-8 is what a string with nothing declared is in.
    #[must_use]
    pub const fn charset(&self) -> Option<Charset> {
        match self.string_parameters() {
            Some(parameters) => Some(parameters.charset()),
            None => None,
        }
    }

    /// The fixed byte width of one value, when this datatype has one.
    ///
    /// [`crate::DataTypeId::fixed_byte_width`] answers for every
    /// parameter-free variant - the numbers, a UUID; this adds the two whose
    /// width is a parameter: a fixed string and fixed bytes. A registered
    /// code's width is a maximum over the text it stores rather than a
    /// layout, so it answers [`DataType::code_width`] instead.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(DataType::fixed_ascii(4)?.fixed_byte_width(), Some(4));
    /// assert_eq!(DataType::fixed_size_binary(16)?.fixed_byte_width(), Some(16));
    /// assert_eq!(DataType::utf8().fixed_byte_width(), None);
    /// assert_eq!(DataType::Currency.fixed_byte_width(), None);
    /// assert_eq!(DataType::Currency.code_width(), Some(3));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn fixed_byte_width(&self) -> Option<usize> {
        match self {
            Self::String(parameters) => match parameters.fixed() {
                Some(width) => Some(width as usize),
                None => None,
            },
            Self::Bytes(parameters) => match parameters.fixed() {
                Some(width) => Some(width as usize),
                None => None,
            },
            _ => self.id().fixed_byte_width(),
        }
    }
}

// ------------------------------------------------------------------------
// Every string's field marker: the one family and the ten codes.
//
// One file because a marker is one line per datatype and the family is one
// family; splitting them would be two lists to keep in step rather than one.
// ------------------------------------------------------------------------


impl Field {
    /// The enum this field's string values name, if one is declared.
    ///
    /// # Errors
    ///
    /// Returns an error only for externally corrupted serialized state.
    pub fn string_enum(&self) -> Result<Option<StringEnum>> {
        self.get_metadata(FIELD_ENUM_KEY)
            .map(parse_string_enum)
            .transpose()
    }

    /// Declares the enum this field's string values name.
    ///
    /// # Errors
    ///
    /// Returns an error when this field cannot store every enum member: the
    /// datatype must be a fixed US-ASCII string of at most sixteen bytes or
    /// a registered code, because a member's code is its value's own bytes
    /// packed into one integer.
    pub fn set_string_enum(&mut self, value: &StringEnum) -> Result<()> {
        value.into_members(self.dtype())?;
        let (_, changed) = self
            .metadata_mut()
            .insert_validated(FIELD_ENUM_KEY.to_owned(), value.into_json());
        if changed {
            self.invalidate_arrow();
        }
        Ok(())
    }

    /// Returns a persistent field declaring one enum over its string values.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::set_string_enum`] does.
    pub fn try_with_string_enum(mut self, value: &StringEnum) -> Result<Self> {
        self.set_string_enum(value)?;
        Ok(self)
    }

    /// Removes the declaration and returns the enum it held.
    ///
    /// # Errors
    ///
    /// Returns an error only for externally corrupted serialized state.
    pub fn remove_string_enum(&mut self) -> Result<Option<StringEnum>> {
        self.remove_metadata(FIELD_ENUM_KEY)
            .map(|value| parse_string_enum(&value))
            .transpose()
    }
}

// ------------------------------------------------------------------------
// What a string datatype declares beyond its layout.
// ------------------------------------------------------------------------

/// The name this crate's string datatypes ride Arrow under.
pub const STRING_EXTENSION_NAME: &str = "yggdryl.string";

/// A string layout, the charset its bytes are written in, and its bound.
///
/// Arrow carries none of the three together: its string layouts imply UTF-8
/// and declare no length at all, and its binary layouts declare neither. So
/// all three ride here, and cross an Arrow boundary as this crate's own
/// extension metadata under `yggdryl.string`.
///
/// The bound counts **bytes of the stored encoding**, not scalars. That is
/// the number the buffer holds, the number Arrow's offsets measure, and - for
/// every single-byte charset - the scalar count as well. Counting scalars
/// instead would make a bound a walk of the value rather than a subtraction
/// of two offsets.
///
/// One number carries both bounds because a string is one shape or the other:
/// on [`StringLayout::FixedString`] it is the exact width every value fills,
/// and on every other layout it is the most bytes a value may hold. So
/// [`Self::fixed`] and [`Self::max`] are two readings of one fact, and
/// exactly one of them ever answers.
///
/// ```
/// use yggdryl::types::{StringLayout, StringType};
/// use yggdryl::{Charset, DataType};
///
/// # fn main() -> yggdryl::Result<()> {
/// let parameters = StringType::new(StringLayout::String, Charset::Cp1252).try_with_bound(32)?;
/// assert_eq!(parameters.charset(), Charset::Cp1252);
/// assert_eq!(parameters.max(), Some(32));
/// assert_eq!(parameters.fixed(), None);
///
/// let dtype = DataType::string(parameters)?;
/// assert_eq!(dtype.to_string(), "string(windows-1252,32)");
/// assert_eq!(DataType::from_str(&dtype.to_string())?, dtype);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StringType {
    layout: StringLayout,
    charset: Charset,
    bound: Option<NonZeroU32>,
}

impl StringType {
    /// An unbounded string in one layout and charset.
    #[must_use]
    pub const fn new(layout: StringLayout, charset: Charset) -> Self {
        Self {
            layout,
            charset,
            bound: None,
        }
    }

    /// An unbounded UTF-8 string in one layout.
    #[must_use]
    pub const fn utf8(layout: StringLayout) -> Self {
        Self::new(layout, Charset::Utf8)
    }

    /// An unbounded US-ASCII string in one layout.
    #[must_use]
    pub const fn ascii(layout: StringLayout) -> Self {
        Self::new(layout, Charset::Ascii)
    }

    /// The layout the values are stored in.
    #[must_use]
    pub const fn layout(self) -> StringLayout {
        self.layout
    }

    /// The charset the stored bytes are written in.
    #[must_use]
    pub const fn charset(self) -> Charset {
        self.charset
    }

    /// The declared byte bound, whichever shape the layout gives it.
    #[must_use]
    pub const fn bound(self) -> Option<u32> {
        match self.bound {
            Some(bound) => Some(bound.get()),
            None => None,
        }
    }

    /// The exact bytes every value fills, on a fixed layout.
    #[must_use]
    pub const fn fixed(self) -> Option<u32> {
        match self.layout.is_fixed() {
            true => self.bound(),
            false => None,
        }
    }

    /// The most bytes a value may hold, on a variable layout.
    #[must_use]
    pub const fn max(self) -> Option<u32> {
        match self.layout.is_fixed() {
            true => None,
            false => self.bound(),
        }
    }

    /// Return whether every value is the same width.
    #[must_use]
    pub const fn is_fixed(self) -> bool {
        self.layout.is_fixed()
    }

    /// Return whether the values are bounded at all.
    #[must_use]
    pub const fn is_bounded(self) -> bool {
        self.bound.is_some()
    }

    /// Return these parameters in another layout.
    #[must_use]
    pub const fn with_layout(mut self, layout: StringLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Return these parameters in another charset.
    #[must_use]
    pub const fn with_charset(mut self, charset: Charset) -> Self {
        self.charset = charset;
        self
    }

    /// Return these parameters bounded to `bound` bytes, the bound already
    /// proven non-zero.
    #[must_use]
    pub const fn with_bound(mut self, bound: NonZeroU32) -> Self {
        self.bound = Some(bound);
        self
    }

    /// Return these parameters bounded to `bound` bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a bound of zero: a string of no
    /// bytes is a column of one value, which is a declaration nobody means.
    pub fn try_with_bound(mut self, bound: u32) -> Result<Self> {
        self.bound = Some(NonZeroU32::new(bound).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a {} of at least one byte, got 0",
                self.bound_word()
            ))
        })?);
        Ok(self)
    }

    /// Return these parameters with no bound.
    #[must_use]
    pub const fn without_bound(mut self) -> Self {
        self.bound = None;
        self
    }

    /// Return these parameters with no maximum, keeping a fixed width.
    ///
    /// A maximum is a column's rule and a fixed width is a value's shape, so
    /// this is what a value carries out of a bounded column.
    #[must_use]
    pub const fn without_max(self) -> Self {
        match self.layout.is_fixed() {
            true => self,
            false => self.without_bound(),
        }
    }

    /// Check that the layout and the bound agree.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a fixed layout with no width:
    /// the width is what makes it fixed, so there is no width-free spelling
    /// of it.
    pub fn validate(self) -> Result<()> {
        if self.layout.is_fixed() && self.bound.is_none() {
            return Err(invalid(format_smolstr!(
                "expected {}(width), got no width",
                self.layout.as_str()
            )));
        }
        Ok(())
    }

    /// The word this layout calls its bound by.
    const fn bound_word(self) -> &'static str {
        match self.layout.is_fixed() {
            true => "width",
            false => "maximum",
        }
    }

    /// The name this layout takes under this charset.
    ///
    /// UTF-8 and US-ASCII each earn the layout's short spelling; every other
    /// charset renders under the general name and states itself beside it.
    #[must_use]
    pub const fn layout_name(self) -> &'static str {
        match self.charset {
            Charset::Utf8 => self.layout.as_utf8_str(),
            Charset::Ascii => self.layout.as_ascii_str(),
            _ => self.layout.as_str(),
        }
    }

    /// Return whether a charset-named spelling already says the charset.
    const fn charset_is_named(self) -> bool {
        matches!(self.charset, Charset::Utf8 | Charset::Ascii)
    }

    /// The extension metadata an Arrow field carries these in.
    ///
    /// Arrow has nowhere else to put them: neither a string nor a binary
    /// array declares a charset, a length bound, or which of the two view
    /// layouts it is, so all three ride the `ARROW:extension:metadata`
    /// document beside the `yggdryl.string` name. The layout is written
    /// whole rather than inferred, because a reader that only knows Arrow
    /// sees one view layout where this crate declares two.
    #[must_use]
    pub fn extension_json(self) -> String {
        let mut rendered = String::with_capacity(64);
        rendered.push_str("{\"layout\":\"");
        rendered.push_str(self.layout.as_str());
        rendered.push_str("\",\"charset\":\"");
        rendered.push_str(self.charset.as_str());
        rendered.push('"');
        if let Some(bound) = self.bound {
            rendered.push(',');
            rendered.push('"');
            rendered.push_str(self.bound_word_key());
            rendered.push_str("\":");
            rendered.push_str(&format_smolstr!("{bound}"));
        }
        rendered.push('}');
        rendered
    }

    /// The metadata key the bound is written under.
    const fn bound_word_key(self) -> &'static str {
        match self.layout.is_fixed() {
            true => "fixed",
            false => "max",
        }
    }

    /// Read parameters back out of Arrow extension metadata.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] when the document is not an object
    /// naming a layout and a charset this crate knows, or bounds a layout the
    /// wrong way.
    pub fn from_extension_json(value: &str) -> Result<Self> {
        #[derive(Deserialize)]
        struct Document {
            layout: SmolStr,
            charset: SmolStr,
            #[serde(default)]
            max: Option<u32>,
            #[serde(default)]
            fixed: Option<u32>,
        }

        let document: Document = serde_json::from_str(value)
            .map_err(|error| invalid(format_smolstr!("expected string parameters, got {error}")))?;
        let layout = StringLayout::from_str(&document.layout)
            .map_err(|error| invalid(format_smolstr!("{error}")))?;
        let charset = Charset::from_str(&document.charset)
            .map_err(|error| invalid(format_smolstr!("{error}")))?;
        let mut parameters = Self::new(layout, charset);
        match (layout.is_fixed(), document.fixed, document.max) {
            (true, Some(fixed), None) => parameters = parameters.try_with_bound(fixed)?,
            (false, None, Some(max)) => parameters = parameters.try_with_bound(max)?,
            (_, None, None) => {}
            (true, _, Some(max)) => {
                return Err(invalid(format_smolstr!(
                    "expected a fixed width on {layout}, got max={max}"
                )));
            }
            (false, Some(fixed), _) => {
                return Err(invalid(format_smolstr!(
                    "expected a maximum on {layout}, got fixed={fixed}"
                )));
            }
        }
        parameters.validate()?;
        Ok(parameters)
    }

}

/// The refusal every invalid parameter answers with.
fn invalid(reason: impl Into<SmolStr>) -> Error {
    Error::InvalidDataType {
        kind: "string",
        reason: reason.into(),
    }
}

impl Default for StringType {
    fn default() -> Self {
        Self::utf8(StringLayout::String)
    }
}

impl From<Charset> for StringType {
    fn from(value: Charset) -> Self {
        Self::new(StringLayout::String, value)
    }
}

impl From<StringLayout> for StringType {
    fn from(value: StringLayout) -> Self {
        Self::utf8(value)
    }
}

impl fmt::Display for StringType {
    /// The canonical spelling, which [`crate::DataType`]'s grammar reads back.
    ///
    /// UTF-8 renders under the layout's `utf8` name and US-ASCII under its
    /// `ascii` name, with no charset to state; every other charset renders
    /// under the `string` name and states it.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.layout_name())?;
        match (self.charset_is_named(), self.bound) {
            (true, None) => Ok(()),
            (true, Some(bound)) => write!(formatter, "({bound})"),
            (false, None) => write!(formatter, "({})", self.charset),
            (false, Some(bound)) => write!(formatter, "({},{bound})", self.charset),
        }
    }
}

impl Serialize for StringType {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        let declared = usize::from(self.bound.is_some());
        let mut state = serializer.serialize_struct("StringType", 2 + declared)?;
        state.serialize_field("layout", &self.layout)?;
        state.serialize_field("charset", &self.charset)?;
        if let Some(bound) = self.bound {
            state.serialize_field(self.bound_word_key(), &bound.get())?;
        }
        state.end()
    }
}

impl<'de> Deserialize<'de> for StringType {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Representation {
            layout: StringLayout,
            charset: Charset,
            #[serde(default)]
            max: Option<u32>,
            #[serde(default)]
            fixed: Option<u32>,
        }

        let value = Representation::deserialize(deserializer)?;
        let mut parameters = Self::new(value.layout, value.charset);
        if let Some(bound) = value.fixed.or(value.max) {
            parameters = parameters
                .try_with_bound(bound)
                .map_err(serde::de::Error::custom)?;
        }
        parameters.validate().map_err(serde::de::Error::custom)?;
        Ok(parameters)
    }
}

// ------------------------------------------------------------------------
// The one grammar every string spelling reads through.
// ------------------------------------------------------------------------
impl Parser<'_> {
    /// Parse one string's optional charset and optional byte bound.
    ///
    /// The parameter list is positional and at most two long: a word is the
    /// charset, a number is the bound. Which bound it is follows from the
    /// layout - the exact width on a fixed string, the maximum on every
    /// other - so there is one number to write and one meaning it can have.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Parse`] for a charset beside a `utf8` or
    /// `ascii` spelling, since the name already declares one; for a charset
    /// no name answers to; and for a bound outside a positive `u32`.
    pub(crate) fn parse_string(&mut self, layout: StringLayout, keyword: &str) -> Result<DataType> {
        // The `utf8` and `ascii` spellings put the charset in the name, so
        // naming another one beside them would be two answers to one
        // question. The keyword arrives folded and the layout's names are
        // not, so they meet on the fold rather than on the underscores.
        let named = if crate::types::parser::folds_equal(keyword, layout.as_utf8_str()) {
            Some(Charset::Utf8)
        } else if crate::types::parser::folds_equal(keyword, layout.as_ascii_str()) {
            Some(Charset::Ascii)
        } else {
            None
        };
        let mut parameters = StringType::new(layout, named.unwrap_or(Charset::Utf8));
        let mut bound = None;

        if let Some(close) = self.consume_opening() {
            // Empty parentheses are the bare spelling with punctuation.
            if !self.consume_symbol(close) {
                if self.peek_integer().is_none() {
                    let position = self.current_position();
                    let name = self.parse_text("a charset")?;
                    if named.is_some() {
                        return Err(self.error_at(
                            position,
                            format_smolstr!(
                                "expected no charset on {keyword}, got {name:?}; {} is the spelling that takes one",
                                layout.as_str()
                            ),
                        ));
                    }
                    let charset = Charset::from_str(&name)
                        .map_err(|error| self.error_at(position, format_smolstr!("{error}")))?;
                    parameters = parameters.with_charset(charset);
                    if self.consume_separator() {
                        bound = Some(self.parse_bound(layout)?);
                    }
                } else {
                    bound = Some(self.parse_bound(layout)?);
                }
                self.expect_symbol(close)?;
            }
        }

        let position = self.current_position();
        if let Some(bound) = bound {
            parameters = parameters
                .try_with_bound(bound)
                .map_err(|error| self.error_at(position, format_smolstr!("{error}")))?;
        }
        DataType::string(parameters)
            .map_err(|error| self.error_at(position, format_smolstr!("{error}")))
    }

    /// Read the one number a string's parameter list carries.
    fn parse_bound(&mut self, layout: StringLayout) -> Result<u32> {
        let position = self.current_position();
        let label = match layout.is_fixed() {
            true => "a byte width",
            false => "a maximum byte length",
        };
        let value = self.parse_integer(label)?;
        u32::try_from(value).map_err(|_| {
            self.error_at(
                position,
                format_smolstr!("expected {label} inside u32, got {value}"),
            )
        })
    }
}

// ------------------------------------------------------------------------
// The prebuilt vocabularies: the codes a common column starts from.
//
// A registered-code column carries values from a published registry, and
// most of a stream is the handful of codes that registry actually assigns.
// These are those listings, one constant per registry: ISO 4217 currencies,
// ISO 3166-1 alpha-2 countries, and the ISO 10383 market identifier codes of
// the venues those trades reach. Each is sorted, so a reviewer can diff it
// and a repeat is visible. The MICs are a common set rather than the whole
// ISO 10383 registry, which is thousands of segment codes: a vocabulary
// holding all of them costs every column the whole registry and buys nothing
// a declaration does not already give.
//
// [`StringEnum::from_logical_name`] builds one as the enum a field declares,
// so the members a schema carries under `field:enum` come from one listing
// rather than from a copy per language. Every value fits the width its
// registered name resolves to, so a prebuilt vocabulary never refuses its own
// listing.
// ------------------------------------------------------------------------
impl StringEnum {
    /// The currently assigned ISO 4217 alphabetic currency codes, sorted.
    ///
    /// The whole active table rather than a major-currency subset: the fund
    /// codes (`CHE`, `USN`, `UYW`), the precious metals (`XAU`, `XAG`, `XPT`,
    /// `XPD`), and the two a system needs in place of a currency - `XXX` for
    /// no currency and `XTS` for a test value - are codes a real stream
    /// carries, and a subset would push them onto auto-registration.
    ///
    /// A withdrawn code is not here, successor and all: `XCG` is assigned and
    /// `ANG` is not, `SLE` and not `SLL`, `ZWG` and not `ZWL`. A stream
    /// replaying older trades still encodes them - they register on first
    /// sight, which is what the constant leaves auto-registration for.
    pub const CURRENCIES: &'static [&'static str] = &[
        "AED", "AFN", "ALL", "AMD", "AOA", "ARS", "AUD", "AWG", "AZN", "BAM", "BBD", "BDT", "BHD",
        "BIF", "BMD", "BND", "BOB", "BOV", "BRL", "BSD", "BTN", "BWP", "BYN", "BZD", "CAD", "CDF",
        "CHE", "CHF", "CHW", "CLF", "CLP", "CNY", "COP", "COU", "CRC", "CUP", "CVE", "CZK", "DJF",
        "DKK", "DOP", "DZD", "EGP", "ERN", "ETB", "EUR", "FJD", "FKP", "GBP", "GEL", "GHS", "GIP",
        "GMD", "GNF", "GTQ", "GYD", "HKD", "HNL", "HTG", "HUF", "IDR", "ILS", "INR", "IQD", "IRR",
        "ISK", "JMD", "JOD", "JPY", "KES", "KGS", "KHR", "KMF", "KPW", "KRW", "KWD", "KYD", "KZT",
        "LAK", "LBP", "LKR", "LRD", "LSL", "LYD", "MAD", "MDL", "MGA", "MKD", "MMK", "MNT", "MOP",
        "MRU", "MUR", "MVR", "MWK", "MXN", "MXV", "MYR", "MZN", "NAD", "NGN", "NIO", "NOK", "NPR",
        "NZD", "OMR", "PAB", "PEN", "PGK", "PHP", "PKR", "PLN", "PYG", "QAR", "RON", "RSD", "RUB",
        "RWF", "SAR", "SBD", "SCR", "SDG", "SEK", "SGD", "SHP", "SLE", "SOS", "SRD", "SSP", "STN",
        "SVC", "SYP", "SZL", "THB", "TJS", "TMT", "TND", "TOP", "TRY", "TTD", "TWD", "TZS", "UAH",
        "UGX", "USD", "USN", "UYI", "UYU", "UYW", "UZS", "VED", "VES", "VND", "VUV", "WST", "XAF",
        "XAG", "XAU", "XBA", "XBB", "XBC", "XBD", "XCD", "XCG", "XDR", "XOF", "XPD", "XPF", "XPT",
        "XSU", "XTS", "XUA", "XXX", "YER", "ZAR", "ZMW", "ZWG",
    ];

    /// The ISO 3166-1 alpha-2 country codes, sorted.
    ///
    /// Every currently assigned code, territories and dependencies included.
    /// The transitionally reserved codes (`AN`, `CS`, `YU`) and the
    /// user-assigned range (`AA`, `QM` through `QZ`, `XA` through `XZ`, `ZZ`)
    /// are not assigned, so a stream carrying one registers it.
    pub const COUNTRIES: &'static [&'static str] = &[
        "AD", "AE", "AF", "AG", "AI", "AL", "AM", "AO", "AQ", "AR", "AS", "AT", "AU", "AW", "AX",
        "AZ", "BA", "BB", "BD", "BE", "BF", "BG", "BH", "BI", "BJ", "BL", "BM", "BN", "BO", "BQ",
        "BR", "BS", "BT", "BV", "BW", "BY", "BZ", "CA", "CC", "CD", "CF", "CG", "CH", "CI", "CK",
        "CL", "CM", "CN", "CO", "CR", "CU", "CV", "CW", "CX", "CY", "CZ", "DE", "DJ", "DK", "DM",
        "DO", "DZ", "EC", "EE", "EG", "EH", "ER", "ES", "ET", "FI", "FJ", "FK", "FM", "FO", "FR",
        "GA", "GB", "GD", "GE", "GF", "GG", "GH", "GI", "GL", "GM", "GN", "GP", "GQ", "GR", "GS",
        "GT", "GU", "GW", "GY", "HK", "HM", "HN", "HR", "HT", "HU", "ID", "IE", "IL", "IM", "IN",
        "IO", "IQ", "IR", "IS", "IT", "JE", "JM", "JO", "JP", "KE", "KG", "KH", "KI", "KM", "KN",
        "KP", "KR", "KW", "KY", "KZ", "LA", "LB", "LC", "LI", "LK", "LR", "LS", "LT", "LU", "LV",
        "LY", "MA", "MC", "MD", "ME", "MF", "MG", "MH", "MK", "ML", "MM", "MN", "MO", "MP", "MQ",
        "MR", "MS", "MT", "MU", "MV", "MW", "MX", "MY", "MZ", "NA", "NC", "NE", "NF", "NG", "NI",
        "NL", "NO", "NP", "NR", "NU", "NZ", "OM", "PA", "PE", "PF", "PG", "PH", "PK", "PL", "PM",
        "PN", "PR", "PS", "PT", "PW", "PY", "QA", "RE", "RO", "RS", "RU", "RW", "SA", "SB", "SC",
        "SD", "SE", "SG", "SH", "SI", "SJ", "SK", "SL", "SM", "SN", "SO", "SR", "SS", "ST", "SV",
        "SX", "SY", "SZ", "TC", "TD", "TF", "TG", "TH", "TJ", "TK", "TL", "TM", "TN", "TO", "TR",
        "TT", "TV", "TW", "TZ", "UA", "UG", "UM", "US", "UY", "UZ", "VA", "VC", "VE", "VG", "VI",
        "VN", "VU", "WF", "WS", "YE", "YT", "ZA", "ZM", "ZW",
    ];

    /// The ISO 10383 market identifier codes of the common venues, sorted.
    ///
    /// The operating and segment MICs a multi-asset or commodity system
    /// actually meets - the listed exchanges, the derivatives and commodity
    /// venues, the large MTFs - plus `XOFF` for an off-exchange trade and
    /// `XXXX` for no market. It is deliberately not the whole registry, which
    /// is thousands of segment codes: a venue outside this set registers on
    /// first sight, at the cost of a code that is this dictionary's own.
    pub const MICS: &'static [&'static str] = &[
        "AQEU", "AQXE", "ARCX", "BATD", "BATE", "BATS", "BATY", "BCXE", "BMTF", "BVMF", "C2OX",
        "CCFX", "CEDX", "CEUX", "CHID", "CHIX", "DIFX", "DUMX", "EDGA", "EDGX", "EPEX", "GMNI",
        "IEXG", "IFAD", "IFEU", "IFLL", "IFSG", "IFUS", "MCRY", "MEMX", "MISX", "NDEX", "NEOE",
        "NORX", "OTCM", "RTSX", "SGMX", "TRQX", "XADS", "XAMS", "XASE", "XASX", "XATH", "XBER",
        "XBKK", "XBOM", "XBOS", "XBRU", "XBUD", "XCBF", "XCBO", "XCBT", "XCEC", "XCHI", "XCIS",
        "XCME", "XCSE", "XDCE", "XDFM", "XDUB", "XDUS", "XEEE", "XETR", "XEUR", "XFRA", "XHEL",
        "XHKG", "XICE", "XIDX", "XINE", "XIST", "XISX", "XJSE", "XKFE", "XKLS", "XKOS", "XKRX",
        "XLIS", "XLIT", "XLME", "XLON", "XMAD", "XMAT", "XMEX", "XMIL", "XMOD", "XMON", "XMUN",
        "XNAS", "XNGO", "XNSE", "XNYM", "XNYS", "XOFF", "XOSE", "XOSL", "XPAR", "XPHL", "XPRA",
        "XRIS", "XSAU", "XSES", "XSFE", "XSGE", "XSHE", "XSHG", "XSIM", "XSTO", "XSTU", "XSWX",
        "XTAE", "XTAI", "XTAL", "XTKS", "XTKT", "XTSE", "XTSX", "XVTX", "XWAR", "XWBO", "XXXX",
        "XZCE",
    ];

    /// Every side of the market a value may hold, sorted: the crate's own
    /// explicit spellings, one per side FIX's `Side(54)` code set names
    /// across every version, and `UNKNOWN` for a side stated as none.
    ///
    /// The stored value is the spelling and never FIX's one-character code:
    /// `BUY` rather than `1`, `SSHORT` rather than `5`, so a column reads
    /// without a dictionary beside it and a 4.2 message and a newest one
    /// agree about what a side is. A FIX code or the specification's name
    /// reaches the value through [`Side::from_spelling`](crate::types::Side::from_spelling),
    /// which is how a registry maps tag 54 onto it; per-member pedigree stays
    /// in the field's own `fix:codes` document, because that is where a
    /// version can be asked about. A spelling that names no side is refused
    /// rather than stored, exactly as a state is.
    pub const SIDES: &'static [&'static str] = &[
        "ASDEF", "BORROW", "BUY", "BUYMINUS", "CROSS", "CROSSSH", "CROSSSHX", "LEND", "OPPOSITE",
        "REDEEM", "SELL", "SELLPLUS", "SELLUND", "SSHORT", "SSHORTEX", "SUBSCR", "UNDISC",
        "UNKNOWN",
    ];

    /// Which way a captured line moved.
    ///
    /// Two members and no third. A row whose line does not say which way it
    /// moved has no direction, and the crate already spells "no answer" one
    /// way: a member meaning *unknown* would be a second spelling of null,
    /// two things to check at every read and the one a caller forgets.
    pub const DIRECTIONS: &'static [&'static str] = &["RECV", "SENT"];

    /// Every state one thing can be in, ordered from first to last.
    ///
    /// One vocabulary over two worlds. FIX names an order's state twice -
    /// `OrdStatus` says where the order stands and `ExecType` says what the
    /// report is - and a scheduler names a job's state in ordinary English.
    /// They are the same shape: a thing is created, it works, and it ends one
    /// of three ways. A capture and the pipeline that reads it should not need
    /// two vocabularies and a join to answer "what happened".
    ///
    /// # The first two bytes are the rank
    ///
    /// A value is two decimal digits of rank then a name of up to eight
    /// bytes, and the rank is what makes the *stored bytes* sort from first
    /// state to terminal. That matters because most things that sort a column
    /// are not this crate: a Parquet row group's min and max, an external
    /// sort, a `ORDER BY` in whatever reads the file. Ordering by name would
    /// put `CANCELED` before `NEW`; ordering by these bytes puts every live
    /// state before every ended one, and that ordering survives every format
    /// the value crosses.
    ///
    /// Ranks run `00`-`99`. Every shipped state sits on a round rank, and the
    /// digits between two of them - `01`-`09`, `11`-`19`, and so on - are the
    /// placeholders a state that belongs between two ranks takes, so adding
    /// one moves nothing already stored:
    ///
    /// | rank | meaning |
    /// | --- | --- |
    /// | `00` | stated, but not a state anything reached |
    /// | `10` | asked for, not yet acknowledged |
    /// | `20` | acknowledged, not yet working |
    /// | `30` | working |
    /// | `40` | working, and something has happened |
    /// | `50` | halted, and able to resume |
    /// | `60` | a change is outstanding |
    /// | `70` | changed, and the new thing carries on |
    /// | `80` | ended, having done what was asked |
    /// | `90` | ended, because someone stopped it |
    /// | `95` | ended, because it could not be done |
    ///
    /// The three endings are ranked apart deliberately: "did it finish" and
    /// "did it work" are different questions, and a single terminal rank would
    /// answer neither without reading the name. Each ending owns a band -
    /// `80`-`89` done, `90`-`94` cancelled, `95`-`99` failed - and
    /// [`State::is_done`](crate::types::State::is_done),
    /// [`State::is_cancelled`](crate::types::State::is_cancelled) and
    /// [`State::is_failed`](crate::types::State::is_failed) read the band, so
    /// a placeholder inside one answers as its ending does.
    pub const STATES: &'static [&'static str] = &[
        "00UNKNOWN",
        "10PENDING",
        "10PENDNEW",
        "10QUEUED",
        "20ACCEPTED",
        "20NEW",
        "20STARTING",
        "20SUBMITTD",
        "30RUNNING",
        "30STATUS",
        "30TRIGGER",
        "40INPROGR",
        "40PARTFILL",
        "40TRADE",
        "40TRDCORR",
        "40TRDCXL",
        "40TRDHOLD",
        "50PAUSED",
        "50STOPPED",
        "50SUSPEND",
        "60PENDCXL",
        "60PENDRPL",
        "70REPLACED",
        "70RESTATED",
        "80CALCULAT",
        "80COMPLETE",
        "80DONEDAY",
        "80FILLED",
        "80SUCCESS",
        "80TRDRELS",
        "90CANCELED",
        "95EXPIRED",
        "95FAILED",
        "95REJECTED",
        "95TIMEOUT",
    ];

    /// FIX's `TimeInForceCodeSet`, the union across every version, sorted.
    ///
    /// The wire values rather than the names, exactly as [`Self::SIDES`] is:
    /// a code set's value is what a message carries, and the name is what a
    /// dictionary translates it to.
    pub const TIMESINFORCE: &'static [&'static str] = &[
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "A", "B", "C", "D",
    ];

    /// The prebuilt vocabularies, by the logical name that spells them.
    ///
    /// `exchange` and `mic` name one list because they name one thing: FIX
    /// calls the ISO 10383 code an `Exchange`, and ISO calls it a MIC.
    pub const PREBUILT: &'static [(&'static str, &'static [&'static str])] = &[
        ("currency", Self::CURRENCIES),
        ("country", Self::COUNTRIES),
        ("mic", Self::MICS),
        ("exchange", Self::MICS),
        ("side", Self::SIDES),
        ("state", Self::STATES),
        ("timeinforce", Self::TIMESINFORCE),
    ];

    /// Creates the enum a registered logical name prebuilds.
    ///
    /// The enum is named for the registration and holds one member per value
    /// of its constant, each named by [`StringEnum::member_name`] - which, for
    /// an ISO code, is the code itself. A registered name with no constant -
    /// `language`, `monthyear`, `tenor` - answers an enum of no members,
    /// because a listing is what it has to offer and it has none.
    ///
    /// ```
    /// use yggdryl::{StringEnum, DataType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let venues = StringEnum::from_logical_name("mic")?;
    /// assert_eq!(venues.len(), StringEnum::MICS.len());
    /// assert_eq!(venues.get("XCME"), Some("XCME"));
    ///
    /// // A member's code is the value's own bytes under the resolved width.
    /// assert_eq!(
    ///     venues.into_members(&DataType::Mic)?[0].1,
    ///     DataType::Mic.ascii_packed(StringEnum::MICS[0].as_bytes())?
    /// );
    ///
    /// // `exchange` is FIX's name for the same list, under the same type.
    /// assert_eq!(
    ///     StringEnum::from_logical_name("Exchange")?.len(),
    ///     StringEnum::from_logical_name("mic")?.len()
    /// );
    ///
    /// // A name with no listing answers an enum of no members.
    /// assert!(StringEnum::from_logical_name("tenor")?.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the vocabulary when `name` is not a registered
    /// logical name.
    pub fn from_logical_name(name: &str) -> Result<Self> {
        // The name has to resolve, so a caller cannot prebuild a vocabulary
        // for a registration that does not exist.
        DataType::from_logical_name(name)?;
        Self::from_members(
            parser::normalized(name.trim()),
            Self::prebuilt_values(name)
                .iter()
                .map(|value| (Self::member_name(value), *value)),
        )
    }

    /// The constant a logical name prebuilds, empty when it has none.
    ///
    /// The name folds the way [`DataType::from_logical_name`] folds it, so one
    /// spelling reaches one list.
    pub fn prebuilt_values(name: &str) -> &'static [&'static str] {
        let folded = parser::normalized(name.trim());
        Self::PREBUILT
            .iter()
            .find(|(registered, _)| *registered == folded)
            .map_or(&[], |(_, values)| *values)
    }
}

/// The string value: the characters, held compactly, beside the layout and
/// charset its column stores them under.
///
/// A string value holds UTF-8 whatever charset it arrived in. That is the
/// whole point of decoding at the seam: the bytes are read once, at the
/// boundary that knows the charset, and everything above it reads characters.
/// What the value keeps is the layout and charset it is *written* under - and
/// the width, on the fixed layout - so the same value goes back out the way
/// it came without the column being consulted twice. A maximum is the
/// column's rule and never the value's: a value read out of `utf8(32)` is a
/// `utf8`, exactly as an integer read out of a bounded column is an integer.
///
/// [`Str`] is the one representation, and it is the crate's compact string:
/// up to [`INLINE_CAPACITY`] bytes live inside the value with no heap behind
/// them, a longer text is one shared `Arc<str>` that clones by reference
/// count, and a `&'static str` costs nothing at all. Equality, order and
/// hashing read the characters alone - a value is one value whichever column
/// holds it - and the parameters ride beside them.
mod scalars {
    use std::borrow::{Borrow, Cow};
    use std::fmt;
    use std::hash::{Hash, Hasher};
    use std::ops::Deref;
    use std::str::FromStr;
    use std::sync::Arc;

    use serde::{Deserialize, Serialize};
    use smol_str::{SmolStr, format_smolstr};

    use super::{StringLayout, StringType, trim_padding};
    use crate::types::Scalar;
    use crate::{Charset, DataType, Error, Result, Value};

    /// How many bytes of text a [`Str`] holds without reaching the heap.
    ///
    /// The storage is `smol_str`'s, which does not export the number, so it is
    /// pinned here and asserted in the tests beside it: a code, a currency pair,
    /// a ticker or an ISO date all fit under it and never allocate.
    pub const INLINE_CAPACITY: usize = 23;

    /// One string value: its characters and the parameters it is stored under.
    ///
    /// ```
    /// use yggdryl::types::{INLINE_CAPACITY, Str, StringLayout, StringType};
    /// use yggdryl::{Charset, DataType, Scalar};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// // Short text lives inside the value; longer text is one shared handle.
    /// let short = Str::new("AAPL");
    /// assert!(short.is_inline());
    /// let long = Str::new("a".repeat(INLINE_CAPACITY + 1));
    /// assert!(!long.is_inline());
    ///
    /// // A value is one value whichever layout or charset it is stored under.
    /// let latin = StringType::new(StringLayout::LargeString, Charset::Cp1252);
    /// let restated = short.clone().try_with_parameters(latin)?;
    /// assert_eq!(restated, short);
    /// assert_eq!(restated.charset(), Charset::Cp1252);
    /// assert_eq!(restated.dtype()?, DataType::from_str("large_string(windows-1252)")?);
    ///
    /// // It is the crate's string, so it is the `Scalar` string too.
    /// assert_eq!(Scalar::from("AAPL"), Scalar::String(short));
    /// # Ok(())
    /// # }
    /// ```
    #[derive(Clone)]
    pub struct Str {
        text: SmolStr,
        parameters: StringType,
    }

    const _: () = assert!(std::mem::size_of::<Str>() == 32);

    impl Str {
        /// Text the binary already holds, under the default parameters.
        ///
        /// Costs nothing: no copy, no count, so a constant spelling is a
        /// constant value.
        #[must_use]
        pub const fn new_static(text: &'static str) -> Self {
            Self {
                text: SmolStr::new_static(text),
                parameters: StringType::utf8(StringLayout::String),
            }
        }

        /// A string value under the default parameters: UTF-8, `string` layout.
        ///
        /// Text up to [`INLINE_CAPACITY`] bytes is copied into the value and
        /// allocates nothing; longer text is one shared `Arc<str>`.
        pub fn new(text: impl AsRef<str>) -> Self {
            Self {
                text: SmolStr::new(text),
                parameters: StringType::default(),
            }
        }

        /// Read the bytes a column stores, under the parameters it declares.
        ///
        /// This is the one door bytes take into a string value, and the charset
        /// decides how strict it is. UTF-8 and US-ASCII are validated
        /// repertoires - Arrow guarantees the first and the second rides Arrow's
        /// text storage - so bytes that are not what they claim are refused
        /// through [`Charset::decode`], and a US-ASCII value holds no NUL and no
        /// byte above `0x7F`. Every other charset is a declaration that the
        /// column holds legacy bytes, and those are read through
        /// [`Charset::transcribe`]: a byte the charset leaves unassigned reads as
        /// the scalar ISO 8859-1 gives it, because a legacy export with one bad
        /// byte in a million is a file that still has to be read.
        ///
        /// On the fixed layout the trailing NUL padding the storage writes is
        /// taken off first, and the value carries the width the parameters
        /// declare. A maximum is checked and not carried.
        ///
        /// # Errors
        ///
        /// Returns [`Error::Codec`] naming the charset and the first byte it
        /// refuses, [`Error::InvalidDataType`] for a fixed layout with no width,
        /// and [`Error::InvalidRecord`] when the text does not fit the bound.
        pub fn from_bytes(bytes: &[u8], parameters: StringType) -> Result<Self> {
            let payload = match parameters.is_fixed() {
                true => trim_padding(bytes),
                false => bytes,
            };
            let charset = parameters.charset();
            let text = match charset {
                Charset::Utf8 | Charset::Ascii => SmolStr::new(charset.decode(payload)?),
                _ => charset.transcribe_smol(payload),
            };
            Self {
                text,
                parameters: StringType::default(),
            }
            .try_with_parameters(parameters)
        }

        /// The text a column's own storage holds, under the parameters it
        /// declares, checked when it was written and not again here.
        pub(crate) fn from_storage(text: &str, parameters: StringType) -> Self {
            Self {
                text: SmolStr::new(text),
                parameters: parameters.without_max(),
            }
        }

        /// Restate the same characters under other parameters.
        ///
        /// The characters do not change and the storage is shared, not copied;
        /// what changes is the bytes [`Self::encode`] answers and the datatype
        /// [`Self::dtype`] declares. On the fixed layout trailing NUL is padding,
        /// so it is taken off here rather than left for every reader to trim.
        ///
        /// The bound is checked and, on a variable layout, not carried: it is the
        /// column's rule, and the value answers the layout alone. The bound counts
        /// stored bytes, which [`Charset::encoded_len`] answers without building
        /// them. US-ASCII is a repertoire and is judged here - a value holding a
        /// scalar above `0x7F` or a NUL is refused - but every other charset is
        /// only counted: whether its bytes can be written is the write seam's
        /// question, because a value read back through [`Charset::transcribe`]
        /// carries scalars the charset does not assign, and that is what
        /// recovering damage means.
        ///
        /// # Errors
        ///
        /// Returns [`Error::InvalidDataType`] for a fixed layout with no width,
        /// and [`Error::InvalidRecord`] naming the bound and the stored length
        /// when the text does not fit it, or the byte that is not US-ASCII.
        pub fn try_with_parameters(mut self, parameters: StringType) -> Result<Self> {
            parameters.validate()?;
            if parameters.is_fixed() {
                let trimmed = self.text.trim_end_matches('\0');
                if trimmed.len() != self.text.len() {
                    self.text = SmolStr::new(trimmed);
                }
            }
            let charset = parameters.charset();
            if charset == Charset::Ascii {
                ascii_repertoire(self.text.as_bytes())?;
            }
            if let Some(bound) = parameters.bound() {
                let stored = charset.encoded_len(&self.text);
                if stored > bound as usize {
                    return Err(Error::InvalidRecord {
                        path: SmolStr::new_static("$"),
                        reason: crate::text::expected_got(
                            format_args!("at most {bound} bytes of {charset}"),
                            format_smolstr!("{stored}"),
                        ),
                    });
                }
            }
            self.parameters = parameters.without_max();
            Ok(self)
        }

        /// Borrow the characters.
        #[must_use]
        pub fn as_str(&self) -> &str {
            self.text.as_str()
        }

        /// Borrow the shared storage without copying the text.
        ///
        /// The storage is the crate's ordinary compact string, so a name, a key
        /// or a code adopts a value's text by cloning this handle.
        #[must_use]
        pub const fn storage(&self) -> &SmolStr {
            &self.text
        }

        /// The parameters this value is stored under.
        ///
        /// Never a maximum: that is the column's declaration, and a value read
        /// out of a bounded column answers its layout and charset alone.
        #[must_use]
        pub const fn parameters(&self) -> StringType {
            self.parameters
        }

        /// The layout this value is stored in.
        #[must_use]
        pub const fn layout(&self) -> StringLayout {
            self.parameters.layout()
        }

        /// The charset this value's bytes are written in.
        #[must_use]
        pub const fn charset(&self) -> Charset {
            self.parameters.charset()
        }

        /// The padded storage width, on the fixed layout alone.
        #[must_use]
        pub const fn fixed(&self) -> Option<u32> {
            self.parameters.fixed()
        }

        /// Whether the characters live inside the value with no heap behind them.
        #[must_use]
        pub fn is_inline(&self) -> bool {
            !self.text.is_heap_allocated()
        }

        /// The stored bytes of this value, in the charset it declares.
        ///
        /// UTF-8 text borrows, and so does any all-ASCII value in any of the
        /// ASCII-compatible charsets, so the ordinary column costs nothing to
        /// write back out. The fixed layout answers its whole padded slot.
        ///
        /// # Errors
        ///
        /// Returns [`Error::Codec`] naming the first scalar the charset has no
        /// byte for.
        pub fn encode(&self) -> Result<Cow<'_, [u8]>> {
            let encoded = self.charset().encode(self.as_str())?;
            Ok(match self.fixed() {
                Some(width) => {
                    let mut padded = encoded.into_owned();
                    padded.resize(width as usize, 0);
                    Cow::Owned(padded)
                }
                None => encoded,
            })
        }

        /// How many bytes [`Self::encode`] answers, without building them.
        #[must_use]
        pub fn encoded_len(&self) -> usize {
            match self.fixed() {
                Some(width) => width as usize,
                None => self.charset().encoded_len(self.as_str()),
            }
        }

        /// Consume this value and return its characters as an owned `String`.
        #[must_use]
        pub fn into_string(self) -> String {
            self.text.to_string()
        }

        /// Consume this value and return its compact storage.
        #[must_use]
        pub fn into_inner(self) -> SmolStr {
            self.text
        }

        /// The datatype this value materializes into.
        ///
        /// # Errors
        ///
        /// Returns [`Error::InvalidDataType`] only for parameters a constructor
        /// would have refused, which no door here builds.
        pub fn dtype(&self) -> Result<DataType> {
            DataType::string(self.parameters)
        }
    }

    /// Refuse what US-ASCII text never holds: a NUL, or a byte above `0x7F`.
    ///
    /// The byte class is one fact and [`Charset`] owns it: this is that
    /// charset's own scan, which reads a machine word at a time.
    fn ascii_repertoire(bytes: &[u8]) -> Result<()> {
        let refusal = |actual: SmolStr| Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: crate::text::expected_got(format_args!("US-ASCII text"), actual),
        };
        if let Some(position) = bytes.iter().position(|byte| *byte == 0) {
            return Err(refusal(format_smolstr!("a NUL byte at {position}")));
        }
        let position = crate::charset::ascii_len(bytes);
        if position < bytes.len() {
            return Err(refusal(format_smolstr!(
                "a non-ASCII byte 0x{:02X} at {position}",
                bytes[position]
            )));
        }
        Ok(())
    }

    impl Default for Str {
        fn default() -> Self {
            Self::new_static("")
        }
    }

    impl Deref for Str {
        type Target = str;

        fn deref(&self) -> &str {
            self.as_str()
        }
    }

    impl AsRef<str> for Str {
        fn as_ref(&self) -> &str {
            self.as_str()
        }
    }

    impl AsRef<[u8]> for Str {
        fn as_ref(&self) -> &[u8] {
            self.as_str().as_bytes()
        }
    }

    /// Sound because [`Eq`], [`Ord`] and [`Hash`] read the characters alone, as
    /// `str`'s do; folding the parameters into any of them would break every
    /// `BTreeMap<Str, _>::get(&str)` in the crate.
    impl Borrow<str> for Str {
        fn borrow(&self) -> &str {
            self.as_str()
        }
    }

    impl fmt::Display for Str {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.as_str())
        }
    }

    impl fmt::Debug for Str {
        /// The characters, and the parameters when they are not the default, so
        /// two values that compare equal but declare different columns print
        /// apart in a failing assertion.
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt::Debug::fmt(self.as_str(), formatter)?;
            if self.parameters != StringType::default() {
                write!(formatter, " as {}", self.parameters)?;
            }
            Ok(())
        }
    }

    impl PartialEq for Str {
        fn eq(&self, other: &Self) -> bool {
            self.as_str() == other.as_str()
        }
    }

    impl Eq for Str {}

    impl PartialEq<str> for Str {
        fn eq(&self, other: &str) -> bool {
            self.as_str() == other
        }
    }

    impl PartialEq<&str> for Str {
        fn eq(&self, other: &&str) -> bool {
            self.as_str() == *other
        }
    }

    impl PartialEq<Str> for str {
        fn eq(&self, other: &Str) -> bool {
            self == other.as_str()
        }
    }

    impl PartialEq<Str> for &str {
        fn eq(&self, other: &Str) -> bool {
            *self == other.as_str()
        }
    }

    impl PartialEq<String> for Str {
        fn eq(&self, other: &String) -> bool {
            self.as_str() == other.as_str()
        }
    }

    impl PartialEq<Str> for String {
        fn eq(&self, other: &Str) -> bool {
            self.as_str() == other.as_str()
        }
    }

    impl PartialEq<SmolStr> for Str {
        fn eq(&self, other: &SmolStr) -> bool {
            self.as_str() == other.as_str()
        }
    }

    impl PartialEq<Str> for SmolStr {
        fn eq(&self, other: &Str) -> bool {
            self.as_str() == other.as_str()
        }
    }

    impl PartialOrd for Str {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for Str {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            self.as_str().cmp(other.as_str())
        }
    }

    impl Hash for Str {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.as_str().hash(state);
        }
    }

    impl From<&str> for Str {
        fn from(value: &str) -> Self {
            Self::new(value)
        }
    }

    impl From<&mut str> for Str {
        fn from(value: &mut str) -> Self {
            Self::new(value)
        }
    }

    impl From<&String> for Str {
        fn from(value: &String) -> Self {
            Self::new(value)
        }
    }

    impl From<String> for Str {
        fn from(value: String) -> Self {
            Self::new(value)
        }
    }

    impl From<Box<str>> for Str {
        fn from(value: Box<str>) -> Self {
            Self::new(value)
        }
    }

    impl From<Arc<str>> for Str {
        fn from(value: Arc<str>) -> Self {
            Self::new(value)
        }
    }

    impl From<Cow<'_, str>> for Str {
        fn from(value: Cow<'_, str>) -> Self {
            Self::new(value)
        }
    }

    impl From<char> for Str {
        fn from(value: char) -> Self {
            let mut encoded = [0_u8; 4];
            Self::new(value.encode_utf8(&mut encoded))
        }
    }

    impl From<SmolStr> for Str {
        fn from(value: SmolStr) -> Self {
            Self {
                text: value,
                parameters: StringType::default(),
            }
        }
    }

    impl From<&SmolStr> for Str {
        fn from(value: &SmolStr) -> Self {
            Self::from(value.clone())
        }
    }

    impl From<Str> for String {
        fn from(value: Str) -> Self {
            value.into_string()
        }
    }

    impl From<&Str> for String {
        fn from(value: &Str) -> Self {
            value.as_str().to_owned()
        }
    }

    impl From<Str> for SmolStr {
        fn from(value: Str) -> Self {
            value.text
        }
    }

    impl From<&Str> for SmolStr {
        fn from(value: &Str) -> Self {
            value.text.clone()
        }
    }

    impl From<Str> for Arc<str> {
        fn from(value: Str) -> Self {
            Self::from(value.as_str())
        }
    }

    impl FromStr for Str {
        type Err = std::convert::Infallible;

        fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
            Ok(Self::new(value))
        }
    }

    impl FromIterator<char> for Str {
        fn from_iter<I: IntoIterator<Item = char>>(iter: I) -> Self {
            Self::from(iter.into_iter().collect::<SmolStr>())
        }
    }

    impl<'a> FromIterator<&'a str> for Str {
        fn from_iter<I: IntoIterator<Item = &'a str>>(iter: I) -> Self {
            Self::from(iter.into_iter().collect::<SmolStr>())
        }
    }

    /// The serde representation of a string that declares more than its text.
    ///
    /// The ordinary value - UTF-8, the `string` layout - serializes its
    /// characters and nothing else, exactly as it always has; a layout, a
    /// charset or a fixed width is what makes a value carry more than that.
    #[derive(Deserialize, Serialize)]
    struct Declared<'a> {
        layout: StringLayout,
        charset: Charset,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fixed: Option<u32>,
        text: Cow<'a, str>,
    }

    impl Serialize for Str {
        fn serialize<S: serde::Serializer>(
            &self,
            serializer: S,
        ) -> std::result::Result<S::Ok, S::Error> {
            if self.parameters == StringType::default() {
                return serializer.serialize_str(self.as_str());
            }
            Declared {
                layout: self.layout(),
                charset: self.charset(),
                fixed: self.fixed(),
                text: Cow::Borrowed(self.as_str()),
            }
            .serialize(serializer)
        }
    }

    impl<'de> Deserialize<'de> for Str {
        fn deserialize<D: serde::Deserializer<'de>>(
            deserializer: D,
        ) -> std::result::Result<Self, D::Error> {
            #[derive(Deserialize)]
            #[serde(untagged)]
            enum Representation<'a> {
                Plain(Cow<'a, str>),
                Declared(Declared<'a>),
            }

            match Representation::deserialize(deserializer)? {
                Representation::Plain(text) => Ok(Self::from(text)),
                Representation::Declared(declared) => {
                    let mut parameters = StringType::new(declared.layout, declared.charset);
                    if let Some(width) = declared.fixed {
                        parameters = parameters
                            .try_with_bound(width)
                            .map_err(serde::de::Error::custom)?;
                    }
                    Self::from(declared.text)
                        .try_with_parameters(parameters)
                        .map_err(serde::de::Error::custom)
                }
            }
        }
    }

    impl Value for Str {
        fn dtype(&self) -> Result<DataType> {
            Self::dtype(self)
        }

        fn into_scalar(self) -> Scalar {
            Scalar::String(self)
        }

        fn from_scalar(value: &Scalar) -> Option<&Self> {
            match value {
                Scalar::String(value) => Some(value),
                _ => None,
            }
        }
    }

    impl From<Str> for Scalar {
        fn from(value: Str) -> Self {
            Self::String(value)
        }
    }

    impl From<&str> for Scalar {
        fn from(value: &str) -> Self {
            Self::String(Str::new(value))
        }
    }

    impl From<String> for Scalar {
        fn from(value: String) -> Self {
            Self::String(Str::new(value))
        }
    }

    impl From<SmolStr> for Scalar {
        fn from(value: SmolStr) -> Self {
            Self::String(Str::from(value))
        }
    }

    /// The canonical text a value spells, shared rather than rebuilt.
    ///
    /// A string column stores one string per row, and this is the spelling every
    /// tier prints: the canonical [`std::fmt::Display`] each width owns for the
    /// numbers and the boolean, [`Scalar::into_temporal_text`] for a temporal -
    /// which is what a temporal *column* renders too, zone rules included - the
    /// WKT a geometry column renders, and the payload's own characters for bytes.
    /// A string, a code and a generic enum member already hold their text and
    /// hand it over without allocating.
    ///
    /// `None` is a kind that spells no text at all; `Some(Err)` is a payload that
    /// was read and refused, so the refusal names what was wrong with it rather
    /// than which kind arrived.
    pub(crate) fn str_from_value(value: &Scalar) -> Option<Result<Str>> {
        Some(match value {
            Scalar::String(text) => Ok(text.clone()),
            code if code.is_code() => Ok(Str::from(
                code.code_storage().expect("a code borrowed its storage"),
            )),
            // A number of any width spells its own leaf's canonical `Display`.
            number if number.is_number() => {
                Ok(Str::from(format_smolstr!("{}", number.leaf_display()?)))
            }
            Scalar::Boolean(flag) => Ok(Str::from(format_smolstr!("{flag}"))),
            // The renderer answers the compact string directly: going through
            // `Display` would write the same 36 bytes into a second buffer.
            Scalar::Uuid(uuid) => Ok(Str::from(crate::types::uuid_text(&uuid.into_bytes()))),
            Scalar::Version(version) => Ok(Str::from(format_smolstr!("{version}"))),
            Scalar::Url(url) => Ok(Str::from(format_smolstr!("{url}"))),
            // An interval has no classic spelling, so a temporal answers for the
            // seven that do and leaves the rest to the ordinary refusal.
            temporal if temporal.is_temporal() => {
                return temporal
                    .into_temporal_text()
                    .map(|text| Ok(Str::from(text)));
            }
            Scalar::Bytes(bytes) => {
                std::str::from_utf8(bytes.as_bytes())
                    .map(Str::new)
                    .map_err(|error| Error::InvalidRecord {
                        path: SmolStr::new_static("$"),
                        reason: format_smolstr!("payload is not UTF-8: {error}"),
                    })
            }
            Scalar::Geometry(value) => {
                crate::types::wkb::into_wkt(value.as_bytes()).map(Str::from)
            }
            Scalar::Geography(value) => {
                crate::types::wkb::into_wkt(value.as_bytes()).map(Str::from)
            }
            _ => return None,
        })
    }

    /// The inline threshold an integration test cannot reach.
    ///
    /// `INLINE_CAPACITY` is crate-private: it is the byte count below which a
    /// `Str` stores its text in the value rather than behind an `Arc`, so the
    /// boundary has to be crossed from inside. Everything a caller can observe
    /// lives in `tests/types/strings.rs`.
    #[cfg(test)]
    mod tests {
        use super::{INLINE_CAPACITY, Str};
        use crate::types::{StringLayout, StringType};
        use crate::{Charset, DataType, Scalar};

        #[test]
        fn short_text_is_inline_and_long_text_is_shared() {
            let short = Str::new("a".repeat(INLINE_CAPACITY));
            assert!(short.is_inline());
            assert_eq!(short.len(), INLINE_CAPACITY);
            let long = Str::new("a".repeat(INLINE_CAPACITY + 1));
            assert!(!long.is_inline());
            assert_eq!(long.len(), INLINE_CAPACITY + 1);
            assert!(Str::new_static("held").is_inline());
            assert_eq!(Str::default(), "");
            assert_eq!(std::mem::size_of::<Str>(), 32);
        }

        #[test]
        fn equality_order_and_hash_read_the_characters_only() {
            use std::collections::HashSet;

            let plain = Str::new("Grüße");
            let latin = Str::new("Grüße")
                .try_with_parameters(StringType::new(
                    StringLayout::LargeString,
                    Charset::Cp1252,
                ))
                .unwrap();
            assert_eq!(plain, latin);
            assert_eq!(plain.cmp(&latin), std::cmp::Ordering::Equal);
            assert_eq!(HashSet::from([plain.clone(), latin.clone()]).len(), 1);
            assert_ne!(plain.parameters(), latin.parameters());
            assert!(Str::new("b") > Str::new("a"));
            assert_eq!(
                format!("{latin:?}"),
                "\"Grüße\" as large_string(windows-1252)"
            );
            assert_eq!(format!("{plain:?}"), "\"Grüße\"");
        }

        #[test]
        fn restating_shares_the_storage_and_checks_but_never_carries_a_maximum() {
            let text = "x".repeat(INLINE_CAPACITY + 22);
            let shared = Str::new(&text);
            let bounded = StringType::utf8(StringLayout::String)
                .try_with_bound(64)
                .unwrap();
            let restated = shared.clone().try_with_parameters(bounded).unwrap();
            assert!(std::ptr::eq(shared.as_str(), restated.as_str()));
            assert_eq!(restated.parameters(), StringType::default());
            assert_eq!(restated.dtype().unwrap(), DataType::utf8());

            let tight = StringType::utf8(StringLayout::String)
                .try_with_bound(8)
                .unwrap();
            let refused = shared.try_with_parameters(tight).unwrap_err().to_string();
            assert!(refused.contains("at most 8 bytes"), "{refused}");

            // The bound counts stored bytes, so five scalars are five bytes in
            // windows-1252 and seven in UTF-8.
            let five = StringType::new(StringLayout::String, Charset::Cp1252)
                .try_with_bound(5)
                .unwrap();
            assert!(Str::new("Grüße").try_with_parameters(five).is_ok());
            let five = StringType::utf8(StringLayout::String)
                .try_with_bound(5)
                .unwrap();
            assert!(Str::new("Grüße").try_with_parameters(five).is_err());
        }

        #[test]
        fn us_ascii_is_a_repertoire_and_every_other_charset_is_counted() {
            let ascii = StringType::ascii(StringLayout::String);
            assert!(Str::new("plain").try_with_parameters(ascii).is_ok());
            let refused = Str::new("café")
                .try_with_parameters(ascii)
                .unwrap_err()
                .to_string();
            assert!(refused.contains("non-ASCII byte"), "{refused}");
            assert!(Str::new("a\0b").try_with_parameters(ascii).is_err());
            // `U+0081` has no windows-1252 byte, and the value door only counts.
            let latin = StringType::new(StringLayout::String, Charset::Cp1252);
            let recovered = Str::new("ok\u{0081}").try_with_parameters(latin).unwrap();
            assert!(recovered.encode().is_err());
            assert_eq!(recovered.encoded_len(), 3);
        }

        #[test]
        fn a_fixed_layout_trims_its_padding_and_pads_on_the_way_out() {
            let fixed = StringType::ascii(StringLayout::FixedString)
                .try_with_bound(4)
                .unwrap();
            let value = Str::new("USD\0").try_with_parameters(fixed).unwrap();
            assert_eq!(value, "USD");
            assert_eq!(value.fixed(), Some(4));
            assert_eq!(value.parameters(), fixed);
            assert_eq!(value.encode().unwrap().as_ref(), b"USD\0");
            assert_eq!(value.encoded_len(), 4);
            assert!(Str::new("EURO!").try_with_parameters(fixed).is_err());
            assert!(
                Str::new("x")
                    .try_with_parameters(StringType::utf8(StringLayout::FixedString))
                    .is_err()
            );
        }

        #[test]
        fn bytes_are_read_strictly_or_transcribed_by_their_charset() {
            let latin = StringType::new(StringLayout::String, Charset::Cp1252);
            let value = Str::from_bytes(b"Gr\xFC\xDFe", latin).unwrap();
            assert_eq!(value, "Grüße");
            assert_eq!(value.charset(), Charset::Cp1252);
            assert_eq!(value.encode().unwrap().as_ref(), b"Gr\xFC\xDFe");
            // `0x81` is unassigned in windows-1252 and still reads.
            assert_eq!(Str::from_bytes(b"ok\x81", latin).unwrap(), "ok\u{0081}");
            // UTF-8 and US-ASCII are validated, not transcribed.
            assert!(Str::from_bytes(b"caf\xe9", StringType::default()).is_err());
            assert!(
                Str::from_bytes(
                    b"caf\xc3\xa9",
                    StringType::ascii(StringLayout::String)
                )
                .is_err()
            );
            assert_eq!(
                Str::from_bytes(b"caf\xc3\xa9", StringType::default()).unwrap(),
                "café"
            );
            // A padded slot comes back trimmed and carries its width.
            let slot = StringType::utf8(StringLayout::FixedString)
                .try_with_bound(6)
                .unwrap();
            let padded = Str::from_bytes(b"ab\0\0\0\0", slot).unwrap();
            assert_eq!(padded, "ab");
            assert_eq!(padded.fixed(), Some(6));
        }

        #[test]
        fn serde_writes_the_text_alone_unless_the_value_declares_more() {
            let plain = Str::new("plain");
            assert_eq!(serde_json::to_string(&plain).unwrap(), "\"plain\"");
            assert_eq!(serde_json::from_str::<Str>("\"plain\"").unwrap(), plain);
            let latin = Str::new("Grüße")
                .try_with_parameters(
                    StringType::new(StringLayout::FixedString, Charset::Cp1252)
                        .try_with_bound(8)
                        .unwrap(),
                )
                .unwrap();
            let document = serde_json::to_string(&latin).unwrap();
            assert_eq!(
                document,
                r#"{"layout":"fixed_string","charset":"windows-1252","fixed":8,"text":"Grüße"}"#
            );
            let back = serde_json::from_str::<Str>(&document).unwrap();
            assert_eq!(back.parameters(), latin.parameters());
            assert_eq!(back, latin);
            // A maximum is never part of a value, so it never reaches the wire.
            let bounded = Str::new("x")
                .try_with_parameters(
                    StringType::utf8(StringLayout::LargeString)
                        .try_with_bound(8)
                        .unwrap(),
                )
                .unwrap();
            assert_eq!(
                serde_json::to_string(&bounded).unwrap(),
                r#"{"layout":"large_string","charset":"utf-8","text":"x"}"#
            );
        }

        #[test]
        fn a_string_scalar_names_its_own_datatype() {
            assert_eq!(Scalar::from("x").as_str(), Some("x"));
            assert_eq!(Scalar::from("x").dtype().unwrap(), DataType::utf8());
            let latin = Str::new("x")
                .try_with_parameters(StringType::new(
                    StringLayout::StringView,
                    Charset::Latin1,
                ))
                .unwrap();
            assert_eq!(
                Scalar::String(latin).dtype().unwrap(),
                DataType::from_str("string_view(iso-8859-1)").unwrap()
            );
        }
    }
}

/// One of the five ways this crate lays a string out.
///
/// The layout is the physical shape alone, how a value's bytes are addressed.
/// It says nothing about what the bytes mean; that is the charset beside it in
/// [`StringType`].
///
/// Each layout has three spellings, and they are one datatype: the `string`
/// name is the general one, the `utf8` name is what the same layout is called
/// when its charset is UTF-8, which is the default, and the `ascii` name is
/// what it is called when its charset is US-ASCII. So `large_string`,
/// `large_utf8` and `large_ascii` name one layout, and a value renders under
/// whichever name its charset earns.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum StringLayout {
    /// Variable width, 32-bit offsets - Arrow's `Utf8` and `Binary`.
    #[default]
    String,
    /// One fixed byte width every value fills, padded with trailing NUL.
    ///
    /// Arrow has no fixed-width string at all, so this rides its
    /// `FixedSizeBinary` and the width travels in this crate's own metadata.
    FixedString,
    /// The view layout: a short prefix inline, the rest out of line.
    StringView,
    /// Variable width, 64-bit offsets - Arrow's `LargeUtf8` and `LargeBinary`.
    LargeString,
    /// The view layout, declared large.
    ///
    /// Arrow has one view layout and no large form of it, so this projects as
    /// that one view and the `large` declaration travels in this crate's own
    /// metadata. Nothing about the buffers differs; what differs is what the
    /// column promises about the offsets a writer may emit.
    LargeStringView,
}

/// Strip the padding a fixed-width storage writes.
///
/// Trailing NUL is that padding wherever this crate lays a value out in a
/// slot wider than itself - a [`StringLayout::FixedString`] cell - so the
/// rule lives here once rather than at each reader. A code stores as the
/// text it is and pads nothing, but a code's *intake* is wide: a cell
/// arriving from a fixed-width column is trimmed here on the way in.
pub(crate) fn trim_padding(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .rposition(|byte| *byte != 0)
        .map_or(0, |last| last + 1);
    &bytes[..end]
}

impl StringLayout {
    /// Every layout in canonical declaration order.
    pub const ALL: [Self; 5] = [
        Self::String,
        Self::FixedString,
        Self::StringView,
        Self::LargeString,
        Self::LargeStringView,
    ];

    /// The canonical name of this layout in any charset.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::FixedString => "fixed_string",
            Self::StringView => "string_view",
            Self::LargeString => "large_string",
            Self::LargeStringView => "large_string_view",
        }
    }

    /// The canonical name of this layout when its charset is UTF-8.
    #[must_use]
    pub const fn as_utf8_str(self) -> &'static str {
        match self {
            Self::String => "utf8",
            Self::FixedString => "fixed_utf8",
            Self::StringView => "utf8_view",
            Self::LargeString => "large_utf8",
            Self::LargeStringView => "large_utf8_view",
        }
    }

    /// The canonical name of this layout when its charset is US-ASCII.
    #[must_use]
    pub const fn as_ascii_str(self) -> &'static str {
        match self {
            Self::String => "ascii",
            Self::FixedString => "fixed_ascii",
            Self::StringView => "ascii_view",
            Self::LargeString => "large_ascii",
            Self::LargeStringView => "large_ascii_view",
        }
    }

    /// Resolve a layout from its general spelling or its UTF-8 one.
    ///
    /// Case, underscores, hyphens and spaces are all ignored, so
    /// `LARGE_STRING`, `large-string` and `largestring` are one layout, and
    /// so are `large_utf8` and `largeutf8` - UTF-8 is the charset a layout
    /// has when nothing is declared, so that spelling names no more than the
    /// layout. The US-ASCII spellings do name more, and a layout alone would
    /// drop it, so they are refused here and read only where the charset
    /// travels with them: [`crate::DataType::from_str`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownDataType`] for a name no layout answers to,
    /// and for a US-ASCII spelling, naming the charset it would lose.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        if let Some(layout) = Self::ALL
            .into_iter()
            .find(|layout| super::folds_equal(value, layout.as_ascii_str()))
        {
            return Err(Error::UnknownDataType(format_smolstr!(
                "{value} names the {} layout in the us-ascii charset, not a layout alone",
                layout.as_str()
            )));
        }
        Self::ALL
            .into_iter()
            .find(|layout| {
                super::folds_equal(value, layout.as_str())
                    || super::folds_equal(value, layout.as_utf8_str())
            })
            .ok_or_else(|| Error::UnknownDataType(format_smolstr!("{value}")))
    }

    /// The identifier naming this layout.
    #[must_use]
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::String => DataTypeId::String,
            Self::FixedString => DataTypeId::FixedString,
            Self::StringView => DataTypeId::StringView,
            Self::LargeString => DataTypeId::LargeString,
            Self::LargeStringView => DataTypeId::LargeStringView,
        }
    }

    /// The layout one identifier names, `None` for an identifier that is not
    /// a string's.
    #[must_use]
    pub const fn from_id(id: DataTypeId) -> Option<Self> {
        match id {
            DataTypeId::String => Some(Self::String),
            DataTypeId::FixedString => Some(Self::FixedString),
            DataTypeId::StringView => Some(Self::StringView),
            DataTypeId::LargeString => Some(Self::LargeString),
            DataTypeId::LargeStringView => Some(Self::LargeStringView),
            _ => None,
        }
    }

    /// Return whether every value in this layout is the same width.
    #[must_use]
    pub const fn is_fixed(self) -> bool {
        matches!(self, Self::FixedString)
    }

    /// Return whether this layout addresses its bytes through a view.
    #[must_use]
    pub const fn is_view(self) -> bool {
        matches!(self, Self::StringView | Self::LargeStringView)
    }

    /// Return whether this layout declares 64-bit offsets.
    #[must_use]
    pub const fn is_large(self) -> bool {
        matches!(self, Self::LargeString | Self::LargeStringView)
    }
}

impl fmt::Display for StringLayout {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for StringLayout {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for StringLayout {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = <std::borrow::Cow<'_, str>>::deserialize(deserializer)?;
        Self::from_str(&value).map_err(serde::de::Error::custom)
    }
}

/// The code readers an integration test cannot reach.
///
/// `code_text`, `code_cell_text` and `code_for_extension` are crate-private:
/// they are the doors every registered code goes through, and a caller sees
/// only the datatype they answer for. The rest of the suite lives in
/// `tests/types/datatype/coded.rs`.
#[cfg(test)]
mod tests {
    use crate::types::code::{code_cell_text, code_for_extension, code_text};
    use crate::DataType;

    #[test]
    fn every_code_names_itself_and_its_width() {
        for (name, dtype, width) in DataType::CODES {
            assert_eq!(dtype.code_name(), Some(*name));
            assert_eq!(dtype.code_width(), Some(*width));
            assert_eq!(dtype.id().code_width(), Some(*width));
            // The width is a maximum, so no code claims a fixed layout.
            assert_eq!(dtype.fixed_byte_width(), None);
            assert_eq!(dtype.to_string(), *name);
            assert_eq!(DataType::from_str(name).unwrap(), *dtype);
            assert!(dtype.is_code());
            assert!(!dtype.is_string());
        }
        assert!(!DataType::fixed_ascii(3).unwrap().is_code());
        assert_eq!(DataType::fixed_ascii(3).unwrap().code_width(), None);
    }

    #[test]
    fn only_a_registered_name_is_a_code() {
        assert_eq!(
            code_for_extension("yggdryl.currency"),
            Some(DataType::Currency)
        );
        assert_eq!(code_for_extension("yggdryl.cfi"), Some(DataType::Cfi));
        assert_eq!(code_for_extension("yggdryl.cusip"), Some(DataType::Cusip));
        assert_eq!(code_for_extension("yggdryl.sedol"), Some(DataType::Sedol));
        assert_eq!(code_for_extension("yggdryl.ascii"), None);
        assert_eq!(code_for_extension("arrow.uuid"), None);
    }

    #[test]
    fn a_code_packs_at_the_width_its_standard_fixes() {
        // The packing pads; the column does not. Both codes and fixed ASCII
        // widths answer, and nothing else does.
        assert_eq!(
            DataType::Currency.ascii_packed(b"USD").unwrap(),
            0x0055_5344
        );
        assert_eq!(DataType::Currency.ascii_value(0x0055_5344).unwrap(), "USD");
        assert_eq!(DataType::Country.ascii_packed(b"FR").unwrap(), 0x4652);
        assert_eq!(
            DataType::fixed_ascii(4)
                .unwrap()
                .ascii_packed(b"USD")
                .unwrap(),
            0x5553_4400
        );
        assert!(DataType::Currency.ascii_packed(b"EURO").is_err());
        assert!(DataType::utf8().ascii_packed(b"USD").is_err());
    }

    #[test]
    fn a_code_holds_ascii_text_up_to_its_width() {
        assert_eq!(code_text::<3>(b"USD").unwrap(), "USD");
        assert_eq!(code_text::<3>(b"US\0").unwrap(), "US");
        assert_eq!(code_text::<6>(b"ESVUFR").unwrap(), "ESVUFR");
        let refused = code_text::<3>(b"EURO").unwrap_err().to_string();
        assert!(refused.contains("at most 3 bytes"), "{refused}");
    }

    #[test]
    fn a_cell_is_validated_at_the_code_width() {
        assert_eq!(code_cell_text(&DataType::Currency, b"USD").unwrap(), "USD");
        assert_eq!(code_cell_text(&DataType::Country, b"FR").unwrap(), "FR");
        assert_eq!(code_cell_text(&DataType::Cfi, b"ESVUFR").unwrap(), "ESVUFR");
        let refused = code_cell_text(&DataType::Country, b"USD")
            .unwrap_err()
            .to_string();
        assert!(refused.contains("at most 2 bytes"), "{refused}");
        let wrong = code_cell_text(&DataType::fixed_ascii(3).unwrap(), b"USD")
            .unwrap_err()
            .to_string();
        assert!(wrong.contains("registered codes"), "{wrong}");
    }
}

impl crate::types::DataTypeValue for StringType {
    const FAMILY: &'static str = "string";

    type Sidecar = ();

    fn id(&self) -> crate::DataTypeId {
        DataType::String(*self).id()
    }

    fn kind(&self) -> crate::DataTypeKind {
        crate::DataTypeKind::Text
    }

    fn validate(&self) -> Result<()> {
        DataType::String(*self).validate()
    }

    fn into_dtype(self) -> DataType {
        DataType::String(self)
    }

    fn from_dtype(dtype: &DataType) -> Option<Self> {
        match dtype {
            DataType::String(parameters) => Some(*parameters),
            _ => None,
        }
    }
}
