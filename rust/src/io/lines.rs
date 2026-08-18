//! The Arrow projection of matched line records.
//!
//! This module is a *text-line* surface beside [`IOBase::read_lines`] and
//! [`IOBase::read_lines_matching`], **not a fourth record method**: the record
//! surface stays exactly `read_arrow_batch_reader`, `write_arrow_batch_reader`,
//! and `append_arrow_batch_reader`, and nothing here decodes a record
//! encoding. What streams out of [`IOBase::read_arrow_lines`] and
//! [`IOBase::into_arrow_lines`] is the same decoded text
//! `read_lines_matching` yields - one record per header-pattern match, content
//! codings peeled as streams - projected into Arrow batches with one batch in
//! memory at a time.
//!
//! # The schema
//!
//! A non-null Struct [`Field`] is the schema, and every base column is a
//! datatype the strict Iceberg codec ([`PrimitiveType::from_data_type`])
//! accepts **unchanged**, so the parsed batches append into an Iceberg table
//! exactly as declared - not merely after widening:
//!
//! | column    | datatype              | Iceberg  | meaning |
//! | --------- | --------------------- | -------- | ------- |
//! | `url`     | `utf8`                | `string` | the resource's canonical [`Url`](crate::Url) display |
//! | `rownum`  | `int64`               | `long`   | 1-based record index within the resource |
//! | `date`    | `date32`              | `date`   | the entry's civil date |
//! | `time`    | `time64(us)`          | `time`   | the clock reading, truncated - never rounded - to microseconds (Iceberg's `time` has no nanosecond form; full precision lives in `unix`) |
//! | `unix`    | `int64`               | `long`   | total nanoseconds since the Unix epoch, naive - no zone is applied |
//! | `hash`    | `int64`               | `long`   | the stable FNV-1a hash of `message` **only**, the `u64` state reinterpreted as two's-complement `i64` (`u64 as i64`, bit pattern preserved) because Iceberg has no unsigned types; deterministic across runs, so it serves as a dedupe or join key |
//! | `header`  | `utf8`                | `string` | the exact text the pattern matched |
//! | `message` | `utf8`                | `string` | the record with the header match removed, then trimmed |
//! | `offset`  | `int64`               | `long`   | byte offset of the record's first line in the *decoded* stream - the resume key a tailing reader seeks back to |
//! | `lines`   | `int32`               | `int`    | how many lines the record spans - a free flag for stack traces |
//!
//! Then one nullable column per **named capture group** of the header
//! pattern, in group order - the primary custom-field mechanism. A capture
//! whose whole sub-pattern is one of a closed table of exact spellings types
//! itself - `(?<thread_id>\d+)` or `[0-9]+` is `int64`, `(?<qty>\d+\.\d+)`
//! is `float64` - and every other capture is `utf8`; the table is exact
//! because inference is deterministic, never a guess about what a regex
//! might match. [`LineRecordOptions::try_with_capture_types`] declares a
//! capture's datatype explicitly - `(?<price>[0-9.]+)` as `decimal(9, 2)`,
//! or an inferred numeric back to `utf8` - and the typed columns parse
//! through the one cast definition as each batch closes, strictly: a
//! captured text the datatype cannot read is an error, never a silent null.
//! Then the constant [`custom_fields`](LineRecordOptions::custom_fields)
//! columns, typed from their [`Value`]. Capture declarations and custom
//! columns alike are validated against the Iceberg vocabulary when the
//! options are built, so an unspellable column fails before the first batch
//! rather than at table-append time. [`schema_from_pattern`] answers the
//! emitted root straight from a pattern - the schema without a reader, for
//! creating the table before the first log line exists.
//!
//! A record whose opening line the pattern did not match - the preamble a
//! rotated file starts with - has null `date`, `time`, `unix`, `header`, and
//! capture columns, and the whole record as `message`; those five stay
//! nullable while `url`, `rownum`, `hash`, `message`, `offset`, and `lines`
//! never are.
//!
//! The entry timestamp is read from the front of the matched header by the
//! flexible ISO parser - `T`, `t`, or a space between date and clock, and an
//! optional fraction whose digits may be grouped with `_`, so
//! `2024-02-01 10:00:00.000_000` reads exactly as
//! `2024-02-01T10:00:00.000000`. A malformed timestamp inside a *matched*
//! header is a typed error naming the row and byte position, never a silent
//! null. [`LineRecordOptions::try_with_timestamp_capture`] points the parser
//! at a named capture instead of the header's front.
//!
//! ```
//! use yggdryl::io::{Buffer, IOBase, LineRecordOptions};
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
//! let options = LineRecordOptions::new(
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

use std::borrow::Cow;
use std::io::Read;
use std::sync::Arc;

