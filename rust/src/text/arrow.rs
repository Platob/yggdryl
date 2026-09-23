//! `text/plain` rows through the shared Scalar/Arrow record boundary.

use std::borrow::Cow;
use std::io::{BufRead, BufReader, Chain, Read, Write};
use std::ops::Range;
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_schema::{DataType as ArrowDataType, Schema};
use regex::bytes::CaptureLocations;
use regex_automata::dfa::{
    Automaton,
    dense::{Builder as DfaBuilder, DFA},
};
use regex_automata::{Input, nfa::thompson, util::syntax};
use smol_str::{SmolStr, format_smolstr};

use crate::arrow::BatchReader;
use crate::holder::Buffer;
use crate::holder::Holder;
use crate::media::IORecordOptions;
use crate::temporal as iso;
use crate::{Charset, Codec, DataType, Error, Result, Scalar, TimeUnit, Timezone, Url};
use crate::{Cursor, IOBase, charset};

use super::leading::LeadingFragment;
use super::line::LineSource;
use super::options::TextOptions;
use super::reader::Lines;
use super::{TextBytes, TextLine};

/// Decode one borrowed leaf into ordinary record batches.
pub(crate) fn read_arrow_reader(
    handle: &(impl IOBase + ?Sized),
    options: &TextOptions,
) -> Result<BatchReader> {
    options.require_framing_rowheader()?;
    options.source_field()?;
    // One ask for one fact: the handle owes its identifier, and the location
    // is that identifier narrowed - asking it for both would be two calls
    // over one answer, and the narrowing is the read's, once, not each row's.
    let source = handle.uri().map(LineSource::narrowed);
    // Read from the handle the caller gave, before `owned_handle` may answer
    // with a copy: a buffered copy of the bytes is not the object whose
    // modification time this is.
    let mtime = handle_mtime(handle, options);
    read_owned_arrow_reader_at(owned_handle(handle)?, source, mtime, options)
}

/// The handle's own modification time, asked for only when a column wants it.
fn handle_mtime(handle: &(impl IOBase + ?Sized), options: &TextOptions) -> Option<i64> {
    if !options.parse_mtime {
        return None;
    }
    handle.mtime()
}

/// Decode an owned leaf without retaining decoded pages in its caller.
pub(crate) fn read_owned_arrow_reader<H: IOBase + 'static>(
    handle: H,
    options: &TextOptions,
) -> Result<BatchReader> {
    // One ask for one fact: the handle owes its identifier, and the location
    // is that identifier narrowed - asking it for both would be two calls
    // over one answer, and the narrowing is the read's, once, not each row's.
    let source = handle.uri().map(LineSource::narrowed);
    let mtime = handle_mtime(&handle, options);
    read_owned_arrow_reader_at(handle, source, mtime, options)
}

fn read_owned_arrow_reader_at<H: IOBase + 'static>(
    handle: H,
    source: Option<LineSource>,
    mtime: Option<i64>,
    options: &TextOptions,
) -> Result<BatchReader> {
    options.require_framing_rowheader()?;
    // One ask of the handle answers both: the codings the transport peels
    // and the charset it decodes under, which the call-count pins hold to
    // the one `media_type` read the codings always took.
    let media_type = handle.media_type();
    let codings = media_type.encodings().to_vec();
    let charset = Charset::from_media_type(media_type);
    let bytes: Box<dyn Read + Send + 'static> = match handle.bound_location().cloned() {
        Some(bound) => Box::new(BoundReader::new(bound, codings, charset)),
        None => Box::new(NonemptySendDecodedReader::new(
            Box::new(Cursor::new(handle)),
            codings,
            charset,
        )),
    };

    let lines = text_lines(bytes, source, mtime, options)?;
    super::batch::into_arrow_reader(lines, options)
}

/// Build the one decode iterator over an already-opened stream.
///
/// The options are shared once, with the splitter and with every line it
/// cuts: a line resolves its readings under them on its first ask, so
/// nothing optional is paid for here.
fn text_lines(
    bytes: Box<dyn Read + Send + 'static>,
    source: Option<LineSource>,
    mtime: Option<i64>,
    options: &TextOptions,
) -> Result<TextLines> {
    // The plan's refusals - a rename naming no column, a lifted path with no
    // name - before a byte is read.
    options.line_plan()?;
    let options = Arc::new(options.clone());
    // The row errors want an owned location, which only a located read has.
    let url = source.as_ref().and_then(LineSource::url).cloned();
    let raw = RawRows::new(bytes, url, Arc::clone(&options));
    Ok(TextLines {
        raw,
        source,
        mtime,
        options,
    })
}

/// Decode one borrowed leaf into typed lines.
///
/// The one decode entry point. Every record method routes through it, and
/// nothing else parses a line.
///
/// The decode is not the query: this answers every line the object holds,
/// whatever the options' `where`, `select` and row bounds say. Those are
/// record clauses, applied once by the record surface -
/// [`IOMedia::read_arrow_reader`](crate::IOMedia::read_arrow_reader) - over
/// the rows the lines become, and they have to be, because a `where` may name
/// a column the `select` builds and no line states one. Reading lines is
/// therefore reading the resource, not reading the result; a caller who wants
/// the result reads rows.
///
/// # Errors
///
/// Returns the configuration's refusals - a framing mode with no header
/// pattern, a rename naming no column, a lifted path with no name - before a
/// byte is read. A clause naming no column is not one of them: nothing binds
/// it here, and the record surface refuses it by name.
pub fn read_text_lines(
    handle: &(impl IOBase + ?Sized),
    options: &TextOptions,
) -> Result<TextLines> {
    options.require_framing_rowheader()?;
    // One ask for one fact: the handle owes its identifier, and the location
    // is that identifier narrowed - asking it for both would be two calls
    // over one answer, and the narrowing is the read's, once, not each row's.
    let source = handle.uri().map(LineSource::narrowed);
    // Read from the handle the caller gave, before `owned_handle` may answer
    // with a copy: a buffered copy of the bytes is not the object whose
    // modification time this is.
    let mtime = handle_mtime(handle, options);
    read_owned_text_lines_at(owned_handle(handle)?, source, mtime, options)
}

/// The same decode over a handle the iterator owns.
fn read_owned_text_lines_at<H: IOBase + 'static>(
    handle: H,
    source: Option<LineSource>,
    mtime: Option<i64>,
    options: &TextOptions,
) -> Result<TextLines> {
    options.require_framing_rowheader()?;
    // One ask of the handle answers both: the codings the transport peels
    // and the charset it decodes under, which the call-count pins hold to
    // the one `media_type` read the codings always took.
    let media_type = handle.media_type();
    let codings = media_type.encodings().to_vec();
    let charset = Charset::from_media_type(media_type);
    let bytes: Box<dyn Read + Send + 'static> = match handle.bound_location().cloned() {
        Some(bound) => Box::new(BoundReader::new(bound, codings, charset)),
        None => Box::new(NonemptySendDecodedReader::new(
            Box::new(Cursor::new(handle)),
            codings,
            charset,
        )),
    };
    text_lines(bytes, source, mtime, options)
}

