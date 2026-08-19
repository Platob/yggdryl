//! The Arrow projection of text records - the only Arrow-gated half.
//!
//! What streams out is the same decoded text [`Text::read_lines`] yields,
//! grouped into records and projected into Arrow batches with **one batch in
//! memory at a time**.
//!
//! # The schema
//!
//! A non-null Struct [`Field`] is the schema, and every base column is a
//! datatype the strict Iceberg codec accepts **unchanged**, so the parsed
//! batches append into an Iceberg table exactly as declared - not merely after
//! widening:
//!
//! | column    | datatype     | Iceberg  | meaning |
//! | --------- | ------------ | -------- | ------- |
//! | `url`     | `utf8`       | `string` | the resource's canonical [`Url`](crate::Url) display |
//! | `rownum`  | `int64`      | `long`   | 1-based record index within the resource |
//! | `date`    | `date32`     | `date`   | the entry's civil date, **as written** |
//! | `time`    | `time64(us)` | `time`   | the clock reading **as written**, truncated - never rounded - to microseconds |
//! | `unix`    | `int64`      | `long`   | nanoseconds since the Unix epoch (see below) |
//! | `hash`    | `int64`      | `long`   | the stable FNV-1a hash of `message` **only** |
//! | `header`  | `utf8`       | `string` | the exact text the header expression matched |
//! | `message` | `utf8`       | `string` | the record with the header removed, then stripped |
//! | `offset`  | `int64`      | `long`   | byte offset of the record's first line in the *decoded* stream |
//! | `lines`   | `int32`      | `int`    | how many lines the record spans |
//!
//! In **log mode** - no pattern, so records open where a timestamp opens -
//! three more columns follow, always emitted and always nullable: `level`,
//! `logger`, `thread`. They are a *fixed* set, never discovered from the data,
//! so [`TextLineOptions::schema`] still answers from configuration alone.
//!
//! Then one nullable column per **named capture group**, in group order - the
//! union of the opening pattern's and the header pattern's - and finally the
//! constant [`custom_fields`](TextLineOptions::custom_fields) columns.
//!
//! # `unix` versus `date` and `time`
//!
//! `date` and `time` are the **civil reading the record contains**, exactly as
//! written. `unix` is the *instant*, which is the same number only when the
//! reading is UTC. With [`timezone`](TextLineOptions::set_timezone) set, or with
//! an offset in the text, a line reading `00:30` at `+02:00` carries the local
//! date and a `unix` on the previous UTC day. Both are right; a reader has to
//! be told, because it looks like a bug.
//!
//! Unset and with no offset in the text, `unix` is what it has always been: the
//! civil reading counted from the epoch, with no zone applied - so it is *not*
//! a Unix timestamp unless the log happens to be written in UTC.
//!
//! # Batching
//!
//! A batch closes on whichever bound trips first, and on a leaf boundary
//! regardless. [`byte_size`](TextLineOptions::byte_size) measures **decoded
//! input bytes** - each record's length plus its terminator, accounted as
//! records arrive, never by introspecting builder memory - so it is not an
//! allocation cap and reading it as one would be a mistake.
//! [`batch_size`](TextLineOptions::batch_size) is the row bound.
//!
//! ```
//! use yggdryl::io::{Buffer, IOBase};
//! use yggdryl::text::TextLineOptions;
//! use yggdryl::Url;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut handle = Buffer::new()
//!     .with_media_type(Url::from_str("file:///app.log")?.media_type());
//! handle.write_all_bytes(
//!     b"2024-02-01 10:00:00.000_000 [ee] [alpha] boom\n  at frame one\n\
//!       2024-02-01 10:00:01.500_000 [ii] [beta] fine\n",
//! )?;
//!
//! let options = TextLineOptions::with_pattern(
//!     r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\S* \[(?<level>[^\]]+)\] \[(?<logger>[^\]]+)\]",
//! )?;
//! let batches: Vec<_> = handle
//!     .read_arrow_lines(&options)?
//!     .collect::<Result<_, _>>()?;
//!
//! assert_eq!(batches.len(), 1);
//! assert_eq!(batches[0].num_rows(), 2);
//! assert_eq!(batches[0].schema().field(0).name(), "url");
//! assert_eq!(batches[0].schema().field(10).name(), "level");
//! # Ok(())
//! # }
//! ```

use std::fmt::Write as _;
use std::io::Read;
use std::sync::Arc;

