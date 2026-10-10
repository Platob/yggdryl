//! Series retaining their medium and the scan clauses their own verbs
//! stated, until rows are requested. The medium holds the options - it
//! states them, or infers them from what it is and defaults the rest - and
//! a serie keeps no copy: every read asks the medium and lays its own
//! clauses over the answer.
//!
//! The one composition of a read lives here too: [`compose`] turns the
//! options a read runs under into the options its medium's native reader is
//! handed and the [`Residual`] that runs once over what that reader answers.
//! Every record read door - a leaf's, a folder's, a coded handle's, a media
//! serie's own - composes through it, so a clause is pushed down, kept or
//! dropped by one rule and applied exactly once.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use smol_str::SmolStr;

use crate::expression::{IntoFilter, IntoSelector};
use crate::media::{IORecordOptions, RecordOptions};
use crate::{
    Field, Filter, IOMedia, Result, Scalar, Selector, Serie, SerieValue, StreamChunkedSerie,
    StreamSerie,
};

type Read<T> = fn(&T, &RecordOptions) -> Result<Serie>;

/// What a media serie's own verbs stated over its medium's options, and
/// nothing the medium answered: the predicate `with_filter` and `with_key`
/// conjoined, the selection `with_select` restated, the row range
/// `with_row_range` set. A clause left unstated is the medium's own; the
/// default states nothing, and is what a read door outside a serie composes
/// under.
#[derive(Clone, Debug, Default)]
pub(crate) struct Scan {
    filter: Filter,
    select: Option<Selector>,
    range: Option<(u64, Option<u64>)>,
}

impl Scan {
    /// Lay the stated clauses over the medium's options: the predicate
    /// conjoined with the medium's, the selection and the range replacing
    /// them where stated.
    fn lay(&self, options: &mut RecordOptions) {
        if !self.filter.is_always_true() {
            options.set_filter(options.filter().clone().and(self.filter.clone()));
        }
        if let Some(select) = &self.select {
            options.set_select(select.clone());
        }
        if let Some((offset, length)) = self.range {
            options.set_row_offset(Some(offset));
            options.set_max_row_size(length);
        }
    }

    /// The options a read under this scan runs under: `medium`'s, the stated
    /// clauses laid over them, refused where a bound meets a merge key.
    fn folded(&self, medium: &RecordOptions) -> Result<RecordOptions> {
        let mut options = medium.clone();
        self.lay(&mut options);
        options.require_write_limits()?;
        Ok(options)
    }
}

/// One read composed: what its medium's native reader is handed, and what
/// runs once over the rows that reader answers.
pub(crate) struct Composed {
    /// The options the medium's native reader is handed: the declared root
    /// narrowed to the columns the read decodes, the `where` conjuncts over
    /// stored columns - every conjunct where the stored columns are unknown -
    /// with those a unit settles dropped, and the row bounds only where the
    /// rows it yields are the rows they count. Every other setting is the
    /// read's own.
    pub(crate) handed: RecordOptions,
    /// What the rows go through once, after the medium.
    pub(crate) residual: Residual,
}

/// What a read applies once over the rows its medium answered: the `where`
/// clause less the conjuncts the unit settles - the conjuncts over stored
/// columns before the selection, the ones naming what only it publishes
/// after, placed against the rows themselves, since the medium prunes by
/// the first and filters no row - then the selection, then the row skip and
/// the row and byte bounds, exact. A medium reads a bound as a fetch plan at
/// most; the trim is here.
///
/// A read over units - a folder's leaves - splits it in two: each unit runs
/// [`Self::of_unit`] over its own rows, its settled conjuncts already out,
/// and the read runs [`Self::over_units`] once over them all.
#[derive(Clone, Debug)]
pub(crate) struct Residual {
    filter: Filter,
    select: Option<Selector>,
    offset: Option<u64>,
    limit: Option<u64>,
    max_byte_size: Option<u64>,
    phase: Phase,
}

/// Which part of a [`Residual`] one application runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    /// Every clause: a read over one unit.
    Whole,
    /// The conjuncts over stored columns alone: one unit of a read over
    /// several.
    Unit,
    /// The selection, the conjuncts after it and the bounds: a read over
    /// units that each ran [`Phase::Unit`].
    OverUnits,
}

impl Residual {
    /// Whether the residual leaves every row as the medium answered it.
    pub(crate) fn is_identity(&self) -> bool {
        let bounded = self.offset.is_some() || self.limit.is_some() || self.max_byte_size.is_some();
        match self.phase {
            Phase::Whole => self.filter.is_always_true() && self.select.is_none() && !bounded,
            Phase::Unit => self.filter.is_always_true(),
            // With every column published, no conjunct waits for the
            // selection.
            Phase::OverUnits => self.select.is_none() && !bounded,
        }
    }

