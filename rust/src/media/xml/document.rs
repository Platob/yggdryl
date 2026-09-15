//! One XML document as one [`Scalar`], and the rows inside it.
//!
//! The mapping is the whole contract, so it is stated here once and read by
//! both surfaces:
//!
//! - An element's **attributes and its child elements share one namespace of
//!   column names**, each keyed by its local name. XML spells the same fact
//!   two ways and a row has one cell for it, so both spellings are accepted -
//!   and a name claimed by an attribute *and* a child of the same element is
//!   refused rather than silently disambiguated.
//! - A **namespace prefix names a document's own vocabulary, never a field**,
//!   so it is dropped. Two different namespaces folding onto one local name
//!   inside one element are two vocabularies, and that is refused too.
//! - A **leaf** is an element with no attributes and no element children: its
//!   character data is the cell.
//! - **Repeated** same-named children collapse into one sequence, in document
//!   order, because a record names each field once.
//! - **Mixed content** - character data beside element children - is the one
//!   shape the row model cannot hold, and it is refused by name.
//! - `xsi:nil="true"` is null; `<x/>` and `<x></x>` are the empty string; an
//!   absent attribute or element is null.
//!
//! Nothing here reads a type. Every leaf is the text it was written as, and
//! what turns that text into a decimal or an instant is a declared
//! [`Field`](crate::Field), through the one value contract.

use smol_str::{SmolStr, format_smolstr};

use crate::text::Limits;
use crate::{Result, Scalar};

use super::reader::{Attr, Cursor, Name, Step, codec_error, quoted};

/// Read one whole document as the value its document element states.
///
/// # Errors
///
/// Returns a parse, limit, or shape refusal naming its byte position.
pub(crate) fn read_document(input: &[u8], limits: Limits) -> Result<Scalar> {
    let mut cursor = Cursor::new(input, limits);
    let value = match cursor.next()? {
        Step::Open {
            name,
            attributes,
            empty,
            nil,
        } => element(&mut cursor, &name, attributes, empty, nil)?,
        Step::End => {
            return Err(codec_error(
                cursor.position(),
                "expected one XML document element",
            ));
        }
        Step::Close { .. } => {
            return Err(codec_error(
                cursor.position(),
                "expected an element to open",
            ));
        }
    };
    match cursor.next()? {
        Step::End => Ok(value),
        _ => Err(codec_error(
            cursor.position(),
            "expected one document element, got content after it",
        )),
    }
}

/// Read every row the document element holds.
///
/// Rows are the document element's element children. They all name one
/// element: a document whose children disagree is a shape this cannot publish
/// one field for, and it says so rather than picking the first.
///
/// `row_element` names the row explicitly and skips the agreement rule, which
/// is what reads a document whose wrapper holds more than rows.
///
/// # Errors
///
/// Returns a parse, limit, or shape refusal naming its byte position.
pub(crate) fn read_rows(
    input: &[u8],
    limits: Limits,
    row_element: Option<&str>,
) -> Result<(SmolStr, Vec<Scalar>)> {
    let mut cursor = Cursor::new(input, limits);
    let Step::Open { empty, .. } = cursor.next()? else {
        return Err(codec_error(
            cursor.position(),
            "expected one XML document element",
        ));
    };
    let mut rows = Vec::new();
    let mut row_name: Option<SmolStr> = row_element.map(SmolStr::new);
    if empty {
        return Ok((row_name.unwrap_or_default(), rows));
    }
    loop {
        match cursor.next()? {
            Step::Open {
                name,
                attributes,
                empty,
                nil,
            } => {
                let selected = match (row_element, &row_name) {
                    (Some(wanted), _) => name.local() == wanted,
                    (None, Some(held)) => {
                        if name.local() == held.as_str() {
                            true
                        } else {
                            return Err(codec_error(
                                cursor.position(),
                                format_smolstr!(
                                    "expected every row to be <{}>, got <{}>; name the row with \
                                     the row element option",
                                    quoted(held),
                                    quoted(name.local())
                                ),
                            ));
                        }
                    }
                    (None, None) => {
                        row_name = Some(SmolStr::new(name.local()));
                        true
                    }
                };
                if selected {
                    rows.push(element(&mut cursor, &name, attributes, empty, nil)?);
                } else if !empty {
                    cursor.skip(&name)?;
                }
            }
            // The document element's own character data is not a row. Only
            // whitespace can sit between rows; anything else is content the
            // row model has no cell for.
            Step::Close { text } if cursor.depth() == 0 => {
                if !text.trim().is_empty() {
                    return Err(codec_error(
                        cursor.position(),
                        format_smolstr!(
                            "expected only rows under the document element, got the text {}",
                            quoted(text.trim())
                        ),
                    ));
                }
                break;
            }
            Step::Close { .. } => break,
            Step::End => break,
        }
    }
    Ok((row_name.unwrap_or_default(), rows))
}

