//! A stream's rows cut by a key into partitions, held apart under the process
//! spill bound, at most a bounded number open at once, closed as they
//! complete: [`SerieReader::partition_by`] and the one partitioner every
//! writer that splits a stream by partition runs through.
//!
//! Each batch is landed, keyed and cut into the pieces its partitions take on
//! the stream's threads, one batch per thread, and answered in the order the
//! batches arrived; the pieces are pushed to their open partitions on the
//! pulling thread in that order, so a partition's rows keep the order they
//! arrived in whatever thread cut them. Every open partition is a
//! [`ChunkedSerie`] settled as it is pushed to, and the open partitions
//! together are settled after each batch, heaviest first, so what they keep
//! resident stays under the process bound however long the stream is.
//!
//! Past `max_open` open partitions, the ones of the lowest keys close: a
//! stream arriving in key order closes each partition once its last row has
//! been read, so it is written while the stream is still arriving and no
//! partition is ever held whole; a key arriving again after its partition
//! closed opens a new piece of it. When the stream ends, every partition
//! still open closes in ascending key order.
//!
//! A clustered stream - every row of a key arriving before any row of the
//! next, as a stream whose root declares an order leading with the key's
//! terms does - keeps one partition open: each closes as soon as another
//! key arrives, and the partitions close in the order they arrived.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::arrow::{BatchReader, Result};
use crate::expression::IntoSelector;
use crate::{ChunkedSerie, Field, Scalar, Serie, SerieReader, SpillOptions};

/// How [`SerieReader::partition_by`] cuts a stream: how many partitions it
/// keeps open at once, and the threads its batches are cut on.
///
/// `Default` keeps every partition open until the stream ends - each closed
/// once, whole - on every thread the host offers, unless the stream's root
/// declares an order that clusters the key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PartitionOptions {
    max_open: Option<usize>,
    threads: Option<usize>,
    clustered: bool,
}

impl PartitionOptions {
    /// Every partition open until the stream ends, on the host's threads.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            max_open: None,
            threads: None,
            clustered: false,
        }
    }

    /// Whether the stream's rows of one key arrive together - every row of
    /// a key before any row of the next, as a stream sorted on the key's
    /// terms in any direction is. Clustered, each partition closes as soon
    /// as another key arrives, so one is open at a time and
    /// [`Self::max_open`] is moot; a key arriving again after its partition
    /// closed opens a new piece of it, yielded again under that key, so a
    /// stream that is not clustered after all costs pieces, never rows. A
    /// stream whose root declares an order leading with the key's terms is
    /// clustered without being told.
    #[must_use]
    pub const fn with_clustered(mut self, clustered: bool) -> Self {
        self.clustered = clustered;
        self
    }

    /// At most `partitions` open at once - at least one - the lowest keys
    /// closed past it.
    #[must_use]
    pub const fn with_max_open(mut self, partitions: usize) -> Self {
        self.max_open = Some(if partitions == 0 { 1 } else { partitions });
        self
    }

    /// Cut the batches on `threads` threads - at least one; one cuts each as
    /// it is pulled and reads nothing ahead.
    #[must_use]
    pub const fn with_threads(mut self, threads: usize) -> Self {
        self.threads = Some(if threads == 0 { 1 } else { threads });
        self
    }

    /// How many partitions may be open at once, `None` every one.
    #[must_use]
    pub const fn max_open(&self) -> Option<usize> {
        self.max_open
    }

    /// The threads the batches are cut on: as stated, else the host's.
    #[must_use]
    pub fn threads(&self) -> usize {
        self.threads
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, usize::from))
    }

    /// Whether the stream was stated clustered by its key.
    #[must_use]
    pub const fn is_clustered(&self) -> bool {
        self.clustered
    }
}

/// One closed partition of a stream: its key and its rows, in the order they
/// arrived, held as the chunks the stream's batches gave it.
#[derive(Clone, Debug)]
pub struct SeriePartition {
    key: Scalar,
    rows: ChunkedSerie,
}

impl SeriePartition {
    /// The key every row of the partition computes to: the record of the key
    /// cells, as [`ChunkedSerie::window_by`] states a key.
    #[must_use]
    pub const fn key(&self) -> &Scalar {
        &self.key
    }

