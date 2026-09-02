//! Hive partition columns, projected in and out of Arrow data.
//!
//! A Hive layout stores a column *in the path*: every row under
//! `year=2024/month=01` shares those values, so the file leaves them out. That
//! makes a partitioned read incomplete and a partitioned write wasteful unless
//! something restores and removes them, which is what this module does.
//!
//! The projection follows the schema. A [`crate::Field`] that declares `year`
//! as an `Int32` gets an `Int32` column back, not a string, so a partitioned
//! read produces exactly the batch an unpartitioned one would. Without a
//! schema the values stay text, which is what the directory names hold.
//!
//! The module also owns the other half of the convention: a handle that
//! addresses the *folder* rather than one file reads across the leaves beneath
//! it and routes each row of a write to the leaf its partition values name.
//! [`crate::io::IOBase`]'s three write intents call in here whenever the handle
//! is a container, which is what lets a caller address a lake and one file with
//! the same call.

use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch, StringArray, UInt32Array};
use arrow_cast::display::{ArrayFormatter, FormatOptions};
use arrow_schema::{ArrowError, DataType as ArrowDataType, Field as ArrowField, Schema, SchemaRef};

use crate::arrow::{BatchReader, arrow_schema_from_field, field_from_arrow_schema};
use crate::field::cast::cast_field_array;
use crate::generic::{Holder, IORecordOptions, RecordOptions};
use crate::io::{IOBase, IOMedia, Listing};
use crate::{ArrowCast, DataType, Error, Field, Result, Url};

/// One partition's `column=value` pairs and the rows that belong to it.
type PartitionGroup = (Vec<(String, String)>, RecordBatch);

pub use super::NULL_PARTITION;

/// How every partition value in the project is rendered as directory text.
///
/// One set of options is the whole convention: the encoding's own display for
/// the value, and [`NULL_PARTITION`] for the absence of one. Both the
/// column-at-a-time renderer here and the one-value renderer
/// [`partition_text`] read it, so a table format writing a directory name and a
/// folder writing the same one cannot disagree about how a date is spelled.
fn partition_format() -> FormatOptions<'static> {
    FormatOptions::new().with_null(NULL_PARTITION)
}

/// Render one value the way a `column=value` directory spells it.
///
/// The value names its own datatype - a date counts days, a timestamp carries
/// its unit and zone - so the rendering needs nothing beside it. This is the
/// single-value form of what a partitioned write applies to a whole column, and
/// it goes through the same formatter and the same options, which is what makes
/// a table format's directory names identical to a folder's.
///
/// ```
/// use yggdryl::io::partition::partition_text;
/// use yggdryl::Scalar;
///
/// # fn main() -> yggdryl::Result<()> {
/// assert_eq!(partition_text(&Scalar::from("XNAS"))?, "XNAS");
/// assert_eq!(partition_text(&Scalar::date32(19_723))?, "2024-01-01");
/// assert_eq!(partition_text(&Scalar::d128(150, 2))?, "1.50");
/// assert_eq!(partition_text(&Scalar::Null)?, "null");
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error when the value names no single datatype, or when it cannot
/// be materialized as the one-element array the formatter reads.
pub fn partition_text(value: &crate::Scalar) -> Result<smol_str::SmolStr> {
    if value.is_null() {
        return Ok(smol_str::SmolStr::new_static(NULL_PARTITION));
    }
    // The value is non-null here, so the typed pairing's own projection is the
    // one-row array the formatter reads; a value the datatype cannot hold was
    // already refused when the pairing was built.
    let typed = crate::TypedScalar::from_value(value.clone())?;
    let array = typed.into_arrow_array()?;
    let formatter =
        ArrayFormatter::try_new(array.as_ref(), &partition_format()).map_err(Error::Arrow)?;
    Ok(smol_str::SmolStr::new(formatter.value(0).to_string()))
}

/// Build a constant column holding `value` for every row of a batch.
///
/// The directory name is text, and a declared column turns it into its own
/// type through the field cast: an ASCII width pads it, a date parses it.
/// `null` is both the spelling for absence and a perfectly good four-letter
/// value, which a path cannot settle by itself, so the declared nullability
/// decides: a nullable column reads what it cannot convert as absent, a
/// required one refuses it.
fn constant_column(value: &str, rows: usize, child: Option<&Field>) -> Result<ArrayRef> {
    let text: ArrayRef = Arc::new(StringArray::from(vec![value; rows]));
    match child {
        Some(child) => Ok(child.cast_arrow_array(text, child.is_nullable())?),
        None => Ok(text),
    }
}

