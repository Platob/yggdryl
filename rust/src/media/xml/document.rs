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
//! - An element carrying **attributes beside its characters** -
//!   `<Amt Ccy="EUR">9.50</Amt>`, the commonest shape in a real document - is
//!   not mixed content: it is one value with annotations. Its characters land
//!   in a column named [`VALUE_COLUMN`], which is the same name the crate
//!   already gives the payload of a root that is not a record.
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

/// How one document spelled each of a row's columns.
///
/// Collected while the rows are read, because that is the only time it is
/// known, and applied to the inferred field so a write can put a document back
/// the way it was found. A column spelled two ways across rows keeps the first
/// spelling; the read already refuses the two spellings that actually collide,
/// inside one element.
#[derive(Debug, Default)]
pub(crate) struct Spelling {
    columns: Vec<(SmolStr, Kind, Option<SmolStr>)>,
}

impl Spelling {
    /// Record one column's spelling, keeping the first seen.
    fn observe(&mut self, name: &str, kind: Kind, namespace: Option<&str>) {
        if self.columns.iter().any(|(held, _, _)| held == name) {
            return;
        }
        self.columns
            .push((SmolStr::new(name), kind, namespace.map(SmolStr::new)));
    }

    /// Restate a field so each column says how a document spells it.
    ///
    /// # Errors
    ///
    /// Returns a metadata failure.
    pub(crate) fn apply(&self, field: crate::Field) -> Result<crate::Field> {
        use crate::DataType;
        use crate::types::protocol::XmlKind;

        let DataType::Struct(children) = field.dtype() else {
            return Ok(field);
        };
        let mut rebuilt = Vec::with_capacity(children.len());
        for child in children.iter() {
            let mut child = child.clone();
            if let Some((_, kind, namespace)) = self
                .columns
                .iter()
                .find(|(held, _, _)| held.as_str() == child.name())
            {
                let spelling = match kind {
                    Kind::Attribute => XmlKind::Attribute,
                    Kind::Element => XmlKind::Element,
                };
                let mut view = child.as_xml_mut();
                view.set_kind(spelling)?;
                view.set_namespace(namespace.as_deref())?;
            }
            rebuilt.push(child);
        }
        Ok(crate::Field::new(
            field.name(),
            DataType::from_fields(rebuilt)?,
            field.is_nullable(),
        ))
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
) -> Result<(SmolStr, Vec<Scalar>, Spelling)> {
    let mut cursor = Cursor::new(input, limits);
    let Step::Open { empty, .. } = cursor.next()? else {
        return Err(codec_error(
            cursor.position(),
            "expected one XML document element",
        ));
    };
    let mut rows = Vec::new();
    let mut spelling = Spelling::default();
    let mut row_name: Option<SmolStr> = row_element.map(SmolStr::new);
    let mut row_namespace: Option<Option<SmolStr>> = None;
    if empty {
        // A document element that closed in its own tag still has to be the
        // whole document.
        finish_document(&mut cursor)?;
        return Ok((row_name.unwrap_or_default(), rows, spelling));
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
                    // A named row element is the one fact a caller stated, so
                    // it is looked for wherever it sits rather than only among
                    // the document element's own children.
                    (Some(wanted), _) => name.local() == wanted,
                    (None, Some(held)) => {
                        if name.local() != held.as_str() {
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
                        true
                    }
                    (None, None) => {
                        row_name = Some(SmolStr::new(name.local()));
                        true
                    }
                };
                if selected {
                    // Two rows sharing a local name but bound in different
                    // namespaces are two vocabularies, which is the same
                    // collision a column refuses.
                    let namespace = name.namespace().map(SmolStr::new);
                    match &row_namespace {
                        None => row_namespace = Some(namespace),
                        Some(held) if held.as_deref() == namespace.as_deref() => {}
                        Some(held) => {
                            return Err(codec_error(
                                cursor.position(),
                                format_smolstr!(
                                    "expected one namespace for the row <{}>, got {} and {}",
                                    quoted(name.local()),
                                    quoted(held.as_deref().unwrap_or("none")),
                                    quoted(namespace.as_deref().unwrap_or("none"))
                                ),
                            ));
                        }
                    }
                    let row = element_spelled(
                        &mut cursor,
                        &name,
                        attributes,
                        empty,
                        nil,
                        Some(&mut spelling),
                    )?;
                    rows.push(row_value(row));
                } else if !empty && row_element.is_none() {
                    // Nothing but rows is expected here, so a sibling costs
                    // the bytes of its own tags and nothing more.
                    cursor.skip(&name)?;
                }
                // With a row element named, a sibling may still hold it, so
                // the walk descends into it rather than skipping past it.
            }
            // A wrapper closing while a named row is still being looked for
            // is not the end of anything.
            Step::Close { .. } if cursor.depth() > 0 => {}
            // The document element's own character data is not a row. Only
            // whitespace can sit between rows; anything else is content the
            // row model has no cell for.
            Step::Close { text } => {
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
            Step::End => break,
        }
    }
    finish_document(&mut cursor)?;
    Ok((row_name.unwrap_or_default(), rows, spelling))
}

/// Require that the document ended where its element did.
///
/// A record read has exactly the same one-document rule a value read has: a
/// second root, or trailing text, is not rows this can publish.
fn finish_document(cursor: &mut Cursor<'_>) -> Result<()> {
    match cursor.next()? {
        Step::End => Ok(()),
        _ => Err(codec_error(
            cursor.position(),
            "expected one document element, got content after it",
        )),
    }
}