    /// The partition's rows.
    #[must_use]
    pub const fn rows(&self) -> &ChunkedSerie {
        &self.rows
    }

    /// The key and the rows, owned.
    #[must_use]
    pub fn into_parts(self) -> (Scalar, ChunkedSerie) {
        (self.key, self.rows)
    }

    /// The rows as an Arrow stream of one batch per chunk, sharing their
    /// buffers.
    ///
    /// # Errors
    ///
    /// [`ChunkedSerie::into_arrow_reader`]'s refusal.
    pub fn into_arrow_reader(&self) -> Result<BatchReader> {
        self.rows.into_arrow_reader()
    }
}

/// The partitions [`SerieReader::partition_by`] cuts a stream into, lazily:
/// each closed partition as soon as it closes - past the bound on open
/// partitions, the lowest keys first, or, clustered, once another key
/// arrives - and every partition still open when the stream ends, in
/// ascending key order. Fused after its first error.
pub struct SerieReaderPartitions {
    partitions: Partitions<Scalar>,
}

impl SerieReaderPartitions {
    /// The record root every partition's rows are held under.
    #[must_use]
    pub fn field(&self) -> &Field {
        self.partitions.root()
    }
}

impl Iterator for SerieReaderPartitions {
    type Item = Result<SeriePartition>;

    fn next(&mut self) -> Option<Self::Item> {
        self.partitions
            .next()
            .map(|closed| closed.map(|(key, rows)| SeriePartition { key, rows }))
    }
}

impl SerieReader {
    /// This stream's rows cut by `by` into partitions, each closed as it
    /// completes: one [`SeriePartition`] per partition, its key the record of
    /// the cells `by` computes and its rows held apart, in the order they
    /// arrived, as a [`ChunkedSerie`] under this reader's root.
    ///
    /// `by` is bound once against the root, as [`Self::window_by`] binds its
    /// key; each batch's key column is computed on one of
    /// [`PartitionOptions::threads`] threads and cut there - one zero-copy
    /// slice per run of a key, one take per key otherwise - and the batches
    /// are answered in order. The open partitions are held under the process
    /// spill bound, settled after each batch, heaviest first. Past
    /// [`PartitionOptions::max_open`] open partitions, the lowest keys close
    /// and are yielded; a stream in key order therefore closes each
    /// partition once it has been read whole, and a key arriving again after
    /// its partition closed opens a new piece of it, yielded again under the
    /// same key. When the stream ends, every partition still open closes in
    /// ascending key order. A closed partition's rows are the caller's: they
    /// are no longer counted against the bound.
    ///
    /// A clustered stream keeps one partition open, each yielded as soon as
    /// another key arrives, in arrival order: one stated
    /// [`PartitionOptions::with_clustered`], or one whose root declares an
    /// order - proven as its batches land - whose leading keys are the
    /// terms `by` projects, uncast, in any order.
    ///
    /// ```
    /// use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, PartitionOptions, Scalar, Serie, SerieReader, StructType};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let root = DataType::from(StructType::from_fields([
    ///     DataType::utf8().required_field("venue"),
    ///     DataType::Int64.required_field("qty"),
    /// ])?)
    /// .required_field("row");
    /// let fill = |venue: &str, qty: i64| Scalar::from_sequence(vec![Scalar::from(venue), Scalar::from(qty)]);
    /// let batches = vec![
    ///     Serie::from_scalars(root.clone(), [fill("XNAS", 1), fill("XLON", 2)])?,
    ///     Serie::from_scalars(root.clone(), [fill("XNAS", 3), fill("XPAR", 4)])?,
    /// ];
    /// let stream = SerieReader::from_chunked(ChunkedSerie::from_series(Some(&root), batches, ArrowCastOptions::new())?)?;
    ///
    /// // At most two open at once: the third venue closes the lowest, XLON.
    /// let options = PartitionOptions::new().with_max_open(2).with_threads(1);
    /// let closed: Vec<(Scalar, usize)> = stream
    ///     .partition_by("venue", options)?
    ///     .map(|partition| partition.map(|partition| {
    ///         let (key, rows) = partition.into_parts();
    ///         (key, rows.len())
    ///     }))
    ///     .collect::<Result<_, _>>()?;
    /// let venue = |name: &str| Scalar::from_sequence(vec![Scalar::from(name)]);
    /// assert_eq!(closed, [(venue("XLON"), 1), (venue("XNAS"), 2), (venue("XPAR"), 1)]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Before any batch is pulled: the text's own parse error, and the
    /// binder's refusals of `by` naming this reader's root, as
    /// [`Self::window_by`] refuses them. While the partitions are pulled: the
    /// stream's own failure, and a spill that cannot be written.
    pub fn partition_by(
        self,
        by: impl IntoSelector,
        options: PartitionOptions,
    ) -> Result<SerieReaderPartitions> {
        let root = Arc::new(self.field().clone());
        let selector = by.into_selector()?;
        let closing = if options.is_clustered() || declares_clustering(&root, &selector) {
            Closing::Clustered
        } else {
            options.max_open().map_or(Closing::Never, Closing::Bounded)
        };
        let key = Arc::new(selector.bind_key(&root, root.name(), "partition by")?);
        let pieces = self.map_landed(options.threads(), move |record: Serie| {
            let keys = key.apply_serie(&record)?;
            Ok(record.partition_by(&keys)?)
        });
        Ok(SerieReaderPartitions {
            partitions: Partitions::new(root, pieces, closing),
        })
    }
}

