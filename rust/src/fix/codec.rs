//! What every line of one capture is read against.
//!
//! A [`FixCodec`] is the dictionary plus the few facts a whole run shares -
//! the dialect, the version, the spellings that mean nothing was sent - and
//! the constructors on [`FixMsg`] take one. Each of them splits its own
//! dialect, rewrites it into the key forms the one [builder](super::build)
//! understands, and hands the pairs over. None of them parses a message and
//! none of them builds a tree of its own.
//!
//! # Why `transform_*` and not `read_*`
//!
//! `read_*` names an I/O operation in this crate: it asks a handle for bytes
//! and names the core type it answers, as `read_all_bytes` and
//! `read_arrow_reader` do. A codec asks nothing of storage. It is handed
//! bytes, a record or a batch the caller already holds and answers the same
//! rows in the message vocabulary, so the family is named for what it does to
//! its input rather than for where the input came from. The two families then
//! compose without either shadowing the other: a handle's `read_arrow_reader`
//! feeds this codec's [`FixCodec::transform_arrow_reader`].
//!
//! # A row is a log line
//!
//! A capture line is a message wrapped in whatever the process printed around
//! it - `sending >> 8=FIX.4.2|…|10=203| << queued seq=1092` - so the frame is
//! located first and everything before it is prefix. Reading from byte zero
//! would take `sending >> 8` as the first key and pick the wrong dialect on
//! nearly every real line.
//!
//! The dialect is decided once, from the frame, and never re-sniffed: all
//! ASCII digits before the frame's first `=` means numeric FIX, anything else
//! means a bridge row. Once decided it holds for the whole body, so a `#` or a
//! `<` inside a *value* is part of that value.
//!
//! # Nothing is skipped
//!
//! A row with no message type is built anyway and named `unknown`; a value
//! that will not type is null; a group that will not split stays whole. What
//! is left - input that is not a row at all - is an `Err` item carrying it,
//! and the stream continues, because one corrupt line must not end a run over
//! ten million.

use std::borrow::Cow;
use std::sync::Arc;

use quick_xml::events::Event;
use smol_str::SmolStr;

use crate::mime_type::line;
use crate::{DataType, Error, Field, Result, Scalar, Version};

use super::build::{Builder, root_name};
use super::{FixBranch, FixMessages, FixMsg, FixRegistry};

/// What separates the members packed inside one bridge group occurrence.
///
/// ULLINK writes EOT then ETX. A bridge relaying into a FIX session writes the
/// protocol's own SOH instead, which is unambiguous inside an occurrence
/// because no FIX value may contain one.
const MEMBER_SEPARATORS: [&[u8]; 2] = [b"\x04\x03", b"\x01"];

/// The spellings that mean "nothing was sent", by default.
///
/// A bridge with nothing to say writes one of these, and a reader that keeps
/// them puts the four characters `null` into a column whose answer is no
/// answer. The match is on the raw bytes after the whitespace trim, compared
/// case-insensitively as ASCII - never through the crate's fold, which serves
/// names and code spellings and would match spellings nobody wrote.
pub const DEFAULT_NULL_VALUES: [&str; 3] = ["", "null", "<null>"];

/// One capture read row by row, holding what is constant across them.
///
/// A capture is millions of lines and calling a singular reader per line
/// re-does per message what is constant for the whole run. Pinning a branch
/// and the two versions skips inference for every row - and a capture is one
/// session, so pinning is the normal case rather than an optimization.
#[derive(Clone)]
pub struct FixCodec {
    registry: Arc<FixRegistry>,
    branch: Option<FixBranch>,
    version: Option<Version>,
    separator: Option<u8>,
    payload_column: SmolStr,
    null_values: Vec<String>,
}

impl FixCodec {
    /// Borrows the message type declared by a captured line without parsing a message.
    #[must_use]
    pub fn infer_msgtype_bytes(line: &[u8]) -> Option<&[u8]> {
        line::inspect(line).msgtype()
    }

    /// Borrows the message type declared by a captured text line.
    #[must_use]
    pub fn infer_msgtype_text(line: &str) -> Option<&str> {
        std::str::from_utf8(Self::infer_msgtype_bytes(line.as_bytes())?).ok()
    }

    /// Opens a codec over one dictionary.
    #[must_use]
    pub fn new(registry: Arc<FixRegistry>) -> Self {
        Self {
            registry,
            branch: None,
            version: None,
            separator: None,
            payload_column: SmolStr::new_static(super::record::DEFAULT_PAYLOAD_COLUMN),
            null_values: DEFAULT_NULL_VALUES
                .iter()
                .map(|spelling| (*spelling).to_owned())
                .collect(),
        }
    }

