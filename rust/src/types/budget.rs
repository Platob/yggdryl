//! Bounded scratch and output reservations for Arrow casts.

use std::sync::Arc;

use arrow_array::types::{BinaryType, BinaryViewType, ByteArrayType, ByteViewType, Int16Type, Int32Type, Int64Type, Int8Type, LargeBinaryType, LargeUtf8Type, RunEndIndexType, StringViewType, UInt16Type, UInt32Type, UInt64Type, UInt8Type, Utf8Type};
use arrow_array::{Array, ArrayRef, BinaryViewArray, DictionaryArray, FixedSizeListArray, GenericByteArray, GenericByteViewArray, Int16RunArray, Int32RunArray, Int64RunArray, LargeListArray, LargeListViewArray, ListArray, ListViewArray, MapArray, RunArray, StringViewArray, StructArray, UnionArray};
use arrow_buffer::ArrowNativeType;
use arrow_cast::display::{ArrayFormatter, FormatOptions};
use arrow_schema::DataType as ArrowDataType;
pub(crate) use limits::{ MAX_PHYSICAL_SLOTS, MaterializationBudget, checked_physical_mul, invalid_value, physical_limit_error, physical_union_branch, unsupported, };

use crate::arrow::{Error, Result};
use crate::types::bytes::casts::{byte_array_storage_ptr_eq, checked_valid_payload_bytes, projected_byte_len};
use crate::types::cast::downcast;
use crate::types::cast::columns::{dictionary_values_ref, offset_pair};
use crate::types::{bytes, string};
use crate::{DataType, Field, UnionMode};
use crate::types::sequence::SequenceType;
use crate::types::enums::EnumType;

/// Bounded Arrow materialization accounting.
mod limits {
    use crate::types::enums::EnumType;
    use crate::types::sequence::SequenceType;
    
    use crate::arrow::{Error, Result};
    use crate::{DataType, Field, Scalar, TimeUnit, UnionMode};
    use crate::types::DecimalType;

    // Composite Arrow layouts can turn one logical null or inactive union member
    // into a large number of mandatory physical child slots. Keep the same
    // conservative limits as the core default planner, but account for the Arrow
    // buffers that the logical Scalar tree does not own.
    pub(crate) const MAX_PHYSICAL_SLOTS: usize = 1_000_000;
    const MAX_PHYSICAL_BYTES: usize = 64 * 1024 * 1024;

    #[derive(Clone, Copy)]
    pub(crate) struct MaterializationMark {
        slots: usize,
        fixed_bytes: usize,
    }

    #[derive(Default)]
    pub(crate) struct MaterializationBudget {
        slots: usize,
        fixed_bytes: usize,
    }

    impl MaterializationBudget {
        /// Captures the retained allocation total before a temporary phase.
        pub(crate) fn mark(&self) -> MaterializationMark {
            MaterializationMark {
                slots: self.slots,
                fixed_bytes: self.fixed_bytes,
            }
        }

        /// Releases reservations whose allocations cannot outlive a completed phase.
        pub(crate) fn restore(&mut self, mark: MaterializationMark) {
            self.slots = mark.slots;
            self.fixed_bytes = mark.fixed_bytes;
        }

        pub(crate) fn add_bitmap(&mut self, rows: usize) -> Result<()> {
            self.add_bytes(bitmap_bytes(rows)?)
        }

        pub(crate) fn add_repeated_default(&mut self, dtype: &DataType, rows: usize) -> Result<()> {
            self.add_repeated_default_impl(dtype, rows, true)
        }

        pub(crate) fn add_repeated_default_without_dictionary_values(
            &mut self,
            dtype: &DataType,
            rows: usize,
        ) -> Result<()> {
            self.add_repeated_default_impl(dtype, rows, false)
        }

        fn add_repeated_default_impl(
            &mut self,
            dtype: &DataType,
            rows: usize,
            include_dictionary_values: bool,
        ) -> Result<()> {
            if rows == 0 {
                return Ok(());
            }
            match dtype {
                DataType::Sequence(SequenceType::FixedSizeList(child, size)) => {
                    self.add_array_layout(dtype, rows)?;
                    let size = usize::try_from(*size)
                        .map_err(|_| invalid_value("a fixed list size within usize", size))?;
                    let child_rows =
                        checked_physical_mul(rows, size, "fixed-size-list slots", MAX_PHYSICAL_SLOTS)?;
                    self.add_repeated_field_default(child, child_rows, include_dictionary_values)
                }
                DataType::Structure(fields) => {
                    self.add_array_layout(dtype, rows)?;
                    for field in fields {
                        self.add_repeated_field_default(field, rows, include_dictionary_values)?;
                    }
                    Ok(())
                }
                DataType::Union(fields, mode) => {
                    self.add_array_layout(dtype, rows)?;
                    let (selected_id, _) = physical_union_branch(dtype, fields)?;
                    for (type_id, field) in fields {
                        if matches!(mode, UnionMode::Dense) && type_id != selected_id {
                            continue;
                        }
                        if type_id == selected_id {
                            self.add_repeated_field_default(field, rows, include_dictionary_values)?;
                        } else {
                            self.add_null_array(field.dtype(), rows)?;
                        }
                    }
                    Ok(())
                }
                DataType::Enum(EnumType::Dictionary(dictionary)) => {
                    self.add_array_layout(dtype, rows)?;
                    if !include_dictionary_values
                        || dictionary.value().is_default_value(&Scalar::Null)?
                    {
                        Ok(())
                    } else {
                        self.add_repeated_default_impl(dictionary.value(), 1, true)
                    }
                }
                DataType::RunEndEncoded(encoded) => {
                    self.add_slots(1)?;
                    self.add_array(encoded.run_ends().dtype(), 1)?;
                    self.add_repeated_field_default(encoded.values(), 1, include_dictionary_values)
                }
                _ => self.add_array(dtype, rows),
            }
        }

        fn add_repeated_field_default(
            &mut self,
            field: &Field,
            rows: usize,
            include_dictionary_values: bool,
        ) -> Result<()> {
            if field.is_nullable() {
                self.add_null_array(field.dtype(), rows)
            } else {
                self.add_repeated_default_impl(field.dtype(), rows, include_dictionary_values)
            }
        }

        /// Reserves a live one-row default without charging its reusable logical
        /// root slot twice. Physical descendants and every owned root buffer are
        /// still charged because a deeply nested scalar can itself reach a cap.
        pub(crate) fn add_default_scalar_scratch(&mut self, dtype: &DataType) -> Result<()> {
            match dtype {
                DataType::Sequence(SequenceType::FixedSizeList(child, size)) => {
                    self.add_array_layout_without_slots(dtype, 1)?;
                    let size = usize::try_from(*size)
                        .map_err(|_| invalid_value("a fixed list size within usize", size))?;
                    self.add_repeated_field_default(child, size, true)
                }
                DataType::Structure(fields) => {
                    self.add_array_layout_without_slots(dtype, 1)?;
                    for field in fields {
                        self.add_repeated_field_default(field, 1, true)?;
                    }
                    Ok(())
                }
                DataType::Union(fields, mode) => {
                    self.add_array_layout_without_slots(dtype, 1)?;
                    let (selected_id, _) = physical_union_branch(dtype, fields)?;
                    for (type_id, field) in fields {
                        if matches!(mode, UnionMode::Dense) && type_id != selected_id {
                            continue;
                        }
                        if type_id == selected_id {
                            self.add_repeated_field_default(field, 1, true)?;
                        } else {
                            self.add_null_array(field.dtype(), 1)?;
                        }
                    }
                    Ok(())
                }
                DataType::Enum(EnumType::Dictionary(dictionary)) => {
                    self.add_array_layout_without_slots(dtype, 1)?;
                    if dictionary.value().is_default_value(&Scalar::Null)? {
                        Ok(())
                    } else {
                        self.add_repeated_default(dictionary.value(), 1)
                    }
                }
                DataType::RunEndEncoded(encoded) => {
                    self.add_array(encoded.run_ends().dtype(), 1)?;
                    self.add_repeated_field_default(encoded.values(), 1, true)
                }
                _ => self.add_array_without_root_slots(dtype, 1),
            }
        }

        pub(crate) fn add_array(&mut self, dtype: &DataType, rows: usize) -> Result<()> {
            self.add_array_impl(dtype, rows, true)
        }

        fn add_array_without_root_slots(&mut self, dtype: &DataType, rows: usize) -> Result<()> {
            self.add_array_impl(dtype, rows, false)
        }

