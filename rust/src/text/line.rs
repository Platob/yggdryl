//! One text row: the line as the reader cut it, and every reading of it
//! resolved from the body under the options on first ask.

use std::collections::BTreeMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use smol_str::{SmolStr, format_smolstr};

use crate::graph::{Element, Event};
use crate::{DataType, FieldPath, MimeType, Result, Scalar, State, Str, Uri, Url, Uuid};

use super::arrow::{parse_capture, physical_rownum, row_error};
use super::options::{MTIME_COLUMN, TextOptions, mtime_dtype};
use super::{TextBytes, TextEntries, TextEntry};

/// One text row, and the [`Event`] it is.
///
/// A line holds what the reader cut and nothing it derived: its position in
/// the object, the object, the handle's own modification time, the body -
/// the whole line as cut, the row header included, made text where it is
/// made - and one `Arc<TextOptions>` every line of a read shares. Every
/// other fact is a reading of those held inputs under the options, resolved
/// on the first ask, once, into a slot of its own, so a caller that reads the
/// body and the row number resolves nothing else:
///
/// | reading | resolves as |
/// | --- | --- |
/// | [`captures`](Self::captures) | `rowheader` matched over the body, one slot per declared capture, in the order the expression declares them |
/// | [`entries`](Self::entries) | the key/value tree the body states, by [`TextEntries::from_bytes`] |
/// | [`bodytype`](Self::bodytype) | what the body is classified as |
/// | [`mtime`](Self::mtime) | the header's `mtime` capture as an instant, else the handle's modification time |
/// | `get_identifiers` | the named captures the line matched, each under the capture's name |
/// | `get_currunix` | [`mtime`](Self::mtime); the handle's time over a refused capture, the epoch where the line has none |
/// | `get_seqnum` | the row number under `start_rownum`, else the physical index |
/// | `get_state` | a `state` capture, else `00UNKNOWN` |
/// | `get_creaunix`, `get_execunix`, `get_recdunix`, `get_exprtime`, `get_prevunix`, `get_snapunix` | the capture of that name as an instant, else none |
/// | `get_prevuuid` | a `prevuuid` capture, else none |
/// | `get_crosscode` | the canonical text of the identifier the line was read under, else none |
/// | `get_currhashcode` | the XXH3-64 of the cross code, the row number and the body |
/// | `get_crosshashcode` | the cross code's XXH3-64, zero where none |
/// | `get_curruuid` | [`Event::time_uuid`] over the millisecond, sequence, code and cross-hash seed |
/// | `get_crossuuid` | [`Element::cross_uuid`] |
/// | `get_srcuuids` | none: a line is read from a handle |
///
/// The identity orders by millisecond and row sequence, then fingerprints the
/// code under the cross-hash seed. The code also tells two identical bodies
/// apart: the source they were read under and the row they sat on are digested
/// with the body, so two byte-identical lines of one handle that dates no row
/// still answer two identities, and one line read from two objects answers
/// two. A stated `crosshashcode` moves the UUID seed; a stated cross element
/// or source identity moves neither the content code nor current identity.
///
/// A capture named for an [`Event`] fact that the line does not derive feeds
/// that reading by its exact name, parsed at the fact's own datatype - an
/// instant at the `mtime` column's clock, a `uuid`, a `state` - and a capture
/// that does not parse as it is a named refusal on the inherent reading that
/// owns it and on every column built from it; the trait door, which cannot
/// refuse, answers the fact's default over a refused reading. `seqnum` and
/// `crosscode` are reserved because the row number and the identifier the
/// line was read under own them. A
/// `set_*` states a fact and wins over the resolved reading;
/// [`set_body`](Self::set_body) drops every resolved slot, because every one
/// of them read the body.
/// Equality, order and hash read the stated facts alone and never a
/// resolved slot.
///
/// The body is never empty, and it is text by construction. A line is the
/// line it holds, so a body stating nothing is refused wherever one is set -
/// [`from_bytes`](Self::from_bytes) and [`set_body`](Self::set_body) - and
/// the reader never offers one: a physical line that cut to nothing, a blank
/// line or one the strips took whole, is a separator and not a record. That
/// is what lets the `body` column a read answers be a column no null and no
/// empty cell reaches.
///
/// As for the text: a line is made from the bytes the reader cut, and where
/// those are not UTF-8 they are read once, where the line is made, by the
/// charset layer's one rule for bytes offered as UTF-8 that are not - the
/// rule behind
/// [`Charset::transcribe`](crate::Charset::transcribe) - so every reader
/// after that point reads text and none of them validates again. A body that
/// was UTF-8 - every line of every capture this crate holds - stays the range
/// of the page it was read into, so nothing is copied between the stream and
/// the line, and a capture is a range of that same page.
/// What a read was addressed by, as the one value it is.
///
/// A location is a *narrowing* of an identifier rather than a second fact
/// beside it, so where the read was addressed by one the line keeps that one
/// value and lends both readings off it: nothing can make them disagree, and
/// a read shares one reference-counted value across its rows instead of one
/// per reading.
///
/// A name is the case where the two really are two values, because where a
/// name is is not what it says: it resolves, by default under the process
/// working directory, and the line keeps that resolution beside the name so
/// a read under a name still answers where its bytes were. The cross code is
/// the identifier eitherway - the name, never where it resolved - so what a
/// line is crossed by does not move with the directory it was read from.
#[derive(Clone, Debug)]
pub(crate) enum LineSource {
    /// A location, which answers the identifier and the object alike.
    Located(Arc<Url>),
    /// A name, beside where it resolves to - nothing where it resolves
    /// nowhere.
    Named {
        /// The name the read was addressed by, and the cross code.
        uri: Arc<Uri>,
        /// Where that name is, resolved once for the whole read.
        at: Option<Arc<Url>>,
    },
}

impl LineSource {
    /// Narrow one identifier the way a handle is addressed by one: the
    /// location where it is one, the name itself where it is not.
    ///
    /// The caller's reference count is kept where the identifier is a name,
    /// because nothing else has to be built from it.
    fn shared(uri: Arc<Uri>) -> Self {
        match Url::from_uri(Uri::clone(&uri)) {
            Ok(url) => Self::Located(Arc::new(url)),
            Err(_) => Self::named(uri),
        }
    }

    /// The same narrowing from the identifier a handle lends, which a read
    /// does once and every row of it then shares.
    pub(crate) fn narrowed(uri: &Uri) -> Self {
        match Url::from_uri(uri.clone()) {
            Ok(url) => Self::Located(Arc::new(url)),
            Err(_) => Self::named(Arc::new(uri.clone())),
        }
    }

    /// A name beside where it resolves to, by [`Uri::locator`].
    ///
    /// Resolved here, once for a read, rather than at each ask: the
    /// resolution reads the process working directory, which is a syscall and
    /// an allocation no row should repeat, and where a read's bytes came from
    /// is settled when the read is. A name that resolves nowhere - an ARN
    /// naming a service no location serves, a URN with an empty part, a
    /// working directory the process cannot read - answers no location rather
    /// than refusing the read: the line still has its name, which is what it
    /// is crossed by.
    fn named(uri: Arc<Uri>) -> Self {
        let at = uri.locator().ok().map(Arc::new);
        Self::Named { uri, at }
    }

