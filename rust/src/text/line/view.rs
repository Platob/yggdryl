//! [`TextLine`], a borrowed view over one parsed record.
//!
//! A view, never an owned row copy. The record's bytes stay in the reader's
//! window; the header and message are `Range<usize>` spans into them, the
//! captures are spans, and no accessor allocates. The costs a caller does not
//! ask for are not paid: UTF-8 validation happens once per record on first ask
//! rather than once per accessor, the stable hash folds on first ask and
//! caches, and the header match runs once and is reused for the header span,
//! the message spans, the timestamp, and every capture.
//!
//! Where that does not hold is stated rather than hidden:
//!
//! - **A content coding cannot be borrowed.** An inflate has to produce bytes,
//!   so a coded resource is read into one reused, growable window that records
//!   are borrowed out of - the allocation count is a function of the window,
//!   not of the row count.
//! - **A record straddling a window boundary** is the one case that copies. The
//!   window grows to hold it, so the copy is amortized rather than per record.
//! - **[`TextLine::into_owned`]** copies, because a caller asked it to.

use std::cell::{OnceCell, RefCell};
use std::ops::Range;

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result};

use regex::CaptureLocations;

use super::options::TextLineOptions;

/// One parsed record, borrowed from the reader's window.
///
/// Built by the readers in [`super`], which document what is borrowed and what
/// is not.
#[derive(Debug)]
pub struct TextLine<'window> {
    /// The record's bytes, terminator excluded.
    bytes: &'window [u8],
    /// The extractor this record is read under.
    ///
    /// Borrowed rather than held, so a view costs one pointer and the options
    /// are the same value the Arrow path consumes - which is what keeps the
    /// accessors and the columns from drifting into two implementations.
    options: &'window TextLineOptions,
    /// The resource's canonical location, empty when it has none.
    url: &'window str,
    /// 1-based record index within the resource.
    rownum: i64,
    /// Byte offset of the record's first line in the *decoded* stream - the
    /// resume key a tailing reader seeks back to.
    offset: u64,
    /// How many lines the record spans.
    lines: i32,
    /// The record as text, validated once on first ask.
    ///
    /// The failure half is the byte the invalid sequence starts at, not a
    /// built `Error`: the cell has to be `Copy`-cheap to read, and the message
    /// is assembled on the cold path from that one number.
    text: OnceCell<std::result::Result<&'window str, usize>>,
    /// The stable hash of the message, folded on first ask.
    hash: OnceCell<i64>,
    /// The header match, run once and reused by every derived field.
    matched: OnceCell<Option<Matched>>,
    /// The reader's capture buffers, borrowed rather than allocated per record.
    scratch: &'window Scratch,
}

/// Everything one header match yields, resolved once.
#[derive(Clone, Debug)]
pub(crate) struct Matched {
    /// The header's span within the record.
    pub(crate) header: Range<usize>,
    /// The message's spans: the record with the header spliced out, stripped.
    ///
    /// Two spans when the header does not open the record, one otherwise. They
    /// are appended in order wherever the message is consumed, so nothing has
    /// to be joined into a fresh `String` first.
    pub(crate) message: (Range<usize>, Range<usize>),
}

/// The buffers a matched record borrows instead of allocating.
///
/// Both would otherwise be one allocation per *record*: the engine's
/// `captures` builds a fresh `Captures` for every call, and the span vector is
/// one per match. A record is a view, so neither belongs to it - they belong
/// to the reader, which holds exactly one of each and rewrites them as records
/// go by.
///
/// That is sound because exactly one record is alive at a time:
/// [`TextLines::next`](super::TextLines::next) borrows the reader for the
/// record it returns, so the next call cannot run until that record is gone.
/// The interior mutability is what lets the accessors keep taking `&self`.
#[derive(Debug, Default)]
pub(crate) struct Scratch {
    /// Filled by `captures_read` rather than allocated per call. `None` when
    /// the extractor has no header expression to fill it from.
    locations: RefCell<Option<CaptureLocations>>,
    /// Each named capture's span, in group order; `None` when it did not
    /// participate in the match. Cleared and refilled per record.
    spans: RefCell<Vec<Option<Range<usize>>>>,
    /// Each capture column's group index within the header expression, resolved
    /// once here rather than by a name walk per record. `None` for a column the
    /// header expression does not capture.
    indices: Vec<Option<usize>>,
}

