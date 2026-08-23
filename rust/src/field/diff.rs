//! Metadata-aware schema equality and lazy, terminal-readable differences.

use std::cmp::Ordering;
use std::collections::VecDeque;
use std::iter::FusedIterator;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::{DataType, Fields, Metadata, UnionFields};

use super::Field;

/// A lazy iterator over stable, UTF-8 schema difference lines.
///
/// Lines use `≠` for changed values, `−` for values present only on the left,
/// and `+` for values present only on the right. They contain no ANSI escape
/// sequences, so output remains readable in terminals, logs, and snapshots.
pub struct Differences<'schema> {
    engine: DiffEngine,
    /// Emit [`EQUAL_LINE`] when the comparison finds nothing.
    return_equal: bool,
    /// Set once the iterator has yielded anything at all.
    yielded: bool,
    marker: PhantomData<&'schema ()>,
}

/// The line reported when two values are equal.
pub(crate) const EQUAL_LINE: &str = "✓ equal";

struct DiffEngine {
    work: Vec<Work>,
    pending: VecDeque<String>,
    with_metadata: bool,
}

enum Work {
    Field {
        left: Field,
        right: Field,
        path: String,
    },
    DataType {
        left: DataType,
        right: DataType,
        path: String,
    },
    Metadata {
        left: Metadata,
        right: Metadata,
        left_after: Option<String>,
        right_after: Option<String>,
        path: String,
    },
    FieldSlices {
        left: Fields,
        right: Fields,
        path: String,
        phase: SlicePhase,
        index: usize,
    },
    UnionSlices {
        left: UnionFields,
        right: UnionFields,
        path: String,
        phase: UnionSlicePhase,
        index: usize,
    },
}

/// A lazy owning schema-difference cursor suitable for FFI runtimes.
///
/// Construction clones only Yggdryl's shared native snapshots. Nested fields,
/// metadata, and Arrow caches remain shared; formatted lines are produced one
/// at a time by the same traversal used by [`Differences`].
pub struct OwnedDifferences {
    engine: DiffEngine,
    /// Emit [`EQUAL_LINE`] when the comparison finds nothing.
    return_equal: bool,
    /// Set once the cursor has yielded anything at all.
    yielded: bool,
}

#[derive(Clone, Copy)]
enum SlicePhase {
    LeftExtras,
    RightExtras,
    Common,
}

#[derive(Clone, Copy)]
enum UnionSlicePhase {
    TypeIds,
    LeftExtras,
    RightExtras,
    Common,
}

fn data_type_snapshots_identical(left: &DataType, right: &DataType) -> bool {
    use DataType as D;

    if std::ptr::eq(left, right) {
        return true;
    }
    match (left, right) {
        (D::List(left), D::List(right))
        | (D::ListView(left), D::ListView(right))
        | (D::LargeList(left), D::LargeList(right))
        | (D::LargeListView(left), D::LargeListView(right)) => Arc::ptr_eq(left, right),
        (D::FixedSizeList(left, left_size), D::FixedSizeList(right, right_size)) => {
            left_size == right_size && Arc::ptr_eq(left, right)
        }
        (D::Struct(left), D::Struct(right)) => left.shares_storage_with(right),
        (D::Union(left, left_mode), D::Union(right, right_mode)) => {
            left_mode == right_mode && left.shares_storage_with(right)
        }
        (D::Dictionary(left), D::Dictionary(right)) => Arc::ptr_eq(left, right),
        (D::Map(left), D::Map(right)) => Arc::ptr_eq(left, right),
        (D::RunEndEncoded(left), D::RunEndEncoded(right)) => Arc::ptr_eq(left, right),
        // Every remaining variant is scalar or carries only compact parameters.
        _ => left == right,
    }
}

