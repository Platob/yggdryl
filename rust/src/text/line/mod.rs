//! The one home of every line read and write.
//!
//! This is a *text-line* surface, **not a fourth record method**. The record
//! surface stays exactly [`read_arrow_reader`](crate::io::IOMedia::read_arrow_reader),
//! [`overwrite_arrow_reader`](crate::io::IOMedia::overwrite_arrow_reader),
//! and [`append_arrow_reader`](crate::io::IOMedia::append_arrow_reader),
//! and nothing here decodes a record encoding. What it reads is *text*: bytes
//! split into records by a line terminator, grouped by a pattern, and either
//! handed back as borrowed views or projected into Arrow batches.
//!
//! [`Text<H>`] is the handle. Like [`Coded`](crate::io::Coded),
//! [`Gzip`](crate::gzip::Gzip), [`Ipc`](crate::ipc::Ipc), and
//! [`Parquet`](crate::parquet::Parquet), it wraps another handle, mirrors every
//! byte method, and exposes the *raw encoded bytes unchanged* - so a `.log.gz`
//! behind a `Text` can still be copied or uploaded verbatim. It overrides only
//! [`open`](crate::io::IOBase::open) and [`close`](crate::io::IOBase::close),
//! which materialize and release what repeated calls would re-derive.
//!
//! ```
//! use yggdryl::io::{Buffer, IOBase};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let mut handle = Buffer::new().into_text();
//! handle.write_lines(["first", "second"])?;
//!
//! // Read back as borrowed views - no `String` per line.
//! let mut lines = handle.read_lines()?;
//! assert_eq!(lines.next().transpose()?.map(|line| line.bytes()), Some(&b"first"[..]));
//! assert_eq!(lines.next().transpose()?.map(|line| line.bytes()), Some(&b"second"[..]));
//! assert!(lines.next().is_none());
//!
//! // The wrapped bytes are exactly what was written.
//! assert_eq!(handle.read_all_bytes()?, b"first\nsecond\n");
//! # Ok(())
//! # }
//! ```

mod handle;
pub mod log;
mod options;
mod reader;
#[cfg(feature = "arrow")]
mod record;
mod sep;
mod strip;
mod timestamp;
mod view;

#[cfg(feature = "arrow")]
pub mod arrow;

pub use handle::{Text, TextLines};

pub(crate) use handle::borrowed_lines;
#[cfg(any(feature = "arrow", test))]
pub(crate) use handle::row_size;
pub use options::{Opening, TextLineOptions};
#[cfg(feature = "arrow")]
pub use record::TextOptions;
pub use sep::LineSep;
pub use strip::Strip;
pub use view::{TextLine, TextLineBuf};

/// The predicate that decides what opens a record.
///
/// A supplied pattern is matched against the line; unset, a record opens where
/// a **timestamp** opens - see [`log`]. The closure is built once per read, not
/// per line, so the configuration branch is paid once.
pub(crate) fn opener(options: &TextLineOptions) -> impl FnMut(&[u8]) -> bool + '_ {
    let opening = options.opening();
    move |line: &[u8]| match opening {
        // Every line opens a record, so the grouping never looks at content.
        Opening::EveryLine => true,
        Opening::Timestamp => log::opens_with_timestamp(line),
        Opening::Pattern(pattern) => match std::str::from_utf8(line) {
            Ok(text) => pattern.is_match(text),
            // A line that is not text cannot match a text pattern; the failure
            // is reported where the record's text is asked for.
            Err(_) => false,
        },
    }
}

#[cfg(test)]
mod tests;