use arrow_array::builder::{
    Date32Builder, Int32Builder, Int64Builder, StringBuilder, Time64MicrosecondBuilder,
};
use arrow_array::{ArrayRef, RecordBatch, UInt32Array};
use arrow_schema::{ArrowError, SchemaRef};
use smol_str::{SmolStr, ToSmolStr, format_smolstr};

use crate::arrow::{BatchReader, scalar_array, schema_from_field};
use crate::generic::{Holder, iso};
use crate::io::{Buffer, IOBase};
use crate::{DataType, Error, Field, IOKind, Result};

use super::handle::{Text, TextLines, url_text};
use super::options::TextLineOptions;
use super::view::TextLine;

/// Project a borrowed handle's records, per the trait method's contract.
///
/// [`IOKind`] decides the shape, never a second existence check:
///
/// - A **leaf** parses that one resource's records. This borrowed variant needs
///   an owned view behind the reader it returns, so it reopens the same
///   location through [`IOBase::parent`] and [`IOBase::child_by_path`]; a handle with
///   no parent - an in-memory buffer - contributes a snapshot of its
///   still-encoded bytes instead, which those handles already hold in memory.
///   [`into_arrow_lines`] avoids both by consuming the handle.
/// - A **container** streams across the leaf files beneath it in deterministic
///   name-sorted order, each opened lazily when the reader reaches it, each
///   contributing its own `url` and restarting `rownum` at 1, and each decoded
///   by its *own* media type - a folder mixing `a.log` and `b.log.gz` reads
///   uniformly. A batch never spans two leaves.
/// - [`IOKind::Unknown`] - the resource does not exist - reads as an **empty**
///   reader: zero batches, schema still answered. Absence is never an error on
///   the read path.
pub(crate) fn read_arrow_lines<H: IOBase + ?Sized>(
    handle: &H,
    options: &TextLineOptions,
) -> Result<BatchReader> {
    match handle.kind() {
        IOKind::Directory => folder_lines(handle, options),
        IOKind::Unknown => empty_lines(options),
        _ => {
            // The url column reports this handle's canonical location even when
            // the owned view is reached another way.
            let url = url_text(handle);
            leaf_lines(reopened(handle)?, url, options)
        }
    }
}

/// Project a snapshot of the bytes a *view* handle presents.
///
/// [`reopened`] assumes a handle's bytes are its location's bytes, which is
/// true of every storage handle and false of a decoding view such as
/// [`Coded`](crate::io::Coded): its reads present decoded bytes while its
/// location holds the encoded form. A view's projection routes here instead - a
/// copy of the presented value, under the handle's own url and media type. A
/// coded view materializes that value to serve any read, so the copy is its
/// ordinary cost, not a new one.
pub(crate) fn snapshot_arrow_lines<H: IOBase + ?Sized>(
    handle: &H,
    options: &TextLineOptions,
) -> Result<BatchReader> {
    match handle.kind() {
        IOKind::Directory => folder_lines(handle, options),
        IOKind::Unknown => empty_lines(options),
        _ => {
            let url = url_text(handle);
            let mut buffer = Buffer::new();
            handle.copy_into(&mut buffer)?;
            leaf_lines(Holder::buffer(buffer), url, options)
        }
    }
}

/// Project an owned handle's records, per the trait method's contract.
pub(crate) fn into_arrow_lines<H: IOBase + 'static>(
    handle: H,
    options: &TextLineOptions,
) -> Result<BatchReader> {
    match handle.kind() {
        IOKind::Directory => folder_lines(&handle, options),
        IOKind::Unknown => empty_lines(options),
        _ => {
            let url = url_text(&handle);
            leaf_lines(handle, url, options)
        }
    }
}

/// An owned view of the same location a borrowed handle addresses.
fn reopened<H: IOBase + ?Sized>(handle: &H) -> Result<Holder> {
    if let Some(parent) = handle.parent() {
        if let Some(name) = handle.url().and_then(crate::Url::file_name) {
            let mut child = parent.child_by_path(name)?;
            // The reopened view keeps the caller's declared media type, so an
            // override - a coding the name does not spell - survives.
            child.set_media_type(handle.media_type().clone());
            return Ok(child);
        }
    }
    let mut buffer = Buffer::new();
    handle.copy_into(&mut buffer)?;
    Ok(Holder::buffer(buffer))
}