        fn add_array_impl(
            &mut self,
            dtype: &DataType,
            rows: usize,
            count_root_slots: bool,
        ) -> Result<()> {
            if rows == 0 {
                return Ok(());
            }
            self.add_array_layout_impl(dtype, rows, count_root_slots)?;
            match dtype {
                DataType::Sequence(SequenceType::FixedSizeList(child, size)) => {
                    let size = usize::try_from(*size)
                        .map_err(|_| invalid_value("a fixed list size within usize", size))?;
                    let child_rows =
                        checked_physical_mul(rows, size, "fixed-size-list slots", MAX_PHYSICAL_SLOTS)?;
                    self.add_array(child.dtype(), child_rows)?;
                }
                DataType::Structure(fields) => {
                    for field in fields {
                        self.add_array(field.dtype(), rows)?;
                    }
                }
                DataType::Union(fields, mode) => {
                    if matches!(mode, UnionMode::Sparse) {
                        // Sparse unions require every child at the parent length.
                        for (_, field) in fields {
                            self.add_array(field.dtype(), rows)?;
                        }
                    } else {
                        // Hidden dense-union fillers all use the core canonical
                        // default branch when one exists. A logically uninhabited
                        // union can still occupy a slot masked by an ancestor;
                        // select its first physically bounded branch without
                        // visiting inactive payloads. Charge exactly that child
                        // into the shared aggregate budget.
                        let (_, field) = physical_union_branch(dtype, fields)?;
                        self.add_array(field.dtype(), rows)?;
                    }
                }
                DataType::Enum(EnumType::Dictionary(dictionary)) => {
                    // There can be at most one distinct dictionary value per row.
                    self.add_array(dictionary.value(), rows)?;
                }
                DataType::RunEndEncoded(encoded) => {
                    // Both physical children contain at most one slot per logical
                    // row. Recurse so wide value storage and nested wrappers join
                    // the same aggregate budget before either child is built.
                    self.add_array(encoded.run_ends().dtype(), rows)?;
                    self.add_array(encoded.values().dtype(), rows)?;
                }
                _ => {}
            }
            Ok(())
        }

        /// Reserves only buffers owned by an array's outer layout.
        ///
        /// Selection kernels can share some children and compact others. Keeping
        /// the shallow reservation separate lets those callers charge the actual
        /// selected child rows without pessimistically charging hidden payloads.
        pub(crate) fn add_array_layout(&mut self, dtype: &DataType, rows: usize) -> Result<()> {
            self.add_array_layout_impl(dtype, rows, true)
        }

        fn add_array_layout_without_slots(&mut self, dtype: &DataType, rows: usize) -> Result<()> {
            self.add_array_layout_impl(dtype, rows, false)
        }

        fn add_array_layout_impl(
            &mut self,
            dtype: &DataType,
            rows: usize,
            count_slots: bool,
        ) -> Result<()> {
            if rows == 0 {
                return Ok(());
            }
            if count_slots {
                self.add_slots(rows)?;
            }
            // The constructors in this module use nullable builders even when a
            // particular batch contains no null, so reserve the worst-case bitmap
            // as fixed physical overhead.
            self.add_bytes(bitmap_bytes(rows)?)?;

            match dtype {
                DataType::Boolean => self.add_bytes(bitmap_bytes(rows)?)?,
                DataType::Int8 | DataType::UInt8 => self.add_fixed_rows(rows, 1)?,
                DataType::Int16 | DataType::UInt16 | DataType::Float16 => {
                    self.add_fixed_rows(rows, 2)?;
                }
                DataType::Int32
                | DataType::UInt32
                | DataType::Float32
                | DataType::Date32
                | DataType::Time32(_)
                | DataType::Interval(TimeUnit::YearMonth)
                | DataType::Decimal(DecimalType::Decimal32 { .. }) => self.add_fixed_rows(rows, 4)?,
                // A registered code is US-ASCII text bounded at the width its
                // standard fixes, so it charges one 32-bit offset a row and at
                // most that many payload bytes. The variants stay spelled out so
                // this match keeps refusing to compile when a datatype is added;
                // the number they charge is read from `code_width`, which owns
                // it, rather than restated here.
                DataType::Country
                | DataType::Currency
                | DataType::Mic
                | DataType::Cfi
                | DataType::Isin
                | DataType::Cusip
                | DataType::Sedol
                | DataType::Bloomberg
                | DataType::Side
                | DataType::State
                | DataType::TimeInForce => {
                    self.add_offsets(rows, 4)?;
                    self.add_fixed_rows(rows, dtype.code_width().unwrap_or_default())?;
                }
                DataType::Int64
                | DataType::UInt64
                | DataType::Float64
                | DataType::DateTime64 { .. }
                | DataType::Date64
                | DataType::Time64(_)
                | DataType::Duration32(_)
                | DataType::Duration64(_)
                | DataType::Interval(TimeUnit::DayTime)
                | DataType::Decimal(DecimalType::Decimal64 { .. })
                | DataType::Sequence(SequenceType::ListView(_)) => self.add_fixed_rows(rows, 8)?,
                DataType::Interval(TimeUnit::MonthDayNano)
                | DataType::Decimal(DecimalType::Decimal128 { .. })
                | DataType::Uuid(_)
                | DataType::Sequence(SequenceType::LargeListView(_)) => {
                    self.add_fixed_rows(rows, 16)?;
                }
                DataType::Decimal(DecimalType::Decimal256 { .. }) => self.add_fixed_rows(rows, 32)?,
                DataType::Interval(_) => {
                    return Err(unsupported(dtype, "invalid interval layout"));
                }
                DataType::Version
                | DataType::Url
                | DataType::Timezone
                | DataType::MimeType
                | DataType::MediaType
                | DataType::Sequence(SequenceType::List(_))
                | DataType::Mapping(_)
                // A geospatial column is one binary column of WKB payloads.
                | DataType::Geometry(_)
                | DataType::Geography(_) => {
                    self.add_offsets(rows, 4)?;
                }
                // The variant's storage is two required binary children, so the
                // worst-case buffer charge is two offset runs.
                DataType::Variant => {
                    self.add_offsets(rows, 4)?;
                    self.add_offsets(rows, 4)?;
                }
                DataType::Sequence(SequenceType::LargeList(_)) => self.add_offsets(rows, 8)?,
                // A byte or string column's cost is its storage's: a fixed width
                // is that width per row, a view is one sixteen-byte descriptor,
                // and the two variable layouts are their offset runs.
                DataType::Bytes(parameters) => self.add_bytes_rows(rows, *parameters)?,
                DataType::String(parameters) => self.add_string_rows(rows, *parameters)?,
                DataType::Null
                | DataType::Sequence(SequenceType::FixedSizeList(..))
                | DataType::Structure(_)
                | DataType::RunEndEncoded(_) => {}
                DataType::Union(_, mode) => self.add_union_buffers(rows, *mode)?,
                DataType::Enum(EnumType::Dictionary(dictionary)) => {
                    self.add_fixed_rows(rows, integer_width(dictionary.key())?)?;
                }
            }
            Ok(())
        }

        #[allow(clippy::too_many_lines)] // Mirrors every Arrow null-array physical layout.
        pub(crate) fn add_null_array(&mut self, dtype: &DataType, rows: usize) -> Result<()> {
            self.add_null_array_impl(dtype, rows, true)
        }

        /// Reserves a one-row physical null placeholder while excluding its root
        /// slot, which is already represented by the eventual output row.
        pub(crate) fn add_null_scalar_scratch(&mut self, dtype: &DataType) -> Result<()> {
            self.add_null_array_impl(dtype, 1, false)
        }

