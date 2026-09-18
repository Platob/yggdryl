//! The byte datatype family: one layout, one length bound.
//!
//! Arrow has four binary layouts and no way to bound one: a `Binary` array
//! declares 32-bit offsets and nothing else, and `varbinary(n)` exists
//! nowhere in the format. This family is the one place this crate answers
//! both questions - which layout, how long - and every byte column the crate
//! has is one member of it.
//!
//! [`BytesLayout`] names the four layouts. [`BytesType`] is a layout
//! beside the bound its values are held to. [`crate::DataType::bytes`] builds
//! the one byte datatype, [`crate::DataType::Bytes`], from them; `binary`,
//! `varbinary(16)` and `fixed_size_binary(16)` are spellings of it, never
//! datatypes of their own. [`Bytes`] is the one byte value.
//!
//! ```
//! use yggdryl::types::{BytesLayout, BytesType};
//! use yggdryl::DataType;
//!
//! # fn main() -> yggdryl::Result<()> {
//! // Every spelling is one datatype, and it reads back as itself.
//! assert_eq!(DataType::from_str("bytes")?, DataType::binary());
//! assert_eq!(DataType::from_str("fixed_binary(16)")?.to_string(), "fixed_size_binary(16)");
//!
//! // The layout and the bound are what a byte column declares.
//! let bounded = DataType::from_str("varbinary(32)")?;
//! let parameters = bounded.bytes_parameters().expect("a byte datatype");
//! assert_eq!(parameters.layout(), BytesLayout::Binary);
//! assert_eq!(parameters.max(), Some(32));
//! assert_eq!(bounded.to_string(), "binary(32)");
//! # Ok(())
//! # }
//! ```

use std::borrow::{Borrow, Cow};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::num::NonZeroU32;
use std::ops::Deref;
use std::sync::Arc;

pub(crate) use arrow::{arrow_storage, describes_storage, from_arrow_storage, needs_extension};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::{SmolStr, format_smolstr};

use crate::types::Scalar;
use crate::types::parser::Parser;
use crate::{DataType, DataTypeId, Error, Result, Value};

/// What a byte datatype lays out in Arrow, and what it reads back from.
///
/// The four layouts are Arrow's own, so the storage is the layout. What Arrow
/// cannot say - a maximum on a variable layout - rides the `yggdryl.bytes`
/// extension document beside it; a fixed width is the storage itself and
/// needs no document.
mod arrow {
    use arrow_schema::DataType as ArrowDataType;

    use super::{BytesLayout, BytesType};
    use crate::{Error, Result};

    /// Whether a byte field needs the `yggdryl.bytes` document beside its
    /// storage: only a maximum, which no Arrow layout can state.
    pub(crate) const fn needs_extension(parameters: BytesType) -> bool {
        parameters.is_bounded() && !parameters.is_fixed()
    }

    /// The Arrow storage one byte datatype lays out.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] when a fixed width is outside `i32`,
    /// which is as wide as Arrow's own fixed binary counts.
    pub(crate) fn arrow_storage(parameters: BytesType) -> Result<ArrowDataType> {
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
        parameters: BytesType,
        storage: &ArrowDataType,
    ) -> Result<bool> {
        Ok(arrow_storage(parameters)? == *storage)
    }

    /// The width an Arrow fixed binary declares, as the count this crate
    /// bounds bytes in. Arrow's field is signed; a negative width is no width.
    ///
    /// # Errors
    ///
    /// Returns an error naming the width when it is negative.
    pub(crate) fn arrow_fixed_width(width: i32) -> Result<u32> {
        u32::try_from(width).map_err(|_| {
            crate::types::invalid(
                "bytes",
                smol_str::format_smolstr!("width must be non-negative: {width}"),
            )
        })
    }

    /// The byte datatype one Arrow storage imports as.
    ///
    /// A bound is a `yggdryl.bytes` document on the field, so a bare storage
    /// imports as the unbounded layout it is; the field level puts the bound
    /// back when the document is there.
    ///
    /// # Errors
    ///
    /// Returns an error when a fixed width is negative, or when the storage
    /// belongs to another family.
    pub(crate) fn from_arrow_storage(value: &ArrowDataType) -> Result<crate::DataType> {
        match value {
            ArrowDataType::Binary => Ok(crate::DataType::binary()),
            ArrowDataType::LargeBinary => Ok(crate::DataType::large_binary()),
            ArrowDataType::BinaryView => Ok(crate::DataType::binary_view()),
            ArrowDataType::FixedSizeBinary(width) => {
                crate::DataType::fixed_size_binary(arrow_fixed_width(*width)?)
            }
            other => Err(crate::types::invalid(
                "bytes",
                smol_str::format_smolstr!("expected a byte storage, got {other}"),
            )),
        }
    }
}
/// Binary layout accounting and identity checks for Arrow casts.
pub(crate) mod casts {
    use crate::types::enums::EnumType;
    use arrow_array::builder::{
        BinaryBuilder, BinaryViewBuilder, FixedSizeBinaryBuilder, LargeBinaryBuilder,
    };
    use arrow_array::types::{
        Int8Type, Int16Type, Int32Type, Int64Type, UInt8Type, UInt16Type, UInt32Type, UInt64Type,
    };
    use arrow_array::{
        Array, BinaryArray, BinaryViewArray, DictionaryArray, FixedSizeBinaryArray, Int16RunArray,
        Int32RunArray, Int64RunArray, LargeBinaryArray, LargeStringArray, StringArray, StringViewArray,
        UnionArray,
    };

    use std::sync::Arc;

    use arrow_array::ArrayRef;
    use arrow_buffer::BooleanBuffer;
    use arrow_cast::can_cast_types;
    use arrow_schema::DataType as ArrowDataType;
    use smol_str::{SmolStr, format_smolstr};

    use crate::arrow::{Error, Result};
    use crate::types::budget::MaterializationBudget;
    use crate::types::bytes::{BytesLayout, BytesType};
    use crate::types::cast::{arrow_cast_exposed, downcast, internal_target_error, named_cell};
    use crate::types::cast::columns::{is_exposed, null_buffers_ptr_eq};
    use crate::{DataType, Field};
    use crate::types::DecimalType;

    /// Whether one byte layout reaches another only through Arrow's `Binary`.
    ///
    /// Arrow's kernel reads every variable byte layout as every other one, but it
    /// has no direct reading between a fixed binary and text, nor between a binary
    /// view and a fixed binary. Both of those are the same payload under two
    /// framings, and `Binary` is the framing both sides already convert to, so the
    /// reading exists - it just takes two hops instead of one.
    pub(crate) fn bridges_through_binary(source: &ArrowDataType, target: &ArrowDataType) -> bool {
        is_byte_layout(source)
            && is_byte_layout(target)
            && !can_cast_types(source, target)
            && can_cast_types(source, &ArrowDataType::Binary)
            && can_cast_types(&ArrowDataType::Binary, target)
    }

    /// Whether an Arrow layout stores one byte payload per row.
    fn is_byte_layout(dtype: &ArrowDataType) -> bool {
        matches!(
            dtype,
            ArrowDataType::Binary
                | ArrowDataType::LargeBinary
                | ArrowDataType::BinaryView
                | ArrowDataType::FixedSizeBinary(_)
                | ArrowDataType::Utf8
                | ArrowDataType::LargeUtf8
                | ArrowDataType::Utf8View
        )
    }