/// Append the partition columns a path spells out to one batch.
///
/// A column the batch already carries is left alone: the file wins, because
/// rewriting stored values from a directory name would hide a mismatch.
///
/// ```
/// use std::sync::Arc;
///
/// use arrow_array::{ArrayRef, Int64Array, RecordBatch};
/// use yggdryl::io::partition::with_partitions;
/// use yggdryl::DataType;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let batch = RecordBatch::try_from_iter([(
///     "price",
///     Arc::new(Int64Array::from(vec![1, 2])) as ArrayRef,
/// )])?;
/// let schema = DataType::from_fields([DataType::Int32.required_field("year")])?
///     .required_field("row");
///
/// let restored = with_partitions(&batch, &[("year".into(), "2024".into())], Some(&schema))?;
///
/// assert_eq!(restored.num_columns(), 2);
/// assert_eq!(restored.schema().field(1).data_type(), &arrow_schema::DataType::Int32);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error when a partition value cannot be cast to the type the
/// schema declares for it, or when the widened batch is inconsistent.
pub fn with_partitions(
    batch: &RecordBatch,
    partitions: &[(String, String)],
    field: Option<&Field>,
) -> Result<RecordBatch> {
    if partitions.is_empty() {
        return Ok(batch.clone());
    }
    let mut fields: Vec<Arc<ArrowField>> = batch.schema().fields().iter().map(Arc::clone).collect();
    let mut columns = batch.columns().to_vec();
    let rows = batch.num_rows();

    for (column, value) in partitions {
        if batch.schema().index_of(column).is_ok() {
            continue;
        }
        let child = field.and_then(|field| field.get_field_by_path(column));
        let array = constant_column(value, rows, child)?;
        // The restored column keeps its declaration - nullability and any
        // extension identity - and says it came from the path. That is the one
        // fact the batch would otherwise lose, and it is what lets a read of a
        // lake be written back out with the same layout.
        let restored = match child {
            Some(child) => child.clone().into_arrow()?,
            // A path value is spelled out, so it is never null.
            None => ArrowField::new(column, ArrowDataType::Utf8, false),
        };
        let mut metadata = restored.metadata().clone();
        metadata.insert(
            crate::metadata::FIELD_PARTITION_KEY.to_owned(),
            "true".to_owned(),
        );
        fields.push(Arc::new(restored.with_metadata(metadata)));
        columns.push(array);
    }

    let schema = Arc::new(Schema::new(fields).with_metadata(batch.schema().metadata().clone()));
    RecordBatch::try_new(schema, columns).map_err(Error::Arrow)
}

/// Drop the columns a path already spells out from one batch.
///
/// This is the write side of the same rule: a value the directory name carries
/// does not need to be stored again in every row.
///
/// # Errors
///
/// Returns an error when the narrowed batch cannot be rebuilt.
pub fn without_partitions(
    batch: &RecordBatch,
    partitions: &[(String, String)],
) -> Result<RecordBatch> {
    if partitions.is_empty() {
        return Ok(batch.clone());
    }
    let keep: Vec<usize> = (0..batch.num_columns())
        .filter(|index| {
            let name = batch.schema().field(*index).name().clone();
            !partitions.iter().any(|(column, _)| column == &name)
        })
        .collect();
    if keep.len() == batch.num_columns() {
        return Ok(batch.clone());
    }

    let fields: Vec<Arc<ArrowField>> = keep
        .iter()
        .map(|index| Arc::clone(batch.schema().fields().get(*index).expect("a kept column")))
        .collect();
    let columns: Vec<ArrayRef> = keep
        .iter()
        .map(|index| Arc::clone(batch.column(*index)))
        .collect();
    let schema = Arc::new(Schema::new(fields).with_metadata(batch.schema().metadata().clone()));

    // A batch with no columns left still has to remember how many rows it had.
    if columns.is_empty() {
        let options = arrow_array::RecordBatchOptions::new().with_row_count(Some(batch.num_rows()));
        return RecordBatch::try_new_with_options(schema, columns, &options).map_err(Error::Arrow);
    }
    RecordBatch::try_new(schema, columns).map_err(Error::Arrow)
}

/// A batch reader that restores the partition columns of a location.
struct Partitioned {
    inner: crate::arrow::BatchReader,
    partitions: Vec<(String, String)>,
    schema: Arc<Schema>,
    field: Option<Field>,
}

impl Iterator for Partitioned {
    type Item = std::result::Result<RecordBatch, arrow_schema::ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        let batch = match self.inner.next()? {
            Ok(batch) => batch,
            Err(error) => return Some(Err(error)),
        };
        Some(
            with_partitions(&batch, &self.partitions, self.field.as_ref()).map_err(|error| {
                arrow_schema::ArrowError::ComputeError(format!(
                    "the partition columns could not be projected: {error}"
                ))
            }),
        )
    }
}

