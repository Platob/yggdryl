//! The one owned, decoded byte source every record encoding reads through.
//!
//! A record reader outlives the borrow it was built from and a compressed leaf
//! must not be snapshotted into memory to be scanned, so both answers live
//! here rather than in one encoding: the owned view of a leaf, the fetch
//! window the transport is buffered at, and the decoders that treat an absent
//! resource as empty instead of as a truncated stream.

use std::io::{BufRead as _, BufReader, Read};

use crate::holder::{Buffer, Holder};
use crate::{Codec, IOBase, Result};

/// Return an owned view for a reader that must outlive this borrow.
pub(crate) fn owned_leaf(handle: &(impl IOBase + ?Sized)) -> Result<Holder> {
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
pub(crate) fn fetched<R: Read>(source: R) -> BufReader<R> {
    BufReader::with_capacity(crate::DEFAULT_FETCH_BYTE_SIZE, source)
}

pub(crate) struct SendDecodedReader {
    source: Option<Box<dyn Read + Send>>,
    codings: Vec<crate::MimeType>,
    reader: Option<Box<dyn Read + Send>>,
    done: bool,
}

impl SendDecodedReader {
    pub(crate) fn new(source: Box<dyn Read + Send>, codings: Vec<crate::MimeType>) -> Self {
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

impl Read for SendDecodedReader {
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
pub(crate) struct DecodedReader<'source> {
    source: Option<Box<dyn Read + 'source>>,
    codings: Vec<crate::MimeType>,
    reader: Option<Box<dyn Read + 'source>>,
    done: bool,
}

impl<'source> DecodedReader<'source> {
    pub(crate) fn new(source: Box<dyn Read + 'source>, codings: Vec<crate::MimeType>) -> Self {
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

impl Read for DecodedReader<'_> {
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

