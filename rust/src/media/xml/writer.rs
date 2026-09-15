//! One [`Scalar`] as one XML document.
//!
//! # Escaping is the whole of correctness here
//!
//! An XML parser rewrites some characters before a reader ever sees them, so a
//! writer that escapes only the markup characters loses data that no later
//! layer can recover:
//!
//! - **Attribute-value normalization** (XML 1.0 3.3.3) replaces every tab,
//!   newline and carriage return in an attribute value with a space. A tab
//!   written literally comes back as a space.
//! - **Line-ending normalization** (XML 1.0 2.11) turns a carriage return into
//!   a newline everywhere, text content included.
//!
//! So a character a parser would rewrite is written as a character reference
//! instead, which normalization leaves alone. quick-xml's own `escape` covers
//! `< > & ' "` and none of the three whitespace characters, which is why this
//! module owns the two escape sets rather than calling it:
//!
//! So a character a parser would rewrite is written as a character reference
//! instead, which normalization leaves alone. A text node therefore escapes
//! `&`, `<`, `>` and U+000D. quick-xml's own `escape` covers `< > & ' "` and
//! none of the three whitespace characters, which is why this module owns the
//! set rather than calling it.
//!
//! `>` is escaped because the `]]>` sequence is forbidden in text and escaping
//! the `>` is the one rule that needs no lookbehind.
//!
//! A written leaf is always a child element, never an attribute - a document
//! spells the same fact both ways and a writer picks one - so an attribute
//! value's own escape set has no caller here. The one attribute this writes,
//! `xsi:nil="true"`, is a constant.

use std::io::Write;

use smol_str::{SmolStr, format_smolstr};

use crate::text::{Formatting, Indent};
use crate::{Error, Result, Scalar};

use super::reader::quoted;

/// Refuse a value XML cannot carry at all.
///
/// XML 1.0's `Char` production excludes most of the C0 controls and the two
/// non-characters at the end of the BMP, and a reference cannot spell them
/// either - `&#0;` is as illegal as a raw NUL. So there is no escape to reach
/// for and a refusal is the only correct answer, exactly as it is for a column
/// name no element can be called.
fn check_text(value: &str) -> Result<()> {
    let Some(character) = value.chars().find(|character| {
        let code = *character as u32;
        !(matches!(code, 0x9 | 0xA | 0xD)
            || matches!(code, 0x20..=0xD7FF)
            || matches!(code, 0xE000..=0xFFFD)
            || matches!(code, 0x10000..=0x10FFFF))
    }) else {
        return Ok(());
    };
    Err(Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: format_smolstr!(
            "expected a value XML can carry, got U+{:04X}, which no document and no character \
             reference can spell",
            character as u32
        ),
    })
}

/// Escape one text node's characters into `out`.
pub(crate) fn escape_text(value: &str, out: &mut String) {
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#xD;"),
            other => out.push(other),
        }
    }
}

/// Whether a scalar may open an XML name.
const fn is_name_start(character: char) -> bool {
    matches!(character as u32,
        0x41..=0x5A | 0x5F | 0x61..=0x7A | 0xC0..=0xD6 | 0xD8..=0xF6 | 0xF8..=0x2FF
        | 0x370..=0x37D | 0x37F..=0x1FFF | 0x200C..=0x200D | 0x2070..=0x218F
        | 0x2C00..=0x2FEF | 0x3001..=0xD7FF | 0xF900..=0xFDCF | 0xFDF0..=0xFFFD
        | 0x10000..=0xEFFFF)
}

/// Whether a scalar may continue an XML name.
const fn is_name_char(character: char) -> bool {
    is_name_start(character)
        || matches!(character as u32,
            0x2D | 0x2E | 0x30..=0x39 | 0xB7 | 0x300..=0x36F | 0x203F..=0x2040)
}

/// Whether a name can be written as an XML element or attribute name.
///
/// The writer validates, because quick-xml's does not: it would emit an
/// ill-formed document rather than refuse one. This is XML's own `Name`
/// production minus the colon, which would spell a namespace prefix nothing
/// here declared. Anything a read can name a column, a write can spell.
fn is_name(value: &str) -> bool {
    let mut characters = value.chars();
    characters.next().is_some_and(is_name_start) && characters.all(is_name_char)
}

/// Refuse a column whose name no document can spell.
fn check_name(name: &str) -> Result<()> {
    if is_name(name) {
        return Ok(());
    }
    Err(Error::InvalidRecord {
        path: format_smolstr!("$.{name}"),
        reason: format_smolstr!(
            "expected a column name an XML element can be called, got {}",
            quoted(name)
        ),
    })
}