    /// The dictionary every message is read against.
    #[must_use]
    pub const fn registry(&self) -> &Arc<FixRegistry> {
        &self.registry
    }

    /// The version messages are built at, where the caller pinned one.
    #[must_use]
    pub const fn version(&self) -> Option<Version> {
        self.version
    }

    /// The dialect every message is read under, where the caller pinned one.
    #[must_use]
    pub const fn branch(&self) -> Option<&FixBranch> {
        self.branch.as_ref()
    }

    /// Pins the dialect, so no row is read under the standard one.
    #[must_use]
    pub fn with_branch(mut self, branch: &FixBranch) -> Self {
        self.branch = Some(branch.clone());
        self
    }

    /// Pins the version the built messages are read at.
    ///
    /// A value is translated through the code spellings that version declares,
    /// and nothing else changes: a tag is one column under the name the
    /// dictionary holds it by, whatever version read it. Unpinned, each row
    /// answers for itself:
    /// `ApplVerID(1128)` first, then `BeginString(8)`, then the dialect's own
    /// default, then the dictionary's newest - which is what a capture
    /// carrying more than one application version needs.
    #[must_use]
    pub const fn with_version(mut self, version: Version) -> Self {
        self.version = Some(version);
        self
    }

    /// Pins the byte a numeric frame is split on, so none is inferred.
    #[must_use]
    pub const fn with_separator(mut self, separator: u8) -> Self {
        self.separator = Some(separator);
        self
    }

    /// Names the record column [`Self::transform_record`] reads the payload from.
    #[must_use]
    pub fn with_payload_column(mut self, column: impl Into<SmolStr>) -> Self {
        self.payload_column = column.into();
        self
    }

    /// Replaces the spellings that mean "nothing was sent".
    ///
    ///
    /// Empty keeps every literal, which is what a venue for whom the text
    /// `null` is a value sets: the default is a convention, and a convention
    /// has to be overridable to be safe.
    #[must_use]
    pub fn with_null_values<I, S>(mut self, spellings: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.null_values = spellings.into_iter().map(Into::into).collect();
        self
    }

    /// Whether one raw value is a stated absence rather than a value.
    fn is_absent(&self, value: &[u8]) -> bool {
        let trimmed = line::trim_ascii(value);
        self.null_values
            .iter()
            .any(|spelling| spelling.as_bytes().eq_ignore_ascii_case(trimmed))
    }

    /// Transforms one log line into an iterator of messages, selecting the
    /// dialect from its frame. A bulk UL configuration yields one message
    /// per selected MBean; the other dialects yield one message.
    ///
    /// The prefix a process printed around the message is located and dropped
    /// first, then one shallow look decides which reader owns
    /// the body: a run of digits before the frame's first `=` is numeric FIX,
    /// and anything else is a bridge row. The decision is made once and holds
    /// for the whole body, so a `#` or a `<` inside a *value* is part of that
    /// value.
    ///
    /// A FIXML row is the one that states no `key=value` frame at all, so the
    /// locator finds nothing to read; a row holding a tag is read as the
    /// document it is rather than as a line that held nothing.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] for input that is not a row at all.
    pub fn transform_line(&self, row: &[u8], enrich: bool) -> Result<FixMessages> {
        if row.is_empty() {
            return Err(Error::Parse {
                target: "fix",
                position: 0,
                reason: "expected a captured row, got no bytes".into(),
            });
        }
        let start = line::payload_at(row).unwrap_or(row.len());
        let body = &row[start..];
        if numeric_frame(body) {
            return self.transform_fix_line(body, enrich).map(FixMessages::one);
        }
        // A payload opening with `{` is a bridge configuration document, and
        // nothing else is: the locator points at a key, which starts with a
        // digit or a letter, and points at an object only where it found one.
        // So the test costs one byte rather than a second classification.
        if matches!(body.first(), Some(b'{' | b'[')) {
            return self.transform_ulconfig_line(body, enrich);
        }
        // A FIXML row states no `key=value` frame, so the locator finds none
        // and leaves nothing to read. The document is the payload, and it
        // opens at the first tag - which is also how a prefix is dropped from
        // one, since everything before that tag is text the reader skips.
        if body.is_empty() && memchr::memchr(b'<', row).is_some() {
            return self.transform_fixml_line(row, enrich).map(FixMessages::one);
        }
        self.transform_ullink_line(body, enrich)
            .map(FixMessages::one)
    }