    /// The identifier a cross code spells, where it spells one.
    ///
    /// The code a row carries *is* the identifier the line was read under,
    /// so reading it back is reading that identifier: a location restores
    /// itself, and a name restores as the name it was, locating itself again
    /// where it resolves to now. Text carrying no scheme is deliberately not
    /// read here as a relative path, though `Uri` would read one: a code a
    /// caller stated is an ordinary code, and every one of them would
    /// otherwise come back naming a file.
    pub(crate) fn from_crosscode(text: &str) -> Option<Self> {
        if let Ok(url) = Url::from_str(text) {
            return Some(Self::Located(Arc::new(url)));
        }
        let name = crate::Urn::from_str(text)
            .map(crate::Urn::into_uri)
            .or_else(|_| crate::Arn::from_str(text).map(crate::Arn::into_uri))
            .ok()?;
        Some(Self::named(Arc::new(name)))
    }

    /// The identifier the read was addressed by, whichever narrowing it is.
    pub(crate) fn uri(&self) -> &Uri {
        match self {
            Self::Located(url) => <Url as AsRef<Uri>>::as_ref(url),
            Self::Named { uri, .. } => uri,
        }
    }

    /// The object: the identifier itself where it is a location, else where
    /// the name it is resolves to.
    pub(crate) fn url(&self) -> Option<&Url> {
        match self {
            Self::Located(url) => Some(url),
            Self::Named { at, .. } => at.as_deref(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TextLine {
    index: u64,
    source: Option<LineSource>,
    /// The handle's own modification time, nanoseconds since the Unix epoch,
    /// UTC: the instant a line its header does not date happened at.
    handle_mtime: Option<i64>,
    bodytype: Option<MimeType>,
    /// Text: [`decoded`] made it so, and every door onto this field goes
    /// through it.
    body: TextBytes,
    /// The options the line is read under, shared by every line of a read.
    options: Arc<TextOptions>,
    dropped_byte_size: Option<u64>,
    /// How many bytes of the body were decoded.
    decoded_body: u64,
    stated: Stated,
    resolved: Resolved,
}

/// One row-header match: where it ends in the body, and the captures it
/// named, one slot per declared capture.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Header {
    end: usize,
    captures: Vec<Option<TextBytes>>,
    /// Whether a caller stated the captures - the line's word, standing over
    /// every body - or the cut matched them over the body it read.
    stated: bool,
}

/// The facts a `set_*` stated, each winning over the resolved reading of
/// the same name; `None` leaves the reading to the body.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Stated {
    header: Option<Header>,
    entries: Option<Option<TextEntries>>,
    curruuid: Option<Uuid>,
    crossuuid: Option<Uuid>,
    crosscode: Option<Str>,
    currhashcode: Option<u64>,
    crosshashcode: Option<u64>,
    identifiers: Option<BTreeMap<String, String>>,
    srcuuids: Option<Vec<Uuid>>,
    currunix: Option<i64>,
    state: Option<State>,
    seqnum: Option<u64>,
    creaunix: Option<Option<i64>>,
    execunix: Option<Option<i64>>,
    recdunix: Option<Option<i64>>,
    exprtime: Option<Option<i64>>,
    prevunix: Option<Option<i64>>,
    prevuuid: Option<Option<Uuid>>,
    snapunix: Option<Option<i64>>,
}

/// One reading of a capture: the value it answers, and the refusal where
/// the capture did not parse as the fact it names - the value is then the
/// fact's default, which the trait door answers.
#[derive(Clone, Debug)]
struct Reading<T> {
    value: T,
    refused: Option<Refusal>,
}

impl<T> Reading<T> {
    const fn read(value: T) -> Self {
        Self {
            value,
            refused: None,
        }
    }

    fn refused(value: T, column: &str, reason: SmolStr) -> Self {
        Self {
            value,
            refused: Some(Refusal {
                column: SmolStr::new(column),
                reason,
            }),
        }
    }
}

/// A capture that does not read as the fact it names: the column and why.
#[derive(Clone, Debug)]
struct Refusal {
    column: SmolStr,
    reason: SmolStr,
}

/// Every reading of the body, each resolved once on the first ask.
#[derive(Clone, Debug, Default)]
struct Resolved {
    header: OnceLock<Header>,
    bodytype: OnceLock<MimeType>,
    entries: OnceLock<Option<TextEntries>>,
    identifiers: OnceLock<BTreeMap<String, String>>,
    mtime: OnceLock<Reading<Option<i64>>>,
    seqnum: OnceLock<Reading<u64>>,
    state: OnceLock<Reading<State>>,
    creaunix: OnceLock<Reading<Option<i64>>>,
    execunix: OnceLock<Reading<Option<i64>>>,
    recdunix: OnceLock<Reading<Option<i64>>>,
    exprtime: OnceLock<Reading<Option<i64>>>,
    prevunix: OnceLock<Reading<Option<i64>>>,
    snapunix: OnceLock<Reading<Option<i64>>>,
    prevuuid: OnceLock<Reading<Option<Uuid>>>,
    currhashcode: OnceLock<u64>,
    crosshashcode: OnceLock<u64>,
    curruuid: OnceLock<Uuid>,
    crossuuid: OnceLock<Uuid>,
}

impl Resolved {
    /// Drops the four derived identity readings, so they resolve afresh from
    /// the body, instant and cross code as they now are.
    fn reset_identity(&mut self) {
        self.currhashcode = OnceLock::new();
        self.crosshashcode = OnceLock::new();
        self.curruuid = OnceLock::new();
        self.crossuuid = OnceLock::new();
    }
}

impl TextLine {
    /// One line from its position in the object, the bytes the reader cut
    /// for it and the options it is read under.
    ///
    /// The bytes become text here. A body that is valid UTF-8 is kept as the
    /// range it is; one that is not is read once into a page of its own, as
    /// [`Charset::transcribe`](crate::Charset::transcribe) reads bytes offered
    /// as UTF-8 - every valid run kept, every other byte as Windows-1252
    /// through the charset layer's table - and how many bytes were read that
    /// way is what [`decoded_byte_size`] counts. Nothing else is read: the
    /// header, the entries, the instant and the identity resolve on their
    /// first ask.
    ///
    /// The row header is taken off here, once: the body is the line past
    /// what the header matched, and the captures it named are the line's.
    /// A line that is all header therefore has an empty body, and what it
    /// states is its captures - which is what a header that consumes the
    /// line is for.
    ///
    /// A line's body is the line, so an empty one is refused here - the
    /// bytes offered, before the header is taken off them: every reading
    /// below is a reading of the line, and a row stating nothing is a row
    /// nobody can read back. The reader never offers one - a physical line
    /// that cut to nothing is a separator and not a record - and a caller
    /// building a line by hand is told so by name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`](crate::Error::InvalidRecord) when the
    /// offered bytes are empty, and when the decoded text is longer than a
    /// page can address in 32-bit offsets.
    ///
    /// [`decoded_byte_size`]: Self::decoded_byte_size
    pub fn from_bytes(index: u64, body: TextBytes, options: Arc<TextOptions>) -> Result<Self> {
        let mut line = Self::cut(index, body, options)?;
        // The header is matched over the text this line is, because nothing
        // matched it for us: a caller's bytes are a line as they stand.
        let header = match_header(&line.options, &line.body);
        line.take_header(header)?;
        Ok(line)
    }

    /// The line as the bytes stand, with no header taken off yet.
    fn cut(index: u64, body: TextBytes, options: Arc<TextOptions>) -> Result<Self> {
        require_body(index, &body)?;
        let (body, decoded_body) = decoded(body)?;
        Ok(Self {
            index,
            source: None,
            handle_mtime: None,
            bodytype: None,
            body,
            options,
            dropped_byte_size: None,
            decoded_body,
            stated: Stated::default(),
            resolved: Resolved::default(),
        })
    }

    /// One line the reader cut, with the header it matched at the cut.
    ///
    /// The offsets the cut reports name the bytes it read. A body the decode
    /// rewrote is not those bytes, so such a line matches its own header
    /// over the text it is; every other line takes the cut's word and never
    /// matches twice.
    pub(super) fn from_cut(
        index: u64,
        body: TextBytes,
        options: Arc<TextOptions>,
        header: Option<(usize, Vec<Option<TextBytes>>)>,
    ) -> Result<Self> {
        let mut line = Self::cut(index, body, options)?;
        let matched = match header {
            Some((end, captures)) if line.decoded_body == 0 => Header {
                end,
                captures,
                stated: false,
            },
            _ => match_header(&line.options, &line.body),
        };
        line.take_header(matched)?;
        Ok(line)
    }

    /// One line rebuilt from a row, whose body is already past its header.
    ///
    /// Nothing is matched and nothing is taken off: a row states the line the
    /// read handed it, which is the body past the header, and the header's own
    /// captures are that row's columns - stated after this, where the batch
    /// carries them. Matching here would read a second header out of the
    /// payload and take it off a body that already lost one.
    pub(super) fn from_row(index: u64, body: TextBytes, options: Arc<TextOptions>) -> Result<Self> {
        let (body, decoded_body) = decoded(body)?;
        let mut line = Self {
            index,
            source: None,
            handle_mtime: None,
            bodytype: None,
            body,
            options,
            dropped_byte_size: None,
            decoded_body,
            stated: Stated::default(),
            resolved: Resolved::default(),
        };
        let captures = vec![None; line.options.capture_names().len()];
        line.stated.header = Some(Header {
            end: 0,
            captures,
            stated: false,
        });
        Ok(line)
    }

    /// Narrows the body past what `header` matched and states the captures.
    ///
    /// The header ends at zero once it is off the body, so no later reading
    /// looks for it again and no second match can find another one.
    fn take_header(&mut self, header: Header) -> Result<()> {
        let Header {
            end,
            captures,
            stated,
        } = header;
        if end > 0 {
            self.body = self.body.slice(end, self.body.len())?;
        }
        self.stated.header = Some(Header {
            end: 0,
            captures: decoded_captures(captures)?,
            stated,
        });
        Ok(())
    }

    /// The physical line number within the object, from zero.
    ///
    /// The row-number column and every row-located error need it, and no other
    /// field can recover it.
    #[must_use]
    pub const fn index(&self) -> u64 {
        self.index
    }

    /// Set the physical line number; the place in the chain and the current
    /// identity it answers resolve afresh.
    pub fn set_index(&mut self, index: u64) {
        self.index = index;
        self.resolved.seqnum = OnceLock::new();
        self.resolved.currhashcode = OnceLock::new();
        self.derive_uuids();
    }

    /// The options this line is read under: the row header, the strips, the
    /// first row number, the clock's zone.
    #[must_use]
    pub fn options(&self) -> &TextOptions {
        &self.options
    }

    /// The identifier this line was read under.
    ///
    /// What a handle *is* addressed by, which is not always a place: a read
    /// through a name or an ARN answers that name here, where
    /// [`sourceurl`](Self::sourceurl) answers where the name resolved to.
    /// This is what the line's cross code spells, so two reads of one body
    /// under two identifiers stay two elements - and a read under a name is
    /// crossed by the name however the directory it ran in resolved it.
    ///
    /// Shared rather than owned: every line of one handle carries the same
    /// identifier, and a URI is several small strings that would otherwise be
    /// rebuilt once per row.
    #[must_use]
    pub fn sourceuri(&self) -> Option<&Uri> {
        self.source.as_ref().map(LineSource::uri)
    }

    /// The object this line was read from.
    ///
    /// Where the identifier is a location this is that same value, read as
    /// the narrowing it is. Where it is a name, this is where the name
    /// resolves to - by [`Uri::locator`], which spells a URN as a path and
    /// roots it in the process working directory unless the name carries a
    /// base of its own - resolved once for the read rather than at each ask.
    /// A name that resolves nowhere answers nothing here and still answers
    /// itself at [`sourceuri`](Self::sourceuri).
    #[must_use]
    pub fn sourceurl(&self) -> Option<&Url> {
        self.source.as_ref().and_then(LineSource::url)
    }

    /// The object this line was read from, as the handle every line shares.
    ///
    /// [`sourceurl`](Self::sourceurl) is what a reader comparing or rendering one wants.
    /// A column that *holds* the URL wants this: a URL is several small
    /// strings, and a whole read answers one, so the column takes a
    /// reference count of the handle the read already built rather than
    /// rebuilding the strings once per row.
    #[must_use]
    pub const fn shared_url(&self) -> Option<&Arc<Url>> {
        match &self.source {
            Some(LineSource::Located(url)) => Some(url),
            Some(LineSource::Named { at, .. }) => at.as_ref(),
            None => None,
        }
    }

    /// Set or clear the identifier this line was read under.
    ///
    /// The identifier is narrowed once here rather than once per row, so
    /// [`sourceurl`](Self::sourceurl) answers it where it is a location and
    /// where it resolves to where it is a name. Refreshes the derived cross
    /// code,
    /// cross hash, current identity and cross identity. An explicitly stated
    /// event value continues to win.
    pub fn set_sourceuri(&mut self, uri: Option<Arc<Uri>>) {
        self.state_source(uri.map(LineSource::shared));
    }

    /// State the source a reader already narrowed, as the one value it is.
    ///
    /// A read narrows once and hands that value to every row it converts, so
    /// no row pays for a narrowing.
    pub(crate) fn state_source(&mut self, source: Option<LineSource>) {
        self.source = source;
        self.resolved.crosshashcode = OnceLock::new();
        self.resolved.currhashcode = OnceLock::new();
        self.derive_uuids();
    }

    /// Return this line addressed by one identifier.
    #[must_use]
    pub fn with_sourceuri(mut self, uri: Arc<Uri>) -> Self {
        self.set_sourceuri(Some(uri));
        self
    }

    /// The handle's own modification time, in nanoseconds since the Unix
    /// epoch, UTC: what dates a line whose header declares no `mtime`
    /// capture or did not match it.
    #[must_use]
    pub const fn handle_mtime(&self) -> Option<i64> {
        self.handle_mtime
    }

    /// Set or clear the handle's own modification time; the instant and the
    /// identity it dates resolve afresh.
    pub fn set_handle_mtime(&mut self, mtime: Option<i64>) {
        self.handle_mtime = mtime;
        self.resolved.mtime = OnceLock::new();
        self.derive_uuids();
    }

    /// Return this line dated by its handle.
    #[must_use]
    pub fn with_handle_mtime(mut self, mtime: i64) -> Self {
        self.set_handle_mtime(Some(mtime));
        self
    }

    /// What the line is classified as: the stated classification, else the
    /// payload's, read on the first ask.
    #[must_use]
    pub fn bodytype(&self) -> &MimeType {
        if let Some(stated) = &self.bodytype {
            return stated;
        }
        self.resolved.bodytype.get_or_init(|| {
            let (shape, _) = crate::mime_type::line::classify(self.body.as_bytes());
            shape
        })
    }

    /// State or unsay the classification; unsaid, it resolves afresh.
    pub fn set_bodytype(&mut self, bodytype: Option<MimeType>) {
        self.bodytype = bodytype;
    }

    /// Return this line classified.
    #[must_use]
    pub fn with_bodytype(mut self, bodytype: MimeType) -> Self {
        self.bodytype = Some(bodytype);
        self
    }

    /// The line past its row header: the edges stripped, the byte limit
    /// applied, and what the header matched already taken off.
    ///
    /// Empty exactly where the header consumed the line, whose captures are
    /// then what it states.
    ///
    /// Text, always: what [`from_bytes`](Self::from_bytes) decoded is what this answers.
    /// Readers that address the line by offset - the codec re-slicing a data
    /// field, the Arrow builder registering a page - take the same bytes as
    /// a range through [`body_bytes`](Self::body_bytes), which is free; this
    /// validates the range on the way out, once per call, so the per-line
    /// path does not ask it.
    #[must_use]
    pub fn body(&self) -> &str {
        std::str::from_utf8(self.body.as_bytes()).expect("a line's body is text by construction")
    }

    /// The body as the range of its page, for a reader that works in offsets.
    #[must_use]
    pub const fn body_bytes(&self) -> &TextBytes {
        &self.body
    }

    /// Replace the body, decoded exactly as [`from_bytes`](Self::from_bytes) decodes one;
    /// [`decoded_byte_size`](Self::decoded_byte_size) counts the new body,
    /// and every resolved reading is dropped, because every one of them read
    /// the body - the header the cut matched with them, since it was read
    /// over a body the line no longer has. What a `set_*` stated stands:
    /// stated captures are the line's word over every body, and the payload
    /// is then the whole of the new one.
    ///
    /// # Errors
    ///
    /// Returns the refusal [`from_bytes`](Self::from_bytes) does - an empty
    /// body included - leaving the line unchanged.
    pub fn set_body(&mut self, body: TextBytes) -> Result<()> {
        require_body(self.index, &body)?;
        let (body, decoded_body) = decoded(body)?;
        self.body = body;
        self.decoded_body = decoded_body;
        let stated = self.stated.header.take().filter(|header| header.stated);
        self.resolved = Resolved::default();
        match stated {
            // The line's word stands over every body, and the body is then
            // the whole of the new one.
            Some(header) => self.stated.header = Some(header),
            None => {
                let matched = match_header(&self.options, &self.body);
                self.take_header(matched)?;
            }
        }
        Ok(())
    }

    /// How many bytes of the line as read were not UTF-8 and were read as
    /// [`Charset::transcribe`](crate::Charset::transcribe) reads them.
    ///
    /// Zero for a line that was text as read. It is the one fact the decode
    /// keeps, so a reader auditing a capture can find the lines that were
    /// repaired without decoding them again. `0` under a declared charset
    /// for a line the byte limit did not cut inside a scalar: the transport
    /// read it as declared and the line repaired nothing - whether a
    /// resource was declared is the handle's fact, `MediaType::charset`,
    /// and not a per-line count. A limit that lands inside one scalar of
    /// the decoded text leaves the stray bytes the cut made, and the line
    /// reads and counts them exactly as on an undeclared read: `Zürich`
    /// declared `windows-1252` under a limit of `2` is the body `ZÃ` with
    /// `1` decoded.
    #[must_use]
    pub const fn decoded_byte_size(&self) -> u64 {
        self.decoded_body
    }

    /// How many bytes of this record went over the retained limit.
    #[must_use]
    pub const fn dropped_byte_size(&self) -> Option<u64> {
        self.dropped_byte_size
    }

    /// Set or clear the dropped byte count.
    pub const fn set_dropped_byte_size(&mut self, size: Option<u64>) {
        self.dropped_byte_size = size;
    }

    /// The row header's match: stated, else `rowheader` matched over the
    /// body on the first ask.
    fn header(&self) -> &Header {
        if let Some(stated) = &self.stated.header {
            return stated;
        }
        self.resolved
            .header
            .get_or_init(|| match_header(&self.options, &self.body))
    }

    /// The row header's named captures, in the order the expression declares
    /// them.
    ///
    /// Positional, not named: the expression fixes the order before the read,
    /// so a column reads its capture by position and never by a per-row name
    /// lookup. A capture the header declared but did not match on this line is
    /// `None`, which is the null its column holds; a header that declares
    /// none answers an empty list.
    ///
    /// Resolved on the first ask, once: the match over the body and the
    /// list it fills. Separate from the entries, because they are different
    /// facts with different owners: a capture is what the caller's expression
    /// asked for, an entry is what the line itself wrote down.
    #[must_use]
    pub fn captures(&self) -> &[Option<TextBytes>] {
        &self.header().captures
    }

    /// The capture at `index`, when the header declared and matched it.
    ///
    /// Text, a range of the body; a column reads its capture here and
    /// parses it at its own datatype.
    #[must_use]
    pub fn capture(&self, index: usize) -> Option<&str> {
        self.captures().get(index)?.as_ref()?.as_str()
    }

    /// The capture named `name`, when the header declares and matched it.
    fn capture_named(&self, name: &str) -> Option<&str> {
        let at = self.options.capture_names().position(|held| held == name)?;
        self.capture(at)
    }

    /// State the captures, each decoded exactly as the body is, and that
    /// the body carries no header: the payload is the whole body.
    ///
    /// # Errors
    ///
    /// Returns the refusal [`from_bytes`](Self::from_bytes) does, leaving the line
    /// unchanged.
    pub fn set_captures(&mut self, captures: Vec<Option<TextBytes>>) -> Result<()> {
        let header = Header {
            end: 0,
            captures: decoded_captures(captures)?,
            stated: true,
        };
        self.state_header(header);
        Ok(())
    }

    /// Return this line carrying captures, each decoded exactly as the body
    /// is.
    ///
    /// # Errors
    ///
    /// Returns the refusal [`from_bytes`](Self::from_bytes) does.
    pub fn with_captures(mut self, captures: Vec<Option<TextBytes>>) -> Result<Self> {
        self.set_captures(captures)?;
        Ok(self)
    }

    /// States one header match, dropping every reading that read past it.
    fn state_header(&mut self, header: Header) {
        self.stated.header = Some(header);
        self.resolved.header = OnceLock::new();
        self.resolved.entries = OnceLock::new();
        self.resolved.bodytype = OnceLock::new();
        self.resolved.identifiers = OnceLock::new();
        self.resolved.mtime = OnceLock::new();
        self.resolved.seqnum = OnceLock::new();
        self.resolved.state = OnceLock::new();
        self.resolved.creaunix = OnceLock::new();
        self.resolved.execunix = OnceLock::new();
        self.resolved.recdunix = OnceLock::new();
        self.resolved.exprtime = OnceLock::new();
        self.resolved.prevunix = OnceLock::new();
        self.resolved.snapunix = OnceLock::new();
        self.resolved.prevuuid = OnceLock::new();
        self.resolved.reset_identity();
    }

    /// The key/value tree this line carries past its header: the stated
    /// tree, else the body's own, resolved on the first ask by
    /// [`TextEntries::from_bytes`].
    ///
    /// `None` where the body states no pair. Materializing the tree is
    /// the one allocation past the header match, so it happens when a
    /// column reads an entry or a caller asks, and not otherwise.
    #[must_use]
    pub fn entries(&self) -> Option<&TextEntries> {
        if let Some(stated) = &self.stated.entries {
            return stated.as_ref();
        }
        self.resolved
            .entries
            .get_or_init(|| TextEntries::from_bytes(&self.body))
            .as_ref()
    }

    /// Borrow the tree for mutation, resolving it where nothing did yet.
    ///
    /// A tree handed out for mutation is the line's word from then on: it
    /// moves from the resolved slot to the stated fact, so a body set later
    /// leaves it standing.
    pub fn entries_mut(&mut self) -> Option<&mut TextEntries> {
        self.stated_entries_mut().as_mut()
    }

    /// The stated tree, for mutation: the resolved one moved into the stated
    /// fact where nothing was stated yet, so what is mutated is what stands.
    fn stated_entries_mut(&mut self) -> &mut Option<TextEntries> {
        if self.stated.entries.is_none() {
            let _ = self.entries();
            self.stated.entries = Some(self.resolved.entries.take().flatten());
        }
        self.stated
            .entries
            .as_mut()
            .expect("the tree was stated above")
    }

    /// State or clear the tree; stated, it stands over the body's own.
    pub fn set_entries(&mut self, entries: Option<TextEntries>) {
        self.stated.entries = Some(entries);
    }

    /// Return this line carrying a tree.
    #[must_use]
    pub fn with_entries(mut self, entries: TextEntries) -> Self {
        self.set_entries(Some(entries));
        self
    }

    /// The entry a path reaches.
    ///
    /// A miss is `None`: a path naming something this line did not carry is
    /// the ordinary case, and it becomes a null in the column that lifted it.
    #[must_use]
    pub fn get_entry_by_path(&self, path: &FieldPath) -> Option<&TextEntry> {
        self.entries()?.get_entry_by_path(path)
    }

    /// The entry a path reaches, raising absence.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidRecord`] naming the path when no entry is
    /// there.
    pub fn entry_by_path(&self, path: &FieldPath) -> Result<&TextEntry> {
        match self.entries() {
            Some(entries) => entries.entry_by_path(path),
            None => Err(crate::Error::InvalidRecord {
                path: format_smolstr!("$.entries.{path}"),
                reason: format_smolstr!("expected an entry at {path}, got a line carrying none"),
            }),
        }
    }

    /// The entry a path reaches, for mutation.
    pub fn get_entry_by_path_mut(&mut self, path: &FieldPath) -> Option<&mut TextEntry> {
        self.entries_mut()?.get_entry_by_path_mut(path)
    }

    /// Set the value a path reaches, creating what is not there.
    ///
    /// # Errors
    ///
    /// Returns the refusals [`TextEntries::set_entry_by_path`] states. Failure
    /// leaves this line unchanged.
    pub fn set_entry_by_path(&mut self, path: &FieldPath, value: TextBytes) -> Result<()> {
        self.stated_entries_mut()
            .get_or_insert_with(TextEntries::new)
            .set_entry_by_path(path, value)
    }

    /// Remove the entry a path reaches.
    pub fn remove_entry_by_path(&mut self, path: &FieldPath) -> Option<TextEntry> {
        self.entries_mut()?.remove_entry_by_path(path)
    }

    /// One reading's value, or the refusal it recorded, located on this
    /// line and the column that owns it.
    fn answer<'reading, T>(&self, reading: &'reading Reading<T>) -> Result<&'reading T> {
        match &reading.refused {
            None => Ok(&reading.value),
            Some(refusal) => Err(row_error(
                self.index,
                None,
                self.sourceurl(),
                &refusal.column,
                refusal.reason.clone(),
            )),
        }
    }

