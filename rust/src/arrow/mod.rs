//! Arrow array and IPC interoperability for [`crate::Scalar`].
//!
//! Conversion is schema-directed and never serializes values through JSON.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Field, StructType};
use arrow_array::{Array, ArrayRef, RecordBatch};
use arrow_schema::{ArrowError, Schema, SchemaRef};

pub(crate) mod rows;

/// Arrow Schema metadata carrying dictionary IDs across the C Data Interface.
///
/// The C schema represents dictionary ordering but has no slot for Arrow's
/// deprecated per-field dictionary ID.  This entry is therefore emitted only
/// by [`Field::into_arrow_exchange_schema`] and consumed by
/// [`Field::from_arrow_schema`];
/// it never becomes root [`Field`] metadata.
pub const IPC_DICTIONARY_IDS_KEY: &str = "YGGDRYL:ipc:dictionary-ids";

/// A failure at the Yggdryl/Arrow runtime boundary.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The language-neutral Yggdryl value or schema was invalid.
    Core(crate::Error),
    /// Arrow rejected an array, batch, or IPC message.
    Arrow(ArrowError),
    /// A valid Yggdryl datatype has no supported physical materialization.
    Unsupported {
        /// Stable Yggdryl datatype kind.
        kind: &'static str,
        /// Concise reason the representation is unsupported.
        reason: String,
    },
    /// A canonical Yggdryl value does not satisfy the physical Arrow field it is
    /// being materialized into.
    #[non_exhaustive]
    InvalidValue {
        /// Dot/bracket path from the record root, such as `$.users[3].zip`.
        path: SmolStr,
        /// What the schema required at `path`, in canonical vocabulary.
        expected: SmolStr,
        /// What the caller supplied, bounded by the shared error-text limit.
        actual: SmolStr,
    },
    /// A non-nullable target field the source does not satisfy.
    ///
    /// Only [`Nullability::Strict`](crate::Nullability::Strict) produces this:
    /// the default policy writes the field's canonical
    /// [default](crate::Field::default_value) instead.
    #[non_exhaustive]
    RequiredField {
        /// Dot/bracket path from the cast root, such as `$.users[].zip`.
        path: SmolStr,
        /// Exposed rows holding logical null, or `None` when no source column
        /// carries the field at all.
        nulls: Option<usize>,
    },
    /// Two physical schemas disagree.
    #[non_exhaustive]
    SchemaMismatch {
        /// Batch, record, or column ordinal when the failure is positional.
        index: Option<usize>,
        /// Dot/bracket path to the disagreeing node, or `$` for a whole schema.
        path: SmolStr,
        /// Rendered `show_diff` output: one line per differing node, never
        /// only the first.
        diff: String,
    },
    /// A root Field is not usable as a record, dataset, or cast target.
    #[non_exhaustive]
    InvalidRootField {
        /// The role the Field was supplied for, such as `tabular root`.
        role: &'static str,
        /// Resource URL when the root came from a tabular descriptor.
        url: Option<SmolStr>,
        /// The Field's name.
        name: SmolStr,
        /// What the role required, such as `a non-nullable struct datatype`.
        expected: SmolStr,
        /// What the Field actually is.
        actual: SmolStr,
    },
    /// A bounded materialization budget was exceeded.
    #[non_exhaustive]
    PhysicalLimit {
        /// What was counted, such as `expanded slots` or `fixed bytes`.
        kind: &'static str,
        /// The inclusive maximum.
        limit: usize,
        /// The count reached, or a truthful lower bound.
        actual: usize,
    },
    /// A bounded allocation could not be reserved.
    #[non_exhaustive]
    Allocation {
        /// What was being reserved, such as `union child offsets`.
        context: &'static str,
        /// Elements or bytes requested.
        requested: usize,
        /// The allocator's reason.
        source: std::collections::TryReserveError,
    },
    /// An internal invariant that caller input cannot reach was violated.
    ///
    /// Reported separately so [`Self::InvalidValue`] and
    /// [`Self::SchemaMismatch`] keep meaning "the caller's data is wrong".
    #[non_exhaustive]
    Internal {
        /// Stable branch identifier, such as `list_array::list_kind`.
        site: &'static str,
    },
    /// Two physical schemas are not record-compatible.
    ///
    /// This is the shrinking residual of failures that do not yet have a
    /// structured variant; prefer one of the typed variants above.
    IncompatibleSchema(String),
    /// A downstream tabular backend failed outside Arrow itself.
    External(Box<dyn std::error::Error + Send + Sync>),
}

impl Error {
    /// Preserves a downstream backend error and its source chain.
    pub fn external(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::External(Box::new(error))
    }

    /// Reports a bounded materialization budget that was exceeded.
    pub(crate) const fn physical_limit(kind: &'static str, actual: usize, limit: usize) -> Self {
        Self::PhysicalLimit {
            kind,
            limit,
            actual,
        }
    }

    /// Reports a bounded allocation that could not be reserved.
    pub(crate) const fn allocation(
        context: &'static str,
        requested: usize,
        source: std::collections::TryReserveError,
    ) -> Self {
        Self::Allocation {
            context,
            requested,
            source,
        }
    }

