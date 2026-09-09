//! Delimited text rows reached through the shared record-media surface.
//!
//! CSV is the one record encoding whose rows are addressable in the bytes that
//! store them, so this module offers both surfaces over the same resource: the
//! ordinary [`IOMedia`](crate::IOMedia) record methods, and the positional
//! reads and writes on [`Csv`] that reach one row or one cell without decoding
//! the rest.
//!
//! Record splitting is the plain-text splitter in
//! [`crate::media::text`], unchanged: `linesep` still terminates records, and
//! `separator` - the word that module reserved for exactly this - delimits the
//! cells inside one. A quoted cell holding the terminator joins physical lines
//! back together with the bytes the resource actually held.
//!
//! ```
//! use yggdryl::holder::Buffer;
//! use yggdryl::media::csv::{Csv, CsvOptions};
//! use yggdryl::{DataType, IOBase, IOMedia, Scalar};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let handle = Buffer::from_bytes(
//!     b"symbol,quantity,traded_on\nBRN,120,2024-01-02\nWTI,80,2024-01-03\n".to_vec(),
//! );
//! let mut csv = Csv::new(handle).with_options(CsvOptions::new());
//!
//! // The header names the columns and the cells type them.
//! let field = csv.read_arrow_field(&csv.record_options()?)?;
//! assert_eq!(field.get_field(1).unwrap().dtype(), &DataType::Int64);
//! assert_eq!(field.get_field(2).unwrap().dtype(), &DataType::Date32);
//!
//! // One row, without decoding the other.
//! assert_eq!(csv.read_cell_text(1, 0)?.as_deref(), Some("WTI"));
//!
//! // And one cell, written back in place.
//! csv.write_cell_scalar(1, 1, &Scalar::from(95_i64))?;
//! assert_eq!(csv.read_cell_text(1, 1)?.as_deref(), Some("95"));
//! # Ok(())
//! # }
//! ```

#[cfg(feature = "arrow")]
pub(crate) mod arrow;
mod handle;
#[cfg(feature = "arrow")]
mod index;
#[cfg(feature = "arrow")]
mod infer;
mod options;
#[cfg(feature = "arrow")]
mod random;
#[cfg(feature = "arrow")]
mod reader;
#[cfg(feature = "arrow")]
mod scan;

pub use handle::Csv;
pub use options::{CsvOptions, DEFAULT_INFER_ROW_SIZE};

#[cfg(feature = "arrow")]
pub use arrow::{
    append_arrow_reader, overwrite_arrow_reader, read_arrow_reader, read_field,
    read_owned_arrow_reader, row_size,
};

#[cfg(test)]
mod tests;
