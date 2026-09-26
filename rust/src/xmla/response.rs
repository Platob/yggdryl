//! What a request is answered with: a rowset, nothing, or a fault.
//!
//! A `DiscoverResponse` and an `ExecuteResponse` share one shape: a `return`
//! element holding a `root`, whose namespace says what it is - the rowset
//! namespace for a tabular answer, the empty namespace for a command that
//! answers nothing, the dataset namespace for a multidimensional answer this
//! crate reads as the natural XML value it is. A failure is a SOAP fault
//! whose detail carries the `Error` element XML for Analysis defines, an
//! [`XmlaError`], and [`fault`] builds one.
//!
//! Writing streams: [`write_rowset`] opens the envelope and the response
//! element, writes the schema and each batch of rows as it is pulled, and
//! closes them, so an Execute over a large table never holds its answer whole.

use std::io::Write;

use smol_str::{SmolStr, format_smolstr};

use crate::soap::{Body, Envelope, EnvelopeWriter, Fault, FaultCode, Fragment};
use crate::xml::{ATTRIBUTE_PREFIX, Element, XSD_NAMESPACE, XSI_NAMESPACE};
use crate::{Error, Field, Result, Scalar, Serie};

use super::rowset::Rowset;
use super::vocabulary::{Content, Method};
use super::{
    EMPTY_NAMESPACE, EXCEPTION_NAMESPACE, MDDATASET_NAMESPACE, NAMESPACE, ROWSET_NAMESPACE,
};

/// The actor every fault this crate raises names.
pub const ACTOR: &str = "yggdryl";

/// The `Error` element XML for Analysis puts in a fault's detail: a numeric
/// code, a description, the component that raised it, and a help file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XmlaError {
    code: u32,
    description: String,
    source: String,
    help_file: String,
}

impl XmlaError {
    /// An error of `code`, explained by `description`, raised by
    /// [`ACTOR`].
    pub fn new(code: u32, description: impl Into<String>) -> Self {
        Self {
            code,
            description: description.into(),
            source: ACTOR.to_owned(),
            help_file: String::new(),
        }
    }