    /// Transforms one numeric FIX frame.
    ///
    /// The separator is the one [`Self::with_separator`] pinned; with none
    /// pinned a frame spelling its `SOH` as `\x01`, `^A` or `<SOH>` is
    /// unescaped and split on the real byte, and any other frame is split on
    /// whichever of `|`, `;` or `SOH` it actually uses.
    ///
    /// A trailing empty segment is tolerated, because a wire message ends
    /// with its separator; a segment with no `=` is dropped, as an empty key
    /// is; duplicate tags stay in arrival order. Every key and value is a
    /// slice of the input and none of it is validated as UTF-8.
    ///
    /// # Errors
    ///
    /// Returns the builder's refusal, which a row's content cannot provoke.
    pub fn transform_fix_line(&self, body: &[u8], enrich: bool) -> Result<FixMsg> {
        if let Some(separator) = self.separator {
            return self.split_fix(body, separator, enrich);
        }
        match unescaped(body) {
            Some(held) => self.split_fix(&held, 0x01, enrich),
            None => self.split_fix(body, separator_of(body), enrich),
        }
    }

    /// One numeric frame split on one byte.
    fn split_fix(&self, body: &[u8], separator: u8, enrich: bool) -> Result<FixMsg> {
        let mut pairs: Vec<(&[u8], &[u8])> = Vec::new();
        for segment in split(body, separator) {
            let Some((key, value)) = split_pair(segment) else {
                continue;
            };
            pairs.push((key, value));
            if key == b"10" {
                // Nothing after the checksum is part of the message.
                break;
            }
        }
        self.build(&pairs, enrich)
    }

