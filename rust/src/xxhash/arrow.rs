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
//! path stays exhaustive over every datatype family. That exhaustiveness is
//! the compiler's: [`DataType`] is matched arm by arm, so a datatype added to
//! the model cannot reach a digest without a decision here. `variant` is the
//! one datatype a column refuses, because its binary encoding lands with the
//! Iceberg v3 layer and the boundary has no value to feed.

use std::collections::HashSet;
use std::hash::Hasher;
use std::sync::Arc;

use arrow_array::{
    Array, ArrayRef, BooleanArray, FixedSizeBinaryArray, RecordBatch, RecordBatchOptions,
    StructArray, UInt32Array, UInt64Array,
};
use arrow_buffer::NullBuffer;
use arrow_select::zip::zip;

use crate::arrow::{Error, Result};
use crate::cast::{ArrowCastOptions, ArrowCastPlan, Deferred, Nullability, Representation};
use crate::metadata::is_all_sources;
use crate::serie::{Proof, land, land_under};
use crate::xxhash::{Xxh3, Xxh32, Xxh64, Xxh128};
use crate::{DataType, Digest, DigestAlgorithm, Digester, Field, Scalar, TimeUnit};
use crate::{Serie, Str};

use super::field::{
    DIGEST_ALGORITHM_KEY, DIGEST_ROLE_KEY, DIGEST_SOURCES_KEY, expected_holder_dtypes,
    holder_accepts, is_digest_source,
};
use super::scalar::{
    write_binary, write_bool, write_float, write_null, write_sequence_header, write_signed,
    write_string, write_unsigned,
};
use crate::txhash::{DIGEST_TIME_KEY, DIGEST_UNIT_KEY};

/// The state operations shared by the runtime dispatcher and concrete states.
///
/// This stays private to the implementation: the public surface remains the
/// inherent `apply_arrow_batch` method on each state.
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
pub(crate) fn apply_arrow_batch_with<S: ArrowDigestState>(
    prototype: &S,
    root: &Field,
    batch: RecordBatch,
    force: bool,
) -> Result<RecordBatch> {
    let plan = StructPlan::compile(root, prototype.algorithm())?;
    let batch = crate::cast::ArrowCastPlan::compile_schema(
        batch.schema_ref(),
        root,
        ArrowCastOptions::new(),
        crate::cast::Deferred::default(),
    )?
    .reconcile_batch(batch)?;
    plan.fill_arrow_batch(prototype, root, batch, force)
}

/// A complete immutable fill plan for one Struct node.
///
/// Every declaration is read and every holder cast compiled here, from the
/// fields alone, so a stream holding the plan moves only rows per batch.
pub(crate) struct StructPlan {
    nested: Vec<(usize, StructPlan)>,
    holders: Vec<HolderPlan>,
}

struct HolderPlan {
    index: usize,
    path: String,
    algorithm: DigestAlgorithm,
    use_prototype: bool,
    default: Scalar,
    /// The child steps from this Struct to each value the holder reads.
    selected: Vec<Vec<usize>>,
    /// The instant a coupled holder stores in front of its digest.
    time: Option<TimePlan>,
    /// The cast a signed holder stores the unsigned digest's bits through.
    bits: Option<ArrowCastPlan>,
}

/// Where a coupled holder reads its instant, and the resolution it keeps.
struct TimePlan {
    steps: Vec<usize>,
    unit: TimeUnit,
}

struct Selection<'field> {
    steps: Vec<usize>,
    field: &'field Field,
}

impl StructPlan {
    /// Plans every holder `root` declares for a prototype of `algorithm`.
    pub(crate) fn compile(root: &Field, algorithm: DigestAlgorithm) -> Result<Self> {
        Self::new(root.fields(), algorithm, "$")
    }

    /// Fill the holders in one batch already cast to the `root` this plan was
    /// compiled from, under a prototype of the algorithm it was compiled for.
    pub(crate) fn fill_arrow_batch<S: ArrowDigestState>(
        &self,
        prototype: &S,
        root: &Field,
        batch: RecordBatch,
        force: bool,
    ) -> Result<RecordBatch> {
        let row_count = batch.num_rows();
        let (columns, changed) = fill_struct(
            prototype,
            self,
            root.fields(),
            batch.columns(),
            None,
            force,
            row_count,
        )?;
        if !changed {
            return Ok(batch);
        }
        let options = RecordBatchOptions::new().with_row_count(Some(row_count));
        RecordBatch::try_new_with_options(batch.schema(), columns, &options).map_err(Into::into)
    }