/// Whether the order `root` declares keeps every row of one `key` together:
/// its leading keys are the terms `key` projects - each uncast, in any order
/// and either direction - so rows equal on them are adjacent. A declaration
/// that does not parse clusters nothing, and the landing names it.
fn declares_clustering(root: &Field, key: &crate::Selector) -> bool {
    let projections = key.projections();
    if key.has_star() || projections.is_empty() {
        return false;
    }
    let Ok(Some(declared)) = root.as_sort().by() else {
        return false;
    };
    let Some(leading) = declared.get(..projections.len()) else {
        return false;
    };
    projections.iter().all(|projection| {
        projection.dtype().is_none()
            && leading
                .iter()
                .any(|ordering| ordering.term() == projection.term())
    }) && leading.iter().all(|ordering| {
        projections
            .iter()
            .any(|projection| projection.term() == ordering.term())
    })
}

/// When an open partition closes.
#[derive(Clone, Debug)]
pub(crate) enum Closing {
    /// Every partition open until the pieces end.
    Never,
    /// At most this many open, the lowest keys closed past it.
    Bounded(usize),
    /// One open: each closed as soon as another key arrives.
    Clustered,
    /// Clustered while the flag holds - a writer verifying, as the rows
    /// arrive, an order their source only claims, and clearing the flag at
    /// the first batch out of it, before its pieces are pushed - and bounded
    /// by the count from then on.
    #[cfg_attr(not(feature = "iceberg"), allow(dead_code))]
    ClusteredWhile(Arc<AtomicBool>, usize),
}

impl Closing {
    /// Whether a batch pushed now closes each partition once another key
    /// arrives, and the bound on open partitions otherwise: read once per
    /// batch, after the pieces' source checked it.
    fn now(&self) -> (bool, Option<usize>) {
        match self {
            Self::Never => (false, None),
            Self::Bounded(bound) => (false, Some(*bound)),
            Self::Clustered => (true, None),
            Self::ClusteredWhile(holds, bound) => {
                if holds.load(Ordering::Acquire) {
                    (true, None)
                } else {
                    (false, Some(*bound))
                }
            }
        }
    }
}

/// Each batch's pieces with their keys, in batch order, as a partitioner
/// pulls them.
pub(crate) type Pieces<K> = Box<dyn Iterator<Item = Result<Vec<(K, Serie)>>> + Send>;

/// The one partitioner every split of a stream by partition runs through:
/// the pieces each batch was cut into, in batch order, pushed to the open
/// partition of their key; closed as `closing` says - past a bound the
/// lowest keys, clustered each as soon as another key arrives - and every
/// partition still open closed in ascending key order when the pieces end.
/// `K` is whatever the caller keys by - a key record, a tuple of partition
/// values - ordered as the partitions close in.
pub(crate) struct Partitions<K> {
    root: Arc<Field>,
    pieces: Option<Pieces<K>>,
    open: BTreeMap<K, ChunkedSerie>,
    closing: Closing,
    closed: VecDeque<(K, ChunkedSerie)>,
}