fn field_snapshots_identical(left: &Field, right: &Field, with_metadata: bool) -> bool {
    std::ptr::eq(left, right)
        || left.name == right.name
            && left.nullable == right.nullable
            && left.dictionary_id == right.dictionary_id
            && left.dictionary_is_ordered == right.dictionary_is_ordered
            && data_type_snapshots_identical(&left.data_type, &right.data_type)
            && (!with_metadata || left.metadata.shares_storage_with(&right.metadata))
}

impl DiffEngine {
    fn from_fields(left: &Field, right: &Field, with_metadata: bool) -> Self {
        Self {
            work: if field_snapshots_identical(left, right, with_metadata) {
                Vec::new()
            } else {
                vec![Work::Field {
                    left: left.clone(),
                    right: right.clone(),
                    path: "$".to_owned(),
                }]
            },
            pending: VecDeque::new(),
            with_metadata,
        }
    }

    fn from_data_types(left: &DataType, right: &DataType, with_metadata: bool) -> Self {
        Self {
            work: if data_type_snapshots_identical(left, right) {
                Vec::new()
            } else {
                vec![Work::DataType {
                    left: left.clone(),
                    right: right.clone(),
                    path: "$".to_owned(),
                }]
            },
            pending: VecDeque::new(),
            with_metadata,
        }
    }

    fn compare_field(&mut self, left: Field, right: Field, path: String) {
        if left.name != right.name {
            self.pending.push_back(changed_debug(
                &property_path(&path, "name"),
                left.name(),
                right.name(),
            ));
        }
        if left.nullable != right.nullable {
            self.pending.push_back(changed_display(
                &property_path(&path, "nullable"),
                left.nullable,
                right.nullable,
            ));
        }
        if left.dictionary_id != right.dictionary_id {
            self.pending.push_back(changed_display(
                &property_path(&path, "dictionary_id"),
                left.dictionary_id,
                right.dictionary_id,
            ));
        }
        if left.dictionary_is_ordered != right.dictionary_is_ordered {
            self.pending.push_back(changed_display(
                &property_path(&path, "dictionary_is_ordered"),
                left.dictionary_is_ordered,
                right.dictionary_is_ordered,
            ));
        }
        // Push metadata first so the LIFO engine can yield an early physical
        // datatype difference without scanning a distinct, equal wide map.
        if self.with_metadata && !left.metadata.shares_storage_with(&right.metadata) {
            self.push_metadata(&left, &right, path.clone());
        }
        if !data_type_snapshots_identical(&left.data_type, &right.data_type) {
            self.work.push(Work::DataType {
                left: left.data_type.clone(),
                right: right.data_type.clone(),
                path: property_path(&path, "data_type"),
            });
        }
    }

    fn push_metadata(&mut self, left: &Field, right: &Field, path: String) {
        self.work.push(Work::Metadata {
            left: left.metadata.clone(),
            right: right.metadata.clone(),
            left_after: None,
            right_after: None,
            path,
        });
    }

