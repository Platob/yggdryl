//! Shared pairs and structured-text dispatch pending their owning layers.

mod pairs;
mod text;

pub(crate) use pairs::sorted_pairs;
#[cfg(feature = "iceberg")]
pub(crate) use pairs::sorted_values;
pub use text::Text;
