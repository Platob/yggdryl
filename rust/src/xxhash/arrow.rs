//! Per-row and per-column digest arrays over Arrow data.
//!
//! A digest column is the dedup key, the change-detection column, and the
//! hash-join key a table actually wants, and it is one pass over the buffers
//! rather than one materialized value per cell.
//!
//! The answer is defined by the value model, not by the layout: a row digest
//! feeds its metadata-selected values as one ordered [`Scalar`]
//! sequence, and a column digest feeds that cell's value. Where the layout
//! allows it the bytes are read straight from the Arrow buffer into the same
//! encoding; everything else falls back to the shared scalar boundary, so the
//! path stays exhaustive over every datatype family.

use std::collections::HashSet;
use std::hash::Hasher;
use std::sync::Arc;

use arrow_array::cast::AsArray as _;
use arrow_array::types::{
    Date32Type, Date64Type, DurationMicrosecondType, DurationMillisecondType,
    DurationNanosecondType, DurationSecondType, Float16Type, Float32Type, Float64Type, Int8Type,
    Int16Type, Int32Type, Int64Type, Time32MillisecondType, Time32SecondType,
    Time64MicrosecondType, Time64NanosecondType, TimestampMicrosecondType,
    TimestampMillisecondType, TimestampNanosecondType, TimestampSecondType, UInt8Type, UInt16Type,
    UInt32Type, UInt64Type,
};
use arrow_array::{
    Array, ArrayRef, BooleanArray, Decimal32Array, Decimal64Array, Decimal128Array,
    Decimal256Array, FixedSizeBinaryArray, RecordBatch, RecordBatchOptions, StructArray,
    UInt32Array, UInt64Array,
};
use arrow_buffer::NullBuffer;
use arrow_select::zip::zip;

use crate::TemporalFamily;
use crate::arrow::{Error, Result};
use crate::xxhash::{Xxh3, Xxh32, Xxh64, Xxh128};
use crate::{
    ArrowCast, DataType, Digest, DigestAlgorithm, Digester, Field, I256, Scalar, TimeUnit, Timezone,
};

use super::field::{
    DIGEST_ALGORITHM_KEY, DIGEST_PATHS_KEY, expected_holder_dtypes, has_explicit_components,
    holder_accepts, is_effective_component,
};
use super::scalar::{
    write_binary, write_bool, write_decimal, write_float, write_null, write_sequence_header,
    write_signed, write_string, write_temporal, write_unsigned,
};

/// The state operations shared by the runtime dispatcher and concrete states.
///
/// This stays private to the implementation: the public surface remains the
/// inherent `fill_arrow_batch` method on each state.
pub(crate) trait ArrowDigestState: Clone + Hasher {
    fn algorithm(&self) -> DigestAlgorithm;
    fn reset(&mut self);
    fn answer(&self) -> Digest;
}

macro_rules! concrete_state {
    ($state:ty, $algorithm:expr) => {
        impl ArrowDigestState for $state {
            fn algorithm(&self) -> DigestAlgorithm {
                $algorithm
            }

            fn reset(&mut self) {
                self.clear();
            }

            fn answer(&self) -> Digest {
                self.as_digest()
            }
        }
    };
}

concrete_state!(Xxh32, DigestAlgorithm::Xxh32);
concrete_state!(Xxh64, DigestAlgorithm::Xxh64);
concrete_state!(Xxh3, DigestAlgorithm::Xxh3);
concrete_state!(Xxh128, DigestAlgorithm::Xxh128);

impl ArrowDigestState for Digester {
    fn algorithm(&self) -> DigestAlgorithm {
        self.algorithm()
    }

    fn reset(&mut self) {
        self.clear();
    }

    fn answer(&self) -> Digest {
        self.as_digest()
    }
}

// The runtime state is intentionally inline: boxing the larger dispatcher
// would add one heap allocation per holder on the batch-fill hot path.
#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
enum FillState<S> {
    Prototype(S),
    Unseeded(Digester),
}

