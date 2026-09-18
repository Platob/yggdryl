//! Plain-text rows reached through the shared record-media surface.

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

pub(crate) mod arrow;

pub use arrow::{TextLines, read_text_lines};
pub use batch::{from_arrow_batch, from_arrow_reader, into_arrow_batch, into_arrow_reader};

pub use bytes::TextBytes;
pub use entry::{TextEntries, TextEntry};
pub use handle::Text;
pub use leading::LeadingFragment;
pub use line::TextLine;
pub use options::TextOptions;
pub use sep::LineSep;
