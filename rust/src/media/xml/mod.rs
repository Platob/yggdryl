//! XML documents reached through the shared record-media surface.
//!
//! A document is rows: the document element holds one element per row, and
//! that element's children are the columns. The mapping from an element to a
//! value is [`crate::text::xml`]'s, so a row read here and a document read
//! there agree by construction - an attribute is a column named with
//! [`ATTRIBUTE_PREFIX`](crate::text::xml::ATTRIBUTE_PREFIX), an element's own
//! text is [`TEXT_KEY`](crate::text::xml::TEXT_KEY), and a repeated element is
//! a list.
//!
//! What this module adds is position. XML carries no offset table, so
//! [`Xml::read_row_index`] reads one: the byte span of every row, decoding no
//! value. With it a row is addressable - [`Xml::read_row_scalar`] reads one row
//! and parses nothing else, [`Xml::write_row_scalar`] replaces one row in place
//! when the replacement is the same length and otherwise moves only the bytes
//! after it, and [`Xml::append_row_scalars`] rewrites the end tag rather than
//! the document.

#[cfg(feature = "arrow")]
mod arrow;
#[cfg(feature = "arrow")]
mod handle;
#[cfg(feature = "arrow")]
mod index;
mod options;
#[cfg(feature = "arrow")]
mod random;
#[cfg(feature = "arrow")]
mod reader;
#[cfg(feature = "arrow")]
mod writer;

#[cfg(feature = "arrow")]
pub use arrow::{
    append_arrow_reader, overwrite_arrow_reader, read_batch_reader, read_field, row_size,
};
#[cfg(feature = "arrow")]
pub use handle::Xml;
#[cfg(feature = "arrow")]
pub use index::{RowIndex, RowSpan};
pub use options::{DEFAULT_DOCUMENT_ELEMENT, XmlOptions};

#[cfg(all(test, feature = "arrow"))]
mod tests;