/// Count emitted records without materializing rows or Arrow arrays.
pub(crate) fn row_size(handle: &(impl IOBase + ?Sized), options: &TextOptions) -> Result<u64> {
    options.require_framing_rowheader()?;
    // Refused here as a read refuses it, so a count never answers for a
    // configuration no read could answer.
    options.require_retained_body()?;
    let mut counting = options.clone();
    counting.start_rownum = None;
    counting.parse_mtime = false;
    counting.set_lstrip::<[&str; 0], &str>([])?;
    counting.set_rstrip::<[&str; 0], &str>([])?;
    // A count keeps no body - unless adjacent duplicates are dropped, where
    // the digest that drops them reads the retained body past the header
    // exactly as the reading path does, so the two paths count alike.
    if !options.dedup_adjacent {
        counting.set_max_record_byte_size(Some(0));
    }
    // One ask of the handle answers both: the codings the transport peels
    // and the charset it decodes under, which the call-count pins hold to
    // the one `media_type` read the codings always took.
    let media_type = handle.media_type();
    let codings = media_type.encodings().to_vec();
    let charset = Charset::from_media_type(media_type);
    let raw: Box<dyn Read + '_> =
        Box::new(handle.pstream_bytes(0, crate::DEFAULT_FETCH_BYTE_SIZE)?);
    let source = NonemptyDecodedReader::new(raw, codings, charset);
    let records = RawRows::counting(source, handle.url().cloned(), Arc::new(counting));
    let mut rows = 0_u64;
    for row in records {
        row?;
        rows = rows.checked_add(1).ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("logical row count exceeds u64::MAX"),
        })?;
    }
    Ok(rows)
}

/// Return an owned view for a reader that must outlive this borrow.
fn owned_handle(handle: &(impl IOBase + ?Sized)) -> Result<Holder> {
    if let Some(bound) = handle.bound_location() {
        let mut file = crate::fs::FsFile::new(bound.clone());
        file.set_media_type(handle.media_type().clone());
        return Ok(Holder::FsFile(file));
    }
    if let Some(parent) = handle.parent() {
        if let Some(name) = handle.uri().and_then(crate::Uri::file_name) {
            let mut child = parent.child_by_path(name)?;
            child.set_media_type(handle.media_type().clone());
            return Ok(child);
        }
    }
    let mut buffer = Buffer::new();
    handle.copy_into(&mut buffer)?;
    Ok(Holder::buffer(buffer))
}

/// Buffer one transport at the fetch window every decoded read pulls through.
///
/// A decoder asks its source for its own internal window - 32 KiB for gzip -
/// and on a remote store each of those asks is a round trip. This is the only
/// place the text reader touches the transport, so it is the one place that
/// has to hold a window big enough to make a scan cost requests proportional
/// to the object's size rather than to the decoder's appetite.
fn fetched<R: Read>(source: R) -> BufReader<R> {
    BufReader::with_capacity(crate::DEFAULT_FETCH_BYTE_SIZE, source)
}

/// Whether a declared charset is laid over the transport at all.
///
/// UTF-8 and US-ASCII never are: the line layer already reads both by rule
/// one - every valid UTF-8 run kept, every stray byte as Windows-1252, where
/// the line is made - and writes UTF-8 as it is, so under either the
/// transport is the object `main` builds and the writer the bytes it renders.
const fn transports(charset: Charset) -> bool {
    !matches!(charset, Charset::Utf8 | Charset::Ascii)
}

/// The declared charset, laid over the coded stream.
///
/// Coding first, charset second - the order `text::io::Plan` composes in,
/// and the only one that can: a coding wraps bytes, and a charset spells
/// text in them. Everything above this - the line splitter, the row header,
/// the strips, adjacent deduplication, the entries - reads the
/// declared text, and `TextLine::from_bytes` finds every line text as read.
/// The stream transcribes and never refuses, as the line never refuses: a
/// byte the charset leaves unassigned reads as its C1 control, a lone
/// surrogate as `U+FFFD`, and a sequence the source cuts short at its very
/// end as one `U+FFFD` per sequence left.
///
/// The one mark taken off is the declared form's own: `FF FE` under
/// `utf-16le` comes off, the declaration winning over the mark, and every
/// other mark is data, replayed with the rest - `FE FF` under `utf-16le` is
/// the code unit it is. The structured plan strips whatever mark it finds,
/// because a parser would refuse `U+FEFF`; the record reader does not,
/// because a line refuses nothing and a mark for another form is a fact of
/// the wire. Nor is a mark looked for under UTF-8, since UTF-8 is never
/// wrapped and the line layer reads the bytes as they are: `EF BB BF` under
/// `charset=utf-8` is the first three bytes of the first line exactly as it
/// is undeclared.
fn declared<R: Read>(
    charset: Charset,
    mut coded: R,
) -> std::io::Result<charset::Reader<Chain<std::io::Cursor<Vec<u8>>, R>>> {
    let mut replayed = Vec::new();
    if let Some(mark) = charset.bom() {
        let mut head = [0_u8; charset::MARK_LEN];
        let filled = fill(&mut coded, &mut head[..mark.len()])?;
        let skipped = if head[..filled].starts_with(mark) {
            mark.len()
        } else {
            0
        };
        replayed.extend_from_slice(&head[skipped..filled]);
    }
    Ok(charset::Reader::new(
        charset.transcriber(),
        std::io::Cursor::new(replayed).chain(coded),
    ))
}

/// Read until `target` is full or the source ends, answering what was filled.
///
/// `Read::read` may answer short for reasons of its own, so a mark split
/// across two reads would otherwise go unrecognized.
fn fill(source: &mut impl Read, target: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < target.len() {
        let read = source.read(&mut target[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}

/// One lazily opened filesystem stream retained for the complete decode.
struct BoundReader {
    bound: Option<crate::fs::BoundLocation>,
    codings: Vec<crate::MimeType>,
    charset: Charset,
    reader: Option<Box<dyn Read + Send>>,
    done: bool,
}

impl BoundReader {
    fn new(
        bound: crate::fs::BoundLocation,
        codings: Vec<crate::MimeType>,
        charset: Charset,
    ) -> Self {
        Self {
            bound: Some(bound),
            codings,
            charset,
            reader: None,
            done: false,
        }
    }

    fn initialize(&mut self) -> std::io::Result<bool> {
        let Some(bound) = self.bound.take() else {
            self.done = true;
            return Err(std::io::Error::other(
                "text filesystem stream lost its binding",
            ));
        };
        let stream = match crate::fs::FsFile::new(bound).open_input_stream() {
            Ok(stream) => stream,
            Err(error) if error.is_absent() => {
                self.done = true;
                return Ok(false);
            }
            Err(error) => {
                self.done = true;
                return Err(std::io::Error::other(error));
            }
        };
        let mut stream = fetched(BoundStream::new(stream));
        // One fetch answers the emptiness question and is the first window the
        // decoder reads from, so an empty object costs no extra request and a
        // present one is not probed a byte at a time.
        if stream.fill_buf()?.is_empty() {
            self.done = true;
            return Ok(false);
        }
        let mut reader: Box<dyn Read + Send> = Box::new(stream);
        for coding in self.codings.iter().rev() {
            reader = Codec::from_mime_type(coding).reader_send(reader);
        }
        if transports(self.charset) {
            reader = Box::new(declared(self.charset, reader)?);
        }
        self.reader = Some(reader);
        Ok(true)
    }
}

impl Read for BoundReader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if bytes.is_empty() || self.done {
            return Ok(0);
        }
        if self.reader.is_none() && !self.initialize()? {
            return Ok(0);
        }
        let read = self
            .reader
            .as_mut()
            .map_or(Ok(0), |reader| reader.read(bytes))?;
        if read == 0 {
            self.done = true;
            self.reader = None;
        }
        Ok(read)
    }
}

/// An opened raw stream that closes when its decoder is dropped.
struct BoundStream {
    stream: Box<dyn crate::fs::ByteReader>,
}

impl BoundStream {
    fn new(stream: Box<dyn crate::fs::ByteReader>) -> Self {
        Self { stream }
    }
}

impl Read for BoundStream {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.stream.read(bytes).map_err(std::io::Error::other)
    }
}

