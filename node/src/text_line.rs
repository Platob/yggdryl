//! Native JavaScript view of the decoded text row and the path that addresses
//! one entry of it.
//!
//! Every class here redirects into the core value it wraps. A body, a key and
//! a value cross as `string`, because the line is text by construction; the
//! `Bytes` accessors answer the same ranges as `Buffer`, copied - state that in
//! the docs rather than claiming a zero copy this boundary does not have.

use napi::bindgen_prelude::{Buffer, Either, Generator, Result};
use napi_derive::napi;

use yggdryl::media::text::{
    TextBytes, TextEntries as CoreTextEntries, TextEntry as CoreTextEntry,
    TextLine as CoreTextLine, TextLines as CoreTextLines,
};
use yggdryl::{FieldPath as CoreFieldPath, FieldSegment};

use crate::napi_error;

/// Whatever spelling of a value the caller used, as the bytes the line holds.
///
/// A `string` is its UTF-8; a `Buffer` is taken as given. What the bytes go
/// to decides the rest: a line decodes what is not text where it is made,
/// while an entry's value is the range it was given and answers the lossy
/// decode of it as text.
fn bytes_from_input(value: Either<Buffer, String>) -> Result<TextBytes> {
    match value {
        Either::A(bytes) => TextBytes::from_bytes(bytes.as_ref()),
        Either::B(text) => TextBytes::from_bytes(text.as_bytes()),
    }
    .map_err(napi_error)
}

/// Whatever spelling of a path the caller used, resolved exactly once.
pub(crate) fn path_from_input(value: Either<String, &JsFieldPath>) -> Result<CoreFieldPath> {
    match value {
        Either::A(text) => CoreFieldPath::from_str(&text).map_err(napi_error),
        Either::B(path) => Ok(path.inner.clone()),
    }
}

/// One resolved path into a nested schema or value.
#[napi(js_name = "FieldPath")]
pub struct JsFieldPath {
    pub(crate) inner: CoreFieldPath,
}

impl JsFieldPath {
    pub(crate) const fn from_core(inner: CoreFieldPath) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsFieldPath {
    /// Parse one path.
    #[napi(constructor)]
    pub fn new(value: Option<String>) -> Result<JsFieldPath> {
        match value {
            Some(text) => CoreFieldPath::from_str(&text)
                .map(Self::from_core)
                .map_err(napi_error),
            None => Ok(Self::from_core(CoreFieldPath::root())),
        }
    }

    /// The empty path, which selects the value it is applied to.
    #[napi(factory, ts_return_type = "FieldPath")]
    pub fn root() -> JsFieldPath {
        Self::from_core(CoreFieldPath::root())
    }

    /// The segments, each a name or a position.
    #[napi(getter, ts_return_type = "Array<string | number>")]
    pub fn segments(&self) -> Vec<Either<String, i64>> {
        self.inner
            .segments()
            .iter()
            .map(|segment| {
                segment.as_name().map_or_else(
                    || Either::B(segment.as_index().unwrap_or_default()),
                    |name| Either::A(name.to_owned()),
                )
            })
            .collect()
    }

    /// The single name this path addresses, when it addresses exactly one.
    #[napi(getter)]
    pub fn name(&self) -> Option<String> {
        self.inner.as_name().map(ToOwned::to_owned)
    }

    /// What to call what this path reaches, written `... as name`.
    #[napi(getter)]
    pub fn alias(&self) -> Option<String> {
        self.inner.alias().map(ToOwned::to_owned)
    }

    /// The name this path gives what it reaches.
    ///
    /// The alias where one is written, and the last segment's own name
    /// otherwise. A lifted text column takes this.
    #[napi(getter)]
    pub fn column_name(&self) -> Option<String> {
        self.inner.column_name().map(ToOwned::to_owned)
    }

    /// Whether this path selects the value it is applied to.
    #[napi(getter)]
    pub fn is_root(&self) -> bool {
        self.inner.is_root()
    }

    /// How many segments this path has.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        // A path is a handful of segments, so `u32` is honest here and the
        // documented bound is the segment count, not a byte count.
        u32::try_from(self.inner.len()).unwrap_or(u32::MAX)
    }

