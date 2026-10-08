//! Keyed series: one record key beside rows, held groups and lazy groups.

use std::fmt;
use std::iter::FusedIterator;
use std::ops::Index;
use std::sync::{Arc, Mutex, OnceLock};

use smol_str::{SmolStr, format_smolstr};

use crate::expression::{BoundSelector, IntoSelector, Projection, Term};
use crate::{
    ChunkedSerie, DataType, Error, Field, FieldPath, Result, Scalar, Selector, Serie, SerieValue,
    StreamChunkedSerie, StructType,
};

/// The one clustering intake: expressions or an external typed key column.
#[derive(Clone, Debug)]
pub enum KeyBy {
    /// Expressions bound once against the rows' record field.
    Selector(Selector),
    /// An external column, or record of columns, as long as the rows.
    External(Serie),
}

/// Read a clustering argument into its native spelling.
pub trait IntoKeyBy {
    /// Resolve text or retain an already typed selector or external column.
    ///
    /// # Errors
    /// Returns the selector's parse error.
    fn into_key_by(self) -> Result<KeyBy>;
}

impl IntoKeyBy for KeyBy {
    fn into_key_by(self) -> Result<KeyBy> {
        Ok(self)
    }
}

impl IntoKeyBy for &KeyBy {
    fn into_key_by(self) -> Result<KeyBy> {
        Ok(self.clone())
    }
}

macro_rules! selector_intake {
    ($($kind:ty),+ $(,)?) => {$(
        impl IntoKeyBy for $kind {
            fn into_key_by(self) -> Result<KeyBy> {
                Ok(KeyBy::Selector(self.into_selector()?))
            }
        }
    )+};
}
selector_intake!(
    &str, String, &String, Selector, &Selector, Projection, Term, FieldPath
);

impl<S: AsRef<str>, const N: usize> IntoKeyBy for [S; N] {
    fn into_key_by(self) -> Result<KeyBy> {
        Ok(KeyBy::Selector(self.into_selector()?))
    }
}

impl IntoKeyBy for &[FieldPath] {
    fn into_key_by(self) -> Result<KeyBy> {
        Ok(KeyBy::Selector(Selector::new(
            self.iter().cloned().map(Projection::from),
        )))
    }
}

impl<const N: usize> IntoKeyBy for &[FieldPath; N] {
    fn into_key_by(self) -> Result<KeyBy> {
        self.as_slice().into_key_by()
    }
}

impl IntoKeyBy for Vec<FieldPath> {
    fn into_key_by(self) -> Result<KeyBy> {
        self.as_slice().into_key_by()
    }
}

impl IntoKeyBy for &Vec<FieldPath> {
    fn into_key_by(self) -> Result<KeyBy> {
        self.as_slice().into_key_by()
    }
}

impl IntoKeyBy for &Serie {
    fn into_key_by(self) -> Result<KeyBy> {
        Ok(KeyBy::External(self.clone()))
    }
}

impl IntoKeyBy for &ChunkedSerie {
    fn into_key_by(self) -> Result<KeyBy> {
        Ok(KeyBy::External(Serie::from(self.clone())))
    }
}

/// A clustering layout shared by its items; no row values are stored here.
#[derive(Clone, Debug)]
pub struct KeyLayout {
    pub(crate) key_field: Arc<Field>,
    pub(crate) key_paths: Arc<[Option<FieldPath>]>,
    pub(crate) serie_field: Arc<Field>,
    pub(crate) field: Arc<Field>,
    remaining: Arc<[usize]>,
}

