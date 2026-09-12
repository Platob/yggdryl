//! What a byte datatype lays out in Arrow, and what it reads back from.
//!
//! The four layouts are Arrow's own, so the storage is the layout. What Arrow
//! cannot say - a maximum on a variable layout - rides the `yggdryl.bytes`
//! extension document beside it; a fixed width is the storage itself and
//! needs no document.

use arrow_schema::DataType as ArrowDataType;

use super::{BytesLayout, BytesParameters};
use crate::{Error, Result};

/// Whether a byte field needs the `yggdryl.bytes` document beside its
/// storage: only a maximum, which no Arrow layout can state.
pub(crate) const fn needs_extension(parameters: BytesParameters) -> bool {
    parameters.is_bounded() && !parameters.is_fixed()
}

/// The Arrow storage one byte datatype lays out.
///
/// # Errors
///
/// Returns [`Error::InvalidDataType`] when a fixed width is outside `i32`,
/// which is as wide as Arrow's own fixed binary counts.
pub(crate) fn arrow_storage(parameters: BytesParameters) -> Result<ArrowDataType> {
    // The variant is public, so a fixed layout can arrive here without the
    // width that makes it fixed. A boundary is where that stops.
    parameters.validate()?;
    if let Some(width) = parameters.fixed() {
        let width = i32::try_from(width).map_err(|_| Error::InvalidDataType {
            kind: "bytes",
            reason: smol_str::format_smolstr!(
                "fixed width {width} is outside the i32 range Arrow counts in"
            ),
        })?;
        return Ok(ArrowDataType::FixedSizeBinary(width));
    }
    Ok(match parameters.layout() {
        BytesLayout::Binary => ArrowDataType::Binary,
        BytesLayout::LargeBinary => ArrowDataType::LargeBinary,
        BytesLayout::BinaryView => ArrowDataType::BinaryView,
        // The fixed layout answered above: `validate` gave it a width and
        // the width gave it its storage.
        BytesLayout::FixedSizeBinary => ArrowDataType::Binary,
    })
}

/// Whether one Arrow storage is what these parameters lay out.
///
/// The import side asks this rather than re-deriving: a `yggdryl.bytes`
/// document over a storage it does not describe is a foreign field wearing
/// our name, and it imports as its storage instead.
pub(crate) fn describes_storage(
    parameters: BytesParameters,
    storage: &ArrowDataType,
) -> Result<bool> {
    Ok(arrow_storage(parameters)? == *storage)
}
