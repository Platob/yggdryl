//! ASCII widths, registered codes, and named value dictionaries.

#[cfg(feature = "arrow")]
pub(crate) mod casts;

mod dictionary;
pub mod dtypes;
mod fields;
pub(crate) mod iso;
mod scalars;
mod vocabulary;

pub use dictionary::AsciiEnum;
pub(crate) use dtypes::{
    ASCII_EXTENSION_NAME, CFI_WIDTH, COUNTRY_WIDTH, CURRENCY_WIDTH, DIRECTION_WIDTH, ISIN_WIDTH,
    MIC_WIDTH, SIDE_WIDTH, STATE_WIDTH, TIMEINFORCE_WIDTH, ascii_bytes, ascii_document_width,
    ascii_free_text, ascii_text, ascii_width_document, code_cell_text, code_extension_name,
    code_for_extension, validate_ascii_width,
};
#[cfg(feature = "arrow")]
pub(crate) use dtypes::{ascii_value_text, code_refusal, code_text, code_value_text};
pub use fields::*;
pub use scalars::{
    Ascii, AsciiFamily, AsciiValue, Cfi, Country, Currency, FixedAscii, Isin, Mic, MsgDirection,
    Side, State, TimeInForce,
};
