//! Schema-directed validation and canonicalization of row values: the walk
//! the module doc of [`super`] describes.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};

use smol_str::{SmolStr, format_smolstr};

use crate::boolean::boolean_from_text;
use crate::bytes::bytes_from_value;
use crate::cast::text::blank_text_read;
use crate::decimal::{validate_decimal_value, validate_decimal256_value};
use crate::floating::{FloatWidth, canonical_float, float_from_text};
use crate::integer::{
    canonical_signed, canonical_unsigned, integer_from_text, validate_integer_tuple,
    validate_signed, validate_unsigned,
};
use crate::string::str_from_value;
use crate::structure::StructType;
use crate::temporal::{validate_date64, validate_time};
use crate::{
    DataType, Error, Field, FieldSegment, Result, Scalar, Serie, TemporalKind, TimeUnit, Timezone,
};
use crate::{
    Decimal32, Decimal64, Decimal128, Interval, Str, StringType, ascii_bytes, ascii_text_sized,
    code_cell_text, default_value_for_field, uuid_bytes, uuid_parse, value_is_logically_null,
};

/// One failing value, with the path walked to reach it.
#[derive(Debug)]
pub(crate) struct ValidationFailure {
    path: Vec<FieldSegment>,
    reason: SmolStr,
}

impl ValidationFailure {
    pub(crate) fn new(reason: impl Into<SmolStr>) -> Self {
        Self {
            path: Vec::new(),
            reason: reason.into(),
        }
    }

    pub(crate) fn prepend(mut self, segment: FieldSegment) -> Self {
        self.path.insert(0, segment);
        self
    }

    pub(crate) fn prepend_segments(
        mut self,
        segments: impl IntoIterator<Item = FieldSegment>,
    ) -> Self {
        self.path.splice(0..0, segments);
        self
    }
}

impl Field {
    /// Materializes this field's bounded canonical scalar default.
    ///
    /// Nullable fields prefer logical null. Union and run-end layouts encode
    /// that null through a physically nullable logical child when possible.
    pub fn default_value(&self) -> Result<Scalar> {
        default_value_for_field(self)
    }

    /// The canonical value this field holds, from any value it accepts.
    ///
    /// [`DataType::scalar`] with this field's nullability on top: the value is
    /// checked and rewritten by the datatype, and a null is refused here when
    /// the column cannot hold one. Every other value contract in the crate is
    /// this one, so a value built here is a value every reader accepts.
    ///
    /// ```
    /// use yggdryl::{DataType, DataTypeId, Field, Scalar};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let ccy = Field::new("ccy", DataType::Ccy, false);
    /// let currency = ccy.scalar("USD\0")?;
    /// assert_eq!(currency.id(), DataTypeId::Ccy);
    /// assert_eq!(currency.as_str(), Some("USD"));
    /// assert!(ccy.scalar(Scalar::Null).is_err());
    /// assert_eq!(
    ///     Field::new("ccy", DataType::Ccy, true).scalar(Scalar::Null)?,
    ///     Scalar::Null
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming this field when the value is not one its
    /// datatype accepts, or is null under a field that is not nullable.
    pub fn scalar(&self, value: impl Into<Scalar>) -> Result<Scalar> {
        // Preserve a caller's bare absence for this field's nullability;
        // every present value crosses the datatype's canonical door first.
        let value = value.into();
        let value = if matches!(self.dtype(), DataType::Variant) && matches!(value, Scalar::Null) {
            value
        } else {
            dtype_scalar(self.dtype(), value)
                .map_err(|error| rooted_at_field(error, self.name()))?
        };
        if !self.is_nullable() && value_is_logically_null(self.dtype(), &value) {
            return Err(Error::InvalidRecord {
                path: SmolStr::from(root_path(self.name())),
                reason: SmolStr::new_static("non-nullable field received null"),
            });
        }
        Ok(value)
    }

    /// Validates one row value against this struct root.
    ///
    /// # Errors
    ///
    /// Returns an error when the root is not a struct, the row has the wrong
    /// arity, or any value violates its field's datatype or nullability.
    pub fn validate_value(&self, value: &Scalar) -> Result<()> {
        self.require_struct()?;
        validate_row(self, value)
    }

    /// Convert one value into what this field holds.
    ///
    /// [`DataType::cast_scalar`] for the datatype, then the field's own
    /// contract: a null lands only in a nullable field. This is what a
    /// declared column casts a computed value with.
    ///
    /// # Errors
    ///
    /// Returns an error when the value cannot be held by the datatype
    /// without loss, or is null in a required field.
    pub fn cast_scalar(&self, value: &Scalar) -> Result<Scalar> {
        if matches!(self.dtype(), DataType::Variant) && matches!(value, Scalar::Null) {
            return self.scalar(value.clone());
        }
        let converted = self.dtype().cast_scalar(value)?;
        if converted.is_null() && !self.is_nullable() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new(self.name()),
                reason: SmolStr::new_static("non-nullable field received null"),
            });
        }
        Ok(converted)
    }

    /// Rewrites one row value into the exact representation this root declares.
    ///
    /// # Errors
    ///
    /// Returns an error when a value cannot be represented by its field.
    pub fn canonicalize_value(&self, value: Scalar) -> Result<Scalar> {
        let value = into_row(value);
        self.validate_value(&value)?;
        canonicalize_row(self, value)
    }

    /// Rewrites one row value under a root already proven to be a Struct.
    ///
    /// [`Self::canonicalize_value`] re-checks the root before every row; a
    /// reader that checked it once when it published its schema pays that
    /// per stream, and this is the per-row half - the row's own validation
    /// and its rewrite, and nothing about the root.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::canonicalize_value`] returns for the row.
    pub(crate) fn canonicalize_row_value(&self, value: Scalar) -> Result<Scalar> {
        let value = into_row(value);
        validate_row(self, &value)?;
        canonicalize_row(self, value)
    }

    /// Recovers this field's exact value from a natural text value.
    ///
    /// # Errors
    ///
    /// Returns the first value whose natural representation cannot satisfy
    /// this field.
    pub fn from_natural_value(&self, value: Scalar) -> Result<Scalar> {
        crate::text::typed::with_field(value, self)
    }

    /// Restates this field's canonical value in the natural text shape.
    ///
    /// The read half resolves a named record to an ordered sequence;
    /// this puts the names back, recursively, so a canonical row renders as
    /// the object a structured text format is expected to contain rather than
    /// as a positional array. Leaf spellings belong to the format writers and
    /// are untouched.
    ///
    /// ```
    /// use yggdryl::{Field, Scalar};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let field = Field::from_str("row: struct<symbol: utf8, size: int64> not null")?;
    /// let row = field.from_natural_value(Scalar::from_struct([
    ///     ("symbol", Scalar::from("AAPL")),
    ///     ("size", Scalar::from(100_i64)),
    /// ])?)?;
    ///
    /// // Canonical rows are positional; the natural restatement names them.
    /// assert!(row.as_sequence().is_some());
    /// assert_eq!(
    ///     field.into_natural_value(row)?.get_key_str("symbol").and_then(Scalar::as_str),
    ///     Some("AAPL"),
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when a struct value does not carry exactly the values
    /// its Field declares, or a union value does not name a declared branch.
    pub fn into_natural_value(&self, value: Scalar) -> Result<Scalar> {
        crate::text::typed::into_natural(value, self)
    }

    /// Validates that this field is a struct, without a nullability opinion.
    ///
    /// # Errors
    ///
    /// Returns an error when the datatype is not a struct.
    pub fn require_struct(&self) -> Result<()> {
        self.validate()?;
        if !self.is_struct() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new("$"),
                reason: format_smolstr!(
                    "expected a struct root, got field {:?} of {}",
                    self.name(),
                    self.dtype()
                ),
            });
        }
        Ok(())
    }

    /// Validates that this field can serve as a record schema root.
    ///
    /// # Errors
    ///
    /// Returns an error naming what the field is when it is not a usable root.
    pub fn validate_struct_root(&self) -> Result<()> {
        self.validate()?;
        self.require_struct_root()
    }

    /// Validates that this field has the shape of a record schema root.
    ///
    /// The shape half of [`Self::validate_struct_root`]: nullability and the
    /// Struct datatype, and nothing below them. A typed row proves each cell
    /// through its child's own value contract, which is the walk
    /// [`Self::scalar`] runs without re-validating the datatype's parameters,
    /// so it asks only this per row and leaves the schema-wide validation to
    /// the boundary that published the schema.
    pub(crate) fn require_struct_root(&self) -> Result<()> {
        if self.is_nullable() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new("$"),
                reason: format_smolstr!(
                    "expected a non-null struct root, got nullable field {:?}",
                    self.name()
                ),
            });
        }
        if !self.is_struct() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new("$"),
                reason: format_smolstr!(
                    "expected a struct root, got field {:?} of {}",
                    self.name(),
                    self.dtype()
                ),
            });
        }
        Ok(())
    }
}