impl<K: Ord> Partitions<K> {
    /// Partition `pieces` - each batch's pieces with their keys, in batch
    /// order, every piece a record column under `root` - closing them as
    /// `closing` says.
    pub(crate) fn new(root: Arc<Field>, pieces: Pieces<K>, closing: Closing) -> Self {
        Self {
            root,
            pieces: Some(pieces),
            open: BTreeMap::new(),
            closing,
            closed: VecDeque::new(),
        }
    }

    /// The record root every partition's rows are held under.
    pub(crate) fn root(&self) -> &Field {
        &self.root
    }

    /// Push one batch's pieces to their open partitions, close what the
    /// closing rule closes, and settle what stays open.
    fn take(&mut self, found: Vec<(K, Serie)>) -> crate::Result<()> {
        let (clustered, bound) = self.closing.now();
        for (key, piece) in found {
            // Clustered, a key other than the one open closes it first, so
            // the partitions close in the order they arrived.
            if clustered && !self.open.contains_key(&key) {
                while let Some(done) = self.open.pop_first() {
                    self.closed.push_back(done);
                }
            }
            match self.open.get_mut(&key) {
                Some(hold) => hold.push_landed(piece)?,
                None => {
                    let mut hold = ChunkedSerie::from_landed(Arc::clone(&self.root), Vec::new());
                    hold.push_landed(piece)?;
                    self.open.insert(key, hold);
                }
            }
        }
        if let Some(max_open) = bound {
            while self.open.len() > max_open {
                let Some(lowest) = self.open.pop_first() else {
                    break;
                };
                self.closed.push_back(lowest);
            }
        }
        settle_holds(self.open.values_mut())
    }

    /// Fuse: drop the pieces and everything held.
    fn fuse(&mut self) {
        self.pieces = None;
        self.open.clear();
        self.closed.clear();
    }
}

impl<K: Ord> Iterator for Partitions<K> {
    type Item = Result<(K, ChunkedSerie)>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(closed) = self.closed.pop_front() {
                return Some(Ok(closed));
            }
            let pieces = self.pieces.as_mut()?;
            match pieces.next() {
                None => {
                    self.pieces = None;
                    while let Some(open) = self.open.pop_first() {
                        self.closed.push_back(open);
                    }
                }
                Some(Err(error)) => {
                    self.fuse();
                    return Some(Err(error));
                }
                Some(Ok(found)) => {
                    if let Err(error) = self.take(found) {
                        self.fuse();
                        return Some(Err(error.into()));
                    }
                }
            }
        }
    }
}

/// Spill the heaviest of `holds` whole until what they keep resident is
/// under the process bound - the one bound across holds kept apart, each of
/// which settles alone as it is pushed to. The resident total is read once
/// and kept as each spill lowers it, so a settle costs one pass and a sort
/// of the holds, never a pass per spill.
pub(crate) fn settle_holds<'a>(
    holds: impl IntoIterator<Item = &'a mut ChunkedSerie>,
) -> crate::Result<()> {
    let options = SpillOptions::from_env()?;
    if options.is_never() {
        return Ok(());
    }
    let bound = options.byte_size();
    let bytes = |hold: &ChunkedSerie| u64::try_from(hold.resident_size()).unwrap_or(u64::MAX);
    let mut holds: Vec<&mut ChunkedSerie> = holds.into_iter().collect();
    let mut resident = holds
        .iter()
        .map(|hold| bytes(hold))
        .fold(0_u64, u64::saturating_add);
    if resident <= bound {
        return Ok(());
    }
    holds.sort_by_key(|hold| std::cmp::Reverse(hold.resident_size()));
    let whole = options.clone().with_byte_size(0);
    for hold in holds {
        if resident <= bound {
            break;
        }
        let before = bytes(hold);
        hold.spill(&whole)?;
        resident = resident.saturating_sub(before.saturating_sub(bytes(hold)));
    }
    Ok(())
}