    /// The capture named `name` read as an instant at the `mtime` column's
    /// clock - nanoseconds UTC, a naive reading in the options' zone - or
    /// nothing where the header declares none or did not match it.
    fn instant_capture(&self, name: &str) -> Reading<Option<i64>> {
        let Some(text) = self.capture_named(name) else {
            return Reading::read(None);
        };
        match parse_capture(text, &mtime_dtype(), self.options.timezone()) {
            Ok(value) => Reading::read(value.temporal_count()),
            Err(reason) => Reading::refused(None, name, reason),
        }
    }

    /// When the record was written: the header's `mtime` capture, else the
    /// handle's own modification time, in nanoseconds since the Unix epoch,
    /// UTC; `None` where neither dates it. [`Event::set_currunix`] states
    /// the instant, and this answers it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`](crate::Error::InvalidRecord) naming
    /// the `mtime` column when the capture does not read as an instant.
    pub fn mtime(&self) -> Result<Option<i64>> {
        if let Some(stated) = self.stated.currunix {
            return Ok(Some(stated));
        }
        let reading = self.resolved.mtime.get_or_init(|| {
            let mut reading = self.instant_capture(MTIME_COLUMN);
            // A header that did not date the line falls back to the handle's
            // own time: a line the expression did not date is exactly the
            // case it is there for. A capture that did not read holds the
            // same fallback as its value, which the trait door - unable to
            // refuse - answers, while this reading refuses it by name.
            if reading.value.is_none() {
                reading.value = self.handle_mtime;
            }
            reading
        });
        self.answer(reading).copied()
    }