    /// A byte source under the one variable framing every value reader takes.
    ///
    /// A payload is one payload under all four binary framings; only the offsets
    /// differ. A reader that validates values - an ASCII width, a code, a UUID -
    /// takes those bytes directly, because pushing them through a text temporary
    /// first turns a payload that is not UTF-8 into a null instead of the refusal
    /// the value rule owes it. A text source is not bytes and answers `None`, so
    /// it keeps the text path it always had, and a fixed binary answers `None` too
    /// because its reader already knows the width it carries.
    pub(crate) fn variable_binary_source(
        array: &ArrayRef,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<Option<ArrayRef>> {
        match array.data_type() {
            ArrowDataType::Binary => Ok(Some(Arc::clone(array))),
            ArrowDataType::LargeBinary | ArrowDataType::BinaryView => Ok(Some(arrow_cast_exposed(
                array,
                &ArrowDataType::Binary,
                false,
                exposure,
                &Field::new(field.name(), DataType::binary(), true),
                budget,
            )?)),
            _ => Ok(None),
        }
    }

    /// Validates every exposed, non-null value entering a bounded byte datatype
    /// and stores it in the target's own layout.
    ///
    /// A maximum is the one thing about bytes Arrow cannot check, so it is the
    /// one byte cast that reads cells rather than buffers: a fixed binary is
    /// read as it is, every variable layout through the one `Binary` framing,
    /// and anything else through the kernel's own reading into that framing. A
    /// cell past the maximum is null under `safe` and an error naming the row
    /// otherwise; nothing is copied until every cell has been measured.
    pub(crate) fn ingest_bytes_array(
        array: &ArrayRef,
        safe: bool,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        let Some(target) = field.dtype().bytes_parameters() else {
            return Err(internal_target_error("bytes"));
        };
        if let ArrowDataType::FixedSizeBinary(_) = array.data_type() {
            let cells = downcast::<FixedSizeBinaryArray>(array.as_ref())?;
            return bytes_storage(
                target,
                field,
                cells.len(),
                safe,
                exposure,
                budget,
                |index| cells.is_valid(index).then(|| cells.value(index)),
            );
        }
        let bytes = match variable_binary_source(array, field, exposure, budget)? {
            Some(bytes) => bytes,
            // The temporary is nullable bytes: the kernel's masked path fills
            // nothing, and the target's own null policy runs after the reading.
            None => arrow_cast_exposed(
                array,
                &ArrowDataType::Binary,
                safe,
                exposure,
                &Field::new(field.name(), DataType::binary(), true),
                budget,
            )?,
        };
        let cells = downcast::<BinaryArray>(bytes.as_ref())?;
        bytes_storage(
            target,
            field,
            cells.len(),
            safe,
            exposure,
            budget,
            |index| cells.is_valid(index).then(|| cells.value(index)),
        )
    }

    /// Builds the storage of one bounded byte layout from one cell per row.
    ///
    /// Unexposed rows are null: an ancestor hides them, so their bytes are
    /// neither measured nor copied. A maximum takes every payload up to it; a
    /// fixed width takes only the payload that fills it exactly, because a slot
    /// is the value rather than a frame around it. Either way the refusal names
    /// the field and the row, which is what Arrow's own builder complaint does
    /// not.
    fn bytes_storage<'a>(
        target: BytesType,
        field: &Field,
        rows: usize,
        safe: bool,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
        cell: impl Fn(usize) -> Option<&'a [u8]>,
    ) -> Result<ArrayRef> {
        let Some(bound) = target.bound() else {
            return Err(internal_target_error("bytes"));
        };
        let exact = target.is_fixed();
        budget.add_array(field.dtype(), rows)?;
        let mut payload = 0_usize;
        let accepted = |index: usize| -> Result<Option<&'a [u8]>> {
            let Some(bytes) = cell(index).filter(|_| is_exposed(exposure, index)) else {
                return Ok(None);
            };
            let fits = match exact {
                true => bytes.len() == bound as usize,
                false => bytes.len() <= bound as usize,
            };
            if fits {
                return Ok(Some(bytes));
            }
            if safe {
                return Ok(None);
            }
            named_cell(
                field,
                index,
                Err(crate::Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: crate::text::expected_got(
                        format_args!(
                            "{} {bound} bytes",
                            match exact {
                                true => "exactly",
                                false => "at most",
                            }
                        ),
                        format_smolstr!("{} bytes", bytes.len()),
                    ),
                }),
            )
        };
        for index in 0..rows {
            payload = payload.saturating_add(accepted(index)?.map_or(0, <[u8]>::len));
        }
        budget.add_bytes(payload)?;
        macro_rules! filled {
            ($builder:expr) => {{
                let mut builder = $builder;
                for index in 0..rows {
                    match accepted(index)? {
                        Some(bytes) => builder.append_value(bytes),
                        None => builder.append_null(),
                    }
                }
                Arc::new(builder.finish()) as ArrayRef
            }};
        }
        Ok(match target.layout() {
            BytesLayout::Binary => filled!(BinaryBuilder::with_capacity(rows, payload)),
            BytesLayout::LargeBinary => filled!(LargeBinaryBuilder::with_capacity(rows, payload)),
            // Arrow's view layout carries a prefix per cell rather than offsets,
            // so it takes the row count and grows its own payload blocks.
            BytesLayout::BinaryView => filled!(BinaryViewBuilder::with_capacity(rows)),
            // Every accepted cell is exactly the width, which is what the builder
            // needs; a cell that is not was refused or nulled above. Its
            // `append_value` answers a `Result` the other three do not, so this
            // arm is written out rather than bent through the macro.
            BytesLayout::FixedSizeBinary => {
                let width = i32::try_from(bound).map_err(|_| internal_target_error("bytes"))?;
                let mut builder = FixedSizeBinaryBuilder::with_capacity(rows, width);
                for index in 0..rows {
                    match accepted(index)? {
                        Some(bytes) => builder.append_value(bytes)?,
                        None => builder.append_null(),
                    }
                }
                Arc::new(builder.finish()) as ArrayRef
            }
        })
    }

    pub(crate) fn projected_byte_len(
        array: &dyn Array,
        source_type: &DataType,
        index: usize,
    ) -> Result<usize> {
        if index >= array.len() {
            return Err(Error::IncompatibleSchema(
                "Arrow byte projection index exceeds its source array".to_owned(),
            ));
        }
        if array.is_null(index)
            && !matches!(
                source_type,
                DataType::Enum(EnumType::Dictionary(_)) | DataType::Union(..) | DataType::RunEndEncoded(_)
            )
        {
            return Ok(0);
        }
        let bytes = match source_type {
            // A byte payload is measured by the framing the array is in: a
            // string, a code and a UUID each project onto one of the byte
            // layouts, and the array says which.
            bytes if bytes.kind().is_bytes() || matches!(bytes, DataType::Uuid) => {
                byte_cell_len(array, index)?
            }
            DataType::Enum(EnumType::Dictionary(dictionary)) => {
                macro_rules! dictionary_len {
                    ($key:ty) => {{
                        let dictionary_array = downcast::<DictionaryArray<$key>>(array)?;
                        if dictionary_array.keys().is_null(index) {
                            0
                        } else {
                            let key = usize::try_from(dictionary_array.keys().value(index)).map_err(
                                |_| {
                                    Error::IncompatibleSchema(
                                        "Arrow dictionary key is negative or exceeds usize".to_owned(),
                                    )
                                },
                            )?;
                            projected_byte_len(
                                dictionary_array.values().as_ref(),
                                dictionary.value(),
                                key,
                            )?
                        }
                    }};
                }
                match dictionary.key() {
                    DataType::Int8 => dictionary_len!(Int8Type),
                    DataType::Int16 => dictionary_len!(Int16Type),
                    DataType::Int32 => dictionary_len!(Int32Type),
                    DataType::Int64 => dictionary_len!(Int64Type),
                    DataType::UInt8 => dictionary_len!(UInt8Type),
                    DataType::UInt16 => dictionary_len!(UInt16Type),
                    DataType::UInt32 => dictionary_len!(UInt32Type),
                    DataType::UInt64 => dictionary_len!(UInt64Type),
                    _ => {
                        return Err(Error::IncompatibleSchema(
                            "Arrow dictionary byte projection key is not an integer".to_owned(),
                        ));
                    }
                }
            }
            DataType::Union(fields, _) => {
                let union = downcast::<UnionArray>(array)?;
                let type_id = union.type_id(index);
                let (_, field) = fields
                    .iter()
                    .find(|(candidate, _)| *candidate == type_id)
                    .ok_or_else(|| {
                        Error::IncompatibleSchema(format!(
                            "Arrow union byte projection has unknown type ID {type_id}"
                        ))
                    })?;
                projected_byte_len(
                    union.child(type_id).as_ref(),
                    field.dtype(),
                    union.value_offset(index),
                )?
            }
            DataType::RunEndEncoded(encoded) => match encoded.run_ends().dtype() {
                DataType::Int16 => {
                    let run = downcast::<Int16RunArray>(array)?;
                    projected_byte_len(
                        run.values().as_ref(),
                        encoded.values().dtype(),
                        run.get_physical_index(index),
                    )?
                }
                DataType::Int32 => {
                    let run = downcast::<Int32RunArray>(array)?;
                    projected_byte_len(
                        run.values().as_ref(),
                        encoded.values().dtype(),
                        run.get_physical_index(index),
                    )?
                }
                DataType::Int64 => {
                    let run = downcast::<Int64RunArray>(array)?;
                    projected_byte_len(
                        run.values().as_ref(),
                        encoded.values().dtype(),
                        run.get_physical_index(index),
                    )?
                }
                _ => {
                    return Err(Error::IncompatibleSchema(
                        "Arrow run-end byte projection type is invalid".to_owned(),
                    ));
                }
            },
            DataType::Boolean | DataType::UInt16 => 5,
            DataType::Int8 => 4,
            DataType::UInt8 => 3,
            DataType::Int16 => 6,
            DataType::Int32 | DataType::Decimal(DecimalType::Decimal32 { .. }) => 12,
            DataType::UInt32 => 10,
            DataType::Int64 | DataType::Decimal(DecimalType::Decimal64 { .. }) => 21,
            DataType::UInt64 => 20,
            DataType::Float16 => 16,
            DataType::Float32 => 24,
            DataType::Float64 => 32,
            DataType::Decimal(DecimalType::Decimal128 { .. }) => 41,
            DataType::Decimal(DecimalType::Decimal256 { .. }) => 78,
            DataType::DateTime64 { .. }
            | DataType::Date32
            | DataType::Date64
            | DataType::Time32(_)
            | DataType::Time64(_)
            | DataType::Duration32(_)
            | DataType::Duration64(_)
            | DataType::Interval(_) => 128,
            _ => 0,
        };
        Ok(bytes)
    }

    /// The bytes one cell holds, under whichever byte framing the array is in.
    fn byte_cell_len(array: &dyn Array, index: usize) -> Result<usize> {
        Ok(match array.data_type() {
            ArrowDataType::Binary => downcast::<BinaryArray>(array)?.value(index).len(),
            ArrowDataType::LargeBinary => downcast::<LargeBinaryArray>(array)?.value(index).len(),
            ArrowDataType::BinaryView => downcast::<BinaryViewArray>(array)?.value(index).len(),
            ArrowDataType::FixedSizeBinary(_) => {
                downcast::<FixedSizeBinaryArray>(array)?.value(index).len()
            }
            ArrowDataType::Utf8 => downcast::<StringArray>(array)?.value(index).len(),
            ArrowDataType::LargeUtf8 => downcast::<LargeStringArray>(array)?.value(index).len(),
            ArrowDataType::Utf8View => downcast::<StringViewArray>(array)?.value(index).len(),
            other => {
                return Err(Error::IncompatibleSchema(format!(
                    "Arrow byte projection over a {other:?} array, which holds no byte payload"
                )));
            }
        })
    }

    pub(crate) fn checked_valid_payload_bytes(
        len: usize,
        mut is_valid: impl FnMut(usize) -> bool,
        mut value_len: impl FnMut(usize) -> usize,
    ) -> Result<usize> {
        (0..len).try_fold(0usize, |bytes, index| {
            if !is_valid(index) {
                return Ok(bytes);
            }
            bytes
                .checked_add(value_len(index))
                .ok_or_else(|| Error::IncompatibleSchema("Arrow payload bytes exceed usize".to_owned()))
        })
    }

    #[allow(clippy::too_many_lines)] // Mirrors every nested Arrow container layout.
    pub(crate) fn byte_array_storage_ptr_eq(
        left: &dyn Array,
        right: &dyn Array,
        dtype: &DataType,
    ) -> Result<bool> {
        macro_rules! shared {
            ($array:ty) => {{
                let left = downcast::<$array>(left)?;
                let right = downcast::<$array>(right)?;
                left.offsets().ptr_eq(right.offsets())
                    && byte_slices_ptr_eq(left.value_data(), right.value_data())
                    && null_buffers_ptr_eq(left.nulls(), right.nulls())
            }};
        }
        if !(dtype.kind().is_bytes() || matches!(dtype, DataType::Uuid)) {
            return Ok(false);
        }
        // The framing is the array's: a string, a code and a UUID each project
        // onto one of the byte layouts, and a view layout owns no offsets to
        // compare.
        Ok(match left.data_type() {
            ArrowDataType::Binary => shared!(BinaryArray),
            ArrowDataType::LargeBinary => shared!(LargeBinaryArray),
            ArrowDataType::Utf8 => shared!(StringArray),
            ArrowDataType::LargeUtf8 => shared!(LargeStringArray),
            ArrowDataType::FixedSizeBinary(_) => {
                let left = downcast::<FixedSizeBinaryArray>(left)?;
                let right = downcast::<FixedSizeBinaryArray>(right)?;
                byte_slices_ptr_eq(left.value_data(), right.value_data())
                    && null_buffers_ptr_eq(left.nulls(), right.nulls())
            }
            _ => false,
        })
    }

    fn byte_slices_ptr_eq(left: &[u8], right: &[u8]) -> bool {
        left.len() == right.len() && (left.is_empty() || std::ptr::eq(left.as_ptr(), right.as_ptr()))
    }
}