    fn new(fields: &[Field], algorithm: DigestAlgorithm, path: &str) -> Result<Self> {
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
            if !field.is_struct() {
                reject_unreachable_digests(field.dtype(), &field_path, &field_path)?;
            }
            let sources = field.as_digest().sources().map_err(|error| {
                digest_sources_error(&field_path, format!("cannot read stored sources: {error}"))
            })?;
            let declared_algorithm = field.as_digest().algorithm().map_err(|error| {
                digest_algorithm_error(
                    &field_path,
                    format!("cannot read stored algorithm: {error}"),
                )
            })?;
            let declared_unit = field.as_digest().unit().map_err(|error| {
                digest_metadata_error(
                    DIGEST_UNIT_KEY,
                    &field_path,
                    format!("cannot read stored unit: {error}"),
                )
            })?;
            if !field.as_digest().is_holder() {
                if sources.is_some() {
                    return Err(digest_sources_error(
                        &field_path,
                        "DIGEST:sources belongs only to a digest holder",
                    ));
                }
                if declared_algorithm.is_some() {
                    return Err(digest_algorithm_error(
                        &field_path,
                        "DIGEST:algorithm belongs only to a digest holder",
                    ));
                }
                if field.as_digest().time().is_some() {
                    return Err(digest_metadata_error(
                        DIGEST_TIME_KEY,
                        &field_path,
                        "DIGEST:time belongs only to a digest holder",
                    ));
                }
                if declared_unit.is_some() {
                    return Err(digest_metadata_error(
                        DIGEST_UNIT_KEY,
                        &field_path,
                        "DIGEST:unit belongs only to a digest holder",
                    ));
                }
                continue;
            }
            let holder_algorithm =
                resolve_holder_algorithm(field, declared_algorithm, algorithm, &field_path)?;
            // The instant is read through the same Struct-only descent a
            // source takes, and it must be a leaf an instant can be read from.
            let time = match field.as_digest().time() {
                Some(path) => {
                    let selection = resolve_selection(fields, path, &field_path, DIGEST_TIME_KEY)?;
                    if selection.field.as_digest().is_holder() {
                        return Err(digest_metadata_error(
                            DIGEST_TIME_KEY,
                            &field_path,
                            format!(
                                "time {path:?} selects digest holder {:?}; holders are outputs, not instants",
                                selection.field.name()
                            ),
                        ));
                    }
                    if !crate::txhash::arrow::accepts_time(selection.field.dtype()) {
                        return Err(digest_metadata_error(
                            DIGEST_TIME_KEY,
                            &field_path,
                            format!(
                                "time {path:?} must name a datetime, date, or integer field, got {}",
                                selection.field.dtype()
                            ),
                        ));
                    }
                    Some(TimePlan {
                        steps: selection.steps,
                        unit: field.as_digest().coupled_unit().map_err(|error| {
                            digest_metadata_error(
                                DIGEST_UNIT_KEY,
                                &field_path,
                                format!("cannot read stored unit: {error}"),
                            )
                        })?,
                    })
                }
                None => {
                    if declared_unit.is_some() {
                        return Err(digest_metadata_error(
                            DIGEST_UNIT_KEY,
                            &field_path,
                            "DIGEST:unit belongs only to a holder naming DIGEST:time",
                        ));
                    }
                    None
                }
            };
            // An absent list and the `["*"]` spelling are the same selection:
            // every field of this Struct the holder does not hold. Naming
            // sources on the holder is what keeps the fields it reads
            // unmarked.
            let named = sources.filter(|sources| !is_all_sources(sources));
            let selected = match named {
                Some(sources) => sources
                    .iter()
                    .map(|path| {
                        let selection =
                            resolve_selection(fields, path, &field_path, DIGEST_SOURCES_KEY)?;
                        if selection
                            .steps
                            .first()
                            .is_some_and(|selected| fields[*selected].as_digest().is_holder())
                        {
                            return Err(digest_sources_error(
                                &field_path,
                                format!(
                                    "source {path:?} selects same-Struct digest holder {:?}; holders are outputs, not sources",
                                    fields[selection.steps[0]].name()
                                ),
                            ));
                        }
                        shortcut_struct_holder(selection, path, Some(&field_path))
                    })
                    .collect::<Result<Vec<_>>>()?,
                None => fields
                    .iter()
                    .enumerate()
                    .filter(|(_, candidate)| is_digest_source(candidate))
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
                    .collect::<Result<Vec<_>>>()?,
            };
            let mut targets = HashSet::with_capacity(selected.len());
            for selection in &selected {
                if !targets.insert(selection.steps.clone()) {
                    return Err(digest_sources_error(
                        &field_path,
                        "multiple digest sources resolve to the same selected value",
                    ));
                }
            }
            let default = field.default_value().map_err(Error::from)?;
            // A signed holder stores the unsigned digest's bits, not a narrower
            // number: the same bytes under the width the schema declared.
            let bits = match field.dtype() {
                DataType::Int32 | DataType::Int64 => {
                    let digests = collect(&[], holder_algorithm, None);
                    Some(ArrowCastPlan::compile_arrow(
                        &Arc::new(arrow_schema::Field::new(
                            field.name(),
                            digests.data_type().clone(),
                            true,
                        )),
                        field,
                        ArrowCastOptions::new().with_representation(Representation::Bits),
                        Deferred::default(),
                    )?)
                }
                _ => None,
            };
            holders.push(HolderPlan {
                index,
                path: field_path,
                algorithm: holder_algorithm,
                use_prototype: holder_algorithm == algorithm,
                default,
                selected: selected
                    .into_iter()
                    .map(|selection| selection.steps)
                    .collect(),
                time,
                bits,
            });
        }
        Ok(Self { nested, holders })
    }
}

