//! Null and Boolean datatypes.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Scalar;
use crate::typed::define_field_types;
use crate::{DataType, Result, Value};

// ------------------------------------------------------------------------
// Parameterless Null and Boolean datatype variants need no constructors.
// ------------------------------------------------------------------------

// ------------------------------------------------------------------------
// Null and Boolean datatypes: neither carries a parameter.
// ------------------------------------------------------------------------

define_field_types!(NullType, Null);

define_field_types!(BooleanType, Boolean);

// ------------------------------------------------------------------------
// Null and Boolean values and typed scalar aliases.
// ------------------------------------------------------------------------

/// The one null value.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
pub struct Null;

impl fmt::Display for Null {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("null")
    }
}

/// One Boolean value.
#[repr(transparent)]
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct Boolean(bool);

impl Boolean {
    /// Construct a Boolean value.
    pub const fn new(value: bool) -> Self {
        Self(value)
    }

    /// Return the native Boolean.
    pub const fn get(self) -> bool {
        self.0
    }
}

impl fmt::Display for Boolean {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl From<bool> for Boolean {
    fn from(value: bool) -> Self {
        Self::new(value)
    }
}

impl From<Boolean> for bool {
    fn from(value: Boolean) -> Self {
        value.get()
    }
}

impl From<()> for Scalar {
    fn from((): ()) -> Self {
        Self::Null
    }
}

impl From<bool> for Scalar {
    fn from(value: bool) -> Self {
        Self::Boolean(Boolean::new(value))
    }
}

impl Value for Null {
    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::Null)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Null
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        static NULL: Null = Null;
        value.is_null().then_some(&NULL)
    }
}

impl Value for Boolean {
    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::Boolean)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Boolean(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Boolean(value) => Some(value),
            _ => None,
        }
    }
}

/// The spellings a boolean is read from: Arrow's string-to-boolean cast's,
/// so a cell and a column read one text alike.
const TRUE_SPELLINGS: [&str; 9] = ["true", "t", "tr", "tru", "yes", "y", "ye", "on", "1"];
const FALSE_SPELLINGS: [&str; 10] = [
    "false", "f", "fa", "fal", "fals", "no", "n", "off", "of", "0",
];

/// What every refusal of a boolean spelling names: the table in one phrase,
/// the prefixes left to the table.
pub(crate) const BOOLEAN_SPELLINGS: &str = "true/false, yes/no, y/n, on/off or 1/0";

/// Read a boolean out of text: the one table every flag in the crate reads.
///
/// The spellings are Arrow's string-to-boolean cast's - `true`, `yes`, `y`,
/// `on`, `1` and the prefixes of `true` and `yes`; `false`, `no`, `n`,
/// `off`, `0` and the prefixes of `false` and `off` - ASCII case-insensitive
/// and trimmed, so FIX's `Y` and `N`, a bridge's `no`, an environment's `on`
/// and a property's `0` are readings rather than refusals, and a row, a
/// column, a setting and a flag read one text alike. Nothing else spells a
/// boolean: text outside the table is `None`, and the caller names what it
/// expected with [`BOOLEAN_SPELLINGS`]. Allocates nothing.
pub(crate) fn bool_from_text(text: &str) -> Option<bool> {
    let text = text.trim();
    let spells = |spellings: &[&str]| spellings.iter().any(|held| text.eq_ignore_ascii_case(held));
    if spells(&TRUE_SPELLINGS) {
        Some(true)
    } else if spells(&FALSE_SPELLINGS) {
        Some(false)
    } else {
        None
    }
}

/// [`bool_from_text`] as the value a boolean column stores: what the value
/// door reads a text cell through. Inference proves a boolean only from what
/// one prints ([`prints_boolean`]).
pub(crate) fn boolean_from_text(text: &str) -> Option<Scalar> {
    bool_from_text(text).map(Scalar::from)
}

/// The boolean a cell holds: a boolean is itself, and a string leaf is read
/// through [`bool_from_text`].
///
/// Only a string is a spelling waiting to be read: a registered code and an
/// enum member also answer [`Scalar::as_str`], but their identity is the
/// registry and the member rather than the characters, so neither is a flag
/// - `Country("NO")` is Norway.
pub(crate) fn bool_of(value: &Scalar) -> Option<bool> {
    value
        .as_bool()
        .or_else(|| bool_from_text(value.as_string()?.as_str()))
}

/// Whether text is truthy: the table's answer where the text spells a
/// boolean, else whether anything but blanks is there.
///
/// The one false set [`Scalar::is_truthy`] reads text by, so a column that
/// spells `no` or `off` is as false to a predicate as to its boolean cast,
/// and `n/a` - text no boolean spells - is present, so true.
pub(crate) fn truthy_text(text: &str) -> bool {
    bool_from_text(text).unwrap_or_else(|| !text.trim().is_empty())
}

/// Whether `text` is a boolean as one prints: `true` or `false`, the case
/// and the surrounding blanks not part of the spelling.
///
/// Inference reads what a value already is, so this - never the wider
/// [`bool_from_text`] - is what proves a column boolean: `1` already is
/// an integer, and a column of them is not a column of flags.
pub(crate) fn prints_boolean(text: &str) -> bool {
    let text = text.trim();
    text.eq_ignore_ascii_case("true") || text.eq_ignore_ascii_case("false")
}