    /// Return this error naming another source.
    #[must_use]
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = source.into();
        self
    }

    /// The `ErrorCode`.
    #[must_use]
    pub const fn code(&self) -> u32 {
        self.code
    }

    /// The `Description`.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The `Source`.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The `HelpFile`, empty when none.
    #[must_use]
    pub fn help_file(&self) -> &str {
        &self.help_file
    }

    /// The `Error` element this is written as, in the exception namespace.
    ///
    /// # Errors
    ///
    /// Returns an error when the element cannot be built, which no text
    /// causes.
    pub fn into_fragment(&self) -> Result<Fragment> {
        Ok(Fragment::new(
            "Error",
            Scalar::from_struct([
                (
                    format_smolstr!("{ATTRIBUTE_PREFIX}xmlns"),
                    Scalar::from(EXCEPTION_NAMESPACE),
                ),
                (
                    format_smolstr!("{ATTRIBUTE_PREFIX}ErrorCode"),
                    Scalar::from(self.code.to_string()),
                ),
                (
                    format_smolstr!("{ATTRIBUTE_PREFIX}Description"),
                    Scalar::from(self.description.as_str()),
                ),
                (
                    format_smolstr!("{ATTRIBUTE_PREFIX}Source"),
                    Scalar::from(self.source.as_str()),
                ),
                (
                    format_smolstr!("{ATTRIBUTE_PREFIX}HelpFile"),
                    Scalar::from(self.help_file.as_str()),
                ),
            ])?,
        ))
    }

    /// Every `Error` element a fault's detail carries, directly or under a
    /// `Messages` element - and the `error` olap4j and Mondrian carry in their
    /// own namespace, its `code` and `desc` read as one - in the order the
    /// detail answers its elements: a
    /// fault built here as given, a read one in name order - so its direct
    /// `Error` elements come before those under `Messages`, each group in
    /// document order.
    #[must_use]
    pub fn from_fault(fault: &Fault) -> Vec<Self> {
        let mut errors = Vec::new();
        for fragment in fault.detail() {
            let element = fragment.element();
            if element.local_name() == "Error" {
                errors.extend(Self::read(&element));
            } else if element.local_name() == "Messages" {
                for child in element.children() {
                    if child.local_name() == "Error" {
                        errors.extend(Self::read(&child));
                    }
                }
            } else if element.is(Some(MONDRIAN_NAMESPACE), "error") {
                errors.extend(Self::read_mondrian(&element));
            }
        }
        errors
    }

    /// One `Error` element - `ErrorCode`, `Description`, `Source` and
    /// `HelpFile` (or olap4j's `Help`) as attributes, or its text as the
    /// description - wherever it stands: a fault's detail, or a rowset's
    /// `Messages`.
    #[must_use]
    pub fn from_element(element: &Element<'_>) -> Option<Self> {
        Self::read(element)
    }

    /// Write this error as the `Error` element, its four attributes, in the
    /// exception namespace the enclosing element declares.
    ///
    /// # Errors
    ///
    /// Returns the XML writer's refusal for text XML cannot carry, or the
    /// sink's failure.
    pub fn write<W: Write>(&self, writer: &mut W) -> Result<()> {
        write!(writer, "<Error ErrorCode=\"{}\" Description=\"", self.code)?;
        crate::xml::write_attribute_text(writer, &self.description)?;
        write!(writer, "\" Source=\"")?;
        crate::xml::write_attribute_text(writer, &self.source)?;
        write!(writer, "\" HelpFile=\"")?;
        crate::xml::write_attribute_text(writer, &self.help_file)?;
        write!(writer, "\"/>")?;
        Ok(())
    }

    fn read(element: &Element<'_>) -> Option<Self> {
        let attribute = |name: &str| {
            element
                .attribute(name)
                .and_then(Scalar::as_str)
                .map(str::to_owned)
        };
        let code = attribute("ErrorCode")
            .and_then(|code| code.trim().parse().ok())
            .unwrap_or(0);
        Some(Self {
            code,
            description: attribute("Description")
                .or_else(|| element.text().map(str::to_owned))
                .unwrap_or_default(),
            source: attribute("Source").unwrap_or_default(),
            // olap4j spells the attribute `Help`.
            help_file: attribute("HelpFile")
                .or_else(|| attribute("Help"))
                .unwrap_or_default(),
        })
    }

    /// The error olap4j and Mondrian carry in a fault's detail as
    /// `<XA:error xmlns:XA="http://mondrian.sourceforge.net"><code/><desc/>`.
    fn read_mondrian(element: &Element<'_>) -> Option<Self> {
        let child_text = |name: &str| {
            element
                .children()
                .find(|child| child.local_name() == name)
                .and_then(|child| child.text().map(str::to_owned))
        };
        let description = child_text("desc")?;
        let code = child_text("code")
            .and_then(|code| u32::from_str_radix(code.trim().trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);
        Some(Self {
            code,
            description,
            source: MONDRIAN.to_owned(),
            help_file: String::new(),
        })
    }
}

/// The namespace of the error olap4j and Mondrian answer a fault with.
const MONDRIAN_NAMESPACE: &str = "http://mondrian.sourceforge.net";
/// What such an error names as its source.
const MONDRIAN: &str = "Mondrian";

/// A SOAP fault carrying one XMLA `Error` in its detail: `code` says whose
/// fault it is, `error` says what went wrong.
///
/// # Errors
///
/// Returns an error when the detail cannot be built, which no text causes.
pub fn fault(code: FaultCode, error: XmlaError) -> Result<Fault> {
    Ok(Fault::new(code, error.description().to_owned())
        .with_actor(ACTOR)
        .with_detail(error.into_fragment()?))
}

/// What a response's `root` holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// A rowset: its columns and its rows.
    Rowset {
        /// The columns.
        rowset: Rowset,
        /// The rows, a record column of the rowset's field.
        rows: Serie,
    },
    /// Nothing: a command that answers no rows.
    Empty,
    /// A multidimensional dataset, kept as the natural XML value it is.
    Dataset(Fragment),
}

/// One response: the method it answers, its header blocks, and its answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    header: Vec<Fragment>,
    method: Method,
    answer: Answer,
}

impl Response {
    /// The header blocks.
    #[must_use]
    pub fn header(&self) -> &[Fragment] {
        &self.header
    }