    /// Reports an invariant that caller input cannot reach.
    pub(crate) const fn internal(site: &'static str) -> Self {
        Self::Internal { site }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Core(error) => error.fmt(formatter),
            Self::Arrow(error) => error.fmt(formatter),
            Self::Unsupported { kind, reason } => {
                write!(
                    formatter,
                    "unsupported Arrow record datatype {kind}: {reason}"
                )
            }
            Self::InvalidValue {
                path,
                expected,
                actual,
            } => write!(
                formatter,
                "invalid Arrow record value at {path}: expected {expected}, got {actual}"
            ),
            Self::RequiredField { path, nulls } => match nulls {
                Some(nulls) => write!(
                    formatter,
                    "required Arrow field {path} holds {nulls} null values"
                ),
                None => write!(
                    formatter,
                    "required Arrow field {path} is missing from the source"
                ),
            },
            Self::SchemaMismatch { index, path, diff } => {
                formatter.write_str("incompatible Arrow record schema")?;
                if let Some(index) = index {
                    write!(formatter, " at index {index}")?;
                }
                write!(formatter, " ({path}):\n{diff}")
            }
            Self::InvalidRootField {
                role,
                url,
                name,
                expected,
                actual,
            } => {
                write!(formatter, "invalid {role} field {name:?}")?;
                if let Some(url) = url {
                    write!(formatter, " for {url}")?;
                }
                write!(formatter, ": expected {expected}, got {actual}")
            }
            Self::PhysicalLimit {
                kind,
                limit,
                actual,
            } => write!(
                formatter,
                "Arrow physical materialization exceeds the {kind} safety limit: expected at most {limit}, got {actual}"
            ),
            Self::Allocation {
                context,
                requested,
                source,
            } => write!(
                formatter,
                "unable to reserve {requested} Arrow {context} during bounded materialization: {source}"
            ),
            Self::Internal { site } => write!(
                formatter,
                "internal Arrow materialization invariant violated at {site}; this is a bug in yggdryl"
            ),
            Self::IncompatibleSchema(reason) => {
                write!(formatter, "incompatible Arrow record schema: {reason}")
            }
            Self::External(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Core(error) => Some(error),
            Self::Arrow(error) => Some(error),
            Self::Allocation { source, .. } => Some(source),
            Self::Unsupported { .. }
            | Self::InvalidValue { .. }
            | Self::RequiredField { .. }
            | Self::SchemaMismatch { .. }
            | Self::InvalidRootField { .. }
            | Self::PhysicalLimit { .. }
            | Self::Internal { .. }
            | Self::IncompatibleSchema(_) => None,
            Self::External(error) => Some(error.as_ref()),
        }
    }
}

impl From<crate::Error> for Error {
    fn from(value: crate::Error) -> Self {
        Self::Core(value)
    }
}

/// Widening an Arrow interop failure back into a core error.
///
/// The two error types wrap each other by design: an Arrow operation can fail
/// on a core value, and a core operation - such as [`crate::IOBase::open`]
/// on a media type - can fail on Arrow interop. A `Core` failure unwraps to
/// itself and an `Arrow` failure keeps its `ArrowError`; anything else is
/// carried whole as the source, so no detail is lost on the way through.
impl From<Error> for crate::Error {
    fn from(value: Error) -> Self {
        match value {
            Error::Core(error) => error,
            Error::Arrow(error) => Self::Arrow(error),
            other => Self::Arrow(ArrowError::ExternalError(Box::new(other))),
        }
    }
}

impl From<ArrowError> for Error {
    fn from(value: ArrowError) -> Self {
        Self::Arrow(value)
    }
}

/// Project a non-null Struct root Field into an Arrow schema.
///
/// This is the one place a root Field becomes an `arrow_schema::Schema`, so
/// field identifiers and root metadata reach every encoding the same way.
///
/// # Errors
///
/// Returns an error unless `field` is a bounded, non-nullable Struct root.
pub(crate) fn arrow_schema_from_field(field: &Field) -> Result<SchemaRef> {
    field.validate_bounded()?;
    if field.is_nullable() {
        return Err(Error::IncompatibleSchema(
            "tabular root Struct Field must be non-nullable".to_owned(),
        ));
    }
    if field.dtype().as_fields().is_none() {
        return Err(Error::IncompatibleSchema(format!(
            "tabular field {:?} must have a Struct datatype",
            field.name()
        )));
    }
    // The root's own projection, built once into its cache, already lists
    // every column: the schema shares that list rather than projecting each
    // child again.
    let arrow_schema::DataType::Struct(fields) = field.as_arrow_field_ref()?.data_type() else {
        return Err(Error::internal("arrow::arrow_schema_from_field"));
    };
    Ok(Arc::new(Schema::new_with_metadata(
        fields.clone(),
        field.as_metadata().clone().into_arrow_metadata(),
    )))
}

/// A streamed, schema-bearing sequence of Arrow batches.
///
/// Reading returns this rather than a `Vec`, so a caller decides whether to
/// hold every batch at once. It owns whatever it reads from, so it outlives the
/// call that produced it.
pub type BatchReader = Box<dyn arrow_array::RecordBatchReader + Send>;

