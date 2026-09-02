//! Arrow array and IPC interoperability for [`crate::Scalar`].
//!
//! Conversion is schema-directed and never serializes values through JSON.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Field, Scalar};
use arrow_array::{Array, ArrayRef, RecordBatch, Scalar as ArrowScalar, StructArray};
use arrow_schema::{ArrowError, Schema, SchemaRef};

pub(crate) mod rows;
pub(crate) mod value;

/// Arrow Schema metadata carrying dictionary IDs across the C Data Interface.
///
/// The C schema represents dictionary ordering but has no slot for Arrow's
/// deprecated per-field dictionary ID.  This entry is therefore emitted only
/// by [`Field::into_arrow_exchange_schema`] and consumed by
/// [`Field::from_arrow_schema`];
/// it never becomes root [`Field`] metadata.
pub const IPC_DICTIONARY_IDS_KEY: &str = "yggdryl:ipc:dictionary-ids";

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
/// on a core value, and a core operation - such as [`crate::io::IOBase::open`]
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
    let Some(fields) = field.dtype().as_fields() else {
        return Err(Error::IncompatibleSchema(format!(
            "tabular field {:?} must have a Struct datatype",
            field.name()
        )));
    };
    Ok(Arc::new(Schema::new_with_metadata(
        fields
            .iter()
            .cloned()
            .map(Field::into_arrow_ref)
            .collect::<crate::Result<Vec<_>>>()?,
        field.as_metadata().clone().into_arrow(),
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
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
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
/// [`cast_reader`], so there is exactly one concatenation and one cast route.
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
        second: cast_reader(incoming, field, safe)?,
        schema: arrow_schema_from_field(field)?,
    }))
}

/// Chain two readers onto one declared root Field, casting both sides.
///
/// The old private `appended` promoted to public and made symmetric: this is
/// what a caller reaches for when they already know the shape both sides must
/// land in.
/// Neither side is drained to inspect it and nothing is collected - a batch is
/// cast when it is pulled, and [`cast_reader`] short-circuits a side that is
/// already the declared shape rather than rebuilding arrays it would hand back
/// unchanged.
///
/// ```
/// use std::sync::Arc;
///
/// use arrow_array::{Int64Array, RecordBatch};
/// use yggdryl::DataType;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let root = DataType::from_fields([DataType::Int64.nullable_field("id")])?
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
        first: cast_reader(left, field, safe)?,
        second: cast_reader(right, field, safe)?,
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
///   non-nullable Struct, as [`cast_reader`] requires. Because the merge never
///   widens a datatype - it refuses instead - every column stays exactly what
///   one of the two sides declared, so a merged reader is appendable to an
///   Iceberg table wherever both inputs were.
///
/// ```
/// use std::sync::Arc;
///
/// use arrow_array::{Int64Array, RecordBatch, StringArray};
/// use yggdryl::DataType;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let left_root = DataType::from_fields([DataType::Int64.nullable_field("id")])?
///     .required_field("row");
/// let right_root = DataType::from_fields([
///     DataType::Int64.nullable_field("id"),
///     DataType::Utf8.nullable_field("venue"),
/// ])?
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
    Ok(DataType::from_fields(columns)?.required_field(left.name()))
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

/// One reader's batches, each cast to a declared root Field as it arrives.
struct Cast {
    inner: BatchReader,
    field: Field,
    safe: bool,
    schema: SchemaRef,
}

impl Iterator for Cast {
    type Item = std::result::Result<arrow_array::RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        use crate::field::cast::ArrowCast;

        let batch = match self.inner.next()? {
            Ok(batch) => batch,
            Err(error) => return Some(Err(error)),
        };
        Some(
            self.field
                .cast_arrow_batch(batch, self.safe)
                .map_err(|error| ArrowError::ExternalError(Box::new(error))),
        )
    }
}