    /// Where the line stands in its source: the row number under
    /// `start_rownum`, else the zero-based physical line number.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`](crate::Error::InvalidRecord) naming
    /// `rownum` for a row number no count can hold.
    pub fn seqnum(&self) -> Result<u64> {
        if let Some(stated) = self.stated.seqnum {
            return Ok(stated);
        }
        let reading = self.resolved.seqnum.get_or_init(|| {
            match physical_rownum(self.options.start_rownum, self.index) {
                Ok(Some(rownum)) => match u64::try_from(rownum) {
                    Ok(count) => Reading::read(count),
                    Err(_) => Reading::refused(
                        self.index,
                        "seqnum",
                        format_smolstr!("expected a row number a count can hold, got {rownum}"),
                    ),
                },
                Ok(None) => Reading::read(self.index),
                // The row number's own refusal, under this reading's name:
                // the reason as it was stated, never a located refusal
                // located again.
                Err(error) => Reading::refused(
                    self.index,
                    "seqnum",
                    match &error {
                        crate::Error::InvalidRecord { reason, .. } => reason.clone(),
                        other => format_smolstr!("{}", super::elide_display(other)),
                    },
                ),
            }
        });
        self.answer(reading).copied()
    }

    /// Where the line stands in its lifecycle: a `state` capture, else
    /// `00UNKNOWN`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`](crate::Error::InvalidRecord) naming
    /// `state` when the capture names no state.
    pub fn state(&self) -> Result<&State> {
        if let Some(stated) = &self.stated.state {
            return Ok(stated);
        }
        let reading = self.resolved.state.get_or_init(|| {
            let Some(text) = self.capture_named("state") else {
                return Reading::read(State::unknown());
            };
            match State::read(text) {
                Ok(state) => Reading::read(state),
                Err(error) => Reading::refused(
                    State::unknown(),
                    "state",
                    format_smolstr!("{}", super::elide_display(&error)),
                ),
            }
        });
        self.answer(reading)
    }