impl KeyLayout {
    fn new(
        root: &Field,
        keys: Field,
        paths: Vec<Option<FieldPath>>,
        moved: &[usize],
    ) -> Result<Arc<Self>> {
        let remaining: Vec<usize> = (0..root.field_len())
            .filter(|index| !moved.contains(index))
            .collect();
        let removed: Vec<&str> = moved
            .iter()
            .map(|index| root.fields()[*index].name())
            .collect();
        let mut rows = root
            .without_fields(&removed)?
            .with_metadata_removed("SORT:by");
        if let Some(order) = root.as_sort().by()? {
            let order: Vec<_> =
                order
                    .into_iter()
                    .filter(|ordering| {
                        !ordering.term().columns().iter().any(|column| {
                            removed.iter().any(|name| name.eq_ignore_ascii_case(column))
                        })
                    })
                    .collect();
            if !order.is_empty() {
                rows.as_sort_mut().set_by(order)?;
            }
        }
        for key in keys.fields() {
            if let Some(child) = rows
                .fields()
                .iter()
                .find(|child| child.name().eq_ignore_ascii_case(key.name()))
            {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new(root.name()),
                    reason: format_smolstr!(
                        "the key cell {:?} collides with the remaining child {:?}; alias the key cell (`... as <name>`)",
                        key.name(),
                        child.name()
                    ),
                });
            }
        }
        let global = root
            .clone()
            .with_metadata_removed("SORT:by")
            .try_with_dtype(DataType::from(StructType::from_fields(
                keys.fields().iter().chain(rows.fields()).cloned(),
            )?))?;
        let mut global = global;
        if let Some(order) = rows.as_sort().by()? {
            global.as_sort_mut().set_by(order)?;
        }
        // Aliasing a moved key can remove a name a partition declaration used.
        if let Some(by) = global.as_partition().by()? {
            let kept: Vec<_> = by
                .into_iter()
                .filter(|projection| projection.field(&global).is_ok())
                .collect();
            global = global.with_metadata_removed("PARTITION:by");
            if !kept.is_empty() {
                global.as_partition_mut().set_by(kept)?;
            }
        }
        Ok(Arc::new(Self {
            // A key is always a record run; an absent payload is represented
            // by absent key cells, never by an absent key record.
            key_field: Arc::new(keys.with_nullable(false)),
            key_paths: paths.into(),
            serie_field: Arc::new(rows),
            field: Arc::new(global),
            remaining: remaining.into(),
        }))
    }

    /// Collections clear order across items; the payload still declares
    /// the order that holds within each item and feeds further composition.
    fn item_field(&self) -> Result<Field> {
        let mut field = self.field.as_ref().clone();
        if let Some(order) = self.serie_field.as_sort().by()? {
            field.as_sort_mut().set_by(order)?;
        }
        Ok(field)
    }

    pub(crate) fn collection(&self) -> Arc<Self> {
        Arc::new(Self {
            field: Arc::new(self.field.as_ref().clone().with_metadata_removed("SORT:by")),
            ..self.clone()
        })
    }

    pub(crate) fn split_row(&self, row: &Scalar) -> Scalar {
        if row.is_null() {
            return Scalar::Null;
        }
        let cells = row.sequence_rows().expect("canonical record rows");
        Scalar::from_sequence(self.remaining.iter().map(|index| cells[*index].clone()))
    }

    pub(crate) fn split_piece(&self, piece: Serie) -> Serie {
        let record = piece.as_struct().expect("the cutter holds record pieces");
        crate::StructSerie::new(
            Arc::clone(&self.serie_field),
            self.remaining
                .iter()
                .map(|index| record.children()[*index].clone())
                .collect(),
            record.nulls().cloned(),
            piece.len(),
        )
        .into_serie()
    }

    pub(crate) fn global_piece(
        &self,
        literals: &mut KeyLiterals,
        rows: Serie,
        field: &Arc<Field>,
    ) -> crate::arrow::Result<Serie> {
        let record = rows
            .as_struct()
            .expect("key payloads have their record field");
        let mut children = Vec::with_capacity(field.field_len());
        children.extend(literals.columns(&self.key_field, rows.len())?);
        children.extend_from_slice(record.children());
        Ok(crate::StructSerie::new(
            Arc::clone(field),
            children,
            record.nulls().cloned(),
            rows.len(),
        )
        .into_serie())
    }
}

/// Held items lay out at their longest chunk; lazy items grow geometrically.
pub(crate) struct KeyLiterals {
    key: Scalar,
    length: usize,
    columns: Vec<Serie>,
}

impl KeyLiterals {
    fn new(key: Scalar, length: usize) -> Self {
        Self {
            key,
            length,
            columns: Vec::new(),
        }
    }

    fn columns(&mut self, field: &Field, length: usize) -> Result<Vec<Serie>> {
        if self.columns.is_empty() || length > self.length {
            self.length = self.length.saturating_mul(2).max(length);
            let cells = self.key.sequence_rows().expect("keys are record runs");
            self.columns = field
                .fields()
                .iter()
                .zip(cells.iter())
                .map(|(child, value)| {
                    Ok(
                        Serie::lit(Arc::new(child.clone()), value.clone(), self.length)?
                            .laid_out()
                            .clone(),
                    )
                })
                .collect::<Result<_>>()?;
        }
        self.columns
            .iter()
            .map(|column| column.slice(0, length))
            .collect()
    }
}

/// The key is bound once; chunks only supply values and positions.
#[derive(Clone)]
pub(crate) struct KeyPlan {
    pub(crate) layout: Arc<KeyLayout>,
    input: KeyInput,
}

#[derive(Clone)]
enum KeyInput {
    Bound(Arc<BoundSelector>),
    External(Serie),
    Prefixed { inner: Box<KeyPlan> },
}