impl arrow_array::RecordBatchReader for Cast {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

/// Return `reader`'s batches cast to `field`, one batch at a time.
///
/// This is the cast half of a schema-directed read: the encoding has already
/// skipped the columns the schema does not name, and this reorders, converts,
/// and fills what is left so every batch really is the declared shape. Nothing
/// is collected - a batch is cast when it is pulled.
///
/// # Errors
///
/// Returns an error unless `field` is a bounded, non-nullable Struct root.
pub fn cast_reader(inner: BatchReader, field: &Field, safe: bool) -> Result<BatchReader> {
    let schema = arrow_schema_from_field(field)?;
    if inner.schema() == schema {
        // An exact reader is already the declared shape, so casting each batch
        // would only rebuild arrays it would then hand back unchanged.
        return Ok(inner);
    }
    Ok(Box::new(Cast {
        inner,
        field: field.clone(),
        safe,
        schema,
    }))
}

/// Recover the failure a batch reader carried, unwrapping a core one.
///
/// A streaming adapter can only fail as an `ArrowError`, so a core failure it
/// raises - a cast it could not plan, a value the target Field rejects - has to
/// travel boxed inside one. Unwrapping it here keeps the typed variant a caller
/// can inspect rather than flattening it into a message.
pub(crate) fn from_reader_error(error: ArrowError) -> Error {
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
pub(crate) fn projection_indices(field: &Field, stored: &Schema) -> Option<Vec<usize>> {
    // Zero columns is not a projection, and asking for every column is the read
    // that already happens, so neither is worth a mask.
    if field.is_nullable()
        || !field.is_struct()
        || field.field_len() == 0
        || field.field_len() >= stored.fields().len()
    {
        return None;
    }
    let mut indices: Vec<usize> = Vec::with_capacity(field.field_len());
    for child in field.fields() {
        indices.push(stored.index_of(child.name()).ok()?);
    }
    indices.sort_unstable();
    indices.dedup();
    Some(indices)
}

/// The result type returned by Arrow record interoperability.
pub type Result<T> = std::result::Result<T, Error>;

/// Materialize one validated native value as an exact one-row Arrow array.
///
/// The array boundary for a single scalar: the value is validated through the
/// same schema-directed walk every row value takes - the exact Field is the
/// authority on nullability, dictionary options, and extension identity - and
/// then materialized under the shared physical budgets. A caller holding a
/// [`crate::TypedScalar`] with no Field around it uses
/// [`crate::TypedScalar::into_arrow_array`] instead.
///
/// # Errors
///
/// Returns an error when the value violates the Field or the physical Arrow
/// layout cannot represent it.
pub fn scalar_array(field: &Field, value: &Scalar) -> Result<ArrayRef> {
    let value = validate_scalar_value(field, value.clone())?;
    value::array_from_values(field, &[&value])
}

/// Materialize a sequence of native values as one Arrow array.
///
/// Each element is validated and canonicalized by `field`; materialization is
/// a single array build, not a concatenation of scalar arrays.
///
/// # Errors
///
/// Returns an error when `values` is not a sequence or an element violates
/// `field`.
pub fn array_from_value(field: &Field, values: &Scalar) -> Result<ArrayRef> {
    let values = values.as_sequence().ok_or_else(|| Error::InvalidValue {
        path: SmolStr::new_static("$"),
        expected: SmolStr::new_static("a sequence of array values"),
        actual: SmolStr::new(values.kind()),
    })?;
    let root = DataType::from_fields([field.clone()])?.required_field("row");
    let mut canonical = Vec::with_capacity(values.len());
    for value in values {
        let row = Scalar::from_sequence([value.clone()]);
        let row = root.canonicalize_value(row)?;
        canonical.push(
            row.as_sequence()
                .and_then(|row| row.first())
                .cloned()
                .ok_or_else(|| Error::internal("arrow::array_from_value"))?,
        );
    }
    let borrowed = canonical.iter().collect::<Vec<_>>();
    value::array_from_values(field, &borrowed)
}

/// Materialize a sequence of native struct rows as one Arrow record batch.
///
/// The outer value is a sequence and each child is an ordered row sequence or
/// a named [`Scalar::Record`]. The root Field validates and canonicalizes every
/// row before one columnar build.
///
/// # Errors
///
/// Returns an error when `root` is not a record root, `rows` is not a
/// sequence, or a row violates the schema.
pub fn batch_from_value(root: &Field, rows: &Scalar) -> Result<RecordBatch> {
    let rows = rows.as_sequence().ok_or_else(|| Error::InvalidValue {
        path: SmolStr::new_static("$"),
        expected: SmolStr::new_static("a sequence of record values"),
        actual: SmolStr::new(rows.kind()),
    })?;
    let schema = arrow_schema_from_field(root)?;
    let mut canonical = Vec::with_capacity(rows.len());
    for row in rows {
        canonical.push(root.canonicalize_value(row.clone())?);
    }
    self::rows::batch_from_values(root, schema, &canonical)
}

/// Validate one external one-row Arrow array and decode its canonical value.
///
/// # Errors
///
/// Returns an error unless `array` has length one, has the Field's exact
/// physical datatype, and decodes to a value satisfying all recursive Field
/// nullability and datatype constraints. A non-nullable Field may contain a
/// logical null only when the decoded value is exactly its datatype's
/// canonical intrinsic default. This narrow exception keeps datatype defaults
/// such as null-only dictionaries, unions, and run-end encodings closed under
/// [`scalar_array`] followed by this function without admitting arbitrary
/// selected-null values.
pub fn scalar_value(field: &Field, array: &dyn Array) -> Result<Scalar> {
    if array.len() != 1 {
        return Err(Error::IncompatibleSchema(format!(
            "Arrow scalar must contain exactly one value, got {}",
            array.len()
        )));
    }
    // Caller-built public DataType variants can be arbitrarily deep.
    // Bound the shape before Arrow's recursive datatype projection so a
    // malformed foreign scalar reports a normal schema error rather than
    // exhausting the native stack.
    field.dtype().validate_bounded()?;
    let expected = field.clone().into_arrow_ref()?.data_type().clone();
    if array.data_type() != &expected {
        return Err(Error::IncompatibleSchema(format!(
            "Arrow scalar datatype {:?} differs from expected {expected:?}",
            array.data_type()
        )));
    }
    let decoded = value::value_from_array(field.dtype(), array, 0)?;
    if let Err(error) = validate_scalar_value(field, decoded.clone()) {
        if !field.dtype().is_default_value(&decoded)? {
            return Err(error);
        }
    }
    Ok(decoded)
}

/// Materialize a bare datatype's canonical default as a one-row array.
///
/// The datatype planner is the authority, so [`DataType::Null`] and
/// transparent logical wrappers with a null-only canonical default may be
/// logically null even though the array projects through a synthetic
/// non-nullable Field. [`Field::default_value`] remains the sole nullability
/// authority for a caller-owned Field, reached through
/// [`default_scalar_array`].
pub(crate) fn default_dtype_scalar_array(dtype: &DataType) -> Result<ArrayRef> {
    let field = Field::new("value", dtype.clone(), false);
    let value = dtype.default_value()?;
    value::array_from_values(&field, &[&value])
}

/// Materialize a Field's canonical default as a one-row array.
pub(crate) fn default_scalar_array(field: &Field) -> Result<ArrayRef> {
    let value = field.default_value()?;
    // The core planner has already bounded and recursively validated this
    // exact Field/value pair. Keep public [`scalar_array`] defensive for
    // caller input without paying for a second Scalar validation here.
    value::array_from_values(field, &[&value])
}

pub(crate) fn validate_scalar_value(field: &Field, value: Scalar) -> Result<Scalar> {
    // Wrap the single value in a one-column row so it goes through exactly the
    // same schema-directed validator every other value does.
    let root = Field::new("scalar", DataType::from_fields([field.clone()])?, false);
    let row = root.canonicalize_value(Scalar::from_sequence([value]))?;
    row.as_sequence()
        .and_then(|values| values.first())
        .cloned()
        .ok_or_else(|| Error::internal("arrow::validate_scalar_value"))
}

/// One real Arrow struct scalar paired with its exact Yggdryl root field.
#[derive(Clone, Debug)]
pub struct StructScalar {
    field: Field,
    array: StructArray,
}

impl StructScalar {
    /// Validates one non-null Arrow struct row against a canonical schema.
    ///
    /// # Errors
    ///
    /// Returns an error unless the array is exactly one present row with a
    /// physical Struct layout compatible with `schema`.
    pub fn from_parts(schema: Field, array: StructArray) -> Result<Self> {
        if array.len() != 1 {
            return Err(Error::IncompatibleSchema(format!(
                "struct scalar must contain exactly one row, got {}",
                array.len()
            )));
        }
        if array.is_null(0) {
            return Err(Error::IncompatibleSchema(
                "a native Scalar cannot represent a null root struct".to_owned(),
            ));
        }
        ensure_struct_compatible(&schema, &array)?;
        Ok(Self {
            field: schema,
            array,
        })
    }

    /// Returns the exact root field.
    pub const fn field(&self) -> &Field {
        &self.field
    }

    /// Borrows the one-row Arrow struct array.
    pub const fn array(&self) -> &StructArray {
        &self.array
    }

    /// Returns a zero-copy one-element slice of the child at `index`.
    pub fn get(&self, index: usize) -> Option<ArrayRef> {
        self.array
            .columns()
            .get(index)
            .map(|array| array.slice(0, 1))
    }

    /// Returns a zero-copy one-element slice by exact field name.
    pub fn get_by_name(&self, name: &str) -> Option<ArrayRef> {
        self.field.index_of(name).and_then(|index| self.get(index))
    }

    /// Returns the exact Field and its zero-copy one-element Arrow slice.
    pub fn entry(&self, index: usize) -> Option<(&Field, ArrayRef)> {
        Some((self.field.get_field(index)?, self.get(index)?))
    }

    /// Consumes this value into Arrow's scalar marker.
    pub fn into_arrow_scalar(self) -> ArrowScalar<StructArray> {
        ArrowScalar::new(self.array)
    }
}

/// Read one Arrow array as a sequence of values, typed by `field`.
///
/// Every row becomes the [`Scalar`] its datatype spells - a null slot is
/// [`Scalar::Null`] - so the result serializes through any text format exactly
/// as the rest of the value model does.
///
/// # Errors
///
/// Returns an error when the array does not hold the field's datatype or a
/// value cannot be represented.
pub fn array_to_value(field: &Field, array: &dyn Array) -> Result<Scalar> {
    let mut rows = Vec::with_capacity(array.len());
    for index in 0..array.len() {
        rows.push(super::arrow::value::value_from_array(
            field.dtype(),
            array,
            index,
        )?);
    }
    Ok(Scalar::from_sequence(rows))
}

/// Read one record batch as a sequence of rows.
///
/// Each row becomes a [`Scalar::Sequence`] with one value per column, in schema
/// order. The batch schema remains the [`RecordBatch`]'s schema rather than
/// being duplicated inside every row.
///
/// # Errors
///
/// Returns an error when the batch's schema does not project to a record root
/// or a value cannot be represented.
pub fn batch_to_value(batch: &RecordBatch) -> Result<Scalar> {
    let root = field_from_arrow_schema("row", batch.schema().as_ref())?;
    let fields: Vec<Field> = root
        .dtype()
        .as_fields()
        .ok_or_else(|| Error::IncompatibleSchema("a batch projects to a struct root".to_owned()))?
        .to_vec();
    let mut rows = Vec::with_capacity(batch.num_rows());
    for index in 0..batch.num_rows() {
        let mut values = Vec::with_capacity(batch.num_columns());
        for (column, field) in batch.columns().iter().zip(fields.iter()) {
            values.push(super::arrow::value::value_from_array(
                field.dtype(),
                column.as_ref(),
                index,
            )?);
        }
        rows.push(Scalar::from_sequence(values));
    }
    Ok(Scalar::from_sequence(rows))
}

/// Build the root a `select_by_names` selection narrows `root` to.
///
/// `None` means no narrowing: an empty selection is the rows as they stand.
/// Names resolve ASCII case-insensitively, the way every cast matches them,
/// in the order they are asked for; a name the root does not have is an error
/// listing what is there, because a selection is a claim about the rows
/// rather than a wish.
pub(crate) fn selected_root(
    root: &Field,
    names: &[String],
    root_name: &str,
) -> Result<Option<Field>> {
    if names.is_empty() {
        return Ok(None);
    }
    let mut selected = Vec::with_capacity(names.len());
    for name in names {
        let child = root
            .fields()
            .iter()
            .find(|child| child.name().eq_ignore_ascii_case(name))
            .ok_or_else(|| crate::Error::InvalidRecord {
                path: smol_str::format_smolstr!("$.{name}"),
                reason: smol_str::format_smolstr!(
                    "expected a column named {name:?} to select, got columns {:?}",
                    root.fields().iter().map(Field::name).collect::<Vec<_>>()
                ),
            })?;
        selected.push(child.clone());
    }
    Ok(Some(
        crate::DataType::from_fields(selected)?.required_field(root_name),
    ))
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
/// Field of its own, while any Struct/List/Map/Union/RunEndEncoded fields below
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
        .map(|field| Field::from_arrow_ref(field.clone()).map_err(Error::Core))
        .collect::<Result<Vec<_>>>()?;
    let dtype = DataType::from_fields(fields)?;
    let mut field = Field::from_parts(name, dtype, false, metadata)?;
    if !dictionary_ids.is_empty() {
        let dtype = restore_dictionary_ids_in_dtype(
            field.dtype(),
            &mut Vec::new(),
            &mut dictionary_ids,
        )?;
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

/// Check that a struct array carries exactly the columns a field declares.
///
/// # Errors
///
/// Returns an error naming both schemas when they disagree.
fn ensure_struct_compatible(schema: &Field, array: &StructArray) -> Result<()> {
    let expected = arrow_schema_from_field(schema)?;
    let actual = array.fields();
    if expected.fields().as_ref() != actual.as_ref() {
        return Err(Error::IncompatibleSchema(format!(
            "expected a struct array matching {}, got {}",
            crate::text::elide_display(&expected),
            crate::text::elide_display(&arrow_schema::Schema::new(actual.clone()))
        )));
    }
    Ok(())
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