use arrow_array::builder::{
    Date32Builder, Int32Builder, Int64Builder, StringBuilder, Time64MicrosecondBuilder,
};
use arrow_array::{ArrayRef, RecordBatch, UInt32Array};
use arrow_schema::{ArrowError, SchemaRef};
use smol_str::{SmolStr, ToSmolStr, format_smolstr};

use super::{Buffer, IOBase, LineRecords};
use crate::arrow::{BatchReader, schema_from_field, scalar_array};
use crate::generic::{Holder, Value, iso};
use crate::iceberg::PrimitiveType;
use crate::{DataType, Error, Field, IOKind, Result, TimeUnit};

/// The columns every line projection opens with, in emission order.
const BASE_COLUMNS: [&str; 10] = [
    "url", "rownum", "date", "time", "unix", "hash", "header", "message", "offset", "lines",
];

/// The name of the root Struct Field the projection reports.
///
/// The record surface infers roots under the same name, so a schema read off
/// this projection and one read off a record encoding compose without rename.
const ROOT_NAME: &str = "row";

/// The settings a line-record projection runs under.
///
/// Construction compiles and validates the header pattern; every later
/// mutation revalidates, so holding a `LineRecordOptions` is holding a schema
/// that is already known to be emittable - [`Self::schema`] answers without a
/// resource in sight, which is what a caller creating an Iceberg table before
/// the first log line needs.
#[derive(Clone, Debug)]
pub struct LineRecordOptions {
    pattern: regex_lite::Regex,
    /// The pattern's named capture groups, in group order.
    captures: Vec<SmolStr>,
    batch_size: Option<usize>,
    timestamp_capture: Option<SmolStr>,
    custom_fields: Vec<(SmolStr, Value)>,
    /// Declared capture column datatypes, overriding the inferred ones.
    capture_types: Vec<(SmolStr, DataType)>,
    /// The emitted root, rebuilt on every effective mutation.
    schema: Field,
}

impl LineRecordOptions {
    /// Rows per batch when none is declared: one batch in memory at a time,
    /// sized so a batch is worth the per-batch bookkeeping.
    pub const DEFAULT_BATCH_SIZE: usize = 1024;

