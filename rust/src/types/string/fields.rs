//! Every string's field marker: the one family and the nine codes.
//!
//! One file because a marker is one line per datatype and the family is one
//! family; splitting them would be two lists to keep in step rather than one.

use crate::metadata::{FIELD_ENUM_KEY, parse_string_enum};
use crate::types::typed::define_field_types;
use crate::{Field, Result, StringEnum, TypedField};

define_field_types!(StringType, "string", crate::DataType::String(_));

/// A string-typed field, whichever layout, charset and bound it declares.
pub type StringField = TypedField<StringType>;

define_field_types!(CountryType, "country", crate::DataType::Country);
define_field_types!(CurrencyType, "currency", crate::DataType::Currency);
define_field_types!(MicType, "mic", crate::DataType::Mic);
define_field_types!(CfiType, "cfi", crate::DataType::Cfi);
define_field_types!(IsinType, "isin", crate::DataType::Isin);
define_field_types!(SideType, "side", crate::DataType::Side);
define_field_types!(
    MsgDirectionType,
    "msgdirection",
    crate::DataType::MsgDirection
);
define_field_types!(StateType, "state", crate::DataType::State);
define_field_types!(TimeInForceType, "timeinforce", crate::DataType::TimeInForce);

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
pub type MsgDirectionField = TypedField<MsgDirectionType>;
/// A field declared as a thing's state.
pub type StateField = TypedField<StateType>;
/// A field declared as how long an order stands.
pub type TimeInForceField = TypedField<TimeInForceType>;

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
        value.into_members(&self.dtype)?;
        let (_, changed) = self
            .metadata
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
