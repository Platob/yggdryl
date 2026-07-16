//! The **nested** typed layer (`io::nested`) built on the root erased primitives (`AnySerie` /
//! `AnyField` / `AnyScalar`): the [`StructField`] / [`StructSerie`] struct family (↔ Arrow `Field` /
//! `Schema` / `StructArray` / `RecordBatch`) and the [`ListField`] / [`ListSerie`] list family (↔ an
//! Arrow `List` `Field` / `ListArray`). Structural round-trips run always; the Arrow interop is gated
//! on the `arrow` feature. Recursion, nullability, and byte-exact round-trips are the focus.
//!
//! Because a nested column's child can itself be any column — a leaf, a struct, a list, or a map —
//! the recursion is funneled through **one** central dispatch per direction:
//! [`read_any_column`] (byte codec) and [`from_arrow_any_column`] (Arrow import). Each family's own
//! reader/importer routes its children through these, so struct / list / map nesting round-trips with
//! no per-family child logic.

pub mod list;
pub mod map;
mod reshape;
pub mod struct_;

pub use list::{ListField, ListScalar, ListSerie, ListType};
pub use map::{MapField, MapScalar, MapSerie, MapType};
pub use struct_::{StructField, StructScalar, StructSerie, StructType};

use crate::io::{
    read_any_leaf, AnyField, AnyScalar, AnySerie, Bytes, DataTypeId, FieldType, IoError,
    PathSegment,
};

/// Reads **one erased column** of the type named by `field` from `source` — the single recursive
/// dispatch every nested reader routes its children through. A leaf delegates to
/// [`read_any_leaf`](crate::io::read_any_leaf); a struct to
/// [`StructSerie`](crate::io::nested::StructSerie)'s frame reader; a list to
/// [`ListSerie`](crate::io::nested::ListSerie)'s; a map to [`MapSerie`](crate::io::nested::MapSerie)'s.
pub fn read_any_column(field: &AnyField, source: &mut Bytes) -> Result<Box<dyn AnySerie>, IoError> {
    read_any_column_at(field, source, 0)
}

/// Reads one erased column at recursion `depth`, refusing a frame nested past
/// [`AnyField::MAX_NESTING`](crate::io::AnyField) with a guided
/// [`NestingTooDeep`](IoError::NestingTooDeep). This is the single point every nested child read
/// funnels through, so a hostile frame whose per-level schemas each stay shallow (evading the schema
/// decoder's own cap) but whose **data** chains arbitrarily deep still cannot overflow the stack.
pub(crate) fn read_any_column_at(
    field: &AnyField,
    source: &mut Bytes,
    depth: usize,
) -> Result<Box<dyn AnySerie>, IoError> {
    if depth > AnyField::MAX_NESTING {
        return Err(IoError::NestingTooDeep {
            max: AnyField::MAX_NESTING,
        });
    }
    if field.is_struct() {
        Ok(Box::new(struct_::StructSerie::read_frame(source, depth)?))
    } else if field.is_list() {
        Ok(Box::new(list::ListSerie::read_frame(source, depth)?))
    } else if field.is_map() {
        Ok(Box::new(map::MapSerie::read_frame(source, depth)?))
    } else {
        read_any_leaf(field, source)
    }
}

/// Resolves one path `segment` from a **nested** column `container` to its addressed child column
/// **mutably** — the `&mut` mirror of the immutable [`child_serie_by`](crate::io::AnySerie::child_serie_by)
/// / [`child_serie_at`](crate::io::AnySerie::child_serie_at), driving the deep-cell setter's interior
/// walk ([`AnySerie::set_by_path`](crate::io::AnySerie::set_by_path)). A leaf column has no children,
/// so it returns `None`.
///
/// This is the **one** place that names the concrete nested types for a `&mut` child: it reads the
/// container's [`type_id`](crate::io::AnySerie::type_id) (an immutable probe that ends at once) to pick
/// the family, then does a **single** [`as_any_mut`](crate::io::AnySerie::as_any_mut) downcast and calls
/// that family's `pub(crate)` `child_serie_{by,at}_mut`. Kept crate-internal so no public `&mut` child
/// leaks (a raw child would let safe code grow it and desync the parent's length invariant).
pub(crate) fn child_serie_mut<'a>(
    container: &'a mut (dyn AnySerie + 'static),
    segment: &PathSegment,
) -> Option<&'a mut (dyn AnySerie + 'static)> {
    match container.type_id() {
        DataTypeId::Struct => {
            let column = container
                .as_any_mut()
                .downcast_mut::<struct_::StructSerie>()?;
            match segment {
                PathSegment::Name(name) => column.child_serie_by_mut(name),
                PathSegment::Index(index) => column.child_serie_at_mut(*index),
            }
        }
        DataTypeId::List => {
            let column = container.as_any_mut().downcast_mut::<list::ListSerie>()?;
            match segment {
                PathSegment::Name(name) => column.child_serie_by_mut(name),
                PathSegment::Index(index) => column.child_serie_at_mut(*index),
            }
        }
        DataTypeId::Map => {
            let column = container.as_any_mut().downcast_mut::<map::MapSerie>()?;
            match segment {
                PathSegment::Name(name) => column.child_serie_by_mut(name),
                PathSegment::Index(index) => column.child_serie_at_mut(*index),
            }
        }
        _ => None, // a leaf column has no child columns
    }
}

