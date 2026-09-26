//! OLE DB's type indicators, which the `DBSCHEMA_*` rowsets state a column's
//! datatype as.
//!
//! `DBSCHEMA_COLUMNS` and `DBSCHEMA_PROVIDER_TYPES` answer a column's type as
//! a `DBTYPE` number, the vocabulary OLE DB defined and every XMLA client
//! reads. This is the one mapping from the crate's [`DataType`] onto it: a
//! family maps to the indicator that holds it, and a nested column to the
//! chapter that names a nested rowset.

use std::fmt;

use crate::DataType;

/// One OLE DB type indicator, by its `DBTYPE_*` name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DbType {
    /// `DBTYPE_EMPTY`: a column that holds nothing.
    Empty,
    /// `DBTYPE_I2`: a two-byte signed integer.
    I2,
    /// `DBTYPE_I4`: a four-byte signed integer.
    I4,
    /// `DBTYPE_R4`: a single-precision float.
    R4,
    /// `DBTYPE_R8`: a double-precision float.
    R8,
    /// `DBTYPE_BOOL`: a boolean.
    Bool,
    /// `DBTYPE_VARIANT`: a value of any type.
    Variant,
    /// `DBTYPE_I1`: a one-byte signed integer.
    I1,
    /// `DBTYPE_UI1`: a one-byte unsigned integer.
    Ui1,
    /// `DBTYPE_UI2`: a two-byte unsigned integer.
    Ui2,
    /// `DBTYPE_UI4`: a four-byte unsigned integer.
    Ui4,
    /// `DBTYPE_I8`: an eight-byte signed integer.
    I8,
    /// `DBTYPE_UI8`: an eight-byte unsigned integer.
    Ui8,
    /// `DBTYPE_GUID`: a 128-bit identifier.
    Guid,
    /// `DBTYPE_BYTES`: a byte string.
    Bytes,
    /// `DBTYPE_WSTR`: a Unicode character string.
    Wstr,
    /// `DBTYPE_NUMERIC`: an exact number with precision and scale.
    Numeric,
    /// `DBTYPE_DBDATE`: a calendar date.
    DbDate,
    /// `DBTYPE_DBTIME`: a time of day.
    DbTime,
    /// `DBTYPE_DBTIMESTAMP`: a date and time.
    DbTimestamp,
    /// `DBTYPE_HCHAPTER`: a nested rowset.
    HChapter,
}

