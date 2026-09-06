//! Where every row of one XML document begins and ends.
//!
//! An XML document carries no offset table of its own, so random access reads
//! one: a scan that decodes no value, only the byte span each row element
//! occupies. The index is what makes a row addressable - `pread` for a read,
//! and one patch bounded by the row's own bytes for a write - and it is
//! maintained across those writes rather than rebuilt, so a run of positional
//! updates costs one scan rather than one per update.

use std::io::Read;

use quick_xml::events::Event;
use smol_str::SmolStr;

use crate::text::xml::FORMAT;
use crate::text::xml::parser::{name_text, position, protocol};
use crate::{Codec, Error, IOBase, Result};

use super::options::XmlOptions;
use super::reader::FETCH_BYTE_SIZE;

/// One row element's byte span in the document that holds it.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RowSpan {
    /// The offset the row's start tag begins at.
    pub start: u64,
    /// The offset just past the row's end tag.
    pub end: u64,
}

impl RowSpan {
    /// Return how many bytes the row occupies.
    #[must_use]
    pub const fn byte_size(self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    /// Move this span by `delta` bytes.
    const fn shifted(self, delta: i64) -> Self {
        Self {
            start: shift(self.start, delta),
            end: shift(self.end, delta),
        }
    }
}

const fn shift(offset: u64, delta: i64) -> u64 {
    if delta < 0 {
        offset.saturating_sub(delta.unsigned_abs())
    } else {
        offset.saturating_add(delta.unsigned_abs())
    }
}

/// The byte span of every row one XML document holds.
///
/// This describes the document's own bytes, so it exists only for a handle
/// that stores them uncompressed; a content coding is the handle's business
/// and has to be removed before a row has an address.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RowIndex {
    root: Option<SmolStr>,
    row: Option<SmolStr>,
    spans: Vec<RowSpan>,
    /// Where the document element's end tag begins.
    content_end: u64,
    /// Whether the document element is written `<root/>` and so has no content.
    self_closed: bool,
    /// The document's byte size when this index was read.
    size: u64,
}

impl RowIndex {
    /// The index of a document that holds nothing yet.
    fn empty() -> Self {
        Self {
            root: None,
            row: None,
            spans: Vec::new(),
            content_end: 0,
            self_closed: false,
            size: 0,
        }
    }

    /// Borrow the document element's name, when the document has one.
    #[must_use]
    pub fn root(&self) -> Option<&str> {
        self.root.as_deref()
    }

    /// Borrow the row element's name, when the document holds a row.
    #[must_use]
    pub fn row(&self) -> Option<&str> {
        self.row.as_deref()
    }

    /// Borrow every row span in document order.
    #[must_use]
    pub fn spans(&self) -> &[RowSpan] {
        &self.spans
    }

    /// Return one row's span.
    #[must_use]
    pub fn get(&self, row: u64) -> Option<RowSpan> {
        usize::try_from(row)
            .ok()
            .and_then(|row| self.spans.get(row))
            .copied()
    }

    /// Return how many rows the document holds.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.spans.len() as u64
    }

    /// Return whether the document holds no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// Return where the document element's end tag begins.
    ///
    /// This is where an append writes, so adding rows rewrites the end tag
    /// rather than the document.
    #[must_use]
    pub const fn content_end(&self) -> u64 {
        self.content_end
    }

    /// Return whether the document element carries no content at all.
    #[must_use]
    pub const fn is_self_closed(&self) -> bool {
        self.self_closed
    }

    /// Return the document's byte size when this index was read.
    #[must_use]
    pub const fn byte_size(&self) -> u64 {
        self.size
    }

    /// Require one row to exist, naming what is there when it does not.
    pub(crate) fn require(&self, row: u64) -> Result<RowSpan> {
        self.get(row).ok_or_else(|| Error::InvalidRecord {
            path: smol_str::format_smolstr!("$[{row}]"),
            reason: crate::text::expected_got(
                format_args!("a row below {}", self.len()),
                row,
            ),
        })
    }

    /// Restate the index after a document element was written around nothing.
    pub(crate) fn opened(&mut self, root: &str, content_end: u64, size: u64) {
        self.root = Some(SmolStr::new(root));
        self.self_closed = false;
        self.spans.clear();
        self.content_end = content_end;
        self.size = size;
    }

    /// Restate the index after rows replaced rows at `row`.
    pub(crate) fn spliced<I>(&mut self, row: usize, removed: usize, spans: I, delta: i64)
    where
        I: IntoIterator<Item = RowSpan>,
    {
        let row = row.min(self.spans.len());
        let removed = removed.min(self.spans.len() - row);
        let added: Vec<RowSpan> = spans.into_iter().collect();
        let follows = row + added.len();
        self.spans.splice(row..row + removed, added);
        for span in self.spans.iter_mut().skip(follows) {
            *span = span.shifted(delta);
        }
        self.content_end = shift(self.content_end, delta);
        self.size = shift(self.size, delta);
    }

    /// Adopt the row element name a write settled on.
    pub(crate) fn set_row(&mut self, row: &str) {
        if self.row.is_none() {
            self.row = Some(SmolStr::new(row));
        }
    }
}

