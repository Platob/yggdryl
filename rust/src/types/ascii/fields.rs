//! The ASCII family and registered code field markers.

use crate::metadata::{FIELD_ENUM_KEY, parse_ascii_enum};
use crate::types::typed::define_field_types;
use crate::{AsciiEnum, Field, Result, TypedField};

define_field_types!(Ascii, "ascii", crate::DataType::Ascii);
define_field_types!(FixedAscii, "fixed_ascii", crate::DataType::FixedAscii(_));

define_field_types!(Country, "country", crate::DataType::Country);
define_field_types!(Currency, "currency", crate::DataType::Currency);
define_field_types!(Mic, "mic", crate::DataType::Mic);
define_field_types!(Cfi, "cfi", crate::DataType::Cfi);

/// A variable-width ASCII-typed field.
pub type AsciiField = TypedField<Ascii>;
/// A fixed-width ASCII-typed field.
pub type FixedAsciiField = TypedField<FixedAscii>;
/// A country-typed field: ISO 3166-1 alpha-2.
pub type CountryField = TypedField<Country>;
/// A currency-typed field: ISO 4217.
pub type CurrencyField = TypedField<Currency>;
/// A MIC-typed field: ISO 10383's market identifier.
pub type MicField = TypedField<Mic>;
/// A CFI-typed field: ISO 10962's instrument classification.
pub type CfiField = TypedField<Cfi>;

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