/// Read one open element's value, consuming up to and including its close.
fn element(
    cursor: &mut Cursor<'_>,
    name: &Name,
    attributes: Vec<Attr>,
    empty: bool,
    nil: bool,
) -> Result<Scalar> {
    // Columns are collected in document order and grouped once at the close,
    // because a repeat is only known to be one after the second occurrence.
    let mut columns: Vec<(SmolStr, Scalar)> = Vec::new();
    let mut claimed: Vec<Claim> = Vec::new();
    for attribute in attributes {
        claim(
            &mut claimed,
            &attribute.name,
            cursor.position(),
            Kind::Attribute,
        )?;
        columns.push((
            SmolStr::new(attribute.name.local()),
            Scalar::from(attribute.value.as_str()),
        ));
    }
    let attribute_count = columns.len();

    if empty {
        return Ok(finish(columns, SmolStr::default(), nil, attribute_count));
    }

    loop {
        match cursor.next()? {
            Step::Open {
                name: child,
                attributes,
                empty,
                nil,
            } => {
                claim(&mut claimed, &child, cursor.position(), Kind::Element)?;
                let value = element(cursor, &child, attributes, empty, nil)?;
                columns.push((SmolStr::new(child.local()), value));
            }
            Step::Close { text } => {
                if columns.len() > attribute_count && !text.trim().is_empty() {
                    return Err(codec_error(
                        cursor.position(),
                        format_smolstr!(
                            "expected <{}> to hold either character data or child elements, \
                             got both: the text {}",
                            quoted(name.local()),
                            quoted(text.trim())
                        ),
                    ));
                }
                return Ok(finish(columns, text, nil, attribute_count));
            }
            Step::End => {
                return Err(codec_error(
                    cursor.position(),
                    format_smolstr!("expected <{}> to close", quoted(name.local())),
                ));
            }
        }
    }
}

/// Which spelling claimed a column name.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Attribute,
    Element,
}

impl Kind {
    const fn spelling(self) -> &'static str {
        match self {
            Self::Attribute => "an attribute",
            Self::Element => "a child element",
        }
    }
}

/// One column name already claimed inside the element being read.
struct Claim {
    local: SmolStr,
    namespace: Option<SmolStr>,
    kind: Kind,
}

/// Record one name, refusing a second claim that means something else.
///
/// One local name inside one element belongs to one thing. Two spellings of
/// it - an attribute beside a child element - or two namespaces folding onto
/// it are two facts competing for one cell, and this is where that is named.
/// A child element repeating its own name is not a competing claim: it is a
/// sequence, which is what [`group`] makes of it.
fn claim(seen: &mut Vec<Claim>, name: &Name, position: usize, kind: Kind) -> Result<()> {
    let local = SmolStr::new(name.local());
    let namespace = name.namespace().map(SmolStr::new);
    if let Some(held) = seen.iter().find(|candidate| candidate.local == local) {
        if held.namespace.as_deref() != namespace.as_deref() {
            return Err(codec_error(
                position,
                format_smolstr!(
                    "expected one namespace for the name {}, got {} and {}",
                    quoted(&local),
                    quoted(held.namespace.as_deref().unwrap_or("none")),
                    quoted(namespace.as_deref().unwrap_or("none"))
                ),
            ));
        }
        if held.kind != kind || kind == Kind::Attribute {
            return Err(codec_error(
                position,
                format_smolstr!(
                    "expected the name {} to be spelled one way, got {} and {}",
                    quoted(&local),
                    held.kind.spelling(),
                    kind.spelling()
                ),
            ));
        }
        return Ok(());
    }
    seen.push(Claim {
        local,
        namespace,
        kind,
    });
    Ok(())
}

/// Turn one element's collected parts into the value it states.
fn finish(
    columns: Vec<(SmolStr, Scalar)>,
    text: SmolStr,
    nil: bool,
    attribute_count: usize,
) -> Scalar {
    if nil {
        return Scalar::Null;
    }
    if columns.is_empty() {
        // A leaf: no attributes, no children, so the characters are the cell.
        return Scalar::from(text.as_str());
    }
    let _ = attribute_count;
    group(columns)
}

/// Collapse repeated names into sequences and answer the record.
fn group(columns: Vec<(SmolStr, Scalar)>) -> Scalar {
    let mut named: Vec<(SmolStr, Vec<Scalar>)> = Vec::with_capacity(columns.len());
    for (name, value) in columns {
        match named.iter_mut().find(|(held, _)| held == &name) {
            Some((_, values)) => values.push(value),
            None => named.push((name, vec![value])),
        }
    }
    let entries = named.into_iter().map(|(name, mut values)| {
        let value = if values.len() == 1 {
            values.pop().unwrap_or(Scalar::Null)
        } else {
            Scalar::from_sequence(values)
        };
        (name, value)
    });
    // Every name is unique by construction here, so the record cannot refuse.
    Scalar::from_record(entries).unwrap_or(Scalar::Null)
}

/// What a row element is called when a document proved no name.
pub(crate) const DEFAULT_ROW_NAME: &str = "row";
