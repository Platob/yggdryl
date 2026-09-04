//! `text/plain` rows through the shared Scalar/Arrow record boundary.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::Arc;

use arrow_array::{Array as _, BinaryArray, RecordBatch};
use arrow_schema::{DataType as ArrowDataType, Schema};
use smol_str::{SmolStr, format_smolstr};

use crate::arrow::BatchReader;
use crate::holder::Buffer;
use crate::holder::Holder;
use crate::media::IORecordOptions;
use crate::types::ascii::iso;
use crate::{Codec, DataType, Error, Result, Scalar, TimeUnit, Timezone};
use crate::{Cursor, IOBase};

use super::options::TextOptions;
use super::reader::Lines;

/// Decode one borrowed leaf into ordinary record batches.
pub(crate) fn read_arrow_reader(
    handle: &(impl IOBase + ?Sized),
    options: &TextOptions,
) -> Result<BatchReader> {
    let url = url_text(handle);
    read_owned_arrow_reader_at(reopened(handle)?, url, options)
}

/// Decode an owned leaf without retaining decoded pages in its caller.
pub(crate) fn read_owned_arrow_reader<H: IOBase + 'static>(
    handle: H,
    options: &TextOptions,
) -> Result<BatchReader> {
    let url = url_text(&handle);
    read_owned_arrow_reader_at(handle, url, options)
}

fn read_owned_arrow_reader_at<H: IOBase + 'static>(
    handle: H,
    url: SmolStr,
    options: &TextOptions,
) -> Result<BatchReader> {
    let codings = handle.media_type().encodings().to_vec();
    let mut source: Box<dyn Read + Send + 'static> = Box::new(Cursor::new(handle));
    for coding in codings.iter().rev() {
        source = Codec::from_mime_type(coding).reader_send(source);
    }

    let mut raw = RawRows::new(source, url, Arc::new(options.clone()));
    let adaptive = options.autotype()
        && options.dtype().is_none()
        && options.capture_names().len() > 0
        && options.max_row_size() != Some(0)
        && options.max_byte_size() != Some(0);
    let mut prefetched = VecDeque::new();
    let capture_dtypes = if adaptive {
        let mut sample_size = options
            .batch_row_size()
            .unwrap_or(crate::media::DEFAULT_RECORD_BATCH_ROW_SIZE)
            .max(1);
        if let Some(maximum) = options.max_row_size() {
            sample_size = sample_size.min(usize::try_from(maximum).unwrap_or(usize::MAX));
        }
        for _ in 0..sample_size {
            let Some(row) = raw.next() else { break };
            prefetched.push_back(row?);
        }
        inferred_capture_dtypes(&prefetched, options)?
    } else {
        vec![DataType::Utf8; options.capture_names().len()]
    };
    let field = options.source_field(&capture_dtypes)?;
    let rows = Records {
        raw,
        prefetched,
        capture_dtypes,
        timezone: options.timezone().cloned(),
    };
    Ok(crate::arrow::rows::result_reader(
        &field,
        rows,
        options.batch_row_size(),
        None,
        options.max_row_size(),
    )?)
}

/// Count physical lines without materializing rows or Arrow arrays.
pub(crate) fn row_size(handle: &(impl IOBase + ?Sized), options: &TextOptions) -> Result<u64> {
    let codings = handle.media_type().encodings().to_vec();
    let mut source: Box<dyn Read + '_> =
        Box::new(handle.pstream_bytes(0, crate::DEFAULT_STREAM_BATCH_SIZE)?);
    for coding in codings.iter().rev() {
        source = Codec::from_mime_type(coding).reader(source);
    }
    let mut lines = Lines::new(source);
    let mut rows = 0_u64;
    while let Some(line) = lines.next_line(options.linesep()) {
        line?;
        rows = rows.checked_add(1).ok_or_else(|| Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("logical row count exceeds u64::MAX"),
        })?;
    }
    Ok(rows)
}