/// Build a [`BatchReader`] over batches a caller already has.
///
/// Every record write takes a reader, so this is how an owned `Vec`, an array,
/// or a lazily-computed iterator becomes one. The reader owns what it yields,
/// which is what lets it outlive the call that built it; `schema` is what the
/// reader reports before the first batch is pulled.
///
/// ```
/// use std::sync::Arc;
///
/// use arrow_array::{Int64Array, RecordBatch, RecordBatchReader};
/// use yggdryl::DataType;
/// use yggdryl::StructType;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
///     .required_field("row");
/// let arrow_schema = schema.into_arrow_schema()?;
/// let batch = RecordBatch::try_new(
///     Arc::clone(&arrow_schema),
///     vec![Arc::new(Int64Array::from(vec![1, 2]))],
/// )?;
///
/// let reader = yggdryl::arrow::batch_reader(arrow_schema, [batch]);
/// assert_eq!(reader.schema().fields().len(), 1);
/// assert_eq!(reader.count(), 1);
/// # Ok(())
/// # }
/// ```
pub fn batch_reader<I>(schema: SchemaRef, batches: I) -> BatchReader
where
    I: IntoIterator<Item = arrow_array::RecordBatch>,
    I::IntoIter: Send + 'static,
{
    Box::new(arrow_array::RecordBatchIterator::new(
        batches.into_iter().map(Ok),
        schema,
    ))
}

/// One reader's batches, then another's - the one chaining implementation.
///
/// This is what an append is, and what a combine is: two streams end to end,
/// each batch encoded as it arrives so neither side is collected. Whether
/// either side is cast is decided *before* it gets here, by wrapping it in
/// [`SerieReader`](crate::SerieReader), so there is exactly one concatenation and one cast route.
struct Chained {
    first: BatchReader,
    second: BatchReader,
    schema: SchemaRef,
}

impl Iterator for Chained {
    type Item = std::result::Result<arrow_array::RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(batch) = self.first.next() {
            return Some(batch);
        }
        self.second.next()
    }
}