impl KeyPlan {
    pub(crate) fn into_bound(self) -> Result<Arc<BoundSelector>> {
        match self.input {
            KeyInput::Bound(bound) => Ok(bound),
            KeyInput::External(_) | KeyInput::Prefixed { .. } => Err(Error::InvalidRecord {
                path: SmolStr::new(self.layout.field.name()),
                reason: SmolStr::new_static(
                    "external keys need held rows; use a selector for StreamKeySerie",
                ),
            }),
        }
    }
    pub(crate) fn bind(root: &Field, by: KeyBy, len: Option<usize>, verb: &str) -> Result<Self> {
        let (input, keys, paths, moved) = match by {
            KeyBy::Selector(selector) => {
                let bound = selector.bind_key(root, root.name(), verb)?;
                let mut paths = Vec::with_capacity(selector.projections().len());
                let mut moved = Vec::new();
                for projection in selector.expanded(root) {
                    let bare = projection.dtype().is_none()
                        && projection.metadata().is_empty()
                        && projection.nullable().is_none();
                    paths.push(if bare {
                        projection
                            .term()
                            .as_path()
                            .map(|steps| FieldPath::new(steps.to_vec()))
                    } else {
                        None
                    });
                    if bare
                        && let Some(name) = projection.term().as_column()
                        && let Some(index) = crate::expression::child_position(
                            root.fields().iter().map(Field::name),
                            name,
                        )
                        && !moved.contains(&index)
                    {
                        moved.push(index);
                    }
                }
                let keys = bound.output().clone();
                (KeyInput::Bound(Arc::new(bound)), keys, paths, moved)
            }
            KeyBy::External(column) => {
                let Some(len) = len else {
                    return Err(Error::InvalidRecord {
                        path: SmolStr::new(root.name()),
                        reason: SmolStr::new_static(
                            "external keys need held rows; use a selector for StreamKeySerie",
                        ),
                    });
                };
                column.raise_held_rows()?;
                if column.len() != len {
                    return Err(Error::InvalidRecord {
                        path: SmolStr::new(root.name()),
                        reason: format_smolstr!(
                            "{} keys cannot {verb} the {len} rows {} holds",
                            column.len(),
                            root.name()
                        ),
                    });
                }
                let field = column.require_field()?;
                let children = if field.dtype().as_fields().is_some() {
                    field
                        .fields()
                        .iter()
                        .map(|child| {
                            child
                                .clone()
                                .with_nullable(child.is_nullable() || field.is_nullable())
                        })
                        .collect::<Vec<_>>()
                } else {
                    vec![field.clone()]
                };
                let keys =
                    DataType::from(StructType::from_fields(children)?).required_field(root.name());
                let paths = vec![None; keys.field_len()];
                (KeyInput::External(column), keys, paths, Vec::new())
            }
        };
        Ok(Self {
            layout: KeyLayout::new(root, keys, paths, &moved)?,
            input,
        })
    }

    pub(crate) fn keys(&self, rows: &Serie, offset: usize) -> Result<Serie> {
        match &self.input {
            KeyInput::Bound(bound) => bound.apply_serie(rows),
            KeyInput::External(column) => {
                let piece = match column.held_chunks() {
                    Some(chunks) => {
                        let slice = chunks.slice(offset, rows.len())?;
                        match slice.chunks() {
                            [piece] => piece.clone(),
                            _ => slice.into_serie()?,
                        }
                    }
                    None => column.slice(offset, rows.len())?,
                };
                record_piece(piece, Arc::clone(&self.layout.key_field))
            }
            KeyInput::Prefixed { inner } => {
                let keys = inner.keys(rows, offset)?;
                let record = keys.as_struct().expect("a key column is a record");
                let count = self.layout.key_field.field_len() - inner.layout.key_field.field_len();
                let mut children = rows
                    .as_struct()
                    .expect("global key rows are records")
                    .children()[..count]
                    .to_vec();
                children.extend_from_slice(record.children());
                Ok(crate::StructSerie::new(
                    Arc::clone(&self.layout.key_field),
                    children,
                    record.nulls().cloned(),
                    rows.len(),
                )
                .into_serie())
            }
        }
    }

    fn with_prefix(self, root: &Field, outer: &KeyLayout) -> Result<Self> {
        let layout = prefixed_layout(root, outer, &self.layout)?;
        Ok(Self {
            layout,
            input: KeyInput::Prefixed {
                inner: Box::new(self),
            },
        })
    }

    fn nullable_keys(&mut self) -> Result<()> {
        let keys = DataType::from(StructType::from_fields(
            self.layout
                .key_field
                .fields()
                .iter()
                .map(|child| child.clone().with_nullable(true)),
        )?)
        .required_field(self.layout.key_field.name());
        let global = self
            .layout
            .field
            .as_ref()
            .clone()
            .try_with_dtype(DataType::from(StructType::from_fields(
                keys.fields()
                    .iter()
                    .chain(self.layout.serie_field.fields())
                    .cloned(),
            )?))?;
        self.layout = Arc::new(KeyLayout {
            key_field: Arc::new(keys),
            field: Arc::new(global),
            ..self.layout.as_ref().clone()
        });
        Ok(())
    }
}

fn record_piece(piece: Serie, root: Arc<Field>) -> Result<Serie> {
    if piece.require_field()?.dtype().as_fields().is_some() {
        let record = piece
            .as_struct()
            .expect("a record column has struct storage");
        Ok(crate::StructSerie::new(
            root,
            record.children().to_vec(),
            record.nulls().cloned(),
            piece.len(),
        )
        .into_serie())
    } else {
        let len = piece.len();
        Ok(crate::StructSerie::new(root, vec![piece], None, len).into_serie())
    }
}

fn key_record(value: Scalar, layout: &KeyLayout) -> Scalar {
    if value.is_null() {
        Scalar::from_sequence(vec![Scalar::Null; layout.key_field.field_len()])
    } else {
        value
    }
}

impl Serie {
    /// Group held rows by one key, in order of first occurrence.
    ///
    /// # Errors
    /// Refuses an untyped run, an invalid key, a collision with a remaining
    /// child (alias the key cell), or a failed source.
    pub fn partition_by(&self, by: impl IntoKeyBy) -> Result<KeySeries> {
        cluster(self, by.into_key_by()?, None)
    }

    /// Cut adjacent equal keys; `sorted` gathers the runs in ascending order.
    ///
    /// # Errors
    /// The same refusals as [`Self::partition_by`].
    pub fn window_by(&self, by: impl IntoKeyBy, sorted: bool) -> Result<KeySeries> {
        cluster(self, by.into_key_by()?, Some(sorted))
    }
}