impl arrow_array::RecordBatchReader for Partitioned {
    fn schema(&self) -> Arc<Schema> {
        Arc::clone(&self.schema)
    }
}

/// Wrap a reader so every batch it yields carries the partition columns.
///
/// # Errors
///
/// Returns an error when the widened schema cannot be built.
pub fn partitioned_reader(
    inner: crate::arrow::BatchReader,
    partitions: Vec<(String, String)>,
    field: Option<Field>,
) -> Result<crate::arrow::BatchReader> {
    if partitions.is_empty() {
        return Ok(inner);
    }
    // The widened schema is the reader's own plus one field per partition, and
    // it is computed once so a consumer can read it before the first batch.
    let empty = RecordBatch::new_empty(inner.schema());
    let widened = with_partitions(&empty, &partitions, field.as_ref())?;
    Ok(Box::new(Partitioned {
        inner,
        partitions,
        schema: widened.schema(),
        field,
    }))
}

/// A batch reader that drops the partition columns of a location.
struct Narrowed {
    inner: crate::arrow::BatchReader,
    partitions: Vec<(String, String)>,
    schema: Arc<Schema>,
}

impl Iterator for Narrowed {
    type Item = std::result::Result<RecordBatch, arrow_schema::ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        let batch = match self.inner.next()? {
            Ok(batch) => batch,
            Err(error) => return Some(Err(error)),
        };
        Some(
            without_partitions(&batch, &self.partitions).map_err(|error| {
                arrow_schema::ArrowError::ComputeError(format!(
                    "the partition columns could not be removed: {error}"
                ))
            }),
        )
    }
}

impl arrow_array::RecordBatchReader for Narrowed {
    fn schema(&self) -> Arc<Schema> {
        Arc::clone(&self.schema)
    }
}

/// Wrap a reader so every batch it yields has the partition columns removed.
///
/// This is the write-side mirror of [`partitioned_reader`], and it stays a
/// reader for the same reason: a partitioned rewrite should not have to hold
/// the rows it is narrowing. `schema` is what the narrowed reader reports, which
/// is the stored root the caller already derived by subtracting the partition
/// columns.
pub fn narrowed_reader(
    inner: crate::arrow::BatchReader,
    partitions: Vec<(String, String)>,
    schema: Arc<Schema>,
) -> crate::arrow::BatchReader {
    if partitions.is_empty() {
        return inner;
    }
    Box::new(Narrowed {
        inner,
        partitions,
        schema,
    })
}

/// Wrap a reader so only rows matching the options' partition filters flow.
///
/// The pairs are sugar for a predicate: each one builds `column = 'value'` -
/// or `column is null`, because a path spells absence with four letters - and
/// the predicate is bound once against the reader's own schema and applied by
/// the one evaluator. A column the batch does not carry is left out of the
/// predicate entirely, because the leaf's path already answered for it.
pub(crate) fn filtered_reader(inner: BatchReader, options: &RecordOptions) -> Result<BatchReader> {
    if options.filter_partitions().is_empty() {
        return Ok(inner);
    }
    let schema = field_from_arrow_schema("row", inner.schema().as_ref())?;
    let predicate = options.partition_predicate(&schema);
    if predicate.is_always_true() {
        return Ok(inner);
    }
    Ok(predicate.bind(&schema)?.filter_reader(inner))
}

/// Return the Hive pairs `part` spells out below `root`.
///
/// Only what is below the addressed folder counts: a lake reached at
/// `/lake/year=2024` has `month` for a partition column and `year` for part of
/// its address.
fn pairs_under(part: &(impl IOBase + ?Sized), root: Option<&Url>) -> Vec<(String, String)> {
    let Some(url) = part.url() else {
        return Vec::new();
    };
    match root {
        Some(root) => url.hive_partitions_under(root),
        None => url.hive_partitions(),
    }
}

/// Return the leaves beneath `folder` that hold the encoding `options` names.
///
/// A lake usually holds more than its data files - a marker, a checksum, a
/// committed manifest - so a leaf whose media type is not this encoding is not
/// a part of the table and is skipped rather than handed to a decoder.
fn record_parts(folder: &(impl IOBase + ?Sized), options: &RecordOptions) -> Result<Listing> {
    let encoding = options.mime_type();
    Ok(folder
        .children_where(&[], false)?
        .keeping(move |child| child.media_type().base() == &encoding))
}