    fn compare_metadata(
        &mut self,
        left: Metadata,
        right: Metadata,
        mut left_after: Option<String>,
        mut right_after: Option<String>,
        path: String,
    ) {
        loop {
            let left_entry = left.next_entry(left_after.as_deref());
            let right_entry = right.next_entry(right_after.as_deref());
            let difference = match (left_entry, right_entry) {
                (Some((left_key, left_value)), Some((right_key, right_value))) => {
                    match left_key.cmp(right_key) {
                        Ordering::Less => {
                            left_after = Some(left_key.to_owned());
                            Some(removed_debug(&metadata_path(&path, left_key), left_value))
                        }
                        Ordering::Greater => {
                            right_after = Some(right_key.to_owned());
                            Some(added_debug(&metadata_path(&path, right_key), right_value))
                        }
                        Ordering::Equal => {
                            left_after = Some(left_key.to_owned());
                            right_after = Some(right_key.to_owned());
                            if left_value != right_value {
                                Some(changed_debug(
                                    &metadata_path(&path, left_key),
                                    left_value,
                                    right_value,
                                ))
                            } else {
                                None
                            }
                        }
                    }
                }
                (Some((key, value)), None) => {
                    left_after = Some(key.to_owned());
                    Some(removed_debug(&metadata_path(&path, key), value))
                }
                (None, Some((key, value))) => {
                    right_after = Some(key.to_owned());
                    Some(added_debug(&metadata_path(&path, key), value))
                }
                (None, None) => return,
            };
            if let Some(difference) = difference {
                self.work.push(Work::Metadata {
                    left,
                    right,
                    left_after,
                    right_after,
                    path,
                });
                self.pending.push_back(difference);
                return;
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn compare_data_type(&mut self, left: DataType, right: DataType, path: String) {
        if data_type_snapshots_identical(&left, &right) {
            return;
        }
        use DataType as D;
        match (&left, &right) {
            (D::Timestamp(left_unit, left_zone), D::Timestamp(right_unit, right_zone)) => {
                if left_unit != right_unit {
                    self.pending.push_back(changed_display(
                        &property_path(&path, "unit"),
                        left_unit,
                        right_unit,
                    ));
                }
                if left_zone != right_zone {
                    self.pending.push_back(changed_debug(
                        &property_path(&path, "timezone"),
                        left_zone,
                        right_zone,
                    ));
                }
            }
            (D::Time32(left), D::Time32(right))
            | (D::Time64(left), D::Time64(right))
            | (D::Duration32(left), D::Duration32(right))
            | (D::Duration64(left), D::Duration64(right))
            | (D::Interval(left), D::Interval(right)) => {
                if left != right {
                    self.pending.push_back(changed_display(
                        &property_path(&path, "unit"),
                        left,
                        right,
                    ));
                }
            }
            (D::FixedSizeBinary(left), D::FixedSizeBinary(right)) => {
                if left != right {
                    self.pending.push_back(changed_display(
                        &property_path(&path, "width"),
                        left,
                        right,
                    ));
                }
            }
            (D::List(left), D::List(right))
            | (D::ListView(left), D::ListView(right))
            | (D::LargeList(left), D::LargeList(right))
            | (D::LargeListView(left), D::LargeListView(right)) => {
                self.push_field_property(left, right, &path, "item");
            }
            (D::FixedSizeList(left, left_size), D::FixedSizeList(right, right_size)) => {
                if left_size != right_size {
                    self.pending.push_back(changed_display(
                        &property_path(&path, "length"),
                        left_size,
                        right_size,
                    ));
                }
                self.push_field_property(left, right, &path, "item");
            }
            (D::Struct(left), D::Struct(right)) => {
                self.push_field_slices(left.clone(), right.clone(), &path);
            }
            (D::Union(left, left_mode), D::Union(right, right_mode)) => {
                if left_mode != right_mode {
                    self.pending.push_back(changed_display(
                        &property_path(&path, "mode"),
                        left_mode,
                        right_mode,
                    ));
                }
                let left_fields = left.as_fields();
                let right_fields = right.as_fields();
                if left_fields.len() != right_fields.len() {
                    self.pending.push_back(changed_display(
                        &property_path(&path, "field_count"),
                        left_fields.len(),
                        right_fields.len(),
                    ));
                }
                self.work.push(Work::UnionSlices {
                    left: left.clone(),
                    right: right.clone(),
                    path,
                    phase: UnionSlicePhase::TypeIds,
                    index: 0,
                });
            }
            (D::Dictionary(left), D::Dictionary(right)) => {
                self.push_data_type(left.value(), right.value(), property_path(&path, "value"));
                self.push_data_type(left.key(), right.key(), property_path(&path, "key"));
            }
            (
                D::Decimal32 {
                    precision: left_precision,
                    scale: left_scale,
                }
                | D::Decimal64 {
                    precision: left_precision,
                    scale: left_scale,
                }
                | D::Decimal128 {
                    precision: left_precision,
                    scale: left_scale,
                }
                | D::Decimal256 {
                    precision: left_precision,
                    scale: left_scale,
                },
                D::Decimal32 {
                    precision: right_precision,
                    scale: right_scale,
                }
                | D::Decimal64 {
                    precision: right_precision,
                    scale: right_scale,
                }
                | D::Decimal128 {
                    precision: right_precision,
                    scale: right_scale,
                }
                | D::Decimal256 {
                    precision: right_precision,
                    scale: right_scale,
                },
            ) if left.id() == right.id() => {
                if left_precision != right_precision {
                    self.pending.push_back(changed_display(
                        &property_path(&path, "precision"),
                        left_precision,
                        right_precision,
                    ));
                }
                if left_scale != right_scale {
                    self.pending.push_back(changed_display(
                        &property_path(&path, "scale"),
                        left_scale,
                        right_scale,
                    ));
                }
            }
            (D::Map(left), D::Map(right)) => {
                if left.keys_sorted() != right.keys_sorted() {
                    self.pending.push_back(changed_display(
                        &property_path(&path, "keys_sorted"),
                        left.keys_sorted(),
                        right.keys_sorted(),
                    ));
                }
                self.push_field_property(left.entries(), right.entries(), &path, "entries");
            }
            (D::RunEndEncoded(left), D::RunEndEncoded(right)) => {
                self.push_field_property(left.values(), right.values(), &path, "values");
                self.push_field_property(left.run_ends(), right.run_ends(), &path, "run_ends");
            }
            (D::Geometry(left), D::Geometry(right)) | (D::Geography(left), D::Geography(right)) => {
                if left.crs() != right.crs() {
                    self.pending.push_back(changed_debug(
                        &property_path(&path, "crs"),
                        left.crs(),
                        right.crs(),
                    ));
                }
                if let (Some(left_algorithm), Some(right_algorithm)) =
                    (left.algorithm(), right.algorithm())
                {
                    if left_algorithm != right_algorithm {
                        self.pending.push_back(changed_display(
                            &property_path(&path, "algorithm"),
                            left_algorithm,
                            right_algorithm,
                        ));
                    }
                }
            }
            _ if left.id() != right.id() => self.pending.push_back(changed_display(
                &property_path(&path, "kind"),
                left.name(),
                right.name(),
            )),
            _ => {}
        }
    }

    fn push_field(&mut self, left: &Field, right: &Field, path: String) {
        if field_snapshots_identical(left, right, self.with_metadata) {
            return;
        }
        self.work.push(Work::Field {
            left: left.clone(),
            right: right.clone(),
            path,
        });
    }

    fn push_field_property(&mut self, left: &Field, right: &Field, path: &str, property: &str) {
        if field_snapshots_identical(left, right, self.with_metadata) {
            return;
        }
        self.push_field(left, right, property_path(path, property));
    }

    fn push_data_type(&mut self, left: &DataType, right: &DataType, path: String) {
        if data_type_snapshots_identical(left, right) {
            return;
        }
        self.work.push(Work::DataType {
            left: left.clone(),
            right: right.clone(),
            path,
        });
    }

    fn push_field_slices(&mut self, left: Fields, right: Fields, path: &str) {
        if left.shares_storage_with(&right) {
            return;
        }
        if left.len() != right.len() {
            self.pending.push_back(changed_display(
                &property_path(path, "field_count"),
                left.len(),
                right.len(),
            ));
        }
        let right_len = right.len();
        self.work.push(Work::FieldSlices {
            left,
            right,
            path: path.to_owned(),
            phase: SlicePhase::LeftExtras,
            index: right_len,
        });
    }

    fn advance_field_slices(
        &mut self,
        left: Fields,
        right: Fields,
        path: String,
        mut phase: SlicePhase,
        mut index: usize,
    ) {
        loop {
            match phase {
                SlicePhase::LeftExtras if index < left.len() => {
                    let difference = removed_display(
                        &indexed_path(&path, "fields", index),
                        field_diff_value(&left[index], self.with_metadata),
                    );
                    self.work.push(Work::FieldSlices {
                        left,
                        right,
                        path,
                        phase,
                        index: index + 1,
                    });
                    self.pending.push_back(difference);
                    return;
                }
                SlicePhase::LeftExtras => {
                    phase = SlicePhase::RightExtras;
                    index = left.len();
                }
                SlicePhase::RightExtras if index < right.len() => {
                    let difference = added_display(
                        &indexed_path(&path, "fields", index),
                        field_diff_value(&right[index], self.with_metadata),
                    );
                    self.work.push(Work::FieldSlices {
                        left,
                        right,
                        path,
                        phase,
                        index: index + 1,
                    });
                    self.pending.push_back(difference);
                    return;
                }
                SlicePhase::RightExtras => {
                    phase = SlicePhase::Common;
                    index = 0;
                }
                SlicePhase::Common if index < left.len().min(right.len()) => {
                    let child_index = index;
                    index += 1;
                    if field_snapshots_identical(
                        &left[child_index],
                        &right[child_index],
                        self.with_metadata,
                    ) {
                        continue;
                    }
                    let left_field = left[child_index].clone();
                    let right_field = right[child_index].clone();
                    let child_path = indexed_path(&path, "fields", child_index);
                    self.work.push(Work::FieldSlices {
                        left,
                        right,
                        path,
                        phase,
                        index,
                    });
                    self.work.push(Work::Field {
                        left: left_field,
                        right: right_field,
                        path: child_path,
                    });
                    return;
                }
                SlicePhase::Common => return,
            }
        }
    }

    fn advance_union_slices(
        &mut self,
        left: UnionFields,
        right: UnionFields,
        path: String,
        mut phase: UnionSlicePhase,
        mut index: usize,
    ) {
        loop {
            match phase {
                UnionSlicePhase::TypeIds if index < left.len().min(right.len()) => {
                    let left_id = left[index].0;
                    let right_id = right[index].0;
                    index += 1;
                    if left_id == right_id {
                        continue;
                    }
                    let field_path = indexed_path(&path, "fields", index - 1);
                    let difference =
                        changed_display(&property_path(&field_path, "type_id"), left_id, right_id);
                    self.work.push(Work::UnionSlices {
                        left,
                        right,
                        path,
                        phase,
                        index,
                    });
                    self.pending.push_back(difference);
                    return;
                }
                UnionSlicePhase::TypeIds => {
                    phase = UnionSlicePhase::LeftExtras;
                    index = right.len();
                }
                UnionSlicePhase::LeftExtras if index < left.len() => {
                    let (type_id, field) = &left[index];
                    let difference = removed_display(
                        &indexed_path(&path, "fields", index),
                        format!(
                            "type_id={type_id}, {}",
                            field_diff_value(field, self.with_metadata)
                        ),
                    );
                    self.work.push(Work::UnionSlices {
                        left,
                        right,
                        path,
                        phase,
                        index: index + 1,
                    });
                    self.pending.push_back(difference);
                    return;
                }
                UnionSlicePhase::LeftExtras => {
                    phase = UnionSlicePhase::RightExtras;
                    index = left.len();
                }
                UnionSlicePhase::RightExtras if index < right.len() => {
                    let (type_id, field) = &right[index];
                    let difference = added_display(
                        &indexed_path(&path, "fields", index),
                        format!(
                            "type_id={type_id}, {}",
                            field_diff_value(field, self.with_metadata)
                        ),
                    );
                    self.work.push(Work::UnionSlices {
                        left,
                        right,
                        path,
                        phase,
                        index: index + 1,
                    });
                    self.pending.push_back(difference);
                    return;
                }
                UnionSlicePhase::RightExtras => {
                    phase = UnionSlicePhase::Common;
                    index = 0;
                }
                UnionSlicePhase::Common if index < left.len().min(right.len()) => {
                    let child_index = index;
                    index += 1;
                    if field_snapshots_identical(
                        &left[child_index].1,
                        &right[child_index].1,
                        self.with_metadata,
                    ) {
                        continue;
                    }
                    let left_field = left[child_index].1.clone();
                    let right_field = right[child_index].1.clone();
                    let child_path = indexed_path(&path, "fields", child_index);
                    self.work.push(Work::UnionSlices {
                        left,
                        right,
                        path,
                        phase,
                        index,
                    });
                    self.work.push(Work::Field {
                        left: left_field,
                        right: right_field,
                        path: child_path,
                    });
                    return;
                }
                UnionSlicePhase::Common => return,
            }
        }
    }
}

impl Iterator for DiffEngine {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(line) = self.pending.pop_front() {
                return Some(line);
            }
            match self.work.pop()? {
                Work::Field { left, right, path } => self.compare_field(left, right, path),
                Work::DataType { left, right, path } => {
                    self.compare_data_type(left, right, path);
                }
                Work::Metadata {
                    left,
                    right,
                    left_after,
                    right_after,
                    path,
                } => self.compare_metadata(left, right, left_after, right_after, path),
                Work::FieldSlices {
                    left,
                    right,
                    path,
                    phase,
                    index,
                } => self.advance_field_slices(left, right, path, phase, index),
                Work::UnionSlices {
                    left,
                    right,
                    path,
                    phase,
                    index,
                } => self.advance_union_slices(left, right, path, phase, index),
            }
        }
    }
}

impl FusedIterator for DiffEngine {}

impl<'schema> Differences<'schema> {
    pub(crate) fn from_fields(
        left: &'schema Field,
        right: &'schema Field,
        with_metadata: bool,
        return_equal: bool,
    ) -> Self {
        Self {
            engine: DiffEngine::from_fields(left, right, with_metadata),
            return_equal,
            yielded: false,
            marker: PhantomData,
        }
    }