        #[allow(clippy::too_many_lines)] // Mirrors every Arrow null-array physical layout.
        fn add_null_array_impl(
            &mut self,
            dtype: &DataType,
            rows: usize,
            count_root_slots: bool,
        ) -> Result<()> {
            if rows == 0 {
                return Ok(());
            }
            if count_root_slots {
                self.add_slots(rows)?;
            }
            self.add_bytes(bitmap_bytes(rows)?)?;
            match dtype {
                DataType::Null => {}
                DataType::Boolean => self.add_bytes(bitmap_bytes(rows)?)?,
                DataType::Int8 | DataType::UInt8 => self.add_fixed_rows(rows, 1)?,
                DataType::Int16 | DataType::UInt16 | DataType::Float16 => {
                    self.add_fixed_rows(rows, 2)?;
                }
                DataType::Int32
                | DataType::UInt32
                | DataType::Float32
                | DataType::Date32
                | DataType::Time32(_)
                | DataType::Interval(TimeUnit::YearMonth)
                | DataType::Decimal(DecimalType::Decimal32 { .. }) => self.add_fixed_rows(rows, 4)?,
                // A registered code is US-ASCII text bounded at the width its
                // standard fixes, so it charges one 32-bit offset a row and at
                // most that many payload bytes. The variants stay spelled out so
                // this match keeps refusing to compile when a datatype is added;
                // the number they charge is read from `code_width`, which owns
                // it, rather than restated here.
                DataType::Country
                | DataType::Currency
                | DataType::Mic
                | DataType::Cfi
                | DataType::Isin
                | DataType::Cusip
                | DataType::Sedol
                | DataType::Bloomberg
                | DataType::Side
                | DataType::State
                | DataType::TimeInForce => {
                    self.add_offsets(rows, 4)?;
                    self.add_fixed_rows(rows, dtype.code_width().unwrap_or_default())?;
                }
                DataType::Int64
                | DataType::UInt64
                | DataType::Float64
                | DataType::DateTime64 { .. }
                | DataType::Date64
                | DataType::Time64(_)
                | DataType::Duration32(_)
                | DataType::Duration64(_)
                | DataType::Interval(TimeUnit::DayTime)
                | DataType::Decimal(DecimalType::Decimal64 { .. })
                | DataType::Sequence(SequenceType::ListView(_)) => self.add_fixed_rows(rows, 8)?,
                DataType::Interval(TimeUnit::MonthDayNano)
                | DataType::Decimal(DecimalType::Decimal128 { .. })
                | DataType::Uuid(_)
                | DataType::Sequence(SequenceType::LargeListView(_)) => self.add_fixed_rows(rows, 16)?,
                DataType::Decimal(DecimalType::Decimal256 { .. }) => self.add_fixed_rows(rows, 32)?,
                DataType::Interval(_) => {
                    return Err(unsupported(dtype, "invalid interval layout"));
                }
                DataType::Version
                | DataType::Url
                | DataType::Timezone
                | DataType::MimeType
                | DataType::MediaType
                | DataType::Sequence(SequenceType::List(_))
                | DataType::Mapping(_)
                // A geospatial column is one binary column of WKB payloads.
                | DataType::Geometry(_)
                | DataType::Geography(_) => {
                    self.add_offsets(rows, 4)?;
                }
                // The variant's storage is two required binary children, so the
                // worst-case buffer charge is two offset runs.
                DataType::Variant => {
                    self.add_offsets(rows, 4)?;
                    self.add_offsets(rows, 4)?;
                }
                DataType::Sequence(SequenceType::LargeList(_)) => self.add_offsets(rows, 8)?,
                // A byte or string column's cost is its storage's: a fixed width
                // is that width per row, a view is one sixteen-byte descriptor,
                // and the two variable layouts are their offset runs.
                DataType::Bytes(parameters) => self.add_bytes_rows(rows, *parameters)?,
                DataType::String(parameters) => self.add_string_rows(rows, *parameters)?,
                DataType::Sequence(SequenceType::FixedSizeList(child, size)) => {
                    let size = usize::try_from(*size)
                        .map_err(|_| invalid_value("a fixed list size within usize", size))?;
                    let child_rows =
                        checked_physical_mul(rows, size, "fixed-size-list slots", MAX_PHYSICAL_SLOTS)?;
                    self.add_null_array(child.dtype(), child_rows)?;
                }
                DataType::Structure(fields) => {
                    for field in fields {
                        self.add_null_array(field.dtype(), rows)?;
                    }
                }
                DataType::Union(fields, mode) => {
                    self.add_union_buffers(rows, *mode)?;
                    match mode {
                        UnionMode::Sparse => {
                            for (_, field) in fields {
                                self.add_null_array(field.dtype(), rows)?;
                            }
                        }
                        UnionMode::Dense => {
                            let (_, field) = physical_union_branch(dtype, fields)?;
                            self.add_null_array(field.dtype(), rows)?;
                        }
                    }
                }
                DataType::Enum(EnumType::Dictionary(dictionary)) => {
                    self.add_fixed_rows(rows, integer_width(dictionary.key())?)?;
                }
                DataType::RunEndEncoded(encoded) => {
                    let maximum = match encoded.run_ends().dtype() {
                        DataType::Int16 => i16::MAX as usize,
                        DataType::Int32 => i32::MAX as usize,
                        DataType::Int64 => usize::MAX,
                        dtype => return Err(unsupported(dtype, "invalid run-end type")),
                    };
                    if rows > maximum {
                        return Err(physical_limit_error("run-end value", rows, maximum));
                    }
                    self.add_array(encoded.run_ends().dtype(), 1)?;
                    self.add_null_array(encoded.values().dtype(), 1)?;
                }
            }
            Ok(())
        }

        fn add_union_buffers(&mut self, rows: usize, mode: UnionMode) -> Result<()> {
            self.add_fixed_rows(rows, 1)?;
            if matches!(mode, UnionMode::Dense) {
                self.add_fixed_rows(rows, 4)?;
            }
            Ok(())
        }

        /// Charge one byte column against the storage it lays out.
        fn add_bytes_rows(
            &mut self,
            rows: usize,
            parameters: crate::types::BytesType,
        ) -> Result<()> {
            use arrow_schema::DataType as ArrowDataType;

            match crate::types::bytes::arrow_storage(parameters)? {
                ArrowDataType::FixedSizeBinary(width) => {
                    let width = usize::try_from(width)
                        .map_err(|_| invalid_value("a fixed binary width within usize", width))?;
                    self.add_fixed_rows(rows, width)
                }
                ArrowDataType::LargeBinary => self.add_offsets(rows, 8),
                ArrowDataType::BinaryView => self.add_fixed_rows(rows, 16),
                _ => self.add_offsets(rows, 4),
            }
        }

        /// Charge one string column against the layout it declares.
        fn add_string_rows(
            &mut self,
            rows: usize,
            parameters: crate::types::StringType,
        ) -> Result<()> {
            use crate::types::StringLayout;

            if let Some(width) = parameters.fixed() {
                return self.add_fixed_rows(rows, width as usize);
            }
            match parameters.layout() {
                StringLayout::String => self.add_offsets(rows, 4),
                StringLayout::LargeString => self.add_offsets(rows, 8),
                StringLayout::StringView | StringLayout::LargeStringView => {
                    self.add_fixed_rows(rows, 16)
                }
                // The fixed layout answered above; it is the only one with a width.
                StringLayout::FixedString => self.add_offsets(rows, 4),
            }
        }

        fn add_offsets(&mut self, rows: usize, width: usize) -> Result<()> {
            let offsets = rows
                .checked_add(1)
                .ok_or_else(|| physical_limit_error("offset count", rows, MAX_PHYSICAL_SLOTS))?;
            self.add_fixed_rows(offsets, width)
        }

        fn add_fixed_rows(&mut self, rows: usize, width: usize) -> Result<()> {
            self.add_bytes(checked_physical_mul(
                rows,
                width,
                "fixed buffer bytes",
                MAX_PHYSICAL_BYTES,
            )?)
        }

        fn add_slots(&mut self, slots: usize) -> Result<()> {
            self.slots = self.slots.checked_add(slots).ok_or_else(|| {
                physical_limit_error(
                    "expanded slots",
                    self.slots.saturating_add(slots),
                    MAX_PHYSICAL_SLOTS,
                )
            })?;
            if self.slots > MAX_PHYSICAL_SLOTS {
                return Err(physical_limit_error(
                    "expanded slots",
                    self.slots,
                    MAX_PHYSICAL_SLOTS,
                ));
            }
            Ok(())
        }

        pub(crate) fn add_physical_slots(&mut self, slots: usize) -> Result<()> {
            self.add_slots(slots)
        }

        pub(crate) fn add_bytes(&mut self, bytes: usize) -> Result<()> {
            self.fixed_bytes = self.fixed_bytes.checked_add(bytes).ok_or_else(|| {
                physical_limit_error(
                    "fixed bytes",
                    self.fixed_bytes.saturating_add(bytes),
                    MAX_PHYSICAL_BYTES,
                )
            })?;
            if self.fixed_bytes > MAX_PHYSICAL_BYTES {
                return Err(physical_limit_error(
                    "fixed bytes",
                    self.fixed_bytes,
                    MAX_PHYSICAL_BYTES,
                ));
            }
            Ok(())
        }
    }

    fn bitmap_bytes(rows: usize) -> Result<usize> {
        rows.checked_add(7)
            .map(|bits| bits / 8)
            .ok_or_else(|| physical_limit_error("bitmap bytes", rows, MAX_PHYSICAL_BYTES))
    }

    pub(crate) fn checked_physical_mul(
        left: usize,
        right: usize,
        kind: &'static str,
        limit: usize,
    ) -> Result<usize> {
        left.checked_mul(right)
            .ok_or_else(|| physical_limit_error(kind, left.saturating_mul(right), limit))
    }

