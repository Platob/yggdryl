//! Iceberg schemas as core [`Field`] values.
//!
//! An Iceberg schema is a struct with numbered fields. That is exactly a
//! non-null struct [`Field`] whose children carry `PARQUET:field_id`, so this
//! module converts rather than mirrors: reading an Iceberg schema produces a
//! field the rest of the crate already understands, and writing one reads the
//! ids back off that field.
//!
//! Identifier assignment is automatic where the table can do it itself - a
//! creation or an evolution numbers whatever arrives unnumbered - and
//! [`assign_field_ids`] remains for the caller who needs the numbering before
//! the table exists, such as building a [`super::PartitionSpec`] by hand or
//! emitting a schema document for another system. It numbers a field tree
//! depth-first from a starting id; a field that already carries an id keeps it.
//!
//! Everything a field cannot hold structurally - the schema identifier, a
//! column's documentation, the v3 default values - is kept as Iceberg protocol
//! properties, so re-emitting a document reproduces it rather than quietly
//! dropping what the field model has no slot for. Those properties are reached
//! through [`Field::iceberg`] and [`Field::iceberg_mut`], which remember the
//! `iceberg:` prefix so this module never spells a metadata key itself.

use smol_str::{SmolStr, format_smolstr};

use super::PrimitiveType;
use crate::{DataType, Error, Field, Result, Scalar};

/// The Iceberg property naming a schema identifier.
pub(super) const SCHEMA_ID: &str = "schema-id";

/// The Iceberg property holding a column's documentation string.
pub(super) const DOC: &str = "doc";

/// The Iceberg property holding a v3 `initial-default`, as encoded JSON.
const INITIAL_DEFAULT: &str = "initial-default";

/// The Iceberg property holding a v3 `write-default`, as encoded JSON.
const WRITE_DEFAULT: &str = "write-default";

/// The Iceberg property listing the identifier field ids of a schema root.
const IDENTIFIER: &str = "identifier-field-ids";

/// The Iceberg property preserving a declared type the physical datatype
/// cannot distinguish.
///
/// `uuid` materializes as the same 16-byte fixed value `fixed[16]` does, so
/// the declared spelling rides the field's metadata: a schema read from a
/// table that says `uuid` writes `uuid` back, rather than quietly demoting
/// the column to `fixed[16]` on the next metadata commit.
pub(super) const DECLARED_TYPE: &str = "type";

/// Read an Iceberg schema object into a non-null struct root field.
///
/// The root takes `name`, because an Iceberg schema names its columns but not
/// itself. Every column keeps its Iceberg id in `PARQUET:field_id`, so a later
/// Parquet write carries the ids into the file.
///
/// # Errors
///
/// Returns an error when the JSON is not a `struct` schema object, when a
/// field is missing `id`, `name`, `required`, or `type`, or when a type has no
/// core representation.
pub fn schema_from_json(name: &str, schema: &Scalar) -> Result<Field> {
    if schema.as_record().is_none() && schema.as_mapping().is_none() {
        return Err(invalid(format_smolstr!(
            "expected an Iceberg schema object, got {}",
            schema.kind()
        )));
    }

    let type_name = schema
        .get_key_str("type")
        .and_then(Scalar::as_str)
        .unwrap_or("struct");
    if type_name != "struct" {
        return Err(invalid(format_smolstr!(
            "expected an Iceberg schema of type \"struct\", got {type_name:?}"
        )));
    }

    let normalized = super::official::normalize_schema(schema)?;
    let schema = &normalized;
    let mut root = struct_field_from_json(name, schema, false)?;
    if let Some(id) = schema.get_key_str("schema-id").and_then(Scalar::as_i64) {
        root.iceberg_mut().insert(SCHEMA_ID, id.to_string())?;
    }
    if let Some(ids) = schema
        .get_key_str("identifier-field-ids")
        .and_then(Scalar::as_sequence)
    {
        let joined: Vec<String> = ids
            .iter()
            .filter_map(Scalar::as_i64)
            .map(|id| id.to_string())
            .collect();
        root.iceberg_mut().insert(IDENTIFIER, joined.join(","))?;
    }
    Ok(root)
}