    /// Transforms one bridge row of `NAME=VALUE` pairs.
    ///
    /// A `SOH` or pipe delimiter preserves spaces inside a value. A row with
    /// neither delimiter uses spaces between pairs.
    ///
    /// A key opening with `#` names a group: `#NOPARTYIDS=1` is the counter
    /// and `#NOPARTYIDS[0]=…` is one occurrence whose *value* is a run of
    /// member pairs. The `#` is dropped only where it is the row's sole
    /// spelling of that key: `ORDERID=123|#ORDERID=345` states two keys, and
    /// collapsing them would merge two values under one name, so there the
    /// `#` key stays verbatim, whichever of the two arrived first. The twin
    /// is matched under the FIX name fold - the identity every key resolves
    /// by - and a bare pair whose value is a stated absence is no twin,
    /// because a key that said nothing was sent is not a key that was sent.
    /// Residue that will not split stays as one unknown key, verbatim: never
    /// dropped, never fatal.
    ///
    /// # Errors
    ///
    /// Returns the builder's refusal, which a row's content cannot provoke.
    pub fn transform_ullink_line(&self, body: &[u8], enrich: bool) -> Result<FixMsg> {
        let separator = line::ullink_separator(body);
        // The whole row is split before any `#` is judged, because the bare
        // twin that keeps one may arrive on either side of it. The segments
        // are slices of the body, so this pass allocates only the list.
        let mut arrived: Vec<(&[u8], &[u8])> = Vec::new();
        let mut hashed = false;
        for pair in split(body, separator).filter_map(split_pair) {
            hashed |= pair.0.first() == Some(&b'#');
            arrived.push(pair);
        }
        // Each `#` key is judged against the row's bare spellings, gathered
        // once: a bridge row is mostly `#` keys, so the probed list stays
        // short, and a row with no `#` at all gathers nothing. A twin that
        // itself opens with `#` - a `##` key's bare - is not in it, so that
        // one probe falls back to the whole row.
        let bare_keys: Vec<&[u8]> = if hashed {
            arrived
                .iter()
                .filter(|(key, value)| key.first() != Some(&b'#') && !self.is_absent(value))
                .map(|(key, _)| *key)
                .collect()
        } else {
            Vec::new()
        };
        let mut resolved: Vec<(Cow<'_, [u8]>, &[u8])> = Vec::with_capacity(arrived.len());
        let msgtype = msgtype_of(&arrived);
        let message = msgtype
            .as_deref()
            .and_then(|code| self.registry.get_msgtype(code, self.branch.as_ref()));
        for &(key, value) in &arrived {
            let key = match key.strip_prefix(b"#") {
                Some(bare) => {
                    let bare = line::trim_ascii(bare);
                    let twinned = if bare.first() == Some(&b'#') {
                        arrived.iter().any(|(held, held_value)| {
                            folds_twin(held, bare) && !self.is_absent(held_value)
                        })
                    } else {
                        bare_keys.iter().any(|held| folds_twin(held, bare))
                    };
                    if twinned {
                        // Verbatim means whole: the twinned `#` key is its
                        // own key and the packed value is its value, so no
                        // group rendering rewrites either - a group name
                        // opening with `#` resolves in no dictionary anyway.
                        resolved.push((Cow::Borrowed(key), value));
                        continue;
                    }
                    bare
                }
                None => key,
            };
            match group_index(key) {
                Some((group, occurrence)) if memchr::memchr(b'=', value).is_some() => {
                    let declared = self.group_members(group, message);
                    let occurrence = occurrence.to_string();
                    for (member, held) in members(value, declared) {
                        let mut rendered = Vec::with_capacity(group.len() + member.len() + 8);
                        rendered.extend_from_slice(group);
                        rendered.extend_from_slice(b"[");
                        rendered.extend_from_slice(occurrence.as_bytes());
                        rendered.extend_from_slice(b"].");
                        rendered.extend_from_slice(member);
                        resolved.push((Cow::Owned(rendered), held));
                    }
                }
                _ => resolved.push((Cow::Borrowed(key), value)),
            }
        }
        let pairs: Vec<(&[u8], &[u8])> = resolved
            .iter()
            .map(|(key, value)| (key.as_ref(), *value))
            .collect();
        self.build(&pairs, enrich)
    }

    /// Transforms one FIXML row: every element's attributes, in document order.
    ///
    /// FIXML spells a field as an XML attribute and a component as a nested
    /// element, so the attributes *are* the pairs and the nesting flattens the
    /// way a bridge occurrence already does. An element name is not a tag and
    /// nothing invents one: `<Order ClOrdID="A"/>` states one field, and one
    /// field is what this states.
    ///
    /// A namespace prefix is dropped from an attribute name, exactly as the
    /// CBlock reader drops one from an element name, because a prefix names a
    /// document's own vocabulary and never the field.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the byte position when the row is not
    /// well-formed XML, and the builder's refusal otherwise.
    pub fn transform_fixml_line(&self, body: &[u8], enrich: bool) -> Result<FixMsg> {
        let mut reader = quick_xml::Reader::from_reader(body);
        let mut buffer = Vec::new();
        let mut owned: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        let malformed = |reader: &quick_xml::Reader<&[u8]>, reason: String| Error::Parse {
            target: "fixml",
            position: reader.buffer_position() as usize,
            reason: SmolStr::new(reason),
        };
        loop {
            let event = reader
                .read_event_into(&mut buffer)
                .map_err(|error| malformed(&reader, error.to_string()))?;
            match event {
                Event::Eof => break,
                Event::Start(element) | Event::Empty(element) => {
                    for attribute in element.attributes() {
                        let attribute =
                            attribute.map_err(|error| malformed(&reader, error.to_string()))?;
                        owned.push((
                            attribute.key.local_name().as_ref().to_vec(),
                            attribute.value.into_owned(),
                        ));
                    }
                }
                _ => {}
            }
            buffer.clear();
        }
        let pairs: Vec<(&[u8], &[u8])> = owned
            .iter()
            .map(|(key, value)| (key.as_slice(), value.as_slice()))
            .collect();
        self.build(&pairs, enrich)
    }

    /// Transforms one generic record: its payload column, under its own columns.
    ///
    /// The crate already has a generic record - a name-to-value map, one
    /// `Scalar` variant - and every row-oriented reader in it produces one, so
    /// taking that shape means this accepts a row from any of them with no
    /// conversion at the boundary. The payload column is read by
    /// [`Self::transform_line`]; every other named column is a fact this codec
    /// already holds, stated per row.
    ///
    /// | column | supplies |
    /// | --- | --- |
    /// | the payload column, [`Self::with_payload_column`] | the bytes read |
    /// | `branch` | the dialect |
    /// | `beginstring` | the version |
    /// | `sep` | the separator, which also means the payload is a FIX frame |
    /// | `direction` | the direction, stated |
    ///
    /// A row outranks this codec, because a column is the caller speaking per
    /// row where the codec is the caller speaking per run. A column that is
    /// absent, null or empty is silence, never an instruction and never an
    /// error - so a record carrying only a payload reads exactly as the bytes
    /// would, which is what makes this an entry point and not a second
    /// contract.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when the value is not a record at all.
    pub fn transform_record(&self, record: &Scalar, enrich: bool) -> Result<FixMessages> {
        super::record::transform_record_with(self, record, &self.payload_column, enrich)
    }

    /// Transforms a stream of generic records, one message per record, lazily.
    ///
    /// Nothing is collected: the iterator is the stream, so a capture of ten
    /// million rows costs one message at a time. Each record is read exactly
    /// as [`Self::transform_record`] reads it, so what a row states about itself
    /// still outranks what this codec holds for the run.
    pub fn transform_records<'codec, I>(
        &'codec self,
        records: I,
        enrich: bool,
    ) -> impl Iterator<Item = Result<FixMsg>> + 'codec
    where
        I: IntoIterator<Item = Scalar>,
        I::IntoIter: 'codec,
    {
        records.into_iter().flat_map(move |record| {
            FixMessages::from_result(self.transform_record(&record, enrich))
        })
    }

