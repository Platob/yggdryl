//! [`Compression`] — a named, bidirectional compression codec.

use crate::{CompressionDecoder, CompressionEncoder};

/// A complete, named compression codec: both a
/// [`CompressionEncoder`](crate::CompressionEncoder) and a
/// [`CompressionDecoder`](crate::CompressionDecoder), round-tripping any byte
/// array losslessly.
///
/// ```
/// use yggdryl_core::{Compression, Gzip};
///
/// let gzip = Gzip::new(6).unwrap();
/// assert_eq!(gzip.name(), "gzip");
/// ```
pub trait Compression: CompressionEncoder + CompressionDecoder {
    /// The lowercase codec name (e.g. `"gzip"`).
    fn name(&self) -> &'static str;
}
