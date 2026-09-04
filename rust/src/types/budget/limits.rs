//! Bounded Arrow materialization accounting.

use crate::arrow::{Error, Result};
use crate::{DataType, Field, Scalar, TimeUnit, UnionMode};

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
            DataType::FixedSizeList(child, size) => {
                self.add_array_layout(dtype, rows)?;
                let size = usize::try_from(*size)
                    .map_err(|_| invalid_value("a fixed list size within usize", size))?;
                let child_rows =
                    checked_physical_mul(rows, size, "fixed-size-list slots", MAX_PHYSICAL_SLOTS)?;
                self.add_repeated_field_default(child, child_rows, include_dictionary_values)
            }
            DataType::Struct(fields) => {
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
            DataType::Dictionary(dictionary) => {
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
            DataType::FixedSizeList(child, size) => {
                self.add_array_layout_without_slots(dtype, 1)?;
                let size = usize::try_from(*size)
                    .map_err(|_| invalid_value("a fixed list size within usize", size))?;
                self.add_repeated_field_default(child, size, true)
            }
            DataType::Struct(fields) => {
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
            DataType::Dictionary(dictionary) => {
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
            DataType::FixedSizeList(child, size) => {
                let size = usize::try_from(*size)
                    .map_err(|_| invalid_value("a fixed list size within usize", size))?;
                let child_rows =
                    checked_physical_mul(rows, size, "fixed-size-list slots", MAX_PHYSICAL_SLOTS)?;
                self.add_array(child.dtype(), child_rows)?;
            }
            DataType::Struct(fields) => {
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
            DataType::Dictionary(dictionary) => {
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
            | DataType::Decimal32 { .. }
            | DataType::Mic => self.add_fixed_rows(rows, 4)?,
            DataType::Country => self.add_fixed_rows(rows, 2)?,
            DataType::Currency => self.add_fixed_rows(rows, 3)?,
            DataType::Cfi => self.add_fixed_rows(rows, 6)?,
            // A fixed ASCII column charges the width it stores, whatever it is.
            DataType::FixedAscii(width) => {
                self.add_fixed_rows(rows, usize::try_from(*width).unwrap_or(0))?;
            }
            DataType::Int64
            | DataType::UInt64
            | DataType::Float64
            | DataType::Timestamp(..)
            | DataType::Date64
            | DataType::Time64(_)
            | DataType::Duration32(_)
            | DataType::Duration64(_)
            | DataType::Interval(TimeUnit::DayTime)
            | DataType::Decimal64 { .. }
            | DataType::ListView(_) => self.add_fixed_rows(rows, 8)?,
            DataType::Interval(TimeUnit::MonthDayNano)
            | DataType::Decimal128 { .. }
            | DataType::BinaryView
            | DataType::Utf8View
            | DataType::Guid
            | DataType::LargeListView(_) => {
                self.add_fixed_rows(rows, 16)?;
            }
            DataType::Decimal256 { .. } => self.add_fixed_rows(rows, 32)?,
            DataType::Interval(_) => {
                return Err(unsupported(dtype, "invalid interval layout"));
            }
            DataType::Binary
            | DataType::Utf8
            | DataType::Ascii
            | DataType::List(_)
            | DataType::Map(_)
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
            DataType::LargeBinary | DataType::LargeUtf8 | DataType::LargeList(_) => {
                self.add_offsets(rows, 8)?;
            }
            DataType::FixedSizeBinary(width) => {
                let width = usize::try_from(*width)
                    .map_err(|_| invalid_value("a fixed binary width within usize", width))?;
                self.add_fixed_rows(rows, width)?;
            }
            DataType::Null
            | DataType::FixedSizeList(..)
            | DataType::Struct(_)
            | DataType::RunEndEncoded(_) => {}
            DataType::Union(_, mode) => self.add_union_buffers(rows, *mode)?,
            DataType::Dictionary(dictionary) => {
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
            | DataType::Decimal32 { .. }
            | DataType::Mic => self.add_fixed_rows(rows, 4)?,
            DataType::Country => self.add_fixed_rows(rows, 2)?,
            DataType::Currency => self.add_fixed_rows(rows, 3)?,
            DataType::Cfi => self.add_fixed_rows(rows, 6)?,
            // A fixed ASCII column charges the width it stores, whatever it is.
            DataType::FixedAscii(width) => {
                self.add_fixed_rows(rows, usize::try_from(*width).unwrap_or(0))?;
            }
            DataType::Int64
            | DataType::UInt64
            | DataType::Float64
            | DataType::Timestamp(..)
            | DataType::Date64
            | DataType::Time64(_)
            | DataType::Duration32(_)
            | DataType::Duration64(_)
            | DataType::Interval(TimeUnit::DayTime)
            | DataType::Decimal64 { .. }
            | DataType::ListView(_) => self.add_fixed_rows(rows, 8)?,
            DataType::Interval(TimeUnit::MonthDayNano)
            | DataType::Decimal128 { .. }
            | DataType::BinaryView
            | DataType::Utf8View
            | DataType::Guid
            | DataType::LargeListView(_) => self.add_fixed_rows(rows, 16)?,
            DataType::Decimal256 { .. } => self.add_fixed_rows(rows, 32)?,
            DataType::Interval(_) => {
                return Err(unsupported(dtype, "invalid interval layout"));
            }
            DataType::Binary
            | DataType::Utf8
            | DataType::Ascii
            | DataType::List(_)
            | DataType::Map(_)
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
            DataType::LargeBinary | DataType::LargeUtf8 | DataType::LargeList(_) => {
                self.add_offsets(rows, 8)?;
            }
            DataType::FixedSizeBinary(width) => {
                let width = usize::try_from(*width)
                    .map_err(|_| invalid_value("a fixed binary width within usize", width))?;
                self.add_fixed_rows(rows, width)?;
            }
            DataType::FixedSizeList(child, size) => {
                let size = usize::try_from(*size)
                    .map_err(|_| invalid_value("a fixed list size within usize", size))?;
                let child_rows =
                    checked_physical_mul(rows, size, "fixed-size-list slots", MAX_PHYSICAL_SLOTS)?;
                self.add_null_array(child.dtype(), child_rows)?;
            }
            DataType::Struct(fields) => {
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
            DataType::Dictionary(dictionary) => {
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