    /// When the line's record was created: a `creaunix` capture, else none.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`](crate::Error::InvalidRecord) naming
    /// the capture when it does not read as an instant.
    pub fn creaunix(&self) -> Result<Option<i64>> {
        if let Some(stated) = self.stated.creaunix {
            return Ok(stated);
        }
        let reading = self
            .resolved
            .creaunix
            .get_or_init(|| self.instant_capture("creaunix"));
        self.answer(reading).copied()
    }

    /// When the line's event was executed: an `execunix` capture, else none.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`](crate::Error::InvalidRecord) naming
    /// the capture when it does not read as an instant.
    pub fn execunix(&self) -> Result<Option<i64>> {
        if let Some(stated) = self.stated.execunix {
            return Ok(stated);
        }
        let reading = self
            .resolved
            .execunix
            .get_or_init(|| self.instant_capture("execunix"));
        self.answer(reading).copied()
    }

    /// When the line's event was recorded: a `recdunix` capture, else none.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`](crate::Error::InvalidRecord) naming
    /// the capture when it does not read as an instant.
    pub fn recdunix(&self) -> Result<Option<i64>> {
        if let Some(stated) = self.stated.recdunix {
            return Ok(stated);
        }
        let reading = self
            .resolved
            .recdunix
            .get_or_init(|| self.instant_capture("recdunix"));
        self.answer(reading).copied()
    }