/// Validate one row value against a struct root field.
pub(crate) fn validate_row(root: &Field, value: &Scalar) -> Result<()> {
    let expected = root.field_len();
    if let Some(record) = value.as_struct() {
        validate_record_fields(root.fields(), record, 0)
            .map_err(|failure| validation_error(root.name(), failure))?;
        return Ok(());
    }
    let values = value.as_serie().ok_or_else(|| Error::InvalidRecord {
        path: SmolStr::new(root.name()),
        reason: format_smolstr!(
            "expected a record or {expected} ordered values, got {}",
            value.kind()
        ),
    })?;
    if values.len() != expected {
        return Err(Error::InvalidRecord {
            path: SmolStr::new(root.name()),
            reason: format_smolstr!(
                "expected {expected} values for {expected} fields, got {}",
                values.len()
            ),
        });
    }
    for (field, value) in root.fields().iter().zip(values.iter()) {
        if let Err(failure) = validate_field_value(field, &value) {
            return Err(validation_error(root.name(), failure));
        }
    }
    Ok(())
}

/// A failure of a value checked with no field around it, rooted at the value:
/// the check half of [`dtype_scalar`] reports what a column value's walk
/// reports, at the value itself.
fn rooted_failure(failure: ValidationFailure) -> Error {
    let mut path = String::from("$");
    for segment in failure.path {
        segment.append_diagnostic(&mut path);
    }
    Error::InvalidRecord {
        path: SmolStr::from(path),
        reason: failure.reason,
    }
}

/// The canonical value one datatype holds, from any value it accepts: the one
/// scalar door, which [`DataType::scalar`], [`Field::scalar`] and the
/// expression evaluator all run.
///
/// The empty-cell rule runs first: an empty text entering a datatype that does
/// not keep it *is* [`Scalar::Null`], and the rest of the door - the bare-null
/// rule, a field's nullability - answers as it does for one. The readers below
/// never see it, so `""` stays no spelling for them.
pub(crate) fn dtype_scalar(dtype: &DataType, value: Scalar) -> Result<Scalar> {
    dtype_canonical(dtype, blank_text_read(dtype, value))
}

/// The canonical value one datatype holds, from a value it accepts, with no
/// rule ahead of the reading.
///
/// Both halves of the value contract in one walk over one value: the check
/// that the datatype accepts it, then the rewrite into the exact
/// representation the datatype declares. Nothing wraps the value in a
/// synthetic row to get there, which is what a scalar used to cost. The
/// default planner materializes a member through this half alone: a code's
/// neutral member is the empty text, and a member is not caller text.
pub(crate) fn dtype_canonical(dtype: &DataType, value: Scalar) -> Result<Scalar> {
    if spells_bare_null(dtype, &value) {
        return Ok(value);
    }
    // A spelling is read once, here: the check and the rewrite below each
    // begin by reading one, so both are handed what this read answered, and
    // a refused spelling is refused as the check refuses it.
    let value = match read_as(dtype, &value) {
        Some(read) => {
            read.map_err(|error| rooted_failure(ValidationFailure::new(reason_of(&error))))?
        }
        None => value,
    };
    // Not the bare-null exemption again: it was answered for what arrived,
    // and a spelling that read as null is checked as the value it read as.
    validate_dtype_value(dtype, &value, 0).map_err(rooted_failure)?;
    // The value has no field around it, so a refusal is already rooted at the
    // value itself, exactly as the check above roots one.
    let (canonical, changed) = canonicalize_dtype_value(dtype, &value)?;
    Ok(if changed { canonical } else { value })
}

/// Whether a bare [`Scalar::Null`] is a value this datatype holds.
///
/// Absence is a value for every layout that stores it beside the values, and
/// a union and a run-end layout do not: they spell it inside a child, so the
/// value is the pair or the values entry that carries the null, never the
/// bare null itself. The crate's nested walk already applies this rule to a
/// child; it is the same rule at a root.
fn spells_bare_null(dtype: &DataType, value: &Scalar) -> bool {
    matches!(value, Scalar::Null)
        && !matches!(
            dtype,
            // An explicit Variant datatype conversion encodes null. The
            // field boundary preserves bare absence before reaching here.
            DataType::Union(..) | DataType::RunEndEncoded(_) | DataType::Variant
        )
}

/// Rewrite one row value into the exact representation a root field declares.
pub(crate) fn canonicalize_row(root: &Field, value: Scalar) -> Result<Scalar> {
    if let Some(record) = value.as_struct() {
        let cells = RowCells::Record(record);
        return Scalar::try_sequence(root.field_len(), |index| {
            cells.canonical(&root.fields()[index], index)
        })
        .map_err(|error| prepend_canonical_error(error, [FieldSegment::field(root.name())]));
    }
    let Some(values) = value.sequence_rows() else {
        return Err(Error::InvalidRecord {
            path: SmolStr::from(root_path(root.name())),
            reason: format_smolstr!(
                "expected an ordered sequence of column values, got {}",
                value.kind()
            ),
        });
    };
    let fields = root.fields();
    let canonical = canonicalize_slice(&values, |index, value| {
        canonicalize_field_value(&fields[index], value)
    })
    .map_err(|error| prepend_canonical_error(error, [FieldSegment::field(root.name())]))?;
    if let Some(canonical) = canonical {
        return Ok(canonical);
    }
    // A row is a run: a column's rows, built to be read, are the run it
    // becomes, and a run's own rows are already it.
    let built = match values {
        Cow::Owned(rows) => Some(rows),
        Cow::Borrowed(_) => None,
    };
    Ok(built.map_or(value, Scalar::from_sequence))
}

/// A row as the run it is: a column's rows built once, so the validation
/// and the rewrite that follow read one run; anything else as it is.
fn into_row(value: Scalar) -> Scalar {
    match value {
        Scalar::Serie(serie)
        | Scalar::SerieView(serie)
        | Scalar::FixedSizeSerie(serie)
        | Scalar::LargeSerie(serie)
        | Scalar::LargeSerieView(serie)
            if serie.is_column() =>
        {
            Scalar::Serie(Serie::Run(serie.into_run()))
        }
        other => other,
    }
}

/// [`into_row`] over a borrowed value: a run or a record is lent.
fn as_row(value: &Scalar) -> Cow<'_, Scalar> {
    match value {
        Scalar::Serie(serie)
        | Scalar::SerieView(serie)
        | Scalar::FixedSizeSerie(serie)
        | Scalar::LargeSerie(serie)
        | Scalar::LargeSerieView(serie)
            if serie.is_column() =>
        {
            Cow::Owned(Scalar::Serie(Serie::Run(serie.clone().into_run())))
        }
        _ => Cow::Borrowed(value),
    }
}

/// Whether a column already holds what `item` declares: its field's datatype
/// is `item`'s, and its nullability fits - `item` nullable, or no row absent.
///
/// A column holds only rows its field accepts, so one of the exact item
/// field needs no walk to be proven; a run, or a column of another field,
/// answers `false` and is read row by row.
pub(crate) fn column_fits_item(serie: &Serie, item: &Field) -> bool {
    serie.field().is_some_and(|field| {
        field.dtype() == item.dtype() && (item.is_nullable() || serie.null_count() == 0)
    })
}

/// Canonicalize a checked row cell-by-cell, without materializing its row
/// sequence first.
pub(crate) fn canonicalize_row_cells<'a>(
    root: &'a Field,
    value: &Scalar,
    mut visit: impl FnMut(&'a Field, Scalar),
) -> Result<()> {
    let value = as_row(value);
    validate_row(root, &value)?;
    let Some(cells) = RowCells::from_value(&value) else {
        return Err(Error::InvalidRecord {
            path: SmolStr::from(root_path(root.name())),
            reason: SmolStr::new_static("expected an ordered sequence of column values"),
        });
    };
    for (index, field) in root.fields().iter().enumerate() {
        let canonical = cells
            .canonical(field, index)
            .map_err(|error| prepend_canonical_error(error, [FieldSegment::field(root.name())]))?;
        visit(field, canonical);
    }
    Ok(())
}

/// The checked cells of one row. Ordered rows lend their positions - a run's
/// as they are, a column's built once; named rows borrow their entries and
/// only own a schema default when it is absent.
enum RowCells<'a> {
    Sequence(Cow<'a, [Scalar]>),
    Record(&'a BTreeMap<SmolStr, Scalar>),
}

impl<'a> RowCells<'a> {
    fn from_value(value: &'a Scalar) -> Option<Self> {
        value
            .sequence_rows()
            .map(Self::Sequence)
            .or_else(|| value.as_struct().map(Self::Record))
    }

    fn value(&self, field: &Field, index: usize) -> Result<Cow<'_, Scalar>> {
        match self {
            Self::Sequence(values) => Ok(Cow::Borrowed(&values[index])),
            Self::Record(record) => record
                .get(field.name())
                .map(Cow::Borrowed)
                .map_or_else(|| field.default_value().map(Cow::Owned), Ok),
        }
    }

    fn canonical(&self, field: &Field, index: usize) -> Result<Scalar> {
        let value = self.value(field, index)?;
        canonicalize_field_value(field, &value).map(|(canonical, _)| canonical)
    }
}

/// Re-root one value refusal at the field that refused it.
fn rooted_at_field(error: Error, name: &str) -> Error {
    prepend_canonical_error(error, [FieldSegment::field(name)])
}