impl ChunkedSerie {
    /// Group rows without joining the chunks.
    ///
    /// # Errors
    /// [`Serie::partition_by`]'s refusals.
    pub fn partition_by(&self, by: impl IntoKeyBy) -> Result<KeySeries> {
        Serie::from(self.clone()).partition_by(by)
    }
    /// Cut windows across chunk edges without joining the chunks.
    ///
    /// # Errors
    /// [`Serie::window_by`]'s refusals.
    pub fn window_by(&self, by: impl IntoKeyBy, sorted: bool) -> Result<KeySeries> {
        Serie::from(self.clone()).window_by(by, sorted)
    }
}

fn cluster(rows: &Serie, by: KeyBy, windows: Option<bool>) -> Result<KeySeries> {
    cluster_prefixed(rows, by, windows, None)
}

fn cluster_prefixed(
    rows: &Serie,
    by: KeyBy,
    windows: Option<bool>,
    outer: Option<&KeySerie>,
) -> Result<KeySeries> {
    rows.raise_held_rows()?;
    let field = rows.require_field()?;
    let root = Arc::new(if field.dtype().as_fields().is_some() {
        field.clone()
    } else {
        StreamChunkedSerie::root_of(field)?
    });
    let mut plan = KeyPlan::bind(
        &root,
        by,
        Some(rows.len()),
        if windows.is_some() {
            "window by"
        } else {
            "partition by"
        },
    )?;
    if let Some(outer) = outer {
        plan = plan.with_prefix(&root, &outer.layout)?;
    }
    cluster_planned(rows, root, plan, windows, outer, 0)
}

fn cluster_planned(
    rows: &Serie,
    root: Arc<Field>,
    mut plan: KeyPlan,
    windows: Option<bool>,
    outer: Option<&KeySerie>,
    key_offset: usize,
) -> Result<KeySeries> {
    let pieces = rows
        .held_chunks()
        .map_or_else(|| vec![rows.clone()], |chunks| chunks.chunks().to_vec());
    let pieces = pieces
        .into_iter()
        .map(|piece| record_piece(piece, Arc::clone(&root)))
        .collect::<Result<Vec<_>>>()?;
    if pieces.iter().any(|piece| piece.null_count() > 0) {
        plan.nullable_keys()?;
    }
    let mut groups: Vec<(Scalar, Vec<Serie>, Option<u64>)> = Vec::new();
    let mut offset = 0;
    if let Some(sorted) = windows {
        let mut descent = false;
        for piece in pieces {
            let keys = plan.keys(&piece, key_offset + offset)?;
            let cut = keys.window_starts();
            descent |= cut.descent.is_some();
            let payload = plan.layout.split_piece(piece);
            let mut start = 0;
            while start < keys.len() {
                let end = crate::window_serie::window_end(&cut.starts, start);
                let edge = match groups.last() {
                    Some(group) if start == 0 => {
                        keys.compare_to_row(&group.0, start, crate::SortOptions::default())
                    }
                    _ => std::cmp::Ordering::Less,
                };
                descent |= edge == std::cmp::Ordering::Greater;
                let rows = payload.slice(start, end - start)?;
                if edge == std::cmp::Ordering::Equal
                    && let Some(group) = groups.last_mut()
                {
                    group.1.push(rows);
                } else {
                    groups.push((
                        keys.scalar(start)?,
                        vec![rows],
                        Some((offset + start) as u64),
                    ));
                }
                start = end;
            }
            offset += keys.len();
        }
        if sorted && descent {
            groups.sort_by(|left, right| {
                crate::serie::compare_values(&left.0, &right.0, crate::SortOptions::default())
            });
            let mut gathered: Vec<(Scalar, Vec<Serie>, Option<u64>)> = Vec::new();
            for (key, pieces, _) in groups {
                if let Some(last) = gathered.last_mut()
                    && last.0 == key
                {
                    last.1.extend(pieces);
                } else {
                    gathered.push((key, pieces, None));
                }
            }
            groups = gathered;
        }
    } else {
        #[allow(clippy::mutable_key_type)]
        let mut positions = std::collections::HashMap::new();
        for piece in pieces {
            let keys = plan.keys(&piece, key_offset + offset)?;
            for (key, payload) in plan.layout.split_piece(piece).cut_partitions(&keys)? {
                let next = groups.len();
                let position = *positions.entry(key.clone()).or_insert(next);
                if position == next {
                    groups.push((key, Vec::new(), None));
                }
                groups[position].1.push(payload);
            }
            offset += keys.len();
        }
    }
    let items = groups
        .into_iter()
        .map(|(key, pieces, rownum)| {
            let mut pieces = pieces.into_iter();
            let first = pieces.next().expect("a group has at least one piece");
            let rows = if let Some(second) = pieces.next() {
                Serie::from(ChunkedSerie::from_landed(
                    Arc::clone(&plan.layout.serie_field),
                    std::iter::once(first)
                        .chain(std::iter::once(second))
                        .chain(pieces)
                        .collect(),
                ))
            } else {
                first
            };
            KeySerie::new(
                key_record(key, &plan.layout),
                Arc::clone(&plan.layout),
                rows,
                rownum,
            )
        })
        .collect();
    let mut answer = KeySeries::new(plan.layout, items);
    if let Some(outer) = outer {
        if let Some(base) = outer.rownum {
            answer.shift_rownum(base)?;
        } else {
            for item in &mut answer.items {
                item.rownum = None;
            }
        }
    }
    Ok(answer)
}

