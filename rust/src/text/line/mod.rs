//! Plain-text rows reached through the shared record-media surface.

mod handle;
mod options;
mod reader;
mod sep;

#[cfg(feature = "arrow")]
pub(crate) mod arrow;

pub use handle::Text;
pub use options::TextOptions;
pub use sep::LineSep;

#[cfg(test)]
mod tests;