/// Return the partition columns the tree beneath `folder` already spells out.
///
/// The layout is the authority here, because it is the only thing that knows.
/// Nothing in a batch says which of its columns belong in a path, so a folder
/// holding `year=.../month=...` directories partitions by `year` and `month`,
/// and a folder holding no such directory is one table in one leaf. A tree is
/// therefore *created* by making the directories that name its columns - an
/// empty `year=2024/month=01` is enough, and it need not be the partition the
/// first rows land in.
///
/// The deepest chain wins, because a shallower one is a prefix of it: seeing
/// `year=2024` alone would otherwise stop at one column in a lake that is
/// partitioned by two.
fn folder_partition_columns(entries: &[Holder], root: Option<&Url>) -> Vec<String> {
    let mut deepest: Vec<String> = Vec::new();
    for entry in entries {
        let pairs = pairs_under(entry, root);
        if pairs.len() > deepest.len() {
            deepest = pairs.into_iter().map(|(column, _)| column).collect();
        }
    }
    deepest
}

/// Return the partition columns a write should lay the tree out by.
///
/// A tree that already spells a layout out is the authority on it, because its
/// leaves are already stored that way. A tree that does not - an empty folder,
/// or one holding a single flat leaf - takes the layout from the declared
/// schema, so a caller who marked `year` and `venue` as partition fields gets
/// `year=…/venue=…` directories without first creating one by hand.
///
/// A declared layout that contradicts the stored one is refused rather than
/// merged: writing the two into one tree would leave leaves whose directories
/// no longer say which columns they are missing.
///
/// # Errors
///
/// Returns an error naming both layouts when the schema and the tree disagree.
fn write_partition_columns(
    entries: &[Holder],
    root: Option<&Url>,
    options: &RecordOptions,
) -> Result<Vec<String>> {
    let stored = folder_partition_columns(entries, root);
    let declared: Vec<String> = options
        .field()
        .map(|field| {
            field
                .partition_field_names()
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if stored.is_empty() || declared.is_empty() || stored == declared {
        return Ok(if stored.is_empty() { declared } else { stored });
    }
    Err(Error::InvalidRecord {
        path: smol_str::SmolStr::new_static("$"),
        reason: crate::text::expected_got(
            format_args!(
                "the declared partition columns to match the ones this tree stores, [{}]",
                stored.join(", ")
            ),
            format_args!("[{}]", declared.join(", ")),
        ),
    })
}

/// Return the value each row of `batch` spells in a partition directory.
///
/// The rendering is the encoding's own text: an `Int32` `2024` is `2024`, and a
/// null is [`NULL_PARTITION`], because a path has no other way to say it.
fn partition_values(batch: &RecordBatch, columns: &[String]) -> Result<Vec<Vec<String>>> {
    let format = partition_format();
    let mut rendered: Vec<Vec<String>> = vec![Vec::with_capacity(columns.len()); batch.num_rows()];
    for column in columns {
        let Ok(index) = batch.schema().index_of(column) else {
            return Err(Error::InvalidRecord {
                path: smol_str::format_smolstr!("$.{column}"),
                reason: crate::text::expected_got(
                    format_args!("partition column {column:?} among the written columns"),
                    crate::text::elide_display(&batch.schema()),
                ),
            });
        };
        // An ASCII width is stored as fixed bytes, which the formatter would
        // spell as hex; the directory carries the trimmed text the value is.
        let schema = batch.schema();
        let column = if Field::from_arrow(schema.field(index))?
            .dtype()
            .ascii_width()
            .is_some()
        {
            cast_field_array(
                &DataType::Utf8.nullable_field(column.as_str()),
                Some(schema.field(index).metadata()),
                Arc::clone(batch.column(index)),
                false,
            )?
        } else {
            Arc::clone(batch.column(index))
        };
        let formatter = ArrayFormatter::try_new(column.as_ref(), &format).map_err(Error::Arrow)?;
        for (row, values) in rendered.iter_mut().enumerate() {
            values.push(formatter.value(row).to_string());
        }
    }
    Ok(rendered)
}

/// Split one batch into the partitions its rows belong to.
///
/// Groups keep first-appearance order so a write lands in a stable sequence,
/// and the partition columns are removed from each group: the directory name
/// carries them, which is the whole point of the layout.
fn split_by_partition(batch: &RecordBatch, columns: &[String]) -> Result<Vec<PartitionGroup>> {
    if columns.is_empty() {
        return Ok(vec![(Vec::new(), batch.clone())]);
    }
    let rendered = partition_values(batch, columns)?;
    let mut order: Vec<Vec<String>> = Vec::new();
    let mut groups: HashMap<Vec<String>, Vec<u32>> = HashMap::new();
    for (row, values) in rendered.into_iter().enumerate() {
        let row = u32::try_from(row).map_err(|_| Error::InvalidRecord {
            path: smol_str::SmolStr::new_static("$"),
            reason: smol_str::SmolStr::new_static(
                "expected a batch addressable by u32 row indices, got one with more rows than that",
            ),
        })?;
        match groups.get_mut(&values) {
            Some(rows) => rows.push(row),
            None => {
                order.push(values.clone());
                groups.insert(values, vec![row]);
            }
        }
    }

    let mut split = Vec::with_capacity(order.len());
    for values in order {
        let rows = groups
            .remove(&values)
            .expect("a group that was just ordered");
        let pairs: Vec<(String, String)> = columns.iter().cloned().zip(values).collect();
        let taken = arrow_select::take::take_record_batch(batch, &UInt32Array::from(rows))
            .map_err(Error::Arrow)?;
        let narrowed = without_partitions(&taken, &pairs)?;
        split.push((pairs, narrowed));
    }
    Ok(split)
}

/// Return the relative location of the leaf holding one partition.
///
/// The deterministic leaf for one partition.
///
/// A fixed name lets a write route a row without first collecting the folder's
/// existing leaves. Other leaves remain readable; append leaves them untouched,
/// while overwrite drains and clears them before publishing this one.
fn leaf_name(
    existing: &HashMap<Vec<(String, String)>, String>,
    pairs: &[(String, String)],
    options: &RecordOptions,
) -> String {
    if let Some(name) = existing.get(pairs) {
        return name.clone();
    }
    let extension = options.mime_type().extension().unwrap_or("bin");
    let mut relative = String::new();
    for (column, value) in pairs {
        relative.push_str(column);
        relative.push('=');
        relative.push_str(value);
        relative.push('/');
    }
    relative.push_str("part-0.");
    relative.push_str(extension);
    relative
}

/// Return the options one leaf of a partitioned tree is written under.
///
/// A partition column is not stored in the leaf, so it is neither part of the
/// leaf's schema nor part of its match key: every row under `year=2024` already
/// agrees on `year`, which makes it useless for telling two of them apart.
fn leaf_options(options: &RecordOptions, pairs: &[(String, String)]) -> Result<RecordOptions> {
    let mut leaf = options.clone();
    // The row and byte limits were already applied to the whole operation at
    // the record-method seam, so a leaf must not apply them again: a limit on
    // the tree re-applied per leaf would become one bound per partition, and
    // a byte bound would re-cut a sliced batch whose buffers still report
    // their full size.
    leaf.set_max_row_size(None);
    leaf.set_max_byte_size(None);
    leaf.set_commit_row_size(None);
    if pairs.is_empty() {
        return Ok(leaf);
    }
    let columns: Vec<&str> = pairs.iter().map(|(column, _)| column.as_str()).collect();
    if let Some(field) = options.field() {
        leaf.set_field(field.without_fields(&columns)?);
    }
    leaf.set_merge_by_names(
        options
            .merge_by_names()
            .iter()
            .filter(|name| {
                !columns
                    .iter()
                    .any(|column| column.eq_ignore_ascii_case(name))
            })
            .cloned()
            .collect(),
    );
    Ok(leaf)
}

/// Read every leaf beneath a folder as one reader.
///
/// Each leaf is read with the partition columns removed from its pushdown -
/// they are not stored there - restored from its own directory names, and cast
/// to the declared root, so a partitioned tree yields exactly the batches one
/// unpartitioned file would.
///
/// # Errors
///
/// Returns a listing, read, schema, or cast failure.
pub(crate) fn folder_reader(
    folder: &(impl IOBase + ?Sized),
    options: &RecordOptions,
) -> Result<BatchReader> {
    let mut parts = record_parts(folder, options)?;
    let root = folder.url().cloned();
    // A leaf whose path names a different value for a filtered column cannot
    // hold a matching row, so it is skipped before anything is decoded; a
    // leaf that does not name the column stays, and the row filter answers.
    let filter = options.partition_filter();
    if !filter.is_always_true() {
        // The same predicate a listing answers, asked of each leaf's own path.
        // A leaf that does not name a filtered column is unknown rather than
        // false, so it stays and the row filter answers for it.
        let bound = filter.bind(&crate::DataType::from_fields([])?.required_field("holder"))?;
        parts = Listing::new(parts.filter_map(move |part| match part {
            Err(error) => Some(Err(error)),
            Ok(part) => match bound.matches_holder(&crate::expression::Handle(&part)) {
                Ok(true) => Some(Ok(part)),
                Ok(false) => None,
                Err(error) => Some(Err(error)),
            },
        }));
    }
    let field = match options.field() {
        Some(field) => Some(field.clone()),
        // The line projection's shape follows from the options alone - no
        // leaf holds a schema to probe, and an empty folder still answers.
        None => match options {
            RecordOptions::Text(text) => Some(text.lines.field().clone()),
            _ => {
                let (field, remaining) = derived_field(parts, root.as_ref(), options)?;
                parts = remaining;
                field
            }
        },
    };
    let Some(field) = field else {
        // Nothing is stored and nothing was declared, so there is no shape to
        // report; an empty reader is what the laziness contract asks for.
        return Ok(crate::arrow::batch_reader(Arc::new(Schema::empty()), []));
    };
    let schema = arrow_schema_from_field(&field)?;
    Ok(Box::new(Chained {
        parts,
        root,
        field,
        options: options.clone(),
        current: None,
        schema,
        done: false,
    }))
}

/// Derive the root Field a folder holds from the first leaf that has one.
fn derived_field(
    mut parts: Listing,
    root: Option<&Url>,
    options: &RecordOptions,
) -> Result<(Option<Field>, Listing)> {
    while let Some(part) = parts.next() {
        let part = part?;
        let Some(stored) = super::stored_field(&part, options)? else {
            continue;
        };
        let pairs = pairs_under(&part, root);
        // The partition columns are appended untyped, which is what the
        // directory names actually hold; a caller wanting them typed declares a
        // schema and gets that cast for free. They arrive marked as partition
        // columns, because the layout is where that fact came from.
        let empty = RecordBatch::new_empty(arrow_schema_from_field(&stored)?);
        let widened = with_partitions(&empty, &pairs, None)?;
        let field = field_from_arrow_schema(options.root_name(), widened.schema().as_ref())?;
        // The schema-bearing part may also contain rows. Put it back in front
        // of the still-live listing instead of reopening or discarding it.
        let remaining = Listing::new(std::iter::once(Ok(part)).chain(parts));
        return Ok((Some(field), remaining));
    }
    Ok((None, parts))
}

/// A reader over every leaf of a partitioned folder, opened one at a time.
struct Chained {
    parts: Listing,
    root: Option<Url>,
    field: Field,
    options: RecordOptions,
    current: Option<BatchReader>,
    schema: SchemaRef,
    done: bool,
}

impl Iterator for Chained {
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        loop {
            if let Some(current) = self.current.as_mut() {
                match current.next() {
                    Some(Ok(batch)) => return Some(Ok(batch)),
                    Some(Err(error)) => {
                        self.done = true;
                        return Some(Err(error));
                    }
                    // A leaf is dropped as soon as it is drained, so a lake
                    // costs one open file rather than one per part.
                    None => self.current = None,
                }
            }
            let part = match self.parts.next()? {
                Ok(part) => part,
                Err(error) => {
                    self.done = true;
                    return Some(Err(ArrowError::ExternalError(Box::new(error))));
                }
            };
            match part_reader(&part, self.root.as_ref(), &self.field, &self.options) {
                Ok(reader) => self.current = Some(reader),
                Err(error) => {
                    self.done = true;
                    return Some(Err(ArrowError::ExternalError(Box::new(error))));
                }
            }
        }
    }
}

impl arrow_array::RecordBatchReader for Chained {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

/// Read one leaf of a partitioned folder as the declared root.
fn part_reader(
    part: &Holder,
    root: Option<&Url>,
    field: &Field,
    options: &RecordOptions,
) -> Result<BatchReader> {
    let pairs = pairs_under(part, root);
    let mut leaf = leaf_options(options, &pairs)?;
    if leaf.field().is_none() {
        let columns: Vec<&str> = pairs.iter().map(|(column, _)| column.as_str()).collect();
        leaf.set_field(field.without_fields(&columns)?);
    }
    let reader = super::leaf_reader(part, &leaf)?;
    let restored = partitioned_reader(reader, pairs, Some(field.clone()))?;
    Ok(crate::arrow::cast_reader(restored, field, options.safe())?)
}

/// One stable routing plan for every cadence of a folder write.
///
/// An incoming reader is consumed one batch at a time and each batch is split
/// by partition before it is written, so a write across a lake costs one batch
/// of memory rather than one per partition. The price is paid on the other
/// side: these encodings rewrite a whole leaf, so a partition touched by five
/// batches is rewritten five times. The first batch to reach a leaf performs the
/// caller's operation and the rest append to it, which is what keeps an
/// overwrite an overwrite without buffering the whole write first.
///
/// Publications are atomic per leaf, not across the folder: [`IOBase`] has no
/// transaction, rename, or compare-and-swap primitive spanning independent
/// child handles. A second handle can therefore observe a completed prefix of
/// a multi-partition write. Table formats provide their own snapshot commit
/// and are redirected before reaching this writer.
/// This per-leaf publication is also the explicit exception to an unset
/// cadence's single-publication rule: materializing an unbounded source merely
/// to approximate a folder transaction would violate the streaming contract,
/// while [`IOBase`] supplies no cross-leaf atomic primitive.
///
/// Layout discovery drains the tree once at top-level preflight. Reusing this
/// value keeps `commit_row_size = 1` from turning one listing into one listing
/// per row, and prevents rows in the same operation from observing different
/// layouts if another writer changes the folder between publications.
pub(crate) struct FolderWriter {
    existing: HashMap<Vec<(String, String)>, String>,
    parts: Vec<Holder>,
    columns: Vec<String>,
    options: RecordOptions,
}

impl FolderWriter {
    /// Resolve and validate a folder's layout without touching the input.
    pub(crate) fn new(folder: &(impl IOBase + ?Sized), options: &RecordOptions) -> Result<Self> {
        // One walk resolves both the path layout and existing leaf routing for
        // every cadence of this top-level operation. The handles are bounded
        // by the folder, not by the incoming stream; no row batch is held.
        let entries: Vec<Holder> =
            retried(|| folder.ls(true, false).collect::<Result<Vec<Holder>>>())?;
        let root = folder.url().cloned();
        let columns = write_partition_columns(&entries, root.as_ref(), options)?;
        Self::validate_merge_key(options, &columns)?;
        let encoding = options.mime_type();
        let mut parts: Vec<Holder> = entries
            .into_iter()
            .filter(|entry| !entry.is_container() && entry.media_type().base() == &encoding)
            .collect();
        parts.sort_by_key(|part| part.url().map(ToString::to_string));
        let mut existing = HashMap::new();
        for part in &parts {
            let Some(url) = part.url() else { continue };
            let Some(relative) = root.as_ref().and_then(|root| url.segments_under(root)) else {
                continue;
            };
            existing
                .entry(pairs_under(part, root.as_ref()))
                .or_insert_with(|| relative.join("/"));
        }
        Ok(Self {
            existing,
            parts,
            columns,
            options: options.clone(),
        })
    }

    /// Replace incoming-only options after the top-level shaping pass.
    pub(crate) fn set_options(&mut self, options: RecordOptions) -> Result<()> {
        Self::validate_merge_key(&options, &self.columns)?;
        self.options = options;
        Ok(())
    }

    /// Publish one overwrite cadence.
    pub(crate) fn overwrite(
        &mut self,
        folder: &(impl IOBase + ?Sized),
        batches: BatchReader,
    ) -> Result<()> {
        let schema = batches.schema();
        match super::non_empty_arrow_reader(batches)? {
            Some(batches) => self.write(folder, batches, false),
            None => self.overwrite_empty(folder, schema),
        }
    }

    /// Publish one append cadence.
    pub(crate) fn append(
        &mut self,
        folder: &(impl IOBase + ?Sized),
        batches: BatchReader,
    ) -> Result<()> {
        self.write(folder, batches, true)
    }

    /// Publish one merge cadence.
    pub(crate) fn merge(
        &mut self,
        folder: &(impl IOBase + ?Sized),
        batches: BatchReader,
    ) -> Result<()> {
        self.write(folder, batches, false)
    }

    /// A path-only key cannot identify two rows stored in the same leaf.
    fn validate_merge_key(options: &RecordOptions, columns: &[String]) -> Result<()> {
        let keys = options.merge_by_names();
        if keys.is_empty()
            || keys.iter().any(|key| {
                !columns
                    .iter()
                    .any(|column| column.eq_ignore_ascii_case(key))
            })
        {
            return Ok(());
        }
        Err(Error::InvalidRecord {
            path: smol_str::SmolStr::new_static("$.merge_by_names"),
            reason: crate::text::expected_got(
                "at least one merge key stored inside each partition leaf",
                format_args!(
                    "only partition columns [{}], which are constant within a leaf",
                    keys.join(", ")
                ),
            ),
        })
    }

    /// Clear the addressed rows while leaving one decodable schema carrier.
    ///
    /// Zero-byte leaves cannot answer an inferred field. A flat folder uses
    /// `part-0`; an existing partitioned folder reuses its first leaf so the
    /// path still supplies its constant partition columns. A brand-new
    /// partitioned layout has no values with which to name such a path and is
    /// refused before any existing leaf is cleared.
    fn overwrite_empty(
        &mut self,
        folder: &(impl IOBase + ?Sized),
        schema: SchemaRef,
    ) -> Result<()> {
        let selected = self
            .existing
            .iter()
            .min_by(|left, right| left.1.cmp(right.1))
            .map(|(pairs, relative)| (pairs.clone(), relative.clone()));
        let (pairs, relative) = match selected {
            Some(selected) => selected,
            None if self.columns.is_empty() => {
                let pairs = Vec::new();
                let relative = leaf_name(&self.existing, &pairs, &self.options);
                (pairs, relative)
            }
            None => {
                return Err(Error::InvalidRecord {
                    path: smol_str::SmolStr::new_static("$"),
                    reason: crate::text::expected_got(
                        "at least one row or stored partition path to publish an empty partitioned folder",
                        format_args!(
                            "an empty stream for partition columns [{}]",
                            self.columns.join(", ")
                        ),
                    ),
                });
            }
        };

        let root = field_from_arrow_schema(self.options.root_name(), schema.as_ref())?;
        let columns: Vec<&str> = self.columns.iter().map(String::as_str).collect();
        let leaf_field = root.without_fields(&columns)?;
        let leaf_schema = arrow_schema_from_field(&leaf_field)?;
        let mut leaf = leaf_options(&self.options, &pairs)?;
        // The empty reader already has the exact leaf shape; retaining the
        // logical declaration here would repeat the top-level cast.
        leaf.take_field();

        for part in &mut self.parts {
            part.clear()?;
        }
        let mut handle = folder.child_by_path(&relative)?;
        handle.overwrite_arrow_reader(crate::arrow::batch_reader(leaf_schema, []), &leaf)?;
        handle.flush()
    }

    fn write(
        &mut self,
        folder: &(impl IOBase + ?Sized),
        batches: BatchReader,
        append: bool,
    ) -> Result<()> {
        let merging = !self.options.merge_by_names().is_empty();
        if !append && !merging {
            // An overwrite replaces the tree, so a partition the incoming rows
            // never mention has to end up empty rather than keeping stale rows.
            for part in &mut self.parts {
                part.clear()?;
            }
        }

        let mut written: std::collections::HashSet<String> = std::collections::HashSet::new();
        for batch in batches {
            let batch = batch.map_err(crate::arrow::from_reader_error)?;
            if batch.num_rows() == 0 {
                continue;
            }
            for (pairs, part) in split_by_partition(&batch, &self.columns)? {
                let relative = leaf_name(&self.existing, &pairs, &self.options);
                let mut leaf = leaf_options(&self.options, &pairs)?;
                // The folder entry point cast the whole incoming stream before
                // it split path columns. Pop the declaration before encoding.
                leaf.take_field();
                let mut handle = folder.child_by_path(&relative)?;
                let first = written.insert(relative);
                let replacing = !leaf.merge_by_names().is_empty() || (!append && first);
                if replacing {
                    // A replace and a merge are idempotent, so a fresh reader
                    // can replay the same bounded partition on a transient race.
                    retried(|| {
                        let reader = crate::arrow::batch_reader(part.schema(), [part.clone()]);
                        if leaf.merge_by_names().is_empty() {
                            handle.overwrite_arrow_reader(reader, &leaf)?;
                        } else {
                            handle.merge_arrow_reader(reader, &leaf)?;
                        }
                        handle.flush()
                    })?;
                } else {
                    // Append is not idempotent: retrying could duplicate rows.
                    let reader = crate::arrow::batch_reader(part.schema(), [part]);
                    handle.append_arrow_reader(reader, &leaf)?;
                    handle.flush()?;
                }
            }
        }
        Ok(())
    }
}

/// Retry a folder step that can lose a race with a concurrent writer.
///
/// A shared folder has no commit protocol: a listing can catch a leaf
/// half-published, and a leaf write can collide with another writer growing
/// the same file. Three bounded attempts with a short growing pause smooth
/// exactly those races; a genuine failure still surfaces as itself on the
/// last attempt.
fn retried<T>(mut step: impl FnMut() -> Result<T>) -> Result<T> {
    let mut delay = std::time::Duration::from_millis(20);
    let mut last = None;
    for attempt in 0..3 {
        match step() {
            Ok(value) => return Ok(value),
            Err(error) => {
                last = Some(error);
                if attempt < 2 {
                    std::thread::sleep(delay);
                    delay *= 4;
                }
            }
        }
    }
    Err(last.expect("three attempts ran"))
}

#[cfg(test)]
mod tests;