    pub(crate) fn from_data_types(
        left: &'schema DataType,
        right: &'schema DataType,
        with_metadata: bool,
        return_equal: bool,
    ) -> Self {
        Self {
            engine: DiffEngine::from_data_types(left, right, with_metadata),
            return_equal,
            yielded: false,
            marker: PhantomData,
        }
    }
}

impl Iterator for Differences<'_> {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        next_with_equal(&mut self.engine, self.return_equal, &mut self.yielded)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        size_hint_with_equal(&self.engine, self.return_equal, self.yielded)
    }
}

impl FusedIterator for Differences<'_> {}

/// Yield the engine's next line, or the equal line once when asked.
fn next_with_equal(
    engine: &mut DiffEngine,
    return_equal: bool,
    yielded: &mut bool,
) -> Option<String> {
    if let Some(difference) = engine.next() {
        *yielded = true;
        return Some(difference);
    }
    if return_equal && !*yielded {
        *yielded = true;
        return Some(EQUAL_LINE.to_owned());
    }
    None
}

/// Widen an engine's upper bound when the equal line may still be produced.
fn size_hint_with_equal(
    engine: &DiffEngine,
    return_equal: bool,
    yielded: bool,
) -> (usize, Option<usize>) {
    let (lower, upper) = engine.size_hint();
    if return_equal && !yielded {
        return (lower, upper.and_then(|upper| upper.checked_add(1)));
    }
    (lower, upper)
}

