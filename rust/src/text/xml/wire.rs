//! One `Scalar` written as an XML document.

use std::io::Write;

use smol_str::format_smolstr;

use crate::text::{Formatting, Scalar};
use crate::types::{Nested, Temporal};
use crate::{Error, Result};

use super::{ATTRIBUTE_PREFIX, FORMAT, TEXT_KEY};

/// How nested elements are laid out.
#[derive(Clone, Copy)]
pub(crate) struct Layout {
    unit: Option<&'static [u8]>,
}

impl From<Formatting> for Layout {
    fn from(value: Formatting) -> Self {
        Self {
            unit: value.indent().unit(),
        }
    }
}

/// Write one value as a complete XML document.
///
/// A record holding exactly one named value is the document that value's name
/// spells; anything else is the document element
/// [`DEFAULT_ROOT_NAME`](crate::media::DEFAULT_ROOT_NAME) holds, which is what
/// a row written on its own needs.
pub(crate) fn write_document<W: Write>(
    writer: &mut W,
    value: &Scalar,
    layout: Layout,
) -> Result<()> {
    let (name, value) = document_element(value);
    write_element(writer, name, value, layout, 0)
}

/// Name the document element a value spells, and the value it holds.
fn document_element(value: &Scalar) -> (&str, &Scalar) {
    if let Scalar::Nested(Nested::Record(entries)) = value {
        let entries = entries.as_map();
        if entries.len() == 1 {
            if let Some((name, value)) = entries.iter().next() {
                if !name.starts_with(ATTRIBUTE_PREFIX) && name != TEXT_KEY {
                    return (name.as_str(), value);
                }
            }
        }
    }
    (crate::media::DEFAULT_ROOT_NAME, value)
}

/// Write one element holding `value`.
pub(crate) fn write_element<W: Write>(
    writer: &mut W,
    name: &str,
    value: &Scalar,
    layout: Layout,
    depth: usize,
) -> Result<()> {
    check_name(name)?;
    match value {
        Scalar::Nested(Nested::Record(entries)) => {
            let named = entries
                .as_map()
                .iter()
                .map(|(key, value)| (key.as_str(), value));
            write_children(writer, name, named, layout, depth)
        }
        Scalar::Nested(Nested::Mapping(entries)) => {
            let named = entries
                .as_slice()
                .iter()
                .map(|(key, value)| {
                    let key = key.as_str().ok_or_else(|| {
                        codec_error("XML element and attribute names must be strings")
                    })?;
                    Ok((key, value))
                })
                .collect::<Result<Vec<_>>>()?;
            write_children(writer, name, named.into_iter(), layout, depth)
        }
        // A sequence is repetition of the element around it, so it has no
        // element of its own to be written into.
        Scalar::Nested(Nested::Sequence(values)) => Err(Error::Codec {
            format: FORMAT,
            position: 0,
            reason: crate::text::expected_got(
                format_args!("one value for element <{name}>"),
                format_args!("a sequence of {}", values.as_slice().len()),
            ),
        }),
        Scalar::Null => {
            write!(writer, "<{name}/>")?;
            Ok(())
        }
        leaf => {
            write!(writer, "<{name}>")?;
            write_escaped(writer, &text(leaf)?, Escaping::Content)?;
            write!(writer, "</{name}>")?;
            Ok(())
        }
    }
}

/// Write one element from its attributes, its own text, and its children.
pub(crate) fn write_children<'a, W, I>(
    writer: &mut W,
    name: &str,
    entries: I,
    layout: Layout,
    depth: usize,
) -> Result<()>
where
    W: Write,
    I: Iterator<Item = (&'a str, &'a Scalar)> + Clone,
{
    write!(writer, "<{name}")?;
    let mut own_text = None;
    for (key, value) in entries.clone() {
        let Some(attribute) = key.strip_prefix(ATTRIBUTE_PREFIX) else {
            if key == TEXT_KEY {
                own_text = Some(value);
            }
            continue;
        };
        check_name(attribute)?;
        write!(writer, " {attribute}=\"")?;
        write_escaped(
            writer,
            &attribute_text(value, attribute)?,
            Escaping::Attribute,
        )?;
        writer.write_all(b"\"")?;
    }

    // A child whose value is an empty sequence writes no element, so it is not
    // a child here either: an element holding only those closes itself.
    let children = entries.filter(|(key, value)| {
        !key.starts_with(ATTRIBUTE_PREFIX) && *key != TEXT_KEY && !is_empty_sequence(value)
    });
    if own_text.is_none() && children.clone().next().is_none() {
        writer.write_all(b"/>")?;
        return Ok(());
    }
    writer.write_all(b">")?;

    if let Some(value) = own_text {
        // Character data beside child elements is mixed content: laying it out
        // would change the text, so an element that holds its own text is
        // written on one line whatever the layout asks for.
        write_escaped(writer, &text(value)?, Escaping::Content)?;
        for (key, value) in children {
            write_child(writer, key, value, layout, depth + 1)?;
        }
        write!(writer, "</{name}>")?;
        return Ok(());
    }

    for (key, value) in children {
        if let Some(unit) = layout.unit {
            writer.write_all(b"\n")?;
            write_units(writer, unit, depth + 1)?;
        }
        write_child(writer, key, value, layout, depth + 1)?;
    }
    if let Some(unit) = layout.unit {
        writer.write_all(b"\n")?;
        write_units(writer, unit, depth)?;
    }
    write!(writer, "</{name}>")?;
    Ok(())
}