fn prefixed_layout(root: &Field, outer: &KeyLayout, inner: &KeyLayout) -> Result<Arc<KeyLayout>> {
    for key in inner.key_field.fields() {
        if outer
            .key_field
            .fields()
            .iter()
            .any(|field| field.name().eq_ignore_ascii_case(key.name()))
        {
            return Err(Error::InvalidRecord {
                path: SmolStr::new(root.name()),
                reason: format_smolstr!(
                    "the key cell {:?} collides with an outer key; alias the key cell (`... as <name>`)",
                    key.name()
                ),
            });
        }
    }
    let keys = DataType::from(StructType::from_fields(
        outer
            .key_field
            .fields()
            .iter()
            .chain(inner.key_field.fields())
            .cloned(),
    )?)
    .required_field(root.name());
    let paths = outer
        .key_paths
        .iter()
        .chain(inner.key_paths.iter())
        .cloned()
        .collect();
    let moved: Vec<_> = (0..root.field_len())
        .filter(|index| *index < outer.key_field.field_len() || !inner.remaining.contains(index))
        .collect();
    KeyLayout::new(root, keys, paths, &moved)
}

/// One record key beside its rows, with their global field and source position.
#[derive(Clone)]
pub struct KeySerie {
    key: Scalar,
    pub(crate) layout: Arc<KeyLayout>,
    rows: Serie,
    rownum: Option<u64>,
    held: Arc<OnceLock<(ChunkedSerie, Option<SmolStr>)>>,
}

impl KeySerie {
    fn compose(&self, by: KeyBy, windows: Option<bool>) -> Result<KeySeries> {
        if !self.rows.is_held() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new(self.field().name()),
                reason: SmolStr::new_static(
                    "this key's rows are streaming; compose through StreamKeySerie",
                ),
            });
        }
        let (rows, failure) = self.global_chunks();
        if let Some(reason) = failure {
            return Err(crate::shared_stream::failed(self.field(), reason).into());
        }
        cluster_prefixed(&Serie::from(rows.clone()), by, windows, Some(self))
    }

    /// Compose another key on held global rows, retaining outer key cells.
    ///
    /// # Errors
    /// Refuses streaming payloads (use `StreamKeySerie`), invalid keys and collisions.
    pub fn partition_by(&self, by: impl IntoKeyBy) -> Result<KeySeries> {
        self.compose(by.into_key_by()?, None)
    }
    /// Compose windows, adding their position to this item's source position.
    ///
    /// # Errors
    /// [`Self::partition_by`]'s refusals, and a row number exceeding uint64.
    pub fn window_by(&self, by: impl IntoKeyBy, sorted: bool) -> Result<KeySeries> {
        self.compose(by.into_key_by()?, Some(sorted))
    }
    pub(crate) fn hold_rows(&mut self) -> crate::arrow::Result<()> {
        self.rows.raise_held_rows()?;
        if let Some(chunks) = self.rows.held_chunks() {
            self.rows = Serie::from(chunks.clone());
        }
        Ok(())
    }

    pub(crate) fn spill_rows(&mut self, options: &crate::SpillOptions) -> Result<()> {
        self.rows.spill(options)?;
        self.held = Arc::new(OnceLock::new());
        Ok(())
    }
    pub(crate) fn new(
        key: Scalar,
        layout: Arc<KeyLayout>,
        rows: Serie,
        rownum: Option<u64>,
    ) -> Self {
        Self {
            key,
            layout,
            rows,
            rownum,
            held: Arc::new(OnceLock::new()),
        }
    }

    /// The record run of cells shared by every row of this item.
    pub const fn key(&self) -> &Scalar {
        &self.key
    }
    /// The field of the key record.
    pub fn key_field(&self) -> &Field {
        &self.layout.key_field
    }
    /// The source path of each key cell; computed and external cells have none.
    pub fn key_paths(&self) -> &[Option<FieldPath>] {
        &self.layout.key_paths
    }
    /// The rows' field, with moved key columns removed.
    pub fn serie_field(&self) -> &Field {
        &self.layout.serie_field
    }
    /// The global field: key cells followed by the rows' children.
    pub fn field(&self) -> &Field {
        &self.layout.field
    }
    /// The payload, which may be held, chunked or streaming.
    pub const fn rows(&self) -> &Serie {
        &self.rows
    }
    /// The first source row; partitions and gathered windows have no position.
    pub const fn rownum(&self) -> Option<u64> {
        self.rownum
    }
    /// Take the key and payload, retaining no stream handle.
    pub fn into_parts(self) -> (Scalar, Serie) {
        (self.key, self.rows)
    }
    /// Bytes held by the key and payload; asking never pulls a stream.
    pub fn memory_size(&self) -> usize {
        crate::arrow::scalar_memory_size(&self.key).saturating_add(self.rows.memory_size())
    }

    pub(crate) fn global_chunks(&self) -> &(ChunkedSerie, Option<SmolStr>) {
        self.held.get_or_init(|| {
            let failure = self
                .rows
                .raise_held_rows()
                .err()
                .map(|error| format_smolstr!("{error}"));
            let pieces = self
                .rows
                .held_chunks()
                .map_or_else(|| vec![self.rows.clone()], |rows| rows.chunks().to_vec());
            let mut chunks = ChunkedSerie::from_landed(Arc::clone(&self.layout.field), Vec::new());
            let mut failure = failure;
            let mut literals = KeyLiterals::new(
                self.key.clone(),
                pieces.iter().map(Serie::len).max().unwrap_or(0),
            );
            for piece in pieces {
                match self
                    .layout
                    .global_piece(&mut literals, piece, &self.layout.field)
                    .and_then(|piece| chunks.push_landed(piece).map_err(Into::into))
                {
                    Ok(()) => {}
                    Err(error) => {
                        failure.get_or_insert_with(|| format_smolstr!("{error}"));
                        break;
                    }
                }
            }
            (chunks, failure)
        })
    }

    pub(crate) fn record_stream(self) -> crate::arrow::Result<StreamChunkedSerie> {
        let root = if self.layout.field.is_nullable() {
            Arc::new(StreamChunkedSerie::root_of(&self.layout.field)?)
        } else {
            Arc::clone(&self.layout.field)
        };
        let length = if self.rows.is_held() {
            self.rows.held_chunks().map_or_else(
                || self.rows.len(),
                |chunks| chunks.chunks().iter().map(Serie::len).max().unwrap_or(0),
            )
        } else {
            0
        };
        let rows = StreamChunkedSerie::from_serie(self.rows)?;
        let mut literals = KeyLiterals::new(self.key, length);
        let layout = self.layout;
        let field = Arc::clone(&root);
        StreamChunkedSerie::from_landed_iter(
            root,
            rows.into_chunks()
                .map(move |piece| layout.global_piece(&mut literals, piece?, &field)),
        )
    }

    pub(crate) fn bounded_record_stream(
        self,
        row_size: Option<usize>,
        byte_size: Option<u64>,
    ) -> crate::arrow::Result<StreamChunkedSerie> {
        self.record_stream()?
            .into_chunked_stream(row_size, byte_size)
    }
}