impl OwnedDifferences {
    /// Creates an owning lazy cursor over two Field snapshots.
    pub fn from_fields(
        left: &Field,
        right: &Field,
        with_metadata: bool,
        return_equal: bool,
    ) -> Self {
        Self {
            engine: DiffEngine::from_fields(left, right, with_metadata),
            return_equal,
            yielded: false,
        }
    }

    /// Creates an owning lazy cursor over two DataType snapshots.
    pub fn from_data_types(
        left: &DataType,
        right: &DataType,
        with_metadata: bool,
        return_equal: bool,
    ) -> Self {
        Self {
            engine: DiffEngine::from_data_types(left, right, with_metadata),
            return_equal,
            yielded: false,
        }
    }
}

impl Iterator for OwnedDifferences {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        next_with_equal(&mut self.engine, self.return_equal, &mut self.yielded)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        size_hint_with_equal(&self.engine, self.return_equal, self.yielded)
    }
}

impl FusedIterator for OwnedDifferences {}

pub(crate) fn fields_equal(left: &Field, right: &Field, with_metadata: bool) -> bool {
    if std::ptr::eq(left, right) {
        return true;
    }
    if with_metadata {
        return left == right;
    }
    left.name == right.name
        && data_types_equal(&left.data_type, &right.data_type, false)
        && left.nullable == right.nullable
        && left.dictionary_id == right.dictionary_id
        && left.dictionary_is_ordered == right.dictionary_is_ordered
}