/// Return an owned handle for a reader that must outlive this borrow.
fn reopened(handle: &(impl IOBase + ?Sized)) -> Result<Holder> {
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

fn url_text(handle: &(impl IOBase + ?Sized)) -> SmolStr {
    handle.url().map_or_else(
        || SmolStr::new_static(""),
        |url| SmolStr::new(url.to_string()),
    )
}

/// One parsed line before capture types are fixed.
struct RawRow {
    url: SmolStr,
    rownum: i64,
    body: Arc<[u8]>,
    captures: Vec<Option<SmolStr>>,
}

/// Physical lines parsed into the three base values and raw captures.
struct RawRows<R> {
    lines: Lines<R>,
    url: SmolStr,
    options: Arc<TextOptions>,
    rownum: i64,
    done: bool,
}

impl<R: Read> RawRows<R> {
    fn new(source: R, url: SmolStr, options: Arc<TextOptions>) -> Self {
        Self {
            lines: Lines::new(source),
            url,
            options,
            rownum: 0,
            done: false,
        }
    }

    fn parse(options: &TextOptions, url: &str, line: &[u8], rownum: i64) -> Result<RawRow> {
        let mut captures = vec![None; options.capture_names().len()];
        let mut body = Vec::with_capacity(line.len());
        if let Some(rowheader) = options.rowheader_regex() {
            if let Some(found) = rowheader.captures(line) {
                if let Some(whole) = found.get(0) {
                    body.extend_from_slice(&line[..whole.start()]);
                    body.extend_from_slice(&line[whole.end()..]);
                } else {
                    body.extend_from_slice(line);
                }
                for (target, (index, name)) in captures.iter_mut().zip(
                    rowheader
                        .capture_names()
                        .enumerate()
                        .filter_map(|(index, name)| name.map(|name| (index, name))),
                ) {
                    let Some(value) = found.get(index) else {
                        continue;
                    };
                    let value = std::str::from_utf8(value.as_bytes()).map_err(|error| {
                        row_error(
                            rownum,
                            url,
                            name,
                            format_smolstr!(
                                "expected a UTF-8 row-header capture, got invalid byte at {}",
                                error.valid_up_to()
                            ),
                        )
                    })?;
                    *target = Some(SmolStr::new(value));
                }
            } else {
                body.extend_from_slice(line);
            }
        } else {
            body.extend_from_slice(line);
        }

        let mut start = 0;
        let mut end = body.len();
        if let Some(lstrip) = options.lstrip_regex() {
            if let Some(found) = lstrip
                .find(&body[start..end])
                .filter(|found| found.start() == 0)
            {
                start += found.end();
            }
        }
        if let Some(rstrip) = options.rstrip_regex() {
            if let Some(found) = rstrip
                .find_iter(&body[start..end])
                .filter(|found| found.end() == end - start)
                .last()
            {
                end = start + found.start();
            }
        }

        Ok(RawRow {
            url: SmolStr::new(url),
            rownum,
            body: Arc::from(&body[start..end]),
            captures,
        })
    }
}

impl<R: Read> Iterator for RawRows<R> {
    type Item = Result<RawRow>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let options = Arc::clone(&self.options);
        let url = self.url.clone();
        let line = match self.lines.next_line(options.linesep())? {
            Ok(line) => line,
            Err(error) => {
                self.done = true;
                return Some(Err(error));
            }
        };
        let Some(rownum) = self.rownum.checked_add(1) else {
            self.done = true;
            return Some(Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.rownum"),
                reason: SmolStr::new_static("text row number exceeds i64::MAX"),
            }));
        };
        self.rownum = rownum;
        Some(Self::parse(&options, &url, line, rownum))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Inferred {
    Bool,
    Int,
    Float,
    Date,
    Time(TimeUnit),
    Timestamp(TimeUnit, Option<Timezone>),
    Text,
}

fn inferred_capture_dtypes(
    rows: &VecDeque<RawRow>,
    options: &TextOptions,
) -> Result<Vec<DataType>> {
    let mut inferred = vec![None; options.capture_names().len()];
    for row in rows {
        for (slot, value) in inferred.iter_mut().zip(&row.captures) {
            let Some(value) = value else { continue };
            let next = infer(value, options.timezone());
            *slot = Some(match slot.take() {
                Some(current) => merge_inferred(current, next),
                None => next,
            });
        }
    }
    inferred
        .into_iter()
        .map(|kind| inferred_dtype(kind.unwrap_or(Inferred::Text)))
        .collect()
}

fn infer(value: &str, timezone: Option<&Timezone>) -> Inferred {
    if matches!(value, "true" | "false") {
        return Inferred::Bool;
    }
    if value.parse::<i64>().is_ok() {
        return Inferred::Int;
    }
    if value.parse::<f64>().is_ok_and(f64::is_finite) {
        return Inferred::Float;
    }
    if let Ok((_, unit, zone)) = iso::parse_timestamp(value) {
        return Inferred::Timestamp(unit, Some(timezone.cloned().unwrap_or(zone)));
    }
    if let Ok((_, unit)) = iso::parse_datetime(value) {
        return Inferred::Timestamp(unit, timezone.cloned());
    }
    if iso::parse_date(value).is_ok() {
        return Inferred::Date;
    }
    if let Ok((_, unit)) = iso::parse_time(value) {
        return Inferred::Time(unit);
    }
    Inferred::Text
}