// ------------------------------------------------------------------------
// The one door into the byte family, and the questions every byte column
// answers.
// ------------------------------------------------------------------------

impl DataType {
    /// The byte datatype these parameters name.
    ///
    /// This is the family's one constructor. Every byte column is
    /// [`DataType::Bytes`]; what differs is what it declares, and the
    /// parameters say all of it - a layout and a bound.
    ///
    /// ```
    /// use yggdryl::types::{BytesLayout, BytesType};
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(DataType::bytes(BytesLayout::LargeBinary)?, DataType::large_binary());
    /// let bounded = BytesType::new(BytesLayout::Binary).try_with_bound(32)?;
    /// assert_eq!(DataType::bytes(bounded)?.to_string(), "binary(32)");
    /// assert_eq!(DataType::fixed_size_binary(16)?.to_string(), "fixed_size_binary(16)");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] for the fixed layout with no
    /// width: the width is what makes it fixed.
    pub fn bytes(parameters: impl Into<BytesType>) -> Result<Self> {
        let parameters = parameters.into();
        parameters.validate()?;
        Ok(Self::Bytes(parameters))
    }

    /// Unbounded bytes with 32-bit offsets - Arrow's `Binary`.
    #[must_use]
    pub const fn binary() -> Self {
        Self::Bytes(BytesType::new(BytesLayout::Binary))
    }