impl PartialEq for KeySerie {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.rows == other.rows
    }
}
impl Eq for KeySerie {}
impl fmt::Debug for KeySerie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeySerie")
            .field("key", &self.key)
            .field("field", &self.layout.field)
            .field("rows", &self.rows)
            .field("rownum", &self.rownum)
            .finish()
    }
}

/// Held keyed items, in the order the clustering verb answers them.
#[derive(Clone, Debug)]
pub struct KeySeries {
    pub(crate) layout: Arc<KeyLayout>,
    pub(crate) items: Vec<KeySerie>,
    held: Arc<OnceLock<(ChunkedSerie, Option<SmolStr>)>>,
}

impl KeySeries {
    fn compose(&self, by: KeyBy, windows: Option<bool>) -> Result<Self> {
        let total: usize = self.items.iter().map(|item| item.rows.len()).sum();
        let root = Arc::new(self.layout.item_field()?);
        // Every item has this same key/global schema. Resolve the selector
        // once, including external-key length and all collisions.
        let plan = KeyPlan::bind(&root, by, Some(total), "cluster by")?
            .with_prefix(&root, &self.layout)?;
        let layout = Arc::clone(&plan.layout);
        let mut offset = 0;
        let mut items = Vec::new();
        for item in &self.items {
            if !item.rows.is_held() {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new(item.field().name()),
                    reason: SmolStr::new_static(
                        "this key's rows are streaming; compose through StreamKeySerie",
                    ),
                });
            }
            let (rows, failure) = item.global_chunks();
            if let Some(reason) = failure {
                return Err(crate::shared_stream::failed(item.field(), reason).into());
            }
            let answer = cluster_planned(
                &Serie::from(rows.clone()),
                Arc::clone(&root),
                plan.clone(),
                windows,
                Some(item),
                offset,
            )?;
            offset += item.rows.len();
            items.extend(answer.items);
        }
        Ok(Self::new(layout, items))
    }
    /// Compose partitions per item in item order.
    ///
    /// # Errors
    /// [`KeySerie::partition_by`]'s refusals.
    pub fn partition_by(&self, by: impl IntoKeyBy) -> Result<Self> {
        self.compose(by.into_key_by()?, None)
    }
    /// Compose windows per item, keeping their absolute source positions.
    ///
    /// # Errors
    /// [`KeySerie::window_by`]'s refusals.
    pub fn window_by(&self, by: impl IntoKeyBy, sorted: bool) -> Result<Self> {
        self.compose(by.into_key_by()?, Some(sorted))
    }
    pub(crate) fn new(layout: Arc<KeyLayout>, items: Vec<KeySerie>) -> Self {
        Self {
            layout: layout.collection(),
            items,
            held: Arc::new(OnceLock::new()),
        }
    }
    /// The number of keyed items.
    pub fn len(&self) -> usize {
        self.items.len()
    }
    /// Whether there is no keyed item.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    /// One item, or none past the last.
    pub fn get(&self, index: usize) -> Option<&KeySerie> {
        self.items.get(index)
    }
    /// The items in their clustering order.
    pub fn iter(&self) -> std::slice::Iter<'_, KeySerie> {
        self.items.iter()
    }
    /// The key record's field, shared by every item.
    pub fn key_field(&self) -> &Field {
        &self.layout.key_field
    }
    /// The source path of each key cell.
    pub fn key_paths(&self) -> &[Option<FieldPath>] {
        &self.layout.key_paths
    }
    /// The payload field, shared by every item.
    pub fn serie_field(&self) -> &Field {
        &self.layout.serie_field
    }
    /// The global record field, declaring no order across items.
    pub fn field(&self) -> &Field {
        &self.layout.field
    }
    /// Bytes held by all items, without pulling any payload.
    pub fn memory_size(&self) -> usize {
        self.items.iter().map(KeySerie::memory_size).sum()
    }

    pub(crate) fn require_table_rows(&self) -> crate::arrow::Result<()> {
        for item in &self.items {
            if item.rows.is_held() {
                crate::serie::require_record_rows(&item.rows)?;
            }
        }
        Ok(())
    }

    pub(crate) fn record_stream(self) -> crate::arrow::Result<StreamChunkedSerie> {
        self.require_table_rows()?;
        StreamKeySerie::new(Arc::clone(&self.layout), self.items.into_iter().map(Ok))
            .record_stream()
    }

    pub(crate) fn shift_rownum(&mut self, base: u64) -> Result<()> {
        for item in &mut self.items {
            item.rownum = item
                .rownum
                .map(|row| {
                    base.checked_add(row).ok_or_else(|| Error::InvalidRecord {
                        path: SmolStr::new(item.field().name()),
                        reason: SmolStr::new_static("the window's rownum exceeds uint64"),
                    })
                })
                .transpose()?;
        }
        Ok(())
    }

    pub(crate) fn global_chunks(&self) -> &(ChunkedSerie, Option<SmolStr>) {
        self.held.get_or_init(|| {
            let mut chunks = ChunkedSerie::from_landed(Arc::clone(&self.layout.field), Vec::new());
            for item in &self.items {
                let (rows, failure) = item.global_chunks();
                for piece in rows.chunks() {
                    let record = piece.as_struct().expect("key items lower to records");
                    let piece = crate::StructSerie::new(
                        Arc::clone(&self.layout.field),
                        record.children().to_vec(),
                        record.nulls().cloned(),
                        piece.len(),
                    )
                    .into_serie();
                    if let Err(error) = chunks.push_landed(piece) {
                        return (chunks, Some(format_smolstr!("{error}")));
                    }
                }
                if let Some(failure) = failure {
                    return (chunks, Some(failure.clone()));
                }
            }
            (chunks, None)
        })
    }
}

