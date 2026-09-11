use base64::Engine as _;
use smol_str::{SmolStr, format_smolstr};

use crate::types::Nested;
use crate::{DataType, Error, Field, Result, Scalar};

/// Interpret a natural text value under one field, then validate it.
pub(crate) fn with_field(value: Scalar, field: &Field) -> Result<Scalar> {
    // [`Field::scalar`] is the single schema conversion implementation, and it
    // reads every spelling a document leaves behind; only the base64 a
    // document spells bytes with is substituted before it.
    field.scalar(prepare(value, field)?)
}

/// Restate a canonical value in the natural text shape, naming its members.
///
/// This is the write half of [`with_field`]: canonicalization resolves a
/// record to an ordered sequence, and a text format wants the names back, so
/// every Struct below `field` becomes a name-keyed record again. Leaf
/// spellings - base64 bytes, decimal text, ISO temporals - belong to the
/// format writers and are left exactly as they are.
pub(crate) fn into_natural(value: Scalar, field: &Field) -> Result<Scalar> {
    if value.is_null() {
        return Ok(value);
    }
    match field.dtype() {
        DataType::Struct(fields) => named(value, fields, field),
        DataType::List(child)
        | DataType::ListView(child)
        | DataType::FixedSizeList(child, _)
        | DataType::LargeList(child)
        | DataType::LargeListView(child) => {
            sequence(value, |value| into_natural(value, child), field)
        }
        DataType::Union(fields, _) => {
            let Some([type_id, payload]) = value.as_sequence() else {
                return Err(invalid(field, "expected [type_id, value] for a union"));
            };
            let id = type_id
                .as_i128()
                .and_then(|id| i8::try_from(id).ok())
                .ok_or_else(|| invalid(field, "union type id must fit i8"))?;
            let branch = fields
                .iter()
                .find_map(|(candidate, branch)| (candidate == id).then_some(branch))
                .ok_or_else(|| invalid(field, "union type id is not declared"))?;
            Ok(Scalar::from_sequence([
                type_id.clone(),
                into_natural(payload.clone(), branch)?,
            ]))
        }
        DataType::Dictionary(dictionary) => into_natural(
            value,
            &Field::new(
                field.name(),
                dictionary.value().clone(),
                field.is_nullable(),
            ),
        ),
        DataType::RunEndEncoded(encoded) => into_natural(value, encoded.values()),
        DataType::Map(map) => {
            let fields = map.entries().fields();
            let [_, value_field] = fields else {
                return Err(invalid(
                    field,
                    "map entries do not contain key and value fields",
                ));
            };
            let Some(entries) = value.as_mapping() else {
                return Ok(value);
            };
            Scalar::from_mapping(
                entries
                    .iter()
                    .map(|(key, item)| Ok((key.clone(), into_natural(item.clone(), value_field)?)))
                    .collect::<Result<Vec<_>>>()?,
            )
        }
        _ => Ok(value),
    }
}