impl Drop for BoundStream {
    fn drop(&mut self) {
        let _ = self.stream.close();
    }
}

/// The owned, thread-safe form of [`NonemptyDecodedReader`].
struct NonemptySendDecodedReader {
    source: Option<Box<dyn Read + Send>>,
    codings: Vec<crate::MimeType>,
    charset: Charset,
    reader: Option<Box<dyn Read + Send>>,
    done: bool,
}

impl NonemptySendDecodedReader {
    fn new(source: Box<dyn Read + Send>, codings: Vec<crate::MimeType>, charset: Charset) -> Self {
        Self {
            source: Some(source),
            codings,
            charset,
            reader: None,
            done: false,
        }
    }

    fn initialize(&mut self) -> std::io::Result<bool> {
        let Some(source) = self.source.take() else {
            self.done = true;
            return Ok(false);
        };
        let mut source = fetched(source);
        if source.fill_buf()?.is_empty() {
            self.done = true;
            return Ok(false);
        }
        let mut reader: Box<dyn Read + Send> = Box::new(source);
        for coding in self.codings.iter().rev() {
            reader = Codec::from_mime_type(coding).reader_send(reader);
        }
        if transports(self.charset) {
            reader = Box::new(declared(self.charset, reader)?);
        }
        self.reader = Some(reader);
        Ok(true)
    }
}

impl Read for NonemptySendDecodedReader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if bytes.is_empty() || self.done {
            return Ok(0);
        }
        if self.reader.is_none() && !self.initialize()? {
            return Ok(0);
        }
        let read = self
            .reader
            .as_mut()
            .map_or(Ok(0), |reader| reader.read(bytes))?;
        if read == 0 {
            self.done = true;
            self.reader = None;
        }
        Ok(read)
    }
}

/// A decoder that treats a raw empty stream as empty without constructing a
/// compression reader. This preserves missing-read semantics for row counts.
struct NonemptyDecodedReader<'source> {
    source: Option<Box<dyn Read + 'source>>,
    codings: Vec<crate::MimeType>,
    charset: Charset,
    reader: Option<Box<dyn Read + 'source>>,
    done: bool,
}

impl<'source> NonemptyDecodedReader<'source> {
    fn new(
        source: Box<dyn Read + 'source>,
        codings: Vec<crate::MimeType>,
        charset: Charset,
    ) -> Self {
        Self {
            source: Some(source),
            codings,
            charset,
            reader: None,
            done: false,
        }
    }

    fn initialize(&mut self) -> std::io::Result<bool> {
        let Some(source) = self.source.take() else {
            self.done = true;
            return Ok(false);
        };
        let mut source = fetched(source);
        if source.fill_buf()?.is_empty() {
            self.done = true;
            return Ok(false);
        }
        let mut reader: Box<dyn Read + 'source> = Box::new(source);
        for coding in self.codings.iter().rev() {
            reader = Codec::from_mime_type(coding).reader(reader);
        }
        if transports(self.charset) {
            reader = Box::new(declared(self.charset, reader)?);
        }
        self.reader = Some(reader);
        Ok(true)
    }
}

impl Read for NonemptyDecodedReader<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if bytes.is_empty() || self.done {
            return Ok(0);
        }
        if self.reader.is_none() && !self.initialize()? {
            return Ok(0);
        }
        let read = self
            .reader
            .as_mut()
            .map_or(Ok(0), |reader| reader.read(bytes))?;
        if read == 0 {
            self.done = true;
            self.reader = None;
        }
        Ok(read)
    }
}

/// One parsed physical line with still-textual named captures.
struct RawRow {
    index: u64,
    /// The retained body, as the range of the page it was read into.
    ///
    /// Ordinarily that page is the splitter's own window, shared with every
    /// other line cut from it, so a line costs one reference count rather
    /// than a copy of its bytes. A record that had to be assembled - one
    /// spanning two windows, one joining several physical lines - carries a
    /// page of its own instead.
    body: TextBytes,
    dropped_byte_size: Option<u64>,
    /// The row header as the cut matched it - where it ends and the captures
    /// it named - where the cut needed the match; the line matches it on
    /// its first ask otherwise.
    header: Option<(usize, Vec<Option<TextBytes>>)>,
}

/// Physical lines or framed records parsed against one precomputed schema.
struct RawRows<R> {
    lines: Lines<R>,
    header_dfa: Option<DFA<Vec<u32>>>,
    url: Option<Url>,
    options: Arc<TextOptions>,
    capture_values: bool,
    /// Whether the cut itself reads the row header: a framed read decides
    /// its records by it, a bounded one matches it over bytes the record
    /// may not retain, and adjacent deduplication digests what follows it.
    /// Otherwise the header is the line's to match, once, when asked.
    resolves_header: bool,
    /// Where the row header's groups land, reused across every line.
    ///
    /// Built from the expression on the first line that is scanned, so a
    /// read declaring no row header never builds one at all.
    locations: Option<CaptureLocations>,
    index: u64,
    active: Option<RawRecord>,
    done: bool,
    /// The digest of the body the previous row carried, when deduplicating.
    ///
    /// One `u128`, whatever the stream's length: adjacent deduplication is
    /// the whole rule, so nothing else has to be remembered. 128 bits because
    /// a day of capture is comfortably a billion rows and a 64-bit key would
    /// silently drop a real one about once per large capture.
    previous: Option<u128>,
}

/// The bytes of one line as they are held while it is being cut.
///
/// A line that arrived whole inside one window is a range of that window,
/// and the window is the page it stays a range of: the splitter seals it
/// before handing any of it out, so every cut below - the header off the
/// front, the strips off both edges, the byte limit off the tail - moves an
/// offset and copies nothing. A line becomes a vector of its own only where
/// it cannot be a range: one that spanned two windows, and one a header
/// matched in the middle of, where the bytes on either side are not
/// contiguous once the match is gone.
enum Held {
    Span {
        page: Arc<Vec<u8>>,
        range: Range<usize>,
    },
    Owned(Vec<u8>),
}