fn child_path(parent: &str, name: &str) -> String {
    format!("{parent}.{name}")
}

fn digest_sources_error(holder: &str, reason: impl std::fmt::Display) -> Error {
    digest_metadata_error(DIGEST_SOURCES_KEY, holder, reason)
}

fn digest_algorithm_error(holder: &str, reason: impl std::fmt::Display) -> Error {
    digest_metadata_error(DIGEST_ALGORITHM_KEY, holder, reason)
}

fn digest_role_error(holder: &str, reason: impl std::fmt::Display) -> Error {
    digest_metadata_error(DIGEST_ROLE_KEY, holder, reason)
}

fn digest_metadata_error(key: &'static str, holder: &str, reason: impl std::fmt::Display) -> Error {
    Error::Core(crate::Error::InvalidMetadataValue {
        key: smol_str::SmolStr::new_static(key),
        reason: smol_str::format_smolstr!("holder {holder}: {reason}"),
    })
}

/// Refuse a digest declaration no fill plan can reach.
///
/// A plan descends into Struct children, because those are the ones that are
/// columns of their own. Under a list, map, union, dictionary, or run-end
/// layout a holder is written by nobody and left at its canonical default,
/// which a containing holder would then read as though it were an answer. The
/// declaration is refused where it is written rather than silently ignored,
/// exactly as a `DIGEST:sources` path that descends through a collection is.
fn reject_unreachable_digests(dtype: &DataType, path: &str, container: &str) -> Result<()> {
    // A dictionary encodes a value type rather than a child column, so what it
    // holds carries no name of its own to extend the path with.
    if let DataType::Dictionary(dictionary) = dtype {
        reject_unreachable_digests(dictionary.value(), path, container)?;
    }
    for index in 0..dtype.field_len() {
        let Some(child) = dtype.get_field_at(index) else {
            continue;
        };
        let child_path = child_path(path, child.name());
        let digest = child.as_digest();
        if digest.is_holder() {
            return Err(digest_role_error(
                &child_path,
                format!(
                    "a holder is filled as a Struct column, and no fill descends into {container}"
                ),
            ));
        }
        if digest
            .sources()
            .map_err(|error| {
                digest_sources_error(&child_path, format!("cannot read stored sources: {error}"))
            })?
            .is_some()
        {
            return Err(digest_sources_error(
                &child_path,
                "DIGEST:sources belongs only to a digest holder",
            ));
        }
        if digest
            .algorithm()
            .map_err(|error| {
                digest_algorithm_error(
                    &child_path,
                    format!("cannot read stored algorithm: {error}"),
                )
            })?
            .is_some()
        {
            return Err(digest_algorithm_error(
                &child_path,
                "DIGEST:algorithm belongs only to a digest holder",
            ));
        }
        if digest.time().is_some() {
            return Err(digest_metadata_error(
                DIGEST_TIME_KEY,
                &child_path,
                "DIGEST:time belongs only to a digest holder",
            ));
        }
        if digest
            .unit()
            .map_err(|error| {
                digest_metadata_error(
                    DIGEST_UNIT_KEY,
                    &child_path,
                    format!("cannot read stored unit: {error}"),
                )
            })?
            .is_some()
        {
            return Err(digest_metadata_error(
                DIGEST_UNIT_KEY,
                &child_path,
                "DIGEST:unit belongs only to a digest holder",
            ));
        }
        reject_unreachable_digests(child.dtype(), &child_path, container)?;
    }
    Ok(())
}