#[allow(clippy::too_many_lines)]
pub(crate) fn data_types_equal(left: &DataType, right: &DataType, with_metadata: bool) -> bool {
    if std::ptr::eq(left, right) {
        return true;
    }
    if with_metadata {
        return left == right;
    }
    use DataType as D;
    match (left, right) {
        (D::List(left), D::List(right))
        | (D::ListView(left), D::ListView(right))
        | (D::LargeList(left), D::LargeList(right))
        | (D::LargeListView(left), D::LargeListView(right)) => fields_equal(left, right, false),
        (D::FixedSizeList(left, left_size), D::FixedSizeList(right, right_size)) => {
            left_size == right_size && fields_equal(left, right, false)
        }
        (D::Struct(left), D::Struct(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right.iter())
                    .all(|(left, right)| fields_equal(left, right, false))
        }
        (D::Union(left, left_mode), D::Union(right, right_mode)) => {
            left_mode == right_mode
                && left.len() == right.len()
                && left
                    .iter()
                    .zip(right.iter())
                    .all(|((left_id, left), (right_id, right))| {
                        left_id == right_id && fields_equal(left, right, false)
                    })
        }
        (D::Dictionary(left), D::Dictionary(right)) => {
            data_types_equal(left.key(), right.key(), false)
                && data_types_equal(left.value(), right.value(), false)
        }
        (D::Map(left), D::Map(right)) => {
            left.keys_sorted() == right.keys_sorted()
                && fields_equal(left.entries(), right.entries(), false)
        }
        (D::RunEndEncoded(left), D::RunEndEncoded(right)) => {
            fields_equal(left.run_ends(), right.run_ends(), false)
                && fields_equal(left.values(), right.values(), false)
        }
        _ => left == right,
    }
}

