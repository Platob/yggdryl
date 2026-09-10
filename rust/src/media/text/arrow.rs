//! `text/plain` rows through the shared Scalar/Arrow record boundary.

use std::io::{BufRead, BufReader, Read, Write};
use std::ops::Range;
use std::sync::Arc;

use arrow_array::{Array as _, BinaryArray, RecordBatch};
use arrow_schema::{DataType as ArrowDataType, Schema};
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
use crate::types::ascii::iso;
use crate::{Codec, DataType, Error, Result, Scalar, TimeUnit, Timezone, Url};
use crate::{Cursor, IOBase};

use super::leading::LeadingFragment;
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
    let url = handle.url().cloned();
    // Read from the handle the caller gave, before `owned_handle` may answer
    // with a copy: a buffered copy of the bytes is not the object whose
    // modification time this is.
    let mtime = handle_mtime(handle, options);
    read_owned_arrow_reader_at(owned_handle(handle)?, url, mtime, options)
}

/// The handle's own modification time, asked for only when a column wants it.
fn handle_mtime(handle: &(impl IOBase + ?Sized), options: &TextOptions) -> Option<i128> {
    if !options.parse_mtime {
        return None;
    }
    handle.mtime().map(i128::from)
}

/// Decode an owned leaf without retaining decoded pages in its caller.
pub(crate) fn read_owned_arrow_reader<H: IOBase + 'static>(
    handle: H,
    options: &TextOptions,
) -> Result<BatchReader> {
    let url = handle.url().cloned();
    let mtime = handle_mtime(&handle, options);
    read_owned_arrow_reader_at(handle, url, mtime, options)
}

fn read_owned_arrow_reader_at<H: IOBase + 'static>(
    handle: H,
    url: Option<Url>,
    mtime: Option<i128>,
    options: &TextOptions,
) -> Result<BatchReader> {
    options.require_framing_rowheader()?;
    let codings = handle.media_type().encodings().to_vec();
    let source: Box<dyn Read + Send + 'static> = match handle.bound_location().cloned() {
        Some(bound) => Box::new(BoundReader::new(bound, codings)),
        None => Box::new(NonemptySendDecodedReader::new(
            Box::new(Cursor::new(handle)),
            codings,
        )),
    };

    let lines = text_lines(source, url, mtime, options)?;
    super::batch::into_arrow_reader(lines, options)
}

/// Build the one decode iterator over an already-opened stream.
///
/// The plan decides here, once, which optional work every line pays for: a read
/// whose columns never touch an entry never materializes a tree, and one whose
/// columns never name the classification never scans for it.
fn text_lines(
    source: Box<dyn Read + Send + 'static>,
    url: Option<Url>,
    mtime: Option<i128>,
    options: &TextOptions,
) -> Result<TextLines> {
    let plan = options.plan()?;
    let timestamp_capture = options
        .parse_mtime
        .then(|| (0..options.capture_names().len()).find(|index| options.consumes_capture(*index)))
        .flatten();
    Ok(TextLines {
        raw: RawRows::new(source, url.clone(), Arc::new(options.clone())),
        url: url.map(Arc::new),
        mtime,
        reads_entries: plan.reads_entries(),
        reads_classification: plan.reads_classification(),
        timezone: options.timezone().copied(),
        timestamp_capture,
    })
}

/// Decode one borrowed leaf into typed lines.
///
/// The one decode entry point. Every record method routes through it, and
/// nothing else parses a line.
///
/// # Errors
///
/// Returns the configuration's refusals - a framing mode with no header
/// pattern, a rename naming no column, a lifted path with no name - before a
/// byte is read.
pub fn read_text_lines(
    handle: &(impl IOBase + ?Sized),
    options: &TextOptions,
) -> Result<TextLines> {
    options.require_framing_rowheader()?;
    let url = handle.url().cloned();
    // Read from the handle the caller gave, before `owned_handle` may answer
    // with a copy: a buffered copy of the bytes is not the object whose
    // modification time this is.
    let mtime = handle_mtime(handle, options);
    read_owned_text_lines_at(owned_handle(handle)?, url, mtime, options)
}