    /// This path without its last segment.
    #[napi(ts_return_type = "FieldPath | null")]
    pub fn parent(&self) -> Option<JsFieldPath> {
        self.inner.parent().map(Self::from_core)
    }

    /// This path with one more named or positional segment.
    #[napi(ts_return_type = "FieldPath")]
    pub fn join(&self, segment: Either<String, i64>) -> JsFieldPath {
        let segment = match segment {
            Either::A(name) => FieldSegment::field(name),
            Either::B(index) => FieldSegment::index(index),
        };
        Self::from_core(self.inner.join(segment))
    }

    /// A deterministic hash of the complete path.
    #[napi]
    pub fn stable_hash(&self) -> i64 {
        // Widened rather than truncated: a 64-bit digest does not fit a
        // JavaScript number, and this boundary never silently narrows one.
        i64::from_ne_bytes(self.inner.stable_hash().to_ne_bytes())
    }

    #[napi(js_name = "toString")]
    pub fn to_js_string(&self) -> String {
        self.inner.to_string()
    }

    /// Whether two paths select the same thing.
    #[napi]
    pub fn equals(&self, other: &JsFieldPath) -> bool {
        self.inner == other.inner
    }
}

/// One key and value a line declared, with whatever it nested.
#[napi(js_name = "TextEntry")]
pub struct JsTextEntry {
    inner: CoreTextEntry,
}

impl JsTextEntry {
    pub(crate) const fn from_core(inner: CoreTextEntry) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsTextEntry {
    /// The key, as text.
    #[napi(getter)]
    pub fn key(&self) -> String {
        self.inner.key().into_owned()
    }

    /// The value, as text.
    #[napi(getter)]
    pub fn value(&self) -> String {
        self.inner.value().into_owned()
    }

    /// The key as the bytes of its range, copied.
    #[napi(getter)]
    pub fn key_bytes(&self) -> Buffer {
        Buffer::from(self.inner.key_bytes().as_bytes())
    }

    /// The value as the bytes of its range, copied: what a reader working in
    /// offsets - a data field re-sliced to its stated length - reads.
    #[napi(getter)]
    pub fn value_bytes(&self) -> Buffer {
        Buffer::from(self.inner.value_bytes().as_bytes())
    }

    #[napi(getter, ts_return_type = "TextEntries | null")]
    pub fn entries(&self) -> Option<JsTextEntries> {
        self.inner.entries().cloned().map(JsTextEntries::from_core)
    }

    #[napi(js_name = "toString")]
    pub fn to_js_string(&self) -> String {
        self.inner.to_string()
    }
}

/// The ordered entries one line or one nested payload declared.
#[napi(js_name = "TextEntries")]
pub struct JsTextEntries {
    inner: CoreTextEntries,
}

impl JsTextEntries {
    pub(crate) const fn from_core(inner: CoreTextEntries) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsTextEntries {
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        u32::try_from(self.inner.len()).unwrap_or(u32::MAX)
    }

    /// One entry by position, counting back from the end when negative.
    #[napi(ts_return_type = "TextEntry | null")]
    pub fn at(&self, index: i32) -> Option<JsTextEntry> {
        let len = self.inner.len();
        let at = if index < 0 {
            len.checked_sub(index.unsigned_abs() as usize)?
        } else {
            usize::try_from(index).ok()?
        };
        self.inner
            .as_slice()
            .get(at)
            .cloned()
            .map(JsTextEntry::from_core)
    }

    /// Every entry, in the order the line declared them.
    #[napi(ts_return_type = "Array<TextEntry>")]
    pub fn to_array(&self) -> Vec<JsTextEntry> {
        self.inner
            .iter()
            .cloned()
            .map(JsTextEntry::from_core)
            .collect()
    }

