//! What a string datatype lays out in Arrow, and what it reads back from.
//!
//! Arrow declares a layout and, for its three string layouts, UTF-8. It
//! declares no charset, no length bound, and it has one view layout where
//! this crate has two. So the projection is: text this crate stores as UTF-8
//! or US-ASCII rides Arrow's own string layouts - ASCII bytes are UTF-8, and
//! Arrow is told the truth about the bytes - text in any other charset rides
//! the matching *binary* layout, because the bytes are not UTF-8 and saying
//! they are would be a lie a reader acts on, and everything Arrow cannot say
//! rides the `yggdryl.string` extension document beside it.

use arrow_schema::DataType as ArrowDataType;

use super::{StringLayout, StringParameters};
use crate::{Charset, Error, Result};

/// Whether a string's bytes ride Arrow's text layouts rather than its binary
/// ones.
///
/// The one owner of that fact: the projection below, the column writer, the
/// cell reader, the digest feed and the cast planner all ask here. UTF-8 is
/// Arrow's own text, and US-ASCII is a subset of it.
pub(crate) const fn is_text_storage(parameters: StringParameters) -> bool {
    matches!(parameters.charset(), Charset::Utf8 | Charset::Ascii)
}

/// Whether a string field needs the `yggdryl.string` document beside its
/// storage.
///
/// Only where Arrow cannot say what the string declares: a charset other
/// than UTF-8, a bound, or the large view layout, which Arrow projects onto
/// its one view. Plain `utf8`, `large_utf8` and `utf8_view` are Arrow's own
/// datatypes and cross bare.
pub(crate) const fn needs_extension(parameters: StringParameters) -> bool {
    !parameters.charset().is_utf8()
        || parameters.is_bounded()
        || matches!(parameters.layout(), StringLayout::LargeStringView)
}

/// The Arrow storage one string datatype lays out.
///
/// # Errors
///
/// Returns [`Error::InvalidDataType`] when a fixed width is outside `i32`,
/// which is as wide as Arrow's own fixed binary counts.
pub(crate) fn arrow_storage(parameters: StringParameters) -> Result<ArrowDataType> {
    // The variant is public, so a fixed string can arrive here without the
    // width that makes it fixed. A boundary is where that stops.
    parameters.validate()?;
    // A fixed width is one layout in Arrow whatever the charset: Arrow has no
    // fixed-width string, so the bytes ride its fixed binary and the charset
    // travels beside them.
    if let Some(width) = parameters.fixed() {
        let width = i32::try_from(width).map_err(|_| Error::InvalidDataType {
            kind: "string",
            reason: smol_str::format_smolstr!(
                "fixed width {width} is outside the i32 range Arrow counts in"
            ),
        })?;
        return Ok(ArrowDataType::FixedSizeBinary(width));
    }
    let text = is_text_storage(parameters);
    Ok(match (parameters.layout(), text) {
        (StringLayout::String, true) => ArrowDataType::Utf8,
        (StringLayout::String, false) => ArrowDataType::Binary,
        (StringLayout::LargeString, true) => ArrowDataType::LargeUtf8,
        (StringLayout::LargeString, false) => ArrowDataType::LargeBinary,
        // Arrow has one view layout, so both of this crate's project onto it
        // and the `large` half of the distinction rides the metadata.
        (StringLayout::StringView | StringLayout::LargeStringView, true) => ArrowDataType::Utf8View,
        (StringLayout::StringView | StringLayout::LargeStringView, false) => {
            ArrowDataType::BinaryView
        }
        // The fixed layout answered above: `validate` gave it a width and
        // the width gave it its storage.
        (StringLayout::FixedString, _) => ArrowDataType::Binary,
    })
}

/// Whether one Arrow storage is what these parameters lay out.
///
/// The import side asks this rather than re-deriving: a `yggdryl.string`
/// document over a storage it does not describe is a foreign field wearing
/// our name, and it imports as its storage instead.
pub(crate) fn describes_storage(
    parameters: StringParameters,
    storage: &ArrowDataType,
) -> Result<bool> {
    Ok(arrow_storage(parameters)? == *storage)
}