/// The same decode over a handle the iterator owns.
fn read_owned_text_lines_at<H: IOBase + 'static>(
    handle: H,
    url: Option<Url>,
    mtime: Option<i128>,
    options: &TextOptions,
) -> Result<TextLines> {
    options.require_framing_rowheader()?;
    let codings = handle.media_type().encodings().to_vec();
    let source: Box<dyn Read + Send + 'static> = match handle.bound_location().cloned() {
        Some(bound) => Box::new(BoundReader::new(bound, codings)),
        None => Box::new(NonemptySendDecodedReader::new(
            Box::new(Cursor::new(handle)),
            codings,
        )),
    };
    text_lines(source, url, mtime, options)
}

/// Count emitted records without materializing rows or Arrow arrays.
pub(crate) fn row_size(handle: &(impl IOBase + ?Sized), options: &TextOptions) -> Result<u64> {
    options.require_framing_rowheader()?;
    let mut counting = options.clone();
    counting.start_rownum = None;
    counting.parse_mtime = false;
    counting.set_lstrip::<[&str; 0], &str>([])?;
    counting.set_rstrip::<[&str; 0], &str>([])?;
    counting.set_max_record_byte_size(Some(0));
    let codings = handle.media_type().encodings().to_vec();
    let raw: Box<dyn Read + '_> =
        Box::new(handle.pstream_bytes(0, crate::DEFAULT_FETCH_BYTE_SIZE)?);
    let source = NonemptyDecodedReader::new(raw, codings);
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
        let mut file = crate::holder::fs::File::new(bound.clone());
        file.set_media_type(handle.media_type().clone());
        return Ok(Holder::FsFile(file));
    }
    if let Some(parent) = handle.parent() {
        if let Some(name) = handle.url().and_then(crate::Url::file_name) {
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

/// One lazily opened filesystem stream retained for the complete decode.
struct BoundReader {
    bound: Option<crate::holder::fs::BoundLocation>,
    codings: Vec<crate::MimeType>,
    reader: Option<Box<dyn Read + Send>>,
    done: bool,
}

impl BoundReader {
    fn new(bound: crate::holder::fs::BoundLocation, codings: Vec<crate::MimeType>) -> Self {
        Self {
            bound: Some(bound),
            codings,
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
        let stream = match crate::holder::fs::File::new(bound).open_input_stream() {
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
    stream: Box<dyn crate::holder::fs::ByteReader>,
}

impl BoundStream {
    fn new(stream: Box<dyn crate::holder::fs::ByteReader>) -> Self {
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
    reader: Option<Box<dyn Read + Send>>,
    done: bool,
}

impl NonemptySendDecodedReader {
    fn new(source: Box<dyn Read + Send>, codings: Vec<crate::MimeType>) -> Self {
        Self {
            source: Some(source),
            codings,
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
    reader: Option<Box<dyn Read + 'source>>,
    done: bool,
}

impl<'source> NonemptyDecodedReader<'source> {
    fn new(source: Box<dyn Read + 'source>, codings: Vec<crate::MimeType>) -> Self {
        Self {
            source: Some(source),
            codings,
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
    /// The retained body, owned once and shared from there.
    ///
    /// A sized owner rather than `Arc<[u8]>`: the conversion from `Vec` to a
    /// shared slice reallocates and copies every byte, and wrapping the vector
    /// instead is free.
    body: Arc<Vec<u8>>,
    dropped_byte_size: Option<u64>,
    captures: Vec<Option<Vec<u8>>>,
    direction: Option<&'static str>,
}

/// Physical lines or framed records parsed against one precomputed schema.
struct RawRows<R> {
    lines: Lines<R>,
    header_dfa: Option<DFA<Vec<u32>>>,
    url: Option<Url>,
    options: Arc<TextOptions>,
    capture_values: bool,
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

/// One physical line after header removal and edge stripping.
struct ParsedLine {
    index: u64,
    body: Body,
    captures: Vec<Option<Vec<u8>>>,
    matched: bool,
    direction: Option<&'static str>,
}

/// One physical line, possibly reduced after its header DFA proves no match.
struct PhysicalLine {
    bytes: Vec<u8>,
    decoded_size: u64,
    header: ScannedHeader,
}

/// What the incremental row-header scan proved before the line ended.
enum ScannedHeader {
    /// The complete-line regex still decides the answer.
    Unresolved,
    /// No later byte can make this physical line match.
    Nonmatching,
    /// The header was removed while its match prefix was still retained.
    Matched {
        removed_size: u64,
        captures: Vec<Option<Vec<u8>>>,
    },
}

/// One complete row-header match and its raw capture bytes.
struct HeaderMatch {
    range: Range<usize>,
    captures: Vec<Option<Vec<u8>>>,
}

/// A bounded retained prefix plus its complete decoded byte length.
struct Body {
    bytes: Vec<u8>,
    decoded_size: u64,
}

/// One logical record retained across physical input and Arrow batch pulls.
struct RawRecord {
    index: u64,
    body: Vec<u8>,
    decoded_size: u64,
    captures: Vec<Option<Vec<u8>>>,
    /// The direction the first physical line of this record carried.
    ///
    /// A framed record spans several lines and the marker sits in front of
    /// the first, which is the one the transport wrote it on.
    direction: Option<&'static str>,
}

fn header_dfa(source: &str) -> Option<DFA<Vec<u32>>> {
    DfaBuilder::new()
        .syntax(syntax::Config::new().utf8(false))
        .thompson(thompson::Config::new().utf8(false))
        .build(source)
        .ok()
}

fn header_match(options: &TextOptions, bytes: &[u8], capture_values: bool) -> Option<HeaderMatch> {
    let rowheader = options.rowheader_regex()?;
    let found = rowheader.captures(bytes)?;
    let whole = found.get(0)?;
    let range = whole.start()..whole.end();
    if !capture_values {
        return Some(HeaderMatch {
            range,
            captures: Vec::new(),
        });
    }
    let mut captures = vec![None; options.capture_names().len()];
    for (target, capture_index) in captures.iter_mut().zip(
        rowheader
            .capture_names()
            .enumerate()
            .filter_map(|(index, name)| name.map(|_| index)),
    ) {
        let Some(value) = found.get(capture_index) else {
            continue;
        };
        *target = Some(value.as_bytes().to_vec());
    }
    Some(HeaderMatch { range, captures })
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
        Self {
            lines: Lines::new(source),
            header_dfa,
            url,
            options,
            capture_values,
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
    ) -> Result<ParsedLine> {
        let PhysicalLine {
            mut bytes,
            decoded_size,
            header,
        } = line;
        let (body, captures, matched, body_decoded_size) = match header {
            ScannedHeader::Nonmatching => (
                bytes,
                if capture_values {
                    vec![None; options.capture_names().len()]
                } else {
                    Vec::new()
                },
                false,
                decoded_size,
            ),
            ScannedHeader::Matched {
                removed_size,
                captures,
            } => {
                let body_decoded_size =
                    decoded_size
                        .checked_sub(removed_size)
                        .ok_or_else(|| Error::InvalidRecord {
                            path: format_smolstr!("$[{index}].body"),
                            reason: SmolStr::new_static(
                                "row-header match exceeds the decoded physical line",
                            ),
                        })?;
                (bytes, captures, true, body_decoded_size)
            }
            ScannedHeader::Unresolved => {
                if let Some(found) = header_match(options, &bytes, capture_values) {
                    let removed_size =
                        u64::try_from(found.range.len()).map_err(|_| Error::InvalidRecord {
                            path: format_smolstr!("$[{index}].body"),
                            reason: SmolStr::new_static("row-header match exceeds u64::MAX bytes"),
                        })?;
                    bytes.drain(found.range);
                    let body_decoded_size =
                        decoded_size.checked_sub(removed_size).ok_or_else(|| {
                            Error::InvalidRecord {
                                path: format_smolstr!("$[{index}].body"),
                                reason: SmolStr::new_static(
                                    "row-header match exceeds the decoded physical line",
                                ),
                            }
                        })?;
                    (bytes, found.captures, true, body_decoded_size)
                } else {
                    (
                        bytes,
                        if capture_values {
                            vec![None; options.capture_names().len()]
                        } else {
                            Vec::new()
                        },
                        false,
                        decoded_size,
                    )
                }
            }
        };

        let mut start = 0;
        let mut end = body.len();
        // Each pattern strips from the edge the one before it left, so a
        // layered prefix comes off a layer at a time rather than through one
        // expression nobody can read.
        for lstrip in options.lstrip_regexes() {
            if let Some(found) = lstrip
                .find(&body[start..end])
                .filter(|found| found.start() == 0)
            {
                start += found.end();
            }
        }
        for rstrip in options.rstrip_regexes() {
            if let Some(found) = rstrip
                .find_iter(&body[start..end])
                .filter(|found| found.end() == end - start)
                .last()
            {
                end = start + found.start();
            }
        }
        // The direction marker is transport prose in front of the payload, so
        // reading it takes it off the body: a body that kept it would carry a
        // word no protocol sent.
        let direction = if options.parse_direction {
            let (direction, kept) = crate::types::MsgDirection::split_bytes(&body[start..end]);
            start = end - kept.len();
            direction
        } else {
            None
        };

        let decoded_size = if options.rewrites_body() {
            u64::try_from(end - start).map_err(|_| Error::InvalidRecord {
                path: format_smolstr!("$[{index}].body"),
                reason: SmolStr::new_static("decoded text record exceeds u64::MAX bytes"),
            })?
        } else {
            body_decoded_size
        };
        let retained = options.max_record_byte_size().map_or(end - start, |limit| {
            usize::try_from(limit)
                .unwrap_or(usize::MAX)
                .min(end - start)
        });
        Ok(ParsedLine {
            index,
            body: Body {
                bytes: body[start..start + retained].to_vec(),
                decoded_size,
            },
            captures,
            matched,
            direction,
        })
    }

    fn next_line(&mut self) -> Option<Result<ParsedLine>> {
        let options = Arc::clone(&self.options);
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
        let mut bytes = Vec::new();
        let mut decoded_size = 0_u64;
        loop {
            let part = match self.lines.next_part(options.linesep())? {
                Ok(part) => part,
                Err(error) => {
                    self.done = true;
                    return Some(Err(error));
                }
            };
            let part_size = u64::try_from(part.bytes.len()).unwrap_or(u64::MAX);
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
            if !matches!(header, ScannedHeader::Unresolved) {
                let available = retained.saturating_sub(bytes.len()).min(part.bytes.len());
                bytes.extend_from_slice(&part.bytes[..available]);
            } else {
                let part_start = bytes.len();
                bytes.extend_from_slice(part.bytes);
                if let (Some(dfa), Some(mut current)) = (&self.header_dfa, state) {
                    for (offset, &byte) in part.bytes.iter().enumerate() {
                        current = dfa.next_state(current, byte);
                        if dfa.is_match_state(current) {
                            dfa_matched = true;
                            continue;
                        }
                        if dfa.is_dead_state(current) {
                            let scan_end = part_start + offset + 1;
                            if dfa_matched {
                                if let Some(found) =
                                    header_match(&options, &bytes[..scan_end], self.capture_values)
                                {
                                    let removed_size = match u64::try_from(found.range.len()) {
                                        Ok(size) => size,
                                        Err(_) => {
                                            self.done = true;
                                            return Some(Err(Error::InvalidRecord {
                                                path: format_smolstr!("$[{index}].body"),
                                                reason: SmolStr::new_static(
                                                    "row-header match exceeds u64::MAX bytes",
                                                ),
                                            }));
                                        }
                                    };
                                    bytes.drain(found.range);
                                    bytes.truncate(retained);
                                    header = ScannedHeader::Matched {
                                        removed_size,
                                        captures: found.captures,
                                    };
                                }
                            } else {
                                header = ScannedHeader::Nonmatching;
                                bytes.truncate(retained);
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
            if part.end {
                break;
            }
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
        Some(Self::parse_line(&options, line, index, self.capture_values))
    }

    fn next_framed(&mut self) -> Option<Result<RawRow>> {
        loop {
            let line = match self.next_line() {
                Some(Ok(line)) => line,
                Some(Err(error)) => return Some(Err(error)),
                None => {
                    self.done = true;
                    return self.active.take().map(RawRecord::finish).map(Ok);
                }
            };
            if line.matched {
                let next = RawRecord::new(line, self.options.max_record_byte_size());
                if let Some(record) = self.active.replace(next) {
                    return Some(Ok(record.finish()));
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
            body,
            captures,
            direction,
            ..
        } = line;
        let retained = retained_size(limit, 0, body.bytes.len());
        Self {
            index,
            body: body.bytes[..retained].to_vec(),
            decoded_size: body.decoded_size,
            captures,
            direction,
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
        let separator = retained_size(limit, self.body.len(), 1);
        if separator == 1 {
            self.body.push(b'\n');
        }
        let retained = retained_size(limit, self.body.len(), body.bytes.len());
        self.body.extend_from_slice(&body.bytes[..retained]);
        Ok(())
    }

    fn finish(self) -> RawRow {
        let retained = u64::try_from(self.body.len()).unwrap_or(u64::MAX);
        let dropped_byte_size = self
            .decoded_size
            .checked_sub(retained)
            .filter(|size| *size > 0);
        RawRow {
            index: self.index,
            body: Arc::new(self.body),
            dropped_byte_size,
            captures: self.captures,
            direction: self.direction,
        }
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
                self.next_line().map(|line| {
                    line.map(|line| {
                        RawRecord::new(line, self.options.max_record_byte_size()).finish()
                    })
                })
            };
            let row = row?;
            if !self.options.dedup_adjacent {
                return Some(row);
            }
            let Ok(row) = row else {
                return Some(row);
            };
            let digest = crate::xxhash::xxh128(&row.body);
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
/// else parses a line.
pub struct TextLines {
    raw: RawRows<Box<dyn Read + Send + 'static>>,
    /// The object every line of this read came from, shared rather than rebuilt.
    url: Option<Arc<Url>>,
    /// The handle's own modification time, the fallback every row shares.
    mtime: Option<i128>,
    /// Which optional work the plan actually asked for, decided once.
    reads_entries: bool,
    reads_classification: bool,
    timezone: Option<Timezone>,
    /// Where the timestamp capture sits, when the header declares one.
    timestamp_capture: Option<usize>,
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
    /// Turn one parsed row into the typed line the columns are built from.
    fn convert(&self, row: RawRow) -> Result<TextLine> {
        let index = row.index;
        let body = TextBytes::from_whole_page(row.body)?;
        let mut line = TextLine::new(index, body);
        line.set_url(self.url.clone());
        line.set_direction(row.direction);
        line.set_dropped_byte_size(row.dropped_byte_size);

        if self.reads_classification {
            let (shape, _) = crate::mime_type::line::classify(line.body().as_bytes());
            line.set_bodytype(Some(shape));
        }
        if self.reads_entries {
            let entries = super::entry::read_entries(line.body());
            line.set_entries(entries);
        }

        let mut captures = Vec::with_capacity(row.captures.len());
        for value in row.captures {
            captures.push(match value {
                Some(bytes) => Some(TextBytes::from_bytes(bytes)?),
                None => None,
            });
        }
        match self.timestamp_capture {
            Some(at) => {
                let stamp = self.row_timestamp(captures.get(at), index)?;
                line.set_timestamp(stamp);
            }
            None => line.set_timestamp(self.mtime),
        }
        line.set_captures(captures);
        Ok(line)
    }

    /// The record's own captured write time, falling back to the handle's.
    ///
    /// A header that declares the capture but did not match it on this line
    /// falls back too: a line the expression did not date is exactly the case
    /// the handle's own time is there to answer.
    fn row_timestamp(
        &self,
        capture: Option<&Option<TextBytes>>,
        index: u64,
    ) -> Result<Option<i128>> {
        let Some(Some(raw)) = capture else {
            return Ok(self.mtime);
        };
        let Some(text) = raw.as_str() else {
            return Err(row_error(
                index,
                None,
                self.url.as_deref(),
                super::options::MTIME_COLUMN,
                SmolStr::new_static("expected a UTF-8 row-header capture, got invalid bytes"),
            ));
        };
        let parsed = parse_capture(text, &super::options::mtime_dtype(), self.timezone.as_ref())
            .map_err(|reason| {
                row_error(
                    index,
                    None,
                    self.url.as_deref(),
                    super::options::MTIME_COLUMN,
                    reason,
                )
            })?;
        Ok(parsed
            .as_temporal()
            .map(|held| i128::from(held.count()))
            .or(self.mtime))
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
        DataType::Utf8 => Ok(Scalar::from(value)),
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
        path: format_smolstr!("$[{index}].rownum"),
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
    let encoded = encoded_bodies(batches, options, handle.codec(), options.level())?;
    handle.write_all_bytes(&encoded)
}

/// Append input `body` values after the leaf's current final line.
pub(crate) fn append_arrow_reader(
    handle: &mut (impl IOBase + ?Sized),
    batches: BatchReader,
    options: &TextOptions,
) -> Result<()> {
    let terminator = options.output_linesep();
    let codec = handle.codec();
    if codec == Codec::Identity {
        let rendered = encoded_bodies(batches, options, codec, options.level())?;
        let mut offset = handle.size();
        if offset > 0 && !ends_with(handle, terminator)? {
            handle.pwrite_all(offset, terminator)?;
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
        if !suffix.is_empty() && suffix.as_slice() != terminator {
            encoder.write_all(terminator)?;
        }
        render_batches(batches, options, &mut encoder)?;
        encoder.finish()?;
    }
    handle.write_all_bytes(&encoded)
}

fn encoded_bodies(
    batches: BatchReader,
    options: &TextOptions,
    codec: Codec,
    level: crate::Level,
) -> Result<Vec<u8>> {
    let mut encoded = Vec::new();
    {
        let mut encoder = codec.writer_with_level(&mut encoded, level);
        render_batches(batches, options, &mut encoder)?;
        encoder.finish()?;
    }
    Ok(encoded)
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

struct BodyColumn(usize);

impl BodyColumn {
    fn resolve(schema: &Schema) -> Result<Self> {
        let index = schema
            .fields()
            .iter()
            .position(|field| field.name().eq_ignore_ascii_case("body"))
            .ok_or_else(|| Error::InvalidRecord {
                path: SmolStr::new_static("$.body"),
                reason: SmolStr::new_static("expected a binary body column to encode text rows"),
            })?;
        if schema.field(index).data_type() != &ArrowDataType::Binary {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.body"),
                reason: format_smolstr!(
                    "expected a binary body column, got {}",
                    schema.field(index).data_type()
                ),
            });
        }
        Ok(Self(index))
    }

    fn render(
        &self,
        batch: &RecordBatch,
        flexible: bool,
        terminator: &[u8],
        output: &mut Vec<u8>,
    ) -> Result<()> {
        let body = batch
            .column(self.0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .ok_or_else(|| Error::InvalidRecord {
                path: SmolStr::new_static("$.body"),
                reason: SmolStr::new_static("expected a binary body column"),
            })?;
        for row in 0..batch.num_rows() {
            if body.is_null(row) {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$[{row}].body"),
                    reason: SmolStr::new_static("expected a non-null binary line body"),
                });
            }
            let value = body.value(row);
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