impl<S: ArrowDigestState> Hasher for FillState<S> {
    fn finish(&self) -> u64 {
        match self {
            Self::Prototype(state) => state.finish(),
            Self::Unseeded(state) => state.finish(),
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        match self {
            Self::Prototype(state) => state.write(bytes),
            Self::Unseeded(state) => state.write(bytes),
        }
    }
}

impl<S: ArrowDigestState> ArrowDigestState for FillState<S> {
    fn algorithm(&self) -> DigestAlgorithm {
        match self {
            Self::Prototype(state) => state.algorithm(),
            Self::Unseeded(state) => state.algorithm(),
        }
    }

    fn reset(&mut self) {
        match self {
            Self::Prototype(state) => state.reset(),
            Self::Unseeded(state) => state.reset(),
        }
    }

    fn answer(&self) -> Digest {
        match self {
            Self::Prototype(state) => state.answer(),
            Self::Unseeded(state) => state.answer(),
        }
    }
}

/// Fill every digest holder declared by `root` in one Arrow batch.
///
/// The state is a configuration prototype: its seed and secret are retained,
/// but bytes already written to it are ignored and the state itself is never
/// changed. The source is first cast to the exact root schema. Nested Struct
/// holders are filled bottom-up, then each containing holder streams one row
/// through the canonical scalar feed. Unless `force` is set, only holder cells
/// equal to that Field's canonical default are replaced.
pub(crate) fn fill_arrow_batch_with<S: ArrowDigestState>(
    prototype: &S,
    root: &Field,
    batch: RecordBatch,
    force: bool,
) -> Result<RecordBatch> {
    let batch = root.cast_arrow_batch(batch, true)?;
    let plan = StructPlan::new(root.fields(), prototype.algorithm(), "$")?;
    let row_count = batch.num_rows();
    let (columns, changed) =
        fill_struct(prototype, &plan, batch.columns(), None, force, row_count)?;
    if !changed {
        return Ok(batch);
    }
    let options = RecordBatchOptions::new().with_row_count(Some(row_count));
    RecordBatch::try_new_with_options(batch.schema(), columns, &options).map_err(Into::into)
}

/// A complete immutable fill plan for one Struct node.
struct StructPlan<'field> {
    fields: &'field [Field],
    nested: Vec<(usize, StructPlan<'field>)>,
    holders: Vec<HolderPlan<'field>>,
}

struct HolderPlan<'field> {
    index: usize,
    field: &'field Field,
    algorithm: DigestAlgorithm,
    use_prototype: bool,
    default: Scalar,
    selected: Vec<Selection<'field>>,
}

struct Selection<'field> {
    steps: Vec<usize>,
    field: &'field Field,
}