/// Render the `$`-rooted path of a schema root.
pub(crate) fn root_path(name: &str) -> String {
    let mut path = String::from("$");
    crate::path::push_field_name(&mut path, name);
    path
}

fn canonicalize_field_value(field: &Field, value: &Scalar) -> Result<(Scalar, bool)> {
    canonicalize_field_payload(field, value)
        .map_err(|error| prepend_canonical_error(error, [FieldSegment::field(field.name())]))
}

fn canonicalize_field_payload(field: &Field, value: &Scalar) -> Result<(Scalar, bool)> {
    if matches!(value, Scalar::Null) {
        return Ok((Scalar::Null, false));
    }
    canonicalize_dtype_value(field.dtype(), value)
}

/// The physical count a self-describing value carries for one column.
///
/// A decimal and a temporal each remember the scale or unit they were built
/// with, so a column declaring another scale or unit restates them, and only
/// when the restatement is exact. Every other value already *is* the physical
/// count the column stores and answers `None`, as does a restatement that would
/// have dropped a digit - which then fails the ordinary check below, naming the
/// kind that did not fit.
fn restated(dtype: &DataType, value: &Scalar) -> Option<i128> {
    use DataType as D;
    match dtype {
        D::Decimal32 { scale, .. } | D::Decimal64 { scale, .. } | D::Decimal128 { scale, .. }
            if value.is_decimal() =>
        {
            value.decimal_unscaled_at(*scale)
        }
        D::DateTime64 { unit, timezone }
            if temporal_matches(value, TemporalKind::DateTime, Some(timezone)) =>
        {
            value.temporal_count_at(*unit).map(i128::from)
        }
        D::Duration32(unit) | D::Duration64(unit)
            if temporal_matches(value, TemporalKind::Duration, None) =>
        {
            value.temporal_count_at(*unit).map(i128::from)
        }
        D::Time32(unit) | D::Time64(unit) if temporal_matches(value, TemporalKind::Time, None) => {
            value.temporal_count_at(*unit).map(i128::from)
        }
        D::Date32 | D::Date64 if temporal_matches(value, TemporalKind::Date, None) => {
            let leaf = dtype.date_type()?;
            value.temporal_count_at(leaf.unit()).map(i128::from)
        }
        _ => None,
    }
}

/// The value a datatype reads out of a spelling that is not its own storage.
///
/// A datatype declares one storage and a caller may hold the same value under
/// another: text spells every number, boolean, decimal and temporal, and every
/// value carrying characters or bytes spells a byte payload. The readings are
/// the ones this crate already owns - [`Scalar::from_temporal_text`] is what a
/// *text column* entering a temporal reads through, and each family's
/// canonical `Display` is what it prints - so a row takes the spellings a
/// column takes rather than a narrower set of its own.
///
/// `None` means there is nothing to read: the value is already in the
/// datatype's own family, or it carries no spelling of it, and the ordinary
/// refusal names what arrived. `Some(Err)` is a spelling that was read and
/// then refused, which keeps the reader's own reason.
fn read_as(dtype: &DataType, value: &Scalar) -> Option<Result<Scalar>> {
    use DataType as D;
    // A value already in the datatype's own family has no spelling to read,
    // and that is what a canonical row is made of, so the walk answers it with
    // one comparison rather than by falling through every arm below.
    if dtype.id() == value.id() {
        return None;
    }
    // A variant is a spelling of the value its bytes hold: every other
    // datatype reads it as that value, which is what makes a variant column
    // castable to the columns its values would be. The variant datatype
    // itself returned above, its ids being equal.
    if let Scalar::Variant(held) = value {
        return Some(held.scalar());
    }
    match dtype {
        // A string column stores the spelling every tier prints, and bytes
        // arriving at one are read through the charset the column declares:
        // the payload is that charset by definition, so decoding it here is
        // what the declaration is for. `StringType::scalar_from_bytes` is
        // the one door that decides how strict that read is.
        crate::string_dtypes!() if value.as_string().is_none() => match value.as_binary() {
            Some(bytes) => Some(
                dtype
                    .string_parameters()
                    .expect("a string leaf")
                    .scalar_from_bytes(bytes.as_bytes()),
            ),
            None => Some(str_from_value(value)?.map(Scalar::from)),
        },
        // A byte column stores one payload, however the value spells it. The
        // declared leaf is the offset width, which the restatement below
        // retags without copying the payload.
        crate::bytes_dtypes!() if value.as_binary().is_none() => {
            Some(Ok(Scalar::from(bytes_from_value(value)?)))
        }
        // A record is a name-to-value map, so a map column reads it as its
        // entries; the key field then reads each name as its own datatype,
        // exactly as a struct root reads a record's field names.
        D::Map(_) | D::SortedMap(_) => {
            let record = value.as_struct()?;
            Some(Scalar::from_mapping(
                record
                    .iter()
                    .map(|(name, value)| (Scalar::from(name.as_str()), value.clone()))
                    .collect::<Vec<_>>(),
            ))
        }
        _ => read_text_as(dtype, text_reading(value)?),
    }
}

/// The text a value offers a datatype that stores something else.
///
/// Only a string, of any leaf, is a spelling waiting to be read. A code and
/// a generic enum member also answer [`Scalar::as_str`], but their identity
/// is the registry and the member rather than the characters, and a column
/// refuses them into a number for the same reason.
fn text_reading(value: &Scalar) -> Option<&str> {
    value.as_string().map(Str::as_str)
}

/// Read one text spelling into the family a datatype declares.
fn read_text_as(dtype: &DataType, text: &str) -> Option<Result<Scalar>> {
    use DataType as D;
    Some(match dtype {
        D::Boolean => Ok(boolean_from_text(text)?),
        D::Int8 | D::Int16 | D::Int32 | D::Int64 | D::UInt8 | D::UInt16 | D::UInt32 | D::UInt64 => {
            Ok(integer_from_text(text)?)
        }
        D::Float16 | D::Float32 | D::Float64 => Ok(float_from_text(text)?),
        D::Decimal32 { .. } | D::Decimal64 { .. } | D::Decimal128 { .. } | D::Decimal256 { .. } => {
            Scalar::from_decimal_text(dtype, text)
        }
        D::Date32
        | D::Date64
        | D::Time32(_)
        | D::Time64(_)
        | D::DateTime64 { .. }
        | D::Duration32(_)
        | D::Duration64(_) => Scalar::from_temporal_text(dtype, text),
        _ => return None,
    })
}

/// Check the logical temporal family and the zone a datatype can preserve.
fn temporal_matches(
    value: &Scalar,
    family: TemporalKind,
    expected_zone: Option<&Timezone>,
) -> bool {
    let Some(zone) = value.temporal_timezone() else {
        return false;
    };
    if value.temporal_kind() != Some(family) {
        return false;
    }
    match (family, expected_zone) {
        (TemporalKind::DateTime, Some(expected)) => zone == *expected,
        (TemporalKind::DateTime, None) => zone.is_naive(),
        _ => zone.is_naive(),
    }
}

/// Read a value's coefficient at the scale a decimal column declares.
///
/// A decimal is restated at that scale. A whole number is a decimal of scale
/// zero and is restated the same way, because that is what it is: one hundred
/// written into `decimal(12, 2)` is `100.00`, which is already what the same
/// value answers spelled as a decimal, spelled as text, and cast into that
/// column by Arrow itself.
///
/// `None` when no exact restatement exists, which every caller reports naming
/// the width it was writing into.
fn decimal_coefficient_at(value: &Scalar, scale: i8) -> Option<crate::i256> {
    if value.is_decimal() {
        return value.decimal256_unscaled_at(scale);
    }
    // Scale zero is the whole number's own scale, so the one restatement
    // implementation answers this too rather than being written out again.
    Scalar::d256(crate::i256::from_i128(value.as_i128()?), 0).decimal256_unscaled_at(scale)
}