    /// The method answered.
    #[must_use]
    pub const fn method(&self) -> Method {
        self.method
    }

    /// The answer.
    #[must_use]
    pub const fn answer(&self) -> &Answer {
        &self.answer
    }

    /// The rows, when the answer is a rowset.
    #[must_use]
    pub const fn rows(&self) -> Option<&Serie> {
        match &self.answer {
            Answer::Rowset { rows, .. } => Some(rows),
            Answer::Empty | Answer::Dataset(_) => None,
        }
    }

    /// The rowset's columns, when the answer is one.
    #[must_use]
    pub const fn rowset(&self) -> Option<&Rowset> {
        match &self.answer {
            Answer::Rowset { rowset, .. } => Some(rowset),
            Answer::Empty | Answer::Dataset(_) => None,
        }
    }

    /// Read a response out of a message.
    ///
    /// `field` is what a rowset's rows come back as, whatever schema the
    /// rowset carries - what a declared field means to every medium, and
    /// what types a rowset written without its schema (`Content` of `Data`).
    /// Without one, a rowset states its own columns and a schemaless one is
    /// refused.
    ///
    /// # Errors
    ///
    /// Returns the fault as an error when the body is one, and a refusal when
    /// the body is not a `DiscoverResponse` or `ExecuteResponse` holding one
    /// `return` and one `root`.
    pub fn from_envelope(envelope: &Envelope, field: Option<&Field>) -> Result<Self> {
        Self::from_envelope_with(
            envelope,
            field,
            crate::ArrowCastOptions::default().with_safe(false),
        )
    }

    /// [`Self::from_envelope`] under `cast`, which is how a rowset whose
    /// document states other columns lands on the declared field: strictly,
    /// or nulling what the field cannot hold when `cast` is safe
    /// ([`Rowset::read_root_with`]).
    ///
    /// # Errors
    ///
    /// As [`Self::from_envelope`].
    pub fn from_envelope_with(
        envelope: &Envelope,
        field: Option<&Field>,
        cast: crate::ArrowCastOptions,
    ) -> Result<Self> {
        let payload = match envelope.body() {
            Body::Payload(fragment) => fragment,
            Body::Fault(fault) => {
                let errors = XmlaError::from_fault(fault);
                let detail = errors
                    .first()
                    .map(|error| format!(" ({} {})", error.code(), error.description()))
                    .unwrap_or_default();
                return Err(invalid(format_smolstr!("{fault}{detail}")));
            }
        };
        let element = payload.element();
        let method = if element.is_in(NAMESPACE, Method::Discover.response_name()) {
            Method::Discover
        } else if element.is_in(NAMESPACE, Method::Execute.response_name()) {
            Method::Execute
        } else {
            return Err(invalid(format_smolstr!(
                "expected `DiscoverResponse` or `ExecuteResponse` in {NAMESPACE:?}, got `{}`",
                element.name()
            )));
        };
        let returned = element.one_child_in(NAMESPACE, "return")?;
        // One `root`, whichever namespace names the result it holds: the
        // rowset's, the empty one, the dataset's, or none at all.
        let mut roots = returned
            .children()
            .filter(|child| child.local_name() == "root");
        let root = roots
            .next()
            .ok_or_else(|| invalid("the response's `return` holds no `root`"))?;
        if roots.next().is_some() {
            return Err(invalid(
                "the response's `return` holds several `root` elements; a response carries one result",
            ));
        }
        let answer = match root.namespace() {
            Some(ROWSET_NAMESPACE) | None => {
                let (rowset, rows) = Rowset::read_root_with(&root, field, cast)?;
                Answer::Rowset { rowset, rows }
            }
            Some(EMPTY_NAMESPACE) => Answer::Empty,
            Some(MDDATASET_NAMESPACE) => Answer::Dataset(Fragment::from_element(&root)),
            Some(other) => {
                return Err(invalid(format_smolstr!(
                    "the response's `root` is in {other:?}, which names no result this crate reads"
                )));
            }
        };
        Ok(Self {
            header: envelope.header().to_vec(),
            method,
            answer,
        })
    }