fn default_holder_algorithm(field: &Field) -> Option<DigestAlgorithm> {
    if field.as_digest().time().is_some() {
        return crate::txhash::coupled_holder_algorithm(field);
    }
    match field.dtype() {
        DataType::Int32 | DataType::UInt32 => Some(DigestAlgorithm::Xxh32),
        DataType::Int64 | DataType::UInt64 => Some(DigestAlgorithm::Xxh3),
        DataType::Bytes(parameters) if parameters.fixed() == Some(16) => {
            Some(DigestAlgorithm::Xxh128)
        }
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
                    expected_holder_dtypes(field, algorithm),
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
        let expected = if field.as_digest().time().is_some() {
            "fixed_size_binary[12], [16], or [24] when coupling an instant"
        } else {
            "int32, uint32, int64, uint64, or fixed_size_binary[16]"
        };
        Error::IncompatibleSchema(format!(
            "digest holder {holder_path} must be {expected}, got {}",
            field.dtype()
        ))
    })
}

/// Resolve an exact-name-first path through Struct children only.
///
/// `key` names the property the path was written under, so a refusal points
/// at `DIGEST:sources` or `DIGEST:time` as the holder spelled it.
fn resolve_selection<'field>(
    fields: &'field [Field],
    path: &str,
    holder: &str,
    key: &'static str,
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
            if let Ok(mut tail) =
                resolve_selection(field.fields(), &path[boundary + 1..], holder, key)
            {
                tail.steps.insert(0, index);
                return Ok(tail);
            }
        }
        offset = boundary + 1;
    }
    Err(digest_metadata_error(
        key,
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
            Some(holder) => digest_sources_error(holder, reason),
            None => Error::IncompatibleSchema(reason),
        });
    }
    selection.steps.push(index);
    selection.field = holder;
    Ok(selection)
}

fn fill_struct<S: ArrowDigestState>(
    prototype: &S,
    plan: &StructPlan,
    fields: &[Field],
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
            fields[*index].fields(),
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
                        fields[*index].name(),
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
        let field = &fields[holder.index];
        let original = Arc::clone(&columns[holder.index]);
        // The holder's own column lands once and is read per row through
        // its leaf; a forced pass reads none of it.
        let stored = if force {
            None
        } else {
            Some(land_under(
                Arc::new(field.clone()),
                Arc::clone(&original),
                parent_nulls,
                &Proof::Unproven,
            )?)
        };
        let mut mask = Vec::with_capacity(row_count);
        for row in 0..row_count {
            let visible = parent_nulls.is_none_or(|nulls| nulls.is_valid(row));
            let recompute = match &stored {
                _ if !visible => false,
                None => true,
                Some(stored) => stored.scalar(row)? == holder.default,
            };
            mask.push(recompute);
        }
        if !mask.iter().any(|selected| *selected) {
            continue;
        }
        // Read under the mask: a row this pass leaves alone is never
        // restated, so a preserved cell cannot fail the batch over an instant
        // it does not couple.
        let unix = match &holder.time {
            Some(time) => Some(crate::txhash::arrow::unix_selection(
                &columns,
                fields,
                &time.steps,
                parent_nulls,
                time.unit,
                &mask,
                &holder.path,
            )?),
            None => None,
        };
        // Each column a selection starts from lands once, beneath the
        // record's validity, after every holder before this one filled it.
        let landed = landed_roots(&columns, fields, &holder.selected, parent_nulls)?;
        let mut values = Vec::with_capacity(row_count);
        let mut worker = if holder.use_prototype {
            FillState::Prototype(prototype.clone())
        } else {
            FillState::Unseeded(holder.algorithm.digester())
        };
        for (row, selected) in mask.iter().copied().enumerate() {
            worker.reset();
            if selected {
                write_sequence_header(&mut worker, holder.selected.len());
                for steps in &holder.selected {
                    feed_selection(&mut worker, &landed, steps, row)?;
                }
            }
            values.push(worker.answer());
        }
        let computed = match (&holder.time, &unix) {
            (Some(time), Some(unix)) => {
                // A null instant names no key. A nullable holder stores that
                // absence; a required one cannot, and inventing an instant
                // would be worse than no digest.
                if !field.is_nullable() {
                    if let Some(row) = (0..row_count).find(|row| mask[*row] && unix.is_null(*row)) {
                        return Err(Error::IncompatibleSchema(format!(
                            "holder {} row {row}: DIGEST:time source is null and the holder is required",
                            holder.path
                        )));
                    }
                }
                crate::txhash::arrow::collect(unix, &values, None, time.unit, holder.algorithm)?
            }
            _ => collect(&values, holder.algorithm, None),
        };
        let computed = match &holder.bits {
            Some(bits) => bits.reconcile_array(computed)?,
            None => computed,
        };
        let mask = BooleanArray::from(mask);
        columns[holder.index] = zip(&mask, &computed.as_ref(), &original.as_ref())?;
        changed = true;
    }
    Ok((columns, changed))
}

