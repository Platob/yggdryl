//! ASCII typed scalar aliases.

use crate::types::typed::define_scalar_type;

define_scalar_type!(AsciiScalar, super::Ascii, "ascii", crate::DataType::Ascii);
define_scalar_type!(FixedAsciiScalar, super::FixedAscii, "fixed_ascii");
define_scalar_type!(
    CountryScalar,
    super::Country,
    "country",
    crate::DataType::Country
);
define_scalar_type!(
    CurrencyScalar,
    super::Currency,
    "currency",
    crate::DataType::Currency
);
define_scalar_type!(MicScalar, super::Mic, "mic", crate::DataType::Mic);
define_scalar_type!(CfiScalar, super::Cfi, "cfi", crate::DataType::Cfi);