/// Write one value as a whole document rooted at `name`.
///
/// # Errors
///
/// Returns a write failure, or a refusal naming a value or a column name XML
/// cannot spell.
pub(crate) fn write_value<W: Write>(
    out: &mut W,
    name: &str,
    value: &Scalar,
    formatting: Formatting,
) -> Result<()> {
    check_name(name)?;
    let mut text = String::new();
    text.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    if !matches!(formatting.indent(), Indent::None) {
        text.push('\n');
    }
    write_element(&mut text, name, value, 0, formatting)?;
    if !matches!(formatting.indent(), Indent::None) {
        text.push('\n');
    }
    out.write_all(text.as_bytes()).map_err(Error::from)
}

/// Write one document whose rows are spelled as `field` declares.
///
/// The declaration is what carries the spellings a document used - which
/// columns were attributes, which were the element's own characters, what each
/// was called on the wire - so a document read under a field and written back
/// under the same field comes out the way it went in.
///
/// # Errors
///
/// Returns a write failure, or a refusal naming a value or a column name XML
/// cannot spell.
pub(crate) fn write_rows<W: Write>(
    out: &mut W,
    root: &str,
    field: &crate::Field,
    rows: &[Scalar],
    formatting: Formatting,
) -> Result<()> {
    check_name(root)?;
    let row = field.as_xml().wire_name().to_owned();
    check_name(&row)?;
    let mut text = String::new();
    text.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    let newline = !matches!(formatting.indent(), Indent::None);
    if newline {
        text.push('\n');
    }
    text.push('<');
    text.push_str(root);
    text.push('>');
    for value in rows {
        if newline {
            text.push('\n');
        }
        write_declared(&mut text, &row, field, value, 1, formatting)?;
    }
    if newline {
        text.push('\n');
    }
    text.push_str("</");
    text.push_str(root);
    text.push('>');
    if newline {
        text.push('\n');
    }
    out.write_all(text.as_bytes()).map_err(Error::from)
}

/// Write one value under the field that says how it is spelled.
fn write_declared(
    out: &mut String,
    name: &str,
    field: &crate::Field,
    value: &Scalar,
    depth: usize,
    formatting: Formatting,
) -> Result<()> {
    use crate::DataType;

    let children = match field.dtype() {
        DataType::Struct(children) => children,
        // Only a struct carries per-column spellings; anything else is written
        // the way an undeclared value is.
        _ => return write_element(out, name, value, depth, formatting),
    };
    let Some(record) = value.as_record() else {
        return write_element(out, name, value, depth, formatting);
    };

    indent(out, depth, formatting);
    out.push('<');
    out.push_str(name);
    // Attributes are written into the open tag, so they are collected first.
    for child in children.iter() {
        let spelling = child.as_xml();
        if !spelling.kind()?.is_attribute() {
            continue;
        }
        let Some(held) = record.get(child.name()).filter(|held| !held.is_null()) else {
            continue;
        };
        let wire = spelling.wire_name().to_owned();
        check_name(&wire)?;
        let rendered = leaf_text(held)?;
        check_text(&rendered)?;
        out.push(' ');
        out.push_str(&wire);
        out.push_str("=\"");
        escape_attribute(&rendered, out);
        out.push('"');
    }
    out.push('>');

    // Then the element's own characters, if a column claims them.
    let mut wrote_child = false;
    for child in children.iter() {
        if !child.as_xml().kind()?.is_text() {
            continue;
        }
        if let Some(held) = record.get(child.name()).filter(|held| !held.is_null()) {
            let rendered = leaf_text(held)?;
            check_text(&rendered)?;
            escape_text(&rendered, out);
        }
    }

    for child in children.iter() {
        let spelling = child.as_xml();
        let kind = spelling.kind()?;
        if kind.is_attribute() || kind.is_text() {
            continue;
        }
        let Some(held) = record.get(child.name()).filter(|held| !held.is_null()) else {
            continue;
        };
        let wire = spelling.wire_name().to_owned();
        if !matches!(formatting.indent(), Indent::None) {
            out.push('\n');
        }
        wrote_child = true;
        write_declared(out, &wire, child, held, depth.saturating_add(1), formatting)?;
    }
    if wrote_child && !matches!(formatting.indent(), Indent::None) {
        out.push('\n');
        indent(out, depth, formatting);
    }
    out.push_str("</");
    out.push_str(name);
    out.push('>');
    Ok(())
}

/// Escape one attribute value's characters into `out`.
///
/// Attribute-value normalization replaces a literal tab, newline or carriage
/// return with a space before any reader sees it, so those three are written
/// as character references, which normalization leaves alone.
fn escape_attribute(value: &str, out: &mut String) {
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            other => out.push(other),
        }
    }
}