impl DbType {
    /// Every indicator this crate maps a column to, in numeric order.
    pub const ALL: &'static [Self] = &[
        Self::Empty,
        Self::I2,
        Self::I4,
        Self::R4,
        Self::R8,
        Self::Bool,
        Self::Variant,
        Self::I1,
        Self::Ui1,
        Self::Ui2,
        Self::Ui4,
        Self::I8,
        Self::Ui8,
        Self::Guid,
        Self::Bytes,
        Self::Wstr,
        Self::Numeric,
        Self::DbDate,
        Self::DbTime,
        Self::DbTimestamp,
        Self::HChapter,
    ];

    /// The indicator's number, as `DATA_TYPE` states it.
    #[must_use]
    pub const fn code(self) -> u16 {
        match self {
            Self::Empty => 0,
            Self::I2 => 2,
            Self::I4 => 3,
            Self::R4 => 4,
            Self::R8 => 5,
            Self::Bool => 11,
            Self::Variant => 12,
            Self::I1 => 16,
            Self::Ui1 => 17,
            Self::Ui2 => 18,
            Self::Ui4 => 19,
            Self::I8 => 20,
            Self::Ui8 => 21,
            Self::Guid => 72,
            Self::Bytes => 128,
            Self::Wstr => 130,
            Self::Numeric => 131,
            Self::DbDate => 133,
            Self::DbTime => 134,
            Self::DbTimestamp => 135,
            Self::HChapter => 136,
        }
    }

    /// The `DBTYPE_*` name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "DBTYPE_EMPTY",
            Self::I2 => "DBTYPE_I2",
            Self::I4 => "DBTYPE_I4",
            Self::R4 => "DBTYPE_R4",
            Self::R8 => "DBTYPE_R8",
            Self::Bool => "DBTYPE_BOOL",
            Self::Variant => "DBTYPE_VARIANT",
            Self::I1 => "DBTYPE_I1",
            Self::Ui1 => "DBTYPE_UI1",
            Self::Ui2 => "DBTYPE_UI2",
            Self::Ui4 => "DBTYPE_UI4",
            Self::I8 => "DBTYPE_I8",
            Self::Ui8 => "DBTYPE_UI8",
            Self::Guid => "DBTYPE_GUID",
            Self::Bytes => "DBTYPE_BYTES",
            Self::Wstr => "DBTYPE_WSTR",
            Self::Numeric => "DBTYPE_NUMERIC",
            Self::DbDate => "DBTYPE_DBDATE",
            Self::DbTime => "DBTYPE_DBTIME",
            Self::DbTimestamp => "DBTYPE_DBTIMESTAMP",
            Self::HChapter => "DBTYPE_HCHAPTER",
        }
    }

    /// The indicator with `code`, `None` for one this crate maps nothing to.
    #[must_use]
    pub fn from_code(code: u16) -> Option<Self> {
        Self::ALL.iter().copied().find(|held| held.code() == code)
    }

    /// The indicator a column of `dtype` is stated as.
    ///
    /// Every string, code and identifier is `DBTYPE_WSTR`, every byte payload
    /// `DBTYPE_BYTES`, every decimal `DBTYPE_NUMERIC`, a struct or a sequence
    /// `DBTYPE_HCHAPTER`, and a union or a variant `DBTYPE_VARIANT`.
    #[must_use]
    pub fn of(dtype: &DataType) -> Self {
        match dtype {
            DataType::Null => Self::Empty,
            DataType::Boolean => Self::Bool,
            DataType::Int8 => Self::I1,
            DataType::Int16 => Self::I2,
            DataType::Int32 => Self::I4,
            DataType::Int64 => Self::I8,
            DataType::UInt8 => Self::Ui1,
            DataType::UInt16 => Self::Ui2,
            DataType::UInt32 => Self::Ui4,
            DataType::UInt64 => Self::Ui8,
            DataType::Float16 | DataType::Float32 => Self::R4,
            DataType::Float64 => Self::R8,
            DataType::Decimal32 { .. }
            | DataType::Decimal64 { .. }
            | DataType::Decimal128 { .. }
            | DataType::Decimal256 { .. } => Self::Numeric,
            DataType::Date32 | DataType::Date64 => Self::DbDate,
            DataType::Time32(_) | DataType::Time64(_) => Self::DbTime,
            DataType::DateTime64 { .. } => Self::DbTimestamp,
            DataType::Uuid => Self::Guid,
            DataType::Struct(_)
            | DataType::Serie(_)
            | DataType::SerieView(_)
            | DataType::FixedSizeSerie(_, _)
            | DataType::LargeSerie(_)
            | DataType::LargeSerieView(_)
            | DataType::Map(_)
            | DataType::SortedMap(_) => Self::HChapter,
            DataType::Union(_, _) | DataType::Variant => Self::Variant,
            DataType::Dictionary(dictionary) => Self::of(dictionary.value()),
            DataType::RunEndEncoded(encoded) => Self::of(encoded.values().dtype()),
            other if other.bytes_parameters().is_some() => Self::Bytes,
            DataType::Geometry(_) | DataType::Geography(_) => Self::Bytes,
            // Every string leaf, every registered code, every identifier and
            // spelled value - a version, a URL, a zone, a media type - and
            // the durations and intervals OLE DB has no indicator for.
            _ => Self::Wstr,
        }
    }

    /// The `COLUMN_SIZE` OLE DB states for the indicator in
    /// `DBSCHEMA_PROVIDER_TYPES`: the maximum decimal precision of a number
    /// (`I4` holds ten digits, `UI8` twenty, a double fifteen), the length of
    /// a temporal's text, the byte width of a GUID or a boolean, and `None`
    /// where the width is the column's own.
    #[must_use]
    pub const fn column_size(self) -> Option<u32> {
        match self {
            Self::Empty => Some(0),
            Self::Bool => Some(1),
            Self::I1 | Self::Ui1 => Some(3),
            Self::I2 | Self::Ui2 => Some(5),
            Self::I4 | Self::Ui4 => Some(10),
            Self::I8 => Some(19),
            Self::Ui8 => Some(20),
            Self::R4 => Some(7),
            Self::R8 => Some(15),
            Self::Guid => Some(16),
            Self::Numeric => Some(38),
            Self::DbDate => Some(10),
            // `hh:mm:ss`: a DBTIME carries no fraction.
            Self::DbTime => Some(8),
            Self::DbTimestamp => Some(29),
            Self::Bytes | Self::Wstr | Self::Variant | Self::HChapter => None,
        }
    }

    /// Whether the indicator is a signed or unsigned number, `None` for one
    /// that is not a number.
    #[must_use]
    pub const fn is_unsigned(self) -> Option<bool> {
        match self {
            Self::Ui1 | Self::Ui2 | Self::Ui4 | Self::Ui8 => Some(true),
            Self::I1 | Self::I2 | Self::I4 | Self::I8 | Self::R4 | Self::R8 | Self::Numeric => {
                Some(false)
            }
            _ => None,
        }
    }

    /// Whether the indicator has a fixed precision and scale.
    #[must_use]
    pub const fn is_fixed_precision(self) -> bool {
        matches!(
            self,
            Self::I1
                | Self::I2
                | Self::I4
                | Self::I8
                | Self::Ui1
                | Self::Ui2
                | Self::Ui4
                | Self::Ui8
                | Self::Numeric
        )
    }
}

impl fmt::Display for DbType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