    /// Unbounded bytes with 64-bit offsets - Arrow's `LargeBinary`.
    #[must_use]
    pub const fn large_binary() -> Self {
        Self::Bytes(BytesType::new(BytesLayout::LargeBinary))
    }

    /// Unbounded bytes in the view layout - Arrow's `BinaryView`.
    #[must_use]
    pub const fn binary_view() -> Self {
        Self::Bytes(BytesType::new(BytesLayout::BinaryView))
    }

    /// Exactly `width` bytes per value - Arrow's `FixedSizeBinary`.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(DataType::fixed_size_binary(16)?.fixed_byte_width(), Some(16));
    /// assert!(DataType::fixed_size_binary(0).is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] for a width of zero.
    pub fn fixed_size_binary(width: u32) -> Result<Self> {
        Self::bytes(BytesType::new(BytesLayout::FixedSizeBinary).try_with_bound(width)?)
    }

    /// The parameters a byte datatype declares, `None` for every other.
    ///
    /// A UUID and a geospatial value are bytes with an identity rather than
    /// byte columns, so they answer `None` here exactly as a code answers no
    /// [`Self::string_parameters`].
    #[must_use]
    pub const fn bytes_parameters(&self) -> Option<BytesType> {
        match self {
            Self::Bytes(parameters) => Some(*parameters),
            _ => None,
        }
    }
}

// ------------------------------------------------------------------------
// The byte family's field marker.
// ------------------------------------------------------------------------


// ------------------------------------------------------------------------
// What a byte datatype declares beyond its layout.
// ------------------------------------------------------------------------

/// The name this crate's bounded byte datatypes ride Arrow under.
pub const BYTES_EXTENSION_NAME: &str = "yggdryl.bytes";

/// A byte layout and its bound.
///
/// Arrow carries the layout and, for the fixed one, the width; it has no
/// `varbinary(n)`. So the maximum rides here, and crosses an Arrow boundary
/// as this crate's own extension metadata under `yggdryl.bytes`.
///
/// One number carries both bounds because a column is one shape or the
/// other: on [`BytesLayout::FixedSizeBinary`] it is the exact width every
/// value fills, and on every other layout it is the most bytes a value may
/// hold. So [`Self::fixed`] and [`Self::max`] are two readings of one fact,
/// and exactly one of them ever answers.
///
/// ```
/// use yggdryl::types::{BytesLayout, BytesType};
/// use yggdryl::DataType;
///
/// # fn main() -> yggdryl::Result<()> {
/// let parameters = BytesType::new(BytesLayout::Binary).try_with_bound(32)?;
/// assert_eq!(parameters.max(), Some(32));
/// assert_eq!(parameters.fixed(), None);
///
/// let dtype = DataType::bytes(parameters)?;
/// assert_eq!(dtype.to_string(), "binary(32)");
/// assert_eq!(DataType::from_str(&dtype.to_string())?, dtype);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BytesType {
    layout: BytesLayout,
    bound: Option<NonZeroU32>,
}

impl BytesType {
    /// Unbounded bytes in one layout.
    #[must_use]
    pub const fn new(layout: BytesLayout) -> Self {
        Self {
            layout,
            bound: None,
        }
    }

    /// The layout the values are stored in.
    #[must_use]
    pub const fn layout(self) -> BytesLayout {
        self.layout
    }

    /// The declared byte bound, whichever shape the layout gives it.
    #[must_use]
    pub const fn bound(self) -> Option<u32> {
        match self.bound {
            Some(bound) => Some(bound.get()),
            None => None,
        }
    }

    /// The exact bytes every value fills, on the fixed layout.
    #[must_use]
    pub const fn fixed(self) -> Option<u32> {
        match self.layout.is_fixed() {
            true => self.bound(),
            false => None,
        }
    }

    /// The most bytes a value may hold, on a variable layout.
    #[must_use]
    pub const fn max(self) -> Option<u32> {
        match self.layout.is_fixed() {
            true => None,
            false => self.bound(),
        }
    }

    /// Return whether every value is the same width.
    #[must_use]
    pub const fn is_fixed(self) -> bool {
        self.layout.is_fixed()
    }

    /// Return whether the values are bounded at all.
    #[must_use]
    pub const fn is_bounded(self) -> bool {
        self.bound.is_some()
    }