#[allow(clippy::too_many_lines)]
fn canonicalize_dtype_value(dtype: &DataType, value: &Scalar) -> Result<(Scalar, bool)> {
    use DataType as D;
    // A spelling is read once, into the datatype's own family; the walk below
    // then restates that value into the exact representation declared. A
    // reading always rewrote something, so it is always a change.
    if let Some(read) = read_as(dtype, value) {
        let (canonical, _) = canonicalize_dtype_value(dtype, &read?)?;
        return Ok((canonical, true));
    }
    match dtype {
        D::Decimal32 { scale, .. } => {
            let coefficient = decimal_coefficient_at(value, *scale)
                .and_then(|wide| wide.as_i128())
                .ok_or_else(|| Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("expected a d32 representable at scale {scale}"),
                })?;
            let coefficient = i32::try_from(coefficient).map_err(|_| {
                canonical_error("decimal32 coefficient does not fit signed 32 bits")
            })?;
            let canonical = Scalar::Decimal32(Decimal32::new(coefficient, *scale));
            let changed = !same_decimal_representation(value, &canonical);
            return Ok((canonical, changed));
        }
        D::Decimal64 { scale, .. } => {
            let coefficient = decimal_coefficient_at(value, *scale)
                .and_then(|wide| wide.as_i128())
                .ok_or_else(|| Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("expected a d64 representable at scale {scale}"),
                })?;
            let coefficient = i64::try_from(coefficient).map_err(|_| {
                canonical_error("decimal64 coefficient does not fit signed 64 bits")
            })?;
            let canonical = Scalar::Decimal64(Decimal64::new(coefficient, *scale));
            let changed = !same_decimal_representation(value, &canonical);
            return Ok((canonical, changed));
        }
        D::Decimal128 { scale, .. } => {
            let coefficient = decimal_coefficient_at(value, *scale)
                .and_then(|wide| wide.as_i128())
                .ok_or_else(|| Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("expected a d128 representable at scale {scale}"),
                })?;
            let canonical = Scalar::Decimal128(Decimal128::new(coefficient, *scale));
            let changed = !same_decimal_representation(value, &canonical);
            return Ok((canonical, changed));
        }
        D::Decimal256 { scale, .. } => {
            let coefficient =
                decimal_coefficient_at(value, *scale).ok_or_else(|| Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("expected a d256 representable at scale {scale}"),
                })?;
            let canonical = Scalar::d256(coefficient, *scale);
            let changed = !same_decimal_representation(value, &canonical);
            return Ok((canonical, changed));
        }
        D::Date32 => {
            let count = temporal_or_integer(value, TimeUnit::Day, TemporalKind::Date, None)?;
            let canonical = Scalar::date32(
                i32::try_from(count)
                    .map_err(|_| canonical_error("date32 count does not fit signed 32 bits"))?,
            );
            let changed = !same_temporal_representation(value, &canonical);
            return Ok((canonical, changed));
        }
        D::Date64 => {
            let count =
                temporal_or_integer(value, TimeUnit::Millisecond, TemporalKind::Date, None)?;
            let canonical = Scalar::date64(count);
            let changed = !same_temporal_representation(value, &canonical);
            return Ok((canonical, changed));
        }
        D::Time32(unit) => {
            let count = temporal_or_integer(value, *unit, TemporalKind::Time, None)?;
            let canonical = Scalar::time32(
                i32::try_from(count)
                    .map_err(|_| canonical_error("time32 count does not fit signed 32 bits"))?,
                *unit,
                Timezone::NAIVE,
            )?;
            let changed = !same_temporal_representation(value, &canonical);
            return Ok((canonical, changed));
        }
        D::Time64(unit) => {
            let count = temporal_or_integer(value, *unit, TemporalKind::Time, None)?;
            let canonical = Scalar::time64(count, *unit, Timezone::NAIVE)?;
            let changed = !same_temporal_representation(value, &canonical);
            return Ok((canonical, changed));
        }
        D::DateTime64 { unit, timezone } => {
            let count = temporal_or_integer(value, *unit, TemporalKind::DateTime, Some(timezone))?;
            let canonical = Scalar::datetime64(count, *unit, *timezone)?;
            let changed = !same_temporal_representation(value, &canonical);
            return Ok((canonical, changed));
        }
        D::Duration32(unit) => {
            let count = temporal_or_integer(value, *unit, TemporalKind::Duration, None)?;
            let canonical = Scalar::duration32(
                i32::try_from(count)
                    .map_err(|_| canonical_error("duration32 count does not fit signed 32 bits"))?,
                *unit,
            )?;
            let changed = !same_temporal_representation(value, &canonical);
            return Ok((canonical, changed));
        }
        D::Duration64(unit) => {
            let count = temporal_or_integer(value, *unit, TemporalKind::Duration, None)?;
            let canonical = Scalar::duration64(count, *unit)?;
            let changed = !same_temporal_representation(value, &canonical);
            return Ok((canonical, changed));
        }
        _ => {}
    }
    // Every datatype `restated` names returned from the match above, which
    // restates the value inline; only validation, which has no such match,
    // still consults it.
    match dtype {
        D::Null | D::Boolean => Ok((value.clone(), false)),
        D::Int8 | D::Int16 | D::Int32 | D::Int64 => canonical_signed(dtype, value),
        D::Interval(leaf) => canonical_interval(*leaf, value),
        D::UInt8 | D::UInt16 | D::UInt32 | D::UInt64 => canonical_unsigned(dtype, value),
        D::Float16 => canonical_float(value, FloatWidth::Float16),
        D::Float32 => canonical_float(value, FloatWidth::Float32),
        D::Float64 => canonical_float(value, FloatWidth::Float64),
        // A byte leaf is an Arrow offset width over the same payload, so a
        // value already of the declared leaf - its number included - is its
        // own canonical form once it fits, and nothing is built; a rewrite
        // adopts the source's storage under the column's leaf rather than
        // copying the payload into a second buffer. The reading above
        // rewrote every other kind into bytes, so only bytes reach here.
        crate::bytes_dtypes!() => {
            let (Some(leaf), Some(source)) = (dtype.bytes_parameters(), value.as_binary()) else {
                return canonicalization_failure(dtype);
            };
            let restated = leaf.admit(source.clone())?;
            match value.bytes_parameters() == Some(leaf) {
                true => Ok((value.clone(), false)),
                false => Ok((leaf.adopt(restated), true)),
            }
        }
        // A string is one layout, one charset and one number, and a value
        // already of that leaf is its own canonical form once it fits.
        // Anything else adopts the source's storage handle under the
        // column's leaf rather than copying its characters into a second
        // buffer: the reading above rewrote every other kind into a string,
        // so only a string reaches here.
        crate::string_dtypes!() => {
            let (Some(leaf), Some(source)) = (dtype.string_parameters(), value.as_string()) else {
                return canonicalization_failure(dtype);
            };
            if string_matches(leaf, value) {
                check_string_bound(leaf, source.as_str())?;
                return Ok((value.clone(), false));
            }
            Ok((leaf.adopt(leaf.admit(source.clone())?), true))
        }
        // The canonical code spelling is the trimmed string; bytes and a
        // string carrying trailing NULs are rewritten here, at the width the
        // code's own type fixes. A value already stored as this code holds
        // that trimmed text, so it is returned without re-walking its bytes.
        D::Country
        | D::Ccy
        | D::MicCode
        | D::CfiCode
        | D::IsinCode
        | D::CusipCode
        | D::SedolCode
        | D::BloombergCode
        | D::FIGICode
        | D::Side
        | D::State
        | D::TimeInForce
        | D::Unit => {
            if value.is_code() && value.id() == dtype.id() {
                return Ok((value.clone(), false));
            }
            let Some(bytes) = ascii_bytes(value) else {
                return canonicalization_failure(dtype);
            };
            // A side and a state are read by their spelling, and a name is
            // longer than the value it names - `SellShortExempt` for
            // `SSHORTEX` - so the width holds the value read, never the
            // spelling; every other code is the text it is, at its width.
            let text = match dtype {
                D::Side | D::State => ascii_text_sized(None, bytes)?,
                _ => code_cell_text(dtype, bytes)?,
            };
            let canonical = match dtype {
                D::Country => Scalar::Country(crate::Country::new(text)?),
                D::Ccy => Scalar::Ccy(crate::Ccy::new(text)?),
                D::MicCode => Scalar::MicCode(crate::MicCode::new(text)?),
                D::CfiCode => Scalar::CfiCode(crate::CfiCode::new(text)?),
                D::IsinCode => Scalar::IsinCode(crate::IsinCode::new(text)?),
                D::CusipCode => Scalar::CusipCode(crate::CusipCode::new(text)?),
                D::SedolCode => Scalar::SedolCode(crate::SedolCode::new(text)?),
                D::BloombergCode => Scalar::BloombergCode(crate::BloombergCode::new(text)?),
                D::FIGICode => Scalar::FIGICode(crate::FIGICode::new(text)?),
                // A side and a state are read by their spelling: the wire
                // code, the specification's name or a stored value all reach
                // the one explicit value, and a spelling that names none is
                // refused rather than stored unread.
                D::Side => Scalar::Side(crate::Side::read(text)?),
                D::State => Scalar::State(crate::State::read(text)?),
                D::TimeInForce => Scalar::TimeInForce(crate::TimeInForce::new(text)?),
                D::Unit => Scalar::Unit(crate::Unit::new(text)?),
                _ => unreachable!("registered code matched above"),
            };
            Ok((canonical, true))
        }
        // The canonical UUID spelling is the hyphenated text; the sixteen
        // stored bytes and the bare-hex spelling are rewritten here.
        D::Uuid => {
            if let Scalar::Uuid(_) = value {
                return Ok((value.clone(), false));
            }
            let bytes =
                uuid_bytes(value).ok_or_else(|| canonical_error("expected UUID text or bytes"))?;
            let uuid = crate::Uuid::new(u128::from_be_bytes(uuid_parse(bytes)?));
            Ok((Scalar::Uuid(uuid), true))
        }
        D::Version => match value {
            Scalar::Version(_) => Ok((value.clone(), false)),
            crate::string_scalars!(text) => text
                .as_str()
                .parse::<crate::Version>()
                .map(|version| (Scalar::Version(version), true))
                .map_err(|error| Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("expected version text: {error}"),
                }),
            _ => canonicalization_failure(dtype),
        },
        D::Url => match value {
            Scalar::Url(_) => Ok((value.clone(), false)),
            // Text is canonicalized on the way in, so a column of URLs holds
            // one spelling per location however it was written.
            crate::string_scalars!(text) => crate::Url::from_str(text.as_str())
                .map(|url| (Scalar::Url(std::sync::Arc::new(url)), true))
                .map_err(|error| Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("expected url text: {error}"),
                }),
            _ => canonicalization_failure(dtype),
        },
        D::Urn => match value {
            Scalar::Urn(_) => Ok((value.clone(), false)),
            crate::string_scalars!(text) => crate::Urn::from_str(text.as_str())
                .map(|urn| (Scalar::Urn(std::sync::Arc::new(urn)), true))
                .map_err(|error| Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("expected urn text: {error}"),
                }),
            _ => canonicalization_failure(dtype),
        },
        // A zone, a MIME type and a media type each canonicalize their own
        // text - an alias, a case, a parameter order - so text on the way in
        // crosses the same `from_str` every other spelling of them crosses.
        D::Timezone => match value {
            Scalar::Timezone(_) => Ok((value.clone(), false)),
            crate::string_scalars!(text) => crate::Timezone::from_str(text.as_str())
                .map(|zone| (Scalar::Timezone(zone), true))
                .map_err(|error| Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("expected timezone text: {error}"),
                }),
            _ => canonicalization_failure(dtype),
        },
        D::MimeType => match value {
            Scalar::MimeType(_) => Ok((value.clone(), false)),
            crate::string_scalars!(text) => crate::MimeType::from_str(text.as_str())
                .map(|mime| (Scalar::MimeType(mime), true))
                .map_err(|error| Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("expected mimetype text: {error}"),
                }),
            _ => canonicalization_failure(dtype),
        },
        D::MediaType => match value {
            Scalar::MediaType(_) => Ok((value.clone(), false)),
            crate::string_scalars!(text) => crate::MediaType::from_str(text.as_str())
                .map(|media| (Scalar::from(media), true))
                .map_err(|error| Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("expected mediatype text: {error}"),
                }),
            _ => canonicalization_failure(dtype),
        },
        D::Serie(field)
        | D::SerieView(field)
        | D::FixedSizeSerie(field, _)
        | D::LargeSerie(field)
        | D::LargeSerieView(field) => {
            canonical_sequence(field, value).map(|held| declared(dtype, value, held))
        }
        D::Struct(fields) => canonical_struct(fields, value),
        D::Union(fields, _) => canonical_union(fields, value),
        D::Dictionary(dictionary) => canonicalize_dtype_value(dictionary.value(), value),
        D::Decimal32 { .. }
        | D::Decimal64 { .. }
        | D::Decimal128 { .. }
        | D::Decimal256 { .. }
        | D::DateTime64 { .. }
        | D::Date32
        | D::Date64
        | D::Time32(_)
        | D::Time64(_)
        | D::Duration32(_)
        | D::Duration64(_) => unreachable!("typed scalars returned above"),
        map_dtype @ (D::Map(_) | D::SortedMap(_)) => {
            let map = &map_dtype
                .as_mapping()
                .expect("the variant was just matched");
            canonical_map(map, value).map(|held| declared(dtype, value, held))
        }
        D::RunEndEncoded(encoded) => canonicalize_field_value(encoded.values(), value),
        // A variant column holds variant values, so a value entering one
        // is encoded here - canonically, keys sorted and sizes narrowest -
        // and a value already encoded is answered untouched.
        D::Variant => match value {
            Scalar::Variant(_) => Ok((value.clone(), false)),
            held => Ok((Scalar::Variant(crate::Variant::encode(held)?), true)),
        },
        // The canonical geospatial spelling is `Scalar::Geometry` or
        // `Scalar::Geography`; plain
        // bytes are accepted on the way in and rewritten here.
        D::Geometry(_) | D::Geography(_) => {
            // The payload is read once, when the value is built. A value
            // already carrying this interpretation costs neither a copy of
            // its payload nor a second read of its own framing.
            if matches!(
                (dtype, value),
                (D::Geometry(_), Scalar::Geometry(_)) | (D::Geography(_), Scalar::Geography(_))
            ) {
                return Ok((value.clone(), false));
            }
            let Some(bytes) = bytes_from_value(value) else {
                return canonicalization_failure(dtype);
            };
            let canonical = match dtype {
                D::Geometry(_) => Scalar::Geometry(crate::Geometry::new(bytes)?),
                D::Geography(_) => Scalar::Geography(crate::Geography::new(bytes)?),
                _ => unreachable!("geospatial datatype matched above"),
            };
            Ok((canonical, true))
        }
    }
}