pub(crate) fn show_diff(differences: Differences<'_>) -> String {
    let mut output = String::new();
    for difference in differences {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&difference);
    }
    output
}

fn property_path(path: &str, property: &str) -> String {
    format!("{path}.{property}")
}

fn indexed_path(path: &str, property: &str, index: usize) -> String {
    format!("{path}.{property}[{index}]")
}

fn metadata_path(path: &str, key: &str) -> String {
    format!("{path}.metadata[{key:?}]")
}

pub(crate) fn push_field_name_path(path: &mut String, name: &str) {
    crate::path::push_field_name(path, name);
}

fn changed_display(
    path: &str,
    left: impl std::fmt::Display,
    right: impl std::fmt::Display,
) -> String {
    format!("≠ {path}: {left} → {right}")
}

fn changed_debug(path: &str, left: impl std::fmt::Debug, right: impl std::fmt::Debug) -> String {
    format!("≠ {path}: {left:?} → {right:?}")
}

fn removed_display(path: &str, value: impl std::fmt::Display) -> String {
    format!("− {path}: {value}")
}

fn removed_debug(path: &str, value: impl std::fmt::Debug) -> String {
    format!("− {path}: {value:?}")
}

fn added_display(path: &str, value: impl std::fmt::Display) -> String {
    format!("+ {path}: {value}")
}

fn added_debug(path: &str, value: impl std::fmt::Debug) -> String {
    format!("+ {path}: {value:?}")
}