/// Land the root columns `selected` starts from, each once.
fn landed_roots(
    columns: &[ArrayRef],
    fields: &[Field],
    selected: &[Vec<usize>],
    parent_nulls: Option<&NullBuffer>,
) -> Result<Vec<Option<Serie>>> {
    let mut landed: Vec<Option<Serie>> = vec![None; columns.len()];
    for root in selected.iter().filter_map(|steps| steps.first().copied()) {
        if landed[root].is_none() {
            landed[root] = Some(land_under(
                Arc::new(fields[root].clone()),
                Arc::clone(&columns[root]),
                parent_nulls,
                &Proof::Unproven,
            )?);
        }
    }
    Ok(landed)
}

fn feed_selection(
    digester: &mut impl Hasher,
    roots: &[Option<Serie>],
    steps: &[usize],
    row: usize,
) -> Result<()> {
    let Some((first, below)) = steps.split_first() else {
        return Err(Error::IncompatibleSchema(
            "a digest selection cannot have an empty path".to_owned(),
        ));
    };
    let mut column = roots[*first].as_ref().ok_or(Error::Internal {
        site: "xxhash::arrow::feed_selection",
    })?;
    for index in below {
        if column.is_null(row)? {
            write_null(digester);
            return Ok(());
        }
        column = column.child_at(*index).ok_or(Error::Internal {
            site: "xxhash::arrow::feed_selection",
        })?;
    }
    feed_selected_cell(digester, column, row)
}

/// Feed a selected holder by its unsigned digest payload, independent of the
/// signed or unsigned same-width Arrow storage chosen for that payload.
fn feed_selected_cell(digester: &mut impl Hasher, column: &Serie, index: usize) -> Result<()> {
    if column
        .field()
        .is_some_and(|field| field.as_digest().is_holder())
    {
        if let Some(value) = column.as_int32().and_then(|held| held.value(index)) {
            write_unsigned(
                digester,
                u128::from(u32::from_ne_bytes(value.to_ne_bytes())),
            );
            return Ok(());
        }
        if let Some(value) = column.as_int64().and_then(|held| held.value(index)) {
            write_unsigned(
                digester,
                u128::from(u64::from_ne_bytes(value.to_ne_bytes())),
            );
            return Ok(());
        }
    }
    feed_cell(digester, column, index)
}

/// Digest every row of a batch, in schema order.
///
/// Every field contributes except one carrying `DIGEST:role=holder`, which is
/// an output rather than an input. `holder` is the only digest role, so a
/// schema marks the field it fills and leaves the ones that field reads
/// ordinary columns. The contributing values remain an ordered sequence, which
/// is the canonical row shape everywhere in this project.
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
    let digests = row_digests_with(&algorithm.digester(), batch)?;
    Ok(collect(&digests, algorithm, None))
}