impl arrow_array::RecordBatchReader for Chained {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

/// Chain `incoming`, cast to `field`, after everything `stored` yields.
///
/// What is already stored is yielded as it stands, because the stored batches
/// are already what they are; only the incoming side is cast.
///
/// # Errors
///
/// Returns an error unless `field` is a bounded, non-nullable Struct root.
pub(crate) fn appended(
    stored: BatchReader,
    incoming: BatchReader,
    field: &Field,
    safe: bool,
) -> Result<BatchReader> {
    Ok(Box::new(Chained {
        first: stored,
        second: crate::SerieReader::from_arrow_reader(
            Some(field),
            incoming,
            crate::ArrowCastOptions::declared(safe),
        )?
        .into_arrow_reader(),
        schema: arrow_schema_from_field(field)?,
    }))
}

/// Chain two readers onto one declared root Field, casting both sides.
///
/// The old private `appended` promoted to public and made symmetric: this is
/// what a caller reaches for when they already know the shape both sides must
/// land in. `field` is a declaration, so both sides cast by the one
/// declared-column rule: a nullable column takes a value it cannot convert
/// as null when `safe`, and a not-null column refuses that value, a null and
/// a missing column by name.
/// Neither side is drained to inspect it and nothing is collected - a batch is
/// cast when it is pulled, and [`SerieReader`](crate::SerieReader) short-circuits a side that is
/// already the declared shape rather than rebuilding arrays it would hand back
/// unchanged.
///
/// ```
/// use std::sync::Arc;
///
/// use arrow_array::{Int64Array, RecordBatch};
/// use yggdryl::DataType;
/// use yggdryl::StructType;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let root = DataType::from(StructType::from_fields([DataType::Int64.nullable_field("id")])?)
///     .required_field("row");
/// let schema = root.clone().into_arrow_schema()?;
/// let batch = RecordBatch::try_new(
///     Arc::clone(&schema),
///     vec![Arc::new(Int64Array::from(vec![1_i64]))],
/// )?;
///
/// let left = yggdryl::arrow::batch_reader(Arc::clone(&schema), [batch.clone()]);
/// let right = yggdryl::arrow::batch_reader(schema, [batch]);
/// let joined = yggdryl::arrow::combined_as(left, right, &root, false)?;
///
/// assert_eq!(joined.map(|batch| batch.unwrap().num_rows()).sum::<usize>(), 2);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error unless `field` is a bounded, non-nullable Struct root.
pub fn combined_as(
    left: BatchReader,
    right: BatchReader,
    field: &Field,
    safe: bool,
) -> Result<BatchReader> {
    Ok(Box::new(Chained {
        first: crate::SerieReader::from_arrow_reader(
            Some(field),
            left,
            crate::ArrowCastOptions::declared(safe),
        )?
        .into_arrow_reader(),
        second: crate::SerieReader::from_arrow_reader(
            Some(field),
            right,
            crate::ArrowCastOptions::declared(safe),
        )?
        .into_arrow_reader(),
        schema: arrow_schema_from_field(field)?,
    }))
}

/// Chain two readers onto the root their two schemas merge into.
///
/// The case [`combined_as`] cannot serve: two readers whose schemas differ and
/// no target known in advance. The merged root is derived from both schemas
/// alone - [`BatchReader::schema`](arrow_array::RecordBatchReader::schema)
/// answers without pulling a batch - so the combine is **fully lazy**: nothing
/// is collected, and neither side is drained to inspect it.
///
/// # The merge rules
///
/// These are the contract, not an accident of the implementation:
///
/// - **Columns unite by name**, resolved ASCII case-insensitively, the way
///   column names already resolve everywhere a cast or a selection matches
///   them. Left's columns keep left's order; columns only in right are appended
///   after, in right's order.
/// - **A column in both must reconcile to one datatype.** Differing datatypes
///   are **refused**, naming both sides. Refusing is the honest default: a
///   silent widening is how a decimal quietly becomes a float.
/// - **A column present in only one side becomes nullable**, necessarily - the
///   other side's rows have no value for it and the cast fills null. This holds
///   even when that column is non-nullable on its own side, so a caller
///   expecting their non-null declaration to survive a merge reads it here
///   rather than discovering it.
/// - **Metadata and field ids: left's are kept**, and a conflicting
///   `PARQUET:field_id` is refused rather than silently reassigned - Iceberg
///   cares about field identity, and a reassigned id corrupts a table's schema
///   evolution.
/// - **The root name is left's**, and the merged root is a bounded,
///   non-nullable Struct, as [`SerieReader`](crate::SerieReader) requires. Because the merge never
///   widens a datatype - it refuses instead - every column stays exactly what
///   one of the two sides declared, so a merged reader is appendable to an
///   Iceberg table wherever both inputs were.
///
/// ```
/// use std::sync::Arc;
///
/// use arrow_array::{Int64Array, RecordBatch, StringArray};
/// use yggdryl::DataType;
/// use yggdryl::StructType;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let left_root = DataType::from(StructType::from_fields([DataType::Int64.nullable_field("id")])?)
///     .required_field("row");
/// let right_root = DataType::from(StructType::from_fields([
///     DataType::Int64.nullable_field("id"),
///     DataType::utf8().nullable_field("venue"),
/// ])?)
/// .required_field("row");
///
/// let left_schema = left_root.into_arrow_schema()?;
/// let right_schema = right_root.into_arrow_schema()?;
/// let left = yggdryl::arrow::batch_reader(
///     Arc::clone(&left_schema),
///     [RecordBatch::try_new(left_schema, vec![Arc::new(Int64Array::from(vec![1_i64]))])?],
/// );
/// let right = yggdryl::arrow::batch_reader(
///     Arc::clone(&right_schema),
///     [RecordBatch::try_new(
///         right_schema,
///         vec![
///             Arc::new(Int64Array::from(vec![2_i64])),
///             Arc::new(StringArray::from(vec!["XPAR"])),
///         ],
///     )?],
/// );
///
/// let joined = yggdryl::arrow::combined(left, right)?;
/// // The merged root carries both columns; left's rows read null for `venue`.
/// assert_eq!(joined.schema().fields().len(), 2);
/// assert_eq!(joined.map(|batch| batch.unwrap().num_rows()).sum::<usize>(), 2);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error naming both sides when a shared column's datatype or
/// `PARQUET:field_id` disagrees, or when either schema is not a bounded,
/// non-nullable Struct root.
pub fn combined(left: BatchReader, right: BatchReader) -> Result<BatchReader> {
    // Both schemas are answered without pulling a batch, so the merge costs no
    // rows and the result stays lazy.
    let left_root = field_from_arrow_schema("row", left.schema().as_ref())?;
    let right_root = field_from_arrow_schema("row", right.schema().as_ref())?;
    let merged = merged_root(&left_root, &right_root)?;
    // Safe casting: a merge never widens, so a value that will not fit is a
    // disagreement worth raising rather than a null worth inventing.
    combined_as(left, right, &merged, true)
}

/// Merge two struct roots into the one both sides cast onto.
///
/// The rules are documented on [`combined`]; this is the one implementation of
/// them.
fn merged_root(left: &Field, right: &Field) -> Result<Field> {
    let mut columns: Vec<Field> = Vec::with_capacity(left.field_len() + right.field_len());
    for column in left.fields() {
        let Some(counterpart) = right
            .fields()
            .iter()
            .find(|held| held.name().eq_ignore_ascii_case(column.name()))
        else {
            // Only on the left, so the right's rows have no value for it.
            columns.push(column.clone().with_nullable(true));
            continue;
        };
        columns.push(reconciled(column, counterpart)?);
    }
    for column in right.fields() {
        if left
            .fields()
            .iter()
            .any(|held| held.name().eq_ignore_ascii_case(column.name()))
        {
            continue;
        }
        // Only on the right, so the left's rows have no value for it.
        columns.push(column.clone().with_nullable(true));
    }
    // The root name is left's, and the root is what a cast target must be.
    Ok(DataType::from(StructType::from_fields(columns)?).required_field(left.name()))
}

/// Reconcile one column present on both sides, or refuse naming both.
fn reconciled(left: &Field, right: &Field) -> Result<Field> {
    if left.dtype() != right.dtype() {
        return Err(Error::IncompatibleSchema(format!(
            "expected one datatype for the merged column {:?}, got {} on the left and {} on the \
             right",
            left.name(),
            left.dtype(),
            right.dtype()
        )));
    }
    let left_id = left.parquet_field_id()?;
    let right_id = right.parquet_field_id()?;
    if let (Some(left_id), Some(right_id)) = (left_id, right_id) {
        if left_id != right_id {
            return Err(Error::IncompatibleSchema(format!(
                "expected one PARQUET:field_id for the merged column {:?}, got {left_id} on the \
                 left and {right_id} on the right",
                left.name()
            )));
        }
    }
    // Left's metadata and identity are kept; nullability widens, because a
    // column required on one side and nullable on the other is nullable in a
    // stream that carries both.
    Ok(left
        .clone()
        .with_nullable(left.is_nullable() || right.is_nullable()))
}

/// The bytes a batch's rows occupy, as its own slices count them.
///
/// [`RecordBatch::get_array_memory_size`] counts every buffer whole, so a
/// zero-copy slice of a large batch reports its parent's allocation; a size
/// estimate spread over the slice's rows then comes out as many times too
/// large as there are slices. This counts each column's sliced extent, and
/// falls back to the whole buffers for a layout that cannot be sliced so.
pub(crate) fn sliced_memory_size(batch: &arrow_array::RecordBatch) -> usize {
    batch.columns().iter().map(sliced_array_size).sum()
}

/// The bytes one column's rows occupy, as its own slice counts them - the
/// per-column half of [`sliced_memory_size`].
///
/// A flat column counts its sliced buffers. The layouts whose values live
/// elsewhere count what the slice reaches: a view column its sixteen-byte
/// views and the out-of-line bytes they point at, a list or map only the
/// child range its offsets span, a struct its children, a dictionary its keys
/// and its values whole.
pub(crate) fn sliced_array_size(column: &arrow_array::ArrayRef) -> usize {
    sliced_size(column.as_ref())
}

fn sliced_size(array: &dyn arrow_array::Array) -> usize {
    use arrow_array::cast::AsArray;
    use arrow_schema::DataType as ArrowType;

    let nulls = array.nulls().map_or(0, |nulls| nulls.len().div_ceil(8));
    // A view's low 32 bits are its length; one of at most twelve bytes is
    // stored inline, and a longer one points at a data buffer.
    let viewed = |views: &[u128]| {
        views.len() * 16
            + views
                .iter()
                .map(|view| *view as u32 as usize)
                .filter(|length| *length > 12)
                .sum::<usize>()
    };
    match array.data_type() {
        ArrowType::Utf8View => nulls + viewed(array.as_string_view().views()),
        ArrowType::BinaryView => nulls + viewed(array.as_binary_view().views()),
        ArrowType::List(_) => {
            let list = array.as_list::<i32>();
            nulls + offsets_size(list.offsets(), list.values())
        }
        ArrowType::LargeList(_) => {
            let list = array.as_list::<i64>();
            nulls + offsets_size(list.offsets(), list.values())
        }
        ArrowType::Map(..) => {
            let map = array.as_map();
            let entries: arrow_array::ArrayRef = Arc::new(map.entries().clone());
            nulls + offsets_size(map.offsets(), &entries)
        }
        ArrowType::Struct(_) => {
            nulls
                + array
                    .as_struct()
                    .columns()
                    .iter()
                    .map(|child| sliced_size(child.as_ref()))
                    .sum::<usize>()
        }
        ArrowType::Dictionary(..) => {
            let dictionary = array.as_any_dictionary();
            sliced_size(dictionary.keys()) + dictionary.values().get_array_memory_size()
        }
        _ => {
            let whole = array.get_array_memory_size();
            array
                .to_data()
                .get_slice_memory_size()
                .map_or(whole, |sliced| sliced.min(whole))
        }
    }
}

/// A list's offsets and the child range they span.
fn offsets_size<O: arrow_array::OffsetSizeTrait>(
    offsets: &arrow_buffer::OffsetBuffer<O>,
    values: &arrow_array::ArrayRef,
) -> usize {
    let first = offsets.first().map_or(0, |offset| offset.as_usize());
    let last = offsets.last().map_or(0, |offset| offset.as_usize());
    std::mem::size_of_val(offsets.as_ref())
        + sliced_size(values.slice(first, last.saturating_sub(first)).as_ref())
}

/// Return whether two schemas name the same columns, in the same order.
///
/// This is the question a stream asks of a batch that is not the shape it
/// expected: same columns is a batch that reconciles - differing only in a
/// nullable flag, an extension entry, or a storage width - and different
/// columns is different data, which no reconciliation should invent its way
/// past. Names fold the way every other lookup in the crate folds them.
pub(crate) fn same_columns(left: &arrow_schema::Schema, right: &arrow_schema::Schema) -> bool {
    left.fields().len() == right.fields().len()
        && left
            .fields()
            .iter()
            .zip(right.fields())
            .all(|(left, right)| left.name().eq_ignore_ascii_case(right.name()))
}

/// Recover the failure a batch reader carried, unwrapping a core one.
///
/// A streaming adapter can only fail as an `ArrowError`, so a core failure it
/// raises - a cast it could not plan, a value the target Field rejects - has to
/// travel boxed inside one. Unwrapping it here keeps the typed variant a caller
/// can inspect rather than flattening it into a message.
///
/// A binding that drains a reader itself needs this for the same reason: the
/// envelope is transport, and reporting it would hand a caller
/// `External error: <the real one>` instead of the failure the cast raised.
pub fn from_reader_error(error: ArrowError) -> Error {
    let ArrowError::ExternalError(external) = error else {
        return Error::Arrow(error);
    };
    // A runtime failure is boxed as it stands; a schema or codec failure from
    // the language-neutral core is boxed one layer further in.
    let external = match external.downcast::<Error>() {
        Ok(runtime) => return *runtime,
        Err(external) => external,
    };
    match external.downcast::<crate::Error>() {
        Ok(core) => Error::Core(*core),
        Err(other) => Error::Arrow(ArrowError::ExternalError(other)),
    }
}

/// Return the stored column positions `field` names, when it names a subset.
///
/// This is the input a column pushdown needs: an encoding can skip a column it
/// is never asked for, but only when every name asked for is one it actually
/// stores. `None` therefore means "read everything" - either the caller
/// declared no schema, or the schema names something the resource does not
/// hold, which a projection cannot conjure and a later cast has to supply.
/// Positions come back ascending, because both encodings' masks select columns
/// without reordering them.
pub(crate) fn projection_indices(
    field: Option<&Field>,
    columns: Option<&[String]>,
    stored: &Schema,
) -> Option<Vec<usize>> {
    let names: Vec<&str> = match (field, columns) {
        // A declared root says what the rows are meant to be, and its columns
        // are what the encoding decodes; the expressions run over those.
        (Some(field), _) => {
            if field.is_nullable() || !field.is_struct() {
                return None;
            }
            field.fields().iter().map(Field::name).collect()
        }
        // Without one, the expressions themselves say which stored columns
        // they read, and nothing else is decoded.
        (None, Some(columns)) => columns.iter().map(String::as_str).collect(),
        (None, None) => return None,
    };
    // Zero columns is not a projection, and asking for every column is the read
    // that already happens, so neither is worth a mask.
    if names.is_empty() || names.len() >= stored.fields().len() {
        return None;
    }
    let mut indices: Vec<usize> = Vec::with_capacity(names.len());
    for name in names {
        let position = stored
            .fields()
            .iter()
            .position(|held| held.name().eq_ignore_ascii_case(name))?;
        indices.push(position);
    }
    indices.sort_unstable();
    indices.dedup();
    Some(indices)
}

/// The result type returned by Arrow record interoperability.
pub type Result<T> = std::result::Result<T, Error>;

/// The Arrow datatype `field` projects to, without building more than it
/// has to.
///
/// A leaf's projection is its datatype's Arrow storage, so it is answered
/// alone rather than by building - and caching - a whole Arrow field around
/// it, which a reader resolving its columns once would pay per reader. A
/// nested layout states child fields, and a field whose metadata restates an
/// Arrow extension is refused by the projection, so both borrow the
/// projection itself.
///
/// # Errors
///
/// Returns an error when the field has no Arrow projection.
pub(crate) fn projected_datatype(
    field: &Field,
) -> crate::Result<std::borrow::Cow<'_, arrow_schema::DataType>> {
    if !field.dtype().is_nested() && !field.as_metadata().may_hold_arrow_extension() {
        return Ok(std::borrow::Cow::Owned(field.dtype().to_arrow_datatype()?));
    }
    Ok(std::borrow::Cow::Borrowed(
        field.as_arrow_field_ref()?.data_type(),
    ))
}