/// Write one named child, repeating the element once per sequence item.
fn write_child<W: Write>(
    writer: &mut W,
    name: &str,
    value: &Scalar,
    layout: Layout,
    depth: usize,
) -> Result<()> {
    let Scalar::Nested(Nested::Sequence(values)) = value else {
        return write_element(writer, name, value, layout, depth);
    };
    for (index, value) in values.as_slice().iter().enumerate() {
        if index != 0 {
            if let Some(unit) = layout.unit {
                writer.write_all(b"\n")?;
                write_units(writer, unit, depth)?;
            }
        }
        if matches!(value, Scalar::Nested(Nested::Sequence(_))) {
            return Err(codec_error(
                "XML repeats an element per item, so a sequence of sequences has no spelling",
            ));
        }
        write_element(writer, name, value, layout, depth)?;
    }
    Ok(())
}

/// Return whether a value writes no element at all.
fn is_empty_sequence(value: &Scalar) -> bool {
    match value {
        Scalar::Nested(Nested::Sequence(values)) => values.as_slice().is_empty(),
        _ => false,
    }
}

fn write_units<W: Write>(writer: &mut W, unit: &[u8], count: usize) -> Result<()> {
    for _ in 0..count {
        writer.write_all(unit)?;
    }
    Ok(())
}

/// Return the character data one attribute carries.
fn attribute_text(value: &Scalar, name: &str) -> Result<smol_str::SmolStr> {
    if value.is_null() {
        return Err(Error::Codec {
            format: FORMAT,
            position: 0,
            reason: format_smolstr!(
                "expected a value for attribute {name}, got null, which XML spells only by \
                 leaving the attribute out"
            ),
        });
    }
    text(value)
}

/// Spell one leaf value as the character data it holds.
///
/// XML carries text, so every leaf answers its canonical spelling: the one a
/// temporal already prints, the digits a decimal keeps at its scale, and the
/// standard base64 a byte payload reads back from.
pub(crate) fn text(value: &Scalar) -> Result<smol_str::SmolStr> {
    use base64::Engine as _;

    Ok(match value {
        Scalar::Boolean(value) => {
            smol_str::SmolStr::new_static(if value.get() { "true" } else { "false" })
        }
        Scalar::Integer(value) => {
            if value.is_negative() {
                format_smolstr!("-{}", value.magnitude())
            } else {
                format_smolstr!("{}", value.magnitude())
            }
        }
        Scalar::Floating(value) => float_text(value.as_f64()),
        Scalar::Decimal(value) => smol_str::SmolStr::new(
            crate::types::decimal::scalars::decimal_text(value.coefficient(), value.scale()),
        ),
        Scalar::Text(value) => smol_str::SmolStr::new(value.as_str()),
        Scalar::Ascii(value) => smol_str::SmolStr::new(value.as_str()),
        Scalar::Enum(value) => smol_str::SmolStr::new(value.as_str()),
        Scalar::Version(value) => format_smolstr!("{value}"),
        Scalar::Uuid(value) => format_smolstr!("{value}"),
        Scalar::Bytes(value) => smol_str::SmolStr::new(
            base64::engine::general_purpose::STANDARD.encode(value.as_bytes()),
        ),
        Scalar::Geospatial(value) => smol_str::SmolStr::new(
            base64::engine::general_purpose::STANDARD.encode(value.as_bytes()),
        ),
        Scalar::Temporal(Temporal::Interval(_)) => {
            return Err(codec_error(
                "a calendar interval has no text spelling to write as character data",
            ));
        }
        Scalar::Temporal(_) => value.into_temporal_text().ok_or_else(|| {
            codec_error("a temporal beyond the classic spelling has no character data")
        })?,
        Scalar::Null | Scalar::Nested(_) => {
            return Err(codec_error(
                "expected a leaf value to write as character data",
            ));
        }
    })
}