impl<'field> StructPlan<'field> {
    fn new(fields: &'field [Field], algorithm: DigestAlgorithm, path: &str) -> Result<Self> {
        let nested = fields
            .iter()
            .enumerate()
            .filter(|(_, field)| field.is_struct())
            .map(|(index, field)| {
                let path = child_path(path, field.name());
                Self::new(field.fields(), algorithm, &path).map(|plan| (index, plan))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut holders = Vec::new();
        for (index, field) in fields.iter().enumerate() {
            let field_path = child_path(path, field.name());
            let paths = field.as_digest().paths().map_err(|error| {
                digest_paths_error(&field_path, format!("cannot read stored paths: {error}"))
            })?;
            let declared_algorithm = field.as_digest().algorithm().map_err(|error| {
                digest_algorithm_error(
                    &field_path,
                    format!("cannot read stored algorithm: {error}"),
                )
            })?;
            if !field.as_digest().is_holder() {
                if paths.is_some() {
                    return Err(digest_paths_error(
                        &field_path,
                        "digest:paths belongs only to a digest holder",
                    ));
                }
                if declared_algorithm.is_some() {
                    return Err(digest_algorithm_error(
                        &field_path,
                        "digest:algorithm belongs only to a digest holder",
                    ));
                }
                continue;
            }
            let holder_algorithm =
                resolve_holder_algorithm(field, declared_algorithm, algorithm, &field_path)?;
            let selected = match paths {
                Some(paths) => paths
                    .iter()
                    .map(|path| {
                        let selection = resolve_selection(fields, path, &field_path)?;
                        if selection
                            .steps
                            .first()
                            .is_some_and(|selected| fields[*selected].as_digest().is_holder())
                        {
                            return Err(digest_paths_error(
                                &field_path,
                                format!(
                                    "path {path:?} selects same-Struct digest holder {:?}; holders are outputs, not peer components",
                                    fields[selection.steps[0]].name()
                                ),
                            ));
                        }
                        shortcut_struct_holder(selection, path, Some(&field_path))
                    })
                    .collect::<Result<Vec<_>>>()?,
                None => {
                    let explicit = has_explicit_components(fields);
                    fields
                        .iter()
                        .enumerate()
                        .filter(|(_, candidate)| is_effective_component(candidate, explicit))
                        .map(|(selected, candidate)| {
                            shortcut_struct_holder(
                                Selection {
                                    steps: vec![selected],
                                    field: candidate,
                                },
                                candidate.name(),
                                None,
                            )
                        })
                        .collect::<Result<Vec<_>>>()?
                }
            };
            let mut targets = HashSet::with_capacity(selected.len());
            for selection in &selected {
                if !targets.insert(selection.steps.clone()) {
                    return Err(digest_paths_error(
                        &field_path,
                        "multiple digest paths resolve to the same selected value",
                    ));
                }
            }
            holders.push(HolderPlan {
                index,
                field,
                algorithm: holder_algorithm,
                use_prototype: holder_algorithm == algorithm,
                default: field.default_value().map_err(Error::from)?,
                selected,
            });
        }
        Ok(Self {
            fields,
            nested,
            holders,
        })
    }
}

fn child_path(parent: &str, name: &str) -> String {
    format!("{parent}.{name}")
}

fn digest_paths_error(holder: &str, reason: impl std::fmt::Display) -> Error {
    digest_metadata_error(DIGEST_PATHS_KEY, holder, reason)
}

fn digest_algorithm_error(holder: &str, reason: impl std::fmt::Display) -> Error {
    digest_metadata_error(DIGEST_ALGORITHM_KEY, holder, reason)
}

fn digest_metadata_error(key: &'static str, holder: &str, reason: impl std::fmt::Display) -> Error {
    Error::Core(crate::Error::InvalidMetadataValue {
        key: smol_str::SmolStr::new_static(key),
        reason: smol_str::format_smolstr!("holder {holder}: {reason}"),
    })
}

fn default_holder_algorithm(field: &Field) -> Option<DigestAlgorithm> {
    match field.dtype() {
        DataType::Int32 | DataType::UInt32 => Some(DigestAlgorithm::Xxh32),
        DataType::Int64 | DataType::UInt64 => Some(DigestAlgorithm::Xxh3),
        DataType::FixedSizeBinary(16) => Some(DigestAlgorithm::Xxh128),
        _ => None,
    }
}

fn resolve_holder_algorithm(
    field: &Field,
    declared: Option<DigestAlgorithm>,
    prototype: DigestAlgorithm,
    holder_path: &str,
) -> Result<DigestAlgorithm> {
    if let Some(algorithm) = declared {
        if !holder_accepts(field, algorithm) {
            return Err(digest_algorithm_error(
                holder_path,
                format!(
                    "algorithm {algorithm} requires {}, got {}",
                    expected_holder_dtypes(algorithm),
                    field.dtype()
                ),
            ));
        }
        return Ok(algorithm);
    }
    if holder_accepts(field, prototype) {
        return Ok(prototype);
    }
    default_holder_algorithm(field).ok_or_else(|| {
        Error::IncompatibleSchema(format!(
            "digest holder {holder_path} must be int32, uint32, int64, uint64, or fixed_size_binary[16], got {}",
            field.dtype()
        ))
    })
}

/// Resolve an exact-name-first path through Struct children only.
fn resolve_selection<'field>(
    fields: &'field [Field],
    path: &str,
    holder: &str,
) -> Result<Selection<'field>> {
    if let Some((index, field)) = fields
        .iter()
        .enumerate()
        .find(|(_, field)| field.name() == path)
    {
        return Ok(Selection {
            steps: vec![index],
            field,
        });
    }
    let mut offset = 0;
    let mut blocked = None;
    while let Some(relative) = path[offset..].find('.') {
        let boundary = offset + relative;
        if let Some((index, field)) = fields
            .iter()
            .enumerate()
            .find(|(_, field)| field.name() == &path[..boundary])
        {
            if !field.is_struct() {
                blocked = Some(format!(
                    "path {path:?} cannot descend through non-Struct field {:?} of {}",
                    field.name(),
                    field.dtype()
                ));
                offset = boundary + 1;
                continue;
            }
            if let Ok(mut tail) = resolve_selection(field.fields(), &path[boundary + 1..], holder) {
                tail.steps.insert(0, index);
                return Ok(tail);
            }
        }
        offset = boundary + 1;
    }
    Err(digest_paths_error(
        holder,
        blocked.unwrap_or_else(|| format!("path {path:?} does not name a field")),
    ))
}

