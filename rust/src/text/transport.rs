//! The decoded transport every text-shaped medium reads through and the
//! declared charset it writes through.
//!
//! One stream over a handle, the codings its media type names peeled
//! innermost-last, then the charset it declares laid over the bytes where
//! that charset is not UTF-8 or US-ASCII. [`owned_decoded`] and
//! [`borrowed_decoded`] compose it once for every text-shaped medium, so the
//! call counts the media pin are the same counts: one `pstream_bytes` or one
//! owned stream per read, and one `media_type` ask answering both the
//! codings and the charset.

use std::borrow::Cow;
use std::io::{BufRead, BufReader, Chain, Read};

use smol_str::SmolStr;

use crate::{Charset, Codec, Cursor, Error, IOBase, Result, charset};

/// The decoded transport over a handle the reader owns, for a read that
/// outlives the call: a located handle streams from its location, any
/// other through a cursor over itself.
///
/// One ask of the handle answers both the codings the transport peels and
/// the charset it decodes under, the one `media_type` read the call-count
/// pins hold every text-shaped read to.
pub(crate) fn owned_decoded<H: IOBase + 'static>(handle: H) -> Box<dyn Read + Send + 'static> {
    let media_type = handle.media_type();
    let codings = media_type.encodings().to_vec();
    let charset = Charset::from_media_type(media_type);
    match handle.bound_location().cloned() {
        Some(bound) => Box::new(BoundReader::new(bound, codings, charset)),
        None => Box::new(NonemptySendDecodedReader::new(
            Box::new(Cursor::new(handle)),
            codings,
            charset,
        )),
    }
}

/// The same transport borrowed, for a read that ends inside the call - a
/// count, or a header and a sample: one `pstream_bytes` from the start.
///
/// # Errors
///
/// Returns the handle's refusal to stream.
pub(crate) fn borrowed_decoded(
    handle: &(impl IOBase + ?Sized),
) -> Result<NonemptyDecodedReader<'_>> {
    let media_type = handle.media_type();
    let codings = media_type.encodings().to_vec();
    let charset = Charset::from_media_type(media_type);
    let raw: Box<dyn Read + '_> =
        Box::new(handle.pstream_bytes(0, crate::DEFAULT_FETCH_BYTE_SIZE)?);
    Ok(NonemptyDecodedReader::new(raw, codings, charset))
}

/// Buffer one transport at the fetch window every decoded read pulls through.
///
/// A decoder asks its source for its own internal window - 32 KiB for gzip -
/// and on a remote store each of those asks is a round trip. This is the only
/// place the text reader touches the transport, so it is the one place that
/// has to hold a window big enough to make a scan cost requests proportional
/// to the object's size rather than to the decoder's appetite.
pub(crate) fn fetched<R: Read>(source: R) -> BufReader<R> {
    BufReader::with_capacity(crate::DEFAULT_FETCH_BYTE_SIZE, source)
}

/// Whether a declared charset is laid over the transport at all.
///
/// UTF-8 and US-ASCII never are: the line layer already reads both by rule
/// one - every valid UTF-8 run kept, every stray byte as Windows-1252, where
/// the line is made - and writes UTF-8 as it is, so under either the
/// transport is the object `main` builds and the writer the bytes it renders.
pub(crate) const fn transports(charset: Charset) -> bool {
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
pub(crate) fn declared<R: Read>(
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
pub(crate) struct BoundReader {
    bound: Option<crate::fs::BoundLocation>,
    codings: Vec<crate::MimeType>,
    charset: Charset,
    reader: Option<Box<dyn Read + Send>>,
    done: bool,
}

impl BoundReader {
    pub(crate) fn new(
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
        let mut reader = Codec::decoding_send(&self.codings, stream);
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
pub(crate) struct NonemptySendDecodedReader {
    source: Option<Box<dyn Read + Send>>,
    codings: Vec<crate::MimeType>,
    charset: Charset,
    reader: Option<Box<dyn Read + Send>>,
    done: bool,
}

impl NonemptySendDecodedReader {
    pub(crate) fn new(
        source: Box<dyn Read + Send>,
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
        let mut reader = Codec::decoding_send(&self.codings, source);
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
pub(crate) struct NonemptyDecodedReader<'source> {
    source: Option<Box<dyn Read + 'source>>,
    codings: Vec<crate::MimeType>,
    charset: Charset,
    reader: Option<Box<dyn Read + 'source>>,
    done: bool,
}

impl<'source> NonemptyDecodedReader<'source> {
    pub(crate) fn new(
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
        let mut reader = Codec::decoding(&self.codings, source);
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

/// The record terminator as the declared charset spells it.
///
/// Borrowed under UTF-8 and US-ASCII, where the terminator is written as it
/// is; under any other charset the terminator is text like the bodies it
/// ends, and one that is not text has no spelling there to compare against.
pub(crate) fn encoded_terminator(charset: Charset, terminator: &[u8]) -> Result<Cow<'_, [u8]>> {
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

/// Whether the handle's bytes end with `suffix`.
pub(crate) fn ends_with(handle: &(impl IOBase + ?Sized), suffix: &[u8]) -> Result<bool> {
    let size = handle.size();
    if size < suffix.len() as u64 {
        return Ok(false);
    }
    Ok(handle.read_range_bytes(size - suffix.len() as u64, suffix.len())? == suffix)
}

/// Keep the last `width` bytes seen across chunks in `suffix`.
pub(crate) fn update_suffix(suffix: &mut Vec<u8>, bytes: &[u8], width: usize) {
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