fn float_text(value: f64) -> smol_str::SmolStr {
    if value.is_nan() {
        return smol_str::SmolStr::new_static("NaN");
    }
    if value == f64::INFINITY {
        return smol_str::SmolStr::new_static("inf");
    }
    if value == f64::NEG_INFINITY {
        return smol_str::SmolStr::new_static("-inf");
    }
    format_smolstr!("{value}")
}

/// Which markup delimiters one position has to escape.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Escaping {
    Content,
    Attribute,
}

/// Write text as character data, escaping what markup would otherwise claim.
fn write_escaped<W: Write>(writer: &mut W, value: &str, escaping: Escaping) -> Result<()> {
    let mut written = 0;
    for (index, character) in value.char_indices() {
        let replacement = match character {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' if escaping == Escaping::Attribute => "&quot;",
            // A literal end-of-line or tab inside an attribute is normalized
            // to a space when it is read back, so it is written as the
            // reference that survives that normalization.
            '\n' if escaping == Escaping::Attribute => "&#10;",
            '\t' if escaping == Escaping::Attribute => "&#9;",
            // Carriage return is normalized away everywhere, in content too.
            '\r' => "&#13;",
            '\t' | '\n' => continue,
            // U+FFFE and U+FFFF are not XML characters and no reference can
            // spell them, so a value holding one has no document to be written
            // into rather than one another parser would reject.
            character
                if (character as u32) < 0x20 || matches!(character, '\u{fffe}' | '\u{ffff}') =>
            {
                return Err(Error::Codec {
                    format: FORMAT,
                    position: index,
                    reason: format_smolstr!(
                        "expected a character XML can carry, got U+{:04X}",
                        character as u32
                    ),
                });
            }
            _ => continue,
        };
        writer.write_all(&value.as_bytes()[written..index])?;
        writer.write_all(replacement.as_bytes())?;
        written = index + character.len_utf8();
    }
    writer.write_all(&value.as_bytes()[written..])?;
    Ok(())
}

/// Check one element or attribute name against the XML `Name` production.
pub(crate) fn check_name(name: &str) -> Result<()> {
    let mut characters = name.char_indices();
    let Some((_, first)) = characters.next() else {
        return Err(codec_error("expected an XML name, got an empty name"));
    };
    if !is_name_start(first) {
        return Err(invalid_name(name, 0));
    }
    for (index, character) in characters {
        if !is_name_char(character) {
            return Err(invalid_name(name, index));
        }
    }
    Ok(())
}

const fn is_name_start(character: char) -> bool {
    matches!(character,
        ':' | '_' | 'A'..='Z' | 'a'..='z'
        | '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{2ff}'
        | '\u{370}'..='\u{37d}' | '\u{37f}'..='\u{1fff}' | '\u{200c}'..='\u{200d}'
        | '\u{2070}'..='\u{218f}' | '\u{2c00}'..='\u{2fef}' | '\u{3001}'..='\u{d7ff}'
        | '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}' | '\u{10000}'..='\u{effff}')
}

const fn is_name_char(character: char) -> bool {
    is_name_start(character)
        || matches!(character,
            '-' | '.' | '0'..='9' | '\u{b7}' | '\u{300}'..='\u{36f}' | '\u{203f}'..='\u{2040}')
}

fn invalid_name(name: &str, position: usize) -> Error {
    Error::Codec {
        format: FORMAT,
        position,
        reason: crate::text::expected_got(
            "an XML element or attribute name",
            crate::text::elide_display(&name),
        ),
    }
}

fn codec_error(reason: &'static str) -> Error {
    Error::Codec {
        format: FORMAT,
        position: 0,
        reason: smol_str::SmolStr::new_static(reason),
    }
}

/// Check a value's element nesting before a destination is opened.
///
/// A sequence is the repetition of the element around it rather than a level
/// of its own, so it costs no depth here, exactly as it costs none on the wire.
pub(crate) fn check_depth(value: &Scalar, maximum: usize) -> Result<()> {
    fn visit(value: &Scalar, depth: usize, maximum: usize) -> Result<()> {
        if depth > maximum {
            return Err(codec_error("nesting depth limit exceeded while encoding"));
        }
        match value {
            Scalar::Nested(Nested::Record(entries)) => {
                for value in entries.as_map().values() {
                    visit(value, depth.saturating_add(1), maximum)?;
                }
            }
            Scalar::Nested(Nested::Mapping(entries)) => {
                for (_, value) in entries.as_slice() {
                    visit(value, depth.saturating_add(1), maximum)?;
                }
            }
            Scalar::Nested(Nested::Sequence(values)) => {
                for value in values.as_slice() {
                    visit(value, depth, maximum)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    visit(value, 1, maximum)
}