    /// What one unit of a read over several - a folder's leaf - applies to
    /// its own rows: the conjuncts over stored columns, its settled ones
    /// already out. The selection, the conjuncts after it and the bounds
    /// are the read's, applied once by [`Self::over_units`].
    pub(crate) fn of_unit(&self) -> Self {
        Self {
            offset: None,
            limit: None,
            max_byte_size: None,
            phase: Phase::Unit,
            ..self.clone()
        }
    }

    /// What a read over units that each ran [`Self::of_unit`] applies once
    /// over them all: the selection, the conjuncts after it, and the bounds.
    pub(crate) fn over_units(&self) -> Self {
        Self {
            phase: Phase::OverUnits,
            ..self.clone()
        }
    }

    /// Run the residual over a reader, batch by batch: each clause bound
    /// once against the schema it meets, the bounds holding one batch and
    /// pulling nothing past themselves.
    ///
    /// # Errors
    ///
    /// Returns a clause that does not bind against the reader's schema.
    pub(crate) fn apply_reader(
        &self,
        reader: crate::arrow::BatchReader,
    ) -> crate::arrow::Result<crate::arrow::BatchReader> {
        use arrow_array::RecordBatchReader as _;

        if self.is_identity() {
            return Ok(reader);
        }
        let all;
        let select = if let Some(select) = &self.select {
            select
        } else {
            all = Selector::all();
            &all
        };
        let reader = match self.phase {
            Phase::Whole => crate::media::options::expressions(&self.filter, select, reader)?,
            Phase::Unit | Phase::OverUnits => {
                let schema = reader.schema();
                let (early, late) = crate::expression::filter_phases(
                    &self.filter,
                    select,
                    schema.fields().iter().map(|field| field.name().as_str()),
                );
                if self.phase == Phase::Unit {
                    return Ok(early.apply_arrow_reader(reader)?);
                }
                late.apply_arrow_reader(select.apply_arrow_reader(reader)?)?
            }
        };
        Ok(crate::media::options::limited(
            reader,
            self.offset.unwrap_or(0),
            self.limit,
            self.max_byte_size,
        ))
    }

    /// Run the whole residual over a native row stream, row by row with no
    /// batch built where no byte bound or `unnest` needs Arrow storage: a
    /// row stream is one unit's.
    ///
    /// # Errors
    ///
    /// Returns a stream that is not of records, or a clause that does not
    /// bind against its rows.
    pub(crate) fn apply_stream(&self, rows: StreamSerie) -> Result<StreamSerie> {
        let all;
        let select = if let Some(select) = &self.select {
            select
        } else {
            all = Selector::all();
            &all
        };
        crate::media::options::shaped_stream(
            rows,
            &self.filter,
            select,
            self.offset.unwrap_or(0),
            self.limit,
            self.max_byte_size,
        )
    }
}

/// From the medium's options, the serie's clauses, the origin's root and the
/// conjuncts the unit's location settles, to the options the medium is
/// handed and what is applied once after it.
///
/// The `where` clause is split once against the stored columns - the
/// declared root's children, else `origin`'s - into the conjuncts over them,
/// which the medium is handed to prune by, and the ones naming what only the
/// `select` publishes; with neither known the medium is handed the clause
/// whole and splits it against its own schema. A conjunct `settled` proves
/// for every row of the unit - `column = value`, or `column is null` against
/// a null - leaves both halves. A declared root is handed narrowed to the
/// columns the `select` and the handed conjuncts read, so a declared column
/// outside the selection is never asked for; with none declared the medium
/// reads the same set through
/// [`apply_columns`](crate::media::IORecordOptions::apply_columns). The row
/// skip and bounds are handed as each medium reads them today - a fetch plan
/// at most (Parquet's leading row groups, one decode thread, a smaller
/// batch) - except where a conjunct placed after the selection or an
/// `unnest` makes the rows the medium yields other than the rows they count,
/// and the residual applies them exactly in every case.
///
/// # Errors
///
/// Returns a bound combined with a merge key, or a declared root a
/// narrowing cannot be laid out from.
pub(crate) fn compose(
    medium: &RecordOptions,
    scan: &Scan,
    origin: Option<&Field>,
    settled: &[(SmolStr, Scalar)],
) -> Result<Composed> {
    let options = scan.folded(medium)?;
    let filter = unsettled(options.filter(), settled);
    let select = options.select();
    let (early, late) = match options.declared().or(origin) {
        Some(stored) => {
            let (early, late) = crate::expression::filter_phases(
                &filter,
                select,
                stored.fields().iter().map(Field::name),
            );
            (early.into_owned(), late.into_owned())
        }
        None => (filter.clone(), Filter::always_true()),
    };
    let mut handed = options.clone();
    // A medium with no header row pairs the declared children with its
    // columns by position, so it is handed the root whole - a narrowed one
    // would shift them - and the selection narrows what it answered.
    if let Some(declared) = options.declared()
        && options.header() != Some(false)
    {
        handed.set_declared(Some(narrowed(declared, select, &early)?));
    }
    if !late.is_always_true() || select.unnests() {
        handed.set_row_offset(None);
        handed.set_max_row_size(None);
        handed.set_max_byte_size(None);
    }
    handed.set_filter(early);
    let residual = Residual {
        filter,
        select: (!select.is_all()).then(|| select.clone()),
        offset: options.row_offset().filter(|rows| *rows != 0),
        limit: options.max_row_size(),
        max_byte_size: options.max_byte_size(),
        phase: Phase::Whole,
    };
    Ok(Composed { handed, residual })
}