/// Digest every row of a batch under a configured state.
///
/// The prototype supplies the algorithm, seed, and secret; bytes already
/// fed to it are never read. This is the loop [`row_digests`] runs, and what
/// a coupled column reads its digest half from.
pub(crate) fn row_digests_with<S: ArrowDigestState>(
    prototype: &S,
    batch: &RecordBatch,
) -> Result<Vec<Digest>> {
    let fields: Vec<Field> = batch
        .schema()
        .fields()
        .iter()
        .map(|field| Field::from_arrow_field_ref(Arc::clone(field)).map_err(Error::from))
        .collect::<Result<_>>()?;
    // Each contributing column lands once and is read through its leaf.
    let selected = fields
        .into_iter()
        .zip(batch.columns())
        .filter(|(field, _)| is_digest_source(field))
        .map(|(field, column)| land(Arc::new(field), Arc::clone(column), &Proof::Unproven))
        .collect::<Result<Vec<Serie>>>()?;
    let mut digests = Vec::with_capacity(batch.num_rows());
    let mut digester = prototype.clone();
    for index in 0..batch.num_rows() {
        digester.reset();
        write_sequence_header(&mut digester, selected.len());
        for column in &selected {
            feed_cell(&mut digester, column, index)?;
        }
        digests.push(digester.answer());
    }
    Ok(digests)
}

/// Digest every value of one column.
///
/// This is the single-column form [`row_digests`] composes: each answer is the
/// cell's own value fed through
/// [`Scalar::write_bytes`](crate::Scalar::write_bytes), with no row framing
/// around it. A null feeds the null tag, so a null and an empty string never
/// collide.
///
/// The array is reconciled to `field` first, because the answer is defined by
/// the value model rather than by the layout: an `int32` column read under an
/// `int64` declaration is the same numbers and must answer the same digests,
/// and a struct whose children are stored in another order is the same row.
/// Reconciling costs nothing when the array is already the declared shape.
///
/// The reconciliation is strict on both questions a cast asks, unlike the one
/// that completes a stored shape on a write: a value the declaration cannot
/// hold is named rather than nulled, because a null is a value here and two
/// unconvertible cells must not answer one digest, and a required column the
/// array does not carry is named rather than defaulted, because a digest of an
/// invented value is worse than no digest.
///
/// # Errors
///
/// Returns an error when the array cannot be reconciled to `field`, or a value
/// cannot be represented.
pub fn column_digests(
    array: ArrayRef,
    field: &Field,
    algorithm: DigestAlgorithm,
) -> Result<ArrayRef> {
    let digests = column_digests_with(&algorithm.digester(), array, field)?;
    Ok(collect(&digests, algorithm, None))
}

/// Digest every value of one column under a configured state.
///
/// The loop [`column_digests`] runs, reconciliation included; the prototype
/// supplies the algorithm, seed, and secret.
pub(crate) fn column_digests_with<S: ArrowDigestState>(
    prototype: &S,
    array: ArrayRef,
    field: &Field,
) -> Result<Vec<Digest>> {
    let column = Serie::from_arrow_array(
        Some(field),
        array,
        ArrowCastOptions::new()
            .with_safe(false)
            .with_nullability(Nullability::Strict),
    )?;
    let mut digests = Vec::with_capacity(column.len());
    let mut digester = prototype.clone();
    for index in 0..column.len() {
        digester.reset();
        feed_cell(&mut digester, &column, index)?;
        digests.push(digester.answer());
    }
    Ok(digests)
}

/// Build the digest column the algorithm's width calls for.
///
/// `nulls` marks the rows that hold no digest; the digest functions answer
/// none, and only a coupled column split back into its halves carries any.
pub(crate) fn collect(
    digests: &[Digest],
    algorithm: DigestAlgorithm,
    nulls: Option<NullBuffer>,
) -> ArrayRef {
    match algorithm {
        // Reserved once for every digest: a filtered collect has no size to
        // reserve by and would grow the buffer as it fills.
        DigestAlgorithm::Xxh32 => {
            let mut values = Vec::with_capacity(digests.len());
            values.extend(digests.iter().filter_map(|digest| digest.as_u32()));
            Arc::new(UInt32Array::new(values.into(), nulls))
        }
        DigestAlgorithm::Xxh64 | DigestAlgorithm::Xxh3 => {
            let mut values = Vec::with_capacity(digests.len());
            values.extend(digests.iter().filter_map(|digest| digest.as_u64()));
            Arc::new(UInt64Array::new(values.into(), nulls))
        }
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
                nulls,
            ))
        }
    }
}