fn temporal_or_integer(
    value: &Scalar,
    unit: TimeUnit,
    family: TemporalKind,
    zone: Option<&Timezone>,
) -> Result<i64> {
    if value.is_temporal() {
        if !temporal_matches(value, family, zone) {
            return Err(canonical_error(
                "temporal family or timezone does not match the declared datatype",
            ));
        }
        return value.temporal_count_at(unit).ok_or_else(|| {
            canonical_error("temporal count cannot be represented in the declared unit")
        });
    }
    value
        .as_i128()
        .and_then(|value| i64::try_from(value).ok())
        .ok_or_else(|| canonical_error("expected a signed 64-bit temporal count"))
}

/// The reason one error carries, as a record error restates it.
pub(crate) fn reason_of(error: &Error) -> SmolStr {
    match error {
        Error::Parse {
            target,
            position,
            reason,
        } => format_smolstr!("expected an ISO {target}: {reason} at byte {position}"),
        Error::InvalidRecord { reason, .. } => reason.clone(),
        other => SmolStr::new(other.to_string()),
    }
}

pub(crate) fn canonical_error(reason: &'static str) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: reason.into(),
    }
}

fn same_decimal_representation(left: &Scalar, right: &Scalar) -> bool {
    match (left, right) {
        (Scalar::Decimal32(left), Scalar::Decimal32(right)) => left == right,
        (Scalar::Decimal64(left), Scalar::Decimal64(right)) => left == right,
        (Scalar::Decimal128(left), Scalar::Decimal128(right)) => left == right,
        (Scalar::Decimal256(left), Scalar::Decimal256(right)) => left == right,
        _ => false,
    }
}

fn same_temporal_representation(left: &Scalar, right: &Scalar) -> bool {
    match (left, right) {
        (Scalar::Date32(left), Scalar::Date32(right)) => left == right,
        (Scalar::Date64(left), Scalar::Date64(right)) => left == right,
        (Scalar::Time32(left), Scalar::Time32(right)) => left == right,
        (Scalar::Time64(left), Scalar::Time64(right)) => left == right,
        (Scalar::DateTime64(left), Scalar::DateTime64(right)) => left == right,
        (Scalar::Duration32(left), Scalar::Duration32(right)) => left == right,
        (Scalar::Duration64(left), Scalar::Duration64(right)) => left == right,
        (Scalar::Interval(left), Scalar::Interval(right)) => left == right,
        _ => false,
    }
}

fn canonical_interval(unit: TimeUnit, value: &Scalar) -> Result<(Scalar, bool)> {
    if let Scalar::Interval(interval) = value {
        if interval.unit() != unit {
            return Err(canonical_error(
                "interval layout does not match the declared datatype",
            ));
        }
        return Ok((value.clone(), false));
    }

    let component = |value: &Scalar, name: &'static str| {
        value
            .as_i128()
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| canonical_error(name))
    };
    let interval = match unit {
        TimeUnit::YearMonth => Interval::new(
            component(value, "interval month count does not fit signed 32 bits")?,
            0,
            0,
            unit,
        )?,
        TimeUnit::DayTime => {
            let components = value
                .sequence_rows()
                .ok_or_else(|| canonical_error("expected a [days, milliseconds] interval"))?;
            let [days, milliseconds] = &*components else {
                return Err(canonical_error(
                    "day_time interval requires exactly two components",
                ));
            };
            let days = component(days, "interval day count does not fit signed 32 bits")?;
            let milliseconds = component(
                milliseconds,
                "interval millisecond count does not fit signed 32 bits",
            )?;
            Interval::new(0, days, i64::from(milliseconds) * 1_000_000, unit)?
        }
        TimeUnit::MonthDayNano => {
            let components = value.sequence_rows().ok_or_else(|| {
                canonical_error("expected a [months, days, nanoseconds] interval")
            })?;
            let [months, days, nanoseconds] = &*components else {
                return Err(canonical_error(
                    "month_day_nano interval requires exactly three components",
                ));
            };
            let nanoseconds = nanoseconds
                .as_i128()
                .and_then(|value| i64::try_from(value).ok())
                .ok_or_else(|| {
                    canonical_error("interval nanosecond count does not fit signed 64 bits")
                })?;
            Interval::new(
                component(months, "interval month count does not fit signed 32 bits")?,
                component(days, "interval day count does not fit signed 32 bits")?,
                nanoseconds,
                unit,
            )?
        }
        _ => return Err(canonical_error("invalid interval layout")),
    };
    Ok((Scalar::Interval(interval), true))
}