    fn integer_width(dtype: &DataType) -> Result<usize> {
        match dtype {
            DataType::Int8 | DataType::UInt8 => Ok(1),
            DataType::Int16 | DataType::UInt16 => Ok(2),
            DataType::Int32 | DataType::UInt32 => Ok(4),
            DataType::Int64 | DataType::UInt64 => Ok(8),
            other => Err(unsupported(
                other,
                format!(
                    "expected an integer dictionary key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got {other}"
                ),
            )),
        }
    }

    pub(crate) fn physical_limit_error(kind: &'static str, actual: usize, limit: usize) -> Error {
        Error::physical_limit(kind, actual, limit)
    }

    pub(crate) fn physical_union_branch<'a>(
        dtype: &DataType,
        fields: &'a crate::UnionFields,
    ) -> Result<(i8, &'a Field)> {
        if let Ok(Some(selected)) = dtype.default_union_type_id() {
            if let Some((type_id, field)) = fields.iter().find(|(type_id, _)| *type_id == selected) {
                return Ok((type_id, field));
            }
        }

        let mut first_error = None;
        for (type_id, field) in fields {
            let mut probe = MaterializationBudget::default();
            match probe.add_array(field.dtype(), 1) {
                Ok(()) => return Ok((type_id, field)),
                Err(error) => first_error.get_or_insert(error),
            };
        }
        Err(first_error.unwrap_or_else(|| Error::internal("union_array::no_physical_branch")))
    }

    pub(crate) fn unsupported(dtype: &DataType, reason: impl Into<String>) -> Error {
        Error::Unsupported {
            kind: dtype.name(),
            reason: reason.into(),
        }
    }

    pub(crate) fn invalid_value(expected: &str, actual: impl std::fmt::Display) -> Error {
        Error::InvalidValue {
            path: smol_str::SmolStr::new_static("$"),
            expected: smol_str::SmolStr::new(expected),
            actual: smol_str::format_smolstr!("{actual}"),
        }
    }
}

// ------------------------------------------------------------------------
// Bounded selections over source-array rows.
// ------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub(crate) enum SourceSelection<'a> {
    Indices(&'a [u32]),
    Ranges(&'a [(usize, usize)]),
}

impl SourceSelection<'_> {
    pub(crate) fn row_count(self, upper_bound: usize) -> Result<usize> {
        let mut rows = 0usize;
        self.try_for_each(upper_bound, |_| {
            rows = rows.checked_add(1).ok_or_else(|| {
                Error::IncompatibleSchema(
                    "masked Arrow source selection length exceeds usize".to_owned(),
                )
            })?;
            Ok(())
        })?;
        Ok(rows)
    }

    pub(crate) fn try_for_each(
        self,
        upper_bound: usize,
        mut visit: impl FnMut(usize) -> Result<()>,
    ) -> Result<()> {
        match self {
            Self::Indices(indices) => {
                for index in indices {
                    let index = usize::try_from(*index).map_err(|_| {
                        Error::IncompatibleSchema(
                            "masked Arrow source index exceeds usize".to_owned(),
                        )
                    })?;
                    if index >= upper_bound {
                        return Err(Error::IncompatibleSchema(
                            "masked Arrow source index exceeds its array length".to_owned(),
                        ));
                    }
                    visit(index)?;
                }
            }
            Self::Ranges(ranges) => {
                for &(start, end) in ranges {
                    if start > end || end > upper_bound {
                        return Err(Error::IncompatibleSchema(
                            "masked Arrow source range exceeds its array length".to_owned(),
                        ));
                    }
                    for index in start..end {
                        visit(index)?;
                    }
                }
            }
        }
        Ok(())
    }
}

pub(crate) fn reserve_vec_bytes<T>(
    budget: &mut MaterializationBudget,
    capacity: usize,
) -> Result<()> {
    let bytes = std::mem::size_of::<T>()
        .checked_mul(capacity)
        .ok_or_else(|| {
            Error::IncompatibleSchema(
                "masked Arrow source scratch allocation exceeds usize".to_owned(),
            )
        })?;
    budget.add_bytes(bytes)
}

pub(crate) fn scratch_vec<T>(
    budget: &mut MaterializationBudget,
    capacity: usize,
    purpose: &str,
) -> Result<Vec<T>> {
    reserve_vec_bytes::<T>(budget, capacity)?;
    let mut values = Vec::new();
    values.try_reserve_exact(capacity).map_err(|error| {
        Error::IncompatibleSchema(format!("masked Arrow {purpose} allocation failed: {error}"))
    })?;
    Ok(values)
}

pub(crate) fn selected_child_ranges<Valid, Range>(
    selection: SourceSelection<'_>,
    parent_len: usize,
    child_len: usize,
    is_valid: Valid,
    range: Range,
    budget: &mut MaterializationBudget,
) -> Result<Vec<(usize, usize)>>
where
    Valid: Fn(usize) -> bool,
    Range: Fn(usize) -> Result<(usize, usize)>,
{
    let mut range_count = 0usize;
    let mut previous_end = None;
    selection.try_for_each(parent_len, |row| {
        if !is_valid(row) {
            return Ok(());
        }
        let (start, end) = range(row)?;
        if start > end || end > child_len {
            return Err(Error::IncompatibleSchema(
                "masked Arrow nested source range exceeds its child array".to_owned(),
            ));
        }
        if start != end {
            if previous_end != Some(start) {
                range_count = range_count.checked_add(1).ok_or_else(|| {
                    Error::IncompatibleSchema(
                        "masked Arrow nested range count exceeds usize".to_owned(),
                    )
                })?;
            }
            previous_end = Some(end);
        }
        Ok(())
    })?;

    let mut ranges = scratch_vec::<(usize, usize)>(budget, range_count, "range scratch")?;
    selection.try_for_each(parent_len, |row| {
        if !is_valid(row) {
            return Ok(());
        }
        let (start, end) = range(row)?;
        if start == end {
            return Ok(());
        }
        if let Some(previous) = ranges.last_mut() {
            if previous.1 == start {
                previous.1 = end;
                return Ok(());
            }
        }
        ranges.push((start, end));
        Ok(())
    })?;
    Ok(ranges)
}

fn reserve_byte_payload(
    selection: SourceSelection<'_>,
    array_len: usize,
    mut value_len: impl FnMut(usize) -> usize,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let mut bytes = 0usize;
    selection.try_for_each(array_len, |index| {
        bytes = bytes.checked_add(value_len(index)).ok_or_else(|| {
            Error::IncompatibleSchema(
                "masked Arrow selected source payload exceeds usize".to_owned(),
            )
        })?;
        Ok(())
    })?;
    budget.add_bytes(bytes)
}

/// The bytes a selection of one byte array's cells hold, text or binary alike.
fn reserve_selected_bytes<T: ByteArrayType>(
    array: &dyn Array,
    selection: SourceSelection<'_>,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let array = downcast::<GenericByteArray<T>>(array)?;
    reserve_byte_payload(
        selection,
        array.len(),
        |index| {
            if array.is_valid(index) {
                AsRef::<[u8]>::as_ref(array.value(index)).len()
            } else {
                0
            }
        },
        budget,
    )
}

/// One buffer handle per data buffer a view array shares.
fn reserve_view_buffers<T: ByteViewType>(
    array: &dyn Array,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let array = downcast::<GenericByteViewArray<T>>(array)?;
    reserve_vec_bytes::<arrow_buffer::Buffer>(budget, array.data_buffers().len())
}

/// The payload a selection of one string or byte column's cells hold, in
/// whichever storage its parameters lay out. A fixed width was charged by
/// the layout.
fn reserve_storage_source_payload(
    array: &dyn Array,
    storage: ArrowDataType,
    selection: SourceSelection<'_>,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    match storage {
        ArrowDataType::Utf8 => reserve_selected_bytes::<Utf8Type>(array, selection, budget),
        ArrowDataType::LargeUtf8 => {
            reserve_selected_bytes::<LargeUtf8Type>(array, selection, budget)
        }
        ArrowDataType::Binary => reserve_selected_bytes::<BinaryType>(array, selection, budget),
        ArrowDataType::LargeBinary => {
            reserve_selected_bytes::<LargeBinaryType>(array, selection, budget)
        }
        ArrowDataType::Utf8View => reserve_view_buffers::<StringViewType>(array, budget),
        ArrowDataType::BinaryView => reserve_view_buffers::<BinaryViewType>(array, budget),
        _ => Ok(()),
    }
}