/// Zero batches, schema still answered - what absence reads as.
fn empty_lines(options: &TextLineOptions) -> Result<BatchReader> {
    let schema = schema_from_field(options.schema())?;
    Ok(crate::arrow::batch_reader(schema, []))
}

/// Stream a container's leaves, name-sorted, one open leaf at a time.
fn folder_lines(handle: &(impl IOBase + ?Sized), options: &TextLineOptions) -> Result<BatchReader> {
    // The walk enumerates handles only - constructions that touch nothing. Each
    // leaf's bytes wait until the reader reaches it, and what is held is one
    // handle per leaf, bounded by the container being read.
    let leaves: Vec<Holder> = handle
        .children_where(&[], false)?
        .collect::<Result<Vec<_>>>()?;
    ArrowLines::boxed(options, leaves, None)
}

/// Stream one leaf the reader already owns.
fn leaf_lines<H: IOBase + 'static>(
    handle: H,
    url: Arc<str>,
    options: &TextLineOptions,
) -> Result<BatchReader> {
    let current = opened_records(handle, url, options)?;
    ArrowLines::boxed(options, Vec::new(), Some(current))
}

/// Open one resource's records with their per-leaf row state.
fn opened_records<H: IOBase + 'static>(
    handle: H,
    url: Arc<str>,
    options: &TextLineOptions,
) -> Result<LeafRecords> {
    let mut handler = Text::with_options(handle, options.clone());
    handler.set_url_override(url);
    Ok(LeafRecords {
        records: handler.into_read_lines()?,
    })
}

/// One resource's records mid-stream.
struct LeafRecords {
    records: TextLines<Box<dyn Read + Send + 'static>>,
}

/// The streaming projection: text records in, Arrow batches out.
///
/// At most one leaf is open and one batch under construction at any time. A
/// batch never spans two leaves, so every batch already emitted is complete
/// before the next leaf is even opened - which is what makes the laziness
/// observable: a later leaf that fails to decode surfaces its error only after
/// every earlier leaf's batches have arrived.
struct ArrowLines {
    options: Arc<TextLineOptions>,
    /// One-row constants, repeated to each batch's height. Held one row at a
    /// time deliberately: the repetition is a `take`, not a stored column.
    customs: Vec<ArrayRef>,
    byte_size: usize,
    batch_size: usize,
    schema: SchemaRef,
    /// The emitted root; a batch is cast onto it when any capture is typed.
    root: Field,
    /// The all-text shape the builders produce.
    raw_schema: SchemaRef,
    /// Whether any capture column is typed, so an untyped read never pays for a
    /// cast that would hand every array back unchanged.
    typed: bool,
    /// Leaves not yet opened, in name-sorted order.
    pending: std::vec::IntoIter<Holder>,
    current: Option<LeafRecords>,
    done: bool,
}

impl ArrowLines {
    /// Assemble the reader over already-validated options.
    fn boxed(
        options: &TextLineOptions,
        pending: Vec<Holder>,
        current: Option<LeafRecords>,
    ) -> Result<BatchReader> {
        let root = options.schema();
        let schema = schema_from_field(root)?;
        let leading = options.leading_column_count();
        let capture_count = options.capture_names().count();
        let mut customs = Vec::with_capacity(options.custom_fields().len());
        for (index, (_, value)) in options.custom_fields().iter().enumerate() {
            let field = root
                .get_field(leading + capture_count + index)
                .ok_or_else(|| Error::from(crate::arrow::Error::internal("text::line::customs")))?;
            customs.push(scalar_array(field, value)?);
        }
        // The builders always produce text captures; a typed capture is cast
        // onto the declared root as each batch closes, through the one cast
        // definition every schema-directed read uses.
        let typed = (0..capture_count).any(|index| {
            root.get_field(leading + index)
                .is_some_and(|field| field.data_type() != &DataType::Utf8)
        });
        let raw_schema = if typed {
            let mut raw = root.clone();
            for index in 0..capture_count {
                let name = root
                    .get_field(leading + index)
                    .map(|field| field.name().to_owned())
                    .unwrap_or_default();
                raw.set_field(leading + index, DataType::Utf8.nullable_field(name))?;
            }
            schema_from_field(&raw)?
        } else {
            Arc::clone(&schema)
        };
        Ok(Box::new(Self {
            options: Arc::new(options.clone()),
            customs,
            byte_size: options.effective_byte_size(),
            batch_size: options.effective_batch_size(),
            schema,
            root: root.clone(),
            raw_schema,
            typed,
            pending: pending.into_iter(),
            current,
            done: false,
        }))
    }