    /// When the line's record expires: an `exprtime` capture, else none.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`](crate::Error::InvalidRecord) naming
    /// the capture when it does not read as an instant.
    pub fn exprtime(&self) -> Result<Option<i64>> {
        if let Some(stated) = self.stated.exprtime {
            return Ok(stated);
        }
        let reading = self
            .resolved
            .exprtime
            .get_or_init(|| self.instant_capture("exprtime"));
        self.answer(reading).copied()
    }

    /// When the record this one follows happened: a `prevunix` capture,
    /// else none.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`](crate::Error::InvalidRecord) naming
    /// the capture when it does not read as an instant.
    pub fn prevunix(&self) -> Result<Option<i64>> {
        if let Some(stated) = self.stated.prevunix {
            return Ok(stated);
        }
        let reading = self
            .resolved
            .prevunix
            .get_or_init(|| self.instant_capture("prevunix"));
        self.answer(reading).copied()
    }

    /// The grid instant a walk read this line as the snapshot of: a
    /// `snapunix` capture, else none.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`](crate::Error::InvalidRecord) naming
    /// the capture when it does not read as an instant.
    pub fn snapunix(&self) -> Result<Option<i64>> {
        if let Some(stated) = self.stated.snapunix {
            return Ok(stated);
        }
        let reading = self
            .resolved
            .snapunix
            .get_or_init(|| self.instant_capture("snapunix"));
        self.answer(reading).copied()
    }

    /// The identity of the record this one follows: a `prevuuid` capture,
    /// else none.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`](crate::Error::InvalidRecord) naming
    /// the capture when it does not read as a UUID.
    pub fn prevuuid(&self) -> Result<Option<Uuid>> {
        if let Some(stated) = self.stated.prevuuid {
            return Ok(stated);
        }
        let reading = self.resolved.prevuuid.get_or_init(|| {
            let Some(text) = self.capture_named("prevuuid") else {
                return Reading::read(None);
            };
            match DataType::Uuid.scalar(Scalar::from(text)) {
                Ok(Scalar::Uuid(uuid)) => Reading::read(Some(uuid)),
                Ok(_) => Reading::read(None),
                Err(error) => Reading::refused(
                    None,
                    "prevuuid",
                    format_smolstr!("{}", super::elide_display(&error)),
                ),
            }
        });
        self.answer(reading).copied()
    }

    /// Drops what the identity derives - the code, the cross hash code, the
    /// identity and the cross element - stated or resolved, so each resolves
    /// afresh from the body, instant and cross code as they now are.
    fn derive_identity(&mut self) {
        self.stated.currhashcode = None;
        self.stated.crosshashcode = None;
        self.stated.curruuid = None;
        self.stated.crossuuid = None;
        self.resolved.reset_identity();
    }

    /// Drops generic identities so their next read derives from current inputs.
    fn derive_uuids(&mut self) {
        self.stated.curruuid = None;
        self.stated.crossuuid = None;
        self.resolved.curruuid = OnceLock::new();
        self.resolved.crossuuid = OnceLock::new();
    }

    /// Drops the *resolved* content code and the identities it derives, for a
    /// fact the code digests: the names the line goes by, its parents, its
    /// state, its place in the chain and what it follows.
    ///
    /// Resolved and never stated: a code a caller set or a row carried is a
    /// word, and a word stands until the one who said it takes it back -
    /// which is what [`Element::finalize`] is for. A batch restores the
    /// identity it stored before it restores the place, so dropping the
    /// stated code here would lose the identity a message named as its
    /// source on every line read back.
    fn derive_content(&mut self) {
        self.resolved.currhashcode = OnceLock::new();
        self.resolved.curruuid = OnceLock::new();
        self.resolved.crossuuid = OnceLock::new();
    }
}

/// The row header matched over the body: where the match ends and the
/// captures it named, one slot per declared capture, each a range of the
/// body's page; a header that declares nothing ends at zero with no slot,
/// and one that did not match ends at zero with every slot empty.
fn match_header(options: &TextOptions, body: &TextBytes) -> Header {
    let Some(rowheader) = options.rowheader_regex() else {
        return Header::default();
    };
    let count = options.capture_names().len();
    let Some(found) = rowheader.captures(body.as_bytes()) else {
        return Header {
            end: 0,
            captures: vec![None; count],
            stated: false,
        };
    };
    let end = found.get(0).map_or(0, |whole| whole.end());
    let mut captures: Vec<Option<TextBytes>> = vec![None; count];
    for (target, index) in captures.iter_mut().zip(
        rowheader
            .capture_names()
            .enumerate()
            .filter_map(|(index, name)| name.map(|_| index)),
    ) {
        let Some(matched) = found.get(index) else {
            continue;
        };
        // A range of a body that is text is text, but a byte class can cut a
        // character: such a capture is read as the body was, on its own
        // page. A capture is a range of a body that fits a page, so its own
        // reading fits one too.
        let held = body
            .slice(matched.start(), matched.end())
            .expect("a match is a range of the body");
        *target = decoded(held).map(|(held, _)| held).ok();
    }
    Header {
        end,
        captures,
        stated: false,
    }
}

/// Captures a caller stated, each decoded exactly as a body is.
fn decoded_captures(mut captures: Vec<Option<TextBytes>>) -> Result<Vec<Option<TextBytes>>> {
    // Read where they stand: a capture that was already text is the range
    // it was, so a second vector would allocate once per line to hold what
    // this one already holds. A refusal drops the vector that came in and
    // leaves the line untouched, which is the stated contract.
    for capture in &mut captures {
        let Some(held) = capture.take() else { continue };
        let (held, _) = decoded(held)?;
        *capture = Some(held);
    }
    Ok(captures)
}

impl TextLine {
    /// The identifier's shared canonical text, else the stated cross code.
    ///
    /// The identifier wins: a line's chain is what its read was addressed
    /// by, and that is what makes it a fact of the row rather than a column
    /// beside it. It is the identifier and never the location it resolves
    /// to, so a line read under a name is crossed by that name - a read
    /// through a name reaches the code at all, and the code does not move
    /// with the directory the read ran in. A code a caller states names the
    /// chain of a line nothing addressed - a buffer, a line built by hand -
    /// and nothing else.
    fn crosscode_value(&self) -> Option<&Str> {
        if let Some(source) = self.source.as_ref() {
            return Some(source.uri().shared_text());
        }
        self.stated.crosscode.as_ref()
    }

