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
pub mod struct_;

pub use list::{ListField, ListScalar, ListSerie, ListType};
pub use map::{MapField, MapScalar, MapSerie, MapType};
pub use struct_::{StructField, StructScalar, StructSerie, StructType};

use crate::io::{read_any_leaf, AnyField, AnySerie, Bytes, FieldType, IoError};

/// Reads **one erased column** of the type named by `field` from `source` — the single recursive
/// dispatch every nested reader routes its children through. A leaf delegates to
/// [`read_any_leaf`](crate::io::read_any_leaf); a struct to
/// [`StructSerie`](crate::io::nested::StructSerie)'s frame reader; a list to
/// [`ListSerie`](crate::io::nested::ListSerie)'s; a map to [`MapSerie`](crate::io::nested::MapSerie)'s.
pub fn read_any_column(field: &AnyField, source: &mut Bytes) -> Result<Box<dyn AnySerie>, IoError> {
    if field.is_struct() {
        Ok(Box::new(struct_::StructSerie::read_frame(source)?))
    } else if field.is_list() {
        Ok(Box::new(list::ListSerie::read_frame(source)?))
    } else if field.is_map() {
        Ok(Box::new(map::MapSerie::read_frame(source)?))
    } else {
        read_any_leaf(field, source)
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