/// `filter` less the conjuncts `settled` proves for every row.
fn unsettled(filter: &Filter, settled: &[(SmolStr, Scalar)]) -> Filter {
    if settled.is_empty() || filter.is_always_true() {
        return filter.clone();
    }
    Filter::all(
        filter
            .conjuncts()
            .into_iter()
            .filter(|conjunct| !settles(conjunct, settled)),
    )
}

/// Whether `settled` proves `conjunct` for every row: an equality of a
/// column with the value the unit fixes for it - compared as the scalar, or
/// as a partition directory spells it - or `column is null` where the unit
/// fixes it null. A comparison with a null literal is never proven: it holds
/// for no row.
fn settles(conjunct: &Filter, settled: &[(SmolStr, Scalar)]) -> bool {
    use crate::expression::{Comparison, Term};

    let (column, value) = match conjunct.term() {
        Term::Compare(left, Comparison::Eq, right) => {
            match (left.as_column(), right.as_literal()) {
                (Some(column), Some(literal)) => (column, Some(literal.value())),
                _ => match (right.as_column(), left.as_literal()) {
                    (Some(column), Some(literal)) => (column, Some(literal.value())),
                    _ => return false,
                },
            }
        }
        Term::IsNull(inner) => match inner.as_column() {
            Some(column) => (column, None),
            None => return false,
        },
        _ => return false,
    };
    settled.iter().any(|(name, held)| {
        name.eq_ignore_ascii_case(column)
            && match value {
                // A null on either side proves no equality: `col = null`
                // holds for no row, and a unit fixing `col` null holds no row
                // equal to a value - even one spelled as a null directory is.
                Some(value) if value.is_null() || held.is_null() => false,
                Some(value) => {
                    value == held
                        || matches!(
                            (
                                crate::media::partition::partition_text(value),
                                crate::media::partition::partition_text(held),
                            ),
                            (Ok(mine), Ok(theirs)) if mine == theirs
                        )
                }
                None => held.is_null(),
            }
    })
}