    /// Compile a header pattern into line-record options.
    ///
    /// `pattern` opens a record exactly as [`IOBase::read_lines_matching`]
    /// does; its named capture groups become nullable columns in group
    /// order, typed by the capture's own sub-pattern where the deterministic
    /// table recognizes it - `(?<thread_id>\d+)` is an `int64` column - and
    /// `utf8` otherwise. [`Self::try_with_capture_types`] overrides either
    /// way.
    ///
    /// # Errors
    ///
    /// Returns an error when the pattern does not parse or a capture group
    /// name collides with a base column.
    pub fn new(pattern: &str) -> Result<Self> {
        let pattern = regex_lite::Regex::new(pattern).map_err(|error| Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: format_smolstr!("expected a valid line pattern: {error}"),
        })?;
        let captures: Vec<SmolStr> = pattern
            .capture_names()
            .flatten()
            .map(SmolStr::new)
            .collect();
        for (index, name) in captures.iter().enumerate() {
            if is_base_column(name) {
                return Err(collision(name, "a base column of the line projection"));
            }
            // The regex engine keeps `level` and `LEVEL` distinct; column
            // names resolve ASCII case-insensitively everywhere a cast or a
            // selection matches them, so the pair would be ambiguous.
            if captures[..index]
                .iter()
                .any(|held| held.eq_ignore_ascii_case(name))
            {
                return Err(collision(name, "another capture group of the pattern"));
            }
        }
        let schema = build_schema(
            &resolved_capture_types(pattern.as_str(), &captures, &[]),
            &captures,
            &[],
        )?;
        Ok(Self {
            pattern,
            captures,
            batch_size: None,
            timestamp_capture: None,
            custom_fields: Vec::new(),
            capture_types: Vec::new(),
            schema,
        })
    }

    /// Borrow the header pattern's source text.
    pub fn pattern(&self) -> &str {
        self.pattern.as_str()
    }

    /// Borrow the pattern's named capture groups, in group and column order.
    pub fn capture_names(&self) -> impl Iterator<Item = &str> {
        self.captures.iter().map(SmolStr::as_str)
    }

    /// Return the row-per-batch bound, if one was declared.
    pub const fn batch_size(&self) -> Option<usize> {
        self.batch_size
    }

    /// Set the row-per-batch bound; `None` restores the default.
    pub fn set_batch_size(&mut self, batch_size: Option<usize>) {
        self.batch_size = batch_size;
    }

    /// Return these options with a row-per-batch bound.
    #[must_use]
    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.set_batch_size(Some(batch_size));
        self
    }

    /// Return the capture group the entry timestamp is read from, if any.
    ///
    /// Unset, the timestamp is parsed off the front of the matched header.
    pub fn timestamp_capture(&self) -> Option<&str> {
        self.timestamp_capture.as_deref()
    }

    /// Point the timestamp parser at a named capture group.
    ///
    /// `None` restores the default - the front of the matched header.
    ///
    /// # Errors
    ///
    /// Returns an error naming the available captures when `capture` is not a
    /// named group of the pattern. Failure leaves the options unchanged.
    pub fn set_timestamp_capture(&mut self, capture: Option<SmolStr>) -> Result<()> {
        if let Some(name) = &capture {
            if !self.captures.iter().any(|held| held == name) {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$.{name}"),
                    reason: crate::text::expected_got(
                        format_smolstr!(
                            "a named capture group of the pattern ({:?})",
                            self.captures
                        ),
                        format_smolstr!("{name:?}"),
                    ),
                });
            }
        }
        self.timestamp_capture = capture;
        Ok(())
    }

    /// Return these options reading the timestamp from a named capture.
    ///
    /// # Errors
    ///
    /// Returns an error when `capture` is not a named group of the pattern.
    pub fn try_with_timestamp_capture(mut self, capture: impl Into<SmolStr>) -> Result<Self> {
        self.set_timestamp_capture(Some(capture.into()))?;
        Ok(self)
    }

    /// Borrow the constant columns appended to every row, in column order.
    pub fn custom_fields(&self) -> &[(SmolStr, Value)] {
        &self.custom_fields
    }

    /// Replace the constant columns appended to every row.
    ///
    /// Each value's datatype is validated through the strict Iceberg codec
    /// ([`PrimitiveType::from_data_type`]) here, at option construction, so a
    /// column Iceberg cannot spell - an unsigned integer, a non-microsecond
    /// time - fails before the first batch rather than at table-append time.
    ///
    /// # Errors
    ///
    /// Returns the codec's own rejection under the column's path, or a
    /// collision with a base, capture, or earlier custom column. Failure
    /// leaves the options unchanged.
    pub fn set_custom_fields(&mut self, custom_fields: Vec<(SmolStr, Value)>) -> Result<()> {
        let schema = build_schema_checked(
            self.pattern.as_str(),
            &self.captures,
            &self.capture_types,
            &custom_fields,
        )?;
        self.custom_fields = custom_fields;
        self.schema = schema;
        Ok(())
    }

    /// Return these options with constant columns appended to every row.
    ///
    /// This is how a caller stamps `source`, `session`, or `venue` onto a
    /// file's rows - and, marked as partition columns in a declared Iceberg
    /// schema, how one file's rows land in one partition.
    ///
    /// # Errors
    ///
    /// Returns an error when a column's datatype has no Iceberg spelling or
    /// its name collides.
    pub fn try_with_custom_fields<I, N>(mut self, custom_fields: I) -> Result<Self>
    where
        I: IntoIterator<Item = (N, Value)>,
        N: Into<SmolStr>,
    {
        self.set_custom_fields(
            custom_fields
                .into_iter()
                .map(|(name, value)| (name.into(), value))
                .collect(),
        )?;
        Ok(self)
    }

    /// Borrow the declared capture column datatypes, in declaration order.
    pub fn capture_types(&self) -> &[(SmolStr, DataType)] {
        &self.capture_types
    }

    /// Declare capture column datatypes, overriding the inferred ones.
    ///
    /// A declared capture parses through the one cast definition as each
    /// batch closes - `(?<price>[0-9.]+)` declared `decimal(9, 2)` lands
    /// typed - and a declaration of `utf8` turns an inferred numeric column
    /// back into text. The cast is strict: a captured text the datatype
    /// cannot read is an error, never a silent null. Each datatype passes
    /// the same Iceberg gate as a custom field, so the schema stays
    /// table-creatable as declared.
    ///
    /// # Errors
    ///
    /// Returns an error when a name is not a capture group of the pattern,
    /// is declared twice, or names a datatype the created Iceberg tables
    /// cannot declare. Failure leaves the options unchanged.
    pub fn set_capture_types(&mut self, capture_types: Vec<(SmolStr, DataType)>) -> Result<()> {
        for (index, (name, _)) in capture_types.iter().enumerate() {
            if !self.captures.iter().any(|held| held == name) {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$.{name}"),
                    reason: crate::text::expected_got(
                        format_smolstr!(
                            "a named capture group of the pattern ({:?})",
                            self.captures
                        ),
                        format_smolstr!("{name:?}"),
                    ),
                });
            }
            if capture_types[..index].iter().any(|(held, _)| held == name) {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$.{name}"),
                    reason: crate::text::expected_got(
                        "one datatype declaration per capture",
                        format_smolstr!("{name:?} declared twice"),
                    ),
                });
            }
        }
        let schema = build_schema_checked(
            self.pattern.as_str(),
            &self.captures,
            &capture_types,
            &self.custom_fields,
        )?;
        self.capture_types = capture_types;
        self.schema = schema;
        Ok(())
    }

    /// Return these options with declared capture column datatypes.
    ///
    /// # Errors
    ///
    /// Returns an error when a name is not a capture group or a datatype has
    /// no Iceberg spelling.
    pub fn try_with_capture_types<I, N>(mut self, capture_types: I) -> Result<Self>
    where
        I: IntoIterator<Item = (N, DataType)>,
        N: Into<SmolStr>,
    {
        self.set_capture_types(
            capture_types
                .into_iter()
                .map(|(name, data_type)| (name.into(), data_type))
                .collect(),
        )?;
        Ok(self)
    }

    /// Borrow the non-null Struct root the projection emits.
    ///
    /// Base columns, then one nullable column per named capture (typed by
    /// declaration, then by the deterministic sub-pattern table, then
    /// `utf8`), then the custom constant columns. Every column passes the
    /// strict Iceberg codec unchanged, so this root creates an Iceberg table
    /// as it stands.
    pub const fn schema(&self) -> &Field {
        &self.schema
    }

    /// Consume these options into the root Struct Field they emit.
    ///
    /// The owned spelling of [`Self::schema`], for a caller that built the
    /// options only to get the schema - creating a table before the first
    /// log line exists.
    pub fn into_schema(self) -> Field {
        self.schema
    }

    /// The row count a batch is closed at.
    fn effective_batch_size(&self) -> usize {
        self.batch_size.unwrap_or(Self::DEFAULT_BATCH_SIZE).max(1)
    }
}