/// Read where every row of `handle` lives.
///
/// # Errors
///
/// Returns an error when the handle declares a content coding, because the
/// spans address stored bytes, and any read or parse failure.
pub(crate) fn read_row_index<H: IOBase + ?Sized>(
    handle: &H,
    options: &XmlOptions,
) -> Result<RowIndex> {
    require_positional(handle)?;
    if handle.is_empty() {
        return Ok(RowIndex::empty());
    }
    let mut index = scan(handle.pstream_bytes(0, FETCH_BYTE_SIZE)?, options)?;
    index.size = handle.size();
    Ok(index)
}

/// Refuse a handle whose stored bytes are not the document's own.
pub(crate) fn require_positional<H: IOBase + ?Sized>(handle: &H) -> Result<()> {
    if handle.codec() == Codec::Identity {
        return Ok(());
    }
    Err(Error::InvalidRecord {
        path: SmolStr::new_static("$.encoding"),
        reason: crate::text::expected_got(
            "stored XML bytes to address a row in",
            format_args!("a {} content coding, which has no row offsets", handle.codec()),
        ),
    })
}

/// Scan one document for the byte span of every row it holds.
pub(crate) fn scan<R: Read>(source: R, options: &XmlOptions) -> Result<RowIndex> {
    let mut reader = quick_xml::Reader::from_reader(std::io::BufReader::with_capacity(
        FETCH_BYTE_SIZE,
        source,
    ));
    let config = reader.config_mut();
    config.check_end_names = true;
    config.allow_unmatched_ends = false;
    config.allow_dangling_amp = false;
    config.expand_empty_elements = false;

    let mut index = RowIndex::empty();
    index.row = options.row().map(SmolStr::new);
    let mut buffer = Vec::new();
    let mut depth = 0_usize;
    let mut open: Option<u64> = None;

    loop {
        let before = position(&reader) as u64;
        buffer.clear();
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| protocol(error, before as usize))?;
        match event {
            Event::Start(start) => {
                let qname = start.name();
                let name = name_text(qname.as_ref(), before as usize)?;
                if depth == 0 {
                    index.root = Some(SmolStr::new(name));
                    depth = 1;
                    continue;
                }
                if depth == 1 && open.is_none() && claims(&mut index.row, name) {
                    open = Some(before);
                }
                depth += 1;
            }
            Event::Empty(start) => {
                let qname = start.name();
                let name = name_text(qname.as_ref(), before as usize)?;
                if depth == 0 {
                    index.root = Some(SmolStr::new(name));
                    index.content_end = before;
                    index.self_closed = true;
                    break;
                }
                if depth == 1 && open.is_none() && claims(&mut index.row, name) {
                    index.spans.push(RowSpan {
                        start: before,
                        end: position(&reader) as u64,
                    });
                }
            }
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 1 {
                    if let Some(start) = open.take() {
                        index.spans.push(RowSpan {
                            start,
                            end: position(&reader) as u64,
                        });
                    }
                } else if depth == 0 {
                    index.content_end = before;
                    break;
                }
            }
            Event::Eof => {
                if index.root.is_some() {
                    return Err(Error::Codec {
                        format: FORMAT,
                        position: before as usize,
                        reason: "expected the document element to be closed".into(),
                    });
                }
                break;
            }
            _ => {}
        }
    }
    Ok(index)
}

/// Return whether `name` is the row element, adopting it when none is set.
fn claims(row: &mut Option<SmolStr>, name: &str) -> bool {
    match row {
        Some(row) => row == name,
        None => {
            *row = Some(SmolStr::new(name));
            true
        }
    }
}