    /// What the line states under one of the event columns, as the
    /// column's cell, or nothing where it states no fact.
    ///
    /// The facts a capture feeds - the instant, the state, the place, the
    /// optional event instants, the element it follows - are read through
    /// the line's own readings, so a capture the fact's type cannot read
    /// is refused by the fact's name rather than answered as the default
    /// the trait doors fall back to.
    ///
    /// # Errors
    ///
    /// Returns the reading's refusal for a capture that does not parse at
    /// the fact's datatype.
    pub fn event_fact(&self, column: crate::graph::EventColumn) -> Result<Option<Scalar>> {
        use crate::graph::EventColumn;
        match column {
            // The generic event projection starts from `&str`, which would
            // allocate a new long string value for every row. This line owns
            // the shared value already, so its Arrow cell is a cheap clone.
            EventColumn::CrossCode => {
                return Ok(self
                    .crosscode_value()
                    .filter(|code| !code.is_empty())
                    .map(|code| Scalar::Utf8String(code.clone())));
            }
            // The instant is the `mtime` capture where one reads as an
            // instant, else the handle's, whatever the column flag says; the
            // reading is asked to refuse a capture that does not read only
            // where the capture is the column's - with the column off, the
            // capture is an ordinary one typed by its own syntax, and the
            // trait door answers the handle's time over it.
            EventColumn::CurrUnix if self.options.parse_mtime => {
                self.mtime()?;
            }
            EventColumn::State => {
                self.state()?;
            }
            EventColumn::SeqNum => {
                self.seqnum()?;
            }
            EventColumn::CreaUnix => {
                self.creaunix()?;
            }
            EventColumn::ExecUnix => {
                self.execunix()?;
            }
            EventColumn::RecdUnix => {
                self.recdunix()?;
            }
            EventColumn::ExprTime => {
                self.exprtime()?;
            }
            EventColumn::PrevUnix => {
                self.prevunix()?;
            }
            EventColumn::SnapUnix => {
                self.snapunix()?;
            }
            EventColumn::PrevUuid => {
                self.prevuuid()?;
            }
            _ => {}
        }
        Ok(column.fact(self))
    }
}

impl Element for TextLine {
    fn get_curruuid(&self) -> Uuid {
        if let Some(stated) = self.stated.curruuid {
            return stated;
        }
        // The UUIDv7 the millisecond, sequence, line code and cross hash
        // derive; an instant its 48-bit timestamp cannot hold is the nil
        // identity, never a truncated one.
        *self
            .resolved
            .curruuid
            .get_or_init(|| self.time_uuid().unwrap_or_default())
    }

    fn set_curruuid(&mut self, curruuid: Uuid) {
        self.stated.curruuid = Some(curruuid);
        self.resolved.crossuuid = OnceLock::new();
    }

    fn get_crossuuid(&self) -> Uuid {
        if let Some(stated) = self.stated.crossuuid {
            return stated;
        }
        *self.resolved.crossuuid.get_or_init(|| self.cross_uuid())
    }

    fn set_crossuuid(&mut self, crossuuid: Uuid) {
        self.stated.crossuuid = Some(crossuuid);
    }

    fn get_crosscode(&self) -> &str {
        self.crosscode_value().map(Str::as_str).unwrap_or_default()
    }

    fn set_crosscode(&mut self, crosscode: String) {
        self.stated.crosscode = Some(Str::from(crosscode));
        // The cross hash and both identities derive from this code. A line
        // restored from Arrow states those columns explicitly, so dropping
        // only their resolved slots would leave the restored values stale.
        self.stated.crosshashcode = None;
        self.resolved.crosshashcode = OnceLock::new();
        self.resolved.currhashcode = OnceLock::new();
        self.derive_uuids();
    }

    /// The XXH3-64 of what the line states: the facts every event digests -
    /// the names it goes by, its parents, its state, its place and what it
    /// follows - and then the body, behind them.
    ///
    /// The names a line goes by are its row header's captures, so a header
    /// that lifts a level, an id or a symbol out of a line puts them in the
    /// code: two lines whose bodies match but whose headers do not are two
    /// events. The one capture left out is the one that dates the line,
    /// because `currunix` is coupled with this code rather than fed into it -
    /// the crate's time-ordered identity is the pair, and feeding the instant
    /// here would state it twice.
    fn get_currhashcode(&self) -> u64 {
        if let Some(stated) = self.stated.currhashcode {
            return stated;
        }
        *self.resolved.currhashcode.get_or_init(|| {
            let mut state = crate::xxhash::Xxh3::new();
            // The cross code is fed here, ahead of the facts that leave it
            // out, in the order `digest_event` feeds it. The UUID also seeds
            // its payload with the cross hash, but the content code remains a
            // complete statement of the line on its own.
            let crosscode = self.get_crosscode();
            if !crosscode.is_empty() {
                crate::graph::element::feed(&mut state, "crosscode", crosscode.as_bytes());
            }
            crate::graph::element::feed_event_facts(&mut state, self, |name| name != MTIME_COLUMN);
            state.write(self.body.as_bytes());
            state.as_u64()
        })
    }

    fn set_currhashcode(&mut self, hashcode: u64) {
        self.stated.currhashcode = Some(hashcode);
        self.derive_uuids();
    }

    fn get_crosshashcode(&self) -> u64 {
        if let Some(stated) = self.stated.crosshashcode {
            return stated;
        }
        *self.resolved.crosshashcode.get_or_init(|| {
            let crosscode = self.get_crosscode();
            if crosscode.is_empty() {
                0
            } else {
                crate::graph::element::crosshash(crosscode)
            }
        })
    }

    fn set_crosshashcode(&mut self, crosshashcode: u64) {
        self.stated.crosshashcode = Some(crosshashcode);
        self.derive_uuids();
    }

    fn get_identifiers(&self) -> &BTreeMap<String, String> {
        if let Some(stated) = &self.stated.identifiers {
            return stated;
        }
        self.resolved.identifiers.get_or_init(|| {
            self.options
                .capture_names()
                .enumerate()
                .filter_map(|(at, name)| Some((name.to_owned(), self.capture(at)?.to_owned())))
                .collect()
        })
    }

    fn set_identifiers(&mut self, identifiers: BTreeMap<String, String>) {
        self.stated.identifiers = Some(identifiers);
        self.derive_content();
    }

    fn get_srcuuids(&self) -> &[Uuid] {
        self.stated.srcuuids.as_deref().unwrap_or_default()
    }

    fn set_srcuuids(&mut self, mut sources: Vec<Uuid>) {
        crate::graph::element::canonicalize_uuids(&mut sources);
        self.stated.srcuuids = Some(sources);
    }

    /// A line's order is its instant, then its place in the object: two
    /// lines of one handle the header did not date stand in the order they
    /// were written.
    fn is_after(&self, other: &Self) -> bool {
        (self.get_currunix(), self.index) > (other.get_currunix(), other.index)
    }

    /// The identity is what the instant and the line's code derive: the code,
    /// cross hash code, identity and cross element resolve afresh on their
    /// next ask.
    fn finalize(&mut self) {
        self.derive_identity();
    }

    /// The timed reading: a line follows a line as any event follows one.
    fn with_previous(self, previous: &Self) -> Option<Self> {
        self.following(previous)
    }

    fn merge_with(self, other: &Self) -> Option<Self> {
        self.merging(other)
    }
}

impl Event for TextLine {
    /// When the record was written, [`TextLine::mtime`]; the handle's own
    /// time over a refused capture, and the epoch where the line has no
    /// instant at all.
    fn get_currunix(&self) -> i64 {
        if let Some(stated) = self.stated.currunix {
            return stated;
        }
        let _ = self.mtime();
        self.resolved
            .mtime
            .get()
            .and_then(|reading| reading.value)
            .unwrap_or(0)
    }

    fn set_currunix(&mut self, unix: i64) {
        self.stated.currunix = Some(unix);
        self.derive_uuids();
    }

    /// [`TextLine::state`]; `00UNKNOWN` over a refused capture.
    fn get_state(&self) -> &State {
        if let Some(stated) = &self.stated.state {
            return stated;
        }
        let _ = self.state();
        &self
            .resolved
            .state
            .get()
            .expect("the state was resolved above")
            .value
    }