/// Feed one cell's canonical bytes, reading the buffer where the leaf lends
/// it.
///
/// Every other leaf reads its value through the reading it resolved where
/// it landed - no downcast, no dispatch on the datatype - and feeds it
/// through [`Scalar::write_bytes`](crate::Scalar::write_bytes), which is
/// what the buffer arms spell without the value: a code holds its text to
/// the width its standard fixes, a temporal its unit and zone, a nested
/// layout composes its children.
fn feed_cell(digester: &mut impl Hasher, column: &Serie, index: usize) -> Result<()> {
    // A union or a run-end encoding hides its own validity, so absence there
    // is the child's answer rather than the parent's, exactly as its value
    // reads it.
    if !matches!(column, Serie::Union(_) | Serie::RunEndEncoded(_)) && column.is_null(index)? {
        write_null(digester);
        return Ok(());
    }
    match column {
        Serie::Null(_) => write_null(digester),
        Serie::Boolean(held) => write_bool(digester, held.value(index).unwrap_or_default()),
        Serie::Int8(held) => write_signed(digester, i128::from(held.values()[index])),
        Serie::Int16(held) => write_signed(digester, i128::from(held.values()[index])),
        Serie::Int32(held) => write_signed(digester, i128::from(held.values()[index])),
        Serie::Int64(held) => write_signed(digester, i128::from(held.values()[index])),
        Serie::UInt8(held) => write_unsigned(digester, u128::from(held.values()[index])),
        Serie::UInt16(held) => write_unsigned(digester, u128::from(held.values()[index])),
        Serie::UInt32(held) => write_unsigned(digester, u128::from(held.values()[index])),
        Serie::UInt64(held) => write_unsigned(digester, u128::from(held.values()[index])),
        Serie::Float16(held) => write_float(digester, f64::from(held.values()[index].to_f32())),
        Serie::Float32(held) => write_float(digester, f64::from(held.values()[index])),
        Serie::Float64(held) => write_float(digester, held.values()[index]),
        // A string digests as its characters, whatever charset holds them:
        // the digest is of the value, and the charset is how it is stored.
        // Text storage was validated when it was written, so a UTF-8 cell is
        // fed straight from the run it lies in.
        Serie::Utf8String(held) => write_string(digester, held.value(index).unwrap_or_default()),
        Serie::LargeUtf8String(held) => {
            write_string(digester, held.value(index).unwrap_or_default());
        }
        Serie::Utf8ViewString(held) => {
            write_string(digester, held.value(index).unwrap_or_default());
        }
        Serie::BinaryString(_)
        | Serie::LargeBinaryString(_)
        | Serie::BinaryViewString(_)
        | Serie::FixedString(_) => feed_string(digester, column, index)?,
        // Bytes digest as their payload, whichever layout holds them.
        Serie::Binary(_) | Serie::LargeBinary(_) | Serie::BinaryView(_) | Serie::FixedBytes(_) => {
            write_binary(digester, column.value_bytes(index).unwrap_or_default());
        }
        _ => column.scalar(index)?.write_bytes(digester),
    }
    Ok(())
}

/// Downcast an array to the layout its datatype names.
pub(crate) fn downcast<T: 'static>(array: &dyn Array) -> Result<&T> {
    array.as_any().downcast_ref::<T>().ok_or_else(|| {
        Error::IncompatibleSchema(format!(
            "expected an Arrow {} array, got {}",
            std::any::type_name::<T>(),
            array.data_type()
        ))
    })
}

/// Feed one string cell a binary layout holds as the characters a [`Str`]
/// read from it holds.
///
/// Binary storage goes through [`Str::from_bytes`], the one door bytes take
/// into a string value: a fixed slot is trimmed of its padding, and a legacy
/// charset is transcribed rather than refused.
fn feed_string(digester: &mut impl Hasher, column: &Serie, index: usize) -> Result<()> {
    let Some(DataType::String(parameters)) = column.field().map(Field::dtype) else {
        return Err(Error::Internal {
            site: "xxhash::arrow::feed_string",
        });
    };
    let bytes = column.value_bytes(index).unwrap_or_default();
    write_string(digester, &Str::from_bytes(bytes, *parameters)?);
    Ok(())
}