    /// Return these parameters in another layout.
    #[must_use]
    pub const fn with_layout(mut self, layout: BytesLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Return these parameters bounded to `bound` bytes, the bound already
    /// proven non-zero.
    #[must_use]
    pub const fn with_bound(mut self, bound: NonZeroU32) -> Self {
        self.bound = Some(bound);
        self
    }

    /// Return these parameters bounded to `bound` bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a bound of zero: a column of
    /// no bytes is a column of one value, which is a declaration nobody
    /// means.
    pub fn try_with_bound(mut self, bound: u32) -> Result<Self> {
        self.bound = Some(NonZeroU32::new(bound).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a {} of at least one byte, got 0",
                self.bound_word()
            ))
        })?);
        Ok(self)
    }

    /// Return these parameters with no bound.
    #[must_use]
    pub const fn without_bound(mut self) -> Self {
        self.bound = None;
        self
    }

    /// Return these parameters with no maximum, keeping a fixed width.
    ///
    /// A maximum is a column's rule and a fixed width is a value's shape, so
    /// this is what a value carries out of a bounded column.
    #[must_use]
    pub const fn without_max(self) -> Self {
        match self.layout.is_fixed() {
            true => self,
            false => self.without_bound(),
        }
    }

    /// Check that the layout and the bound agree.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for the fixed layout with no width:
    /// the width is what makes it fixed, so there is no width-free spelling
    /// of it.
    pub fn validate(self) -> Result<()> {
        if self.layout.is_fixed() && self.bound.is_none() {
            return Err(invalid(format_smolstr!(
                "expected {}(width), got no width",
                self.layout.as_str()
            )));
        }
        Ok(())
    }

    /// The word this layout calls its bound by.
    const fn bound_word(self) -> &'static str {
        match self.layout.is_fixed() {
            true => "width",
            false => "maximum",
        }
    }

    /// The metadata key the bound is written under.
    const fn bound_word_key(self) -> &'static str {
        match self.layout.is_fixed() {
            true => "fixed",
            false => "max",
        }
    }

    /// The extension metadata an Arrow field carries these in.
    ///
    /// Arrow has nowhere to put a maximum: the four binary layouts declare
    /// their offsets and, for the fixed one, the width, and nothing else. So
    /// a maximum rides the `ARROW:extension:metadata` document beside the
    /// `yggdryl.bytes` name, with the layout written whole so a reader can
    /// check it against the storage.
    #[must_use]
    pub fn extension_json(self) -> String {
        let mut rendered = String::with_capacity(48);
        rendered.push_str("{\"layout\":\"");
        rendered.push_str(self.layout.as_str());
        rendered.push('"');
        if let Some(bound) = self.bound {
            rendered.push(',');
            rendered.push('"');
            rendered.push_str(self.bound_word_key());
            rendered.push_str("\":");
            rendered.push_str(&format_smolstr!("{bound}"));
        }
        rendered.push('}');
        rendered
    }

    /// Read parameters back out of Arrow extension metadata.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] when the document is not an object
    /// naming a layout this crate knows, or bounds a layout the wrong way.
    pub fn from_extension_json(value: &str) -> Result<Self> {
        #[derive(Deserialize)]
        struct Document {
            layout: SmolStr,
            #[serde(default)]
            max: Option<u32>,
            #[serde(default)]
            fixed: Option<u32>,
        }

        let document: Document = serde_json::from_str(value)
            .map_err(|error| invalid(format_smolstr!("expected bytes parameters, got {error}")))?;
        let layout = BytesLayout::from_str(&document.layout)
            .map_err(|error| invalid(format_smolstr!("{error}")))?;
        let mut parameters = Self::new(layout);
        match (layout.is_fixed(), document.fixed, document.max) {
            (true, Some(fixed), None) => parameters = parameters.try_with_bound(fixed)?,
            (false, None, Some(max)) => parameters = parameters.try_with_bound(max)?,
            (_, None, None) => {}
            (true, _, Some(max)) => {
                return Err(invalid(format_smolstr!(
                    "expected a fixed width on {layout}, got max={max}"
                )));
            }
            (false, Some(fixed), _) => {
                return Err(invalid(format_smolstr!(
                    "expected a maximum on {layout}, got fixed={fixed}"
                )));
            }
        }
        parameters.validate()?;
        Ok(parameters)
    }
}

/// The refusal every invalid parameter answers with.
fn invalid(reason: impl Into<SmolStr>) -> Error {
    Error::InvalidDataType {
        kind: "bytes",
        reason: reason.into(),
    }
}

impl Default for BytesType {
    fn default() -> Self {
        Self::new(BytesLayout::Binary)
    }
}

impl From<BytesLayout> for BytesType {
    fn from(value: BytesLayout) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for BytesType {
    /// The canonical spelling, which [`crate::DataType`]'s grammar reads back.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.layout.as_str())?;
        match self.bound {
            None => Ok(()),
            Some(bound) => write!(formatter, "({bound})"),
        }
    }
}

impl Serialize for BytesType {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        let declared = usize::from(self.bound.is_some());
        let mut state = serializer.serialize_struct("BytesType", 1 + declared)?;
        state.serialize_field("layout", &self.layout)?;
        if let Some(bound) = self.bound {
            state.serialize_field(self.bound_word_key(), &bound.get())?;
        }
        state.end()
    }
}

impl<'de> Deserialize<'de> for BytesType {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Representation {
            layout: BytesLayout,
            #[serde(default)]
            max: Option<u32>,
            #[serde(default)]
            fixed: Option<u32>,
        }

        let value = Representation::deserialize(deserializer)?;
        let mut parameters = Self::new(value.layout);
        if let Some(bound) = value.fixed.or(value.max) {
            parameters = parameters
                .try_with_bound(bound)
                .map_err(serde::de::Error::custom)?;
        }
        parameters.validate().map_err(serde::de::Error::custom)?;
        Ok(parameters)
    }
}

// ------------------------------------------------------------------------
// The one grammar every byte spelling reads through.
// ------------------------------------------------------------------------

impl Parser<'_> {
    /// Parse one byte layout's optional bound.
    ///
    /// The parameter list is at most one number long, and which bound it is
    /// follows from the layout - the exact width on the fixed layout, the
    /// maximum on every other - so there is one number to write and one
    /// meaning it can have.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Parse`] for a bound outside a positive `u32`,
    /// and for the fixed layout with no width.
    pub(crate) fn parse_bytes(&mut self, layout: BytesLayout) -> Result<DataType> {
        let mut parameters = BytesType::new(layout);
        if let Some(close) = self.consume_opening() {
            // Empty parentheses are the bare spelling with punctuation.
            if !self.consume_symbol(close) {
                let position = self.current_position();
                let label = match layout.is_fixed() {
                    true => "a byte width",
                    false => "a maximum byte length",
                };
                let value = self.parse_integer(label)?;
                let bound = u32::try_from(value).map_err(|_| {
                    self.error_at(
                        position,
                        format_smolstr!("expected {label} inside u32, got {value}"),
                    )
                })?;
                parameters = parameters
                    .try_with_bound(bound)
                    .map_err(|error| self.error_at(position, format_smolstr!("{error}")))?;
                self.expect_symbol(close)?;
            }
        }
        let position = self.current_position();
        DataType::bytes(parameters)
            .map_err(|error| self.error_at(position, format_smolstr!("{error}")))
    }
}

// ------------------------------------------------------------------------
// The byte value: the payload, held compactly, beside the layout its column
// stores it under.
//
// [`Bytes`] is the one representation, and it is the crate's compact byte
// string: up to [`INLINE_BYTES`] bytes live inside the value with no heap
// behind them, a longer payload is one shared `Arc<[u8]>` that clones by
// reference count, and a `&'static [u8]` costs nothing at all. Equality,
// order and hashing read the payload alone - a value is one value whichever
// column holds it - and the parameters ride beside it. A maximum is the
// column's rule and never the value's: a value read out of `binary(32)` is
// a `binary`, exactly as an integer read out of a bounded column is an
// integer.
// ------------------------------------------------------------------------

/// How many bytes a [`Bytes`] holds without reaching the heap.
///
/// Thirty: the value has forty bytes to spend beside its parameters, and a
/// digest, a key, a UUID's sixteen bytes or a WKB point all fit under it and
/// never allocate.
pub const INLINE_BYTES: usize = 30;

/// One byte value: its payload and the parameters it is stored under.
///
/// ```
/// use yggdryl::types::{Bytes, BytesLayout, BytesType, INLINE_BYTES};
/// use yggdryl::{DataType, Scalar};
///
/// # fn main() -> yggdryl::Result<()> {
/// // A short payload lives inside the value; a longer one is one shared handle.
/// let short = Bytes::new([1_u8, 2, 3]);
/// assert!(short.is_inline());
/// let long = Bytes::new(vec![0_u8; INLINE_BYTES + 1]);
/// assert!(!long.is_inline());
///
/// // A value is one value whichever layout it is stored under.
/// let large = BytesType::new(BytesLayout::LargeBinary);
/// let restated = short.clone().try_with_parameters(large)?;
/// assert_eq!(restated, short);
/// assert_eq!(restated.dtype()?, DataType::large_binary());
///
/// // It is the crate's byte string, so it is the `Scalar` bytes too.
/// assert_eq!(Scalar::from(vec![1_u8, 2, 3]), Scalar::Bytes(short));
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct Bytes {
    repr: Repr,
    parameters: BytesType,
}