/// A selected Struct carrying one direct holder contributes that holder.
fn shortcut_struct_holder<'field>(
    mut selection: Selection<'field>,
    path: &str,
    holder_path: Option<&str>,
) -> Result<Selection<'field>> {
    if !selection.field.is_struct() {
        return Ok(selection);
    }
    let mut holders = selection
        .field
        .fields()
        .iter()
        .enumerate()
        .filter(|(_, field)| field.as_digest().is_holder());
    let Some((index, holder)) = holders.next() else {
        return Ok(selection);
    };
    if holders.next().is_some() {
        let reason = format!(
            "path {path:?} selects Struct field {:?} with multiple direct digest holders",
            selection.field.name()
        );
        return Err(match holder_path {
            Some(holder) => digest_paths_error(holder, reason),
            None => Error::IncompatibleSchema(reason),
        });
    }
    selection.steps.push(index);
    selection.field = holder;
    Ok(selection)
}

fn fill_struct<S: ArrowDigestState>(
    prototype: &S,
    plan: &StructPlan<'_>,
    source: &[ArrayRef],
    parent_nulls: Option<&NullBuffer>,
    force: bool,
    row_count: usize,
) -> Result<(Vec<ArrayRef>, bool)> {
    let mut columns = source.to_vec();
    let mut changed = false;

    // Descendants must be final before a containing holder reads them.
    for (index, nested_plan) in &plan.nested {
        let nested = downcast::<StructArray>(columns[*index].as_ref())?;
        let hidden = NullBuffer::union(parent_nulls, nested.nulls());
        let (children, child_changed) = fill_struct(
            prototype,
            nested_plan,
            nested.columns(),
            hidden.as_ref(),
            force,
            row_count,
        )?;
        if child_changed {
            let fields = match nested.data_type() {
                arrow_schema::DataType::Struct(fields) => fields.clone(),
                _ => {
                    return Err(Error::IncompatibleSchema(format!(
                        "field {:?} was planned as Struct but stores {}",
                        plan.fields[*index].name(),
                        nested.data_type()
                    )));
                }
            };
            columns[*index] = Arc::new(StructArray::try_new_with_length(
                fields,
                children,
                nested.nulls().cloned(),
                row_count,
            )?);
            changed = true;
        }
    }

    for holder in &plan.holders {
        let original = Arc::clone(&columns[holder.index]);
        let mut mask = Vec::with_capacity(row_count);
        let mut values = Vec::with_capacity(row_count);
        let mut worker = if holder.use_prototype {
            FillState::Prototype(prototype.clone())
        } else {
            FillState::Unseeded(holder.algorithm.digester())
        };
        for row in 0..row_count {
            let visible = parent_nulls.is_none_or(|nulls| nulls.is_valid(row));
            let recompute = if !visible {
                false
            } else if force {
                true
            } else {
                crate::arrow::value::value_from_array(holder.field.dtype(), original.as_ref(), row)?
                    == holder.default
            };
            mask.push(recompute);
            worker.reset();
            if recompute {
                write_sequence_header(&mut worker, holder.selected.len());
                for selected in &holder.selected {
                    feed_selection(&mut worker, &columns, plan.fields, selected, row)?;
                }
            }
            values.push(worker.answer());
        }
        if !mask.iter().any(|selected| *selected) {
            continue;
        }
        let computed = collect(&values, holder.algorithm);
        let computed = if matches!(holder.field.dtype(), DataType::Int32 | DataType::Int64) {
            holder.field.cast_arrow_array_bits(computed)?
        } else {
            computed
        };
        let mask = BooleanArray::from(mask);
        columns[holder.index] = zip(&mask, &computed.as_ref(), &original.as_ref())?;
        changed = true;
    }
    Ok((columns, changed))
}