impl Held {
    /// The empty line, retaining no page.
    const fn new() -> Self {
        Self::Owned(Vec::new())
    }

    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Span { page, range } => &page[range.clone()],
            Self::Owned(bytes) => bytes,
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Span { range, .. } => range.end - range.start,
            Self::Owned(bytes) => bytes.len(),
        }
    }

    /// Keep only `start..end` of what this holds.
    fn narrow(&mut self, start: usize, end: usize) {
        match self {
            Self::Span { range, .. } => {
                range.end = range.start + end;
                range.start += start;
            }
            Self::Owned(bytes) => {
                if start > 0 {
                    bytes.copy_within(start..end, 0);
                }
                bytes.truncate(end - start);
            }
        }
    }

    /// Keep only the first `len` bytes.
    fn truncate(&mut self, len: usize) {
        if len < self.len() {
            self.narrow(0, len);
        }
    }

    /// Append `bytes`, taking a vector of this line's own to hold them.
    fn push(&mut self, bytes: &[u8]) {
        match self {
            Self::Owned(held) => held.extend_from_slice(bytes),
            Self::Span { .. } => {
                let mut held = Vec::with_capacity(self.len() + bytes.len());
                held.extend_from_slice(self.as_bytes());
                held.extend_from_slice(bytes);
                *self = Self::Owned(held);
            }
        }
    }

    /// One byte appended, for the terminator a framed record joins on.
    fn push_byte(&mut self, byte: u8) {
        self.push(&[byte]);
    }

    /// Take a vector of this line's own, releasing the page it was a range of.
    ///
    /// For a line the splitter has not finished: it is going to span windows,
    /// and a window it still names cannot be written over, so every refill
    /// the rest of the line needs would take a fresh one. One copy of what
    /// has been read so far buys the window back, and it is the copy the
    /// first append would have made anyway.
    fn detach(&mut self) {
        if matches!(self, Self::Span { .. }) {
            *self = Self::Owned(self.as_bytes().to_vec());
        }
    }

    /// `range` of what this holds, as a range of the same page.
    fn slice(&self, range: Range<usize>) -> Result<TextBytes> {
        match self {
            Self::Span { page, range: held } => {
                TextBytes::from_page(page, held.start + range.start, held.start + range.end)
            }
            Self::Owned(bytes) => TextBytes::from_bytes(&bytes[range]),
        }
    }

    /// What this holds, as the page a line is made on.
    fn into_text_bytes(self) -> Result<TextBytes> {
        match self {
            Self::Span { page, range } => TextBytes::from_page(&page, range.start, range.end),
            Self::Owned(bytes) => TextBytes::try_from(bytes),
        }
    }
}

/// One physical line after edge stripping, with the header the cut matched.
struct ParsedLine {
    index: u64,
    body: Body,
    header: Option<(usize, Vec<Option<TextBytes>>)>,
    matched: bool,
}

/// One physical line, possibly reduced after its header DFA proves no match.
struct PhysicalLine {
    bytes: Held,
    decoded_size: u64,
    header: ScannedHeader,
}

/// What the incremental row-header scan proved before the line ended.
enum ScannedHeader {
    /// The complete-line regex still decides the answer.
    Unresolved,
    /// No later byte can make this physical line match.
    Nonmatching,
    /// The header matched while its bytes were still retained: where the
    /// match ends and the captures it named.
    Matched {
        end: usize,
        captures: Vec<Option<TextBytes>>,
    },
}

/// One complete row-header match and the captures it cut from the line.
struct HeaderMatch {
    range: Range<usize>,
    captures: Vec<Option<TextBytes>>,
}

/// A bounded retained prefix plus its complete decoded byte length.
struct Body {
    bytes: Held,
    decoded_size: u64,
}

/// One logical record retained across physical input and Arrow batch pulls.
struct RawRecord {
    index: u64,
    body: Held,
    decoded_size: u64,
    header: Option<(usize, Vec<Option<TextBytes>>)>,
    /// Where the header ends in the body, zero where none matched: the
    /// bytes the limit does not count.
    header_end: usize,
}

fn header_dfa(source: &str) -> Option<DFA<Vec<u32>>> {
    DfaBuilder::new()
        .syntax(syntax::Config::new().utf8(false))
        .thompson(thompson::Config::new().utf8(false))
        .build(source)
        .ok()
}

/// The row header matched against the first `scan` bytes of `held`.
///
/// A capture comes back as the range of `held`'s own page that the match
/// named, never as a copy of the matched bytes: the line is a range of the
/// page the splitter read it into, so a run of the line is a range of the
/// same page. The one vector the captures need is the one built here.
fn header_match(
    options: &TextOptions,
    held: &Held,
    scan: usize,
    capture_values: bool,
    locations: &mut Option<CaptureLocations>,
) -> Result<Option<HeaderMatch>> {
    let Some(rowheader) = options.rowheader_regex() else {
        return Ok(None);
    };
    // Read into the locations the reader keeps rather than into a fresh
    // `Captures`: where the match lands is a vector as wide as the
    // expression's groups, and the expression is one per read.
    let locations = locations.get_or_insert_with(|| rowheader.capture_locations());
    if rowheader
        .captures_read(locations, &held.as_bytes()[..scan])
        .is_none()
    {
        return Ok(None);
    }
    let Some((start, end)) = locations.get(0) else {
        return Ok(None);
    };
    let range = start..end;
    if !capture_values {
        return Ok(Some(HeaderMatch {
            range,
            captures: Vec::new(),
        }));
    }
    let mut captures: Vec<Option<TextBytes>> = vec![None; options.capture_names().len()];
    for (target, capture_index) in captures.iter_mut().zip(
        rowheader
            .capture_names()
            .enumerate()
            .filter_map(|(index, name)| name.map(|_| index)),
    ) {
        let Some((start, end)) = locations.get(capture_index) else {
            continue;
        };
        *target = Some(held.slice(start..end)?);
    }
    Ok(Some(HeaderMatch { range, captures }))
}

impl<R: Read> RawRows<R> {
    fn new(source: R, url: Option<Url>, options: Arc<TextOptions>) -> Self {
        Self::with_capture_values(source, url, options, true)
    }

    fn counting(source: R, url: Option<Url>, options: Arc<TextOptions>) -> Self {
        Self::with_capture_values(source, url, options, false)
    }

    fn with_capture_values(
        source: R,
        url: Option<Url>,
        options: Arc<TextOptions>,
        capture_values: bool,
    ) -> Self {
        let header_dfa = options
            .max_record_byte_size()
            .and_then(|_| options.rowheader())
            .and_then(header_dfa);
        let resolves_header = options.rowheader().is_some()
            && (options.framing()
                || options.max_record_byte_size().is_some()
                || options.dedup_adjacent);
        Self {
            lines: Lines::new(source),
            header_dfa,
            url,
            options,
            capture_values,
            resolves_header,
            locations: None,
            index: 0,
            active: None,
            done: false,
            previous: None,
        }
    }