fn field_diff_value(field: &Field, with_metadata: bool) -> String {
    if with_metadata {
        return field.to_string();
    }
    let data_type = if field.data_type().is_nested() {
        field.data_type().name().to_owned()
    } else {
        field.data_type().to_string()
    };
    let mut value = format!(
        "field(name={:?},data_type={data_type},nullable={}",
        field.name(),
        field.is_nullable()
    );
    if let Some(dictionary_id) = field.dictionary_id() {
        value.push_str(&format!(",dictionary_id={dictionary_id}"));
    }
    if field.dictionary_is_ordered() == Some(true) {
        value.push_str(",dictionary_is_ordered=true");
    }
    value.push(')');
    value
}

#[cfg(test)]
mod tests {
    use super::{Differences, OwnedDifferences};
    use crate::{DataType, Field};

    fn wide_struct(prefix: &str) -> DataType {
        DataType::from_fields(
            (0..1_024)
                .map(|index| Field::new(format!("{prefix}_{index:04}"), DataType::Int64, false)),
        )
        .unwrap()
    }

    #[test]
    fn wide_slice_work_stays_bounded_before_the_first_difference() {
        let left = Field::new("root", wide_struct("left"), false);
        let right = Field::new("root", wide_struct("right"), false);
        let mut differences = Differences::from_fields(&left, &right, true, false);
        assert_eq!(differences.engine.work.len(), 1);
        assert!(differences.engine.pending.is_empty());

        assert_eq!(
            differences.next().as_deref(),
            Some("≠ $.data_type.fields[0].name: \"left_0000\" → \"right_0000\"")
        );
        assert!(differences.engine.work.len() <= 2);
        assert!(differences.engine.pending.is_empty());

        let left = Field::new(
            "root",
            DataType::from_fields(std::iter::empty()).unwrap(),
            false,
        );
        let right = Field::new("root", wide_struct("added"), false);
        let mut differences = Differences::from_fields(&left, &right, true, false);
        assert_eq!(
            differences.next().as_deref(),
            Some("≠ $.data_type.field_count: 0 → 1024")
        );
        assert_eq!(differences.engine.work.len(), 1);
        assert!(differences.engine.pending.is_empty());
    }

    #[test]
    fn physical_first_difference_does_not_scan_equal_wide_metadata() {
        let entries = (0..1_024)
            .map(|index| (format!("key_{index:04}"), format!("value_{index:04}")))
            .collect::<Vec<_>>();
        let left = Field::from_parts("root", DataType::Int32, false, entries.clone()).unwrap();
        let right = Field::from_parts("root", DataType::Int64, false, entries).unwrap();
        assert!(!left.metadata.shares_storage_with(&right.metadata));

        let mut differences = Differences::from_fields(&left, &right, true, false);
        assert_eq!(differences.engine.work.len(), 1);
        assert_eq!(
            differences.next().as_deref(),
            Some("≠ $.data_type.kind: int32 → int64")
        );
        assert_eq!(differences.engine.work.len(), 1);
        assert!(differences.engine.pending.is_empty());
    }

    #[test]
    fn shared_deep_snapshots_complete_without_traversal() {
        let mut data_type = DataType::Int64;
        for depth in 0..64 {
            data_type = DataType::list(Field::new(format!("item_{depth}"), data_type, false));
        }
        let left = Field::new("root", data_type, false);
        let right = left.clone();
        let mut differences = Differences::from_fields(&left, &right, true, false);
        assert!(differences.engine.work.is_empty());
        assert_eq!(differences.next(), None);
    }

    #[test]
    fn owned_cursor_outlives_source_snapshots() {
        let mut differences = {
            let left = Field::new("left", wide_struct("left"), false);
            let right = Field::new("right", wide_struct("right"), false);
            OwnedDifferences::from_fields(&left, &right, true, false)
        };
        assert_eq!(
            differences.next().as_deref(),
            Some("≠ $.name: \"left\" → \"right\"")
        );
        assert_eq!(
            differences.next().as_deref(),
            Some("≠ $.data_type.fields[0].name: \"left_0000\" → \"right_0000\"")
        );
    }
}