fn reserve_run_source_take<R: RunEndIndexType>(
    source: &RunArray<R>,
    encoded: &crate::RunEndEncodedType,
    selection: SourceSelection<'_>,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let selected_count = selection.row_count(source.len())?;
    // Arrow's run take first expands every selected logical index to usize.
    reserve_vec_bytes::<usize>(budget, selected_count)?;

    let mut run_count = 0usize;
    let mut previous = None;
    selection.try_for_each(source.len(), |row| {
        let physical = source.get_physical_index(row);
        if previous != Some(physical) {
            run_count += 1;
            previous = Some(physical);
        }
        Ok(())
    })?;
    budget.add_array(encoded.run_ends().dtype(), run_count)?;
    budget.add_array_layout(&DataType::UInt32, run_count)?;

    let mut value_indices = Vec::new();
    value_indices
        .try_reserve_exact(run_count)
        .map_err(|error| {
            Error::IncompatibleSchema(format!(
                "masked Arrow run-value index allocation failed: {error}"
            ))
        })?;
    previous = None;
    selection.try_for_each(source.len(), |row| {
        let physical = source.get_physical_index(row);
        if previous != Some(physical) {
            value_indices.push(u32::try_from(physical).map_err(|_| {
                Error::IncompatibleSchema("masked Arrow run-value index exceeds UInt32".to_owned())
            })?);
            previous = Some(physical);
        }
        Ok(())
    })?;
    reserve_source_selection(
        source.values().as_ref(),
        encoded.values().dtype(),
        SourceSelection::Indices(&value_indices),
        budget,
    )
}

#[allow(clippy::too_many_lines)] // Mirrors Arrow's exhaustive take dispatch and sharing rules.
pub(crate) fn reserve_source_selection(
    array: &dyn Array,
    source_type: &DataType,
    selection: SourceSelection<'_>,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let selected_count = selection.row_count(array.len())?;
    budget.add_array_layout(source_type, selected_count)?;
    reserve_source_children_and_payload(array, source_type, selection, budget)
}

#[allow(clippy::too_many_lines)] // Mirrors Arrow's exhaustive take dispatch and sharing rules.
fn reserve_source_children_and_payload(
    array: &dyn Array,
    source_type: &DataType,
    selection: SourceSelection<'_>,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let selected_count = selection.row_count(array.len())?;
    match source_type {
        DataType::Bytes(parameters) => reserve_storage_source_payload(
            array,
            bytes::arrow_storage(*parameters)?,
            selection,
            budget,
        )?,
        DataType::String(parameters) => reserve_storage_source_payload(
            array,
            string::arrow_storage(*parameters)?,
            selection,
            budget,
        )?,
        DataType::Sequence(SequenceType::List(child)) => {
            let array = downcast::<ListArray>(array)?;
            let offsets = array.value_offsets();
            let ranges = selected_child_ranges(
                selection,
                array.len(),
                array.values().len(),
                |row| array.is_valid(row),
                |row| offset_pair(i64::from(offsets[row]), i64::from(offsets[row + 1])),
                budget,
            )?;
            reserve_source_selection(
                array.values().as_ref(),
                child.dtype(),
                SourceSelection::Ranges(&ranges),
                budget,
            )?;
        }
        DataType::Sequence(SequenceType::LargeList(child)) => {
            let array = downcast::<LargeListArray>(array)?;
            let offsets = array.value_offsets();
            let ranges = selected_child_ranges(
                selection,
                array.len(),
                array.values().len(),
                |row| array.is_valid(row),
                |row| offset_pair(offsets[row], offsets[row + 1]),
                budget,
            )?;
            reserve_source_selection(
                array.values().as_ref(),
                child.dtype(),
                SourceSelection::Ranges(&ranges),
                budget,
            )?;
        }
        DataType::Sequence(SequenceType::FixedSizeList(child, size)) => {
            let array = downcast::<FixedSizeListArray>(array)?;
            let size = usize::try_from(*size).map_err(|_| {
                Error::IncompatibleSchema(
                    "masked Arrow fixed-size-list width is negative".to_owned(),
                )
            })?;
            let ranges = selected_child_ranges(
                selection,
                array.len(),
                array.values().len(),
                |_| true,
                |row| {
                    let start = row.checked_mul(size).ok_or_else(|| {
                        Error::IncompatibleSchema(
                            "masked Arrow fixed-size-list offset exceeds usize".to_owned(),
                        )
                    })?;
                    let end = start.checked_add(size).ok_or_else(|| {
                        Error::IncompatibleSchema(
                            "masked Arrow fixed-size-list range exceeds usize".to_owned(),
                        )
                    })?;
                    Ok((start, end))
                },
                budget,
            )?;
            let child_count = SourceSelection::Ranges(&ranges).row_count(array.values().len())?;
            // Arrow expands fixed-list rows into a UInt32 child-index buffer.
            budget.add_array_layout(&DataType::UInt32, child_count)?;
            reserve_source_selection(
                array.values().as_ref(),
                child.dtype(),
                SourceSelection::Ranges(&ranges),
                budget,
            )?;
        }
        DataType::Structure(fields) => {
            let array = downcast::<StructArray>(array)?;
            if fields.len() != array.num_columns() {
                return Err(Error::IncompatibleSchema(
                    "masked Arrow source Struct child count changed after planning".to_owned(),
                ));
            }
            for (field, child) in fields.iter().zip(array.columns()) {
                reserve_source_selection(child.as_ref(), field.dtype(), selection, budget)?;
            }
        }
        DataType::Mapping(map) => {
            let array = downcast::<MapArray>(array)?;
            let offsets = array.value_offsets();
            let ranges = selected_child_ranges(
                selection,
                array.len(),
                array.entries().len(),
                |row| array.is_valid(row),
                |row| offset_pair(i64::from(offsets[row]), i64::from(offsets[row + 1])),
                budget,
            )?;
            reserve_source_selection(
                array.entries(),
                map.entries().dtype(),
                SourceSelection::Ranges(&ranges),
                budget,
            )?;
        }
        DataType::Union(fields, UnionMode::Sparse) => {
            let array = downcast::<UnionArray>(array)?;
            for (type_id, field) in fields {
                reserve_source_selection(
                    array.child(type_id).as_ref(),
                    field.dtype(),
                    selection,
                    budget,
                )?;
            }
        }
        DataType::Union(fields, UnionMode::Dense) => {
            let array = downcast::<UnionArray>(array)?;
            // Dense take keeps one row-sized mask and filtered-offset scratch
            // alive at a time while retaining every already-built child.
            budget.add_bitmap(selected_count)?;
            budget.add_array_layout(&DataType::UInt32, selected_count)?;
            let mut branch = Vec::new();
            branch.try_reserve_exact(selected_count).map_err(|error| {
                Error::IncompatibleSchema(format!(
                    "masked Arrow dense-union index allocation failed: {error}"
                ))
            })?;
            for (type_id, field) in fields {
                branch.clear();
                selection.try_for_each(array.len(), |row| {
                    if array.type_id(row) == type_id {
                        branch.push(u32::try_from(array.value_offset(row)).map_err(|_| {
                            Error::IncompatibleSchema(
                                "masked Arrow dense-union offset exceeds UInt32".to_owned(),
                            )
                        })?);
                    }
                    Ok(())
                })?;
                reserve_source_selection(
                    array.child(type_id).as_ref(),
                    field.dtype(),
                    SourceSelection::Indices(&branch),
                    budget,
                )?;
            }
        }
        DataType::RunEndEncoded(encoded) => match encoded.run_ends().dtype() {
            DataType::Int16 => reserve_run_source_take(
                downcast::<Int16RunArray>(array)?,
                encoded,
                selection,
                budget,
            )?,
            DataType::Int32 => reserve_run_source_take(
                downcast::<Int32RunArray>(array)?,
                encoded,
                selection,
                budget,
            )?,
            DataType::Int64 => reserve_run_source_take(
                downcast::<Int64RunArray>(array)?,
                encoded,
                selection,
                budget,
            )?,
            _ => {
                return Err(Error::IncompatibleSchema(
                    "masked Arrow run-end type must be Int16, Int32, or Int64".to_owned(),
                ));
            }
        },
        // List views and dictionaries share their child/value arrays. All
        // scalar/fixed-width storage was fully charged by the shallow layout.
        _ => {}
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
#[derive(Default)]
struct CountingWriter {
    bytes: usize,
}

impl std::fmt::Write for CountingWriter {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        self.bytes = self.bytes.checked_add(value.len()).ok_or(std::fmt::Error)?;
        Ok(())
    }
}

fn reserve_formatted_payload(
    array: &dyn Array,
    selection: SourceSelection<'_>,
    target_is_view: bool,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let options = FormatOptions::default();
    let formatter = ArrayFormatter::try_new(array, &options)?;
    let mut total = 0usize;
    let mut maximum = 0usize;
    selection.try_for_each(array.len(), |index| {
        if array.is_null(index) {
            return Ok(());
        }
        let mut counter = CountingWriter::default();
        formatter.value(index).write(&mut counter)?;
        total = total.checked_add(counter.bytes).ok_or_else(|| {
            Error::IncompatibleSchema("Arrow formatted payload exceeds usize".to_owned())
        })?;
        maximum = maximum.max(counter.bytes);
        Ok(())
    })?;
    budget.add_bytes(total)?;
    // Utf8View formatting keeps one growable String alive while appending the
    // final view buffers. Account for its largest selected logical value.
    if target_is_view {
        budget.add_bytes(maximum)?;
    }
    Ok(())
}

pub(crate) fn reserve_cast_output_payload(
    array: &dyn Array,
    source_type: &DataType,
    target_type: &DataType,
    selection: SourceSelection<'_>,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    match target_type {
        // A string's payload is its characters whatever charset writes them:
        // every charset here is at most one byte per scalar above US-ASCII,
        // so the UTF-8 rendering is the reservation's upper bound.
        DataType::String(parameters) => {
            reserve_formatted_payload(array, selection, parameters.layout().is_view(), budget)
        }
        // Offset storage copies every projected payload; a view shares its
        // buffers and a fixed width was charged by the layout.
        DataType::Bytes(parameters) => match bytes::arrow_storage(*parameters)? {
            ArrowDataType::Binary | ArrowDataType::LargeBinary => {
                let mut bytes = 0usize;
                selection.try_for_each(array.len(), |index| {
                    bytes = bytes
                        .checked_add(projected_byte_len(array, source_type, index)?)
                        .ok_or_else(|| {
                            Error::IncompatibleSchema(
                                "Arrow cast output payload exceeds usize".to_owned(),
                            )
                        })?;
                    Ok(())
                })?;
                budget.add_bytes(bytes)
            }
            _ => Ok(()),
        },
        DataType::Sequence(SequenceType::List(_))
        | DataType::Sequence(SequenceType::LargeList(_))
        | DataType::Sequence(SequenceType::FixedSizeList(..))
        | DataType::Structure(_)
        | DataType::Mapping(_)
        | DataType::Union(..)
        | DataType::Enum(EnumType::Dictionary(_))
        | DataType::RunEndEncoded(_) => {
            reserve_source_children_and_payload(array, source_type, selection, budget)
        }
        _ => Ok(()),
    }
}