    /// Close the batch under construction into one `RecordBatch`.
    fn finish(&self, builders: &mut RowBuilders) -> std::result::Result<RecordBatch, ArrowError> {
        let rows = builders.rows;
        let mut columns: Vec<ArrayRef> = vec![
            Arc::new(builders.url.finish()),
            Arc::new(builders.rownum.finish()),
            Arc::new(builders.date.finish()),
            Arc::new(builders.time.finish()),
            Arc::new(builders.unix.finish()),
            Arc::new(builders.hash.finish()),
            Arc::new(builders.header.finish()),
            Arc::new(builders.message.finish()),
            Arc::new(builders.offset.finish()),
            Arc::new(builders.lines.finish()),
        ];
        for token in &mut builders.tokens {
            columns.push(Arc::new(token.finish()));
        }
        for capture in &mut builders.captures {
            columns.push(Arc::new(capture.finish()));
        }
        for constant in &self.customs {
            let indices = UInt32Array::from(vec![0_u32; rows]);
            columns.push(arrow_select::take::take(constant.as_ref(), &indices, None)?);
        }
        builders.reset();
        let batch = RecordBatch::try_new(Arc::clone(&self.raw_schema), columns)?;
        if !self.typed {
            return Ok(batch);
        }
        // Typed captures land through the one cast definition, strictly: a
        // captured text the declared datatype cannot read is an error, never a
        // silent null.
        use crate::field::cast::ArrowCast;
        self.root
            .cast_arrow_batch(batch, false)
            .map_err(|error| ArrowError::ExternalError(Box::new(error)))
    }
}

impl Iterator for ArrowLines {
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let log_mode = self.options.is_log_mode();
        let mut builders = RowBuilders::new(self.options.capture_names().count(), log_mode);
        loop {
            let Some(current) = self.current.as_mut() else {
                let Some(leaf) = self.pending.next() else {
                    break;
                };
                let url = url_text(&leaf);
                match opened_records(leaf, url, &self.options) {
                    Ok(records) => self.current = Some(records),
                    Err(error) => {
                        self.done = true;
                        return Some(Err(external(error)));
                    }
                }
                continue;
            };
            match current.records.next() {
                Some(Ok(line)) => {
                    if let Err(error) = append_row(&mut builders, &line, log_mode) {
                        self.done = true;
                        return Some(Err(external(error)));
                    }
                    // Whichever bound trips first closes the batch.
                    if builders.rows >= self.batch_size || builders.bytes >= self.byte_size {
                        return Some(self.finish(&mut builders));
                    }
                }
                Some(Err(error)) => {
                    self.done = true;
                    return Some(Err(external(error)));
                }
                None => {
                    // The drained leaf closes its batch before the next leaf is
                    // opened, so a batch never spans two resources.
                    self.current = None;
                    if builders.rows > 0 {
                        return Some(self.finish(&mut builders));
                    }
                }
            }
        }
        self.done = true;
        if builders.rows == 0 {
            return None;
        }
        Some(self.finish(&mut builders))
    }
}

impl arrow_array::RecordBatchReader for ArrowLines {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

/// Carry a core failure through the Arrow stream, typed and recoverable.
fn external(error: Error) -> ArrowError {
    ArrowError::ExternalError(Box::new(error))
}

/// The column builders of one batch under construction.
struct RowBuilders {
    url: StringBuilder,
    rownum: Int64Builder,
    date: Date32Builder,
    time: Time64MicrosecondBuilder,
    unix: Int64Builder,
    hash: Int64Builder,
    header: StringBuilder,
    message: StringBuilder,
    offset: Int64Builder,
    lines: Int32Builder,
    /// Log mode's fixed token columns, empty otherwise.
    tokens: Vec<StringBuilder>,
    captures: Vec<StringBuilder>,
    rows: usize,
    /// The last resolved zone offset and the interval it holds over.
    offsets: super::timestamp::OffsetCache,
    /// Decoded input bytes appended so far - the byte bound's accounting.
    ///
    /// Counted as records arrive, from each record's own length, rather than by
    /// introspecting builder memory: the bound is over *input*, and asking a
    /// builder how much it has allocated would answer a different question.
    bytes: usize,
}

impl RowBuilders {
    fn new(capture_count: usize, log_mode: bool) -> Self {
        Self {
            url: StringBuilder::new(),
            rownum: Int64Builder::new(),
            date: Date32Builder::new(),
            time: Time64MicrosecondBuilder::new(),
            unix: Int64Builder::new(),
            hash: Int64Builder::new(),
            header: StringBuilder::new(),
            message: StringBuilder::new(),
            offset: Int64Builder::new(),
            lines: Int32Builder::new(),
            tokens: (0..if log_mode {
                super::options::LOG_COLUMNS.len()
            } else {
                0
            })
                .map(|_| StringBuilder::new())
                .collect(),
            captures: (0..capture_count).map(|_| StringBuilder::new()).collect(),
            rows: 0,
            offsets: super::timestamp::OffsetCache::default(),
            bytes: 0,
        }
    }

