//! Comma-separated values, RFC 4180, as record media over any byte handle.
//!
//! A `.csv` handle, or a `.tsv` one, which is the same document under a
//! tab, reads and writes through the ordinary record surface. Reading streams:
//! records are cut from the decoded transport one at a time, so a read holds
//! one batch and a count holds one record. A declared field is the contract
//! every cell is read under; without one the header names the columns and a
//! bounded sample of the records types them - boolean, integer, float, date,
//! instant, else text, every inferred column nullable - and a later cell its
//! column cannot read is refused, naming the sample, never nulled. Every
//! cell reads through its column's value door. Writing renders every leaf as
//! the text it reads back from and quotes only what has to be; a write onto
//! a stored document completes onto its header, never onto its sample.
//! Compression and charsets are the handle's: `trades.csv.gz` is gzip by
//! name and `;charset=windows-1252` a declared charset, both read and
//! written through the same doors every text medium uses.
//!
//! ```
//! use yggdryl::csv::CsvOptions;
//! use yggdryl::holder::Buffer;
//! use yggdryl::media::IORecordOptions;
//! use yggdryl::{DataType, IOBase, IOMedia, MediaType, Scalar};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let mut handle = Buffer::new().with_media_type(MediaType::from_file_name("trades.csv"));
//! handle.write_all_bytes(b"symbol,size,price\nAAPL,100,187.25\nMSFT,,410.10\n")?;
//!
//! // Undeclared, the header names the columns and the rows type them.
//! let options = handle.record_options()?;
//! let field = handle.read_arrow_field(&options)?;
//! assert_eq!(field.name(), "row");
//! assert_eq!(
//!     field.dtype(),
//!     &DataType::from_str("struct<symbol: utf8, size: int64, price: float64>")?
//! );
//! assert_eq!(handle.row_size()?, 2);
//!
//! // Declared, every cell is read under the column's own contract.
//! let declared = DataType::from_str(
//!     "struct<symbol: utf8 not null, size: int32, price: decimal(10, 2) not null>",
//! )?
//! .required_field("trade");
//! let rows = handle.read_arrow(Some(&options.with_field(declared)))?
//!     .map(|batch| batch.map(|batch| batch.len()))
//!     .sum::<yggdryl::arrow::Result<usize>>()?;
//! assert_eq!(rows, 2);
//!
//! // A tab-separated document, by name; a `;` separator, by option.
//! assert_eq!(CsvOptions::tsv().separator(), b'\t');
//! let semicolon = CsvOptions::new().with_separator(b';')?;
//! assert_eq!(semicolon.separator(), b';');
//! # let _ = Scalar::Null;
//! # Ok(())
//! # }
//! ```

mod media;
mod options;
mod reader;
mod writer;

pub use media::{Csv, overwrite_arrow_reader, read_batch_reader, read_field};
pub(crate) use media::{append_arrow_reader, row_size, stated_field, write_target};
pub use options::{CsvOptions, DEFAULT_CSV_BATCH_BYTE_SIZE, DEFAULT_CSV_INFER_ROW_SIZE};