/// Write the indentation one level asks for.
fn indent(out: &mut String, depth: usize, formatting: Formatting) {
    match formatting.indent() {
        // XML's whitespace between elements is insignificant, so the default
        // is the readable one rather than the compact one.
        Indent::None => {}
        Indent::Default => {
            for _ in 0..depth * 2 {
                out.push(' ');
            }
        }
        Indent::Spaces(width) => {
            for _ in 0..depth * usize::from(width) {
                out.push(' ');
            }
        }
        Indent::Tabs => {
            for _ in 0..depth {
                out.push('\t');
            }
        }
    }
}

/// Write one element named `name` holding `value`.
fn write_element(
    out: &mut String,
    name: &str,
    value: &Scalar,
    depth: usize,
    formatting: Formatting,
) -> Result<()> {
    check_name(name)?;
    let indented_start = out.len();
    indent(out, depth, formatting);
    // A null is absence, and absence is the element simply not being there -
    // except at a row's own level, where the row still has to exist.
    if value.is_null() {
        out.push('<');
        out.push_str(name);
        out.push_str(" xsi:nil=\"true\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"/>");
        return Ok(());
    }
    match value {
        // A record is the canonical named shape and a mapping with string
        // keys is the input shape of the same thing, so both are written as
        // the element they describe rather than as text.
        Scalar::Record(_) => write_record(out, name, value, depth, formatting),
        Scalar::Mapping(entries) if named_entries(entries.as_slice()).is_some() => {
            let named = named_entries(entries.as_slice()).unwrap_or_default();
            let record = Scalar::from_record(named).unwrap_or(Scalar::Null);
            write_record(out, name, &record, depth, formatting)
        }
        Scalar::Sequence(values) => {
            // A sequence is a repeat of this element, not an element holding
            // a list, so the name is written once per item.
            // The caller already indented, so the first item must not indent
            // a second time.
            let values = values.as_slice();
            out.truncate(indented_start);
            for (index, item) in values.iter().enumerate() {
                if index > 0 && !matches!(formatting.indent(), Indent::None) {
                    out.push('\n');
                }
                write_element(out, name, item, depth, formatting)?;
            }
            Ok(())
        }
        other => {
            let text = leaf_text(other)?;
            check_text(&text)?;
            out.push('<');
            out.push_str(name);
            out.push('>');
            escape_text(&text, out);
            out.push_str("</");
            out.push_str(name);
            out.push('>');
            Ok(())
        }
    }
}

/// Write one record as an element whose children are its fields.
fn write_record(
    out: &mut String,
    name: &str,
    value: &Scalar,
    depth: usize,
    formatting: Formatting,
) -> Result<()> {
    let Scalar::Record(record) = value else {
        return Ok(());
    };
    let fields = record.as_map();
    out.push('<');
    out.push_str(name);
    // A leaf field rides as an attribute only when a caller asked for it; the
    // default spelling is a child element, because that is the shape a schema
    // validates and the shape a nested value has to take anyway.
    out.push('>');
    let newline = !matches!(formatting.indent(), Indent::None);
    for (field, child) in fields {
        if child.is_null() {
            continue;
        }
        if newline {
            out.push('\n');
        }
        write_element(out, field, child, depth.saturating_add(1), formatting)?;
    }
    if newline {
        out.push('\n');
        indent(out, depth, formatting);
    }
    out.push_str("</");
    out.push_str(name);
    out.push('>');
    Ok(())
}

/// One mapping's entries, when every key names a column.
///
/// A mapping whose keys are all strings is a record written another way - the
/// shape a host runtime's dictionary arrives as - so it is accepted here for
/// the same reason a read accepts an attribute and a child element for one
/// column. A mapping keyed by anything else has no element name to be written
/// under, and falls through to the refusal every unspellable value gets.
fn named_entries(entries: &[(Scalar, Scalar)]) -> Option<Vec<(SmolStr, Scalar)>> {
    entries
        .iter()
        .map(|(key, value)| match key {
            Scalar::String(name) => Some((name.as_str().into(), value.clone())),
            _ => None,
        })
        .collect()
}

/// Render one leaf as the text a document spells it with.
///
/// Every leaf crosses XML as text, and the crate already has exactly one
/// renderer for "this scalar, as text": the one a `column=value` directory
/// writes through. A second spelling here would be a second answer to what a
/// decimal or an instant looks like.
fn leaf_text(value: &Scalar) -> Result<smol_str::SmolStr> {
    // A container has an element to be written as and is never text; reaching
    // here with one means it is a shape no element can hold.
    if matches!(
        value,
        Scalar::Mapping(_) | Scalar::Sequence(_) | Scalar::Record(_)
    ) {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: format_smolstr!(
                "expected a value an element can hold, got {}",
                quoted(&value.kind().to_string())
            ),
        });
    }
    crate::media::partition::partition_text(value)
}