impl Index<usize> for KeySeries {
    type Output = KeySerie;
    fn index(&self, index: usize) -> &KeySerie {
        &self.items[index]
    }
}
impl IntoIterator for KeySeries {
    type Item = KeySerie;
    type IntoIter = std::vec::IntoIter<KeySerie>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}
impl<'a> IntoIterator for &'a KeySeries {
    type Item = &'a KeySerie;
    type IntoIter = std::slice::Iter<'a, KeySerie>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl PartialEq for KeySeries {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
    }
}
impl Eq for KeySeries {}

/// Lazy keyed items under a layout available before the first pull.
pub struct StreamKeySerie {
    pub(crate) layout: Arc<KeyLayout>,
    source: Mutex<Option<Box<dyn Iterator<Item = crate::arrow::Result<KeySerie>> + Send>>>,
}

impl StreamKeySerie {
    fn compose(
        mut self,
        by: KeyBy,
        windows: Option<bool>,
        options: crate::PartitionOptions,
    ) -> crate::arrow::Result<Self> {
        let KeyBy::Selector(selector) = by else {
            return Err(Error::InvalidRecord {
                path: SmolStr::new(self.field().name()),
                reason: SmolStr::new_static(
                    "external keys need held rows; use a selector for StreamKeySerie",
                ),
            }
            .into());
        };
        let projections = selector.expanded(self.field());
        for projection in &projections {
            if self
                .key_field()
                .fields()
                .iter()
                .any(|field| field.name().eq_ignore_ascii_case(&projection.name()))
            {
                return Err(Error::InvalidRecord { path: SmolStr::new(self.field().name()), reason: format_smolstr!("the key cell {:?} collides with an outer key; alias the key cell (`... as <name>`)", projection.name()) }.into());
            }
        }
        let combined = Selector::new(
            self.key_field()
                .fields()
                .iter()
                .map(|field| Projection::column(field.name()))
                .chain(projections),
        );
        let root = self.layout.item_field()?;
        let mut plan = KeyPlan::bind(&root, KeyBy::Selector(combined.clone()), None, "cluster by")?;
        let outer_paths = self.key_paths().to_vec();
        let mut layout = plan.layout.as_ref().clone();
        let paths = outer_paths
            .iter()
            .cloned()
            .chain(layout.key_paths.iter().skip(outer_paths.len()).cloned())
            .collect::<Vec<_>>();
        layout.key_paths = paths.into();
        let layout = Arc::new(layout);
        let mut inner: Option<(StreamKeySerie, Option<u64>)> = None;
        plan.layout = Arc::clone(&layout);
        let item_layout = Arc::clone(&layout);
        Ok(Self::new(
            layout,
            std::iter::from_fn(move || {
                loop {
                    if let Some((source, base)) = &mut inner
                        && let Some(item) = source.next()
                    {
                        return Some(item.and_then(|mut item| {
                            item.rownum = match (item.rownum, *base) {
                                (Some(row), Some(base)) => {
                                    Some(base.checked_add(row).ok_or_else(|| {
                                        Error::InvalidRecord {
                                            path: SmolStr::new(item.field().name()),
                                            reason: SmolStr::new_static(
                                                "the window's rownum exceeds uint64",
                                            ),
                                        }
                                    })?)
                                }
                                _ => None,
                            };
                            let mut layout = item.layout.as_ref().clone();
                            layout.key_paths = Arc::clone(&item_layout.key_paths);
                            item.layout = Arc::new(layout);
                            Ok(item)
                        }));
                    }
                    inner = None;
                    let outer = match self.next()? {
                        Ok(item) => item,
                        Err(error) => return Some(Err(error)),
                    };
                    let base = outer.rownum;
                    let rows = match outer.into_stream() {
                        Ok(rows) => rows,
                        Err(error) => return Some(Err(error.into())),
                    };
                    let source = match windows {
                        Some(sorted) => rows.window_by_planned(plan.clone(), sorted),
                        None => rows.partition_by_planned(plan.clone(), &combined, options),
                    };
                    match source {
                        Ok(source) => inner = Some((source, base)),
                        Err(error) => return Some(Err(error)),
                    }
                }
            }),
        ))
    }
    /// Compose lazy windows within each item, outer key cells first.
    ///
    /// # Errors
    /// Invalid selectors are refused before pulling; source and position failures while pulling.
    pub fn window_by(self, by: impl IntoKeyBy, sorted: bool) -> crate::arrow::Result<Self> {
        self.compose(
            by.into_key_by()?,
            Some(sorted),
            crate::PartitionOptions::new(),
        )
    }
    /// Compose lazy partitions within each item, outer key cells first.
    ///
    /// # Errors
    /// [`Self::window_by`]'s refusals.
    pub fn partition_by(
        self,
        by: impl IntoKeyBy,
        options: crate::PartitionOptions,
    ) -> crate::arrow::Result<Self> {
        self.compose(by.into_key_by()?, None, options)
    }
    pub(crate) fn record_stream(self) -> crate::arrow::Result<StreamChunkedSerie> {
        self.record_stream_with_bounds(None)
    }