    /// Transforms one Arrow batch of capture rows into one Arrow batch of
    /// messages.
    ///
    /// The batch a text reader answers with is already the shape this wants -
    /// one row per line, the payload in a named column and the capture's own
    /// `url`, `rownum` and `direction` beside it - so this takes it whole
    /// rather than through a row-at-a-time boundary, and carries those columns
    /// through ahead of the FIX ones.
    ///
    /// One batch in, one batch out, with the row count preserved. Use
    /// [`Self::transform_arrow_reader`] for a stream, which is the same read
    /// without holding a batch's worth of messages at once.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the options do not make a
    /// root field, and the Arrow layer's own failure.
    #[cfg(feature = "arrow")]
    pub fn transform_arrow_batch(
        &self,
        batch: &arrow_array::RecordBatch,
        options: &super::FixOptions,
        enrich: bool,
    ) -> Result<arrow_array::RecordBatch> {
        let schema = batch.schema();
        let source = crate::arrow::batch_reader(schema, [batch.clone()]);
        let read = self.transform_arrow_reader(source, options, enrich)?;
        Ok(crate::arrow::ArrowValue::from_reader(read)?.into_batch()?)
    }

    /// Transforms a stream of Arrow batches into a stream of message batches.
    ///
    /// The payload column is this codec's, so a reader whose payload is not
    /// `body` names it with [`Self::with_payload_column`] once for the run.
    ///
    /// # Errors
    ///
    /// Returns the schema grammar's refusal when the options do not make a
    /// root field, or the source reader's own failure.
    #[cfg(feature = "arrow")]
    pub fn transform_arrow_reader(
        &self,
        source: crate::arrow::BatchReader,
        options: &super::FixOptions,
        enrich: bool,
    ) -> Result<crate::arrow::BatchReader> {
        // The flag is the argument the caller passed rather than whatever the
        // options happened to carry, so one spelling decides it.
        let mut options = options.clone();
        options.enrich = enrich;
        super::batch::FixBatchReader::from_codec(self, source, &options)
    }

    /// Reads one Arrow batch of capture rows into one batch of filled messages.
    ///
    /// [`Self::transform_arrow_batch`] with the filling asked for, which is
    /// the spelling a caller who always wants it writes once.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::transform_arrow_batch`] returns.
    #[cfg(feature = "arrow")]
    pub fn enrich_arrow_batch(
        &self,
        batch: &arrow_array::RecordBatch,
        options: &super::FixOptions,
    ) -> Result<arrow_array::RecordBatch> {
        self.transform_arrow_batch(batch, options, true)
    }

    /// Reads a stream of capture batches into a stream of filled messages.
    ///
    /// [`Self::transform_arrow_reader`] with the filling asked for.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::transform_arrow_reader`] returns.
    #[cfg(feature = "arrow")]
    pub fn enrich_arrow_reader(
        &self,
        source: crate::arrow::BatchReader,
        options: &super::FixOptions,
    ) -> Result<crate::arrow::BatchReader> {
        self.transform_arrow_reader(source, options, true)
    }

    /// Fills what one message implies but did not carry.
    ///
    /// An order stating `OrderQty` and `CumQty` has said what `LeavesQty` is,
    /// and a fill stating `LastQty` and `LastPx` has said what it was worth.
    /// The rules are the specification's own tables read as
    /// implications - FIX 4.4's Appendix D for an order's life, 4.2's
    /// Appendix O for what a foreign exchange trade settles on - and a rule
    /// answers only where every input is stated and typed.
    ///
    /// Only the row is filled. The entries are what arrived and are carried
    /// through untouched, so [`FixMsg::into_bytes`] re-emits the received line
    /// byte for byte whether the message was enriched or not. A stated value
    /// is never overwritten, which also makes this idempotent.
    ///
    /// # Errors
    ///
    /// Returns the value contract's refusal when a derived value does not fit
    /// the column the dictionary declares for it.
    pub fn enrich_fixmsg(&self, message: FixMsg) -> Result<FixMsg> {
        super::enrich::enrich(&self.registry, message)
    }

