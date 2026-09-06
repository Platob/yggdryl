//! Rows streamed out of one XML document.
//!
//! The mapping from an element to a value is the text codec's, so a row read
//! here and a document read through [`crate::text::xml`] agree by construction.
//! What this module owns is the framing above it: which elements are rows, and
//! how one row at a time leaves a stream that is never held whole.

use std::io::{BufReader, Read};

use quick_xml::XmlVersion;
use quick_xml::events::Event;
use smol_str::SmolStr;

use crate::text::Limits;
use crate::text::xml::TEXT_KEY;
use crate::text::xml::parser::{Element, State, name_text, position, protocol, resolve};
use crate::types::Nested;
use crate::{Result, Scalar};

use super::options::XmlOptions;

/// How many bytes one refill pulls from the transport.
pub(crate) const FETCH_BYTE_SIZE: usize = crate::DEFAULT_FETCH_BYTE_SIZE;

/// One document's row elements, decoded one at a time.
///
/// Only the row being built is held, so a document larger than memory streams
/// exactly as an Avro container's blocks do.
pub(crate) struct Rows<R: Read> {
    reader: quick_xml::Reader<BufReader<R>>,
    buffer: Vec<u8>,
    limits: Limits,
    version: XmlVersion,
    root: Option<SmolStr>,
    row: Option<SmolStr>,
    /// Open elements of the row being built; empty between rows.
    stack: Vec<Element>,
    state: State,
    /// Open depth of an element under the document that is not a row.
    skipping: usize,
    done: bool,
}

impl<R: Read> Rows<R> {
    /// Read `source` as the rows one document holds.
    pub(crate) fn new(source: R, options: &XmlOptions) -> Self {
        Self::with_limits(source, options, Limits::default())
    }

    /// Read `source` under explicit per-row parser bounds.
    pub(crate) fn with_limits(source: R, options: &XmlOptions, limits: Limits) -> Self {
        let mut reader =
            quick_xml::Reader::from_reader(BufReader::with_capacity(FETCH_BYTE_SIZE, source));
        let config = reader.config_mut();
        config.check_end_names = true;
        config.allow_unmatched_ends = false;
        config.allow_dangling_amp = false;
        config.check_comments = true;
        config.expand_empty_elements = false;
        Self {
            reader,
            buffer: Vec::new(),
            limits,
            version: XmlVersion::Implicit1_0,
            root: None,
            row: options.row().map(SmolStr::new),
            stack: Vec::new(),
            state: State::new(limits),
            skipping: 0,
            done: false,
        }
    }

    /// Borrow the document element this stream read, once it has read one.
    pub(crate) fn root(&self) -> Option<&str> {
        self.root.as_deref()
    }

    /// Borrow the row element this stream reads, declared or discovered.
    pub(crate) fn row(&self) -> Option<&str> {
        self.row.as_deref()
    }