/// Where the payload is.
#[derive(Clone)]
enum Repr {
    /// The payload itself, copied into the value; bytes past `len` are zero.
    Inline { len: u8, bytes: [u8; INLINE_BYTES] },
    /// A payload the binary already holds.
    Static(&'static [u8]),
    /// A payload longer than the inline buffer, shared by reference count.
    Shared(Arc<[u8]>),
}

const _: () = assert!(std::mem::size_of::<Repr>() == 32);
const _: () = assert!(std::mem::size_of::<Bytes>() == 40);

impl Repr {
    /// Copy a payload into the value where it fits, else share it.
    fn owned(payload: &[u8]) -> Self {
        match Self::inline(payload) {
            Some(inline) => inline,
            None => Self::Shared(Arc::from(payload)),
        }
    }

    /// Copy a payload into the value, when it fits.
    fn inline(payload: &[u8]) -> Option<Self> {
        if payload.len() > INLINE_BYTES {
            return None;
        }
        let mut bytes = [0_u8; INLINE_BYTES];
        bytes[..payload.len()].copy_from_slice(payload);
        Some(Self::Inline {
            // The length fits the buffer, and the buffer fits a byte.
            len: payload.len() as u8,
            bytes,
        })
    }

    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Inline { len, bytes } => &bytes[..usize::from(*len)],
            Self::Static(bytes) => bytes,
            Self::Shared(bytes) => bytes,
        }
    }
}

impl Bytes {
    /// A payload the binary already holds, under the default parameters.
    ///
    /// Costs nothing: no copy, no count, so a constant payload is a
    /// constant value.
    #[must_use]
    pub const fn new_static(payload: &'static [u8]) -> Self {
        Self {
            repr: Repr::Static(payload),
            parameters: BytesType::new(BytesLayout::Binary),
        }
    }

    /// A byte value under the default parameters: the `binary` layout.
    ///
    /// A payload up to [`INLINE_BYTES`] long is copied into the value and
    /// allocates nothing; a longer one is one shared `Arc<[u8]>`.
    pub fn new(payload: impl AsRef<[u8]>) -> Self {
        Self {
            repr: Repr::owned(payload.as_ref()),
            parameters: BytesType::default(),
        }
    }

    /// A byte value adopting a handle a reader already holds.
    ///
    /// A payload that fits inline is copied and the handle dropped; a longer
    /// one is shared as it is, so nothing is copied twice.
    #[must_use]
    pub fn from_shared(payload: Arc<[u8]>) -> Self {
        let repr = match Repr::inline(&payload) {
            Some(inline) => inline,
            None => Repr::Shared(payload),
        };
        Self {
            repr,
            parameters: BytesType::default(),
        }
    }

    /// The payload a column's own storage holds, under the parameters it
    /// declares, checked when it was written and not again here.
    pub(crate) fn from_storage(payload: &[u8], parameters: BytesType) -> Self {
        Self {
            repr: Repr::owned(payload),
            parameters: parameters.without_max(),
        }
    }

    /// Restate the same payload under other parameters.
    ///
    /// The payload does not change and a shared handle is shared, not copied;
    /// what changes is the datatype [`Self::dtype`] declares. The bound is
    /// checked and, on a variable layout, not carried: it is the column's
    /// rule, and the value answers the layout alone. Bytes are never padded,
    /// so a fixed layout takes exactly its width and nothing shorter.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for the fixed layout with no width,
    /// and [`Error::InvalidRecord`] naming the bound and the payload length
    /// when the payload does not fit it.
    pub fn try_with_parameters(mut self, parameters: BytesType) -> Result<Self> {
        parameters.validate()?;
        let held = self.len();
        let refusal = |expected: std::fmt::Arguments<'_>| Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: crate::text::expected_got(expected, format_smolstr!("{held} bytes")),
        };
        if let Some(width) = parameters.fixed() {
            if held != width as usize {
                return Err(refusal(format_args!("exactly {width} bytes")));
            }
        } else if let Some(max) = parameters.max() {
            if held > max as usize {
                return Err(refusal(format_args!("at most {max} bytes")));
            }
        }
        self.parameters = parameters.without_max();
        Ok(self)
    }

    /// Borrow the payload.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.repr.as_bytes()
    }

    /// The parameters this value is stored under.
    ///
    /// Never a maximum: that is the column's declaration, and a value read
    /// out of a bounded column answers its layout alone.
    #[must_use]
    pub const fn parameters(&self) -> BytesType {
        self.parameters
    }

    /// The layout this value is stored in.
    #[must_use]
    pub const fn layout(&self) -> BytesLayout {
        self.parameters.layout()
    }

    /// The exact storage width, on the fixed layout alone.
    #[must_use]
    pub const fn fixed(&self) -> Option<u32> {
        self.parameters.fixed()
    }

    /// Whether the payload lives inside the value with no heap behind it.
    #[must_use]
    pub const fn is_inline(&self) -> bool {
        matches!(self.repr, Repr::Inline { .. })
    }

    /// Consume this value and return its payload as an owned vector.
    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        match self.repr {
            Repr::Inline { .. } | Repr::Static(_) => self.as_bytes().to_vec(),
            Repr::Shared(bytes) => bytes.to_vec(),
        }
    }

    /// Consume this value and return its payload as one shared handle.
    ///
    /// A shared value hands over its handle; the two other representations
    /// allocate one, since neither has a handle to give.
    #[must_use]
    pub fn into_shared(self) -> Arc<[u8]> {
        match self.repr {
            Repr::Shared(bytes) => bytes,
            Repr::Inline { .. } | Repr::Static(_) => Arc::from(self.as_bytes()),
        }
    }

    /// The datatype this value materializes into.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] only for parameters a constructor
    /// would have refused, which no door here builds.
    pub fn dtype(&self) -> Result<DataType> {
        DataType::bytes(self.parameters)
    }
}

impl Default for Bytes {
    fn default() -> Self {
        Self::new_static(&[])
    }
}