impl Scratch {
    /// The buffers `options` needs, sized once for the whole read.
    pub(crate) fn for_options(options: &TextLineOptions) -> Self {
        let expression = options.header_expression();
        Self {
            locations: RefCell::new(expression.map(regex::Regex::capture_locations)),
            spans: RefCell::new(Vec::with_capacity(options.capture_names().count())),
            indices: options
                .capture_names()
                .map(|name| {
                    expression.and_then(|expression| {
                        expression
                            .capture_names()
                            .position(|held| held == Some(name))
                    })
                })
                .collect(),
        }
    }
}

impl<'window> TextLine<'window> {
    /// Build a view over one record's bytes.
    pub(crate) fn new(
        bytes: &'window [u8],
        options: &'window TextLineOptions,
        url: &'window str,
        rownum: i64,
        offset: u64,
        lines: i32,
        scratch: &'window Scratch,
    ) -> Self {
        Self {
            bytes,
            options,
            url,
            rownum,
            offset,
            lines,
            text: OnceCell::new(),
            hash: OnceCell::new(),
            matched: OnceCell::new(),
            scratch,
        }
    }

    /// The extractor this record was read under.
    #[must_use]
    pub const fn options(&self) -> &'window TextLineOptions {
        self.options
    }

    /// The header match, resolved once and reused by every derived field.
    ///
    /// This is the *one* extraction implementation. The header span, both
    /// message spans, the timestamp, and every capture come from here, so a
    /// caller reading `line.message()` and a batch's `message` column for the
    /// same record see byte-identical text by construction rather than by two
    /// implementations agreeing.
    pub(crate) fn matched(&self) -> Result<Option<&Matched>> {
        // The header is matched against the *opening line* alone. The pattern
        // is a line pattern - the grouping opened this record because its first
        // line matched - so capturing over the whole record would let a greedy
        // class cross into the continuation lines and call text a header that
        // the opening line never contained.
        let resolved = self.matched.get_or_init(|| {
            let text = self.text().ok()?;
            let opening = &text[..text.find('\n').unwrap_or(text.len())];
            let mut spans = self.scratch.spans.borrow_mut();
            spans.clear();
            let header = match self.options.header_expression() {
                Some(expression) => {
                    let mut held = self.scratch.locations.borrow_mut();
                    let locations = held.as_mut()?;
                    expression.captures_read(locations, opening)?;
                    let whole = locations.get(0)?;
                    // Group order, through the index table resolved once per
                    // read - a name the expression does not have stays `None`.
                    spans.extend(self.scratch.indices.iter().map(|index| {
                        index
                            .and_then(|index| locations.get(index))
                            .map(|(start, end)| start..end)
                    }));
                    whole.0..whole.1
                }
                // Log mode has no header expression: the timestamp parse that
                // opened the record yields the span, and the closed token table
                // extends it over the conventional prefix tokens.
                None if self.options.is_log_mode() => 0..super::log::header_extent(opening)?,
                None => return None,
            };
            drop(spans);
            let message = self.message_spans(text, &header);
            Some(Matched { header, message })
        });
        // A record whose text is not UTF-8 has no header, but the failure is
        // the text's and is reported where the text is asked for.
        if resolved.is_none() {
            self.text()?;
        }
        Ok(resolved.as_ref())
    }

    /// The message's spans: the record with the header spliced out, stripped.
    ///
    /// Two spans when the header does not open the record, one otherwise. They
    /// stay spans rather than a joined string because the Arrow builder appends
    /// them in order and the hash folds them in order, so nothing ever has to
    /// materialize the join.
    fn message_spans(&self, text: &str, header: &Range<usize>) -> (Range<usize>, Range<usize>) {
        let mut lead = 0..header.start;
        let mut tail = header.end..text.len();
        // The strip is span arithmetic: it narrows the ranges and never builds
        // a new string, whatever the setting.
        let lstrip = self.options.lstrip();
        let rstrip = self.options.rstrip();
        if !lstrip.is_none() {
            if lead.is_empty() {
                tail.start += lstrip.lead(&text[tail.clone()]);
            } else {
                lead.start += lstrip.lead(&text[lead.clone()]);
            }
        }
        if !rstrip.is_none() {
            if tail.is_empty() {
                lead.end -= rstrip.trail(&text[lead.clone()]);
            } else {
                tail.end -= rstrip.trail(&text[tail.clone()]);
            }
        }
        // A strip can cross an emptied span; keep both well formed.
        if lead.start > lead.end {
            lead.start = lead.end;
        }
        if tail.start > tail.end {
            tail.start = tail.end;
        }
        (lead, tail)
    }

    /// The exact text the header expression matched, when it matched.
    ///
    /// `None` for the preamble a rotated file starts with, and for a record
    /// whose opening line a separate header expression did not match - one rule
    /// for "no header here", not two.
    ///
    /// # Errors
    ///
    /// Returns the record's UTF-8 failure.
    pub fn header(&self) -> Result<Option<&'window str>> {
        let text = self.text()?;
        Ok(self.matched()?.map(|matched| &text[matched.header.clone()]))
    }

    /// The message's two spans, in order, with empty ones dropped.
    ///
    /// The shape a consumer that can append twice wants: the Arrow builder and
    /// the hash both take this, so neither ever joins the halves into a fresh
    /// string.
    ///
    /// An unmatched record is the whole record as one span.
    ///
    /// # Errors
    ///
    /// Returns the record's UTF-8 failure.
    pub fn message_parts(&self) -> Result<[&'window str; 2]> {
        let text = self.text()?;
        let Some(matched) = self.matched()? else {
            // Nothing matched, so the whole record is the message - stripped
            // the same way a matched one is, so the two shapes agree.
            let lead = self.options.lstrip().lead(text);
            let trail = self.options.rstrip().trail(&text[lead..]);
            return Ok([&text[lead..text.len() - trail], ""]);
        };
        Ok([
            &text[matched.message.0.clone()],
            &text[matched.message.1.clone()],
        ])
    }

    /// The message: the record with the header removed, then stripped.
    ///
    /// Borrowed when the header opens the record - the overwhelmingly common
    /// shape - because the message is then one contiguous span. A header
    /// *mid-line* leaves two spans, and joining them is the one place this
    /// accessor allocates; [`Self::message_parts`] is the form that never does,
    /// and is what the Arrow builder and the hash consume.
    ///
    /// # Errors
    ///
    /// Returns the record's UTF-8 failure.
    pub fn message(&self) -> Result<std::borrow::Cow<'window, str>> {
        let [lead, tail] = self.message_parts()?;
        Ok(match (lead.is_empty(), tail.is_empty()) {
            (true, _) => std::borrow::Cow::Borrowed(tail),
            (_, true) => std::borrow::Cow::Borrowed(lead),
            _ => {
                let mut joined = String::with_capacity(lead.len() + tail.len());
                joined.push_str(lead);
                joined.push_str(tail);
                std::borrow::Cow::Owned(joined)
            }
        })
    }

    /// One named capture's text, when it participated in the match.
    ///
    /// # Errors
    ///
    /// Returns the record's UTF-8 failure.
    pub fn capture(&self, name: &str) -> Result<Option<&'window str>> {
        let Some(index) = self.options.capture_names().position(|held| held == name) else {
            return Ok(None);
        };
        self.capture_at(index)
    }

    /// One capture's text by column position, when it participated.
    ///
    /// # Errors
    ///
    /// Returns the record's UTF-8 failure.
    pub fn capture_at(&self, index: usize) -> Result<Option<&'window str>> {
        let text = self.text()?;
        if self.matched()?.is_none() {
            return Ok(None);
        }
        // Copied out of the scratch before the text is indexed, so the borrow
        // ends here and a caller holding the answer holds a plain `&str`.
        let span = self.scratch.spans.borrow().get(index).cloned().flatten();
        Ok(span.map(|span| &text[span]))
    }

    /// Every named capture's text, in column order.
    ///
    /// # Errors
    ///
    /// Returns the record's UTF-8 failure.
    pub fn captures(&self) -> Result<Vec<Option<&'window str>>> {
        let count = self.options.capture_names().count();
        (0..count).map(|index| self.capture_at(index)).collect()
    }

    /// The stable hash of this record's **message**, folded once and cached.
    ///
    /// FNV-1a over the message's bytes, with the `u64` state reinterpreted as
    /// two's-complement `i64` (`u64 as i64`, bit pattern preserved) because
    /// Iceberg has no unsigned types. Equal messages hash equal across files,
    /// runs, and rotations, which is what makes it a dedupe or join key.
    ///
    /// A caller that never asks for the hash never pays for it, and the Arrow
    /// builder asks exactly once per row.
    ///
    /// # The strip options move this
    ///
    /// The hash covers the message, so
    /// [`lstrip`](TextLineOptions::lstrip)/[`rstrip`](TextLineOptions::rstrip)
    /// change it. It stays deterministic - but *given the options*, and two
    /// readers configured differently hash the same log line differently.
    /// [`timezone`](TextLineOptions::timezone) does **not** touch it, because
    /// the hash covers the message only.
    ///
    /// # Errors
    ///
    /// Returns the record's UTF-8 failure.
    pub fn hash(&self) -> Result<i64> {
        if let Some(held) = self.hash.get() {
            return Ok(*held);
        }
        let [lead, tail] = self.message_parts()?;
        // Folded chunk-wise so a two-span message and the equivalent contiguous
        // string hash identically: a hash that depended on where the header sat
        // in the line would be a silent correctness bug.
        let folded = crate::text::stable_hash_chunks([lead.as_bytes(), tail.as_bytes()]) as i64;
        let _ = self.hash.set(folded);
        Ok(folded)
    }

    /// The record's raw bytes, terminator excluded.
    ///
    /// Always available and never validated: a byte-oriented caller pays
    /// nothing for UTF-8 it does not need.
    #[must_use]
    pub const fn bytes(&self) -> &'window [u8] {
        self.bytes
    }

    /// The record as text, validated once and cached.
    ///
    /// Asking for the header, the message, and five captures costs **one**
    /// validation pass, not seven.
    ///
    /// # Errors
    ///
    /// Returns the first invalid UTF-8 sequence's position.
    pub fn text(&self) -> Result<&'window str> {
        let validated = *self
            .text
            .get_or_init(|| std::str::from_utf8(self.bytes).map_err(|error| error.valid_up_to()));
        validated.map_err(|position| Error::InvalidRecord {
            path: format_smolstr!("$[{}]", self.rownum.saturating_sub(1)),
            reason: format_smolstr!(
                "expected UTF-8 record text, got an invalid sequence at byte {position} of row {} \
                 in {}",
                self.rownum,
                self.url
            ),
        })
    }

    /// The resource's canonical location, empty when it has none.
    #[must_use]
    pub const fn url(&self) -> &'window str {
        self.url
    }

    /// The 1-based record index within the resource.
    #[must_use]
    pub const fn rownum(&self) -> i64 {
        self.rownum
    }

    /// The byte offset of this record's first line in the *decoded* stream.
    ///
    /// The resume key a tailing reader seeks back to. It counts decoded bytes -
    /// the stream after every content coding is peeled - and it is exact for
    /// every terminator form, including a resource that mixes them: a `\r\n`
    /// counts two bytes.
    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// How many lines this record spans - a free flag for stack traces.
    #[must_use]
    pub const fn line_count(&self) -> i32 {
        self.lines
    }

    /// The entry timestamp: the civil reading, its unit, and the zone offset.
    ///
    /// The same three numbers the `date`, `time`, and `unix` columns are built
    /// from, through the same resolution, so an accessor and a column can never
    /// disagree. `date` and `time` are the civil reading **as written**; only
    /// `unix` moves with the offset.
    ///
    /// An offset present in the timestamp text wins; the
    /// [`timezone`](TextLineOptions::timezone) option applies only to
    /// timestamps that carry none.
    ///
    /// `None` for a record with no header.
    ///
    /// # Errors
    ///
    /// Returns the record's UTF-8 failure, or the timestamp's parse failure.
    pub fn timestamp(&self) -> Result<Option<(i64, crate::TimeUnit, i64)>> {
        let Some(header) = self.header()? else {
            return Ok(None);
        };
        let source = match self.options.timestamp_capture() {
            Some(name) => match self.capture(name)? {
                Some(value) => value,
                None => return Ok(None),
            },
            None => header,
        };
        let mut cache = super::timestamp::OffsetCache::default();
        super::timestamp::read(source, self.options, &mut cache).map(Some)
    }

    /// The recognized log tokens of this record's header, in column order.
    ///
    /// `level`, `logger`, `thread` - the same values the `level`, `logger`, and
    /// `thread` columns carry in log mode, from the same closed table, so an
    /// accessor and a column can never disagree. All three are `None` for a
    /// record with no header.
    ///
    /// The table is documented shape by shape on
    /// [`log::recognized`](super::log::recognized); an unrecognized token is
    /// left in the header untouched rather than guessed at.
    ///
    /// # Errors
    ///
    /// Returns the record's UTF-8 failure.
    pub fn tokens(&self) -> Result<[Option<&'window str>; 3]> {
        Ok(match self.header()? {
            Some(header) => super::log::recognized(header),
            None => [None; 3],
        })
    }

    /// Whether the header expression matched this record's opening line.
    ///
    /// # Errors
    ///
    /// Returns the record's UTF-8 failure.
    pub fn is_matched(&self) -> Result<bool> {
        Ok(self.matched()?.is_some())
    }

    /// Consume this view into an owned copy.
    ///
    /// The one accessor that copies, and only because a caller asked. Reach for
    /// it when a record must outlive the reader's window; every other path
    /// borrows.
    ///
    /// # Errors
    ///
    /// Returns the record's UTF-8 failure.
    pub fn into_owned(self) -> Result<TextLineBuf> {
        Ok(TextLineBuf {
            text: SmolStr::new(self.text()?),
            url: SmolStr::new(self.url),
            rownum: self.rownum,
            offset: self.offset,
            lines: self.lines,
        })
    }
}

/// One record, owned.
///
/// What [`TextLine::into_owned`] produces, for a caller that genuinely needs a
/// record to outlive the reader's window. Every other path borrows.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TextLineBuf {
    text: SmolStr,
    url: SmolStr,
    rownum: i64,
    offset: u64,
    lines: i32,
}

impl TextLineBuf {
    /// The record's text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The resource's canonical location, empty when it has none.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The 1-based record index within the resource.
    #[must_use]
    pub const fn rownum(&self) -> i64 {
        self.rownum
    }

    /// The byte offset of this record's first line in the decoded stream.
    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    /// How many lines this record spans.
    #[must_use]
    pub const fn line_count(&self) -> i32 {
        self.lines
    }
}

impl std::fmt::Display for TextLineBuf {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.text)
    }
}