/// Write a non-null struct root field as an Iceberg schema object.
///
/// # Errors
///
/// Returns an error when the field is not a non-null struct root, when a
/// column has no field id, or when a datatype has no Iceberg spelling.
pub fn schema_into_json(root: &Field) -> Result<Scalar> {
    root.validate_struct_root()?;

    let mut entries = vec![(Scalar::from("type"), Scalar::from("struct"))];
    if let Some(id) = root.iceberg().get(SCHEMA_ID) {
        let id = id.parse::<i64>().map_err(|_| {
            invalid(format_smolstr!(
                "expected an integer {:?}, got {id:?}",
                root.iceberg().key(SCHEMA_ID)
            ))
        })?;
        entries.push((Scalar::from("schema-id"), json_integer(id)));
    }
    entries.push((
        Scalar::from("fields"),
        Scalar::from_sequence(fields_to_json(root)?),
    ));
    if let Some(ids) = root.iceberg().get(IDENTIFIER) {
        let mut parsed = Vec::new();
        for id in ids.split(',').filter(|id| !id.is_empty()) {
            parsed.push(Scalar::from(id.trim().parse::<i64>().map_err(|_| {
                invalid(format_smolstr!(
                    "expected comma-separated integers in {:?}, got {ids:?}",
                    root.iceberg().key(IDENTIFIER)
                ))
            })?));
        }
        entries.push((
            Scalar::from("identifier-field-ids"),
            Scalar::from_sequence(parsed),
        ));
    }
    let document = Scalar::from_record(entries.into_iter().map(|(key, value)| {
        (
            SmolStr::new(
                key.as_str()
                    .expect("Iceberg schema object keys are always strings"),
            ),
            value,
        )
    }))?;
    super::official::validate_schema(&document)?;
    Ok(document)
}

/// Number every field in a tree that does not already carry an identifier.
///
/// Returns the next unused identifier, which a table metadata document records
/// as `last-column-id`.
///
/// # Errors
///
/// Returns an error when the field tree is not a valid schema root or an
/// identifier would overflow.
pub fn assign_field_ids(root: &mut Field, start: i32) -> Result<i32> {
    root.require_struct()?;
    root.assign_parquet_field_ids(start)
}

/// Return the highest field identifier anywhere in a schema tree.
///
/// A table records this as `last-column-id`, and a schema evolution starts
/// numbering above it so an identifier is never reused for a different column.
///
/// # Errors
///
/// Returns an error when a stored identifier is not a canonical integer.
pub fn last_column_id(root: &Field) -> Result<i32> {
    Ok(root.max_parquet_field_id()?.unwrap_or_default())
}

/// Build a struct field from an Iceberg `fields` array.
fn struct_field_from_json(name: &str, object: &Scalar, nullable: bool) -> Result<Field> {
    let entries = object
        .get_key_str("fields")
        .and_then(Scalar::as_sequence)
        .ok_or_else(|| {
            invalid(format_smolstr!(
                "expected an Iceberg \"fields\" array in {name}"
            ))
        })?;

    let mut children = Vec::with_capacity(entries.len());
    for entry in entries {
        children.push(field_from_json(entry)?);
    }
    Ok(Field::new(name, DataType::from_fields(children)?, nullable))
}

/// Build one column from an Iceberg field object.
fn field_from_json(entry: &Scalar) -> Result<Field> {
    if entry.as_record().is_none() && entry.as_mapping().is_none() {
        return Err(invalid(format_smolstr!(
            "expected an Iceberg field object, got {}",
            entry.kind()
        )));
    }

    let name = entry
        .get_key_str("name")
        .and_then(Scalar::as_str)
        .ok_or_else(|| invalid(SmolStr::new_static("expected an Iceberg field \"name\"")))?;
    let id = entry
        .get_key_str("id")
        .and_then(Scalar::as_i64)
        .ok_or_else(|| {
            invalid(format_smolstr!(
                "expected an Iceberg field \"id\" on {name:?}"
            ))
        })?;
    // Iceberg states requirement; the core states nullability.
    let required = entry
        .get_key_str("required")
        .and_then(Scalar::as_bool)
        .ok_or_else(|| {
            invalid(format_smolstr!(
                "expected an Iceberg field \"required\" flag on {name:?}"
            ))
        })?;

    let type_json = entry.get_key_str("type").ok_or_else(|| {
        invalid(format_smolstr!(
            "expected an Iceberg field \"type\" on {name:?}"
        ))
    })?;

    let mut field = typed_field_from_json(name, type_json, !required)?;
    field.set_parquet_field_id(field_id(id, name)?);
    if let Some(doc) = entry.get_key_str("doc").and_then(Scalar::as_str) {
        field.iceberg_mut().insert(DOC, doc)?;
    }
    // The v3 defaults are values, not schema, so they travel as encoded JSON
    // rather than as a second parallel value model.
    for property in [INITIAL_DEFAULT, WRITE_DEFAULT] {
        if let Some(default) = entry.get_key_str(property) {
            let encoded = crate::json::into_bytes(default)?;
            let encoded = String::from_utf8(encoded).map_err(|error| {
                invalid(format_smolstr!(
                    "expected UTF-8 in an Iceberg {property} on {name:?}, got {error}"
                ))
            })?;
            field.iceberg_mut().insert(property, encoded)?;
        }
    }
    Ok(field)
}