/// Refuse an array whose physical layout is not the one `field` declares.
///
/// The one owner of "this array lays out as this field's projection": the
/// datatype [`projected_datatype`] answers is compared, so a nested field's
/// cached projection is never cloned or rebuilt, and both a held Arrow value
/// and a column take this door.
///
/// # Errors
///
/// Returns an error naming the field and both datatypes when they differ,
/// or when the field has no Arrow projection.
pub(crate) fn require_projection(field: &Field, array: &dyn Array) -> Result<()> {
    let expected = projected_datatype(field)?;
    if array.data_type() == expected.as_ref() {
        return Ok(());
    }
    Err(Error::IncompatibleSchema(format!(
        "Arrow datatype {:?} differs from the {:?} field's {expected:?}",
        array.data_type(),
        field.name()
    )))
}

/// Skip `offset` rows of a stream, then yield at most `limit` more.
///
/// Both bounds are exact and streamed: the batch a bound lands inside is cut
/// with [`RecordBatch::slice`], a view over the same buffers, and a satisfied
/// limit stops pulling.
pub fn sliced_reader(reader: BatchReader, offset: u64, limit: Option<u64>) -> BatchReader {
    if offset == 0 && limit.is_none() {
        return reader;
    }
    let schema = reader.schema();
    Box::new(Sliced {
        inner: reader,
        schema,
        to_skip: offset,
        remaining: limit,
    })
}