    /// Read a response out of a message's bytes.
    ///
    /// # Errors
    ///
    /// Returns the envelope's refusal or [`Self::from_envelope`]'s.
    pub fn from_bytes(input: &[u8], field: Option<&Field>) -> Result<Self> {
        Self::from_envelope(&Envelope::from_bytes(input)?, field)
    }
}

/// Write a rowset response: the envelope with `header`, the response element
/// of `method`, its `return` and `root`, the schema and the rows `content`
/// asks for - each batch as it is pulled - and every closing tag.
///
/// # Errors
///
/// Returns the sink's failure, a batch's, or a cell with no XML spelling.
pub fn write_rowset<W: Write>(
    writer: W,
    header: &[Fragment],
    method: Method,
    rowset: &Rowset,
    batches: impl IntoIterator<Item = crate::arrow::Result<Serie>>,
    content: Content,
) -> Result<W> {
    let mut envelope = EnvelopeWriter::begin(writer, header)?;
    open_response(envelope.body(), method)?;
    rowset.write_root(
        envelope.body(),
        batches,
        content.has_schema(),
        content.has_data(),
    )?;
    close_response(envelope.body(), method)?;
    envelope.finish()
}

/// Write a rowset response as a provider streams one: like
/// [`write_rowset`], except that a batch failing after the answer has begun -
/// a batch its source refuses, a cell with no XML spelling - is reported
/// inside the `root` - `<Messages><Error .../></Messages>` in the exception
/// namespace, the shape the reference providers report an error with once
/// the response has started - and the document is closed, so the client
/// reads a complete answer naming the failure rather than a cut stream. A
/// row is written whole or not at all, so the document stays well formed at
/// the row before the failure. The error is handed back beside the sink.
///
/// # Errors
///
/// Returns the sink's failure, or the envelope's refusal of `header`: two
/// blocks of one name, or a block the XML writer cannot spell.
pub fn write_rowset_reporting<W: Write>(
    writer: W,
    header: &[Fragment],
    method: Method,
    rowset: &Rowset,
    batches: impl IntoIterator<Item = crate::arrow::Result<Serie>>,
    content: Content,
) -> Result<(W, Option<XmlaError>)> {
    let mut envelope = EnvelopeWriter::begin(writer, header)?;
    open_response(envelope.body(), method)?;
    let failed = rowset.write_root_reporting(
        envelope.body(),
        batches,
        content.has_schema(),
        content.has_data(),
    )?;
    close_response(envelope.body(), method)?;
    Ok((envelope.finish()?, failed))
}

/// Write the response to a command that answers nothing: a `root` in the
/// empty namespace.
///
/// # Errors
///
/// Returns the sink's failure, or the envelope's refusal of `header`: two
/// blocks of one name, or a block the XML writer cannot spell.
pub fn write_empty<W: Write>(writer: W, header: &[Fragment], method: Method) -> Result<W> {
    let mut envelope = EnvelopeWriter::begin(writer, header)?;
    open_response(envelope.body(), method)?;
    write!(
        envelope.body(),
        "<root xmlns=\"{EMPTY_NAMESPACE}\" xmlns:xsi=\"{XSI_NAMESPACE}\" \
         xmlns:xsd=\"{XSD_NAMESPACE}\" xmlns:EX=\"{EXCEPTION_NAMESPACE}\"/>"
    )?;
    close_response(envelope.body(), method)?;
    envelope.finish()
}

/// Write a fault as the whole message.
///
/// # Errors
///
/// Returns the sink's failure, or the XML writer's refusal of a fault whose
/// text or detail it cannot spell - a control character XML 1.0 has no
/// escape for.
pub fn write_fault<W: Write>(writer: W, fault: &Fault) -> Result<W> {
    EnvelopeWriter::begin(writer, &[])?.fault(fault)
}

fn open_response<W: Write>(writer: &mut W, method: Method) -> Result<()> {
    write!(
        writer,
        "<{} xmlns=\"{NAMESPACE}\"><return>",
        method.response_name()
    )?;
    Ok(())
}

fn close_response<W: Write>(writer: &mut W, method: Method) -> Result<()> {
    write!(writer, "</return></{}>", method.response_name())?;
    Ok(())
}

pub(crate) fn invalid(reason: impl Into<SmolStr>) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$.xmla"),
        reason: reason.into(),
    }
}
