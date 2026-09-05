//! ASCII widths, registered codes, and named value dictionaries.

#[cfg(feature = "arrow")]
pub(crate) mod casts;

mod dictionary;
mod dtypes;
mod fields;
pub(crate) mod iso;
mod scalars;
mod vocabulary;

pub use dictionary::AsciiEnum;
#[cfg(feature = "arrow")]
pub(crate) use dtypes::ascii_padded;
pub(crate) use dtypes::{
    ASCII_EXTENSION_NAME, CFI_WIDTH, COUNTRY_WIDTH, CURRENCY_WIDTH, MIC_WIDTH, ascii_bytes,
    ascii_free_text, ascii_text, code_cell_text, code_extension_name, code_for_extension,
    code_refusal, code_text,
};
pub use fields::*;
pub use scalars::{
    Ascii, AsciiFamily, AsciiScalar, AsciiValue, Cfi, CfiScalar, Country, CountryScalar, Currency,
    CurrencyScalar, FixedAscii, FixedAsciiScalar, Mic, MicScalar,
};