/// Whether `name` spells a base column, compared as every cast matches names.
fn is_base_column(name: &str) -> bool {
    BASE_COLUMNS
        .iter()
        .any(|base| base.eq_ignore_ascii_case(name))
}

/// Report a column name already taken by `taken_by`.
fn collision(name: &str, taken_by: &str) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$.{name}"),
        reason: crate::text::expected_got(
            "a distinct column name",
            format_smolstr!("{name:?}, already {taken_by}"),
        ),
    }
}

/// Refuse a column datatype the created Iceberg tables cannot declare.
///
/// The strict codec is the arbiter: what it refuses here would have been
/// refused at table-append time, when rows already streamed. The v3-only
/// types - `unknown`, the nanosecond timestamps - are refused too, because
/// the tables this crate creates are format v2 and a column they cannot
/// legally declare must fail before the metadata is committed.
fn iceberg_safe(name: &str, data_type: &DataType) -> Result<()> {
    let primitive =
        PrimitiveType::from_data_type(data_type).map_err(|error| Error::InvalidRecord {
            path: format_smolstr!("$.{name}"),
            reason: error.to_smolstr(),
        })?;
    if matches!(
        primitive,
        PrimitiveType::Unknown | PrimitiveType::TimestampNs | PrimitiveType::TimestamptzNs
    ) {
        return Err(Error::InvalidRecord {
            path: format_smolstr!("$.{name}"),
            reason: crate::text::expected_got(
                "a column every Iceberg format version spells (microsecond timestamps; no \
                 null constants)",
                format_smolstr!("{primitive}, which Iceberg adds in format v3"),
            ),
        });
    }
    Ok(())
}

/// Build the emitted root after re-running every column validation.
fn build_schema_checked(
    pattern: &str,
    captures: &[SmolStr],
    capture_types: &[(SmolStr, DataType)],
    customs: &[(SmolStr, Value)],
) -> Result<Field> {
    for (name, data_type) in capture_types {
        iceberg_safe(name, data_type)?;
    }
    for (index, (name, value)) in customs.iter().enumerate() {
        if is_base_column(name) {
            return Err(collision(name, "a base column of the line projection"));
        }
        if captures.iter().any(|held| held.eq_ignore_ascii_case(name)) {
            return Err(collision(name, "a capture group of the pattern"));
        }
        if customs[..index]
            .iter()
            .any(|(held, _)| held.eq_ignore_ascii_case(name))
        {
            return Err(collision(name, "another custom field"));
        }
        let data_type = value.data_type().map_err(|error| Error::InvalidRecord {
            path: format_smolstr!("$.{name}"),
            reason: error.to_smolstr(),
        })?;
        iceberg_safe(name, &data_type)?;
    }
    build_schema(
        &resolved_capture_types(pattern, captures, capture_types),
        captures,
        customs,
    )
}

/// Resolve each capture column's datatype, in capture order.
///
/// A declaration wins; then the deterministic sub-pattern table; then `utf8`.
fn resolved_capture_types(
    pattern: &str,
    captures: &[SmolStr],
    declared: &[(SmolStr, DataType)],
) -> Vec<DataType> {
    let bodies = named_group_bodies(pattern);
    captures
        .iter()
        .map(|capture| {
            if let Some((_, data_type)) = declared.iter().find(|(name, _)| name == capture) {
                return data_type.clone();
            }
            bodies
                .iter()
                .find(|(name, _)| name == capture)
                .and_then(|(_, body)| inferred_capture_type(body))
                .unwrap_or(DataType::Utf8)
        })
        .collect()
}