fn feed_selection(
    digester: &mut impl Hasher,
    root_arrays: &[ArrayRef],
    root_fields: &[Field],
    selected: &Selection<'_>,
    row: usize,
) -> Result<()> {
    let mut arrays = root_arrays;
    let mut fields = root_fields;
    for (depth, index) in selected.steps.iter().copied().enumerate() {
        let field = &fields[index];
        let array = arrays[index].as_ref();
        if depth + 1 == selected.steps.len() {
            return feed_selected_cell(digester, selected.field, array, row);
        }
        if array.is_null(row) {
            write_null(digester);
            return Ok(());
        }
        let nested = downcast::<StructArray>(array)?;
        arrays = nested.columns();
        fields = field.fields();
    }
    Err(Error::IncompatibleSchema(
        "a digest selection cannot have an empty path".to_owned(),
    ))
}

/// Feed a selected holder by its unsigned digest payload, independent of the
/// signed or unsigned same-width Arrow storage chosen for that payload.
fn feed_selected_cell(
    digester: &mut impl Hasher,
    field: &Field,
    array: &dyn Array,
    index: usize,
) -> Result<()> {
    if field.as_digest().is_holder() && !array.is_null(index) {
        match field.dtype() {
            DataType::Int32 => {
                let value = array.as_primitive::<Int32Type>().value(index);
                write_unsigned(
                    digester,
                    u128::from(u32::from_ne_bytes(value.to_ne_bytes())),
                );
                return Ok(());
            }
            DataType::Int64 => {
                let value = array.as_primitive::<Int64Type>().value(index);
                write_unsigned(
                    digester,
                    u128::from(u64::from_ne_bytes(value.to_ne_bytes())),
                );
                return Ok(());
            }
            _ => {}
        }
    }
    feed_cell(digester, field.dtype(), array, index)
}