/// Build a field from an Iceberg type, primitive or nested.
fn typed_field_from_json(name: &str, type_json: &Scalar, nullable: bool) -> Result<Field> {
    if let Some(primitive) = type_json.as_str() {
        let parsed = PrimitiveType::from_str(primitive)?;
        let dtype = parsed.into_dtype()?;
        let mut field = Field::new(name, dtype, nullable);
        // `uuid` and `fixed[16]` share one physical type, so the declared
        // spelling is kept where the writer will find it again.
        if parsed == PrimitiveType::Uuid {
            field.iceberg_mut().insert(DECLARED_TYPE, "uuid")?;
        }
        return Ok(field);
    }

    if type_json.as_record().is_none() && type_json.as_mapping().is_none() {
        return Err(invalid(format_smolstr!(
            "expected an Iceberg type name or object on {name:?}, got {}",
            type_json.kind()
        )));
    }

    match type_json.get_key_str("type").and_then(Scalar::as_str) {
        Some("struct") => struct_field_from_json(name, type_json, nullable),
        Some("list") => {
            let element = type_json.get_key_str("element").ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected a list \"element\" type on {name:?}"
                ))
            })?;
            let required = type_json
                .get_key_str("element-required")
                .and_then(Scalar::as_bool)
                .unwrap_or(true);
            let mut item = typed_field_from_json("element", element, !required)?;
            if let Some(id) = type_json.get_key_str("element-id").and_then(Scalar::as_i64) {
                item.set_parquet_field_id(field_id(id, name)?);
            }
            Ok(Field::new(name, DataType::list(item), nullable))
        }
        Some("map") => {
            let key_json = type_json.get_key_str("key").ok_or_else(|| {
                invalid(format_smolstr!("expected a map \"key\" type on {name:?}"))
            })?;
            let value_json = type_json.get_key_str("value").ok_or_else(|| {
                invalid(format_smolstr!("expected a map \"value\" type on {name:?}"))
            })?;
            // A map key is always required; only the value carries a flag.
            let mut key = typed_field_from_json("key", key_json, false)?;
            let value_required = type_json
                .get_key_str("value-required")
                .and_then(Scalar::as_bool)
                .unwrap_or(true);
            let mut value = typed_field_from_json("value", value_json, !value_required)?;
            if let Some(id) = type_json.get_key_str("key-id").and_then(Scalar::as_i64) {
                key.set_parquet_field_id(field_id(id, name)?);
            }
            if let Some(id) = type_json.get_key_str("value-id").and_then(Scalar::as_i64) {
                value.set_parquet_field_id(field_id(id, name)?);
            }
            let entries = Field::new("entries", DataType::from_fields([key, value])?, false);
            Ok(Field::new(name, DataType::map(entries, false)?, nullable))
        }
        other => Err(invalid(format_smolstr!(
            "expected an Iceberg nested type of \"struct\", \"list\", or \"map\" on {name:?}, \
             got {other:?}"
        ))),
    }
}

