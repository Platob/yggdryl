//! The byte datatype family: one layout, one length bound.
//!
//! Arrow has four binary layouts and no way to bound one: a `Binary` array
//! declares 32-bit offsets and nothing else, and `varbinary(n)` exists
//! nowhere in the format. This family is the one place this crate answers
//! both questions - which layout, how long - and every byte column the crate
//! has is one member of it.
//!
//! [`BytesType`] names the four layouts. [`BytesType`] is a layout
//! beside the bound its values are held to. [`crate::DataType::bytes`] builds
//! the one byte datatype, [`crate::DataType::Bytes`], from them; `binary`,
//! `varbinary(16)` and `fixed_binary(16)` are spellings of it, never
//! datatypes of their own. [`Bytes`] is the one byte value.
//!
//! ```
//! use yggdryl::BytesType;
//! use yggdryl::DataType;
//!
//! # fn main() -> yggdryl::Result<()> {
//! // Every spelling is one datatype, and it reads back as itself.
//! assert_eq!(DataType::from_str("bytes")?, DataType::binary());
//! assert_eq!(DataType::from_str("fixed_binary(16)")?.to_string(), "fixed_binary(16)");
//!
//! // A maximum is its own leaf, and the leaf is what a byte column declares.
//! let bounded = DataType::from_str("varbinary(32)")?;
//! let parameters = bounded.bytes_parameters().expect("a byte datatype");
//! assert_eq!(parameters, BytesType::SizedBinary(32));
//! assert_eq!(parameters.max(), Some(32));
//! assert_eq!(bounded.to_string(), "sized_binary(32)");
//! # Ok(())
//! # }
//! ```

use std::borrow::{Borrow, Cow};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::Arc;

pub(crate) use arrow::{arrow_storage, describes_storage, from_arrow_storage, needs_extension};
use serde::{Deserialize, Serialize};
use smol_str::{SmolStr, format_smolstr};

use crate::Scalar;
use crate::parser::Parser;
use crate::{DataType, DataTypeId, Error, Result, Value};

/// What a byte datatype lays out in Arrow, and what it reads back from.
///
/// The four layouts are Arrow's own, so the storage is the layout. What Arrow
/// cannot say - a maximum on a variable layout - rides the `yggdryl.bytes`
/// extension document beside it; a fixed width is the storage itself and
/// needs no document.
mod arrow {
    use arrow_schema::DataType as ArrowDataType;

    use super::BytesType;
    use crate::{Error, Result};

    /// Whether a byte field needs the `yggdryl.bytes` document beside its
    /// storage.
    ///
    /// Two leaves state something Arrow cannot: a maximum, which no Arrow
    /// layout carries, and the second view width, which Arrow has one of.
    pub(crate) const fn needs_extension(parameters: BytesType) -> bool {
        matches!(
            parameters,
            BytesType::SizedBinary(_) | BytesType::LargeBinaryView
        )
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
        Ok(match parameters {
            // A maximum is the column's rule, so the storage is the plain
            // binary its values fill.
            BytesType::Binary | BytesType::SizedBinary(_) => ArrowDataType::Binary,
            BytesType::LargeBinary => ArrowDataType::LargeBinary,
            // Arrow has one view width where this crate declares two; which
            // one it is rides the `yggdryl.bytes` document beside it.
            BytesType::BinaryView | BytesType::LargeBinaryView => ArrowDataType::BinaryView,
            // The fixed leaf answered above: its width gave it its storage.
            BytesType::FixedBinary(_) => ArrowDataType::Binary,
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
            crate::invalid(
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
                crate::DataType::fixed_binary(arrow_fixed_width(*width)?)
            }
            other => Err(crate::invalid(
                "bytes",
                smol_str::format_smolstr!("expected a byte storage, got {other}"),
            )),
        }
    }
}
/// Binary layout accounting and identity checks for Arrow casts.
pub(crate) mod casts {