/// Digest every row of a batch, in selected schema order.
///
/// Explicit `digest:role=component` fields form the input. When none are
/// explicit, every field except one carrying `digest:role=holder` contributes.
/// The selected values remain an ordered sequence, which is the canonical row
/// shape everywhere in this project.
///
/// The result is a `UInt32Array` for XXH32, a `UInt64Array` for the two
/// 64-bit algorithms, and a `FixedSizeBinary(16)` of canonical big-endian
/// bytes for XXH3-128, which has no native Arrow integer wide enough.
///
/// ```
/// use arrow_array::{Int64Array, RecordBatch, StringArray, UInt64Array};
/// use arrow_array::cast::AsArray as _;
/// use arrow_schema::{DataType, Field, Schema};
/// use std::sync::Arc;
///
/// use yggdryl::DigestAlgorithm;
/// use yggdryl::xxhash::arrow::row_digests;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let batch = RecordBatch::try_new(
///     Arc::new(Schema::new(vec![
///         Field::new("symbol", DataType::Utf8, false),
///         Field::new("quantity", DataType::Int64, false),
///     ])),
///     vec![
///         Arc::new(StringArray::from(vec!["AAPL", "MSFT", "AAPL"])),
///         Arc::new(Int64Array::from(vec![100, 250, 100])),
///     ],
/// )?;
///
/// let digests = row_digests(&batch, DigestAlgorithm::Xxh3)?;
/// let digests = digests.as_primitive::<arrow_array::types::UInt64Type>();
/// // Identical rows answer identical digests, which is what makes this a
/// // dedup key.
/// assert_eq!(digests.value(0), digests.value(2));
/// assert_ne!(digests.value(0), digests.value(1));
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error when a column's schema does not project to the core
/// datatype model, or a value cannot be represented.
pub fn row_digests(batch: &RecordBatch, algorithm: DigestAlgorithm) -> Result<ArrayRef> {
    let fields: Vec<Field> = batch
        .schema()
        .fields()
        .iter()
        .map(|field| Field::from_arrow_ref(Arc::clone(field)).map_err(Error::from))
        .collect::<Result<_>>()?;
    let columns = batch.columns();
    let explicit = has_explicit_components(&fields);
    let selected: Vec<usize> = fields
        .iter()
        .enumerate()
        .filter_map(|(index, field)| is_effective_component(field, explicit).then_some(index))
        .collect();
    let mut digests = Vec::with_capacity(batch.num_rows());
    let mut digester = algorithm.digester();
    for index in 0..batch.num_rows() {
        digester.clear();
        write_sequence_header(&mut digester, selected.len());
        for &column in &selected {
            feed_cell(
                &mut digester,
                fields[column].dtype(),
                columns[column].as_ref(),
                index,
            )?;
        }
        digests.push(digester.as_digest());
    }
    Ok(collect(&digests, algorithm))
}

/// Digest every value of one column.
///
/// This is the single-column form [`row_digests`] composes: each answer is the
/// cell's own value fed through
/// [`Scalar::write_bytes`](crate::Scalar::write_bytes), with no row framing
/// around it. A null feeds the null tag, so a null and an empty string never
/// collide.
///
/// # Errors
///
/// Returns an error when `field` does not describe `array`, or a value cannot
/// be represented.
pub fn column_digests(
    array: &dyn Array,
    field: &Field,
    algorithm: DigestAlgorithm,
) -> Result<ArrayRef> {
    let mut digests = Vec::with_capacity(array.len());
    let mut digester = algorithm.digester();
    for index in 0..array.len() {
        digester.clear();
        feed_cell(&mut digester, field.dtype(), array, index)?;
        digests.push(digester.as_digest());
    }
    Ok(collect(&digests, algorithm))
}

/// Build the digest column the algorithm's width calls for.
fn collect(digests: &[Digest], algorithm: DigestAlgorithm) -> ArrayRef {
    match algorithm {
        DigestAlgorithm::Xxh32 => Arc::new(UInt32Array::from_iter_values(
            digests.iter().filter_map(|digest| digest.as_u32()),
        )),
        DigestAlgorithm::Xxh64 | DigestAlgorithm::Xxh3 => Arc::new(UInt64Array::from_iter_values(
            digests.iter().filter_map(|digest| digest.as_u64()),
        )),
        DigestAlgorithm::Xxh128 => {
            // The canonical big-endian bytes, because no Arrow integer is 128
            // bits wide and a pair of `u64` columns would put the wire order
            // in the caller's hands.
            let bytes: Vec<[u8; 16]> = digests
                .iter()
                .map(|digest| {
                    let mut wide = [0_u8; 16];
                    wide.copy_from_slice(&digest.into_bytes());
                    wide
                })
                .collect();
            // Built from the flat buffer rather than an iterator, because an
            // empty batch carries no element for a width to be inferred from
            // and the column still has to be `FixedSizeBinary(16)`.
            let flat: Vec<u8> = bytes.concat();
            Arc::new(FixedSizeBinaryArray::new(
                16,
                arrow_buffer::Buffer::from_vec(flat),
                None,
            ))
        }
    }
}