    /// Start the next batch's accounting; the builders reset themselves.
    fn reset(&mut self) {
        self.rows = 0;
        self.bytes = 0;
    }
}

/// Parse one record into one row of every column builder.
///
/// Every value comes from the [`TextLine`] accessors, which is what keeps the
/// columns and a Rust caller's reads from drifting into two implementations.
fn append_row(builders: &mut RowBuilders, line: &TextLine<'_>, log_mode: bool) -> Result<()> {
    builders.rows += 1;
    // The record's own decoded length, plus the terminator that ended it.
    builders.bytes += line.bytes().len() + 1;
    builders.url.append_value(line.url());
    builders.rownum.append_value(line.rownum());

    let offset = i64::try_from(line.offset()).map_err(|_| {
        row_error(
            line,
            "offset",
            format_smolstr!(
                "expected a record offset within the signed 64-bit range, got {}",
                line.offset()
            ),
        )
    })?;
    builders.offset.append_value(offset);
    builders.lines.append_value(line.line_count());
    builders.hash.append_value(line.hash()?);

    // The message is appended span by span, so a header spliced out of the
    // middle of a line never materializes a joined `String`.
    let [lead, tail] = line.message_parts()?;
    if tail.is_empty() {
        builders.message.append_value(lead);
    } else {
        // `GenericStringBuilder` implements `std::fmt::Write`, so the spans go
        // straight into the builder's own buffer and `append_value("")` closes
        // the row - the documented write-then-append pattern.
        builders
            .message
            .write_str(lead)
            .and_then(|()| builders.message.write_str(tail))
            .map_err(|_| Error::from(crate::arrow::Error::internal("text::line::message")))?;
        builders.message.append_value("");
    }

    let Some(header) = line.header()? else {
        // The preamble a rotated file starts with, or a record whose opening
        // line a separate header expression did not match: one rule for "no
        // header here", not two.
        builders.date.append_null();
        builders.time.append_null();
        builders.unix.append_null();
        builders.header.append_null();
        for token in &mut builders.tokens {
            token.append_null();
        }
        for capture in &mut builders.captures {
            capture.append_null();
        }
        return Ok(());
    };
    builders.header.append_value(header);

    if log_mode {
        let tokens = super::log::recognized(header);
        for (builder, value) in builders.tokens.iter_mut().zip(tokens) {
            match value {
                Some(value) => builder.append_value(value),
                None => builder.append_null(),
            }
        }
    }

    append_timestamp(builders, line, header)?;

    for (index, builder) in builders.captures.iter_mut().enumerate() {
        match line.capture_at(index)? {
            Some(value) => builder.append_value(value),
            None => builder.append_null(),
        }
    }
    Ok(())
}

/// Read the entry timestamp and append the three temporal columns.
fn append_timestamp(builders: &mut RowBuilders, line: &TextLine<'_>, header: &str) -> Result<()> {
    let options = line.options();
    let source = match options.timestamp_capture() {
        Some(name) => line.capture(name)?.ok_or_else(|| {
            row_error(
                line,
                "unix",
                format_smolstr!(
                    "expected the timestamp capture {name:?} to participate in the matched header \
                     {:?}",
                    crate::text::elide_to(header, crate::text::ERROR_TEXT_LIMIT)
                ),
            )
        })?,
        None => header,
    };

    let (count, unit, offset) = super::timestamp::read(source, options, &mut builders.offsets)
        .map_err(|error| timestamp_error(line, source, &error))?;
    let per = iso::per_second(unit)
        .ok_or_else(|| Error::from(crate::arrow::Error::internal("text::line::per_second")))?;

    // `date` and `time` are the civil reading as written; only `unix` moves
    // with the zone.
    let day = per * 86_400;
    let days = count.div_euclid(day);
    let in_day = count.rem_euclid(day);
    let date = i32::try_from(days)
        .map_err(|_| Error::from(crate::arrow::Error::internal("text::line::civil_date")))?;
    // Truncation, never rounding: a sub-microsecond clock reading is only fully
    // recoverable from `unix`.
    let micros = if per > 1_000_000 {
        in_day / (per / 1_000_000)
    } else {
        in_day * (1_000_000 / per)
    };
    let instant = count.checked_sub(offset.checked_mul(per).unwrap_or(i64::MAX));
    let nanos = instant
        .and_then(|instant| instant.checked_mul(1_000_000_000 / per))
        .ok_or_else(|| {
            row_error(
                line,
                "unix",
                format_smolstr!(
                    "expected a timestamp within the 64-bit nanosecond range (1677-09-21 to \
                     2262-04-11), got {:?}",
                    crate::text::elide_to(header, crate::text::ERROR_TEXT_LIMIT)
                ),
            )
        })?;
    builders.date.append_value(date);
    builders.time.append_value(micros);
    builders.unix.append_value(nanos);
    Ok(())
}

// ---------------------------------------------------------------------------
// The write half: Arrow batches out as text lines.
//
// This is what makes `Text` a full media rather than a read-only projection:
// `write_arrow_batch_reader` and `append_arrow_batch_reader` land here when
// the record options say text, exactly as they land in the IPC encoder when
// they say IPC.
// ---------------------------------------------------------------------------

/// Replace a handle's contents with `batches` rendered as text lines.
///
/// Each row renders as one line - the `header` column when present and
/// non-null, a single space, then the `message` column; a message holding
/// interior newlines writes back as the multi-line record it was read from.
/// A batch with no `message` column but exactly one `utf8` column writes that
/// column, so "write these strings as lines" needs no rename.
///
/// The rendered value is held whole before anything reaches the handle - the
/// same shape as the IPC encoder, and for the same reason: a failure mid-way
/// must leave the resource exactly as it was.
///
/// # Errors
///
/// Returns an error when no column can be read as the line text, or the
/// handle's resize or write failure.
pub(crate) fn write_arrow_lines(
    handle: &mut (impl IOBase + ?Sized),
    batches: BatchReader,
    options: &super::record::TextOptions,
) -> Result<()> {
    use crate::generic::IORecordOptions;

    let rendered = rendered_lines(batches, &options.lines)?;
    let encoded = handle.codec().dump_with_level(&rendered, options.level())?;
    handle.write_all_bytes(&encoded)?;
    Ok(())
}

/// Add `batches` after a handle's current lines.
///
/// A stored final line without its terminator is closed first, so the first
/// appended row never merges into it. An uncoded handle appends in place at
/// its current end. A content coding is
/// not appendable, so a coded handle decodes its value, extends it, and
/// stores the whole coding again - stated rather than hidden, because it is
/// the coding's cost, not the append's.
///
/// # Errors
///
/// Returns an error when no column can be read as the line text, or the
/// handle's read, resize, or write failure.
pub(crate) fn append_arrow_lines(
    handle: &mut (impl IOBase + ?Sized),
    batches: BatchReader,
    options: &super::record::TextOptions,
) -> Result<()> {
    use crate::generic::IORecordOptions;

    let rendered = rendered_lines(batches, &options.lines)?;
    let linesep = options.lines.write_linesep();
    let codec = handle.codec();
    if matches!(codec, crate::Codec::Identity) {
        let mut offset = handle.size();
        // A stored final line may lack its terminator; appending straight
        // after it would merge the first new row into it, so the terminator
        // is supplied first.
        if offset > 0 {
            let mut last = [0_u8; 1];
            handle.pread_exact(offset - 1, &mut last)?;
            if Some(&last[0]) != linesep.last() {
                handle.pwrite_all(offset, linesep)?;
                offset += linesep.len() as u64;
            }
        }
        handle.pwrite_all(offset, &rendered)?;
        return handle.flush();
    }
    let mut decoded = if handle.is_empty() {
        Vec::new()
    } else {
        codec.load(&handle.read_all_bytes()?)?
    };
    if decoded.last().is_some() && decoded.last() != linesep.last() {
        decoded.extend_from_slice(linesep);
    }
    decoded.extend_from_slice(&rendered);
    let encoded = codec.dump_with_level(&decoded, options.level())?;
    handle.write_all_bytes(&encoded)?;
    Ok(())
}

/// Render every batch's rows as terminated lines, into one buffer.
///
/// Held whole deliberately: the caller publishes it in one write, so a
/// failure while rendering leaves the resource untouched.
fn rendered_lines(batches: BatchReader, options: &TextLineOptions) -> Result<Vec<u8>> {
    let schema = batches.schema();
    let columns = LineColumns::resolve(schema.as_ref())?;
    let linesep = options.write_linesep();
    let mut rendered = Vec::new();
    for batch in batches {
        let batch = batch.map_err(crate::arrow::from_reader_error)?;
        columns.render(&batch, linesep, &mut rendered)?;
    }
    Ok(rendered)
}

/// Where a batch's line text lives: a `message` column, its optional
/// `header`, or the single `utf8` column a plain batch holds.
struct LineColumns {
    message: usize,
    header: Option<usize>,
}

impl LineColumns {
    /// Resolve the line columns once per reader, ASCII case-insensitively -
    /// the way every cast matches names.
    fn resolve(schema: &arrow_schema::Schema) -> Result<Self> {
        let named = |name: &str| {
            schema
                .fields()
                .iter()
                .position(|field| field.name().eq_ignore_ascii_case(name))
        };
        if let Some(message) = named("message") {
            return Ok(Self {
                message,
                header: named("header"),
            });
        }
        let mut texts = schema
            .fields()
            .iter()
            .enumerate()
            .filter(|(_, field)| field.data_type() == &arrow_schema::DataType::Utf8);
        if let (Some((only, _)), None) = (texts.next(), texts.next()) {
            return Ok(Self {
                message: only,
                header: None,
            });
        }
        Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: crate::text::expected_got(
                "rows a line can be rendered from (a `message` utf8 column, optionally beside a \
                 `header` one, or exactly one utf8 column)",
                format_smolstr!(
                    "columns {:?}",
                    schema
                        .fields()
                        .iter()
                        .map(|field| field.name().as_str())
                        .collect::<Vec<_>>()
                ),
            ),
        })
    }

    /// Append one batch's rows to `rendered`, each line terminated.
    fn render(&self, batch: &RecordBatch, linesep: &[u8], rendered: &mut Vec<u8>) -> Result<()> {
        use arrow_array::Array as _;

        let message = text_column(batch, self.message)?;
        let header = self
            .header
            .map(|index| text_column(batch, index))
            .transpose()?;
        for row in 0..batch.num_rows() {
            if let Some(header) = header {
                if !header.is_null(row) && !header.value(row).is_empty() {
                    rendered.extend_from_slice(header.value(row).as_bytes());
                    if !message.is_null(row) && !message.value(row).is_empty() {
                        rendered.push(b' ');
                    }
                }
            }
            if !message.is_null(row) {
                rendered.extend_from_slice(message.value(row).as_bytes());
            }
            rendered.extend_from_slice(linesep);
        }
        Ok(())
    }
}

