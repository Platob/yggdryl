//! One decoded byte stream over a record leaf, shared by every text encoding.
//!
//! A record encoding that parses bytes rather than a container header needs the
//! same three things: an owned view of a borrowed leaf, one transport window
//! wide enough that a decoder's appetite does not become the request size, and
//! a decoder stack that treats a missing resource as empty instead of handing a
//! compression reader an empty stream. They live here because `text/plain` and
//! `text/csv` need them identically, and one implementation is what keeps the
//! two from drifting.

use std::io::{BufRead, BufReader, Read};

use crate::holder::{Buffer, Holder};
use crate::{Codec, IOBase, Result};

/// Return an owned view for a reader that must outlive this borrow.
pub(crate) fn owned_handle(handle: &(impl IOBase + ?Sized)) -> Result<Holder> {
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
/// place a text reader touches the transport, so it is the one place that has
/// to hold a window big enough to make a scan cost requests proportional to the
/// object's size rather than to the decoder's appetite.
pub(crate) fn fetched<R: Read>(source: R) -> BufReader<R> {
    BufReader::with_capacity(crate::DEFAULT_FETCH_BYTE_SIZE, source)
}

/// Open the decoded byte stream a borrowed leaf holds, from its first byte.
///
/// The content codings come from the handle's media type, so a `.csv.gz` leaf
/// answers its rows and a `.csv` leaf answers the same bytes it stores.
pub(crate) fn decoded_reader(handle: &(impl IOBase + ?Sized)) -> Result<impl Read + '_> {
    let codings = handle.media_type().encodings().to_vec();
    let raw: Box<dyn Read + '_> =
        Box::new(handle.pstream_bytes(0, crate::DEFAULT_FETCH_BYTE_SIZE)?);
    Ok(NonemptyDecodedReader::new(raw, codings))
}

/// Open the decoded byte stream a borrowed leaf holds, from decoded `offset`.
///
/// An uncoded leaf seeks: the stream starts at the byte asked for and nothing
/// before it is read. A coded one cannot, because a coding's output offset is
/// not addressable in its input, so the prefix is decoded and discarded. That
/// cost is why positional record access states the coding it is paying for.
pub(crate) fn decoded_reader_at(
    handle: &(impl IOBase + ?Sized),
    offset: u64,
) -> Result<Box<dyn Read + '_>> {
    let codings = handle.media_type().encodings().to_vec();
    if codings.is_empty() {
        return Ok(Box::new(
            handle.pstream_bytes(offset, crate::DEFAULT_FETCH_BYTE_SIZE)?,
        ));
    }
    let raw: Box<dyn Read + '_> =
        Box::new(handle.pstream_bytes(0, crate::DEFAULT_FETCH_BYTE_SIZE)?);
    let mut decoded = NonemptyDecodedReader::new(raw, codings);
    discard(&mut decoded, offset)?;
    Ok(Box::new(decoded))
}

/// Read and drop `count` decoded bytes, holding one bounded window.
fn discard(source: &mut impl Read, count: u64) -> Result<()> {
    let mut window = vec![0_u8; crate::DEFAULT_STREAM_BATCH_SIZE];
    let mut left = count;
    while left > 0 {
        let want = usize::try_from(left)
            .unwrap_or(window.len())
            .min(window.len());
        let read = source.read(&mut window[..want]).map_err(crate::Error::Io)?;
        if read == 0 {
            break;
        }
        left -= read as u64;
    }
    Ok(())
}

/// Open the decoded byte stream an owned leaf holds, without borrowing it.
///
/// A batch reader outlives the call that built it, so the handle travels with
/// the stream: a bound filesystem location opens its own input stream lazily,
/// and anything else is read positionally through a cursor.
pub(crate) fn owned_decoded_reader<H: IOBase + 'static>(
    handle: H,
) -> Box<dyn Read + Send + 'static> {
    let codings = handle.media_type().encodings().to_vec();
    match handle.bound_location().cloned() {
        Some(bound) => Box::new(BoundReader::new(bound, codings)),
        None => Box::new(NonemptySendDecodedReader::new(
            Box::new(crate::Cursor::new(handle)),
            codings,
        )),
    }
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
