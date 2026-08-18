//! The Avro schema model and its JSON parser.

use std::collections::HashMap;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::{Result, Value};

use super::datum::invalid;

/// One Avro schema node.
///
/// Only the branches Iceberg's manifest schemas actually use are modeled. A
/// logical type annotation is deliberately not modeled: it never changes the
/// physical encoding, and the manifest layer above knows what a field means.
#[derive(Clone, Debug)]
pub(crate) enum Schema {
    /// The empty type, encoded as zero bytes.
    Null,
    /// A single byte holding zero or one.
    Boolean,
    /// A zig-zag variable-length 32-bit integer.
    Int,
    /// A zig-zag variable-length 64-bit integer.
    Long,
    /// Four little-endian IEEE 754 bytes.
    Float,
    /// Eight little-endian IEEE 754 bytes.
    Double,
    /// A length-prefixed byte string.
    Bytes,
    /// A length-prefixed UTF-8 string.
    String,
    /// Named ordered fields, encoded back to back.
    Record(Arc<Record>),
    /// A symbol chosen by index.
    Enum(Arc<[SmolStr]>),
    /// Length-prefixed blocks of one item type.
    Array(Arc<Schema>),
    /// Length-prefixed blocks of string-keyed values.
    Map(Arc<Schema>),
    /// A branch index followed by that branch's value.
    Union(Arc<[Schema]>),
    /// Exactly `size` raw bytes.
    Fixed(usize),
}

/// The fields of one Avro record type, in declaration order.
#[derive(Clone, Debug)]
pub(crate) struct Record {
    /// The record's declared name, which later references resolve against.
    pub(crate) name: SmolStr,
    /// Field names paired with their schemas, in encoding order.
    pub(crate) fields: Vec<(SmolStr, Schema)>,
}

impl Schema {
    /// Read an Avro schema from its JSON representation.
    ///
    /// # Errors
    ///
    /// Returns an error when the document is not a schema this implementation
    /// covers, naming the construct that was found.
    pub(crate) fn from_json(document: &Value) -> Result<Self> {
        let mut named = HashMap::new();
        parse_schema(document, &mut named)
    }

    /// Return the name a caller would use to refer to this schema.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean => "boolean",
            Self::Int => "int",
            Self::Long => "long",
            Self::Float => "float",
            Self::Double => "double",
            Self::Bytes => "bytes",
            Self::String => "string",
            Self::Record(_) => "record",
            Self::Enum(_) => "enum",
            Self::Array(_) => "array",
            Self::Map(_) => "map",
            Self::Union(_) => "union",
            Self::Fixed(_) => "fixed",
        }
    }
}

/// Parse one schema node, registering any named type it declares.
fn parse_schema(document: &Value, named: &mut HashMap<SmolStr, Schema>) -> Result<Schema> {
    if let Some(name) = document.as_str() {
        return resolve_name(name, named);
    }
    if let Some(branches) = document.as_sequence() {
        let mut parsed = Vec::with_capacity(branches.len());
        for branch in branches {
            parsed.push(parse_schema(branch, named)?);
        }
        return Ok(Schema::Union(parsed.into()));
    }
    if document.as_mapping().is_none() {
        return Err(invalid(format_smolstr!(
            "expected an Avro schema name, union, or object, got {}",
            document.kind()
        )));
    }

    let type_name = document
        .get_key_str("type")
        .ok_or_else(|| invalid(SmolStr::new_static("expected an Avro schema \"type\"")))?;
    // A `type` may itself be a nested schema, which is how Iceberg spells a
    // logical annotation over an array or a union.
    let Some(type_name) = type_name.as_str() else {
        return parse_schema(type_name, named);
    };

    match type_name {
        "record" | "error" => parse_record(document, named),
        "enum" => {
            let symbols = document
                .get_key_str("symbols")
                .and_then(Value::as_sequence)
                .ok_or_else(|| {
                    invalid(SmolStr::new_static(
                        "expected an Avro enum \"symbols\" array",
                    ))
                })?;
            let mut names = Vec::with_capacity(symbols.len());
            for symbol in symbols {
                names.push(SmolStr::new(symbol.as_str().ok_or_else(|| {
                    invalid(format_smolstr!(
                        "expected an Avro enum symbol string, got {}",
                        symbol.kind()
                    ))
                })?));
            }
            let schema = Schema::Enum(names.into());
            register(document, &schema, named);
            Ok(schema)
        }
        "array" => {
            let items = document.get_key_str("items").ok_or_else(|| {
                invalid(SmolStr::new_static(
                    "expected an Avro array \"items\" schema",
                ))
            })?;
            Ok(Schema::Array(Arc::new(parse_schema(items, named)?)))
        }
        "map" => {
            let values = document.get_key_str("values").ok_or_else(|| {
                invalid(SmolStr::new_static(
                    "expected an Avro map \"values\" schema",
                ))
            })?;
            Ok(Schema::Map(Arc::new(parse_schema(values, named)?)))
        }
        "fixed" => {
            let size = document
                .get_key_str("size")
                .and_then(Value::as_i64)
                .and_then(|size| usize::try_from(size).ok())
                .ok_or_else(|| {
                    invalid(SmolStr::new_static(
                        "expected a non-negative Avro fixed \"size\"",
                    ))
                })?;
            let schema = Schema::Fixed(size);
            register(document, &schema, named);
            Ok(schema)
        }
        other => resolve_name(other, named),
    }
}

/// Parse a record schema and register it before its own fields are read.
fn parse_record(document: &Value, named: &mut HashMap<SmolStr, Schema>) -> Result<Schema> {
    let name = document
        .get_key_str("name")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(SmolStr::new_static("expected an Avro record \"name\"")))?;
    let entries = document
        .get_key_str("fields")
        .and_then(Value::as_sequence)
        .ok_or_else(|| {
            invalid(format_smolstr!(
                "expected an Avro record \"fields\" array on {name:?}"
            ))
        })?;

    let mut fields = Vec::with_capacity(entries.len());
    for entry in entries {
        let field_name = entry
            .get_key_str("name")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected an Avro field \"name\" inside {name:?}"
                ))
            })?;
        let field_type = entry.get_key_str("type").ok_or_else(|| {
            invalid(format_smolstr!(
                "expected an Avro field \"type\" on {name:?}.{field_name}"
            ))
        })?;
        fields.push((SmolStr::new(field_name), parse_schema(field_type, named)?));
    }

    let schema = Schema::Record(Arc::new(Record {
        name: SmolStr::new(name),
        fields,
    }));
    named.insert(SmolStr::new(name), schema.clone());
    Ok(schema)
}

/// Remember a named schema so a later reference by name resolves.
fn register(document: &Value, schema: &Schema, named: &mut HashMap<SmolStr, Schema>) {
    if let Some(name) = document.get_key_str("name").and_then(Value::as_str) {
        named.insert(SmolStr::new(name), schema.clone());
    }
}

/// Resolve a bare schema name: a primitive, or a type declared earlier.
fn resolve_name(name: &str, named: &HashMap<SmolStr, Schema>) -> Result<Schema> {
    Ok(match name {
        "null" => Schema::Null,
        "boolean" => Schema::Boolean,
        "int" => Schema::Int,
        "long" => Schema::Long,
        "float" => Schema::Float,
        "double" => Schema::Double,
        "bytes" => Schema::Bytes,
        "string" => Schema::String,
        other => named.get(other).cloned().ok_or_else(|| {
            invalid(format_smolstr!(
                "expected an Avro primitive name or a type declared earlier, got {other:?}"
            ))
        })?,
    })
}