/// One reader's batches, past an offset and up to a limit.
struct Sliced {
    inner: BatchReader,
    schema: SchemaRef,
    to_skip: u64,
    remaining: Option<u64>,
}

impl Iterator for Sliced {
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == Some(0) {
            return None;
        }
        loop {
            let batch = match self.inner.next()? {
                Ok(batch) => batch,
                Err(error) => return Some(Err(error)),
            };
            let rows = batch.num_rows() as u64;
            if self.to_skip >= rows {
                self.to_skip -= rows;
                continue;
            }
            let start = usize::try_from(self.to_skip).unwrap_or(usize::MAX);
            self.to_skip = 0;
            let available = rows - start as u64;
            let taken = self
                .remaining
                .map_or(available, |remaining| remaining.min(available));
            if let Some(remaining) = &mut self.remaining {
                *remaining -= taken;
            }
            if start == 0 && taken == rows {
                return Some(Ok(batch));
            }
            return Some(Ok(
                batch.slice(start, usize::try_from(taken).unwrap_or(usize::MAX))
            ));
        }
    }
}

impl arrow_array::RecordBatchReader for Sliced {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

/// Rebuild one batch over a new column list, keeping what a rebuild loses.
///
/// `Schema::new` drops the schema-level metadata, and a batch left with no
/// columns forgets how many rows it had. Both are restored here rather than at
/// each call site, and the row count doubles as the check that every column
/// handed in is as long as the batch it is replacing a column of.
pub(crate) fn rebuilt_batch(
    batch: &RecordBatch,
    fields: Vec<Arc<arrow_schema::Field>>,
    columns: Vec<ArrayRef>,
) -> crate::Result<RecordBatch> {
    let schema = Arc::new(Schema::new(fields).with_metadata(batch.schema().metadata().clone()));
    let options = arrow_array::RecordBatchOptions::new().with_row_count(Some(batch.num_rows()));
    RecordBatch::try_new_with_options(schema, columns, &options).map_err(crate::Error::Arrow)
}

type DictionaryIds = BTreeMap<Vec<usize>, i64>;

fn dictionary_ids_error(reason: impl Into<SmolStr>) -> Error {
    Error::Core(crate::Error::InvalidMetadataValue {
        key: SmolStr::new_static(IPC_DICTIONARY_IDS_KEY),
        reason: reason.into(),
    })
}

fn dictionary_path_text(path: &[usize]) -> String {
    path.iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

/// Collect non-default dictionary IDs by field position.
///
/// A dictionary's value datatype is transparent to the path: it introduces no
/// Field of its own, while any Struct/Serie/Map/Union/RunEndEncoded fields below
/// that value do.  Thus every path component always means "the child Field at
/// this position", including the uncommon dictionary-of-struct shape.
fn collect_dictionary_ids_in_dtype(
    dtype: &DataType,
    path: &mut Vec<usize>,
    ids: &mut DictionaryIds,
) {
    if let DataType::Dictionary(dictionary) = dtype {
        collect_dictionary_ids_in_dtype(dictionary.value(), path, ids);
        return;
    }
    for index in 0..dtype.field_len() {
        let child = dtype
            .get_field(index)
            .expect("an index below a datatype's declared field count");
        path.push(index);
        if let Some(id) = child.dictionary_id().filter(|id| *id != 0) {
            ids.insert(path.clone(), id);
        }
        collect_dictionary_ids_in_dtype(child.dtype(), path, ids);
        path.pop();
    }
}

fn encode_dictionary_ids(ids: &DictionaryIds) -> String {
    let mut encoded = String::from("v1");
    for (path, id) in ids {
        encoded.push(';');
        encoded.push_str(&dictionary_path_text(path));
        encoded.push('=');
        encoded.push_str(&id.to_string());
    }
    encoded
}

fn parse_dictionary_ids(encoded: &str) -> Result<DictionaryIds> {
    let Some(entries) = encoded.strip_prefix("v1;") else {
        return Err(dictionary_ids_error(
            "expected canonical v1 dictionary-ID entries such as v1;0=42;1.0=-7",
        ));
    };
    if entries.is_empty() {
        return Err(dictionary_ids_error(
            "expected at least one non-zero dictionary-ID entry after v1;",
        ));
    }

    let mut ids = DictionaryIds::new();
    let mut previous: Option<Vec<usize>> = None;
    for entry in entries.split(';') {
        let Some((raw_path, raw_id)) = entry.split_once('=') else {
            return Err(dictionary_ids_error(format_smolstr!(
                "expected one path=id entry, got {entry:?}"
            )));
        };
        if raw_path.is_empty() || raw_id.is_empty() || raw_id.contains('=') {
            return Err(dictionary_ids_error(format_smolstr!(
                "expected one non-empty path=id entry, got {entry:?}"
            )));
        }

        let mut path = Vec::new();
        for raw_index in raw_path.split('.') {
            let index = raw_index.parse::<usize>().map_err(|_| {
                dictionary_ids_error(format_smolstr!(
                    "expected an unsigned positional path, got {raw_path:?}"
                ))
            })?;
            if raw_index != index.to_string() {
                return Err(dictionary_ids_error(format_smolstr!(
                    "expected a canonical unsigned positional path, got {raw_path:?}"
                )));
            }
            path.push(index);
        }

        let id = raw_id.parse::<i64>().map_err(|_| {
            dictionary_ids_error(format_smolstr!(
                "expected a signed 64-bit dictionary ID, got {raw_id:?} at path {raw_path}"
            ))
        })?;
        if id == 0 || raw_id != id.to_string() {
            return Err(dictionary_ids_error(format_smolstr!(
                "expected a canonical non-zero dictionary ID, got {raw_id:?} at path {raw_path}"
            )));
        }
        if previous.as_ref().is_some_and(|held| held >= &path) {
            return Err(dictionary_ids_error(format_smolstr!(
                "expected strictly increasing unique positional paths, got {raw_path:?} after {:?}",
                previous
                    .as_deref()
                    .map(dictionary_path_text)
                    .unwrap_or_default()
            )));
        }
        previous = Some(path.clone());
        ids.insert(path, id);
    }
    Ok(ids)
}

fn restore_dictionary_ids_in_field(
    mut field: Field,
    path: &mut Vec<usize>,
    ids: &mut DictionaryIds,
) -> Result<Field> {
    if let Some(id) = ids.remove(path.as_slice()) {
        let Some(actual) = field.dictionary_id() else {
            return Err(dictionary_ids_error(format_smolstr!(
                "positional path {} names a {} field, not a dictionary field",
                dictionary_path_text(path),
                field.dtype().name()
            )));
        };
        if actual != 0 && actual != id {
            return Err(dictionary_ids_error(format_smolstr!(
                "positional path {} carries dictionary ID {actual} in Arrow but {id} in the sidecar",
                dictionary_path_text(path)
            )));
        }
        let is_ordered = field
            .dictionary_is_ordered()
            .expect("a dictionary field has an ordering flag");
        field.set_dictionary_options(id, is_ordered)?;
    }

    let dtype = restore_dictionary_ids_in_dtype(field.dtype(), path, ids)?;
    field.set_dtype(dtype)?;
    Ok(field)
}

fn restore_dictionary_ids_in_dtype(
    dtype: &DataType,
    path: &mut Vec<usize>,
    ids: &mut DictionaryIds,
) -> Result<DataType> {
    if let DataType::Dictionary(dictionary) = dtype {
        let value = restore_dictionary_ids_in_dtype(dictionary.value(), path, ids)?;
        return DataType::dictionary(dictionary.key().clone(), value).map_err(Error::Core);
    }

    let mut children = Vec::with_capacity(dtype.field_len());
    for index in 0..dtype.field_len() {
        let child = dtype
            .get_field(index)
            .expect("an index below a datatype's declared field count")
            .clone();
        path.push(index);
        children.push(restore_dictionary_ids_in_field(child, path, ids)?);
        path.pop();
    }
    dtype.with_fields(children).map_err(Error::Core)
}

/// Imports one Arrow Schema as a non-null Struct root Field.
///
/// Every Arrow field becomes one child, and ordinary schema metadata becomes
/// root metadata.  [`IPC_DICTIONARY_IDS_KEY`] restores the nested dictionary
/// IDs that the Arrow C Data Interface cannot carry, then is removed rather
/// than becoming part of the logical root Field.
///
/// # Errors
///
/// Returns an error when the Arrow fields cannot form a non-null Struct root,
/// or when a dictionary-ID sidecar is malformed, conflicts with Arrow state,
/// or addresses anything other than an existing dictionary field.
pub(crate) fn field_from_arrow_schema(name: &str, schema: &Schema) -> Result<Field> {
    let mut metadata = schema.metadata().clone();
    let mut dictionary_ids = metadata
        .remove(IPC_DICTIONARY_IDS_KEY)
        .map(|encoded| parse_dictionary_ids(&encoded))
        .transpose()?
        .unwrap_or_default();
    let fields = schema
        .fields()
        .iter()
        .map(|field| Field::from_arrow_field_ref(field.clone()).map_err(Error::Core))
        .collect::<Result<Vec<_>>>()?;
    let dtype = DataType::from(StructType::from_fields(fields)?);
    let mut field = Field::from_parts(name, dtype, false, metadata)?;
    if !dictionary_ids.is_empty() {
        let dtype =
            restore_dictionary_ids_in_dtype(field.dtype(), &mut Vec::new(), &mut dictionary_ids)?;
        field.set_dtype(dtype)?;
        if let Some((path, _)) = dictionary_ids.first_key_value() {
            return Err(dictionary_ids_error(format_smolstr!(
                "positional path {} does not name an existing dictionary field",
                dictionary_path_text(path)
            )));
        }
    }
    field.validate_struct_root()?;
    Ok(field)
}

/// Projects a non-null Struct root Field as an Arrow schema.
///
/// Non-zero dictionary IDs are also recorded by positional path in the
/// transport-only [`IPC_DICTIONARY_IDS_KEY`] metadata entry.  Arrow's C Data
/// Interface preserves that metadata while omitting the deprecated ID slot,
/// so an outside runtime can return the schema without losing identity.
///
/// # Errors
///
/// Returns an error when the Field is not a non-null Struct root or a child
/// cannot be projected to Arrow, or when caller-owned root metadata uses the
/// reserved dictionary-ID key.
pub(crate) fn arrow_exchange_schema_from_field(schema: &Field) -> Result<Schema> {
    let projected = arrow_schema_from_field(schema)?;
    if schema.has_metadata(IPC_DICTIONARY_IDS_KEY) {
        return Err(dictionary_ids_error(
            "this key is transport-owned; remove the caller-set root metadata entry",
        ));
    }

    let mut ids = DictionaryIds::new();
    collect_dictionary_ids_in_dtype(schema.dtype(), &mut Vec::new(), &mut ids);
    if ids.is_empty() {
        return Ok(projected.as_ref().clone());
    }

    let mut metadata = projected.metadata().clone();
    metadata.insert(
        IPC_DICTIONARY_IDS_KEY.to_owned(),
        encode_dictionary_ids(&ids),
    );
    Ok(projected.as_ref().clone().with_metadata(metadata))
}