    /// Fills a stream of messages, lazily.
    ///
    /// Nothing is collected: the iterator is the stream, so a capture of ten
    /// million messages costs one at a time.
    pub fn enrich_fixmsgs<'codec, I>(
        &'codec self,
        messages: I,
    ) -> impl Iterator<Item = Result<FixMsg>> + 'codec
    where
        I: IntoIterator<Item = FixMsg>,
        I::IntoIter: 'codec,
    {
        messages
            .into_iter()
            .map(move |message| self.enrich_fixmsg(message))
    }

    /// Builds one message from pairs the caller already split.
    ///
    /// # Errors
    ///
    /// Returns the builder's refusal.
    pub fn transform_pairs<'a, I>(&self, pairs: I, enrich: bool) -> Result<FixMsg>
    where
        I: IntoIterator<Item = (&'a [u8], &'a [u8])>,
    {
        let held: Vec<(&[u8], &[u8])> = pairs.into_iter().collect();
        self.build(&held, enrich)
    }

    /// The build a reader outside this module funnels into.
    pub(super) fn build_pairs(&self, pairs: &[(&[u8], &[u8])], enrich: bool) -> Result<FixMsg> {
        self.build(pairs, enrich)
    }

    /// The one build every reader funnels into.
    fn build(&self, pairs: &[(&[u8], &[u8])], enrich: bool) -> Result<FixMsg> {
        // A row states no dialect, so the caller's pin is the only source: a
        // capture is one session and the branch is a fact about the run.
        let branch = self.branch.clone().unwrap_or_default();
        let version = self.version.or_else(|| self.infer_version(pairs, &branch));
        let msgtype = msgtype_of(pairs);

        let message = msgtype
            .as_deref()
            .and_then(|code| self.registry.get_msgtype(code, self.branch.as_ref()));
        let mut builder = Builder::new(
            &self.registry,
            message,
            branch.clone(),
            version,
            pairs.len(),
        );
        builder.push_pairs(pairs, |value| self.is_absent(value));
        let (mut field, value, entries) = builder.finish(root_name(msgtype.as_deref()).as_str())?;
        field.as_fix_mut().set_branch(&branch)?;
        let built = FixMsg::from_parts(Arc::clone(&self.registry), field, value, entries)?;
        if enrich {
            return self.enrich_fixmsg(built);
        }
        Ok(built)
    }

    /// The direct members the addressed repeating group declares.
    ///
    /// Bridge keys are rendered names even when their bytes are digits. Only
    /// a nested field reached by that name can declare boundaries; an
    /// unresolved group leaves its value whole.
    fn group_members<'registry>(
        &'registry self,
        group: &[u8],
        message: Option<&'registry super::MsgType>,
    ) -> &'registry [Field] {
        let Ok(group) = std::str::from_utf8(group) else {
            return &[];
        };
        let field = self
            .registry
            .get_definition(crate::FixCategory::Groups, group, self.branch.as_ref())
            .or_else(|| {
                let counter = if let Some(tag) = super::field::parse_tag(group) {
                    let branch = self.branch.clone().unwrap_or_default();
                    self.registry
                        .get_field_by_id(super::FixId::from_parts(&branch, tag).ok()?)
                } else {
                    self.registry.get_field_by_name(group, self.branch.as_ref())
                }?;
                let id = counter.as_fix().id().ok()??;
                match message.filter(|message| message.has_group_counter(id)) {
                    Some(message) => message.get_group_by_counter(id),
                    None => self.registry.get_group_by_counter(id),
                }
            });
        let Some(field) = field else {
            return &[];
        };
        let item = match field.dtype() {
            DataType::List(item) | DataType::LargeList(item) => item,
            _ => return &[],
        };
        item.fields()
    }

    /// The version an arriving row is written in, when the caller pinned none.
    ///
    /// Each step is a FIX rule rather than a heuristic. `ApplVerID(1128)`
    /// wins, because under FIXT.1.1 the session version says nothing about
    /// the application version. `BeginString(8)` follows, and `FIXT.1.1`
    /// falls through rather than being taken literally. Then the branch's own
    /// default, then the dictionary's real newest - never a sentinel.
    fn infer_version(&self, pairs: &[(&[u8], &[u8])], branch: &FixBranch) -> Option<Version> {
        if let Some(value) = value_of(pairs, b"1128") {
            if let Some(version) = appl_ver_id(&String::from_utf8_lossy(value), &self.registry) {
                return Some(version);
            }
        }
        if let Some(value) = value_of(pairs, b"8") {
            let text = String::from_utf8_lossy(value);
            if let Some(rest) = text.strip_prefix("FIX.") {
                if let Ok(version) = rest.parse::<Version>() {
                    return Some(version);
                }
            }
        }
        if branch.version() != Version::MIN {
            return Some(branch.version());
        }
        self.registry.newest().map(|pedigree| pedigree.version())
    }
}