/// A serie value may stay a column: one of the exact item field is answered
/// untouched, because its door proved every row; any other sequence is read
/// row by row and, where it was a column, answers the run of its rows.
/// A canonical sequence or mapping moved into the variant `dtype` declares,
/// reporting a rewrite when the variant moved.
fn declared(dtype: &DataType, value: &Scalar, (held, changed): (Scalar, bool)) -> (Scalar, bool) {
    let held = dtype.declared_layout(held);
    let moved = std::mem::discriminant(&held) != std::mem::discriminant(value);
    (held, changed || moved)
}

fn canonical_sequence(item: &Field, value: &Scalar) -> Result<(Scalar, bool)> {
    let Some(serie) = value.as_serie() else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("validated sequence could not be canonicalized"),
        });
    };
    if column_fits_item(serie, item) {
        return Ok((value.clone(), false));
    }
    let values = serie.rows();
    if let Some(canonical) = canonicalize_slice(&values, |index, value| {
        canonicalize_field_value(item, value).map_err(|error| {
            prepend_canonical_error(
                error,
                [FieldSegment::index(
                    i64::try_from(index).expect("allocated index fits i64"),
                )],
            )
        })
    })? {
        return Ok((canonical, true));
    }
    Ok(match values {
        Cow::Owned(rows) => (Scalar::from_sequence(rows), true),
        Cow::Borrowed(_) => (value.clone(), false),
    })
}

fn canonical_struct(fields: &StructType, value: &Scalar) -> Result<(Scalar, bool)> {
    if let Some(record) = value.as_struct() {
        let cells = RowCells::Record(record);
        let sequence =
            Scalar::try_sequence(fields.len(), |index| cells.canonical(&fields[index], index))?;
        return Ok((sequence, true));
    }
    let Some(values) = value.sequence_rows() else {
        return canonicalization_failure(&DataType::Struct(fields.clone()));
    };
    if let Some(canonical) = canonicalize_slice(&values, |index, value| {
        canonicalize_field_value(&fields[index], value)
    })? {
        return Ok((canonical, true));
    }
    // A row is a run: a column's rows, built to be read, are the run it
    // becomes.
    Ok(match values {
        Cow::Owned(rows) => (Scalar::from_sequence(rows), true),
        Cow::Borrowed(_) => (value.clone(), false),
    })
}

fn canonical_union(fields: &crate::UnionFields, value: &Scalar) -> Result<(Scalar, bool)> {
    let Some(pair) = value.sequence_rows() else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("validated union could not be canonicalized"),
        });
    };
    // A union value is a run: one spelled as a column is rebuilt as one.
    let column = matches!(pair, Cow::Owned(_));
    let [type_id, payload] = &*pair else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("validated union could not be canonicalized"),
        });
    };
    let Some(type_id_number) = type_id.as_i128().and_then(|value| i8::try_from(value).ok()) else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("validated union type id could not be canonicalized"),
        });
    };
    let Some((_, field)) = fields
        .iter()
        .find(|(candidate, _)| *candidate == type_id_number)
    else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: SmolStr::new_static("validated union branch could not be canonicalized"),
        });
    };
    let (payload, payload_changed) = canonicalize_field_value(field, payload).map_err(|error| {
        prepend_canonical_error(
            error,
            [
                FieldSegment::field("union"),
                FieldSegment::index(i64::from(type_id_number)),
            ],
        )
    })?;
    // The type id has one canonical representation - the `Int64` a union row
    // reads back as - so a narrower spelling of the same number is a change,
    // and the canonical value no longer depends on whether the payload needed
    // one too.
    let id_changed = !matches!(type_id, Scalar::Int64(_));
    if id_changed || payload_changed || column {
        Ok((
            Scalar::from_sequence([Scalar::from(i64::from(type_id_number)), payload]),
            true,
        ))
    } else {
        Ok((value.clone(), false))
    }
}

fn canonical_map(map: &crate::MappingType, value: &Scalar) -> Result<(Scalar, bool)> {
    let Some(entries) = value.as_mapping() else {
        return canonicalization_failure(&DataType::from(map.clone()));
    };
    let Some([key_field, value_field]) = map.entries().dtype().as_fields() else {
        return canonicalization_failure(&DataType::from(map.clone()));
    };
    for (index, (key, entry_value)) in entries.iter().enumerate() {
        let (canonical_key, key_changed) =
            canonicalize_field_payload(key_field, key).map_err(|error| {
                prepend_canonical_error(
                    error,
                    [
                        FieldSegment::index(
                            i64::try_from(index).expect("allocated index fits i64"),
                        ),
                        FieldSegment::field("key"),
                    ],
                )
            })?;
        let (canonical_value, value_changed) = canonicalize_field_payload(value_field, entry_value)
            .map_err(|error| {
                prepend_canonical_error(
                    error,
                    [
                        FieldSegment::index(
                            i64::try_from(index).expect("allocated index fits i64"),
                        ),
                        FieldSegment::field("value"),
                    ],
                )
            })?;
        if key_changed || value_changed {
            let mut canonical = Vec::with_capacity(entries.len());
            canonical.extend_from_slice(&entries[..index]);
            canonical.push((canonical_key, canonical_value));
            for (offset, (key, entry_value)) in entries[index + 1..].iter().enumerate() {
                let entry_index = index + 1 + offset;
                canonical.push((
                    canonicalize_field_payload(key_field, key)
                        .map_err(|error| {
                            prepend_canonical_error(
                                error,
                                [
                                    FieldSegment::index(
                                        i64::try_from(entry_index)
                                            .expect("allocated index fits i64"),
                                    ),
                                    FieldSegment::field("key"),
                                ],
                            )
                        })?
                        .0,
                    canonicalize_field_payload(value_field, entry_value)
                        .map_err(|error| {
                            prepend_canonical_error(
                                error,
                                [
                                    FieldSegment::index(
                                        i64::try_from(entry_index)
                                            .expect("allocated index fits i64"),
                                    ),
                                    FieldSegment::field("value"),
                                ],
                            )
                        })?
                        .0,
                ));
            }
            let canonical = Scalar::from_mapping(canonical)?;
            if let Some((index, reason)) =
                broken_map_invariant(map, canonical.as_mapping().unwrap_or_default())
            {
                return Err(Error::InvalidRecord {
                    path: {
                        let mut path = String::from("$");
                        for segment in [
                            FieldSegment::index(
                                i64::try_from(index).expect("allocated index fits i64"),
                            ),
                            FieldSegment::field("key"),
                        ] {
                            segment.append_diagnostic(&mut path);
                        }
                        SmolStr::from(path)
                    },
                    reason: format_smolstr!(
                        "{reason} after schema-directed physical normalization"
                    ),
                });
            }
            return Ok((canonical, true));
        }
    }
    Ok((value.clone(), false))
}

/// The first entry breaking an invariant a map carries past its two fields.
///
/// A map is a key-to-value function, so a key appears once, and a map that
/// declares sorted keys carries them in order. The column tier checks both on
/// every batch it ingests, so the row tier checks the entries a caller
/// declares and checks them again once canonicalization has run - narrowing
/// two distinct keys can collide them, and restating them can reorder them.
fn broken_map_invariant(
    map: &crate::MappingType,
    entries: &[(Scalar, Scalar)],
) -> Option<(usize, &'static str)> {
    if let Some(index) = duplicate_mapping_key_index(entries) {
        return Some((index, "map keys collide"));
    }
    if map.keys_sorted() {
        if let Some(index) = entries.windows(2).position(|pair| pair[0].0 > pair[1].0) {
            return Some((index + 1, "map keys are not sorted"));
        }
    }
    None
}

fn duplicate_mapping_key_index(entries: &[(Scalar, Scalar)]) -> Option<usize> {
    if entries.len() <= 16 {
        return (1..entries.len()).find(|index| {
            entries[..*index]
                .iter()
                .any(|(key, _)| key == &entries[*index].0)
        });
    }
    // `Scalar`'s hash reads canonical content only, never the
    // interior-mutable caches a datatype holds, so the key is stable.
    #[allow(clippy::mutable_key_type)]
    let mut seen = HashSet::with_capacity(entries.len());
    entries
        .iter()
        .enumerate()
        .find_map(|(index, (key, _))| (!seen.insert(key)).then_some(index))
}

fn canonicalize_slice(
    values: &[Scalar],
    mut canonicalize: impl FnMut(usize, &Scalar) -> Result<(Scalar, bool)>,
) -> Result<Option<Scalar>> {
    for (index, value) in values.iter().enumerate() {
        let (canonical_value, changed) = canonicalize(index, value)?;
        if changed {
            let mut changed_value = canonical_value;
            return Scalar::try_sequence(values.len(), |output_index| {
                if output_index < index {
                    Ok(values[output_index].clone())
                } else if output_index == index {
                    Ok(std::mem::replace(&mut changed_value, Scalar::Null))
                } else {
                    canonicalize(output_index, &values[output_index]).map(|(value, _)| value)
                }
            })
            .map(Some);
        }
    }
    Ok(None)
}