fn merge_inferred(left: Inferred, right: Inferred) -> Inferred {
    use Inferred::{Bool, Date, Float, Int, Text, Time, Timestamp};
    match (left, right) {
        (Text, _) | (_, Text) => Text,
        (Bool, Bool) => Bool,
        (Int, Int) => Int,
        (Int | Float, Int | Float) => Float,
        (Date, Date) => Date,
        (Time(left), Time(right)) => Time(finer_unit(left, right)),
        (Timestamp(left, left_zone), Timestamp(right, right_zone)) if left_zone == right_zone => {
            Timestamp(finer_unit(left, right), left_zone)
        }
        _ => Text,
    }
}

fn finer_unit(left: TimeUnit, right: TimeUnit) -> TimeUnit {
    if unit_rank(left) >= unit_rank(right) {
        left
    } else {
        right
    }
}

const fn unit_rank(unit: TimeUnit) -> u8 {
    match unit {
        TimeUnit::Second => 0,
        TimeUnit::Millisecond => 1,
        TimeUnit::Microsecond => 2,
        TimeUnit::Nanosecond => 3,
        TimeUnit::Day | TimeUnit::YearMonth | TimeUnit::DayTime | TimeUnit::MonthDayNano => 4,
    }
}

fn inferred_dtype(kind: Inferred) -> Result<DataType> {
    Ok(match kind {
        Inferred::Bool => DataType::Boolean,
        Inferred::Int => DataType::Int64,
        Inferred::Float => DataType::Float64,
        Inferred::Date => DataType::Date32,
        Inferred::Time(unit) => DataType::time(unit)?,
        Inferred::Timestamp(unit, zone) => DataType::Timestamp(unit, zone),
        Inferred::Text => DataType::Utf8,
    })
}

/// Raw rows converted under the schema fixed by the first batch.
struct Records<R> {
    raw: RawRows<R>,
    prefetched: VecDeque<RawRow>,
    capture_dtypes: Vec<DataType>,
    timezone: Option<Timezone>,
}

impl<R: Read> Iterator for Records<R> {
    type Item = Result<Scalar>;

    fn next(&mut self) -> Option<Self::Item> {
        let raw = self
            .prefetched
            .pop_front()
            .map(Ok)
            .or_else(|| self.raw.next())?;
        Some(raw.and_then(|row| self.convert(row)))
    }
}

impl<R: Read> Records<R> {
    fn convert(&self, row: RawRow) -> Result<Scalar> {
        let mut entries = Vec::with_capacity(3 + row.captures.len());
        entries.push((SmolStr::new_static("url"), Scalar::from(row.url.clone())));
        entries.push((SmolStr::new_static("rownum"), Scalar::from(row.rownum)));
        entries.push((SmolStr::new_static("body"), Scalar::from(row.body)));
        for ((name, value), dtype) in self
            .raw
            .options
            .capture_names()
            .zip(row.captures)
            .zip(&self.capture_dtypes)
        {
            let value = match value {
                Some(value) => parse_capture(&value, dtype, self.timezone.as_ref())
                    .map_err(|reason| row_error(row.rownum, &row.url, name, reason))?,
                None => Scalar::Null,
            };
            entries.push((SmolStr::new(name), value));
        }
        Scalar::from_record(entries)
    }
}

fn parse_capture(
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
        DataType::Timestamp(unit, Some(zone)) => {
            // A reading that names its own offset is the crate's; a naive one
            // is autotyping's own rule, a wall clock in the column's zone.
            if let Ok(instant) = Scalar::from_temporal_text(dtype, value) {
                return Ok(instant);
            }
            let (local, source) = iso::parse_datetime(value).map_err(|_| invalid())?;
            let count =
                zoned_count(local, source, timezone.unwrap_or(zone)).map_err(|_| invalid())?;
            let count = rescale(count, source, *unit).ok_or_else(invalid)?;
            Scalar::datetime64(count, *unit, zone.clone()).map_err(|_| invalid())
        }
        DataType::Timestamp(..) => Scalar::from_temporal_text(dtype, value).map_err(|_| invalid()),
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
    zone.clone()
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

fn row_error(rownum: i64, url: &str, column: &str, reason: SmolStr) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$[{}].{column}", rownum.saturating_sub(1)),
        reason: format_smolstr!("{reason} in row {rownum} of {url}"),
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
            let source = handle.pstream_bytes(0, crate::DEFAULT_STREAM_BATCH_SIZE)?;
            let mut decoder = codec.reader(source);
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