    fn set_state(&mut self, state: State) {
        self.stated.state = Some(state);
        self.derive_content();
    }

    /// [`TextLine::seqnum`]; the physical line number over a refused row
    /// number.
    fn get_seqnum(&self) -> u64 {
        self.seqnum().unwrap_or(self.index)
    }

    fn set_seqnum(&mut self, seqnum: u64) {
        self.stated.seqnum = Some(seqnum);
        self.resolved.currhashcode = OnceLock::new();
        self.derive_uuids();
    }

    /// [`TextLine::creaunix`]; none over a refused capture.
    fn get_creaunix(&self) -> Option<i64> {
        self.creaunix().ok().flatten()
    }

    fn set_creaunix(&mut self, unix: Option<i64>) {
        self.stated.creaunix = Some(unix);
    }

    /// [`TextLine::execunix`]; none over a refused capture.
    fn get_execunix(&self) -> Option<i64> {
        self.execunix().ok().flatten()
    }

    fn set_execunix(&mut self, unix: Option<i64>) {
        self.stated.execunix = Some(unix);
    }

    /// [`TextLine::recdunix`]; none over a refused capture.
    fn get_recdunix(&self) -> Option<i64> {
        self.recdunix().ok().flatten()
    }

    fn set_recdunix(&mut self, unix: Option<i64>) {
        self.stated.recdunix = Some(unix);
    }

    /// [`TextLine::exprtime`]; none over a refused capture.
    fn get_exprtime(&self) -> Option<i64> {
        self.exprtime().ok().flatten()
    }

    fn set_exprtime(&mut self, unix: Option<i64>) {
        self.stated.exprtime = Some(unix);
    }

    /// [`TextLine::prevunix`]; none over a refused capture.
    fn get_prevunix(&self) -> Option<i64> {
        self.prevunix().ok().flatten()
    }

    fn set_prevunix(&mut self, unix: Option<i64>) {
        self.stated.prevunix = Some(unix);
    }

    /// [`TextLine::prevuuid`]; none over a refused capture.
    fn get_prevuuid(&self) -> Option<Uuid> {
        self.prevuuid().ok().flatten()
    }

    fn set_prevuuid(&mut self, uuid: Option<Uuid>) {
        self.stated.prevuuid = Some(uuid);
        self.derive_content();
    }

    /// [`TextLine::snapunix`]; none over a refused capture.
    fn get_snapunix(&self) -> Option<i64> {
        self.snapunix().ok().flatten()
    }

    fn set_snapunix(&mut self, unix: Option<i64>) {
        self.stated.snapunix = Some(unix);
    }
}

/// The facts a line states, which is what it is compared, ordered and
/// hashed by: what the reader cut and what a `set_*` stated, and never a
/// resolved slot. The options stand beside them: every line of one read
/// shares one `Arc`, so they are compared by pointer first and by value
/// only where the pointers differ, ordered last, and left out of the hash -
/// equal lines hash equal without them, and a hash that walked them would
/// pay for the one shared value once per line.
type Facts<'line> = (
    u64,
    Option<&'line Uri>,
    Option<i64>,
    Option<&'line MimeType>,
    &'line TextBytes,
    Option<u64>,
    u64,
    &'line Stated,
);

impl TextLine {
    fn facts(&self) -> Facts<'_> {
        (
            self.index,
            self.sourceuri(),
            self.handle_mtime,
            self.bodytype.as_ref(),
            &self.body,
            self.dropped_byte_size,
            self.decoded_body,
            &self.stated,
        )
    }

    /// Whether two lines read themselves by the same options: the one
    /// `Arc` a read shares, else the same value.
    fn same_options(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.options, &other.options) || self.options == other.options
    }
}

impl PartialEq for TextLine {
    fn eq(&self, other: &Self) -> bool {
        self.facts() == other.facts() && self.same_options(other)
    }
}

impl Eq for TextLine {}

impl PartialOrd for TextLine {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TextLine {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.facts().cmp(&other.facts()).then_with(|| {
            if self.same_options(other) {
                std::cmp::Ordering::Equal
            } else {
                self.options.cmp(&other.options)
            }
        })
    }
}

impl Hash for TextLine {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.facts().hash(state);
    }
}

impl fmt::Display for TextLine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.body())
    }
}

impl From<TextLine> for Result<TextLine> {
    fn from(value: TextLine) -> Self {
        Ok(value)
    }
}

impl<'a> From<&'a TextLine> for Result<&'a TextLine> {
    fn from(value: &'a TextLine) -> Self {
        Ok(value)
    }
}

/// Refuse a body that states nothing, naming the line it was offered for.
///
/// The one place the invariant is stated: every door that puts bytes in a
/// line goes through it, so `body()` is text a reader can read and the
/// `body` column a read answers is not nullable because it cannot be.
fn require_body(index: u64, body: &TextBytes) -> Result<()> {
    if body.is_empty() {
        return Err(crate::Error::InvalidRecord {
            path: format_smolstr!("$[{index}].body"),
            reason: SmolStr::new_static("expected a line body, got an empty one"),
        });
    }
    Ok(())
}

/// The bytes as text, and how many of them had to be decoded to be so.
///
/// Valid UTF-8 costs nothing: the range is answered as it is, and `0`. Any
/// other input is read once into a page of its own by the charset layer's one
/// rule for bytes offered as UTF-8 that are not, the rule behind
/// [`Charset::transcribe`](crate::Charset::transcribe): every valid run kept
/// as it is, and every other byte read as the character the layer's generated
/// `windows-1252` table gives it, per invalid run rather than per line. The
/// table, its rule for the five bytes that table leaves unassigned and the
/// reason the reading is per run are the layer's and are stated there once;
/// the count is what that reading answers. What is the line's is that it
/// never refuses: a run a truncation cut inside a character is invalid too,
/// and its orphan bytes read as the characters they are rather than as
/// `U+FFFD`, because a byte the wire held is a fact and a replacement
/// character is the absence of one.
///
/// # Errors
///
/// Returns [`Error::InvalidRecord`](crate::Error::InvalidRecord) when the
/// decoded text - up to three bytes per byte decoded - is longer than a page
/// can address in 32-bit offsets.
pub(crate) fn decoded(bytes: TextBytes) -> Result<(TextBytes, u64)> {
    let held = bytes.as_bytes();
    if std::str::from_utf8(held).is_ok() {
        return Ok((bytes, 0));
    }
    // Sized here rather than left to the reading's own reservation, which is
    // the input length: a floor every stray byte overruns by one or two
    // bytes, so a `String::new()` would grow once at the first of them. The
    // line already knows it holds at least one; sixteen bytes of slack keep a
    // line with a handful at the one allocation a text line costs.
    let mut text = String::with_capacity(held.len() + 16);
    let count = crate::utf8::transcribe_into(held, &mut text) as u64;
    let page = TextBytes::from_whole_page(Arc::new(text.into_bytes()))?;
    Ok((page, count))
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/text/line.rs` pins and a caller cannot reach.
    //!
    //! A line's text is decoded once, on the page it was read into, and what
    //! that decode counted is what
    //! [`TextLine::decoded_byte_size`](crate::text::TextLine::decoded_byte_size)
    //! answers.
    //! The step itself is private, so this forwards to it over the public
    //! [`TextBytes`] a caller already holds.

    use crate::Result;
    use crate::text::TextBytes;

    /// Decode one page of read bytes, answering the text and how many of them
    /// were not UTF-8.
    pub fn decoded(bytes: TextBytes) -> Result<(TextBytes, u64)> {
        super::decoded(bytes)
    }
}