/// Feed one cell's canonical bytes, reading the buffer where the layout allows.
///
/// The fallback is the shared scalar boundary, so every datatype family the
/// core can read is covered; the buffer arms exist only to skip materializing
/// a value whose bytes are already sitting in the column.
fn feed_cell(
    digester: &mut impl Hasher,
    dtype: &DataType,
    array: &dyn Array,
    index: usize,
) -> Result<()> {
    // A union or a run-end encoding hides its own validity, so absence there
    // is the child's answer rather than the parent's, exactly as the scalar
    // boundary reads it.
    if array.is_null(index) && !matches!(dtype, DataType::Union(..) | DataType::RunEndEncoded(_)) {
        write_null(digester);
        return Ok(());
    }
    match dtype {
        DataType::Null => write_null(digester),
        DataType::Boolean => write_bool(digester, array.as_boolean().value(index)),
        DataType::Int8 => write_signed(
            digester,
            i128::from(array.as_primitive::<Int8Type>().value(index)),
        ),
        DataType::Int16 => {
            write_signed(
                digester,
                i128::from(array.as_primitive::<Int16Type>().value(index)),
            );
        }
        DataType::Int32 => {
            write_signed(
                digester,
                i128::from(array.as_primitive::<Int32Type>().value(index)),
            );
        }
        DataType::Int64 => {
            write_signed(
                digester,
                i128::from(array.as_primitive::<Int64Type>().value(index)),
            );
        }
        DataType::UInt8 => {
            write_unsigned(
                digester,
                u128::from(array.as_primitive::<UInt8Type>().value(index)),
            );
        }
        DataType::UInt16 => {
            write_unsigned(
                digester,
                u128::from(array.as_primitive::<UInt16Type>().value(index)),
            );
        }
        DataType::UInt32 => {
            write_unsigned(
                digester,
                u128::from(array.as_primitive::<UInt32Type>().value(index)),
            );
        }
        DataType::UInt64 => {
            write_unsigned(
                digester,
                u128::from(array.as_primitive::<UInt64Type>().value(index)),
            );
        }
        DataType::Float16 => write_float(
            digester,
            f64::from(array.as_primitive::<Float16Type>().value(index).to_f32()),
        ),
        DataType::Float32 => write_float(
            digester,
            f64::from(array.as_primitive::<Float32Type>().value(index)),
        ),
        DataType::Float64 => {
            write_float(digester, array.as_primitive::<Float64Type>().value(index))
        }
        DataType::Utf8 => write_string(digester, array.as_string::<i32>().value(index)),
        DataType::LargeUtf8 => write_string(digester, array.as_string::<i64>().value(index)),
        DataType::Utf8View => write_string(digester, array.as_string_view().value(index)),
        DataType::Binary => write_binary(digester, array.as_binary::<i32>().value(index)),
        DataType::LargeBinary => write_binary(digester, array.as_binary::<i64>().value(index)),
        DataType::BinaryView => write_binary(digester, array.as_binary_view().value(index)),
        DataType::FixedSizeBinary(_) => write_binary(
            digester,
            downcast::<FixedSizeBinaryArray>(array)?.value(index),
        ),
        DataType::Decimal32 { scale, .. } => write_decimal(
            digester,
            I256::from_i128(i128::from(downcast::<Decimal32Array>(array)?.value(index))),
            *scale,
        ),
        DataType::Decimal64 { scale, .. } => write_decimal(
            digester,
            I256::from_i128(i128::from(downcast::<Decimal64Array>(array)?.value(index))),
            *scale,
        ),
        DataType::Decimal128 { scale, .. } => write_decimal(
            digester,
            I256::from_i128(downcast::<Decimal128Array>(array)?.value(index)),
            *scale,
        ),
        DataType::Decimal256 { scale, .. } => write_decimal(
            digester,
            I256::from_le_bytes(
                downcast::<Decimal256Array>(array)?
                    .value(index)
                    .to_le_bytes(),
            ),
            *scale,
        ),
        DataType::Date32 => temporal(
            digester,
            TemporalFamily::Date,
            i64::from(array.as_primitive::<Date32Type>().value(index)),
            TimeUnit::Day,
            &Timezone::NAIVE,
        ),
        DataType::Date64 => temporal(
            digester,
            TemporalFamily::Date,
            array.as_primitive::<Date64Type>().value(index),
            TimeUnit::Millisecond,
            &Timezone::NAIVE,
        ),
        DataType::Time32(unit) => {
            let count = match unit {
                TimeUnit::Second => array.as_primitive::<Time32SecondType>().value(index),
                TimeUnit::Millisecond => array.as_primitive::<Time32MillisecondType>().value(index),
                _ => return fallback(digester, dtype, array, index),
            };
            temporal(
                digester,
                TemporalFamily::Time,
                i64::from(count),
                *unit,
                &Timezone::NAIVE,
            );
        }
        DataType::Time64(unit) => {
            let count = match unit {
                TimeUnit::Microsecond => array.as_primitive::<Time64MicrosecondType>().value(index),
                TimeUnit::Nanosecond => array.as_primitive::<Time64NanosecondType>().value(index),
                _ => return fallback(digester, dtype, array, index),
            };
            temporal(
                digester,
                TemporalFamily::Time,
                count,
                *unit,
                &Timezone::NAIVE,
            );
        }
        DataType::DateTime64 { unit, timezone } => {
            let count = match unit {
                TimeUnit::Second => array.as_primitive::<TimestampSecondType>().value(index),
                TimeUnit::Millisecond => array
                    .as_primitive::<TimestampMillisecondType>()
                    .value(index),
                TimeUnit::Microsecond => array
                    .as_primitive::<TimestampMicrosecondType>()
                    .value(index),
                TimeUnit::Nanosecond => {
                    array.as_primitive::<TimestampNanosecondType>().value(index)
                }
                _ => return fallback(digester, dtype, array, index),
            };
            temporal(digester, TemporalFamily::DateTime, count, *unit, timezone);
        }
        DataType::Duration64(unit) => {
            let count = match unit {
                TimeUnit::Second => array.as_primitive::<DurationSecondType>().value(index),
                TimeUnit::Millisecond => {
                    array.as_primitive::<DurationMillisecondType>().value(index)
                }
                TimeUnit::Microsecond => {
                    array.as_primitive::<DurationMicrosecondType>().value(index)
                }
                TimeUnit::Nanosecond => array.as_primitive::<DurationNanosecondType>().value(index),
                _ => return fallback(digester, dtype, array, index),
            };
            temporal(
                digester,
                TemporalFamily::Duration,
                count,
                *unit,
                &Timezone::NAIVE,
            );
        }
        // Ascii text is trimmed on the way out, geospatial carries its own
        // tag, and every nested, dictionary, union, run-end, and variant
        // layout composes values rather than holding one buffer, so all of
        // them read through the shared boundary.
        _ => return fallback(digester, dtype, array, index),
    }
    Ok(())
}

/// Feed a temporal read straight from a buffer.
fn temporal(
    digester: &mut impl Hasher,
    family: TemporalFamily,
    count: i64,
    unit: TimeUnit,
    zone: &Timezone,
) {
    write_temporal(digester, family, count, unit, zone);
}

/// Feed one cell through the shared scalar boundary.
fn fallback(
    digester: &mut impl Hasher,
    dtype: &DataType,
    array: &dyn Array,
    index: usize,
) -> Result<()> {
    let value = crate::arrow::value::value_from_array(dtype, array, index)?;
    value.write_bytes(digester);
    Ok(())
}

/// Downcast an array to the layout its datatype names.
fn downcast<T: 'static>(array: &dyn Array) -> Result<&T> {
    array.as_any().downcast_ref::<T>().ok_or_else(|| {
        Error::IncompatibleSchema(format!(
            "expected an Arrow {} array, got {}",
            std::any::type_name::<T>(),
            array.data_type()
        ))
    })
}

#[cfg(test)]
mod tests;