impl Deref for Bytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// Sound because [`Eq`], [`Ord`] and [`Hash`] read the payload alone, as a
/// slice's do; folding the parameters into any of them would break every
/// keyed lookup by slice.
impl Borrow<[u8]> for Bytes {
    fn borrow(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl fmt::Display for Bytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.as_bytes() {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Bytes {
    /// The payload in hex, and the parameters when they are not the default,
    /// so two values that compare equal but declare different columns print
    /// apart in a failing assertion.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "0x{self}")?;
        if self.parameters != BytesType::default() {
            write!(formatter, " as {}", self.parameters)?;
        }
        Ok(())
    }
}

impl PartialEq for Bytes {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for Bytes {}

impl PartialEq<[u8]> for Bytes {
    fn eq(&self, other: &[u8]) -> bool {
        self.as_bytes() == other
    }
}

impl PartialEq<&[u8]> for Bytes {
    fn eq(&self, other: &&[u8]) -> bool {
        self.as_bytes() == *other
    }
}

impl<const N: usize> PartialEq<[u8; N]> for Bytes {
    fn eq(&self, other: &[u8; N]) -> bool {
        self.as_bytes() == other
    }
}

impl<const N: usize> PartialEq<&[u8; N]> for Bytes {
    fn eq(&self, other: &&[u8; N]) -> bool {
        self.as_bytes() == *other
    }
}

impl PartialEq<Vec<u8>> for Bytes {
    fn eq(&self, other: &Vec<u8>) -> bool {
        self.as_bytes() == other.as_slice()
    }
}

impl PartialEq<Bytes> for [u8] {
    fn eq(&self, other: &Bytes) -> bool {
        self == other.as_bytes()
    }
}

impl PartialEq<Bytes> for Vec<u8> {
    fn eq(&self, other: &Bytes) -> bool {
        self.as_slice() == other.as_bytes()
    }
}

impl PartialOrd for Bytes {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Bytes {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

impl Hash for Bytes {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

impl From<&[u8]> for Bytes {
    fn from(value: &[u8]) -> Self {
        Self::new(value)
    }
}

impl<const N: usize> From<&[u8; N]> for Bytes {
    fn from(value: &[u8; N]) -> Self {
        Self::new(value)
    }
}

impl<const N: usize> From<[u8; N]> for Bytes {
    fn from(value: [u8; N]) -> Self {
        Self::new(value)
    }
}

impl From<Vec<u8>> for Bytes {
    fn from(value: Vec<u8>) -> Self {
        match Repr::inline(&value) {
            Some(repr) => Self {
                repr,
                parameters: BytesType::default(),
            },
            None => Self::from_shared(Arc::from(value)),
        }
    }
}

impl From<Box<[u8]>> for Bytes {
    fn from(value: Box<[u8]>) -> Self {
        match Repr::inline(&value) {
            Some(repr) => Self {
                repr,
                parameters: BytesType::default(),
            },
            None => Self::from_shared(Arc::from(value)),
        }
    }
}

impl From<Arc<[u8]>> for Bytes {
    fn from(value: Arc<[u8]>) -> Self {
        Self::from_shared(value)
    }
}

impl From<Cow<'_, [u8]>> for Bytes {
    fn from(value: Cow<'_, [u8]>) -> Self {
        match value {
            Cow::Borrowed(bytes) => Self::new(bytes),
            Cow::Owned(bytes) => Self::from(bytes),
        }
    }
}

impl From<Bytes> for Vec<u8> {
    fn from(value: Bytes) -> Self {
        value.into_vec()
    }
}

impl From<Bytes> for Arc<[u8]> {
    fn from(value: Bytes) -> Self {
        value.into_shared()
    }
}

impl FromIterator<u8> for Bytes {
    fn from_iter<I: IntoIterator<Item = u8>>(iter: I) -> Self {
        Self::from(iter.into_iter().collect::<Vec<u8>>())
    }
}

/// The serde representation of a byte value that declares more than its
/// payload.
///
/// The ordinary value - the `binary` layout - serializes its payload and
/// nothing else, exactly as it always has; a layout or a fixed width is what
/// makes a value carry more than that.
#[derive(Deserialize, Serialize)]
struct Declared<'a> {
    layout: BytesLayout,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fixed: Option<u32>,
    bytes: Cow<'a, [u8]>,
}

impl Serialize for Bytes {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        if self.parameters == BytesType::default() {
            return serializer.collect_seq(self.as_bytes());
        }
        Declared {
            layout: self.layout(),
            fixed: self.fixed(),
            bytes: Cow::Borrowed(self.as_bytes()),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Representation<'a> {
            Plain(Cow<'a, [u8]>),
            Declared(Declared<'a>),
        }

        match Representation::deserialize(deserializer)? {
            Representation::Plain(bytes) => Ok(Self::from(bytes)),
            Representation::Declared(declared) => {
                let mut parameters = BytesType::new(declared.layout);
                if let Some(width) = declared.fixed {
                    parameters = parameters
                        .try_with_bound(width)
                        .map_err(serde::de::Error::custom)?;
                }
                Self::from(declared.bytes)
                    .try_with_parameters(parameters)
                    .map_err(serde::de::Error::custom)
            }
        }
    }
}

impl Value for Bytes {
    fn dtype(&self) -> Result<DataType> {
        Self::dtype(self)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Bytes(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Bytes(value) => Some(value),
            _ => None,
        }
    }
}

impl From<Bytes> for Scalar {
    fn from(value: Bytes) -> Self {
        Self::Bytes(value)
    }
}

impl From<Vec<u8>> for Scalar {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(Bytes::from(value))
    }
}

impl From<&[u8]> for Scalar {
    fn from(value: &[u8]) -> Self {
        Self::Bytes(Bytes::new(value))
    }
}

impl<const N: usize> From<&[u8; N]> for Scalar {
    fn from(value: &[u8; N]) -> Self {
        Self::Bytes(Bytes::new(value))
    }
}

impl From<Arc<[u8]>> for Scalar {
    fn from(value: Arc<[u8]>) -> Self {
        Self::Bytes(Bytes::from_shared(value))
    }
}

/// The byte payload a value spells, shared rather than copied.
///
/// A byte column stores one payload per row and several kinds already hold
/// one: a byte value is cloned, a geospatial value shares its handle, and a
/// UUID is its sixteen canonical bytes. Every other value that spells text -
/// a registered code among them, which stores as the text it is - spells
/// that text's bytes.
pub(crate) fn bytes_from_value(value: &Scalar) -> Option<Bytes> {
    match value {
        Scalar::Bytes(bytes) => Some(bytes.clone()),
        Scalar::Geometry(value) => Some(Bytes::from_shared(Arc::clone(value.storage()))),
        Scalar::Geography(value) => Some(Bytes::from_shared(Arc::clone(value.storage()))),
        Scalar::Uuid(uuid) => Some(Bytes::new(uuid.into_bytes())),
        _ => value.as_str().map(|text| Bytes::new(text.as_bytes())),
    }
}

/// One of the four ways this crate lays bytes out - Arrow's four.
///
/// The layout is the physical shape alone, how a value's bytes are addressed;
/// the bound beside it in [`BytesType`] is how many there may be.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum BytesLayout {
    /// Variable width, 32-bit offsets - Arrow's `Binary`.
    #[default]
    Binary,
    /// One fixed byte width every value fills exactly - Arrow's
    /// `FixedSizeBinary`. Bytes are never padded: a value is its width.
    FixedSizeBinary,
    /// Variable width, 64-bit offsets - Arrow's `LargeBinary`.
    LargeBinary,
    /// The view layout: a short prefix inline, the rest out of line.
    BinaryView,
}

impl BytesLayout {
    /// Every layout in canonical declaration order.
    pub const ALL: [Self; 4] = [
        Self::Binary,
        Self::FixedSizeBinary,
        Self::LargeBinary,
        Self::BinaryView,
    ];

    /// The canonical name of this layout.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Binary => "binary",
            Self::FixedSizeBinary => "fixed_size_binary",
            Self::LargeBinary => "large_binary",
            Self::BinaryView => "binary_view",
        }
    }

    /// Resolve a layout from its name.
    ///
    /// Case, underscores, hyphens and spaces are all ignored, so
    /// `LARGE_BINARY`, `large-binary` and `largebinary` are one layout, and
    /// `fixed_binary` is the fixed one's second spelling.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownDataType`] for a name no layout answers to.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        if super::folds_equal(value, "fixed_binary") {
            return Ok(Self::FixedSizeBinary);
        }
        Self::ALL
            .into_iter()
            .find(|layout| super::folds_equal(value, layout.as_str()))
            .ok_or_else(|| Error::UnknownDataType(format_smolstr!("{value}")))
    }

    /// The identifier naming this layout.
    #[must_use]
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::Binary => DataTypeId::Binary,
            Self::FixedSizeBinary => DataTypeId::FixedSizeBinary,
            Self::LargeBinary => DataTypeId::LargeBinary,
            Self::BinaryView => DataTypeId::BinaryView,
        }
    }

    /// The layout one identifier names, `None` for an identifier that is not
    /// a byte layout's.
    #[must_use]
    pub const fn from_id(id: DataTypeId) -> Option<Self> {
        match id {
            DataTypeId::Binary => Some(Self::Binary),
            DataTypeId::FixedSizeBinary => Some(Self::FixedSizeBinary),
            DataTypeId::LargeBinary => Some(Self::LargeBinary),
            DataTypeId::BinaryView => Some(Self::BinaryView),
            _ => None,
        }
    }

    /// Return whether every value in this layout is the same width.
    #[must_use]
    pub const fn is_fixed(self) -> bool {
        matches!(self, Self::FixedSizeBinary)
    }

    /// Return whether this layout addresses its bytes through a view.
    #[must_use]
    pub const fn is_view(self) -> bool {
        matches!(self, Self::BinaryView)
    }

    /// Return whether this layout declares 64-bit offsets.
    #[must_use]
    pub const fn is_large(self) -> bool {
        matches!(self, Self::LargeBinary)
    }
}