/// Restate one row's value as the record a row has to be.
///
/// A row is a struct by contract, but a row element can be written as a leaf -
/// `<row>text</row>` - or carry nothing at all - `<row/>`. A leaf row takes
/// the same wrapping a root that is not a record takes elsewhere in the crate,
/// under [`VALUE_COLUMN`]; an empty one is a row whose every column is absent.
fn row_value(value: Scalar) -> Scalar {
    match &value {
        Scalar::Record(_) | Scalar::Null => value,
        _ => {
            let text = value.as_str().unwrap_or_default();
            if text.is_empty() {
                return Scalar::from_record(std::iter::empty::<(SmolStr, Scalar)>())
                    .unwrap_or(Scalar::Null);
            }
            Scalar::from_record([(SmolStr::new_static(VALUE_COLUMN), value.clone())])
                .unwrap_or(value)
        }
    }
}

/// Read one open element's value, consuming up to and including its close.
fn element(
    cursor: &mut Cursor<'_>,
    name: &Name,
    attributes: Vec<Attr>,
    empty: bool,
    nil: bool,
) -> Result<Scalar> {
    element_spelled(cursor, name, attributes, empty, nil, None)
}

/// Read one open element, recording how it spelled its own columns.
fn element_spelled(
    cursor: &mut Cursor<'_>,
    name: &Name,
    attributes: Vec<Attr>,
    empty: bool,
    nil: bool,
    mut spelling: Option<&mut Spelling>,
) -> Result<Scalar> {
    // Columns are collected in document order and grouped once at the close,
    // because a repeat is only known to be one after the second occurrence.
    let mut columns: Vec<(SmolStr, Scalar)> = Vec::new();
    let mut claimed = Claims::default();
    for attribute in attributes {
        claim(
            &mut claimed,
            &attribute.name,
            cursor.position(),
            Kind::Attribute,
        )?;
        if let Some(spelling) = spelling.as_deref_mut() {
            spelling.observe(
                attribute.name.local(),
                Kind::Attribute,
                attribute.name.namespace(),
            );
        }
        columns.push((
            SmolStr::new(attribute.name.local()),
            Scalar::from(attribute.value.as_str()),
        ));
    }
    let attribute_count = columns.len();

    if empty {
        return finish(columns, SmolStr::default(), nil, attribute_count);
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
                if let Some(spelling) = spelling.as_deref_mut() {
                    spelling.observe(child.local(), Kind::Element, child.namespace());
                }
                let value = element(cursor, &child, attributes, empty, nil)?;
                columns.push((SmolStr::new(child.local()), value));
            }
            Step::Close { text } => {
                // Characters beside *elements* are mixed content and have no
                // cell; characters beside *attributes* are the element's own
                // value, and [`finish`] gives them one.
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
                return finish(columns, text, nil, attribute_count);
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
pub(crate) enum Kind {
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
fn claim(seen: &mut Claims, name: &Name, position: usize, kind: Kind) -> Result<()> {
    let local = SmolStr::new(name.local());
    let namespace = name.namespace().map(SmolStr::new);
    if let Some(held) = seen.index.get(&local).map(|at| &seen.held[*at]) {
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
        if kind == Kind::Attribute {
            return Err(codec_error(
                position,
                format_smolstr!(
                    "expected two attributes to have different names, got {} twice",
                    quoted(&local)
                ),
            ));
        }
        if held.kind != kind {
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
    seen.index.insert(local, seen.held.len());
    seen.held.push(Claim { namespace, kind });
    Ok(())
}

/// The names claimed inside one element, and where each was claimed.
#[derive(Default)]
struct Claims {
    held: Vec<Claim>,
    index: std::collections::HashMap<SmolStr, usize>,
}

/// Turn one element's collected parts into the value it states.
fn finish(
    mut columns: Vec<(SmolStr, Scalar)>,
    text: SmolStr,
    nil: bool,
    attribute_count: usize,
) -> Result<Scalar> {
    if nil {
        return Ok(Scalar::Null);
    }
    if columns.is_empty() {
        // A leaf: no attributes, no children, so the characters are the cell.
        return Ok(Scalar::from(text.as_str()));
    }
    // Attributes beside characters: the characters are the element's value and
    // get the name the crate already uses for one. A document that also spells
    // an attribute `value` is claiming the name twice, which is refused where
    // every other double claim is.
    if columns.len() == attribute_count && !text.trim().is_empty() {
        if columns.iter().any(|(name, _)| name == VALUE_COLUMN) {
            return Err(codec_error(
                0,
                format_smolstr!(
                    "expected the name {} once, got an attribute beside the characters it names",
                    quoted(VALUE_COLUMN)
                ),
            ));
        }
        columns.push((
            SmolStr::new_static(VALUE_COLUMN),
            Scalar::from(text.as_str()),
        ));
    }
    Ok(group(columns))
}

/// Collapse repeated names into sequences and answer the record.
///
/// Repeats are found through a name index rather than by rescanning what has
/// been collected, so one element costs its own width rather than its width
/// squared - which is what an element with a few hundred columns would pay.
fn group(columns: Vec<(SmolStr, Scalar)>) -> Scalar {
    let mut named: Vec<(SmolStr, Vec<Scalar>)> = Vec::with_capacity(columns.len());
    let mut index: std::collections::HashMap<SmolStr, usize> =
        std::collections::HashMap::with_capacity(columns.len());
    for (name, value) in columns {
        match index.get(&name) {
            Some(at) => named[*at].1.push(value),
            None => {
                index.insert(name.clone(), named.len());
                named.push((name, vec![value]));
            }
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

/// The column an element's own characters take when it also has attributes.
///
/// The same name the crate gives the payload of a root that is not a record,
/// rather than a sigil invented here.
pub const VALUE_COLUMN: &str = "value";
