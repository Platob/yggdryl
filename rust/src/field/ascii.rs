//! The ASCII family and registered code field markers.

use super::typed::define_field_types;

define_field_types!(Ascii, "ascii", crate::DataType::Ascii);
define_field_types!(FixedAscii, "fixed_ascii", crate::DataType::FixedAscii(_));

define_field_types!(Country, "country", crate::DataType::Country);
define_field_types!(Currency, "currency", crate::DataType::Currency);
define_field_types!(Mic, "mic", crate::DataType::Mic);
define_field_types!(Cfi, "cfi", crate::DataType::Cfi);