/// Whether a string value is already stored the way its column declares.
///
/// The value's leaf, its number included, is the column's, and a fixed
/// leaf's value holds no trailing NUL - that is the slot's padding, taken off
/// where a value is admitted. The maximum is checked beside it.
fn string_matches(leaf: StringType, value: &Scalar) -> bool {
    value.string_parameters() == Some(leaf)
        && !(leaf.is_fixed()
            && value
                .as_string()
                .is_some_and(|text| text.as_str().ends_with('\0')))
}

/// Check one string against the maximum its column declares.
///
/// The bound counts stored bytes, which `Charset::encoded_len` answers
/// without building them. Whether those bytes can be written at all belongs
/// to the write seam, not here: a value read back through
/// `Charset::transcribe` carries scalars the charset does not assign - that
/// is what recovering damage means - and refusing it at the value door would
/// make the permissive read useless.
///
/// A fixed width and the US-ASCII repertoire are checked where the value is
/// validated against its column, before it reaches here.
fn check_string_bound(parameters: StringType, text: &str) -> Result<()> {
    let Some(max) = parameters.max() else {
        return Ok(());
    };
    let charset = parameters.charset();
    let stored = charset.encoded_len(text);
    if stored > max as usize {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: crate::text::expected_got(
                format_args!("at most {max} bytes of {charset}"),
                format_smolstr!("{stored}"),
            ),
        });
    }
    Ok(())
}

fn canonicalization_failure<T>(dtype: &DataType) -> Result<T> {
    Err(Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: format_smolstr!(
            "validated {} value could not be canonicalized",
            dtype.name()
        ),
    })
}

fn prepend_canonical_error(
    error: Error,
    segments: impl IntoIterator<Item = FieldSegment>,
) -> Error {
    let (path, reason) = match error {
        Error::InvalidRecord { path, reason } => (path, reason),
        other => (SmolStr::new_static("$"), format_smolstr!("{other}")),
    };
    let mut prefixed = String::from("$");
    for segment in segments {
        segment.append_diagnostic(&mut prefixed);
    }
    prefixed.push_str(path.strip_prefix('$').unwrap_or(path.as_str()));
    Error::InvalidRecord {
        path: SmolStr::from(prefixed),
        reason,
    }
}

fn validation_error(root: &str, failure: ValidationFailure) -> Error {
    let mut path = root_path(root);
    for segment in failure.path {
        segment.append_diagnostic(&mut path);
    }
    Error::InvalidRecord {
        path: SmolStr::from(path),
        reason: failure.reason,
    }
}

fn validate_field_value(
    field: &Field,
    value: &Scalar,
) -> std::result::Result<(), ValidationFailure> {
    validate_field_value_at_depth(field, value, 0)
}

fn validate_field_value_at_depth(
    field: &Field,
    value: &Scalar,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    validate_field_payload_at_depth(field, value, depth)
        .map_err(|failure| failure.prepend(FieldSegment::field(field.name())))
}

fn validate_field_payload_at_depth(
    field: &Field,
    value: &Scalar,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    if depth >= DataType::PARSE_RECURSION_LIMIT {
        return Err(ValidationFailure::new(format_smolstr!(
            "record nesting exceeds the hard limit of {}",
            DataType::PARSE_RECURSION_LIMIT
        )));
    }
    if value_is_logically_null(field.dtype(), value) && !field.is_nullable() {
        return Err(ValidationFailure::new("non-nullable field received null"));
    }
    if matches!(value, Scalar::Null)
        && !matches!(
            field.dtype(),
            DataType::Union(..) | DataType::RunEndEncoded(_)
        )
    {
        return Ok(());
    }
    validate_dtype_value(field.dtype(), value, depth)
}

fn validate_interval_value(
    value: &Scalar,
    unit: TimeUnit,
) -> std::result::Result<(), ValidationFailure> {
    if let Scalar::Interval(interval) = value {
        return require(
            interval.unit() == unit,
            match unit {
                TimeUnit::YearMonth => "interval year_month",
                TimeUnit::DayTime => "interval day_time",
                TimeUnit::MonthDayNano => "interval month_day_nano",
                _ => "valid interval layout",
            },
            value,
        );
    }
    match unit {
        TimeUnit::YearMonth => validate_signed(
            value,
            i128::from(i32::MIN),
            i128::from(i32::MAX),
            "interval year_month",
        ),
        TimeUnit::DayTime => validate_integer_tuple(value, &[32, 32], "interval day_time"),
        TimeUnit::MonthDayNano => {
            validate_integer_tuple(value, &[32, 32, 64], "interval month_day_nano")
        }
        _ => Err(ValidationFailure::new("invalid interval layout")),
    }
}

#[allow(clippy::too_many_lines)]
fn validate_dtype_value(
    dtype: &DataType,
    value: &Scalar,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    use DataType as D;
    if let Some(read) = read_as(dtype, value) {
        return match read {
            Ok(read) => validate_dtype_value(dtype, &read, depth),
            Err(error) => Err(ValidationFailure::new(reason_of(&error))),
        };
    }
    if let Some(physical) = restated(dtype, value) {
        return validate_dtype_value(dtype, &Scalar::from(physical), depth);
    }
    match dtype {
        D::Null => Err(expected("null", value)),
        D::Boolean => require(value.as_bool().is_some(), "boolean", value),
        D::Int8 => validate_signed(value, i128::from(i8::MIN), i128::from(i8::MAX), "int8"),
        D::Int16 => validate_signed(value, i128::from(i16::MIN), i128::from(i16::MAX), "int16"),
        D::Int32 => validate_signed(value, i128::from(i32::MIN), i128::from(i32::MAX), "int32"),
        D::Int64 => validate_signed(value, i128::from(i64::MIN), i128::from(i64::MAX), "int64"),
        D::UInt8 => validate_unsigned(value, u128::from(u8::MAX), "uint8"),
        D::UInt16 => validate_unsigned(value, u128::from(u16::MAX), "uint16"),
        D::UInt32 => validate_unsigned(value, u128::from(u32::MAX), "uint32"),
        D::UInt64 => validate_unsigned(value, u128::from(u64::MAX), "uint64"),
        D::Float16 | D::Float32 | D::Float64 => {
            require(value.as_f64().is_some(), dtype.name(), value)
        }
        D::DateTime64 { .. } | D::Duration64(_) => validate_signed(
            value,
            i128::from(i64::MIN),
            i128::from(i64::MAX),
            dtype.name(),
        ),
        D::Duration32(_) => validate_signed(
            value,
            i128::from(i32::MIN),
            i128::from(i32::MAX),
            dtype.name(),
        ),
        D::Date32 => validate_signed(
            value,
            i128::from(i32::MIN),
            i128::from(i32::MAX),
            dtype.name(),
        ),
        D::Date64 => validate_date64(value),
        leaf_dtype @ (D::Time32(_) | D::Time64(_)) => {
            let leaf = &leaf_dtype
                .time_type()
                .expect("the variant was just matched");
            validate_time(value, leaf.unit())
        }
        D::Interval(leaf) => validate_interval_value(value, *leaf),
        // Bytes are checked the way they are built, against the column's
        // leaf and never the one the value names: the number alone.
        crate::bytes_dtypes!() => match (dtype.bytes_parameters(), value.as_binary()) {
            (Some(leaf), Some(bytes)) => leaf
                .admit(bytes.clone())
                .map(|_| ())
                .map_err(|error| ValidationFailure::new(reason_of(&error))),
            _ => Err(expected(dtype.name(), value)),
        },
        // A string is checked the way it is built, against the column's leaf
        // and never the one the value names: the number, and the US-ASCII
        // repertoire where the column declares it.
        crate::string_dtypes!() => match (dtype.string_parameters(), value.as_string()) {
            (Some(leaf), Some(text)) => leaf
                .admit(text.clone())
                .map(|_| ())
                .map_err(|error| ValidationFailure::new(reason_of(&error))),
            _ => Err(expected(dtype.name(), value)),
        },
        D::Country
        | D::Ccy
        | D::MicCode
        | D::CfiCode
        | D::IsinCode
        | D::CusipCode
        | D::SedolCode
        | D::BloombergCode
        | D::FIGICode
        | D::Side
        | D::State
        | D::TimeInForce
        | D::Unit => match ascii_bytes(value) {
            // A side and a state are read by their spelling, which may be
            // longer than the value it names; the width holds the value.
            Some(bytes) if matches!(dtype, D::Side | D::State) => ascii_text_sized(None, bytes)
                .map(|_| ())
                .map_err(ascii_failure),
            Some(bytes) => code_cell_text(dtype, bytes)
                .map(|_| ())
                .map_err(ascii_failure),
            None => Err(expected(dtype.name(), value)),
        },
        D::Uuid => match value {
            Scalar::Uuid(_) => Ok(()),
            _ => match uuid_bytes(value).map(uuid_parse) {
                Some(Ok(_)) => Ok(()),
                _ => Err(expected("uuid", value)),
            },
        },
        D::Version => match value {
            Scalar::Version(_) => Ok(()),
            crate::string_scalars!(text) => text
                .as_str()
                .parse::<crate::Version>()
                .map(|_| ())
                .map_err(|_| expected("version", value)),
            _ => Err(expected("version", value)),
        },
        D::Url => match value {
            Scalar::Url(_) => Ok(()),
            crate::string_scalars!(text) => crate::Url::from_str(text.as_str())
                .map(|_| ())
                .map_err(|_| expected("url", value)),
            _ => Err(expected("url", value)),
        },
        D::Urn => match value {
            Scalar::Urn(_) => Ok(()),
            crate::string_scalars!(text) => crate::Urn::from_str(text.as_str())
                .map(|_| ())
                .map_err(|_| expected("urn", value)),
            _ => Err(expected("urn", value)),
        },
        D::Timezone => match value {
            Scalar::Timezone(_) => Ok(()),
            crate::string_scalars!(text) => crate::Timezone::from_str(text.as_str())
                .map(|_| ())
                .map_err(|_| expected("timezone", value)),
            _ => Err(expected("timezone", value)),
        },
        D::MimeType => match value {
            Scalar::MimeType(_) => Ok(()),
            crate::string_scalars!(text) => crate::MimeType::from_str(text.as_str())
                .map(|_| ())
                .map_err(|_| expected("mimetype", value)),
            _ => Err(expected("mimetype", value)),
        },
        D::MediaType => match value {
            Scalar::MediaType(_) => Ok(()),
            crate::string_scalars!(text) => crate::MediaType::from_str(text.as_str())
                .map(|_| ())
                .map_err(|_| expected("mediatype", value)),
            _ => Err(expected("mediatype", value)),
        },
        D::Serie(field) | D::SerieView(field) | D::LargeSerie(field) | D::LargeSerieView(field) => {
            validate_sequence(field, value, None, dtype.name(), depth + 1)
        }
        D::FixedSizeSerie(field, size) => validate_sequence(
            field,
            value,
            usize::try_from(*size).ok(),
            dtype.name(),
            depth + 1,
        ),
        D::Struct(fields) => validate_struct(fields, value, depth + 1),
        D::Union(fields, _) => validate_union(fields, value, depth + 1),
        D::Dictionary(dictionary) => validate_dtype_value(dictionary.value(), value, depth + 1),
        D::Decimal32 { precision, .. } => validate_decimal_value(value, *precision, 32),
        D::Decimal64 { precision, .. } => validate_decimal_value(value, *precision, 64),
        D::Decimal128 { precision, .. } => validate_decimal_value(value, *precision, 128),
        D::Decimal256 { precision, scale } => validate_decimal256_value(value, *precision, *scale),
        map_dtype @ (D::Map(_) | D::SortedMap(_)) => {
            let map = &map_dtype
                .as_mapping()
                .expect("the variant was just matched");
            validate_map(map, value, depth + 1)
        }
        D::RunEndEncoded(encoded) => {
            validate_field_value_at_depth(encoded.values(), value, depth + 1)
        }
        // A variant column validates any value, null included: the variant
        // null is a *value* the encoding can spell, so a required variant
        // holding `Null` is present, not absent.
        D::Variant => Ok(()),
        // A geospatial value is Well-Known Binary, and the payload's own
        // framing is the validation: a buffer the WKB reader refuses is not a
        // geometry, whatever bytes it carries.
        D::Geometry(_) | D::Geography(_) => match value.as_wkb() {
            Some(bytes) => crate::wkb::Geometry::from_slice(bytes)
                .map(|_| ())
                .map_err(|error| expected_because(dtype.name(), value, &error)),
            None => Err(expected(dtype.name(), value)),
        },
    }
}