    fn parse_line(
        options: &TextOptions,
        line: PhysicalLine,
        index: u64,
        capture_values: bool,
        resolves_header: bool,
        locations: &mut Option<CaptureLocations>,
    ) -> Result<ParsedLine> {
        let PhysicalLine {
            mut bytes,
            decoded_size,
            header,
        } = line;
        // The header stays in the body: it is the line's to read, and the
        // cut matches it only where the cut itself depends on it, handing
        // the line the match so nothing matches it twice. The captures are
        // ranges of the same page the body is, so the strips and the limit
        // below move the body's ends and reach none of them.
        let (header, matched) = match header {
            ScannedHeader::Nonmatching => (None, false),
            ScannedHeader::Matched { end, captures } => (Some((end, captures)), true),
            ScannedHeader::Unresolved if resolves_header => {
                let scan = bytes.len();
                match header_match(options, &bytes, scan, capture_values, locations)? {
                    Some(found) => (Some((found.range.end, found.captures)), true),
                    None => (None, false),
                }
            }
            ScannedHeader::Unresolved => (None, false),
        };
        // A counting read wants the record boundaries and no captures: the
        // match answers where the header ends and an empty list, so the
        // limit and the dedup digest count past the header on both paths.
        let body_decoded_size = decoded_size;
        let body = &mut bytes;

        let mut start = 0;
        let mut end = body.len();
        // Each pattern strips from the edge the one before it left, so a
        // layered prefix comes off a layer at a time rather than through one
        // expression nobody can read.
        for lstrip in options.lstrip_regexes() {
            if let Some(found) = lstrip
                .find(&body.as_bytes()[start..end])
                .filter(|found| found.start() == 0)
            {
                start += found.end();
            }
        }
        for rstrip in options.rstrip_regexes() {
            if let Some(found) = rstrip
                .find_iter(&body.as_bytes()[start..end])
                .filter(|found| found.end() == end - start)
                .last()
            {
                end = start + found.start();
            }
        }
        let decoded_size = if options.rewrites_body() {
            u64::try_from(end - start).map_err(|_| Error::InvalidRecord {
                path: format_smolstr!("$[{index}].body"),
                reason: SmolStr::new_static("decoded text record exceeds u64::MAX bytes"),
            })?
        } else {
            body_decoded_size
        };
        // The header is always retained, and the limit bounds what follows
        // it: a record is known by its header, so a limit shorter than the
        // header still leaves the header - and its captures - whole. Where
        // the match ends is stated against the body the strips left.
        let header = header.map(|(matched_end, captures)| {
            (matched_end.saturating_sub(start).min(end - start), captures)
        });
        let header_end = header.as_ref().map_or(0, |(end, _)| *end);
        let retained =
            header_end + retained_size(options.max_record_byte_size(), 0, end - start - header_end);
        // The strips and the limit move the line's ends and nothing else:
        // where it is a range of the splitter's page they are two offsets,
        // and where it is a vector of its own they are moved within it.
        body.narrow(start, start + retained);
        Ok(ParsedLine {
            index,
            body: Body {
                bytes,
                decoded_size,
            },
            header,
            matched,
        })
    }

    fn next_line(&mut self) -> Option<Result<ParsedLine>> {
        // Borrowed, not cloned: the splitter, the automaton and the cursor
        // are fields of their own, so reading the configuration beside them
        // costs nothing where a shared handle would cost two atomics a line.
        let options: &TextOptions = &self.options;
        let index = self.index;
        let can_drain = options.max_record_byte_size().is_some() && !options.rewrites_body();
        let mut header = if can_drain && options.rowheader_regex().is_none() {
            ScannedHeader::Nonmatching
        } else {
            ScannedHeader::Unresolved
        };
        let mut state = if can_drain && matches!(header, ScannedHeader::Unresolved) {
            self.header_dfa
                .as_ref()
                .and_then(|dfa| dfa.start_state_forward(&Input::new(b"")).ok())
        } else {
            None
        };
        let mut dfa_matched = false;
        let retained = options
            .max_record_byte_size()
            .and_then(|size| usize::try_from(size).ok())
            .unwrap_or(usize::MAX);
        // What the line retains: the limit, and the header in front of it
        // once the scan has matched one.
        let mut budget = retained;
        // A line opens as the range of the window the splitter cut it from
        // and stays one unless it has to leave: `Held` takes a vector of its
        // own where a second window or a header in the middle forces one.
        let mut bytes = Held::new();
        let mut opened = false;
        let mut decoded_size = 0_u64;
        loop {
            let part = match self.lines.next_part(options.linesep())? {
                Ok(part) => part,
                Err(error) => {
                    self.done = true;
                    return Some(Err(error));
                }
            };
            let part_size = u64::try_from(part.len()).unwrap_or(u64::MAX);
            decoded_size = match decoded_size.checked_add(part_size) {
                Some(size) => size,
                None => {
                    self.done = true;
                    return Some(Err(Error::InvalidRecord {
                        path: SmolStr::new_static("$.body"),
                        reason: SmolStr::new_static(
                            "decoded physical text line exceeds u64::MAX bytes",
                        ),
                    }));
                }
            };
            let ends = part.end;
            if !matches!(header, ScannedHeader::Unresolved) {
                let available = budget.saturating_sub(bytes.len()).min(part.len());
                let range = part.range.start..part.range.start + available;
                if opened {
                    bytes.push(&self.lines.window()[range]);
                } else {
                    bytes = Held::Span {
                        page: Arc::clone(self.lines.page()),
                        range,
                    };
                }
            } else {
                let part_start = bytes.len();
                if opened {
                    bytes.push(&self.lines.window()[part.range.clone()]);
                } else {
                    bytes = Held::Span {
                        page: Arc::clone(self.lines.page()),
                        range: part.range.clone(),
                    };
                }
                if let (Some(dfa), Some(mut current)) = (&self.header_dfa, state) {
                    let window = self.lines.window();
                    for (offset, &byte) in window[part.range].iter().enumerate() {
                        current = dfa.next_state(current, byte);
                        if dfa.is_match_state(current) {
                            dfa_matched = true;
                            continue;
                        }
                        if dfa.is_dead_state(current) {
                            let scan_end = part_start + offset + 1;
                            if dfa_matched {
                                let found = match header_match(
                                    options,
                                    &bytes,
                                    scan_end,
                                    self.capture_values,
                                    &mut self.locations,
                                ) {
                                    Ok(found) => found,
                                    Err(error) => {
                                        self.done = true;
                                        return Some(Err(error));
                                    }
                                };
                                if let Some(found) = found {
                                    // The captures are ranges of the page the
                                    // header was scanned in, so the body is
                                    // bounded under them and they stay whole;
                                    // the header itself is retained, and the
                                    // limit bounds what follows it.
                                    budget = found.range.end.saturating_add(retained);
                                    bytes.truncate(budget);
                                    header = ScannedHeader::Matched {
                                        end: found.range.end,
                                        captures: found.captures,
                                    };
                                }
                            } else {
                                header = ScannedHeader::Nonmatching;
                                bytes.truncate(budget);
                            }
                            state = None;
                            break;
                        }
                        if dfa.is_quit_state(current) {
                            state = None;
                            break;
                        }
                    }
                    if matches!(header, ScannedHeader::Unresolved) && state.is_some() {
                        state = Some(current);
                    }
                }
            }
            opened = true;
            if ends {
                break;
            }
            bytes.detach();
        }
        let line = PhysicalLine {
            bytes,
            decoded_size,
            header,
        };
        let Some(next_index) = index.checked_add(1) else {
            self.done = true;
            return Some(Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: SmolStr::new_static("physical text line count exceeds u64::MAX"),
            }));
        };
        self.index = next_index;
        Some(Self::parse_line(
            options,
            line,
            index,
            self.capture_values,
            self.resolves_header,
            &mut self.locations,
        ))
    }

    fn next_framed(&mut self) -> Option<Result<RawRow>> {
        loop {
            let line = match self.next_line() {
                Some(Ok(line)) => line,
                Some(Err(error)) => return Some(Err(error)),
                None => {
                    self.done = true;
                    return self.active.take().and_then(RawRecord::finish);
                }
            };
            if line.matched {
                let next = RawRecord::new(line, self.options.max_record_byte_size());
                // A leading fragment of nothing but blank lines states no
                // record, so the one that matched opens the first row.
                if let Some(row) = self.active.replace(next).and_then(RawRecord::finish) {
                    return Some(row);
                }
                continue;
            }
            if let Some(record) = &mut self.active {
                if let Err(error) = record.append(line.body, self.options.max_record_byte_size()) {
                    self.done = true;
                    return Some(Err(error));
                }
                continue;
            }
            match self.options.leading_fragment() {
                LeadingFragment::Keep => {
                    self.active = Some(RawRecord::new(line, self.options.max_record_byte_size()));
                }
                LeadingFragment::Drop => {}
                LeadingFragment::Error => {
                    self.done = true;
                    return Some(Err(row_error(
                        line.index,
                        None,
                        self.url.as_ref(),
                        "body",
                        SmolStr::new_static(
                            "expected the leading physical line to match rowheader",
                        ),
                    )));
                }
            }
        }
    }
}