/// Borrow one column as text, or say what it is instead.
fn text_column(batch: &RecordBatch, index: usize) -> Result<&arrow_array::StringArray> {
    batch
        .column(index)
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .ok_or_else(|| Error::InvalidRecord {
            path: format_smolstr!("$.{}", batch.schema().field(index).name()),
            reason: crate::text::expected_got(
                "a utf8 column to render lines from",
                format_smolstr!("{}", batch.schema().field(index).data_type()),
            ),
        })
}

/// A typed per-row failure: the column path, the row, and the resource.
fn row_error(line: &TextLine<'_>, column: &str, reason: SmolStr) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$[{}].{column}", line.rownum() - 1),
        reason: format_smolstr!("{reason} in row {} of {}", line.rownum(), line.url()),
    }
}

/// A malformed timestamp inside a matched header, with its byte position.
fn timestamp_error(line: &TextLine<'_>, text: &str, error: &Error) -> Error {
    let detail = match error {
        Error::Parse {
            position, reason, ..
        } => format_smolstr!("{reason} at byte {position}"),
        other => other.to_smolstr(),
    };
    row_error(
        line,
        "header",
        format_smolstr!(
            "expected an ISO datetime opening {:?} ({detail})",
            crate::text::elide_to(text, crate::text::ERROR_TEXT_LIMIT)
        ),
    )
}