    /// The entry a path reaches, or `null`.
    #[napi(ts_return_type = "TextEntry | null")]
    pub fn get_entry_by_path(
        &self,
        path: Either<String, &JsFieldPath>,
    ) -> Result<Option<JsTextEntry>> {
        let path = path_from_input(path)?;
        Ok(self
            .inner
            .get_entry_by_path(&path)
            .cloned()
            .map(JsTextEntry::from_core))
    }

    /// The entry a path reaches, raising absence.
    #[napi(ts_return_type = "TextEntry")]
    pub fn entry_by_path(&self, path: Either<String, &JsFieldPath>) -> Result<JsTextEntry> {
        let path = path_from_input(path)?;
        self.inner
            .entry_by_path(&path)
            .map(|held| JsTextEntry::from_core(held.clone()))
            .map_err(napi_error)
    }

    #[napi(js_name = "toString")]
    pub fn to_js_string(&self) -> String {
        self.inner.to_string()
    }
}

/// One decoded text row, typed the way its columns are.
#[napi(js_name = "TextLine")]
pub struct JsTextLine {
    inner: CoreTextLine,
}

impl JsTextLine {
    pub(crate) const fn from_core(inner: CoreTextLine) -> Self {
        Self { inner }
    }

    /// Borrow the line this wraps.
    pub(crate) const fn as_core(&self) -> &CoreTextLine {
        &self.inner
    }
}

#[napi]
impl JsTextLine {
    /// One line a caller holds itself, rather than one a text read answered.
    ///
    /// A capture is what a row header stated about the line, in the order the
    /// header declares them, and `null` is a capture it declared and this line
    /// did not match. The codec reads them by position, so the order is the
    /// contract and `FixCodec`'s `captureNames` is what names it.
    ///
    /// The body is copied into a page this line owns, once: every key and
    /// value a message read from it records is a range of that page. A
    /// `string` body is its UTF-8; a `Buffer` body that is not UTF-8 is
    /// decoded as the core decodes one, each invalid byte as its Windows-1252
    /// character, and `decodedByteSize` counts them.
    #[napi(
        constructor,
        ts_args_type = "index: number, body: string | Buffer, captures?: Array<string | null>"
    )]
    pub fn new(
        index: i64,
        body: Either<Buffer, String>,
        captures: Option<Vec<Option<String>>>,
    ) -> Result<Self> {
        let page = bytes_from_input(body)?;
        let mut line = CoreTextLine::from_bytes(index.unsigned_abs(), page).map_err(napi_error)?;
        if let Some(held) = captures {
            let mut read = Vec::with_capacity(held.len());
            for capture in held {
                read.push(match capture {
                    Some(text) => Some(TextBytes::from_bytes(text.as_bytes()).map_err(napi_error)?),
                    None => None,
                });
            }
            line.set_captures(read).map_err(napi_error)?;
        }
        Ok(Self::from_core(line))
    }

    /// The physical line number within the object, from zero.
    ///
    /// A `bigint`: a line count is 64 bits wide in the core and a JavaScript
    /// number cannot hold one without silently losing the top of it.
    #[napi(getter)]
    pub fn index(&self) -> i64 {
        i64::try_from(self.inner.index()).unwrap_or(i64::MAX)
    }

    /// The object this line was read from.
    #[napi(getter)]
    pub fn url(&self) -> Option<String> {
        self.inner.url().map(ToString::to_string)
    }

    /// When the record was written, in nanoseconds UTC.
    ///
    /// The core counts in 128 bits so a reading past what 64 bits hold has
    /// somewhere to land; this boundary answers `null` for one that does not
    /// fit rather than wrapping it.
    #[napi(getter)]
    pub fn timestamp(&self) -> Option<i64> {
        self.inner
            .timestamp()
            .and_then(|held| i64::try_from(held).ok())
    }

    /// What the line was classified as.
    #[napi(getter)]
    pub fn bodytype(&self) -> Option<String> {
        self.inner.bodytype().map(|held| held.as_str().to_owned())
    }

    /// The line, with whatever was read off its front removed.
    ///
    /// Text, always: what the constructor or the reader decoded.
    #[napi(getter)]
    pub fn body(&self) -> String {
        self.inner.body().to_owned()
    }

    /// How many bytes of the line as read were not UTF-8 and were decoded.
    ///
    /// Zero for a line that was text as read; the body's count and the
    /// captures' together.
    #[napi(getter)]
    pub fn decoded_byte_size(&self) -> i64 {
        i64::try_from(self.inner.decoded_byte_size()).unwrap_or(i64::MAX)
    }

    /// How many bytes of this record went over the retained limit.
    #[napi(getter)]
    pub fn dropped_byte_size(&self) -> Option<i64> {
        self.inner
            .dropped_byte_size()
            .and_then(|held| i64::try_from(held).ok())
    }

    /// The row header's named captures, in the order the expression declares
    /// them.
    #[napi(getter, ts_return_type = "Array<string | null>")]
    pub fn captures(&self) -> Vec<Option<String>> {
        (0..self.inner.captures().len())
            .map(|at| self.inner.capture(at).map(ToOwned::to_owned))
            .collect()
    }

    /// The key/value tree this line carries.
    #[napi(getter, ts_return_type = "TextEntries | null")]
    pub fn entries(&self) -> Option<JsTextEntries> {
        self.inner.entries().cloned().map(JsTextEntries::from_core)
    }

    /// The entry a path reaches, or `null`.
    #[napi(ts_return_type = "TextEntry | null")]
    pub fn get_entry_by_path(
        &self,
        path: Either<String, &JsFieldPath>,
    ) -> Result<Option<JsTextEntry>> {
        let path = path_from_input(path)?;
        Ok(self
            .inner
            .get_entry_by_path(&path)
            .cloned()
            .map(JsTextEntry::from_core))
    }

    /// The entry a path reaches, raising absence.
    #[napi(ts_return_type = "TextEntry")]
    pub fn entry_by_path(&self, path: Either<String, &JsFieldPath>) -> Result<JsTextEntry> {
        let path = path_from_input(path)?;
        self.inner
            .entry_by_path(&path)
            .map(|held| JsTextEntry::from_core(held.clone()))
            .map_err(napi_error)
    }

    /// Set the value a path reaches, creating what is not there.
    #[napi(ts_args_type = "path: string | FieldPath, value: string | Buffer")]
    pub fn set_entry_by_path(
        &mut self,
        path: Either<String, &JsFieldPath>,
        value: Either<Buffer, String>,
    ) -> Result<()> {
        let path = path_from_input(path)?;
        let value = bytes_from_input(value)?;
        self.inner
            .set_entry_by_path(&path, value)
            .map_err(napi_error)
    }

    /// Remove the entry a path reaches.
    #[napi(ts_return_type = "TextEntry | null")]
    pub fn remove_entry_by_path(
        &mut self,
        path: Either<String, &JsFieldPath>,
    ) -> Result<Option<JsTextEntry>> {
        let path = path_from_input(path)?;
        Ok(self
            .inner
            .remove_entry_by_path(&path)
            .map(JsTextEntry::from_core))
    }

    #[napi(js_name = "toString")]
    pub fn to_js_string(&self) -> String {
        self.inner.to_string()
    }
}

/// A lazy iterator over decoded lines.
///
/// Lines are pulled one at a time, never collected: a read larger than memory
/// iterates exactly as a reader would. A failing read raises once and stops.
#[napi(iterator, js_name = "TextLineIterator")]
pub struct JsTextLineIterator {
    inner: Option<CoreTextLines>,
}

impl JsTextLineIterator {
    pub(crate) const fn from_core(inner: CoreTextLines) -> Self {
        Self { inner: Some(inner) }
    }
}

impl Generator for JsTextLineIterator {
    type Yield = JsTextLine;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<Self::Next>) -> Option<Self::Yield> {
        let lines = self.inner.as_mut()?;
        if let Some(Ok(line)) = lines.next() {
            return Some(JsTextLine::from_core(line));
        }
        // A failing read fuses: the iterator stops rather than answering the
        // same failure forever.
        self.inner = None;
        None
    }
}