    pub(crate) fn record_stream_with_bounds(
        mut self,
        bounds: Option<(Option<usize>, Option<u64>)>,
    ) -> crate::arrow::Result<StreamChunkedSerie> {
        let root = if self.layout.field.is_nullable() {
            Arc::new(StreamChunkedSerie::root_of(&self.layout.field)?)
        } else {
            Arc::clone(&self.layout.field)
        };
        let field = Arc::clone(&root);
        let mut current: Option<crate::serie::IntoStreamChunks> = None;
        StreamChunkedSerie::from_landed_iter(
            root,
            std::iter::from_fn(move || {
                loop {
                    if let Some(chunks) = &mut current
                        && let Some(piece) = chunks.next()
                    {
                        return Some(piece.map(|piece| {
                            let record = piece.as_struct().expect("key items lower to records");
                            crate::StructSerie::new(
                                Arc::clone(&field),
                                record.children().to_vec(),
                                record.nulls().cloned(),
                                piece.len(),
                            )
                            .into_serie()
                        }));
                    }
                    current = None;
                    match self.next()? {
                        Ok(item) => match match bounds {
                            None => item.record_stream(),
                            Some((rows, bytes)) => item.into_chunked_stream(rows, bytes),
                        } {
                            Ok(rows) => current = Some(rows.into_chunks()),
                            Err(error) => return Some(Err(error)),
                        },
                        Err(error) => return Some(Err(error)),
                    }
                }
            }),
        )
    }
    pub(crate) fn new(
        layout: Arc<KeyLayout>,
        source: impl Iterator<Item = crate::arrow::Result<KeySerie>> + Send + 'static,
    ) -> Self {
        Self {
            layout: layout.collection(),
            source: Mutex::new(Some(Box::new(source))),
        }
    }
    /// The key record's field, available without pulling an item.
    pub fn key_field(&self) -> &Field {
        &self.layout.key_field
    }
    /// The source path of each key cell, available without a pull.
    pub fn key_paths(&self) -> &[Option<FieldPath>] {
        &self.layout.key_paths
    }
    /// The payload field, available without a pull.
    pub fn serie_field(&self) -> &Field {
        &self.layout.serie_field
    }
    /// The global field, declaring no order across items.
    pub fn field(&self) -> &Field {
        &self.layout.field
    }
}

impl Iterator for StreamKeySerie {
    type Item = crate::arrow::Result<KeySerie>;
    fn next(&mut self) -> Option<Self::Item> {
        let source = self
            .source
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let next = source.as_mut()?.next();
        if !matches!(next, Some(Ok(_))) {
            *source = None;
        }
        next
    }
}
impl FusedIterator for StreamKeySerie {}
impl fmt::Debug for StreamKeySerie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamKeySerie")
            .field("field", &self.layout.field)
            .finish_non_exhaustive()
    }
}
