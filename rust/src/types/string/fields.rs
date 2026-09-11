//! Every string's field marker: the five layouts and the ASCII repertoire.
//!
//! One file because a marker is one line per datatype and the family is one
//! family; splitting them would be two lists to keep in step rather than one.

use crate::metadata::{FIELD_ENUM_KEY, parse_ascii_enum};
use crate::types::typed::define_field_types;
use crate::{AsciiEnum, Field, Result, TypedField};

define_field_types!(Utf8Type, "utf8", crate::DataType::Utf8);
define_field_types!(LargeUtf8Type, "large_utf8", crate::DataType::LargeUtf8);
define_field_types!(Utf8ViewType, "utf8_view", crate::DataType::Utf8View);
define_field_types!(StringType, "string", crate::DataType::String(_));

/// A UTF-8-typed field.
pub type Utf8Field = TypedField<Utf8Type>;
/// A large UTF-8-typed field.
pub type LargeUtf8Field = TypedField<LargeUtf8Type>;
/// A UTF-8-view-typed field.
pub type Utf8ViewField = TypedField<Utf8ViewType>;
/// A field of strings declaring a charset, a bound, or both.
///
/// The three markers above are the plain UTF-8 layouts, which are datatypes
/// of their own; this one is every string that declares more than its layout,
/// whichever of the five layouts it declares.
pub type StringField = TypedField<StringType>;

define_field_types!(AsciiType, "ascii", crate::DataType::Ascii);
define_field_types!(
    FixedAsciiType,
    "fixed_ascii",
    crate::DataType::FixedAscii(_)
);

define_field_types!(CountryType, "country", crate::DataType::Country);
define_field_types!(CurrencyType, "currency", crate::DataType::Currency);
define_field_types!(MicType, "mic", crate::DataType::Mic);
define_field_types!(CfiType, "cfi", crate::DataType::Cfi);
define_field_types!(IsinType, "isin", crate::DataType::Isin);
define_field_types!(SideType, "side", crate::DataType::Side);
define_field_types!(MsgDirectionType, "direction", crate::DataType::MsgDirection);
define_field_types!(StateType, "state", crate::DataType::State);
define_field_types!(TimeInForceType, "timeinforce", crate::DataType::TimeInForce);

/// A variable-width ASCII-typed field.
pub type AsciiField = TypedField<AsciiType>;
/// A fixed-width ASCII-typed field.
pub type FixedAsciiField = TypedField<FixedAsciiType>;
/// A country-typed field: ISO 3166-1 alpha-2.
pub type CountryField = TypedField<CountryType>;
/// A currency-typed field: ISO 4217.
pub type CurrencyField = TypedField<CurrencyType>;
/// A MIC-typed field: ISO 10383's market identifier.
pub type MicField = TypedField<MicType>;
/// A CFI-typed field: ISO 10962's instrument classification.
pub type CfiField = TypedField<CfiType>;
/// An ISIN-typed field: ISO 6166's securities identification number.
pub type IsinField = TypedField<IsinType>;
/// A side-typed field: FIX's side of a trade.
pub type SideField = TypedField<SideType>;
/// A direction-typed field: which way a captured line moved.
pub type DirectionField = TypedField<MsgDirectionType>;
/// A field declared as a thing's state.
pub type StateField = TypedField<StateType>;
/// A field declared as how long an order stands.
pub type TimeInForceField = TypedField<TimeInForceType>;

impl Field {
    /// The enum this field's ASCII values name, if one is declared.
    ///
    /// # Errors
    ///
    /// Returns an error only for externally corrupted serialized state.
    pub fn ascii_enum(&self) -> Result<Option<AsciiEnum>> {
        self.get_metadata(FIELD_ENUM_KEY)
            .map(parse_ascii_enum)
            .transpose()
    }

    /// Declares the enum this field's ASCII values name.
    ///
    /// # Errors
    ///
    /// Returns an error when this field cannot store every enum member.
    pub fn set_ascii_enum(&mut self, value: &AsciiEnum) -> Result<()> {
        value.into_members(&self.dtype)?;
        let (_, changed) = self
            .metadata
            .insert_validated(FIELD_ENUM_KEY.to_owned(), value.into_json());
        if changed {
            self.invalidate_arrow();
        }
        Ok(())
    }

    /// Returns a persistent field declaring one enum over its ASCII values.
    pub fn try_with_ascii_enum(mut self, value: &AsciiEnum) -> Result<Self> {
        self.set_ascii_enum(value)?;
        Ok(self)
    }

    /// Removes the declaration and returns the enum it held.
    ///
    /// # Errors
    ///
    /// Returns an error only for externally corrupted serialized state.
    pub fn remove_ascii_enum(&mut self) -> Result<Option<AsciiEnum>> {
        self.remove_metadata(FIELD_ENUM_KEY)
            .map(|value| parse_ascii_enum(&value))
            .transpose()
    }
}