impl fmt::Display for BytesLayout {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for BytesLayout {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for BytesLayout {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = <std::borrow::Cow<'_, str>>::deserialize(deserializer)?;
        Self::from_str(&value).map_err(serde::de::Error::custom)
    }
}

/// The inline threshold an integration test cannot reach.
///
/// `INLINE_BYTES` is crate-private: it is the byte count below which a
/// `Bytes` stores its payload in the value rather than behind an `Arc`, so
/// the boundary has to be crossed from inside. Everything a caller can
/// observe lives in `tests/types/bytes.rs`.
#[cfg(test)]
mod tests {
    use super::{Bytes, INLINE_BYTES};
    use crate::types::{BytesLayout, BytesType};
    use crate::{DataType, Scalar};

    #[test]
    fn a_short_payload_is_inline_and_a_long_one_is_shared() {
        let short = Bytes::new(vec![7_u8; INLINE_BYTES]);
        assert!(short.is_inline());
        let long = Bytes::new(vec![7_u8; INLINE_BYTES + 1]);
        assert!(!long.is_inline());
        assert_eq!(long.len(), INLINE_BYTES + 1);
        assert!(!Bytes::new_static(b"held").is_inline());
        assert_eq!(Bytes::default(), b"");
        assert_eq!(std::mem::size_of::<Bytes>(), 40);
    }

    #[test]
    fn equality_order_and_hash_read_the_payload_only() {
        use std::collections::HashSet;

        let plain = Bytes::new(b"abc");
        let large = Bytes::new(b"abc")
            .try_with_parameters(BytesType::new(BytesLayout::LargeBinary))
            .unwrap();
        assert_eq!(plain, large);
        assert_eq!(HashSet::from([plain.clone(), large.clone()]).len(), 1);
        assert_ne!(plain.parameters(), large.parameters());
        assert!(Bytes::new(b"b") > Bytes::new(b"a"));
        assert_eq!(format!("{large:?}"), "0x616263 as large_binary");
        assert_eq!(format!("{plain:?}"), "0x616263");
    }

    #[test]
    fn restating_checks_but_never_carries_a_maximum() {
        let bounded = BytesType::new(BytesLayout::Binary)
            .try_with_bound(4)
            .unwrap();
        let value = Bytes::new(b"abcd").try_with_parameters(bounded).unwrap();
        assert_eq!(value.parameters(), BytesType::default());
        assert_eq!(value.dtype().unwrap(), DataType::binary());
        let refused = Bytes::new(b"abcde")
            .try_with_parameters(bounded)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("at most 4 bytes"), "{refused}");
    }

    #[test]
    fn a_fixed_layout_takes_exactly_its_width() {
        let fixed = BytesType::new(BytesLayout::FixedSizeBinary)
            .try_with_bound(4)
            .unwrap();
        let value = Bytes::new(b"abcd").try_with_parameters(fixed).unwrap();
        assert_eq!(value.fixed(), Some(4));
        assert_eq!(
            value.dtype().unwrap(),
            DataType::fixed_size_binary(4).unwrap()
        );
        assert!(Bytes::new(b"abc").try_with_parameters(fixed).is_err());
        assert!(Bytes::new(b"abcde").try_with_parameters(fixed).is_err());
        assert!(
            Bytes::new(b"x")
                .try_with_parameters(BytesType::new(BytesLayout::FixedSizeBinary))
                .is_err()
        );
    }

    #[test]
    fn serde_writes_the_payload_alone_unless_the_value_declares_more() {
        let plain = Bytes::new(b"ab");
        assert_eq!(serde_json::to_string(&plain).unwrap(), "[97,98]");
        assert_eq!(serde_json::from_str::<Bytes>("[97,98]").unwrap(), plain);
        let fixed = Bytes::new(b"ab")
            .try_with_parameters(
                BytesType::new(BytesLayout::FixedSizeBinary)
                    .try_with_bound(2)
                    .unwrap(),
            )
            .unwrap();
        let document = serde_json::to_string(&fixed).unwrap();
        assert_eq!(
            document,
            r#"{"layout":"fixed_size_binary","fixed":2,"bytes":[97,98]}"#
        );
        let back = serde_json::from_str::<Bytes>(&document).unwrap();
        assert_eq!(back.parameters(), fixed.parameters());
        assert_eq!(back, fixed);
    }

    #[test]
    fn a_byte_scalar_names_its_own_datatype() {
        assert_eq!(
            Scalar::from(vec![1_u8]).dtype().unwrap(),
            DataType::binary()
        );
        let view = Bytes::new(b"x")
            .try_with_parameters(BytesType::new(BytesLayout::BinaryView))
            .unwrap();
        assert_eq!(
            Scalar::Bytes(view).dtype().unwrap(),
            DataType::binary_view()
        );
    }
}

impl crate::types::DataTypeValue for BytesType {
    const FAMILY: &'static str = "bytes";

    type Sidecar = ();

    fn id(&self) -> crate::DataTypeId {
        DataType::Bytes(*self).id()
    }

    fn kind(&self) -> crate::DataTypeKind {
        crate::DataTypeKind::Bytes
    }

    fn validate(&self) -> Result<()> {
        DataType::Bytes(*self).validate()
    }

    fn into_dtype(self) -> DataType {
        DataType::Bytes(self)
    }

    fn from_dtype(dtype: &DataType) -> Option<Self> {
        match dtype {
            DataType::Bytes(parameters) => Some(*parameters),
            _ => None,
        }
    }
}