/// The deterministic sub-pattern table behind capture type inference.
///
/// A capture whose *whole* body is one of these exact spellings names a
/// numeric column; every other body - however numeric it looks - stays text,
/// because inference is a closed table, never a guess about what a regex
/// might match. A declaration overrides in both directions.
fn inferred_capture_type(body: &str) -> Option<DataType> {
    match body {
        r"\d+" | "[0-9]+" | r"-?\d+" | r"[+-]?\d+" => Some(DataType::Int64),
        r"\d+\.\d+" | r"-?\d+\.\d+" | r"[+-]?\d+\.\d+" => Some(DataType::Float64),
        _ => None,
    }
}

/// The body text of each named group in an already-compiled pattern.
///
/// A byte scan, not a second regex grammar: it only pairs parentheses -
/// skipping escapes and character classes, where a parenthesis is a literal -
/// and remembers where a `(?<name>` or `(?P<name>` group's body spans. It
/// runs only on patterns the engine has already compiled, and every index it
/// slices at is an ASCII byte, which UTF-8 guarantees is a character
/// boundary.
fn named_group_bodies(pattern: &str) -> Vec<(SmolStr, SmolStr)> {
    let bytes = pattern.as_bytes();
    let mut bodies = Vec::new();
    // One entry per open group: the name and body start when it is named.
    let mut open: Vec<Option<(SmolStr, usize)>> = Vec::new();
    let mut in_class = false;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'[' if !in_class => {
                in_class = true;
                index += 1;
            }
            b']' if in_class => {
                in_class = false;
                index += 1;
            }
            b'(' if !in_class => {
                let rest = &pattern[index..];
                let name_start = if rest.starts_with("(?P<") {
                    Some(index + 4)
                } else if rest.starts_with("(?<")
                    && !rest.starts_with("(?<=")
                    && !rest.starts_with("(?<!")
                {
                    Some(index + 3)
                } else {
                    None
                };
                if let Some(start) = name_start {
                    if let Some(close) = pattern[start..].find('>') {
                        open.push(Some((
                            SmolStr::new(&pattern[start..start + close]),
                            start + close + 1,
                        )));
                        index = start + close + 1;
                        continue;
                    }
                }
                open.push(None);
                index += 1;
            }
            b')' if !in_class => {
                if let Some(Some((name, body_start))) = open.pop() {
                    bodies.push((name, SmolStr::new(&pattern[body_start..index])));
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    bodies
}

/// Assemble the root Struct Field: base, capture, then custom columns.
fn build_schema(
    capture_data_types: &[DataType],
    captures: &[SmolStr],
    customs: &[(SmolStr, Value)],
) -> Result<Field> {
    let mut fields = vec![
        DataType::Utf8.required_field("url"),
        DataType::Int64.required_field("rownum"),
        DataType::Date32.nullable_field("date"),
        DataType::Time64(TimeUnit::Microsecond).nullable_field("time"),
        DataType::Int64.nullable_field("unix"),
        DataType::Int64.required_field("hash"),
        DataType::Utf8.nullable_field("header"),
        DataType::Utf8.required_field("message"),
        DataType::Int64.required_field("offset"),
        DataType::Int32.required_field("lines"),
    ];
    for (capture, data_type) in captures.iter().zip(capture_data_types) {
        fields.push(data_type.clone().nullable_field(capture.clone()));
    }
    for (name, value) in customs {
        let data_type = value.data_type()?;
        // Only a null constant needs a nullable column; a typed constant is
        // present in every row.
        fields.push(Field::new(
            name.clone(),
            data_type,
            matches!(value, Value::Null),
        ));
    }
    Ok(DataType::from_fields(fields)?.required_field(ROOT_NAME))
}

/// Build the projection's root Struct Field straight from a header pattern.
///
/// The one-call spelling of [`LineRecordOptions::new`] followed by
/// [`into_schema`](LineRecordOptions::into_schema): the schema a pattern
/// emits - typed captures inferred from their sub-patterns - without a
/// resource or a reader in sight. Mark its partition columns and hand it to
/// an Iceberg catalog, and the table exists before the first log line does;
/// the reader then emits exactly this shape.
///
/// # Errors
///
/// Returns an error when the pattern does not parse or a capture group name
/// collides with a base column.
pub fn schema_from_pattern(pattern: &str) -> Result<Field> {
    Ok(LineRecordOptions::new(pattern)?.into_schema())
}

/// Project a borrowed handle's line records, per the trait method's contract.
pub(crate) fn read_arrow_lines<H: IOBase>(
    handle: &H,
    options: &LineRecordOptions,
) -> Result<BatchReader> {
    match handle.kind() {
        IOKind::Directory => folder_lines(handle, options),
        IOKind::Unknown => empty_lines(options),
        _ => {
            // The url column reports this handle's canonical location even
            // when the owned view is reached another way.
            let url = url_text(handle);
            leaf_lines(reopened(handle)?, url, options)
        }
    }
}

/// Project a snapshot of the bytes a *view* handle presents.
///
/// [`reopened`] assumes the handle's bytes are its location's bytes, which is
/// true of every storage handle and false of a decoding view such as
/// [`Coded`](super::Coded): its reads present decoded bytes while its
/// location holds the encoded form. A view's `read_arrow_lines` overrides to
/// this instead - a copy of the presented value, under the handle's own url
/// and media type. A coded view materializes that value to serve any read,
/// so the copy is its ordinary cost, not a new one.
pub(crate) fn snapshot_arrow_lines<H: IOBase>(
    handle: &H,
    options: &LineRecordOptions,
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

/// Project an owned handle's line records, per the trait method's contract.
pub(crate) fn into_arrow_lines<H: IOBase + 'static>(
    handle: H,
    options: &LineRecordOptions,
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
///
/// The reader a projection returns is `'static`, so it cannot borrow the
/// caller's handle; the location is what gets reopened, through the same
/// `parent`/`child_by` walk every backend implements. A handle with no
/// parent, such as an in-memory buffer, has its bytes in memory already, so
/// the owned view is a snapshot of the still-encoded value: one copy, no
/// decode.
fn reopened<H: IOBase>(handle: &H) -> Result<Holder> {
    if let Some(parent) = handle.parent() {
        if let Some(name) = handle.url().and_then(crate::Url::file_name) {
            let mut child = parent.child_by(name)?;
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

/// The canonical `Url` display a resource's rows carry, empty when unlocated.
fn url_text(handle: &(impl IOBase + ?Sized)) -> SmolStr {
    handle.url().map(ToSmolStr::to_smolstr).unwrap_or_default()
}

/// Zero batches, schema still answered - what absence reads as.
fn empty_lines(options: &LineRecordOptions) -> Result<BatchReader> {
    let schema = schema_from_field(options.schema())?;
    Ok(crate::arrow::batch_reader(schema, []))
}

/// Stream a container's leaves, name-sorted, one open leaf at a time.
fn folder_lines(
    handle: &(impl IOBase + ?Sized),
    options: &LineRecordOptions,
) -> Result<BatchReader> {
    // The walk enumerates handles only - constructions that touch nothing.
    // Each leaf's bytes wait until the reader reaches it.
    let leaves: Vec<Holder> = handle.children_where(&[], false)?.collect();
    ArrowLines::boxed(options, leaves, None)
}

/// Stream one leaf the reader already owns.
fn leaf_lines<H: IOBase + 'static>(
    handle: H,
    url: SmolStr,
    options: &LineRecordOptions,
) -> Result<BatchReader> {
    let current = opened_records(handle, url, options)?;
    ArrowLines::boxed(options, Vec::new(), Some(current))
}

/// Open one resource's line records with their per-leaf row state.
fn opened_records<H: IOBase + 'static>(
    handle: H,
    url: SmolStr,
    options: &LineRecordOptions,
) -> Result<LeafRecords> {
    Ok(LeafRecords {
        records: LineRecords {
            lines: handle.into_read_lines()?,
            pattern: options.pattern.clone(),
            pending: None,
            pending_offset: 0,
            done: false,
        },
        url,
        rownum: 0,
    })
}

/// One resource's records mid-stream: its url text and 1-based row counter.
struct LeafRecords {
    records: LineRecords<Box<dyn Read + Send + 'static>>,
    url: SmolStr,
    rownum: i64,
}

/// The streaming projection: line records in, Arrow batches out.
///
/// At most one leaf is open and one batch under construction at any time. A
/// batch never spans two leaves, so every batch already emitted is complete
/// before the next leaf is even opened - which is what makes the laziness
/// observable: a later leaf that fails to decode surfaces its error only
/// after every earlier leaf's batches have arrived.
struct ArrowLines {
    pattern: regex_lite::Regex,
    captures: Vec<SmolStr>,
    timestamp_capture: Option<SmolStr>,
    /// One-row constants, repeated to each batch's height. Held one row at a
    /// time deliberately: the repetition is a `take`, not a stored column.
    customs: Vec<ArrayRef>,
    batch_size: usize,
    schema: SchemaRef,
    /// The emitted root; a batch is cast onto it when any capture is typed.
    root: Field,
    /// The all-text shape the builders produce: the same root with every
    /// capture column as `utf8`.
    raw_schema: SchemaRef,
    /// Whether any capture column is typed, so an untyped read never pays
    /// for a cast that would hand every array back unchanged.
    typed: bool,
    /// Leaves not yet opened, in name-sorted order.
    pending: std::vec::IntoIter<Holder>,
    current: Option<LeafRecords>,
    done: bool,
}

impl ArrowLines {
    /// Assemble the reader over already-validated options.
    fn boxed(
        options: &LineRecordOptions,
        pending: Vec<Holder>,
        current: Option<LeafRecords>,
    ) -> Result<BatchReader> {
        let root = options.schema();
        let schema = schema_from_field(root)?;
        let capture_count = options.captures.len();
        let mut customs = Vec::with_capacity(options.custom_fields.len());
        for (index, (_, value)) in options.custom_fields.iter().enumerate() {
            let field = root
                .get_field(BASE_COLUMNS.len() + capture_count + index)
                .ok_or_else(|| Error::from(crate::arrow::Error::internal("io::lines::customs")))?;
            customs.push(scalar_array(field, value)?);
        }
        // The builders always produce text captures; a typed capture is cast
        // onto the declared root as each batch closes, through the one cast
        // definition every schema-directed read uses.
        let typed = (0..capture_count).any(|index| {
            root.get_field(BASE_COLUMNS.len() + index)
                .is_some_and(|field| field.data_type() != &DataType::Utf8)
        });
        let raw_schema = if typed {
            let raw_root = build_schema(
                &vec![DataType::Utf8; capture_count],
                &options.captures,
                &options.custom_fields,
            )?;
            schema_from_field(&raw_root)?
        } else {
            Arc::clone(&schema)
        };
        Ok(Box::new(Self {
            pattern: options.pattern.clone(),
            captures: options.captures.clone(),
            timestamp_capture: options.timestamp_capture.clone(),
            customs,
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
        for capture in &mut builders.captures {
            columns.push(Arc::new(capture.finish()));
        }
        for constant in &self.customs {
            let indices = UInt32Array::from(vec![0_u32; rows]);
            columns.push(arrow_select::take::take(constant.as_ref(), &indices, None)?);
        }
        let batch = RecordBatch::try_new(Arc::clone(&self.raw_schema), columns)?;
        if !self.typed {
            return Ok(batch);
        }
        // Typed captures land through the one cast definition, strictly: a
        // captured text the declared datatype cannot read is an error, never
        // a silent null.
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
        let mut builders = RowBuilders::new(self.captures.len());
        loop {
            let Some(current) = self.current.as_mut() else {
                let Some(leaf) = self.pending.next() else {
                    break;
                };
                let url = url_text(&leaf);
                match leaf.into_read_lines() {
                    Ok(lines) => {
                        self.current = Some(LeafRecords {
                            records: LineRecords {
                                lines,
                                pattern: self.pattern.clone(),
                                pending: None,
                                pending_offset: 0,
                                done: false,
                            },
                            url,
                            rownum: 0,
                        });
                    }
                    Err(error) => {
                        self.done = true;
                        return Some(Err(external(error)));
                    }
                }
                continue;
            };
            match current.records.next_with_offset() {
                Some(Ok((offset, record))) => {
                    current.rownum += 1;
                    let appended = append_row(
                        &mut builders,
                        &record,
                        offset,
                        &current.url,
                        current.rownum,
                        &self.pattern,
                        &self.captures,
                        self.timestamp_capture.as_deref(),
                    );
                    if let Err(error) = appended {
                        self.done = true;
                        return Some(Err(external(error)));
                    }
                    if builders.rows == self.batch_size {
                        return Some(self.finish(&mut builders));
                    }
                }
                Some(Err(error)) => {
                    self.done = true;
                    return Some(Err(external(error)));
                }
                None => {
                    // The drained leaf closes its batch before the next leaf
                    // is opened, so a batch never spans two resources.
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
    captures: Vec<StringBuilder>,
    rows: usize,
}

impl RowBuilders {
    fn new(capture_count: usize) -> Self {
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
            captures: (0..capture_count).map(|_| StringBuilder::new()).collect(),
            rows: 0,
        }
    }
}

/// Parse one record into one row of every column builder.
#[allow(clippy::too_many_arguments)]
fn append_row(
    builders: &mut RowBuilders,
    record: &str,
    offset: u64,
    url: &str,
    rownum: i64,
    pattern: &regex_lite::Regex,
    captures: &[SmolStr],
    timestamp_capture: Option<&str>,
) -> Result<()> {
    builders.rows += 1;
    builders.url.append_value(url);
    builders.rownum.append_value(rownum);
    let offset = i64::try_from(offset).map_err(|_| {
        row_error(
            url,
            rownum,
            "offset",
            format_smolstr!(
                "expected a record offset within the signed 64-bit range, got {offset}"
            ),
        )
    })?;
    builders.offset.append_value(offset);
    let line_count = record.matches('\n').count() + 1;
    let line_count = i32::try_from(line_count).map_err(|_| {
        row_error(
            url,
            rownum,
            "lines",
            format_smolstr!(
                "expected a record within the 32-bit line budget, got {line_count} lines"
            ),
        )
    })?;
    builders.lines.append_value(line_count);

    // The pattern is a *line* pattern - the grouping opened this record
    // because its first line matched - so the header is matched within that
    // opening line alone. Capturing over the whole record would let a greedy
    // class cross into the continuation lines and call text the opening line
    // never contained a header.
    let opening = &record[..record.find('\n').unwrap_or(record.len())];
    let Some(matched) = pattern
        .captures(opening)
        .and_then(|caps| caps.get(0).map(|whole| (whole.start(), whole.end(), caps)))
    else {
        // The preamble record a rotated file starts with: nothing matched, so
        // there is no header to parse and the whole record is the message.
        builders.date.append_null();
        builders.time.append_null();
        builders.unix.append_null();
        builders.header.append_null();
        builders.message.append_value(record);
        builders
            .hash
            .append_value(crate::text::stable_hash_bytes(record.as_bytes()) as i64);
        for capture in &mut builders.captures {
            capture.append_null();
        }
        return Ok(());
    };
    let (start, end, caps) = matched;
    let header = &record[start..end];

    // The message is the record with the header match spliced out, trimmed.
    let message: Cow<'_, str> = if start == 0 {
        Cow::Borrowed(record[end..].trim())
    } else {
        let mut merged = String::with_capacity(record.len() - header.len());
        merged.push_str(&record[..start]);
        merged.push_str(&record[end..]);
        Cow::Owned(merged.trim().to_owned())
    };
    // The hash covers the message only - header stripped, trimmed - so equal
    // messages hash equal across files, runs, and rotations. `u64 as i64`
    // keeps the FNV-1a bit pattern; Iceberg has no unsigned integers.
    builders
        .hash
        .append_value(crate::text::stable_hash_bytes(message.as_bytes()) as i64);
    builders.message.append_value(message.as_ref());
    builders.header.append_value(header);

    let reading = match timestamp_capture {
        Some(name) => match caps.name(name) {
            Some(capture) => iso::parse_datetime(capture.as_str())
                .map_err(|error| timestamp_error(url, rownum, capture.as_str(), &error)),
            None => Err(row_error(
                url,
                rownum,
                "unix",
                format_smolstr!(
                    "expected the timestamp capture {name:?} to participate in the matched header {:?}",
                    crate::text::elide_to(header, crate::text::ERROR_TEXT_LIMIT)
                ),
            )),
        },
        None => iso::parse_datetime_prefix(header)
            .map(|(count, unit, _)| (count, unit))
            .map_err(|error| timestamp_error(url, rownum, header, &error)),
    };
    let (count, unit) = reading?;
    let per = iso::per_second(unit)
        .ok_or_else(|| Error::from(crate::arrow::Error::internal("io::lines::per_second")))?;
    let day = per * 86_400;
    let days = count.div_euclid(day);
    let in_day = count.rem_euclid(day);
    let date = i32::try_from(days)
        .map_err(|_| Error::from(crate::arrow::Error::internal("io::lines::civil_date")))?;
    // Truncation, never rounding: a sub-microsecond clock reading is only
    // fully recoverable from `unix`.
    let micros = if per > 1_000_000 {
        in_day / (per / 1_000_000)
    } else {
        in_day * (1_000_000 / per)
    };
    let nanos = count.checked_mul(1_000_000_000 / per).ok_or_else(|| {
        row_error(
            url,
            rownum,
            "unix",
            format_smolstr!(
                "expected a timestamp within the 64-bit nanosecond range (1677-09-21 to 2262-04-11), got {:?}",
                crate::text::elide_to(header, crate::text::ERROR_TEXT_LIMIT)
            ),
        )
    })?;
    builders.date.append_value(date);
    builders.time.append_value(micros);
    builders.unix.append_value(nanos);

    for (name, builder) in captures.iter().zip(&mut builders.captures) {
        match caps.name(name) {
            Some(capture) => builder.append_value(capture.as_str()),
            None => builder.append_null(),
        }
    }
    Ok(())
}

/// A typed per-row failure: the column path, the row, and the resource.
fn row_error(url: &str, rownum: i64, column: &str, reason: SmolStr) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$[{}].{column}", rownum - 1),
        reason: format_smolstr!("{reason} in row {rownum} of {url}"),
    }
}

/// A malformed timestamp inside a matched header, with its byte position.
fn timestamp_error(url: &str, rownum: i64, text: &str, error: &Error) -> Error {
    let detail = match error {
        Error::Parse {
            position, reason, ..
        } => format_smolstr!("{reason} at byte {position}"),
        other => other.to_smolstr(),
    };
    row_error(
        url,
        rownum,
        "header",
        format_smolstr!(
            "expected an ISO datetime opening {:?} ({detail})",
            crate::text::elide_to(text, crate::text::ERROR_TEXT_LIMIT)
        ),
    )
}