/// `declared` narrowed to the children the `select` and the `early`
/// conjuncts read, in its order, its `SORT:by` kept only where every key
/// column stays. A `*` reads every child; a selection reading no declared
/// child - literals alone - keeps the root whole, since a batch of no column
/// states no row count.
fn narrowed(declared: &Field, select: &Selector, early: &Filter) -> Result<Field> {
    if select.has_star() {
        return Ok(declared.clone());
    }
    let mut read = select.columns();
    read.extend(early.columns());
    let dropped: Vec<&str> = declared
        .fields()
        .iter()
        .map(Field::name)
        .filter(|name| !read.iter().any(|column| column.eq_ignore_ascii_case(name)))
        .collect();
    if dropped.is_empty() || dropped.len() == declared.fields().len() {
        return Ok(declared.clone());
    }
    let narrowed = declared.without_fields(&dropped)?;
    Ok(crate::iomedia::keeping_order(narrowed, |column| {
        !dropped.iter().any(|gone| gone.eq_ignore_ascii_case(column))
    }))
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/media_serie.rs` pins and a caller cannot reach:
    //! a read's residual run over a native row stream, as a read door
    //! composes it, so a test can count what the stream is pulled for.

    use crate::media::RecordOptions;
    use crate::{Result, StreamSerie};

    /// Compose `options` as a read door does and run the residual over
    /// `rows`.
    ///
    /// # Errors
    ///
    /// Returns a bound combined with a merge key, a stream that is not of
    /// records, or a clause that does not bind against its rows.
    pub fn apply_stream(options: &RecordOptions, rows: StreamSerie) -> Result<StreamSerie> {
        super::compose(options, &super::Scan::default(), None, &[])?
            .residual
            .apply_stream(rows)
    }
}

/// Shared storage, the scan clauses this serie stated and its lazily
/// retained rows. The fields are private so a scan cannot change after its
/// first pull; the options are the medium's, read through it on every read.
pub struct MediaSerieState<T: IOMedia + Send + 'static> {
    pub(crate) media: Arc<Mutex<T>>,
    /// The root the medium answered when the serie bound it - its declared
    /// field, else its origin's - which every scan's clauses are laid over,
    /// so a re-plan reads nothing; an edited snapshot's own rows' root.
    pub(crate) root: Arc<Field>,
    pub(crate) field: Arc<Field>,
    pub(crate) scan: Scan,
    pub(crate) read: Read<T>,
    pub(crate) edited: Option<Serie>,
    pub(crate) rows: Arc<OnceLock<Serie>>,
}

impl<T: IOMedia + Send + 'static> Clone for MediaSerieState<T> {
    fn clone(&self) -> Self {
        Self {
            media: Arc::clone(&self.media),
            root: Arc::clone(&self.root),
            field: Arc::clone(&self.field),
            scan: self.scan.clone(),
            read: self.read,
            edited: self.edited.clone(),
            rows: Arc::clone(&self.rows),
        }
    }
}

impl<T: IOMedia + Send + 'static> fmt::Debug for MediaSerieState<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MediaSerieState")
            .field("field", &self.field)
            .field("scan", &self.scan)
            .field("edited", &self.edited.is_some())
            .field("rows", &self.rows.get())
            .finish()
    }
}

impl<T: IOMedia + Send + 'static> MediaSerieState<T> {
    /// Retain a medium under its own options and bind its scan without
    /// decoding result rows: one `record_options` and one `read_arrow_field`
    /// of the medium, nothing of the store a wrapper already answers for.
    ///
    /// # Errors
    /// Schema, scan and limit refusals.
    pub fn new(media: T) -> Result<Self> {
        Self::with_reader(
            media,
            |media, options| media.read_serie(Some(options)),
            |_| Ok(()),
        )
    }

    /// `require` judges the options the medium answers before the schema is
    /// read, so a leaf refuses a medium of another encoding on the one
    /// `record_options` the construction costs.
    pub(crate) fn with_reader(
        media: T,
        read: Read<T>,
        require: fn(&RecordOptions) -> Result<()>,
    ) -> Result<Self> {
        let options = media.record_options()?;
        require(&options)?;
        options.require_write_limits()?;
        let root = Arc::new(media.read_arrow_field(&crate::iomedia::dimensions(options.clone()))?);
        let field = Arc::new(bound_field(&root, &options)?);
        Ok(Self {
            media: Arc::new(Mutex::new(media)),
            root,
            field,
            scan: Scan::default(),
            read,
            edited: None,
            rows: Arc::new(OnceLock::new()),
        })
    }

    pub(crate) fn media(&self) -> MutexGuard<'_, T> {
        self.media
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The options the medium states for this serie's reads: its own - an
    /// edited snapshot's with the clauses the snapshot already answered taken
    /// off and its root declared.
    fn medium_options(&self, media: &T) -> Result<RecordOptions> {
        let mut options = media.record_options()?;
        if self.edited.is_some() {
            options = crate::iomedia::dimensions(options);
            options.set_field((*self.root).clone());
        }
        Ok(options)
    }

    /// Read the rows under `scan`: the medium's read door, handed the
    /// medium's options with the scan's clauses laid over them, composes them
    /// once; an edited snapshot's held rows go through the composed residual
    /// alone.
    fn read(&self) -> Result<Serie> {
        let media = self.media();
        let medium = self.medium_options(&media)?;
        match &self.edited {
            Some(rows) => compose(&medium, &self.scan, Some(self.root.as_ref()), &[])?
                .residual
                .apply_stream(rows.clone().into_stream()?)
                .map(Serie::from),
            None => (self.read)(&media, &self.scan.folded(&medium)?),
        }
    }

    pub(crate) fn rows(&self) -> &Serie {
        self.rows.get_or_init(|| {
            self.read().unwrap_or_else(|error| {
                Serie::from(StreamSerie::from_rows(
                    (*self.field).clone(),
                    std::iter::once(Err(error)),
                ))
            })
        })
    }

    pub(crate) fn into_rows(self) -> Result<Serie> {
        match self.rows.get() {
            Some(rows) => Ok(rows.clone()),
            None => self.read(),
        }
    }

    /// The same medium under `scan`, the rows to be pulled afresh and the
    /// field still the one the clauses before it bound: nothing is asked
    /// of the medium until a leaf's `from_media_state` binds the field.
    fn with_scan(&self, scan: Scan) -> Self {
        Self {
            scan,
            rows: Arc::new(OnceLock::new()),
            ..self.clone()
        }
    }

    /// Bind the field under this scan and install the leaf's reader, on
    /// one ask of the medium that `require` judges first: what every
    /// leaf's `from_media_state` does, so a re-plan - `with_scan` then
    /// this - asks the medium once and the public door refuses a medium of
    /// another encoding on that same ask. A refusal leaves nothing behind.
    pub(crate) fn bound(
        mut self,
        read: Read<T>,
        require: fn(&RecordOptions) -> Result<()>,
    ) -> Result<Self> {
        let field = {
            let media = self.media();
            let options = self.scan.folded(&self.medium_options(&media)?)?;
            require(&options)?;
            bound_field(&self.root, &options)?
        };
        self.field = Arc::new(field);
        self.read = read;
        Ok(self)
    }

    pub(crate) fn splice(
        &mut self,
        range: std::ops::Range<usize>,
        values: Vec<crate::Scalar>,
    ) -> Result<()> {
        let rows = self.rows();
        rows.raise_held()?;
        let mut edited = rows.held_leaf().clone();
        edited.splice(range, values)?;
        let field = Arc::clone(edited.field_ref().expect("media rows have a field"));
        self.root = Arc::clone(&field);
        self.field = field;
        self.scan = Scan::default();
        self.rows = Arc::new(OnceLock::from(edited.clone()));
        self.edited = Some(edited);
        Ok(())
    }

    /// A slice of the held rows as an edited snapshot: the clauses the rows
    /// already answered are stated no more.
    pub(crate) fn sliced(&self, rows: Serie) -> Self {
        let field = Arc::clone(rows.field_ref().expect("media rows have a field"));
        Self {
            root: Arc::clone(&field),
            field,
            scan: Scan::default(),
            rows: Arc::new(OnceLock::from(rows.clone())),
            edited: Some(rows),
            ..self.clone()
        }
    }

    pub(crate) fn held_memory_size(&self) -> usize {
        self.rows.get().map_or(0, Serie::memory_size)
    }

    pub(crate) fn held_resident_size(&self) -> usize {
        self.rows.get().map_or(0, Serie::resident_size)
    }

    pub(crate) fn is_held(&self) -> bool {
        self.rows.get().is_some_and(Serie::is_held)
    }

    pub(crate) fn scalar(&self, index: usize) -> Result<crate::Scalar> {
        if let Some(rows) = self.rows.get() {
            return rows.scalar(index);
        }
        let seek = {
            let media = self.media();
            let options = self.scan.folded(&self.medium_options(&media)?)?;
            // A stated Arrow-byte limit is global and cannot restart at a seek.
            if options.max_byte_size().is_some()
                || options
                    .max_row_size()
                    .is_some_and(|length| index as u64 >= length)
            {
                None
            } else {
                let offset = options
                    .row_offset()
                    .unwrap_or(0)
                    .checked_add(index as u64)
                    .ok_or_else(|| crate::Error::InvalidRecord {
                        path: self.field.name().into(),
                        reason: "the row offset exceeds uint64".into(),
                    })?;
                let mut seek = options;
                seek.set_row_offset(Some(offset));
                seek.set_max_row_size(Some(1));
                seek.require_write_limits()?;
                Some((self.read)(&media, &seek)?)
            }
        };
        let Some(rows) = seek else {
            return self.rows().scalar(index);
        };
        let mut rows = rows.into_stream()?;
        match rows.next() {
            Some(row) => row,
            None => {
                crate::serie::require_row(self.field.name(), index, self.rows().len())?;
                unreachable!("an existing result row is yielded by its native range")
            }
        }
    }
}

/// The field a serie publishes from `root` under `options`: the one schema
/// answer every medium gives ([`IOMedia::read_arrow_field`]'s), laid over the
/// root the serie bound, as the record root a stream lands under.
fn bound_field(root: &Field, options: &RecordOptions) -> Result<Field> {
    StreamChunkedSerie::root_of(&crate::iomedia::field_under(options, root)?)
}

/// Defaults for a series whose rows belong to an [`IOMedia`].
/// Scan clauses reach the medium's native options at every read: Parquet
/// and Iceberg prune metadata, Avro skips unselected fields, and row codecs
/// read rows.
pub trait MediaSerieValue<T: IOMedia + Send + 'static>: SerieValue {
    /// The retained medium and immutable scan state.
    fn media_state(&self) -> &MediaSerieState<T>;
    /// Build this specialized series from its common scan state: one ask
    /// of the medium, judging its encoding and binding the scan's field
    /// under this leaf's rule. Every re-plan crosses it, so a re-plan
    /// asks the medium once.
    ///
    /// # Errors
    /// A medium of another encoding, or a clause its field refuses.
    fn from_media_state(state: MediaSerieState<T>) -> Result<Self>;

    /// Validate the medium's options before a source can be opened.
    ///
    /// # Errors
    /// A configuration belonging to another encoding.
    fn require_media_options(_options: &RecordOptions) -> Result<()> {
        Ok(())
    }

    /// Read in the medium's native representation, without applying bounds
    /// belonging to a later conversion. Row media override this door.
    ///
    /// # Errors
    /// The medium's read and scan refusals.
    fn read_native(media: &T, options: &RecordOptions) -> Result<Serie> {
        media.read_serie(Some(options))
    }

    /// Conjoin a source predicate, retaining it for native pruning.
    ///
    /// # Errors
    /// Parse and binding failures before a row is pulled.
    fn with_filter(self, filter: impl IntoFilter) -> Result<Self> {
        let mut scan = self.media_state().scan.clone();
        scan.filter = scan.filter.and(filter.into_filter()?);
        Self::from_media_state(self.media_state().with_scan(scan))
    }

    /// Choose source expressions before decoding their values.
    ///
    /// # Errors
    /// Parse and binding failures before a row is pulled.
    fn with_select(self, select: impl IntoSelector) -> Result<Self> {
        let mut scan = self.media_state().scan.clone();
        scan.select = Some(select.into_selector()?);
        Self::from_media_state(self.media_state().with_scan(scan))
    }

    /// Bound result rows in the native scan, after filtering and selection.
    ///
    /// # Errors
    /// An invalid combination with a merge key.
    fn with_row_range(self, offset: u64, length: Option<u64>) -> Result<Self> {
        let mut scan = self.media_state().scan.clone();
        scan.range = Some((offset, length));
        Self::from_media_state(self.media_state().with_scan(scan))
    }

    /// Retain an exact key predicate in the native scan, including null cells
    /// and casts. Aliases name the key record; source terms drive pruning.
    ///
    /// # Errors
    /// Invalid key, key value or source predicate before pulling rows.
    fn with_key(self, by: impl IntoSelector, key: crate::Scalar) -> Result<Self> {
        let by = by.into_selector()?;
        let root = &self.media_state().root;
        let bound = by.bind_key(root, root.name(), "prune by")?;
        let key = bound.output().scalar(key)?;
        let cells = key.sequence_rows().expect("a key value is a record");
        let predicates =
            by.expanded(root)
                .into_iter()
                .zip(cells.iter())
                .map(|(projection, value)| {
                    let term = match projection.dtype() {
                        Some(dtype) => projection.term().clone().cast(dtype.clone()),
                        None => projection.term().clone(),
                    };
                    crate::Filter::new(if value.is_null() {
                        term.is_null()
                    } else {
                        term.eq(crate::expression::Term::literal(value.clone()))
                    })
                });
        let filter = crate::Filter::all(predicates);
        self.with_filter(filter)
    }

    /// Stream only the selected key cells, leaving unused payload columns
    /// to the medium's projection plan.
    ///
    /// # Errors
    /// The selector's and read's refusals.
    fn key_values(self, by: impl IntoSelector) -> Result<StreamSerie> {
        self.with_select(by)?.into_stream()
    }

    /// Read native rows lazily. Row codecs allocate no intermediate batch.
    ///
    /// # Errors
    /// The medium's read refusals, followed by source errors while pulling.
    fn into_stream(self) -> Result<StreamSerie> {
        self.media_state().clone().into_rows()?.into_stream()
    }

    /// Convert under the same bounds every serie kind uses.
    ///
    /// # Errors
    /// The medium's read refusals and the conversion's bounds.
    fn into_chunked_stream(
        self,
        row_size: Option<usize>,
        byte_size: Option<u64>,
    ) -> crate::arrow::Result<StreamChunkedSerie> {
        self.media_state()
            .clone()
            .into_rows()?
            .into_chunked_stream(row_size, byte_size)
    }

    /// Cluster native chunks as lazy adjacent windows.
    ///
    /// # Errors
    /// Key binding or read failure; later failures are yielded once.
    fn window_by(
        self,
        by: impl crate::IntoKeyBy,
        sorted: bool,
    ) -> crate::arrow::Result<crate::StreamKeySerie> {
        let rows = self.media_state().clone().into_rows()?;
        match rows {
            Serie::Stream(stream) => {
                crate::SharedStream::into_stream(stream)?.window_by(by, sorted)
            }
            chunks => StreamChunkedSerie::from_serie(chunks)?.window_by(by, sorted),
        }
    }

    /// Cluster native chunks with bounded open partitions.
    ///
    /// # Errors
    /// Key binding or read failure; later failures are yielded once.
    fn partition_by(
        self,
        by: impl crate::IntoKeyBy,
        options: crate::PartitionOptions,
    ) -> crate::arrow::Result<crate::StreamKeySerie> {
        let rows = self.media_state().clone().into_rows()?;
        match rows {
            Serie::Stream(stream) => {
                crate::SharedStream::into_stream(stream)?.partition_by(by, options)
            }
            chunks => StreamChunkedSerie::from_serie(chunks)?.partition_by(by, options),
        }
    }

    /// Publish any serie through the medium's explicit write door, under the
    /// medium's own options where none are given.
    /// A row mutation edits this series' held snapshot until this is called.
    ///
    /// # Errors
    /// The medium's encoding, write and commit refusals.
    fn write_serie(
        &self,
        value: Serie,
        mode: crate::IOMode,
        options: Option<&RecordOptions>,
    ) -> Result<crate::IOResult> {
        let resolved = {
            let media = self.media_state().media();
            let document =
                crate::text::Format::from_media_type(media.as_io_base().media_type()).is_ok();
            if document {
                crate::iomedia::require_document_mode(mode)?;
                options.cloned()
            } else {
                crate::iomedia::require_serie_write_mode(mode)?;
                let options = crate::iomedia::own_options(&*media, options)?.into_owned();
                Self::require_media_options(&options)?;
                options.require_write_mode(mode)?;
                options.require_commit_batch_num()?;
                options.require_num_threads()?;
                options.require_write_limits()?;
                Some(options)
            }
        };
        // Native readers own their source after opening. The target guard is
        // acquired afterwards, so writing this same medium cannot lock itself.
        let value = Serie::from(StreamChunkedSerie::from_serie(value)?);
        self.media_state()
            .media()
            .write_serie(value, mode, resolved.as_ref())
    }
}

/// Refuse options of a medium the `kind` leaf does not read: `accepts` the
/// MIME types it reads, `None` every type.
pub(crate) fn require_kind(
    options: &RecordOptions,
    kind: &str,
    accepts: Option<&[crate::MimeType]>,
) -> Result<()> {
    if accepts.is_none_or(|types| types.contains(&options.mime_type())) {
        return Ok(());
    }
    Err(crate::Error::InvalidRecord {
        path: "$.encoding".into(),
        reason: smol_str::format_smolstr!(
            "expected {kind} serie options, got {}",
            options.mime_type()
        ),
    })
}

/// Define the media's leaf in its own module; defaults have one owner here.
macro_rules! media_serie {
    ($name:ident, $variant:ident, $access:ident, $access_mut:ident, accepts = $accepts:expr) => {
        /// A lazy series over this medium, reading under the medium's own
        /// options and the clauses its verbs state.
        #[derive(Clone, Debug)]
        pub struct $name {
            state: $crate::MediaSerieState<Box<dyn $crate::IOBase>>,
        }
        impl $name {
            /// Retain a media handle under its own options; discover its
            /// schema without decoding result batches. A medium of another
            /// encoding is refused before its schema is read.
            ///
            /// # Errors
            /// Schema, encoding and scan binding refusals.
            pub fn new<H: $crate::IOBase + 'static>(media: H) -> $crate::Result<Self> {
                let media: Box<dyn $crate::IOBase> = Box::new(media);
                Ok(Self { state: $crate::media_serie::MediaSerieState::with_reader(media, <Self as $crate::MediaSerieValue<Box<dyn $crate::IOBase>>>::read_native, <Self as $crate::MediaSerieValue<Box<dyn $crate::IOBase>>>::require_media_options)? })
            }
        }
        impl $crate::MediaSerieValue<Box<dyn $crate::IOBase>> for $name {
            fn media_state(&self) -> &$crate::MediaSerieState<Box<dyn $crate::IOBase>> { &self.state }
            fn from_media_state(state: $crate::MediaSerieState<Box<dyn $crate::IOBase>>) -> $crate::Result<Self> {
                state.bound(<Self as $crate::MediaSerieValue<Box<dyn $crate::IOBase>>>::read_native, <Self as $crate::MediaSerieValue<Box<dyn $crate::IOBase>>>::require_media_options).map(|state| Self { state })
            }
            fn require_media_options(options: &$crate::media::RecordOptions) -> $crate::Result<()> { $crate::media_serie::require_kind(options, stringify!($variant), $accepts) }
        }
        $crate::serie::serie_leaf!($name);
        impl From<$name> for $crate::Serie {
            fn from(value: $name) -> Self { $crate::SerieValue::into_serie(value) }
        }
        impl $crate::Serie {
            /// Borrow the specialized media scan, without opening it.
            pub fn $access(&self) -> Option<&$name> { match self { Self::$variant(value) => Some(value), _ => None } }
            /// Edit the specialized scan's snapshot through copy on write.
            pub fn $access_mut(&mut self) -> Option<&mut $name> { match self { Self::$variant(value) => Some(::std::sync::Arc::make_mut(value)), _ => None } }
        }
        impl $crate::SerieValue for $name {
            fn field(&self) -> &$crate::Field { &self.state.field }
            fn field_ref(&self) -> &::std::sync::Arc<$crate::Field> { &self.state.field }
            fn len(&self) -> usize { self.state.rows().len() }
            fn null_count(&self) -> usize { self.state.rows().null_count() }
            fn is_null(&self, index: usize) -> $crate::Result<bool> { self.state.scalar(index).map(|row| row.is_null()) }
            fn scalar(&self, index: usize) -> $crate::Result<$crate::Scalar> { self.state.scalar(index) }
            fn slice(&self, offset: usize, length: usize) -> $crate::Result<Self> {
                let rows = self.state.rows();
                rows.raise_held_rows()?;
                Ok(Self { state: self.state.sliced(rows.slice(offset, length)?) })
            }
            fn splice(&mut self, range: ::std::ops::Range<usize>, rows: Vec<$crate::Scalar>) -> $crate::Result<()> { self.state.splice(range, rows) }
            fn into_arrow_array(&self) -> ::arrow_array::ArrayRef { self.state.rows().into_arrow_array().expect("media rows have a field") }
            fn memory_size(&self) -> usize { self.state.held_memory_size() }
            fn resident_size(&self) -> usize { self.state.held_resident_size() }
            fn into_serie(self) -> $crate::Serie { $crate::Serie::$variant(::std::sync::Arc::new(self)) }
            fn from_serie(value: &$crate::Serie) -> Option<&Self> {
                match value { $crate::Serie::$variant(value) => Some(value), _ => None }
            }
        }
    };
}
pub(crate) use media_serie;

impl Serie {
    /// The shared scan state of a specialized medium, without pulling rows.
    pub(crate) fn media_state(&self) -> Option<&MediaSerieState<Box<dyn crate::IOBase>>> {
        match self {
            Self::Ipc(value) => Some(value.media_state()),
            Self::Csv(value) => Some(value.media_state()),
            Self::Text(value) => Some(value.media_state()),
            Self::WarehouseTable(value) => Some(value.media_state()),
            #[cfg(feature = "http")]
            Self::Http(value) => Some(value.media_state()),
            Self::GenericMedia(value) => Some(value.media_state()),
            _ => None,
        }
    }

    pub(crate) fn splice_media(
        &mut self,
        range: std::ops::Range<usize>,
        rows: Vec<crate::Scalar>,
    ) -> Result<()> {
        match self {
            Self::Ipc(value) => Arc::make_mut(value).splice(range, rows),
            Self::Csv(value) => Arc::make_mut(value).splice(range, rows),
            Self::Text(value) => Arc::make_mut(value).splice(range, rows),
            Self::WarehouseTable(value) => Arc::make_mut(value).splice(range, rows),
            #[cfg(feature = "http")]
            Self::Http(value) => Arc::make_mut(value).splice(range, rows),
            Self::GenericMedia(value) => Arc::make_mut(value).splice(range, rows),
            _ => unreachable!("only specialized media have a scan state"),
        }
    }
}
