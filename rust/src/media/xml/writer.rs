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

use smol_str::format_smolstr;

use crate::text::{Formatting, Indent};
use crate::{Error, Result, Scalar};

use super::reader::quoted;

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

/// Whether a name can be written as an XML element or attribute name.
///
/// The writer validates, because quick-xml's does not: it would emit an
/// ill-formed document rather than refuse one. This is the production for
/// `Name` narrowed to what a column can be called.
fn is_name(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !(first.is_alphabetic() || first == '_') {
        return false;
    }
    characters.all(|character| character.is_alphanumeric() || matches!(character, '_' | '-' | '.'))
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

/// Write one document: a root element holding `rows`.
///
/// # Errors
///
/// Returns a write failure, or a refusal naming a value or a column name XML
/// cannot spell.
pub(crate) fn write_document<W: Write>(
    out: &mut W,
    root: &str,
    row: &str,
    rows: &[Scalar],
    formatting: Formatting,
) -> Result<()> {
    check_name(root)?;
    check_name(row)?;
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
        write_element(&mut text, row, value, 1, formatting)?;
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
        Scalar::Record(_) => write_record(out, name, value, depth, formatting),
        Scalar::Sequence(values) => {
            // A sequence is a repeat of this element, not an element holding
            // a list, so the name is written once per item.
            let values = values.as_slice();
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

/// Render one leaf as the text a document spells it with.
///
/// Every leaf crosses XML as text, and the crate already has exactly one
/// renderer for "this scalar, as text": the one a `column=value` directory
/// writes through. A second spelling here would be a second answer to what a
/// decimal or an instant looks like.
fn leaf_text(value: &Scalar) -> Result<smol_str::SmolStr> {
    crate::media::partition::partition_text(value)
}