    /// Decode the next row, or answer that the document has no more.
    fn next_row(&mut self) -> Result<Option<Scalar>> {
        loop {
            if self.done {
                return Ok(None);
            }
            let position = position(&self.reader);
            self.buffer.clear();
            let event = self
                .reader
                .read_event_into(&mut self.buffer)
                .map_err(|error| protocol(error, position))?;
            match event {
                Event::Decl(declaration) => {
                    self.version = declaration
                        .xml_version()
                        .map_err(|error| protocol(error, position))?;
                }
                Event::Start(start) => {
                    if self.skipping > 0 {
                        self.skipping += 1;
                        continue;
                    }
                    let qname = start.name();
                    let name = name_text(qname.as_ref(), position)?;
                    if self.root.is_none() {
                        self.root = Some(SmolStr::new(name));
                        continue;
                    }
                    if self.stack.is_empty() {
                        if !claims(&mut self.row, name) {
                            self.skipping = 1;
                            continue;
                        }
                        self.state = State::new(self.limits);
                    }
                    self.state.enter(self.stack.len(), position)?;
                    self.stack.push(Element::open(
                        &start,
                        position,
                        self.version,
                        &mut self.state,
                    )?);
                }
                Event::Empty(start) => {
                    if self.skipping > 0 {
                        continue;
                    }
                    let qname = start.name();
                    let name = name_text(qname.as_ref(), position)?;
                    if self.root.is_none() {
                        // A document element holding nothing holds no rows.
                        self.root = Some(SmolStr::new(name));
                        self.done = true;
                        return Ok(None);
                    }
                    if self.stack.is_empty() {
                        if !claims(&mut self.row, name) {
                            continue;
                        }
                        self.state = State::new(self.limits);
                    }
                    self.state.enter(self.stack.len(), position)?;
                    let element = Element::open(&start, position, self.version, &mut self.state)?;
                    if let Some(row) = self.place(element)? {
                        return Ok(Some(row));
                    }
                }
                Event::End(_) => {
                    if self.skipping > 0 {
                        self.skipping -= 1;
                        continue;
                    }
                    let Some(element) = self.stack.pop() else {
                        // The document element closed: everything after it is
                        // trailing markup, which carries no rows.
                        self.done = true;
                        return Ok(None);
                    };
                    if let Some(row) = self.place(element)? {
                        return Ok(Some(row));
                    }
                }
                Event::Text(text) => {
                    if let Some(element) = self.stack.last_mut() {
                        let text = text
                            .xml_content(self.version)
                            .map_err(|error| protocol(error, position))?;
                        element.push_text(&text, &mut self.state)?;
                    }
                }
                Event::CData(section) => {
                    if let Some(element) = self.stack.last_mut() {
                        let text = section
                            .decode()
                            .map_err(|error| protocol(error, position))?;
                        element.literal = true;
                        element.push_text(&text, &mut self.state)?;
                    }
                }
                Event::GeneralRef(reference) => {
                    if let Some(element) = self.stack.last_mut() {
                        let reference = reference
                            .decode()
                            .map_err(|error| protocol(error, position))?;
                        let resolved = resolve(&reference, position)?;
                        element.push_text(&resolved, &mut self.state)?;
                    }
                }
                // Annotations about the document rather than content in it.
                Event::Comment(_) | Event::PI(_) | Event::DocType(_) => {}
                Event::Eof => {
                    self.done = true;
                    return Ok(None);
                }
            }
        }
    }

    /// Give one closed element to its parent, or answer it as a finished row.
    fn place(&mut self, element: Element) -> Result<Option<Scalar>> {
        let (name, value) = element.close()?;
        match self.stack.last_mut() {
            Some(parent) => {
                parent.push_child(name, value);
                Ok(None)
            }
            None => row_record(value).map(Some),
        }
    }
}

impl<R: Read> Iterator for Rows<R> {
    type Item = Result<Scalar>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_row() {
            Ok(row) => row.map(Ok),
            Err(error) => {
                // A malformed document stays malformed: the stream is fused so
                // a caller that keeps pulling gets the end, not a second error.
                self.done = true;
                Some(Err(error))
            }
        }
    }
}

/// Return whether `name` is the row element, adopting it when none is set.
///
/// An undeclared row element is whichever element the document element opens
/// with, so a document written elsewhere reads without being described first.
fn claims(row: &mut Option<SmolStr>, name: &str) -> bool {
    match row {
        Some(row) => row == name,
        None => {
            *row = Some(SmolStr::new(name));
            true
        }
    }
}

/// Restate one row element's value as the record a row is.
///
/// An element holding child elements is already a record. One holding nothing
/// is a row whose columns are all absent, and one holding only text is a row
/// with the single [`TEXT_KEY`] column that text is - the same name the codec
/// gives an element's own character data anywhere else.
pub(super) fn row_record(value: Scalar) -> Result<Scalar> {
    match value {
        Scalar::Nested(Nested::Record(_)) => Ok(value),
        Scalar::Null => Scalar::from_record(Vec::<(SmolStr, Scalar)>::new()),
        leaf => Scalar::from_record([(SmolStr::new_static(TEXT_KEY), leaf)]),
    }
}

/// Read the document and row element names one stream already uses.
///
/// The scan stops at the first row, so this costs the head of the document
/// rather than the whole of it.
pub(crate) fn read_names<R: Read>(
    source: R,
    options: &XmlOptions,
) -> Result<(Option<SmolStr>, Option<SmolStr>)> {
    let mut rows = Rows::new(source, options);
    rows.next().transpose()?;
    Ok((rows.root().map(SmolStr::new), rows.row().map(SmolStr::new)))
}