    use arrow_array::builder::{
        BinaryBuilder, BinaryViewBuilder, FixedSizeBinaryBuilder, LargeBinaryBuilder,
    };
    use arrow_array::types::{
        Int8Type, Int16Type, Int32Type, Int64Type, UInt8Type, UInt16Type, UInt32Type, UInt64Type,
    };
    use arrow_array::{
        Array, BinaryArray, BinaryViewArray, DictionaryArray, FixedSizeBinaryArray, Int16RunArray,
        Int32RunArray, Int64RunArray, LargeBinaryArray, LargeStringArray, StringArray,
        StringViewArray, UnionArray,
    };

    use std::sync::Arc;

    use arrow_array::ArrayRef;
    use arrow_buffer::BooleanBuffer;
    use arrow_cast::can_cast_types;
    use arrow_schema::DataType as ArrowDataType;
    use smol_str::{SmolStr, format_smolstr};

    use crate::arrow::{Error, Result};
    use crate::budget::MaterializationBudget;
    use crate::bytes::BytesType;
    use crate::cast::columns::{is_exposed, null_buffers_ptr_eq};
    use crate::cast::{arrow_cast_exposed, downcast, internal_target_error, named_cell};
    use crate::{DataType, Field};

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
        Ok(match target {
            // A maximum is the column's rule; the storage it fills is the
            // plain binary, so the two share one builder.
            BytesType::Binary | BytesType::SizedBinary(_) => {
                filled!(BinaryBuilder::with_capacity(rows, payload))
            }
            BytesType::LargeBinary => filled!(LargeBinaryBuilder::with_capacity(rows, payload)),
            // Arrow's view layout carries a prefix per cell rather than offsets,
            // so it takes the row count and grows its own payload blocks.
            BytesType::BinaryView | BytesType::LargeBinaryView => {
                filled!(BinaryViewBuilder::with_capacity(rows))
            }
            // Every accepted cell is exactly the width, which is what the builder
            // needs; a cell that is not was refused or nulled above. Its
            // `append_value` answers a `Result` the other three do not, so this
            // arm is written out rather than bent through the macro.
            BytesType::FixedBinary(_) => {
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
                DataType::Dictionary(_) | DataType::Union(..) | DataType::RunEndEncoded(_)
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
            DataType::Dictionary(dictionary) => {
                macro_rules! dictionary_len {
                    ($key:ty) => {{
                        let dictionary_array = downcast::<DictionaryArray<$key>>(array)?;
                        if dictionary_array.keys().is_null(index) {
                            0
                        } else {
                            let key = usize::try_from(dictionary_array.keys().value(index))
                                .map_err(|_| {
                                    Error::IncompatibleSchema(
                                        "Arrow dictionary key is negative or exceeds usize"
                                            .to_owned(),
                                    )
                                })?;
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
            DataType::Int32 | DataType::Decimal32 { .. } => 12,
            DataType::UInt32 => 10,
            DataType::Int64 | DataType::Decimal64 { .. } => 21,
            DataType::UInt64 => 20,
            DataType::Float16 => 16,
            DataType::Float32 => 24,
            DataType::Float64 => 32,
            DataType::Decimal128 { .. } => 41,
            DataType::Decimal256 { .. } => 78,
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
            bytes.checked_add(value_len(index)).ok_or_else(|| {
                Error::IncompatibleSchema("Arrow payload bytes exceed usize".to_owned())
            })
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
        left.len() == right.len()
            && (left.is_empty() || std::ptr::eq(left.as_ptr(), right.as_ptr()))
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
    /// [`DataType::Bytes`]; what differs is what it declares, and the leaf
    /// says all of it - the layout, and the number where the leaf carries one.
    ///
    /// ```
    /// use yggdryl::BytesType;
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(DataType::bytes(BytesType::LargeBinary)?, DataType::large_binary());
    /// assert_eq!(DataType::bytes(BytesType::SizedBinary(32))?.to_string(), "sized_binary(32)");
    /// assert_eq!(DataType::fixed_binary(16)?.to_string(), "fixed_binary(16)");
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
        Self::Bytes(BytesType::Binary)
    }

    /// Unbounded bytes with 64-bit offsets - Arrow's `LargeBinary`.
    #[must_use]
    pub const fn large_binary() -> Self {
        Self::Bytes(BytesType::LargeBinary)
    }

    /// Unbounded bytes in the view layout - Arrow's `BinaryView`.
    #[must_use]
    pub const fn binary_view() -> Self {
        Self::Bytes(BytesType::BinaryView)
    }

    /// Any length in the viewed layout over 64-bit offsets.
    #[must_use]
    pub const fn large_binary_view() -> Self {
        Self::Bytes(BytesType::LargeBinaryView)
    }

    /// At most `max` bytes per value, over 32-bit offsets.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] for a maximum of zero.
    pub fn sized_binary(max: u32) -> Result<Self> {
        Self::bytes(BytesType::SizedBinary(max))
    }

    /// Exactly `width` bytes per value - Arrow's `FixedSizeBinary`.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(DataType::fixed_binary(16)?.fixed_byte_width(), Some(16));
    /// assert!(DataType::fixed_binary(0).is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] for a width of zero.
    pub fn fixed_binary(width: u32) -> Result<Self> {
        Self::bytes(BytesType::FixedBinary(width))
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

/// The byte family's datatype payload: one leaf per storage a column has.
///
/// Arrow lays bytes out three ways and this crate declares two more: a second
/// view width, which a reader that only knows Arrow sees as one, and a
/// *sized* column, whose maximum Arrow has nowhere to state. Each is a leaf
/// here rather than a flag beside a layout, so a column is one thing and a
/// reader never has to ask whether the number it carries is a width or a
/// bound - the leaf already said.
///
/// ```
/// use yggdryl::BytesType;
/// use yggdryl::DataType;
///
/// # fn main() -> yggdryl::Result<()> {
/// // A maximum is the column's rule, and its own leaf.
/// let bounded = BytesType::SizedBinary(32);
/// assert_eq!(bounded.max(), Some(32));
/// assert_eq!(bounded.fixed(), None);
///
/// // A width is the storage, and its own leaf.
/// assert_eq!(BytesType::FixedBinary(16).fixed(), Some(16));
/// assert_eq!(BytesType::FixedBinary(16).max(), None);
///
/// let dtype = DataType::bytes(bounded)?;
/// assert_eq!(dtype.to_string(), "sized_binary(32)");
/// assert_eq!(DataType::from_str(&dtype.to_string())?, dtype);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum BytesType {
    /// Any length, 32-bit offsets - Arrow's `Binary`.
    #[default]
    Binary,
    /// Any length, 64-bit offsets - Arrow's `LargeBinary`.
    LargeBinary,
    /// Any length, viewed: a short prefix inline, the rest out of line -
    /// Arrow's `BinaryView`.
    BinaryView,
    /// The viewed layout over 64-bit offsets.
    ///
    /// Arrow has one view width, so this crosses an Arrow boundary as
    /// `BinaryView` with the leaf written in the `yggdryl.bytes` document.
    LargeBinaryView,
    /// Exactly this many bytes in every value - Arrow's `FixedSizeBinary`.
    ///
    /// Bytes are never padded: a value *is* its width.
    FixedBinary(u32),
    /// At most this many bytes in a value, over 32-bit offsets.
    ///
    /// Arrow has no `varbinary(n)`, so the maximum rides the `yggdryl.bytes`
    /// document; the storage is the plain `Binary` the values fill.
    SizedBinary(u32),
}

impl BytesType {
    /// Every leaf in canonical declaration order, with a stated number where
    /// the leaf carries one.
    pub const ALL: [Self; 6] = [
        Self::Binary,
        Self::LargeBinary,
        Self::BinaryView,
        Self::LargeBinaryView,
        Self::FixedBinary(1),
        Self::SizedBinary(1),
    ];

    /// Return the exact datatype identifier.
    #[must_use]
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::Binary => DataTypeId::Binary,
            Self::LargeBinary => DataTypeId::LargeBinary,
            Self::BinaryView => DataTypeId::BinaryView,
            Self::LargeBinaryView => DataTypeId::LargeBinaryView,
            Self::FixedBinary(_) => DataTypeId::FixedBinary,
            Self::SizedBinary(_) => DataTypeId::SizedBinary,
        }
    }

    /// The leaf one identifier names, with `width` where the leaf takes one.
    #[must_use]
    pub const fn from_id(id: DataTypeId, width: u32) -> Option<Self> {
        match id {
            DataTypeId::Binary => Some(Self::Binary),
            DataTypeId::LargeBinary => Some(Self::LargeBinary),
            DataTypeId::BinaryView => Some(Self::BinaryView),
            DataTypeId::LargeBinaryView => Some(Self::LargeBinaryView),
            DataTypeId::FixedBinary => Some(Self::FixedBinary(width)),
            DataTypeId::SizedBinary => Some(Self::SizedBinary(width)),
            _ => None,
        }
    }

    /// The canonical name of this leaf.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.id().as_str()
    }

    /// The declared byte bound, whichever shape the leaf gives it.
    #[must_use]
    pub const fn bound(self) -> Option<u32> {
        match self {
            Self::FixedBinary(width) | Self::SizedBinary(width) => Some(width),
            _ => None,
        }
    }

    /// The exact bytes every value fills, on the fixed leaf.
    #[must_use]
    pub const fn fixed(self) -> Option<u32> {
        match self {
            Self::FixedBinary(width) => Some(width),
            _ => None,
        }
    }

    /// The most bytes a value may hold, on the sized leaf.
    #[must_use]
    pub const fn max(self) -> Option<u32> {
        match self {
            Self::SizedBinary(max) => Some(max),
            _ => None,
        }
    }

    /// Whether every value fills one width exactly.
    #[must_use]
    pub const fn is_fixed(self) -> bool {
        matches!(self, Self::FixedBinary(_))
    }

    /// Whether the leaf states a number at all.
    #[must_use]
    pub const fn is_bounded(self) -> bool {
        self.bound().is_some()
    }

    /// Whether values are addressed through the view layout.
    #[must_use]
    pub const fn is_view(self) -> bool {
        matches!(self, Self::BinaryView | Self::LargeBinaryView)
    }

    /// Whether offsets are 64-bit.
    #[must_use]
    pub const fn is_large(self) -> bool {
        matches!(self, Self::LargeBinary | Self::LargeBinaryView)
    }

    /// The leaf a *value* of this column carries.
    ///
    /// A maximum is the column's rule and not the value's, so a value in a
    /// sized column is the plain binary it fills; every other leaf is already
    /// what a value is.
    #[must_use]
    pub const fn storage(self) -> Self {
        match self {
            Self::SizedBinary(_) => Self::Binary,
            other => other,
        }
    }

    /// Whether two leaves differ only in the count they state.
    ///
    /// A maximum is a rule laid over the plain binary its values fill, so a
    /// sized column and an unbounded one are one shape with one number
    /// between them; two fixed widths are likewise one shape. This is what a
    /// diff asks before it reports a changed bound rather than a changed
    /// datatype.
    #[must_use]
    pub const fn same_shape_as(self, other: Self) -> bool {
        matches!(
            (self.storage(), other.storage()),
            (Self::Binary, Self::Binary)
                | (Self::LargeBinary, Self::LargeBinary)
                | (Self::BinaryView, Self::BinaryView)
                | (Self::LargeBinaryView, Self::LargeBinaryView)
                | (Self::FixedBinary(_), Self::FixedBinary(_))
        )
    }

    /// The leaf this one becomes under a stated number.
    ///
    /// The number is the width on the fixed leaf and the maximum on the
    /// sized one. `binary` takes a maximum and answers `sized_binary`,
    /// because plain binary is exactly the storage a bounded column fills;
    /// the other three would lose themselves under a maximum, so they refuse
    /// it rather than silently becoming something narrower. This is the one
    /// place the rule is stated - the grammar and both bindings read it here
    /// rather than each deciding it again.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] when this leaf carries no number,
    /// naming the leaf a bounded column would be, or when the number cannot
    /// describe a column at all.
    pub fn with_bound(self, bound: u32) -> Result<Self> {
        let bounded = match self {
            Self::FixedBinary(_) => Self::FixedBinary(bound),
            Self::Binary | Self::SizedBinary(_) => Self::SizedBinary(bound),
            _ => {
                return Err(Error::InvalidDataType {
                    kind: "bytes",
                    reason: SmolStr::new_static(
                        "expected no maximum on this layout; \
                         a bounded column is sized_binary(maximum)",
                    ),
                });
            }
        };
        bounded.validate()?;
        Ok(bounded)
    }

    /// The leaf this one becomes under a number the caller may not have
    /// stated.
    ///
    /// Two leaves *are* their number - `fixed_binary` is a width and
    /// `sized_binary` a maximum - so neither stands without one; the other
    /// four stand alone and refuse one. This is what a grammar, a binding
    /// and a document all need to agree on, so it is decided here once.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] when a leaf that is its number was
    /// given none, or for any reason [`Self::with_bound`] refuses.
    pub fn with_declared_bound(self, bound: Option<u32>) -> Result<Self> {
        match (bound, self.bound()) {
            (Some(bound), _) => self.with_bound(bound),
            (None, None) => Ok(self),
            (None, Some(_)) => Err(Error::InvalidDataType {
                kind: "bytes",
                reason: format_smolstr!("expected {}(number), got none", self.as_str()),
            }),
        }
    }

    /// Reject a leaf whose stated number cannot describe a column.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a width or maximum of zero:
    /// a column that holds nothing is not a column.
    pub const fn validate(self) -> Result<()> {
        match self.bound() {
            Some(0) => Err(Error::InvalidDataType {
                kind: "bytes",
                reason: SmolStr::new_static("expected a width of at least one byte, got 0"),
            }),
            _ => Ok(()),
        }
    }

    /// The metadata key the number is written under.
    const fn bound_word_key(self) -> &'static str {
        match self.is_fixed() {
            true => "fixed",
            false => "max",
        }
    }

    /// The extension metadata an Arrow field carries this leaf in.
    ///
    /// Arrow has nowhere to put a maximum, and one view width where this
    /// crate declares two, so those two leaves ride the
    /// `ARROW:extension:metadata` document beside the `yggdryl.bytes` name
    /// with the leaf written whole; every other leaf is Arrow's own.
    #[must_use]
    pub fn extension_json(self) -> String {
        let mut rendered = String::with_capacity(48);
        rendered.push_str("{\"layout\":\"");
        rendered.push_str(self.as_str());
        rendered.push('"');
        if let Some(bound) = self.bound() {
            rendered.push(',');
            rendered.push('"');
            rendered.push_str(self.bound_word_key());
            rendered.push_str("\":");
            rendered.push_str(&format_smolstr!("{bound}"));
        }
        rendered.push('}');
        rendered
    }

    /// Read a leaf back out of Arrow extension metadata.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] when the document is not an object
    /// naming a leaf this crate knows, or numbers a leaf the wrong way.
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
        let named = Self::from_str(&document.layout)
            .map_err(|error| invalid(format_smolstr!("{error}")))?;
        // A maximum makes a column sized whichever plain layout names it, so
        // `{"layout":"binary","max":16}` and `{"layout":"sized_binary","max":16}`
        // are one document written two ways; a width only belongs to the
        // fixed leaf.
        let leaf = match (named, document.fixed, document.max) {
            (Self::FixedBinary(_), Some(fixed), None) => Self::FixedBinary(fixed),
            (Self::FixedBinary(_), fixed, max) => {
                return Err(invalid(format_smolstr!(
                    "expected one width on {named}, got fixed={fixed:?} max={max:?}"
                )));
            }
            (_, Some(fixed), _) => {
                return Err(invalid(format_smolstr!(
                    "expected a maximum on {named}, got fixed={fixed}"
                )));
            }
            (_, None, Some(max)) => Self::SizedBinary(max),
            (Self::SizedBinary(_), None, None) => {
                return Err(invalid(format_smolstr!(
                    "expected a maximum on {named}, got none"
                )));
            }
            (other, None, None) => other,
        };
        leaf.validate()?;
        Ok(leaf)
    }

    /// Resolve a leaf from its name, with `1` where the leaf takes a number.
    ///
    /// Case, underscores, hyphens and spaces are all ignored, so
    /// `LARGE_BINARY`, `large-binary` and `largebinary` are one leaf.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] naming the input when no leaf
    /// spells it.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        Self::from_spelling(&crate::normalized(value))
            .ok_or_else(|| invalid(format_smolstr!("expected a byte layout, got {value:?}")))
    }

    /// The leaf one already-folded word names, in every spelling.
    ///
    /// Case, underscores, hyphens and spaces are gone by the time a word
    /// reaches here. The two leaves that carry a number answer with a
    /// placeholder of one, which
    /// [`Self::with_declared_bound`] then replaces or refuses; nothing reads
    /// the placeholder as a stated width.
    ///
    /// The grammar and the bindings both read this, so a spelling a caller
    /// may write is accepted in a datatype expression and under a `layout=`
    /// argument alike, rather than in one of the two.
    #[must_use]
    pub fn from_spelling(word: &str) -> Option<Self> {
        match word {
            "binary" | "bytes" | "varbinary" | "blob" | "bytea" => Some(Self::Binary),
            "fixedbinary" | "fixedsizebinary" => Some(Self::FixedBinary(1)),
            "largebinary" => Some(Self::LargeBinary),
            "binaryview" => Some(Self::BinaryView),
            "largebinaryview" => Some(Self::LargeBinaryView),
            "sizedbinary" | "varbinarybounded" => Some(Self::SizedBinary(1)),
            _ => None,
        }
    }
}

/// The refusal every invalid parameter answers with.
fn invalid(reason: impl Into<SmolStr>) -> Error {
    Error::InvalidDataType {
        kind: "bytes",
        reason: reason.into(),
    }
}

impl fmt::Display for BytesType {
    /// The canonical spelling, which [`crate::DataType`]'s grammar reads back.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())?;
        match self.bound() {
            None => Ok(()),
            Some(bound) => write!(formatter, "({bound})"),
        }
    }
}

impl Serialize for BytesType {
    /// The leaf's name alone.
    ///
    /// A stored document writes the number beside this rather than inside it:
    /// a schema document carries `"layout"` with `"max"` or `"fixed"`, and the
    /// `yggdryl.bytes` document does the same. One name, one place.
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for BytesType {
    /// The leaf one name spells, with no number: the caller puts it back.
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = <std::borrow::Cow<'_, str>>::deserialize(deserializer)?;
        Self::from_str(&value).map_err(serde::de::Error::custom)
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
    pub(crate) fn parse_bytes(&mut self, leaf: BytesType) -> Result<DataType> {
        // What the text stated, never what the leaf happened to carry: the
        // leaf a spelling names carries a placeholder number, so comparing
        // against it read `fixed_binary(1)` as a width nobody wrote.
        let mut declared = None;
        if let Some(close) = self.consume_opening() {
            // Empty parentheses are the bare spelling with punctuation.
            if !self.consume_symbol(close) {
                let position = self.current_position();
                let label = match leaf.is_fixed() {
                    true => "a byte width",
                    false => "a maximum byte length",
                };
                let value = self.parse_integer(label)?;
                declared = Some(u32::try_from(value).map_err(|_| {
                    self.error_at(
                        position,
                        format_smolstr!("expected {label} inside u32, got {value}"),
                    )
                })?);
                self.expect_symbol(close)?;
            }
        }
        let position = self.current_position();
        let parameters = leaf
            .with_declared_bound(declared)
            .map_err(|error| self.error_at(position, format_smolstr!("{error}")))?;
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
/// use yggdryl::{Bytes, BytesType, INLINE_BYTES};
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
/// let large = BytesType::LargeBinary;
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
            parameters: BytesType::Binary,
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
            parameters: parameters.storage(),
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
        self.parameters = parameters.storage();
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
    pub const fn layout(&self) -> BytesType {
        self.parameters
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
    layout: SmolStr,
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
            layout: SmolStr::new(self.parameters().as_str()),
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
                let named =
                    BytesType::from_str(&declared.layout).map_err(serde::de::Error::custom)?;
                let parameters = match (named, declared.fixed) {
                    (BytesType::FixedBinary(_), Some(width)) => BytesType::FixedBinary(width),
                    (other, _) => other,
                };
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

impl crate::DataTypeValue for BytesType {
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