// ------------------------------------------------------------------------
// Arrow casts owned by the boolean datatype.
// ------------------------------------------------------------------------

/// Arrow casts owned by this datatype.
pub(crate) mod casts {
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, BooleanArray};
    use arrow_buffer::{BooleanBuffer, BooleanBufferBuilder, NullBuffer};
    use arrow_schema::DataType as ArrowDataType;

    use super::{BOOLEAN_SPELLINGS, bool_from_text};
    use crate::arrow::{Error, Result};
    use crate::budget::MaterializationBudget;
    use crate::cast::arrow_cast_exposed;
    use crate::cast::columns::is_exposed;
    use crate::cast::text::{TextCells, encoded_value_of};
    use crate::{DataType, Field};

    /// Whether a target datatype holds booleans, however it encodes them.
    pub(crate) fn holds_boolean(target: &DataType) -> bool {
        matches!(encoded_value_of(target), DataType::Boolean)
    }

    /// Reads a column of text into booleans through [`bool_from_text`], the
    /// reading a row takes, so a batch and a cell answer one table and
    /// Arrow's kernel is never a second reader of a flag.
    ///
    /// A plain text layout is read where it lies; a dictionary or run-end
    /// pair over text is decoded to `Utf8` once, through Arrow's kernel. Each
    /// exposed cell is one table lookup and two bits, so the column costs its
    /// two bitmaps and nothing per row. Text no boolean spells is null under
    /// `safe` and refused, naming the field and the row, otherwise; an absent
    /// or unexposed row is never read.
    pub(crate) fn ingest_boolean_text(
        array: &ArrayRef,
        safe: bool,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        let text = match array.data_type() {
            ArrowDataType::Utf8 | ArrowDataType::LargeUtf8 | ArrowDataType::Utf8View => {
                Arc::clone(array)
            }
            _ => arrow_cast_exposed(
                array,
                &ArrowDataType::Utf8,
                safe,
                exposure,
                &Field::new(field.name(), DataType::utf8(), true),
                budget,
            )?,
        };
        let cells = TextCells::of(text.as_ref())?;
        let rows = cells.len();
        budget.add_bitmap(rows)?;
        budget.add_bitmap(rows)?;
        let mut values = BooleanBufferBuilder::new(rows);
        let mut validity = BooleanBufferBuilder::new(rows);
        for index in 0..rows {
            let cell =
                (is_exposed(exposure, index) && cells.is_valid(index)).then(|| cells.value(index));
            let read = cell.and_then(bool_from_text);
            if let (Some(cell), None, false) = (cell, read, safe) {
                return Err(Error::IncompatibleSchema(format!(
                    "field {:?} row {index}: {cell:?} does not read as boolean: \
                     expected {BOOLEAN_SPELLINGS}",
                    field.name(),
                )));
            }
            values.append(read.unwrap_or(false));
            validity.append(read.is_some());
        }
        let nulls = NullBuffer::new(validity.finish());
        Ok(Arc::new(BooleanArray::new(
            values.finish(),
            (nulls.null_count() != 0).then_some(nulls),
        )))
    }
}

// ------------------------------------------------------------------------
// Arrow projection: the two logic-free datatypes are Arrow's own.
// ------------------------------------------------------------------------

mod arrow {
    use arrow_schema::DataType as ArrowDataType;
    use smol_str::format_smolstr;

    use super::{BooleanType, NullType};
    use crate::invalid;
    use crate::{DataType, Result};

    impl NullType {
        /// The Arrow storage a null column lays out.
        pub(crate) const fn arrow_storage() -> ArrowDataType {
            ArrowDataType::Null
        }
    }

    impl BooleanType {
        /// The Arrow storage a boolean column lays out.
        pub(crate) const fn arrow_storage() -> ArrowDataType {
            ArrowDataType::Boolean
        }
    }

    /// The datatype one of Arrow's two logic-free storages imports as.
    ///
    /// # Errors
    ///
    /// Returns an error when the storage belongs to another family.
    pub(crate) fn from_arrow_storage(value: &ArrowDataType) -> Result<DataType> {
        match value {
            ArrowDataType::Null => Ok(DataType::Null),
            ArrowDataType::Boolean => Ok(DataType::Boolean),
            other => Err(invalid(
                "Boolean",
                format_smolstr!("expected a null or boolean storage, got {other}"),
            )),
        }
    }
}

pub(crate) use arrow::from_arrow_storage;

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/boolean.rs` pins and a caller cannot reach: the
    //! one boolean table and the truthiness it decides.
    /// What every refusal of a boolean spelling names.
    pub const BOOLEAN_SPELLINGS: &str = super::BOOLEAN_SPELLINGS;

    /// Read a boolean out of text through the one table.
    pub fn bool_from_text(text: &str) -> Option<bool> {
        super::bool_from_text(text)
    }

    /// The boolean a cell holds, a string leaf read through the one table.
    pub fn bool_of(value: &crate::Scalar) -> Option<bool> {
        super::bool_of(value)
    }

    /// Whether text is truthy by the one table, else by its presence.
    pub fn truthy_text(text: &str) -> bool {
        super::truthy_text(text)
    }
}
