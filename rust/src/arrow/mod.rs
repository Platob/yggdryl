//! Arrow array and IPC interoperability for [`crate::Value`].
//!
//! Conversion is schema-directed and never serializes values through JSON.

#![forbid(unsafe_code)]

use std::fmt;
use std::sync::Arc;

use smol_str::SmolStr;

use crate::{DataType, Field, Value};
use arrow_array::{Array, ArrayRef, Scalar, StructArray};
use arrow_schema::{ArrowError, Schema, SchemaRef};

pub(crate) mod value;

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
pub fn schema_from_field(field: &Field) -> Result<SchemaRef> {
    field.validate_bounded()?;
    if field.is_nullable() {
        return Err(Error::IncompatibleSchema(
            "tabular root Struct Field must be non-nullable".to_owned(),
        ));
    }
    let Some(fields) = field.data_type().as_fields() else {
        return Err(Error::IncompatibleSchema(format!(
            "tabular field {:?} must have a Struct datatype",
            field.name()
        )));
    };
    Ok(Arc::new(Schema::new_with_metadata(
        fields
            .iter()
            .map(Field::to_arrow_ref)
            .collect::<crate::Result<Vec<_>>>()?,
        field.as_metadata().to_arrow(),
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
/// let arrow_schema = schema.to_arrow_schema()?;
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

/// One reader's batches, then another's cast to a declared root Field.
///
/// This is what an append is: the resource's current contents followed by the
/// new ones, encoded as they arrive so neither side is collected.
struct Appended {
    stored: BatchReader,
    incoming: BatchReader,
    field: Field,
    safe: bool,
    schema: SchemaRef,
}

impl Iterator for Appended {
    type Item = std::result::Result<arrow_array::RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        use crate::field::cast::ArrowCast;

        // What is already stored is yielded as it stands; only the new batches
        // are cast, because the stored ones are already what they are.
        if let Some(batch) = self.stored.next() {
            return Some(batch);
        }
        let batch = match self.incoming.next()? {
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

impl arrow_array::RecordBatchReader for Appended {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

/// Chain `incoming`, cast to `field`, after everything `stored` yields.
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
    Ok(Box::new(Appended {
        stored,
        incoming,
        field: field.clone(),
        safe,
        schema: schema_from_field(field)?,
    }))
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
pub(crate) fn cast_reader(inner: BatchReader, field: &Field, safe: bool) -> Result<BatchReader> {
    let schema = schema_from_field(field)?;
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

/// One exact, schema-bound value stored in a one-row Arrow array.
///
/// The Arrow allocation is immutable and shallow-cloneable. The owning
/// [`Field`] retains name, nullability, dictionary options, metadata, and
/// extension identity that an array datatype alone cannot carry.
#[derive(Clone, Debug)]
pub struct ArrowScalar {
    field: Field,
    array: ArrayRef,
}

impl ArrowScalar {
    /// Validates one external Arrow array value against an exact Field.
    ///
    /// # Errors
    ///
    /// Returns an error unless `array` has length one, has the Field's exact
    /// physical datatype, and decodes to a value satisfying all recursive
    /// Field nullability and datatype constraints. A non-nullable Field may
    /// contain a logical null only when the decoded value is exactly its
    /// datatype's canonical intrinsic default. This narrow exception keeps
    /// datatype defaults such as null-only dictionaries, unions, and run-end
    /// encodings closed under [`Self::into_parts`] followed by this
    /// constructor without admitting arbitrary selected-null values.
    pub fn from_parts(field: Field, array: ArrayRef) -> Result<Self> {
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
        field.data_type().validate_bounded()?;
        let expected = field.to_arrow_ref()?.data_type().clone();
        if array.data_type() != &expected {
            return Err(Error::IncompatibleSchema(format!(
                "Arrow scalar datatype {:?} differs from expected {expected:?}",
                array.data_type()
            )));
        }
        let decoded = value::value_from_array(field.data_type(), array.as_ref(), 0)?;
        if let Err(error) = validate_scalar_value(&field, decoded.clone()) {
            if !field.data_type().is_default_value(&decoded)? {
                return Err(error);
            }
        }
        Ok(Self { field, array })
    }

    /// Materializes one validated native value as an exact Arrow scalar.
    ///
    /// # Errors
    ///
    /// Returns an error when the value violates the Field or the physical
    /// Arrow layout cannot represent it.
    pub fn from_value(field: Field, value: Value) -> Result<Self> {
        let value = validate_scalar_value(&field, value)?;
        let array = value::array_from_values(&field, &[&value])?;
        Ok(Self { field, array })
    }

    /// Returns the exact owning Field.
    pub const fn field(&self) -> &Field {
        &self.field
    }

    /// Returns the canonical logical datatype.
    pub fn data_type(&self) -> &DataType {
        self.field.data_type()
    }

    /// Borrows the one-row Arrow array reference.
    pub const fn array(&self) -> &ArrayRef {
        &self.array
    }

    /// Shallow-clones the one-row Arrow array.
    pub fn to_array(&self) -> ArrayRef {
        Arc::clone(&self.array)
    }

    /// Consumes the wrapper into its one-row Arrow array.
    pub fn into_array(self) -> ArrayRef {
        self.array
    }

    /// Decodes the scalar to its canonical native value.
    ///
    /// # Errors
    ///
    /// Returns an error if the Arrow array can no longer be decoded according
    /// to its exact Field. Arrow arrays are immutable, so this principally
    /// reports unsupported future physical layouts.
    pub fn to_value(&self) -> Result<Value> {
        value::value_from_array(self.field.data_type(), self.array.as_ref(), 0)
    }

    /// Consumes the wrapper and decodes its canonical native value.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported future physical layout.
    pub fn into_value(self) -> Result<Value> {
        value::value_from_array(self.field.data_type(), self.array.as_ref(), 0)
    }

    /// Consumes this scalar into its exact Field and one-row Arrow array.
    pub fn into_parts(self) -> (Field, ArrayRef) {
        (self.field, self.array)
    }
}

/// Extension trait for constructing the canonical default Arrow scalar.
pub trait DefaultArrowScalar {
    /// Materializes the bounded native default into an exact one-row array.
    ///
    /// # Errors
    ///
    /// Returns an error when no physically valid default exists or Arrow
    /// cannot materialize the datatype.
    fn default_arrow_scalar(&self) -> Result<ArrowScalar>;
}

impl DefaultArrowScalar for DataType {
    fn default_arrow_scalar(&self) -> Result<ArrowScalar> {
        let field = Field::new("value", self.clone(), false);
        let value = self.default_value()?;
        // Null and transparent logical wrappers with a null-only canonical
        // default may be logically null even though a bare datatype projects
        // through a synthetic non-nullable Field. Field::default_value remains
        // the sole nullability authority for a caller-owned Field.
        let array = value::array_from_values(&field, &[&value])?;
        Ok(ArrowScalar { field, array })
    }
}

impl DefaultArrowScalar for Field {
    fn default_arrow_scalar(&self) -> Result<ArrowScalar> {
        let value = self.default_value()?;
        // The core planner has already bounded and recursively validated this
        // exact Field/value pair. Keep public `from_value` defensive for
        // caller input without paying for a second Value validation here.
        let array = value::array_from_values(self, &[&value])?;
        Ok(ArrowScalar {
            field: self.clone(),
            array,
        })
    }
}

pub(crate) fn validate_scalar_value(field: &Field, value: Value) -> Result<Value> {
    // Wrap the single value in a one-column row so it goes through exactly the
    // same schema-directed validator every other value does.
    let root = Field::new(
        "ArrowScalar",
        DataType::from_fields([field.clone()])?,
        false,
    );
    let row = root.canonicalize_value(Value::from_sequence([value]))?;
    root.validate_value(&row)?;
    row.as_sequence()
        .and_then(|values| values.first())
        .cloned()
        .ok_or_else(|| Error::internal("arrow::validate_scalar_value"))
}

/// One real Arrow struct scalar paired with its exact Yggdryl root field.
#[derive(Clone, Debug)]
pub struct StructScalar {
    schema: Field,
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
                "a native Value cannot represent a null root struct".to_owned(),
            ));
        }
        ensure_struct_compatible(&schema, &array)?;
        Ok(Self { schema, array })
    }

    /// Returns the shared native schema plan.
    pub const fn schema(&self) -> &Field {
        &self.schema
    }

    /// Returns the exact root field.
    pub fn field(&self) -> &Field {
        &self.schema
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
        self.schema.index_of(name).and_then(|index| self.get(index))
    }

    /// Returns the exact Field and its zero-copy one-element Arrow slice.
    pub fn entry(&self, index: usize) -> Option<(&Field, ArrayRef)> {
        Some((self.schema.get_field(index)?, self.get(index)?))
    }

    /// Makes a shallow Arrow scalar clone.
    pub fn to_arrow_scalar(&self) -> Scalar<StructArray> {
        Scalar::new(self.array.clone())
    }

    /// Consumes this value into Arrow's scalar marker.
    pub fn into_arrow_scalar(self) -> Scalar<StructArray> {
        Scalar::new(self.array)
    }
}

/// Imports one Arrow Schema as a non-null Struct root Field.
///
/// Every Arrow field becomes one child, and the schema's own metadata becomes
/// the root's, so field identifiers written as `PARQUET:field_id` survive the
/// import. Dictionary identifiers stay as the transport delivered them.
///
/// # Errors
///
/// Returns an error when the Arrow fields cannot form a non-null Struct root.
pub fn record_schema_from_arrow(name: &str, schema: &Schema) -> Result<Field> {
    let fields = schema
        .fields()
        .iter()
        .map(|field| Field::from_arrow_ref(field.clone()).map_err(Error::Core))
        .collect::<Result<Vec<_>>>()?;
    let data_type = DataType::from_fields(fields)?;
    let field = Field::from_parts(name, data_type, false, schema.metadata().clone())?;
    field.validate_struct_root()?;
    Ok(field)
}

/// Check that a struct array carries exactly the columns a field declares.
///
/// # Errors
///
/// Returns an error naming both schemas when they disagree.
fn ensure_struct_compatible(schema: &Field, array: &StructArray) -> Result<()> {
    let expected = schema_from_field(schema)?;
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
/// # Errors
///
/// Returns an error when the Field is not a non-null Struct root or a child
/// cannot be projected to Arrow.
pub fn record_schema_to_arrow(schema: &Field) -> Result<Schema> {
    Ok(schema_from_field(schema)?.as_ref().clone())
}