impl RawRecord {
    fn new(line: ParsedLine, limit: Option<u64>) -> Self {
        let ParsedLine {
            index,
            body: Body {
                mut bytes,
                decoded_size,
            },
            header,
            ..
        } = line;
        // The record opens on the line exactly as the line holds it: one
        // that stays a single physical line never appends, so it stays a
        // range of the splitter's page and a limit only moves its end. The
        // header is retained whole, and the limit counts from its end.
        let header_end = header.as_ref().map_or(0, |(end, _)| *end);
        bytes.truncate(header_end + retained_size(limit, 0, bytes.len() - header_end));
        Self {
            index,
            body: bytes,
            decoded_size,
            header,
            header_end,
        }
    }

    fn append(&mut self, body: Body, limit: Option<u64>) -> Result<()> {
        self.decoded_size = self
            .decoded_size
            .checked_add(1)
            .and_then(|size| size.checked_add(body.decoded_size))
            .ok_or_else(|| Error::InvalidRecord {
                path: format_smolstr!("$[{}].body", self.index),
                reason: SmolStr::new_static("decoded text record exceeds u64::MAX bytes"),
            })?;
        let separator = retained_size(limit, self.body.len() - self.header_end, 1);
        if separator == 1 {
            self.body.push_byte(b'\n');
        }
        let retained = retained_size(limit, self.body.len() - self.header_end, body.bytes.len());
        self.body.push(&body.bytes.as_bytes()[..retained]);
        Ok(())
    }

    /// The row this record states, or nothing where it stated no byte of
    /// its own.
    ///
    /// A blank line, and one the strips took whole, cut to nothing: it is a
    /// separator between records and not a record, so it is no row and no
    /// line. Decided on what the record cut, never on what the retained
    /// limit kept, so a count and a read drop the same lines - the counting
    /// pass keeps no body at all.
    fn finish(self) -> Option<Result<RawRow>> {
        if self.decoded_size == 0 {
            return None;
        }
        let retained = u64::try_from(self.body.len()).unwrap_or(u64::MAX);
        let dropped_byte_size = self
            .decoded_size
            .checked_sub(retained)
            .filter(|size| *size > 0);
        Some(self.body.into_text_bytes().map(|body| RawRow {
            index: self.index,
            body,
            dropped_byte_size,
            header: self.header,
        }))
    }
}

fn retained_size(limit: Option<u64>, current: usize, offered: usize) -> usize {
    let Some(limit) = limit else {
        return offered;
    };
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    limit.saturating_sub(current).min(offered)
}

impl<R: Read> Iterator for RawRows<R> {
    type Item = Result<RawRow>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        loop {
            let row = if self.options.framing() {
                self.next_framed()
            } else {
                match self.next_line()? {
                    Err(error) => Some(Err(error)),
                    // A physical line that cut to nothing is a separator and
                    // not a record: the next line is the next row, and the
                    // one that was blank keeps its place in the numbering
                    // because a row number is the line's own.
                    Ok(line) => {
                        match RawRecord::new(line, self.options.max_record_byte_size()).finish() {
                            Some(row) => Some(row),
                            None => continue,
                        }
                    }
                }
            };
            let row = row?;
            if !self.options.dedup_adjacent {
                return Some(row);
            }
            let Ok(row) = row else {
                return Some(row);
            };
            // The digest is of what follows the header: two republished
            // copies of one line differ in the clock their headers state
            // and in nothing the line says.
            let past_header = row.header.as_ref().map_or(0, |(end, _)| *end);
            let digest = crate::xxhash::xxh128(&row.body.as_bytes()[past_header..]);
            if self.previous == Some(digest) {
                continue;
            }
            self.previous = Some(digest);
            return Some(Ok(row));
        }
    }
}

/// Physical or framed rows decoded into typed line values.
///
/// The one decode path: every record method routes through this, and nothing
/// else parses a line. It yields every line it decodes - the options' `where`,
/// `select` and row bounds are the record surface's, as
/// [`read_text_lines`] states.
pub struct TextLines {
    raw: RawRows<Box<dyn Read + Send + 'static>>,
    /// What every line of this read was addressed by, narrowed once for the
    /// read rather than once per row and shared rather than rebuilt.
    source: Option<LineSource>,
    /// The handle's own modification time, the instant every line its
    /// header does not date shares.
    mtime: Option<i64>,
    /// The options every line of this read resolves its readings under,
    /// shared with the splitter rather than cloned per line.
    options: Arc<TextOptions>,
}

impl Iterator for TextLines {
    type Item = Result<TextLine>;

    fn next(&mut self) -> Option<Self::Item> {
        let raw = self.raw.next()?;
        Some(raw.and_then(|row| self.convert(row)))
    }
}

impl std::iter::FusedIterator for TextLines {}

impl TextLines {
    /// Turn one parsed row into the line every reading is resolved from.
    ///
    /// The line is made text here - the body decoded where it is not UTF-8 -
    /// and everything after this reads text. Everything before it read the
    /// bytes as they were, and the counts it took are counts of those bytes.
    /// Nothing else is read: the header is stated only where the cut
    /// matched it, and every other reading is the line's, on its first ask.
    fn convert(&self, row: RawRow) -> Result<TextLine> {
        let mut line =
            TextLine::from_cut(row.index, row.body, Arc::clone(&self.options), row.header)?;
        line.state_source(self.source.clone());
        line.set_handle_mtime(self.mtime);
        line.set_dropped_byte_size(row.dropped_byte_size);
        Ok(line)
    }
}