fn validate_sequence(
    field: &Field,
    value: &Scalar,
    expected_len: Option<usize>,
    expected_name: &str,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    let serie = value
        .as_serie()
        .ok_or_else(|| expected(expected_name, value))?;
    if let Some(expected_len) = expected_len {
        if serie.len() != expected_len {
            return Err(ValidationFailure::new(format_smolstr!(
                "{expected_name} requires {expected_len} items, got {}",
                serie.len()
            )));
        }
    }
    // A column of the exact item field holds only rows it accepts - its door
    // proved them - so no row is read.
    if column_fits_item(serie, field) {
        return Ok(());
    }
    for (index, value) in serie.iter().enumerate() {
        validate_field_value_at_depth(field, &value, depth).map_err(|failure| {
            failure.prepend(FieldSegment::index(
                i64::try_from(index).expect("allocated index fits i64"),
            ))
        })?;
    }
    Ok(())
}

fn validate_struct(
    fields: &StructType,
    value: &Scalar,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    if let Some(record) = value.as_struct() {
        return validate_record_fields(fields.as_fields(), record, depth);
    }
    let values = value
        .sequence_rows()
        .ok_or_else(|| expected("struct sequence", value))?;
    if values.len() != fields.len() {
        return Err(ValidationFailure::new(format_smolstr!(
            "struct requires {} fields, got {} values",
            fields.len(),
            values.len()
        )));
    }
    fields
        .iter()
        .zip(values.iter())
        .try_for_each(|(field, value)| validate_field_value_at_depth(field, value, depth))
}

fn validate_record_fields(
    fields: &[Field],
    record: &std::collections::BTreeMap<SmolStr, Scalar>,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    if let Some(name) = record
        .keys()
        .find(|name| !fields.iter().any(|field| field.name() == name.as_str()))
    {
        return Err(ValidationFailure::new(format_smolstr!(
            "record contains unknown field {name:?}"
        )));
    }
    for field in fields.iter() {
        if let Some(value) = record.get(field.name()) {
            validate_field_value_at_depth(field, value, depth)?;
        } else {
            let default = field
                .default_value()
                .map_err(|error| ValidationFailure::new(error.to_string()))?;
            validate_field_value_at_depth(field, &default, depth)?;
        }
    }
    Ok(())
}

fn validate_union(
    fields: &crate::UnionFields,
    value: &Scalar,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    let values = value
        .sequence_rows()
        .ok_or_else(|| expected("union [type_id, payload] sequence", value))?;
    let [type_id, payload] = &*values else {
        return Err(ValidationFailure::new(
            "union value must contain exactly [type_id, payload]",
        ));
    };
    let type_id = type_id
        .as_i128()
        .and_then(|value| i8::try_from(value).ok())
        .ok_or_else(|| ValidationFailure::new("union type_id must fit in signed int8"))?;
    let (_, field) = fields
        .iter()
        .find(|(candidate, _)| *candidate == type_id)
        .ok_or_else(|| {
            ValidationFailure::new(format_smolstr!("unknown union type id {type_id}"))
        })?;
    validate_field_value_at_depth(field, payload, depth).map_err(|failure| {
        failure.prepend_segments([
            FieldSegment::field("union"),
            FieldSegment::index(i64::from(type_id)),
        ])
    })
}

/// The reason an ASCII refusal carries; the walk re-roots its path.
fn ascii_failure(error: Error) -> ValidationFailure {
    ValidationFailure::new(match error {
        Error::InvalidRecord { reason, .. } => reason,
        other => format_smolstr!("{other}"),
    })
}

/// Report a value whose kind does not match what the schema declared.
pub(crate) fn expected(expected_name: &str, value: &Scalar) -> ValidationFailure {
    ValidationFailure::new(crate::text::expected_got(expected_name, value.kind()))
}

/// [`expected`], carrying the refusal that says why the payload failed.
fn expected_because(
    expected_name: &str,
    value: &Scalar,
    because: &crate::Error,
) -> ValidationFailure {
    ValidationFailure::new(format_smolstr!(
        "expected {expected_name}, got {}: {because}",
        value.kind()
    ))
}

/// Accept a value when `matched`, and otherwise report what was expected.
fn require(
    matched: bool,
    expected_name: &str,
    value: &Scalar,
) -> std::result::Result<(), ValidationFailure> {
    if matched {
        Ok(())
    } else {
        Err(expected(expected_name, value))
    }
}

/// Validate one map value against its key and value fields.
///
/// A map is an ordered mapping whose entries struct declares exactly a key and
/// a value field; every entry is validated against those two.
fn validate_map(
    map: &crate::MappingType,
    value: &Scalar,
    depth: usize,
) -> std::result::Result<(), ValidationFailure> {
    let entries = value
        .as_mapping()
        .ok_or_else(|| expected("map entries", value))?;
    let Some([key_field, value_field]) = map.entries().dtype().as_fields() else {
        return Err(ValidationFailure::new(
            "map entries must declare exactly a key and a value field",
        ));
    };
    for (index, (key, entry_value)) in entries.iter().enumerate() {
        validate_field_value_at_depth(key_field, key, depth).map_err(|failure| {
            failure.prepend_segments([
                FieldSegment::index(i64::try_from(index).expect("allocated index fits i64")),
                FieldSegment::field("key"),
            ])
        })?;
        validate_field_value_at_depth(value_field, entry_value, depth).map_err(|failure| {
            failure.prepend_segments([
                FieldSegment::index(i64::try_from(index).expect("allocated index fits i64")),
                FieldSegment::field("value"),
            ])
        })?;
    }
    match broken_map_invariant(map, entries) {
        Some((index, reason)) => Err(ValidationFailure::new(reason).prepend_segments([
            FieldSegment::index(i64::try_from(index).expect("allocated index fits i64")),
            FieldSegment::field("key"),
        ])),
        None => Ok(()),
    }
}
