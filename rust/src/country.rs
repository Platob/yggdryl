//! ISO 3166-1 alpha-2 country codes.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::code::{code_leaf, code_value};
use crate::typed::define_field_types;
use crate::value::CodeValue;
use crate::{DataType, Result, Scalar, Value};

code_leaf!(Country, COUNTRY_WIDTH);

code_value!(Country, Country, COUNTRY_WIDTH);

// ------------------------------------------------------------------------
// The registered codes: the identifiers of a trade, each its own datatype.
//
// A country, a currency, a market identifier and a CFI classification are
// not four names for a bounded ASCII string. Each is a distinct logical type
// over a published registry, each has exactly one width its standard fixes,
// and a column of one is never a column of another however alike their bytes
// look. So each is a [`DataType`] variant of its own, with its own Arrow
// extension name, its own typed field and scalar, and its own cast path.
//
// The value contract, stated once here: a value is ASCII text - every byte
// at most `0x7F` - of at most the code's width in bytes, with no NUL byte.
// Storage is Arrow's `Utf8`, which is what the text is, so a cell holds the
// value exactly and there is no padding to write or trim.
//
// What a code adds over the text that would hold it is identity and a
// bound. The identity crosses Arrow as its own extension name, so a
// currency column reads back a currency and not anonymous text. The bound
// is [`DataType::code_width`]: it is known at compile time for each code,
// so the ingest path here is monomorphized per width rather than reading a
// length out of the datatype on every row, and it is the width
// [`DataType::ascii_packed`] pads into, so [`crate::StringEnum`] works over
// a code with nothing added - an enum member is still the integer its value
// packs into.
//
// The width is a maximum, never a layout: [`DataType::fixed_byte_width`]
// answers `None` for a code, exactly as it does for the variable string a
// code stores like.
//
// The widths are the ones the standards fix: two bytes for ISO 3166-1's
// country code, three for ISO 4217's currency, four for ISO 10383's market
// identifier, six for ISO 10962's classification, and for the three
// securities identifiers twelve for ISO 6166's ISIN, nine for a CUSIP and
// seven for a SEDOL, each closed by its own check digit.
// ------------------------------------------------------------------------

/// The Arrow extension name of the country code.
pub(crate) const COUNTRY_EXTENSION_NAME: &str = "yggdryl.country";

/// The most bytes ISO 3166-1's country code may be.
pub(crate) const COUNTRY_WIDTH: usize = 2;

impl DataType {
    /// Creates ISO 3166-1's two-letter country code.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::country(), DataType::Country);
    /// assert_eq!(DataType::country().to_string(), "country");
    /// assert_eq!(DataType::country().code_width(), Some(2));
    /// ```
    #[must_use]
    pub const fn country() -> Self {
        Self::Country
    }
}

// /// A country-typed field: ISO 3166-1 alpha-2.
define_field_types!(CountryType, Country);