pub(crate) fn parse_capture(
    value: &str,
    dtype: &DataType,
    timezone: Option<&Timezone>,
) -> std::result::Result<Scalar, SmolStr> {
    let invalid = || {
        format_smolstr!(
            "expected a value of inferred datatype {dtype}, got {:?}",
            crate::text::elide_to(value, crate::text::ERROR_TEXT_LIMIT)
        )
    };
    match dtype {
        DataType::String(_) => Ok(Scalar::from(value)),
        DataType::Boolean => value
            .parse::<bool>()
            .map(Scalar::from)
            .map_err(|_| invalid()),
        DataType::Int64 => value
            .parse::<i64>()
            .map(Scalar::from)
            .map_err(|_| invalid()),
        DataType::Float64 => value
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(Scalar::from)
            .ok_or_else(invalid),
        DataType::Date32 | DataType::Time32(_) | DataType::Time64(_) => {
            Scalar::from_temporal_text(dtype, value).map_err(|_| invalid())
        }
        DataType::DateTime64 {
            unit,
            timezone: zone,
        } if !zone.is_naive() => {
            // A reading that names its own offset is the crate's; a naive one
            // is autotyping's own rule, a wall clock in the column's zone.
            if let Ok(instant) = Scalar::from_temporal_text(dtype, value) {
                return Ok(instant);
            }
            let (local, source) = iso::parse_datetime(value).map_err(|_| invalid())?;
            let count =
                zoned_count(local, source, timezone.unwrap_or(zone)).map_err(|_| invalid())?;
            let count = rescale(count, source, *unit).ok_or_else(invalid)?;
            Scalar::datetime64(count, *unit, *zone).map_err(|_| invalid())
        }
        DataType::DateTime64 { .. } => {
            Scalar::from_temporal_text(dtype, value).map_err(|_| invalid())
        }
        _ => Err(format_smolstr!(
            "autotype produced unsupported datatype {dtype}"
        )),
    }
}

fn zoned_count(local: i64, unit: TimeUnit, zone: &Timezone) -> Result<i64> {
    let per = iso::per_second(unit).ok_or_else(|| Error::InvalidRecord {
        path: SmolStr::new_static("$.timezone"),
        reason: SmolStr::new_static("timestamp unit has no fixed second width"),
    })?;
    let seconds = local.div_euclid(per);
    let fraction = local.rem_euclid(per);
    (*zone)
        .into_utc(seconds)?
        .checked_mul(per)
        .and_then(|seconds| seconds.checked_add(fraction))
        .ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new_static("$.timezone"),
            reason: SmolStr::new_static("zoned timestamp is out of range"),
        })
}

fn rescale(count: i64, source: TimeUnit, target: TimeUnit) -> Option<i64> {
    let source = nanos(source)?;
    let target = nanos(target)?;
    let nanos = i128::from(count).checked_mul(source)?;
    (nanos % target == 0)
        .then(|| i64::try_from(nanos / target).ok())
        .flatten()
}

const fn nanos(unit: TimeUnit) -> Option<i128> {
    match unit {
        TimeUnit::Second => Some(1_000_000_000),
        TimeUnit::Millisecond => Some(1_000_000),
        TimeUnit::Microsecond => Some(1_000),
        TimeUnit::Nanosecond => Some(1),
        _ => None,
    }
}

pub(crate) fn physical_rownum(start: Option<i64>, index: u64) -> Result<Option<i64>> {
    let Some(start) = start else {
        return Ok(None);
    };
    let offset = i64::try_from(index).map_err(|_| rownum_overflow(index))?;
    start
        .checked_add(offset)
        .map(Some)
        .ok_or_else(|| rownum_overflow(index))
}

fn rownum_overflow(index: u64) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$[{index}].seqnum"),
        reason: SmolStr::new_static("text row number exceeds i64::MAX"),
    }
}

pub(crate) fn row_error(
    index: u64,
    rownum: Option<i64>,
    url: Option<&Url>,
    column: &str,
    reason: SmolStr,
) -> Error {
    let row = rownum.map_or_else(
        || format_smolstr!("physical line {}", index.saturating_add(1)),
        |rownum| format_smolstr!("row {rownum}"),
    );
    let url = url.map_or_else(
        || SmolStr::new_static("<anonymous>"),
        |url| format_smolstr!("{url}"),
    );
    Error::InvalidRecord {
        path: format_smolstr!("$[{index}].{column}"),
        reason: format_smolstr!("{reason} in {row} of {url}"),
    }
}

/// Replace a leaf with the `body` bytes from each input row.
pub(crate) fn write_arrow_reader(
    handle: &mut (impl IOBase + ?Sized),
    batches: BatchReader,
    options: &TextOptions,
) -> Result<()> {
    let charset = Charset::from_media_type(handle.media_type());
    let encoded = encoded_bodies(batches, options, charset, handle.codec(), options.level())?;
    handle.write_all_bytes(&encoded)
}

/// Append input `body` values after the leaf's current final line.
pub(crate) fn append_arrow_reader(
    handle: &mut (impl IOBase + ?Sized),
    batches: BatchReader,
    options: &TextOptions,
) -> Result<()> {
    // The tail of what is there is compared with the terminator as the
    // handle declares it, since that is how the rows already there end.
    let charset = Charset::from_media_type(handle.media_type());
    let terminator = encoded_terminator(charset, options.output_linesep())?;
    let codec = handle.codec();
    if codec == Codec::Identity {
        let rendered = encoded_bodies(batches, options, charset, codec, options.level())?;
        let mut offset = handle.size();
        if offset > 0 && !ends_with(handle, &terminator)? {
            handle.pwrite_all(offset, &terminator)?;
            offset += terminator.len() as u64;
        }
        handle.pwrite_all(offset, &rendered)?;
        return handle.flush();
    }

    let mut encoded = Vec::new();
    {
        let mut encoder = codec.writer_with_level(&mut encoded, options.level());
        let mut suffix = Vec::new();
        if !handle.is_empty() {
            let source = handle.pstream_bytes(0, crate::DEFAULT_FETCH_BYTE_SIZE)?;
            let mut decoder = codec.reader(fetched(source));
            let mut chunk = vec![0; crate::DEFAULT_STREAM_BATCH_SIZE];
            loop {
                let read = decoder.read(&mut chunk)?;
                if read == 0 {
                    break;
                }
                update_suffix(&mut suffix, &chunk[..read], terminator.len());
                encoder.write_all(&chunk[..read])?;
            }
        }
        if !suffix.is_empty() && suffix.as_slice() != terminator.as_ref() {
            encoder.write_all(&terminator)?;
        }
        render_declared(batches, options, charset, &mut encoder)?;
        encoder.finish()?;
    }
    handle.write_all_bytes(&encoded)
}

fn encoded_bodies(
    batches: BatchReader,
    options: &TextOptions,
    charset: Charset,
    codec: Codec,
    level: crate::Level,
) -> Result<Vec<u8>> {
    let mut encoded = Vec::new();
    {
        let mut encoder = codec.writer_with_level(&mut encoded, level);
        render_declared(batches, options, charset, &mut encoder)?;
        encoder.finish()?;
    }
    Ok(encoded)
}

