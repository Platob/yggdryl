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
// The padding is the payload a declared width stores, which a value answers
// with or without an Arrow array around it.
pub(crate) use dtypes::ascii_padded;
pub(crate) use dtypes::{
    ASCII_EXTENSION_NAME, CFI_WIDTH, COUNTRY_WIDTH, CURRENCY_WIDTH, DIRECTION_WIDTH, ISIN_WIDTH,
    MIC_WIDTH, MSGTYPE_WIDTH, SIDE_WIDTH, STATE_WIDTH, TIMEINFORCE_WIDTH, ascii_bytes,
    ascii_free_text, ascii_text, code_cell_text, code_extension_name, code_for_extension,
};
#[cfg(feature = "arrow")]
pub(crate) use dtypes::{code_refusal, code_text};
pub use fields::*;
pub use scalars::{
    Ascii, AsciiFamily, AsciiScalar, AsciiValue, Cfi, CfiScalar, Country, CountryScalar, Currency,
    CurrencyScalar, FixedAscii, FixedAsciiScalar, Isin, IsinScalar, Mic, MicScalar, MsgDirection,
    MsgDirectionScalar, MsgType, MsgTypeScalar, Side, SideScalar, State, StateScalar, TimeInForce,
    TimeInForceScalar,
};
