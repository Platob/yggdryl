//! Compact, human-readable **type signatures** — the pretty form behind
//! [`DataType::display`](crate::DataType::display), for fast debugging.
//!
//! A signature is our lowercase name plus, for a container, its children in angle
//! brackets: `int64`, `utf8`, `list<int64>`, `struct<x: int64, y: float64>`,
//! `map<utf8, int64>`, `optional<int64>`. It is built from the type's
//! [`arrow_schema::DataType`], so one recursive walk renders every type — including
//! the nested children a container hands back as Arrow types.

use arrow_schema::DataType;

/// The compact signature of an [`arrow_schema::DataType`], recursively — the shared
/// renderer behind [`DataType::display`](crate::DataType::display). An `optional` is
/// stored as a two-variant `null`-or-value union, so a union of exactly that shape
/// renders as `optional<…>` rather than `union<…>`.
///
/// ```
/// use yggdryl_dtype::arrow_schema::{DataType, Field, Fields};
///
/// assert_eq!(yggdryl_dtype::signature(&DataType::Int64), "int64");
/// let list = DataType::List(std::sync::Arc::new(Field::new("item", DataType::Utf8, true)));
/// assert_eq!(yggdryl_dtype::signature(&list), "list<utf8>");
/// let point = DataType::Struct(Fields::from(vec![
///     Field::new("x", DataType::Int64, false),
///     Field::new("y", DataType::Float64, false),
/// ]));
/// assert_eq!(yggdryl_dtype::signature(&point), "struct<x: int64, y: float64>");
/// ```
pub fn signature(data_type: &DataType) -> String {
    match data_type {
        DataType::Null => "null".to_string(),
        DataType::Boolean => "bool".to_string(),
        DataType::Int8 => "int8".to_string(),
        DataType::Int16 => "int16".to_string(),
        DataType::Int32 => "int32".to_string(),
        DataType::Int64 => "int64".to_string(),
        DataType::UInt8 => "uint8".to_string(),
        DataType::UInt16 => "uint16".to_string(),
        DataType::UInt32 => "uint32".to_string(),
        DataType::UInt64 => "uint64".to_string(),
        DataType::Float16 => "float16".to_string(),
        DataType::Float32 => "float32".to_string(),
        DataType::Float64 => "float64".to_string(),
        DataType::Binary | DataType::LargeBinary => "binary".to_string(),
        DataType::Utf8 | DataType::LargeUtf8 => "utf8".to_string(),
        DataType::List(item) | DataType::LargeList(item) => {
            format!("list<{}>", signature(item.data_type()))
        }
        DataType::Struct(fields) => {
            let inner = fields
                .iter()
                .map(|field| format!("{}: {}", field.name(), signature(field.data_type())))
                .collect::<Vec<_>>()
                .join(", ");
            format!("struct<{inner}>")
        }
        DataType::Map(entries, _) => match entries.data_type() {
            // The entries child is a `struct<key, value>`.
            DataType::Struct(kv) if kv.len() == 2 => {
                format!(
                    "map<{}, {}>",
                    signature(kv[0].data_type()),
                    signature(kv[1].data_type())
                )
            }
            _ => "map".to_string(),
        },
        DataType::Union(fields, _) => {
            let variants: Vec<_> = fields.iter().map(|(_, field)| field).collect();
            // The `optional` shape: a `null` variant paired with one value variant.
            if variants.len() == 2 && matches!(variants[0].data_type(), DataType::Null) {
                format!("optional<{}>", signature(variants[1].data_type()))
            } else {
                let inner = variants
                    .iter()
                    .map(|field| signature(field.data_type()))
                    .collect::<Vec<_>>()
                    .join(" | ");
                format!("union<{inner}>")
            }
        }
        // Any Arrow type without a bespoke signature falls back to its lowercase
        // Arrow name (e.g. a future temporal type), so the display never panics.
        other => other.to_string().to_lowercase(),
    }
}