/// Render the bodies in the charset the handle declares, inside the coding.
///
/// The writer follows the declaration the reader follows, or a declared
/// handle would read its own UTF-8 back as legacy bytes. Under UTF-8 or
/// US-ASCII the text is written as it is - the same rule that leaves the
/// reader's transport unwrapped - and under any other charset it goes
/// through the charset's writer, finished before the coding writer is.
fn render_declared(
    batches: BatchReader,
    options: &TextOptions,
    charset: Charset,
    target: &mut impl Write,
) -> Result<()> {
    if !transports(charset) {
        return render_batches(batches, options, target);
    }
    let mut writer = charset.writer(target);
    render_batches(batches, options, &mut writer)?;
    writer.finish()
}

/// The record terminator as the declared charset spells it.
///
/// Borrowed under UTF-8 and US-ASCII, where the terminator is written as it
/// is; under any other charset the terminator is text like the bodies it
/// ends, and one that is not text has no spelling there to compare against.
fn encoded_terminator(charset: Charset, terminator: &[u8]) -> Result<Cow<'_, [u8]>> {
    if !transports(charset) {
        return Ok(Cow::Borrowed(terminator));
    }
    let text = std::str::from_utf8(terminator).map_err(|_| Error::InvalidRecord {
        path: SmolStr::new_static("$.linesep"),
        reason: crate::text::expected_got(
            format_args!("a record terminator that is text, to write it as {charset}"),
            format_args!("{terminator:?}"),
        ),
    })?;
    charset.encode(text)
}

fn render_batches(
    batches: BatchReader,
    options: &TextOptions,
    target: &mut impl Write,
) -> Result<()> {
    let body = BodyColumn::resolve(batches.schema().as_ref())?;
    let terminator = options.output_linesep();
    let mut rendered = Vec::with_capacity(crate::DEFAULT_STREAM_BATCH_SIZE);
    for batch in batches {
        let batch = batch.map_err(crate::arrow::from_reader_error)?;
        rendered.clear();
        body.render(
            &batch,
            options.linesep().is_none(),
            terminator,
            &mut rendered,
        )?;
        target.write_all(&rendered)?;
    }
    Ok(())
}

/// The `body` column a write renders, whichever string layout carries it.
///
/// Text and nothing else, because that is what a text row's body is: a
/// `binary` column may hold anything, and rendering one would write bytes no
/// reader of the file could read back as the rows they were. The three
/// layouts Arrow spells a string in are one spelling here.
struct BodyColumn {
    /// Where the column sits in every batch.
    at: usize,
    /// The field every batch's column lands under, read off the schema once.
    field: Arc<crate::Field>,
}

impl BodyColumn {
    fn resolve(schema: &Schema) -> Result<Self> {
        let index = schema
            .fields()
            .iter()
            .position(|field| field.name().eq_ignore_ascii_case("body"))
            .ok_or_else(|| Error::InvalidRecord {
                path: SmolStr::new_static("$.body"),
                reason: SmolStr::new_static("expected a utf8 body column to encode text rows"),
            })?;
        if !is_string_layout(schema.field(index).data_type()) {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.body"),
                reason: format_smolstr!(
                    "expected a utf8 body column, got {}",
                    schema.field(index).data_type()
                ),
            });
        }
        let field = crate::Field::from_arrow_field(schema.field(index))?;
        Ok(Self {
            at: index,
            field: Arc::new(field),
        })
    }

    fn render(
        &self,
        batch: &RecordBatch,
        flexible: bool,
        terminator: &[u8],
        output: &mut Vec<u8>,
    ) -> Result<()> {
        let column = crate::serie::land(
            Arc::clone(&self.field),
            Arc::clone(batch.column(self.at)),
            &crate::serie::Proof::Unproven,
        )?;
        let body = Bodies::of(&column)?;
        for row in 0..batch.num_rows() {
            let Some(value) = body.get(row)? else {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$[{row}].body"),
                    reason: SmolStr::new_static("expected a non-null line body"),
                });
            };
            // A cell stating nothing would write the terminator alone, and
            // a blank line is not a record a reader reads back: the row
            // would be lost where it was meant to be kept.
            if value.is_empty() {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$[{row}].body"),
                    reason: SmolStr::new_static("expected a line body, got an empty one"),
                });
            }
            let contains_break = if flexible {
                memchr::memchr2(b'\n', b'\r', value).is_some()
            } else {
                memchr::memmem::find(value, terminator).is_some()
            };
            if contains_break {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$[{row}].body"),
                    reason: SmolStr::new_static(
                        "expected one line body without its record terminator",
                    ),
                });
            }
            output.extend_from_slice(value);
            output.extend_from_slice(terminator);
        }
        Ok(())
    }
}

/// Whether a column holds text in one of the layouts Arrow spells a string
/// in - plain, large-offset, view, or any of those behind a dictionary.
fn is_string_layout(dtype: &ArrowDataType) -> bool {
    match dtype {
        ArrowDataType::Utf8 | ArrowDataType::LargeUtf8 | ArrowDataType::Utf8View => true,
        ArrowDataType::Dictionary(_, values) => is_string_layout(values),
        _ => false,
    }
}

/// One landed `body` column, narrowed once to where its text lies.
///
/// Every string layout lends each row's run where it lies. A
/// dictionary-encoded string column - what Arrow JS infers for a plain
/// record's string - reads each row through its key into the values it
/// points at, so nothing is unpacked.
enum Bodies<'a> {
    Runs(&'a crate::Serie),
    Encoded {
        keys: &'a crate::Serie,
        values: &'a crate::Serie,
    },
}

impl<'a> Bodies<'a> {
    fn of(column: &'a crate::Serie) -> Result<Self> {
        // Proven a text storage leaf once, so every row after reads its run
        // and a `None` is an absent row, never another layout.
        let text = |serie: &'a crate::Serie| {
            if serie.is_string_storage() {
                return Ok(serie);
            }
            Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.body"),
                reason: format_smolstr!(
                    "expected a utf8 body column, got {}",
                    serie.field().map_or(&DataType::Null, crate::Field::dtype)
                ),
            })
        };
        Ok(match column.as_dictionary() {
            Some(encoded) => Self::Encoded {
                keys: encoded.keys(),
                values: text(encoded.values())?,
            },
            None => Self::Runs(text(column)?),
        })
    }

    /// The body of one row, `None` where the row has none.
    fn get(&self, row: usize) -> Result<Option<&'a [u8]>> {
        Ok(match self {
            Self::Runs(runs) => runs.value_bytes(row),
            Self::Encoded { keys, values } => keys
                .scalar(row)?
                .as_i128()
                .and_then(|key| usize::try_from(key).ok())
                .and_then(|key| values.value_bytes(key)),
        })
    }
}

fn ends_with(handle: &(impl IOBase + ?Sized), suffix: &[u8]) -> Result<bool> {
    let size = handle.size();
    if size < suffix.len() as u64 {
        return Ok(false);
    }
    Ok(handle.read_range_bytes(size - suffix.len() as u64, suffix.len())? == suffix)
}

fn update_suffix(suffix: &mut Vec<u8>, bytes: &[u8], width: usize) {
    if width == 0 {
        return;
    }
    if bytes.len() >= width {
        suffix.clear();
        suffix.extend_from_slice(&bytes[bytes.len() - width..]);
        return;
    }
    suffix.extend_from_slice(bytes);
    if suffix.len() > width {
        suffix.drain(..suffix.len() - width);
    }
}