pub(crate) fn reserve_selected_source_take(
    array: &dyn Array,
    selected: &[u32],
    target_type: &DataType,
    output_copies: usize,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let source_type = DataType::from_arrow_datatype(array.data_type())?;
    let selection = SourceSelection::Indices(selected);
    reserve_source_selection(array, &source_type, selection, budget)?;
    for _ in 0..output_copies {
        reserve_cast_output_payload(array, &source_type, target_type, selection, budget)?;
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
pub(crate) fn reserve_concat_copy(
    array: &dyn Array,
    dtype: &DataType,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    match dtype {
        DataType::Bytes(parameters) if parameters.is_view() => {
            let array = downcast::<BinaryViewArray>(array)?;
            budget.add_array_layout(dtype, array.len())?;
            reserve_vec_bytes::<arrow_buffer::Buffer>(budget, array.data_buffers().len())?;
        }
        // Dictionary inputs are vocabulary-aligned before concat, so Arrow
        // allocates only the concatenated key array and retains one vocab Arc.
        DataType::Enum(EnumType::Dictionary(_)) => budget.add_array_layout(dtype, array.len())?,
        DataType::Sequence(SequenceType::List(child)) => {
            let array = downcast::<ListArray>(array)?;
            budget.add_array_layout(dtype, array.len())?;
            let start = array.offsets()[0].as_usize();
            let end = array.offsets()[array.len()].as_usize();
            let values = array.values().slice(start, end - start);
            reserve_concat_copy(values.as_ref(), child.dtype(), budget)?;
        }
        DataType::Sequence(SequenceType::LargeList(child)) => {
            let array = downcast::<LargeListArray>(array)?;
            budget.add_array_layout(dtype, array.len())?;
            let start = array.offsets()[0].as_usize();
            let end = array.offsets()[array.len()].as_usize();
            let values = array.values().slice(start, end - start);
            reserve_concat_copy(values.as_ref(), child.dtype(), budget)?;
        }
        // Arrow concat preserves every ListView backing child, including
        // ranges not referenced by a logical view.
        DataType::Sequence(SequenceType::ListView(child)) => {
            let array = downcast::<ListViewArray>(array)?;
            budget.add_array_layout(dtype, array.len())?;
            reserve_concat_copy(array.values().as_ref(), child.dtype(), budget)?;
        }
        DataType::Sequence(SequenceType::LargeListView(child)) => {
            let array = downcast::<LargeListViewArray>(array)?;
            budget.add_array_layout(dtype, array.len())?;
            reserve_concat_copy(array.values().as_ref(), child.dtype(), budget)?;
        }
        DataType::Sequence(SequenceType::FixedSizeList(child, _)) => {
            let array = downcast::<FixedSizeListArray>(array)?;
            budget.add_array_layout(dtype, array.len())?;
            reserve_concat_copy(array.values().as_ref(), child.dtype(), budget)?;
        }
        DataType::Structure(fields) => {
            let array = downcast::<StructArray>(array)?;
            budget.add_array_layout(dtype, array.len())?;
            for (field, child) in fields.iter().zip(array.columns()) {
                reserve_concat_copy(child.as_ref(), field.dtype(), budget)?;
            }
        }
        DataType::Mapping(map) => {
            let array = downcast::<MapArray>(array)?;
            budget.add_array_layout(dtype, array.len())?;
            let start = array.offsets()[0].as_usize();
            let end = array.offsets()[array.len()].as_usize();
            let entries: ArrayRef = Arc::new(array.entries().slice(start, end - start));
            reserve_concat_copy(entries.as_ref(), map.entries().dtype(), budget)?;
        }
        // Union uses MutableArrayData, which visits every physical child, not
        // only the active branch selected by each logical row.
        DataType::Union(fields, _) => {
            let array = downcast::<UnionArray>(array)?;
            budget.add_array_layout(dtype, array.len())?;
            for (type_id, field) in fields {
                reserve_concat_copy(array.child(type_id).as_ref(), field.dtype(), budget)?;
            }
        }
        DataType::RunEndEncoded(encoded) => {
            macro_rules! reserve_run {
                ($run:ty) => {{
                    let array = downcast::<RunArray<$run>>(array)?;
                    let values = array.values_slice();
                    budget.add_physical_slots(values.len())?;
                    budget.add_array(encoded.run_ends().dtype(), values.len())?;
                    reserve_concat_copy(values.as_ref(), encoded.values().dtype(), budget)?
                }};
            }
            match encoded.run_ends().dtype() {
                DataType::Int16 => reserve_run!(Int16Type),
                DataType::Int32 => reserve_run!(Int32Type),
                DataType::Int64 => reserve_run!(Int64Type),
                _ => {
                    return Err(Error::IncompatibleSchema(
                        "run-end type is not a supported signed integer".to_owned(),
                    ));
                }
            }
        }
        _ => {
            let full = [(0, array.len())];
            reserve_source_selection(array, dtype, SourceSelection::Ranges(&full), budget)?;
        }
    }
    Ok(())
}

fn reserve_new_materialized_array_without_dictionary_values(
    output: &ArrayRef,
    source: &ArrayRef,
    dtype: &DataType,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    if Arc::ptr_eq(output, source) {
        return Ok(());
    }
    if !matches!(dtype, DataType::RunEndEncoded(_)) {
        budget.add_array_layout(dtype, output.len())?;
    }
    match dtype {
        DataType::Bytes(parameters) => match bytes::arrow_storage(*parameters)? {
            ArrowDataType::Binary => reserve_new_bytes::<BinaryType>(output, budget)?,
            ArrowDataType::LargeBinary => reserve_new_bytes::<LargeBinaryType>(output, budget)?,
            ArrowDataType::BinaryView => {
                reserve_new_views::<BinaryViewType>(output, source, budget)?;
            }
            // A fixed width was charged by the layout.
            _ => {}
        },
        DataType::String(parameters) => match string::arrow_storage(*parameters)? {
            ArrowDataType::Utf8 => reserve_new_bytes::<Utf8Type>(output, budget)?,
            ArrowDataType::LargeUtf8 => reserve_new_bytes::<LargeUtf8Type>(output, budget)?,
            ArrowDataType::Binary => reserve_new_bytes::<BinaryType>(output, budget)?,
            ArrowDataType::LargeBinary => reserve_new_bytes::<LargeBinaryType>(output, budget)?,
            ArrowDataType::Utf8View => reserve_new_views::<StringViewType>(output, source, budget)?,
            ArrowDataType::BinaryView => {
                reserve_new_views::<BinaryViewType>(output, source, budget)?;
            }
            // A fixed width was charged by the layout.
            _ => {}
        },
        DataType::Sequence(SequenceType::List(child)) => reserve_new_materialized_array_without_dictionary_values(
            downcast::<ListArray>(output.as_ref())?.values(),
            downcast::<ListArray>(source.as_ref())?.values(),
            child.dtype(),
            budget,
        )?,
        DataType::Sequence(SequenceType::LargeList(child)) => reserve_new_materialized_array_without_dictionary_values(
            downcast::<LargeListArray>(output.as_ref())?.values(),
            downcast::<LargeListArray>(source.as_ref())?.values(),
            child.dtype(),
            budget,
        )?,
        DataType::Sequence(SequenceType::ListView(child)) => reserve_new_materialized_array_without_dictionary_values(
            downcast::<ListViewArray>(output.as_ref())?.values(),
            downcast::<ListViewArray>(source.as_ref())?.values(),
            child.dtype(),
            budget,
        )?,
        DataType::Sequence(SequenceType::LargeListView(child)) => {
            reserve_new_materialized_array_without_dictionary_values(
                downcast::<LargeListViewArray>(output.as_ref())?.values(),
                downcast::<LargeListViewArray>(source.as_ref())?.values(),
                child.dtype(),
                budget,
            )?;
        }
        DataType::Sequence(SequenceType::FixedSizeList(child, _)) => {
            reserve_new_materialized_array_without_dictionary_values(
                downcast::<FixedSizeListArray>(output.as_ref())?.values(),
                downcast::<FixedSizeListArray>(source.as_ref())?.values(),
                child.dtype(),
                budget,
            )?;
        }
        DataType::Structure(fields) => {
            let output = downcast::<StructArray>(output.as_ref())?;
            let source = downcast::<StructArray>(source.as_ref())?;
            for ((field, output), source) in
                fields.iter().zip(output.columns()).zip(source.columns())
            {
                reserve_new_materialized_array_without_dictionary_values(
                    output,
                    source,
                    field.dtype(),
                    budget,
                )?;
            }
        }
        DataType::Mapping(map) => {
            let output: ArrayRef =
                Arc::new(downcast::<MapArray>(output.as_ref())?.entries().clone());
            let source: ArrayRef =
                Arc::new(downcast::<MapArray>(source.as_ref())?.entries().clone());
            reserve_new_materialized_array_without_dictionary_values(
                &output,
                &source,
                map.entries().dtype(),
                budget,
            )?;
        }
        DataType::Union(fields, _) => {
            let output = downcast::<UnionArray>(output.as_ref())?;
            let source = downcast::<UnionArray>(source.as_ref())?;
            for (type_id, field) in fields {
                reserve_new_materialized_array_without_dictionary_values(
                    output.child(type_id),
                    source.child(type_id),
                    field.dtype(),
                    budget,
                )?;
            }
        }
        DataType::RunEndEncoded(encoded) => {
            macro_rules! reserve_run {
                ($run:ty) => {{
                    let output = downcast::<RunArray<$run>>(output.as_ref())?;
                    let source = downcast::<RunArray<$run>>(source.as_ref())?;
                    budget.add_physical_slots(output.values().len())?;
                    budget.add_array(encoded.run_ends().dtype(), output.values().len())?;
                    reserve_new_materialized_array_without_dictionary_values(
                        output.values(),
                        source.values(),
                        encoded.values().dtype(),
                        budget,
                    )?;
                }};
            }
            match encoded.run_ends().dtype() {
                DataType::Int16 => reserve_run!(Int16Type),
                DataType::Int32 => reserve_run!(Int32Type),
                DataType::Int64 => reserve_run!(Int64Type),
                _ => {
                    return Err(Error::IncompatibleSchema(
                        "run-end type is not a supported signed integer".to_owned(),
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// The payload bytes one materialized byte array holds, text or binary alike.
fn reserve_new_bytes<T: ByteArrayType>(
    output: &ArrayRef,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let output = downcast::<GenericByteArray<T>>(output.as_ref())?;
    budget.add_bytes(checked_valid_payload_bytes(
        output.len(),
        |index| output.is_valid(index),
        |index| AsRef::<[u8]>::as_ref(output.value(index)).len(),
    )?)
}

/// The data buffers a materialized view array holds beyond its source's.
fn reserve_new_views<T: ByteViewType>(
    output: &ArrayRef,
    source: &ArrayRef,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    reserve_new_view_buffers(
        downcast::<GenericByteViewArray<T>>(output.as_ref())?.data_buffers(),
        downcast::<GenericByteViewArray<T>>(source.as_ref())?.data_buffers(),
        budget,
    )
}

fn reserve_new_view_buffers(
    output: &[arrow_buffer::Buffer],
    source: &[arrow_buffer::Buffer],
    budget: &mut MaterializationBudget,
) -> Result<()> {
    reserve_vec_bytes::<arrow_buffer::Buffer>(budget, output.len())?;
    let shared_prefix = output
        .iter()
        .zip(source)
        .take_while(|(output, source)| output.ptr_eq(source))
        .count();
    if output.len() == source.len() && shared_prefix == source.len() {
        return Ok(());
    }

    let phase = budget.mark();
    let mut source_buffers =
        scratch_vec::<(usize, usize)>(budget, source.len(), "ByteView shared-buffer identities")?;
    source_buffers.extend(
        source
            .iter()
            .map(|buffer| (buffer.as_ptr() as usize, buffer.len())),
    );
    source_buffers.sort_unstable();
    source_buffers.dedup();
    let mut new_buffers =
        scratch_vec::<(usize, usize)>(budget, output.len(), "ByteView new-buffer identities")?;
    for buffer in output {
        let identity = (buffer.as_ptr() as usize, buffer.len());
        if source_buffers.binary_search(&identity).is_err() {
            new_buffers.push(identity);
        }
    }
    new_buffers.sort_unstable();
    new_buffers.dedup();
    let new_bytes = new_buffers.iter().try_fold(0usize, |bytes, (_, len)| {
        bytes.checked_add(*len).ok_or_else(|| {
            Error::IncompatibleSchema("ByteView backing bytes exceed usize".to_owned())
        })
    })?;
    budget.restore(phase);
    budget.add_bytes(new_bytes)?;
    Ok(())
}

#[allow(clippy::too_many_lines)] // Mirrors every nested Arrow container layout.
pub(crate) fn reserve_new_dictionary_vocabularies(
    output: &ArrayRef,
    source: &ArrayRef,
    dtype: &DataType,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    match dtype {
        DataType::Enum(EnumType::Dictionary(dictionary)) => {
            let output_values = dictionary_values_ref(output.as_ref(), dictionary)?;
            let source_values = dictionary_values_ref(source.as_ref(), dictionary)?;
            let shared = if Arc::ptr_eq(output_values, source_values)
                || byte_array_storage_ptr_eq(
                    output_values.as_ref(),
                    source_values.as_ref(),
                    dictionary.value(),
                )? {
                true
            } else {
                let phase = budget.mark();
                reserve_to_data_scratch(output_values, budget)?;
                reserve_to_data_scratch(source_values, budget)?;
                let shared = output_values.to_data().ptr_eq(&source_values.to_data());
                budget.restore(phase);
                shared
            };
            if !shared {
                reserve_new_materialized_array_without_dictionary_values(
                    output_values,
                    source_values,
                    dictionary.value(),
                    budget,
                )?;
                reserve_new_dictionary_vocabularies(
                    output_values,
                    source_values,
                    dictionary.value(),
                    budget,
                )?;
            }
        }
        DataType::Structure(fields) => {
            let output = downcast::<StructArray>(output.as_ref())?;
            let source = downcast::<StructArray>(source.as_ref())?;
            for ((field, output), source) in
                fields.iter().zip(output.columns()).zip(source.columns())
            {
                reserve_new_dictionary_vocabularies(output, source, field.dtype(), budget)?;
            }
        }
        DataType::Sequence(SequenceType::List(child)) => reserve_new_dictionary_vocabularies(
            downcast::<ListArray>(output.as_ref())?.values(),
            downcast::<ListArray>(source.as_ref())?.values(),
            child.dtype(),
            budget,
        )?,
        DataType::Sequence(SequenceType::LargeList(child)) => reserve_new_dictionary_vocabularies(
            downcast::<LargeListArray>(output.as_ref())?.values(),
            downcast::<LargeListArray>(source.as_ref())?.values(),
            child.dtype(),
            budget,
        )?,
        DataType::Sequence(SequenceType::ListView(child)) => reserve_new_dictionary_vocabularies(
            downcast::<ListViewArray>(output.as_ref())?.values(),
            downcast::<ListViewArray>(source.as_ref())?.values(),
            child.dtype(),
            budget,
        )?,
        DataType::Sequence(SequenceType::LargeListView(child)) => reserve_new_dictionary_vocabularies(
            downcast::<LargeListViewArray>(output.as_ref())?.values(),
            downcast::<LargeListViewArray>(source.as_ref())?.values(),
            child.dtype(),
            budget,
        )?,
        DataType::Sequence(SequenceType::FixedSizeList(child, _)) => reserve_new_dictionary_vocabularies(
            downcast::<FixedSizeListArray>(output.as_ref())?.values(),
            downcast::<FixedSizeListArray>(source.as_ref())?.values(),
            child.dtype(),
            budget,
        )?,
        DataType::Mapping(map) => {
            let output: ArrayRef =
                Arc::new(downcast::<MapArray>(output.as_ref())?.entries().clone());
            let source: ArrayRef =
                Arc::new(downcast::<MapArray>(source.as_ref())?.entries().clone());
            reserve_new_dictionary_vocabularies(&output, &source, map.entries().dtype(), budget)?;
        }
        DataType::Union(fields, _) => {
            let output = downcast::<UnionArray>(output.as_ref())?;
            let source = downcast::<UnionArray>(source.as_ref())?;
            for (type_id, field) in fields {
                reserve_new_dictionary_vocabularies(
                    output.child(type_id),
                    source.child(type_id),
                    field.dtype(),
                    budget,
                )?;
            }
        }
        DataType::RunEndEncoded(encoded) => {
            macro_rules! reserve_run {
                ($run:ty) => {{
                    let output = downcast::<RunArray<$run>>(output.as_ref())?;
                    let source = downcast::<RunArray<$run>>(source.as_ref())?;
                    reserve_new_dictionary_vocabularies(
                        output.values(),
                        source.values(),
                        encoded.values().dtype(),
                        budget,
                    )?;
                }};
            }
            match encoded.run_ends().dtype() {
                DataType::Int16 => reserve_run!(Int16Type),
                DataType::Int32 => reserve_run!(Int32Type),
                DataType::Int64 => reserve_run!(Int64Type),
                _ => {
                    return Err(Error::IncompatibleSchema(
                        "run-end type is not a supported signed integer".to_owned(),
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

#[allow(clippy::too_many_lines)] // Mirrors Arrow ArrayData buffers and child trees.
pub(crate) fn reserve_to_data_scratch(
    array: &ArrayRef,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let buffer_count = match array.data_type() {
        ArrowDataType::Null
        | ArrowDataType::Struct(_)
        | ArrowDataType::FixedSizeList(_, _)
        | ArrowDataType::RunEndEncoded(_, _) => 0,
        ArrowDataType::Binary
        | ArrowDataType::LargeBinary
        | ArrowDataType::Utf8
        | ArrowDataType::LargeUtf8
        | ArrowDataType::ListView(_)
        | ArrowDataType::LargeListView(_)
        | ArrowDataType::Union(_, arrow_schema::UnionMode::Dense) => 2,
        ArrowDataType::BinaryView => downcast::<BinaryViewArray>(array.as_ref())?
            .data_buffers()
            .len()
            .checked_add(1)
            .ok_or_else(|| {
                Error::IncompatibleSchema("BinaryView buffer count exceeds usize".to_owned())
            })?,
        ArrowDataType::Utf8View => downcast::<StringViewArray>(array.as_ref())?
            .data_buffers()
            .len()
            .checked_add(1)
            .ok_or_else(|| {
                Error::IncompatibleSchema("Utf8View buffer count exceeds usize".to_owned())
            })?,
        _ => 1,
    };
    reserve_vec_bytes::<arrow_buffer::Buffer>(budget, buffer_count)?;

    macro_rules! one_child {
        ($child:expr) => {{
            reserve_vec_bytes::<arrow_data::ArrayData>(budget, 1)?;
            reserve_to_data_scratch($child, budget)?;
        }};
    }
    match array.data_type() {
        ArrowDataType::List(_) => one_child!(downcast::<ListArray>(array.as_ref())?.values()),
        ArrowDataType::LargeList(_) => {
            one_child!(downcast::<LargeListArray>(array.as_ref())?.values());
        }
        ArrowDataType::ListView(_) => {
            one_child!(downcast::<ListViewArray>(array.as_ref())?.values());
        }
        ArrowDataType::LargeListView(_) => {
            one_child!(downcast::<LargeListViewArray>(array.as_ref())?.values());
        }
        ArrowDataType::FixedSizeList(_, _) => {
            one_child!(downcast::<FixedSizeListArray>(array.as_ref())?.values());
        }
        ArrowDataType::Struct(fields) => {
            reserve_vec_bytes::<arrow_data::ArrayData>(budget, fields.len())?;
            for child in downcast::<StructArray>(array.as_ref())?.columns() {
                reserve_to_data_scratch(child, budget)?;
            }
        }
        ArrowDataType::Map(_, _) => {
            reserve_vec_bytes::<arrow_data::ArrayData>(budget, 1)?;
            let entries: ArrayRef =
                Arc::new(downcast::<MapArray>(array.as_ref())?.entries().clone());
            reserve_to_data_scratch(&entries, budget)?;
        }
        ArrowDataType::Dictionary(key, _) => {
            reserve_vec_bytes::<arrow_data::ArrayData>(budget, 1)?;
            macro_rules! values {
                ($key:ty) => {
                    reserve_to_data_scratch(
                        downcast::<DictionaryArray<$key>>(array.as_ref())?.values(),
                        budget,
                    )?
                };
            }
            match key.as_ref() {
                ArrowDataType::Int8 => values!(Int8Type),
                ArrowDataType::Int16 => values!(Int16Type),
                ArrowDataType::Int32 => values!(Int32Type),
                ArrowDataType::Int64 => values!(Int64Type),
                ArrowDataType::UInt8 => values!(UInt8Type),
                ArrowDataType::UInt16 => values!(UInt16Type),
                ArrowDataType::UInt32 => values!(UInt32Type),
                ArrowDataType::UInt64 => values!(UInt64Type),
                _ => {
                    return Err(Error::IncompatibleSchema(
                        "dictionary key is not a supported integer".to_owned(),
                    ));
                }
            }
        }
        ArrowDataType::Union(fields, _) => {
            reserve_vec_bytes::<arrow_data::ArrayData>(budget, fields.len())?;
            let array = downcast::<UnionArray>(array.as_ref())?;
            for (type_id, _) in fields.iter() {
                reserve_to_data_scratch(array.child(type_id), budget)?;
            }
        }
        ArrowDataType::RunEndEncoded(run_ends, _) => {
            reserve_vec_bytes::<arrow_data::ArrayData>(budget, 2)?;
            reserve_vec_bytes::<arrow_buffer::Buffer>(budget, 1)?;
            match run_ends.data_type() {
                ArrowDataType::Int16 => {
                    reserve_to_data_scratch(
                        downcast::<Int16RunArray>(array.as_ref())?.values(),
                        budget,
                    )?;
                }
                ArrowDataType::Int32 => {
                    reserve_to_data_scratch(
                        downcast::<Int32RunArray>(array.as_ref())?.values(),
                        budget,
                    )?;
                }
                ArrowDataType::Int64 => {
                    reserve_to_data_scratch(
                        downcast::<Int64RunArray>(array.as_ref())?.values(),
                        budget,
                    )?;
                }
                _ => {
                    return Err(Error::IncompatibleSchema(
                        "run-end type is not a supported signed integer".to_owned(),
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn reserve_field_default(
    field: &Field,
    rows: usize,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    if field.is_nullable() {
        budget.add_null_array(field.dtype(), rows)
    } else {
        budget.add_repeated_default(field.dtype(), rows)
    }
}

pub(crate) fn reserve_field_default_scalar(
    field: &Field,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    if field.is_nullable() {
        budget.add_null_scalar_scratch(field.dtype())
    } else {
        budget.add_default_scalar_scratch(field.dtype())
    }
}

pub(crate) fn reserve_missing_output(
    field: &Field,
    exposed: usize,
    hidden: usize,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    reserve_field_default(field, exposed, budget)?;
    budget.add_null_array(field.dtype(), hidden)
}
