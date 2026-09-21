//! Character-encoding integration tests.

#[path = "charset/bom.rs"]
mod bom;
#[path = "charset/decoder.rs"]
mod decoder;
#[path = "charset/reader.rs"]
mod reader;
#[path = "charset/single_byte.rs"]
mod single_byte;
#[path = "charset/transcoded.rs"]
mod transcoded;
#[path = "charset/utf16.rs"]
mod utf16;
#[path = "charset/writer.rs"]
mod writer;
