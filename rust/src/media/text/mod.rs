//! Plain-text rows reached through the shared record-media surface.

#[cfg(feature = "arrow")]
mod batch;
mod bytes;
mod entry;
mod handle;
mod leading;
mod line;
mod options;
mod plan;
mod reader;
mod sep;

#[cfg(feature = "arrow")]
pub(crate) mod arrow;

#[cfg(feature = "arrow")]
pub use arrow::{TextLines, read_text_lines};
#[cfg(feature = "arrow")]
pub use batch::{from_arrow_batch, from_arrow_reader, into_arrow_batch, into_arrow_reader};

pub use bytes::TextBytes;
pub use entry::{TextEntries, TextEntry};
pub use handle::Text;
pub use leading::LeadingFragment;
pub use line::TextLine;
pub use options::TextOptions;
pub use sep::LineSep;

#[cfg(test)]
mod tests;