/// Re-key one canonical struct row by the names its Field declares.
fn named(value: Scalar, fields: &crate::Fields, field: &Field) -> Result<Scalar> {
    let Some(values) = value.as_sequence() else {
        // A record already carries its names; anything else is not a struct
        // row and the format writer refuses it under its own rules.
        return Ok(value);
    };
    if values.len() != fields.len() {
        return Err(invalid(
            field,
            format_smolstr!(
                "expected {} struct values, got {}",
                fields.len(),
                values.len()
            ),
        ));
    }
    Scalar::from_record(
        values
            .iter()
            .zip(fields.iter())
            .map(|(value, child)| {
                Ok((
                    SmolStr::new(child.name()),
                    into_natural(value.clone(), child)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?,
    )
}

/// Substitute the one spelling a document has that a value does not.
///
/// No structured text format has a byte literal, so a document spells a payload
/// in base64; every other reading a document needs - a number, a boolean, a
/// temporal, an ordered struct, a record keyed by name - is the value contract
/// [`Field::scalar`] already owns, and is left to it. The walk only descends to
/// find the byte leaves.
fn prepare(value: Scalar, field: &Field) -> Result<Scalar> {
    // A subtree with no byte leaf has nothing to substitute, so the document
    // value goes to the contract as it is rather than being walked and rebuilt.
    if value.is_null() || !holds_byte_leaf(field.dtype()) {
        return Ok(value);
    }
    match field.dtype() {
        DataType::Binary
        | DataType::FixedSizeBinary(_)
        | DataType::LargeBinary
        | DataType::BinaryView
        | DataType::Geometry(_)
        | DataType::Geography(_) => base64_payload(value, field),
        DataType::List(child)
        | DataType::ListView(child)
        | DataType::FixedSizeList(child, _)
        | DataType::LargeList(child)
        | DataType::LargeListView(child) => sequence(value, |value| prepare(value, child), field),
        DataType::Struct(fields) => structure(value, fields, field),
        DataType::Union(fields, _) => union(value, fields, field),
        DataType::Dictionary(dictionary) => prepare_for_type(value, dictionary.value(), field),
        DataType::Map(map) => mapping(value, map, field),
        DataType::RunEndEncoded(encoded) => prepare(value, encoded.values()),
        _ => Ok(value),
    }
}

fn prepare_for_type(value: Scalar, dtype: &DataType, context: &Field) -> Result<Scalar> {
    prepare(
        value,
        &Field::new(context.name(), dtype.clone(), context.is_nullable()),
    )
}

/// Descend a document array without deciding anything about its items.
fn sequence(
    value: Scalar,
    mut prepare_value: impl FnMut(Scalar) -> Result<Scalar>,
    field: &Field,
) -> Result<Scalar> {
    let Some(values) = value.as_sequence() else {
        return Err(invalid(field, "expected an array"));
    };
    values
        .iter()
        .cloned()
        .map(&mut prepare_value)
        .collect::<Result<Vec<_>>>()
        .map(Scalar::from_sequence)
}

/// Descend a document object or ordered array under a struct's children.
fn structure(value: Scalar, fields: &crate::Fields, field: &Field) -> Result<Scalar> {
    match value {
        Scalar::Nested(Nested::Record(entries)) => {
            let prepared = entries
                .as_map()
                .iter()
                .map(|(name, value)| {
                    let child = fields
                        .get_by_name(name)
                        .ok_or_else(|| invalid(field, format_smolstr!("unknown field {name:?}")))?;
                    Ok((name.clone(), prepare(value.clone(), child)?))
                })
                .collect::<Result<Vec<_>>>()?;
            Scalar::from_record(prepared)
        }
        Scalar::Nested(Nested::Sequence(values)) => {
            if values.as_slice().len() != fields.len() {
                return Err(invalid(field, "struct array has the wrong length"));
            }
            values
                .as_slice()
                .iter()
                .cloned()
                .zip(fields.iter())
                .map(|(value, child)| prepare(value, child))
                .collect::<Result<Vec<_>>>()
                .map(Scalar::from_sequence)
        }
        _ => Err(invalid(field, "expected an object or ordered struct array")),
    }
}

/// Descend the branch a union's type ID selects.
fn union(value: Scalar, fields: &crate::UnionFields, field: &Field) -> Result<Scalar> {
    let Some(values) = value.as_sequence() else {
        return Err(invalid(field, "expected [type_id, value] for a union"));
    };
    let [type_id, payload] = values else {
        return Err(invalid(field, "expected [type_id, value] for a union"));
    };
    let id = type_id
        .as_i128()
        .and_then(|id| i8::try_from(id).ok())
        .ok_or_else(|| invalid(field, "union type id must fit i8"))?;
    let branch = fields
        .iter()
        .find_map(|(candidate, branch)| (candidate == id).then_some(branch))
        .ok_or_else(|| invalid(field, "union type id is not declared"))?;
    Ok(Scalar::from_sequence([
        type_id.clone(),
        prepare(payload.clone(), branch)?,
    ]))
}

/// Descend a document mapping or object under a map's key and value fields.
fn mapping(value: Scalar, map: &crate::MapType, field: &Field) -> Result<Scalar> {
    let fields = map.entries().fields();
    let [key_field, value_field] = fields else {
        return Err(invalid(
            field,
            "map entries do not contain key and value fields",
        ));
    };
    let entries = match value {
        Scalar::Nested(Nested::Mapping(entries)) => entries.as_slice().to_vec(),
        // A record is a map keyed by name, which the value contract reads too;
        // the entries are shaped here so the walk reaches their byte leaves.
        Scalar::Nested(Nested::Record(entries)) => entries
            .as_map()
            .iter()
            .map(|(name, value)| (Scalar::from(name.as_str()), value.clone()))
            .collect(),
        _ => return Err(invalid(field, "expected an object or mapping")),
    };
    Scalar::from_mapping(
        entries
            .into_iter()
            .map(|(key, value)| Ok((prepare(key, key_field)?, prepare(value, value_field)?)))
            .collect::<Result<Vec<_>>>()?,
    )
}

/// Whether a subtree stores bytes anywhere a document would spell base64.
fn holds_byte_leaf(dtype: &DataType) -> bool {
    match dtype {
        DataType::Binary
        | DataType::FixedSizeBinary(_)
        | DataType::LargeBinary
        | DataType::BinaryView
        | DataType::Geometry(_)
        | DataType::Geography(_) => true,
        DataType::List(child)
        | DataType::ListView(child)
        | DataType::FixedSizeList(child, _)
        | DataType::LargeList(child)
        | DataType::LargeListView(child) => holds_byte_leaf(child.dtype()),
        DataType::RunEndEncoded(encoded) => holds_byte_leaf(encoded.values().dtype()),
        DataType::Struct(fields) => fields.iter().any(|field| holds_byte_leaf(field.dtype())),
        DataType::Union(fields, _) => fields
            .iter()
            .any(|(_, field)| holds_byte_leaf(field.dtype())),
        DataType::Dictionary(dictionary) => holds_byte_leaf(dictionary.value()),
        DataType::Map(map) => holds_byte_leaf(map.entries().dtype()),
        _ => false,
    }
}

/// Decode the base64 a document spells a byte payload with.
fn base64_payload(value: Scalar, field: &Field) -> Result<Scalar> {
    match value {
        Scalar::Text(encoded) => base64::engine::general_purpose::STANDARD
            .decode(encoded.as_str().as_bytes())
            .map(Scalar::from)
            .map_err(|_| invalid(field, "expected base64 text")),
        value => Ok(value),
    }
}

fn invalid(field: &Field, reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$.{}", field.name()),
        reason: reason.into(),
    }
}
