//! Where an XML document's shape meets a declared field.
//!
//! XML states less about shape than the other structured formats do, and a
//! declared [`Field`] is the only thing that can say what the missing part
//! was. These are the readings that need it, and nothing else here decides
//! anything:
//!
//! | Document | Field | Reading |
//! | --- | --- | --- |
//! | one root element | the root itself | the root's content is the value |
//! | one `<tag>` occurrence | a list | a list holding that one value |
//! | no occurrence at all | a non-null list | the empty list |
//! | `<tag/>`, no character data | a non-null text or byte field | the empty value |
//! | the parts of an interval | an interval | its counts, read as integers |
//! | a `[type, value]` pair | a union | the type id, read as an integer |
//!
//! The empty element is there because XML spells absence and the empty string
//! the same way, so only a field that refuses absence settles which one was
//! written. An element the document leaves out entirely is still absence, and
//! the value contract still refuses it.
//!
//! The interval and the union type id are there because they are the values
//! the contract reads no text for: an interval's parts are counts of months,
//! days and nanoseconds and a union's type id names a declared branch, while
//! XML writes every leaf as character data.
//!
//! Every other reading - a string that is a number, a record ordered into a
//! row, an absent nullable element - belongs to the value contract
//! [`Field::from_natural_value`](crate::Field::from_natural_value) already
//! owns, and is left to it.

use smol_str::SmolStr;

use crate::types::Nested;
use crate::{DataType, Field, Result, Scalar};

use super::codec_error;

/// Read one document under `field`, whose name is the root element's.
///
/// A document is one root element, so its single entry is what `field`
/// describes; the element's own name is what the field already names.
pub(super) fn shaped_document(document: Scalar, field: &Field) -> Result<Scalar> {
    let Scalar::Nested(Nested::Record(entries)) = &document else {
        return Err(not_a_document(&document));
    };
    let mut values = entries.as_map().values();
    let (Some(root), None) = (values.next(), values.next()) else {
        return Err(not_a_document(&document));
    };
    shaped(root.clone(), field)
}

/// Restate one XML value in the shape `field` declares.
pub(super) fn shaped(value: Scalar, field: &Field) -> Result<Scalar> {
    shaped_for(value, field.dtype(), field.is_nullable())
}

fn shaped_for(value: Scalar, dtype: &DataType, nullable: bool) -> Result<Scalar> {
    if value.is_null() {
        // `<a/>` and `<a></a>` are one document, so an element with no
        // character data is absence until a field refuses absence; then the
        // empty payload is the only thing it can have been.
        if !nullable && holds_empty_payload(dtype) {
            return Ok(Scalar::from(""));
        }
        return Ok(value);
    }
    match dtype {
        DataType::List(child)
        | DataType::ListView(child)
        | DataType::FixedSizeList(child, _)
        | DataType::LargeList(child)
        | DataType::LargeListView(child) => {
            // One occurrence of a repeated element is one item, because XML
            // repeats an element instead of framing a list around it.
            let values = match value.as_sequence() {
                Some(values) => values.to_vec(),
                None => vec![value],
            };
            values
                .into_iter()
                .map(|value| shaped(value, child))
                .collect::<Result<Vec<_>>>()
                .map(Scalar::from_sequence)
        }
        DataType::Struct(fields) => {
            let Scalar::Nested(Nested::Record(entries)) = &value else {
                // An ordered row already names nothing and needs nothing.
                return Ok(value);
            };
            let entries = entries.as_map();
            let mut shaped_entries = Vec::with_capacity(entries.len().max(fields.len()));
            for child in fields.iter() {
                match entries.get(child.name()) {
                    Some(value) => {
                        shaped_entries
                            .push((SmolStr::new(child.name()), shaped(value.clone(), child)?));
                    }
                    // A list with no occurrence is the empty list, which is
                    // the one absence XML has no other spelling for.
                    None if !child.is_nullable() && is_list(child.dtype()) => {
                        shaped_entries
                            .push((SmolStr::new(child.name()), Scalar::from_sequence([])));
                    }
                    // Anything else absent is the value contract's to name.
                    None => {}
                }
            }
            // A name the field does not declare travels unchanged, so the
            // contract refuses it by name rather than this walk dropping it.
            for (name, value) in entries {
                if fields.get_by_name(name).is_none() {
                    shaped_entries.push((name.clone(), value.clone()));
                }
            }
            Scalar::from_record(shaped_entries)
        }
        DataType::Map(map) => {
            let [_, value_field] = map.entries().fields() else {
                return Ok(value);
            };
            let Scalar::Nested(Nested::Record(entries)) = &value else {
                return Ok(value);
            };
            Scalar::from_record(
                entries
                    .as_map()
                    .iter()
                    .map(|(name, item)| Ok((name.clone(), shaped(item.clone(), value_field)?)))
                    .collect::<Result<Vec<_>>>()?,
            )
        }
        // An interval's parts arrive as the character data XML writes, and
        // the value contract reads counts rather than their spelling.
        DataType::Interval(_) => match value.as_sequence() {
            Some(values) => values
                .iter()
                .map(count_of)
                .collect::<Result<Vec<_>>>()
                .map(Scalar::from_sequence),
            None => count_of(&value),
        },
        // A union is the pair its type id opens, and that id names a branch
        // rather than spelling a value.
        DataType::Union(fields, _) => {
            let Some([type_id, payload]) = value.as_sequence() else {
                return Ok(value);
            };
            let type_id = count_of(type_id)?;
            let branch = type_id
                .as_i128()
                .and_then(|id| i8::try_from(id).ok())
                .and_then(|id| {
                    fields
                        .iter()
                        .find_map(|(candidate, branch)| (candidate == id).then_some(branch))
                });
            let payload = match branch {
                Some(branch) => shaped(payload.clone(), branch)?,
                // An undeclared id is the value contract's refusal to name.
                None => payload.clone(),
            };
            Ok(Scalar::from_sequence([type_id, payload]))
        }
        DataType::Dictionary(dictionary) => shaped_for(value, dictionary.value(), nullable),
        DataType::RunEndEncoded(encoded) => shaped(value, encoded.values()),
        _ => Ok(value),
    }
}

/// One interval part, as the count it spells.
fn count_of(value: &Scalar) -> Result<Scalar> {
    let Some(text) = value.as_str() else {
        return Ok(value.clone());
    };
    text.trim().parse::<i64>().map(Scalar::from).map_err(|_| {
        codec_error(
            0,
            smol_str::format_smolstr!(
                "expected an interval part as a whole number, got {:?}",
                crate::text::elide_to(text, crate::text::ERROR_TEXT_LIMIT)
            ),
        )
    })
}

/// Whether a datatype's empty value has a spelling of no characters at all.
fn holds_empty_payload(dtype: &DataType) -> bool {
    // Only the variable-width ones: a fixed width and a coded vocabulary both
    // have a length no empty element satisfies.
    matches!(
        dtype,
        DataType::Utf8
            | DataType::LargeUtf8
            | DataType::Utf8View
            | DataType::Ascii
            | DataType::Binary
            | DataType::LargeBinary
            | DataType::BinaryView
    )
}

/// Whether a datatype frames a list of values.
fn is_list(dtype: &DataType) -> bool {
    matches!(
        dtype,
        DataType::List(_)
            | DataType::ListView(_)
            | DataType::FixedSizeList(..)
            | DataType::LargeList(_)
            | DataType::LargeListView(_)
    )
}

fn not_a_document(value: &Scalar) -> crate::Error {
    codec_error(
        0,
        smol_str::format_smolstr!(
            "an XML document is one root element, got {} at its root",
            value.kind()
        ),
    )
}