/// Fills the nulls of **one child column** for a nested [`fill_null`](crate::io::AnySerie::fill_null):
/// recurses into the child iff it is nested *or* a leaf whose type matches `value` (so a matching
/// leaf's nulls are replaced), else returns the child unchanged — a leaf of a different type is left
/// alone, so a nested fill over a heterogeneous struct is **lenient** (it never errors, it just fills
/// what matches). A [`Null`](crate::io::AnyScalar::Null) `value` is always a clone (filling with a
/// null is the identity). Shared by [`StructSerie`] / [`ListSerie`]'s `fill_null`.
pub(crate) fn fill_null_child(
    child: &(dyn AnySerie + 'static),
    value: &AnyScalar,
) -> Result<Box<dyn AnySerie>, IoError> {
    if value.is_null() {
        return Ok(child.clone_box());
    }
    let child_id = child.type_id();
    if child_id.is_nested() || value.type_id() == Some(child_id) {
        child.fill_null(value)
    } else {
        Ok(child.clone_box())
    }
}

/// An **empty erased column** matching `field`'s type (leaf or nested) — the single recursive
/// dispatch every nested `empty` constructor routes its children through. A struct / list recurses;
/// a leaf reconstructs from a zero-length frame via its `Serie` codec.
pub(crate) fn empty_any_column(field: &AnyField) -> Box<dyn AnySerie> {
    if field.is_struct() {
        Box::new(struct_::StructSerie::empty(
            &StructField::from_any_field(field.clone())
                .expect("a struct field decodes to a StructField"),
        ))
    } else if field.is_list() {
        Box::new(list::ListSerie::empty(
            &ListField::from_any_field(field.clone()).expect("a list field decodes to a ListField"),
        ))
    } else if field.is_map() {
        Box::new(map::MapSerie::empty(
            &MapField::from_any_field(field.clone()).expect("a map field decodes to a MapField"),
        ))
    } else {
        empty_leaf_column(field)
    }
}

/// An empty leaf column matching `field`'s leaf type — a zero-length `Serie` frame decoded through
/// [`read_any_leaf`](crate::io::read_any_leaf).
fn empty_leaf_column(field: &AnyField) -> Box<dyn AnySerie> {
    let empty = if FieldType::type_id(field).is_null() {
        0u64.to_le_bytes().to_vec()
    } else if FieldType::type_id(field).is_variable_length() {
        // `[len=0][no validity][offset 0][data_len 0]`
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes
    } else {
        // `[len=0][no validity]`
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.push(0);
        bytes
    };
    read_any_leaf(field, &mut Bytes::from_slice(&empty)).expect("a zero-length leaf frame is valid")
}

/// Imports **one erased column** from an Arrow array + its [`Field`](arrow_schema::Field) (feature
/// `arrow`) — the single recursive dispatch every nested importer routes its children through. A
/// struct / list / map recurses; every other type delegates to
/// [`from_arrow_any_leaf`](crate::io::from_arrow_any_leaf).
#[cfg(feature = "arrow")]
pub fn from_arrow_any_column(
    array: &dyn arrow_array::Array,
    field: &arrow_schema::Field,
) -> Result<Box<dyn AnySerie>, IoError> {
    match field.data_type() {
        arrow_schema::DataType::Struct(_) => {
            let struct_array = array
                .as_any()
                .downcast_ref::<arrow_array::StructArray>()
                .ok_or_else(|| IoError::Unsupported {
                    what: format!(
                        "expected an Arrow StructArray for field {:?}, got {:?}",
                        field.name(),
                        array.data_type()
                    ),
                })?;
            Ok(Box::new(struct_::StructSerie::from_arrow_array(
                struct_array,
                field,
            )?))
        }
        arrow_schema::DataType::List(_) => {
            Ok(Box::new(list::ListSerie::from_arrow_array(array, field)?))
        }
        arrow_schema::DataType::Map(_, _) => {
            Ok(Box::new(map::MapSerie::from_arrow_array(array, field)?))
        }
        _ => crate::io::from_arrow_any_leaf(array, field),
    }
}

#[cfg(test)]
mod nesting_guard_tests {
    use crate::io::nested::StructSerie;
    use crate::io::{AnyField, Headers, IoError};

    /// A single struct frame: a struct with one **struct** child `"c"` and zero rows, whose child
    /// *data* is the next frame in the chain. Every frame's schema stays shallow (a struct wrapping
    /// an empty struct field), so the schema decoder's own depth cap never fires — only the serie
    /// `read_any_column` recursion guard does, which is exactly what this exercises.
    fn one_chained_struct_frame() -> Vec<u8> {
        let child = AnyField::struct_("c", Vec::new(), false);
        let mut schema = Vec::new();
        AnyField::encode_struct(
            "",
            false,
            &Headers::new(),
            std::slice::from_ref(&child),
            &mut schema,
        );
        let mut frame = Vec::new();
        frame.extend_from_slice(&(schema.len() as u64).to_le_bytes());
        frame.extend_from_slice(&schema);
        frame.extend_from_slice(&0u64.to_le_bytes()); // len = 0 rows
        frame.push(0u8); // validity flag = none
        frame
    }

    #[test]
    fn deeply_chained_struct_frame_reads_to_a_guided_error_not_a_crash() {
        // Chain many more frames than the depth cap; the reader recurses one frame per nesting level
        // (each schema shallow) and the `read_any_column` depth guard fires before the chain — or
        // the stack — is exhausted. Before the fix this overflowed the stack and aborted.
        let frame = one_chained_struct_frame();
        let mut bytes = Vec::with_capacity(frame.len() * (AnyField::MAX_NESTING + 50));
        for _ in 0..(AnyField::MAX_NESTING + 50) {
            bytes.extend_from_slice(&frame);
        }
        let err = StructSerie::deserialize_bytes(&bytes).unwrap_err();
        assert!(
            matches!(err, IoError::NestingTooDeep { max } if max == AnyField::MAX_NESTING),
            "got {err:?}"
        );
    }
}
