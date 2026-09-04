//! The ASCII width and registered code field markers.

use super::typed::define_field_types;

define_field_types!(Ascii16, "ascii16", crate::DataType::Ascii16);
define_field_types!(Ascii24, "ascii24", crate::DataType::Ascii24);
define_field_types!(Ascii32, "ascii32", crate::DataType::Ascii32);
define_field_types!(Ascii64, "ascii64", crate::DataType::Ascii64);
define_field_types!(Ascii96, "ascii96", crate::DataType::Ascii96);
define_field_types!(Ascii128, "ascii128", crate::DataType::Ascii128);

define_field_types!(Country, "country", crate::DataType::Country);
define_field_types!(Currency, "currency", crate::DataType::Currency);
define_field_types!(Mic, "mic", crate::DataType::Mic);
define_field_types!(Cfi, "cfi", crate::DataType::Cfi);