/// `ApplVerID`'s numeric and symbolic spellings.
fn appl_ver_id(value: &str, registry: &FixRegistry) -> Option<Version> {
    let spelling = match value {
        "0" | "FIX27" => "2.7",
        "1" | "FIX30" => "3.0",
        "2" | "FIX40" => "4.0",
        "3" | "FIX41" => "4.1",
        "4" | "FIX42" => "4.2",
        "5" | "FIX43" => "4.3",
        "6" | "FIX44" => "4.4",
        "7" | "FIX50" => "5.0",
        "8" | "FIX50SP1" => "5.0.1",
        "9" | "FIX50SP2" => "5.0.2",
        // "FIX Latest" is a moving label and resolves to the pedigree the
        // dictionary actually carries, never to a sentinel.
        "10" | "FIXLatest" => {
            return registry.newest().map(|pedigree| pedigree.version());
        }
        _ => return None,
    };
    spelling.parse().ok()
}

/// The value one tag carries, without building anything.
fn value_of<'a>(pairs: &[(&'a [u8], &'a [u8])], key: &[u8]) -> Option<&'a [u8]> {
    pairs
        .iter()
        .find(|(held, _)| *held == key)
        .map(|(_, value)| *value)
}

/// The message type a row declares, by `35=` or by `MSGTYPE=`.
fn msgtype_of(pairs: &[(&[u8], &[u8])]) -> Option<String> {
    for (key, value) in pairs {
        // The key folds the way every other key folds, so `MSG_TYPE` and
        // `Msg Type` name the type too.
        let folded =
            std::str::from_utf8(key).is_ok_and(|key| crate::types::folds_equal(key, "MsgType"));
        if folded || *key == b"35" {
            return Some(String::from_utf8_lossy(value).into_owned());
        }
    }
    None
}

/// Whether the frame's own first key is all ASCII digits.
fn numeric_frame(body: &[u8]) -> bool {
    let end = memchr::memchr(b'=', body).unwrap_or(0);
    end > 0 && body[..end].iter().all(u8::is_ascii_digit)
}

/// One body with a printed SOH spelling rewritten to the byte it stands for.
///
/// A capture that cannot print `0x01` writes `^A`, `\x01`, `<SOH>` or `{SOH}`
/// instead. It is the same frame; only the separator was escaped on the way
/// into the log, so it is unescaped once here rather than taught to every
/// splitter. `None` where nothing was escaped, so the ordinary path allocates
/// nothing.
fn unescaped(body: &[u8]) -> Option<Vec<u8>> {
    if memchr::memchr(0x01, body).is_some() {
        return None;
    }
    let marker = line::SOH_MARKERS
        .into_iter()
        .filter_map(|held| memchr::memmem::find(body, held).map(|at| (at, held)))
        .min_by_key(|(at, _)| *at)
        .map(|(_, held)| held)?;
    let mut held = Vec::with_capacity(body.len());
    let mut start = 0;
    while let Some(at) = memchr::memmem::find(&body[start..], marker) {
        held.extend_from_slice(&body[start..start + at]);
        held.push(0x01);
        start += at + marker.len();
    }
    held.extend_from_slice(&body[start..]);
    Some(held)
}

/// The separator a numeric frame uses: SOH when the body holds one, else `|`.
fn separator_of(body: &[u8]) -> u8 {
    if memchr::memchr(0x01, body).is_some() {
        0x01
    } else {
        b'|'
    }
}

/// One body split on its separator, empty segments included.
fn split(body: &[u8], separator: u8) -> impl Iterator<Item = &[u8]> {
    body.split(move |byte| *byte == separator)
}

/// One segment split at its **first** `=` only.
///
/// `Text=a;b` is one value with a semicolon, not two fields.
fn split_pair(segment: &[u8]) -> Option<(&[u8], &[u8])> {
    let at = memchr::memchr(b'=', segment)?;
    let key = line::trim_ascii(&segment[..at]);
    if key.is_empty() {
        return None;
    }
    Some((key, line::trim_ascii(&segment[at + 1..])))
}