/// Render a struct field's children as Iceberg field objects.
fn fields_to_json(root: &Field) -> Result<Vec<Scalar>> {
    let fields = root.fields();
    let mut entries = Vec::with_capacity(fields.len());
    for field in fields {
        let id = field.parquet_field_id()?.ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a PARQUET:field_id on {:?}; call assign_field_ids first",
                field.name()
            ))
        })?;
        let mut object = vec![
            (Scalar::from("id"), json_integer(i64::from(id))),
            (Scalar::from("name"), Scalar::from(field.name())),
            (Scalar::from("required"), Scalar::from(!field.is_nullable())),
            (Scalar::from("type"), type_to_json(field)?),
        ];
        if let Some(doc) = field.iceberg().get(DOC) {
            object.push((Scalar::from("doc"), Scalar::from(doc)));
        }
        for property in [INITIAL_DEFAULT, WRITE_DEFAULT] {
            if let Some(encoded) = field.iceberg().get(property) {
                object.push((Scalar::from(property), crate::json::from_utf8(encoded)?));
            }
        }
        entries.push(Scalar::from_record(object.into_iter().map(
            |(key, value)| {
                (
                    SmolStr::new(
                        key.as_str()
                            .expect("Iceberg field object keys are always strings"),
                    ),
                    value,
                )
            },
        ))?);
    }
    Ok(entries)
}

/// Render one field's datatype as an Iceberg type.
fn type_to_json(field: &Field) -> Result<Scalar> {
    match field.dtype() {
        DataType::Struct(_) => Scalar::from_record([
            ("type", Scalar::from("struct")),
            ("fields", Scalar::from_sequence(fields_to_json(field)?)),
        ]),
        DataType::List(item) | DataType::LargeList(item) | DataType::ListView(item) => {
            let mut object = vec![(Scalar::from("type"), Scalar::from("list"))];
            if let Some(id) = item.parquet_field_id()? {
                object.push((Scalar::from("element-id"), json_integer(i64::from(id))));
            }
            object.push((Scalar::from("element"), type_to_json(item)?));
            object.push((
                Scalar::from("element-required"),
                Scalar::from(!item.is_nullable()),
            ));
            Scalar::from_record(object.into_iter().map(|(key, value)| {
                (
                    SmolStr::new(
                        key.as_str()
                            .expect("Iceberg list object keys are always strings"),
                    ),
                    value,
                )
            }))
        }
        DataType::Map(map) => {
            let entries = map.entries();
            let key = entries.get_field(0).ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected a map key field on {:?}",
                    field.name()
                ))
            })?;
            let value = entries.get_field(1).ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected a map value field on {:?}",
                    field.name()
                ))
            })?;
            let mut object = vec![(Scalar::from("type"), Scalar::from("map"))];
            if let Some(id) = key.parquet_field_id()? {
                object.push((Scalar::from("key-id"), json_integer(i64::from(id))));
            }
            object.push((Scalar::from("key"), type_to_json(key)?));
            if let Some(id) = value.parquet_field_id()? {
                object.push((Scalar::from("value-id"), json_integer(i64::from(id))));
            }
            object.push((Scalar::from("value"), type_to_json(value)?));
            object.push((
                Scalar::from("value-required"),
                Scalar::from(!value.is_nullable()),
            ));
            Scalar::from_record(object.into_iter().map(|(key, value)| {
                (
                    SmolStr::new(
                        key.as_str()
                            .expect("Iceberg map object keys are always strings"),
                    ),
                    value,
                )
            }))
        }
        other => {
            let computed = PrimitiveType::from_dtype(other)?;
            // The declared spelling wins only where the physical type agrees
            // with it, so a stale marker can never misdescribe a column.
            if computed == PrimitiveType::Fixed(16)
                && field.iceberg().get(DECLARED_TYPE) == Some("uuid")
            {
                return Ok(Scalar::from("uuid"));
            }
            Ok(Scalar::from(computed.to_string()))
        }
    }
}

/// Narrow an identifier read from JSON.
fn field_id(id: i64, name: &str) -> Result<i32> {
    i32::try_from(id).map_err(|_| {
        invalid(format_smolstr!(
            "expected a field identifier that fits 32 bits on {name:?}, got {id}"
        ))
    })
}

/// Spell a JSON integer with the same signedness the natural parser infers.
fn json_integer(value: i64) -> Scalar {
    u64::try_from(value).map_or_else(|_| Scalar::from(value), Scalar::from)
}

/// Report a malformed Iceberg schema document.
fn invalid(reason: SmolStr) -> Error {
    Error::Codec {
        format: "iceberg",
        position: 0,
        reason,
    }
}