/// The group and occurrence a `NAME[0]` key addresses.
fn group_index(key: &[u8]) -> Option<(&[u8], usize)> {
    let open = memchr::memchr(b'[', key)?;
    let close = memchr::memchr(b']', &key[open..])? + open;
    let index = std::str::from_utf8(&key[open + 1..close]).ok()?;
    Some((&key[..open], index.parse().ok()?))
}

/// The member pairs packed inside one occurrence's value.
///
/// ULLINK separates them with EOT then ETX, and sometimes omits the separator.
/// An explicit spelling is authoritative. With neither spelling present, only
/// direct members declared by the addressed group can begin another pair.
fn members<'value>(value: &'value [u8], declared: &[Field]) -> Vec<(&'value [u8], &'value [u8])> {
    split_members(value, declared)
        .into_iter()
        .filter_map(split_pair)
        .collect()
}

/// One occurrence's value split on the bridge's member separator.
///
/// The first explicit spelling the run actually carries wins, and only that
/// one splits it. With neither present, declared member names are boundaries;
/// an unresolved run remains one segment.
fn split_members<'value>(value: &'value [u8], declared: &[Field]) -> Vec<&'value [u8]> {
    let Some(separator) = MEMBER_SEPARATORS
        .into_iter()
        .find(|held| memchr::memmem::find(value, held).is_some())
    else {
        return split_on_declared_members(value, declared);
    };
    let mut parts = Vec::new();
    let mut start = 0;
    while let Some(at) = memchr::memmem::find(&value[start..], separator) {
        parts.push(&value[start..start + at]);
        start += at + separator.len();
    }
    parts.push(&value[start..]);
    parts
}

/// Splits a separator-less run at names declared directly by its group.
///
/// The first `KEY=` starts the run. After that, the earliest declared member
/// spelling followed by `=` starts the next pair. Matching uses the FIX name
/// fold, and the longest declared match at one byte wins. Bytes that match no
/// declared member remain verbatim in the surrounding pair.
fn split_on_declared_members<'value>(value: &'value [u8], declared: &[Field]) -> Vec<&'value [u8]> {
    let Some(first_equals) = memchr::memchr(b'=', value) else {
        return vec![value];
    };
    let mut parts = Vec::new();
    let mut start = 0;
    let mut at = first_equals + 1;
    while at < value.len() {
        let Some(prefix_len) = declared_member_prefix(&value[at..], declared) else {
            at += 1;
            continue;
        };
        parts.push(&value[start..at]);
        start = at;
        at += prefix_len;
    }
    parts.push(&value[start..]);
    parts
}

/// The length through `=` of the longest declared member at this byte.
fn declared_member_prefix(value: &[u8], declared: &[Field]) -> Option<usize> {
    declared
        .iter()
        .filter_map(|field| {
            let name = field.name();
            let prefix = folded_name_prefix(value, name)?;
            let length = name.bytes().filter(|byte| !name_separator(*byte)).count();
            Some((length, prefix))
        })
        .max_by_key(|(length, _)| *length)
        .map(|(_, prefix)| prefix)
}

/// The byte length through `=` when `value` opens with one folded FIX name.
fn folded_name_prefix(value: &[u8], name: &str) -> Option<usize> {
    if !name.is_ascii() {
        return None;
    }
    let mut at = 0;
    let mut matched = false;
    for expected in name.bytes().filter(|byte| !name_separator(*byte)) {
        let actual = *value.get(at)?;
        if !actual.eq_ignore_ascii_case(&expected) {
            return None;
        }
        matched = true;
        at += 1;
        while value.get(at).is_some_and(|byte| name_separator(*byte)) {
            at += 1;
        }
    }
    (matched && value.get(at) == Some(&b'=')).then_some(at + 1)
}

/// One ASCII separator ignored by the FIX name fold.
const fn name_separator(byte: u8) -> bool {
    matches!(byte, b'_' | b'-' | b' ')
}

/// Whether two key spellings name one field under the FIX name fold.
///
/// The twin that keeps a `#` is judged by the identity every key resolves
/// by - case and separators fold away - because `OrderId=1|#ORDERID=2` merges
/// under one name exactly as the same-cased pair would.
fn folds_twin(left: &[u8], right: &[u8]) -> bool {
    let mut left = left.iter().filter(|byte| !name_separator(**byte));
    let mut right = right.iter().filter(|byte| !name_separator(**byte));
    loop {
        match (left.next(), right.next()) {
            (None, None) => return true,
            (Some(one), Some(other)) if one.eq_ignore_ascii_case(other) => {}
            _ => return false,
        }
    }
}
