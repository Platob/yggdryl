//! A stream's rows cut by a key into partitions, held apart under the process
//! spill bound, at most a bounded number open at once, closed as they
//! complete: [`StreamChunkedSerie::partition_by`] and the one partitioner every
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

use crate::arrow::Result;
use crate::{ChunkedSerie, Field, Serie, SpillOptions, StreamChunkedSerie};

/// How [`StreamChunkedSerie::partition_by`] cuts a stream: how many partitions it
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

impl StreamChunkedSerie {
    /// Group lazy rows, closing the lowest keys past `max_open`, or closing
    /// each clustered key as the next arrives. Source rows remain chunked.
    ///
    /// # Errors
    /// Refuses an invalid key before pulling. Source and spill failures are
    /// answered while pulling; the iterator is fused after a failure.
    pub fn partition_by(
        self,
        by: impl crate::IntoKeyBy,
        options: PartitionOptions,
    ) -> Result<crate::StreamKeySerie> {
        let root = Arc::new(self.field().clone());
        let by = by.into_key_by()?;
        let clustered = match &by {
            crate::KeyBy::Selector(selector) => declares_clustering(&root, selector),
            crate::KeyBy::External(_) => false,
        };
        let plan = crate::key_serie::KeyPlan::bind(&root, by, None, "partition by")?;
        self.partition_by_planned_with_clustering(plan, options, clustered)
    }

    fn partition_by_planned_with_clustering(
        self,
        plan: crate::key_serie::KeyPlan,
        options: PartitionOptions,
        clustered: bool,
    ) -> Result<crate::StreamKeySerie> {
        let layout = Arc::clone(&plan.layout);
        let closing = if options.is_clustered() || clustered {
            Closing::Clustered
        } else {
            options.max_open().map_or(Closing::Never, Closing::Bounded)
        };
        let key = plan.into_bound()?;
        let cut_layout = Arc::clone(&layout);
        let pieces = self.map_landed(options.threads(), move |record: Serie| {
            let keys = key.apply_serie(&record)?;
            Ok(cut_layout.split_piece(record).cut_partitions(&keys)?)
        });
        let partitions = Partitions::new(Arc::clone(&layout.serie_field), pieces, closing);
        let item_layout = Arc::clone(&layout);
        Ok(crate::StreamKeySerie::new(
            layout,
            partitions.map(move |closed| {
                closed.map(|(key, rows)| {
                    crate::KeySerie::new(key, Arc::clone(&item_layout), Serie::from(rows), None)
                })
            }),
        ))
    }
}

impl crate::StreamSerie {
    pub(crate) fn partition_by_planned(
        self,
        plan: crate::key_serie::KeyPlan,
        selector: &crate::Selector,
        options: PartitionOptions,
    ) -> Result<crate::StreamKeySerie> {
        let clustered = declares_clustering(self.field(), selector);
        self.partition_by_planned_with_clustering(plan, options, clustered)
    }
    /// Partition native rows, holding only open keys and closing at their
    /// natural key boundary or the stated open-key bound.
    ///
    /// # Errors
    /// Field and key refusals before pulling; source and spill failures while pulling.
    pub fn partition_by(
        self,
        by: impl crate::IntoKeyBy,
        options: PartitionOptions,
    ) -> Result<crate::StreamKeySerie> {
        self.require_record_field()?;
        let root = Arc::new(self.field().clone());
        let by = by.into_key_by()?;
        let clustered = match &by {
            crate::KeyBy::Selector(selector) => declares_clustering(&root, selector),
            crate::KeyBy::External(_) => false,
        };
        let plan = crate::key_serie::KeyPlan::bind(&root, by, None, "partition by")?;
        self.partition_by_planned_with_clustering(plan, options, clustered)
    }

    fn partition_by_planned_with_clustering(
        self,
        plan: crate::key_serie::KeyPlan,
        options: PartitionOptions,
        clustered: bool,
    ) -> Result<crate::StreamKeySerie> {
        let layout = Arc::clone(&plan.layout);
        let closing = if options.is_clustered() || clustered {
            Closing::Clustered
        } else {
            options.max_open().map_or(Closing::Never, Closing::Bounded)
        };
        let key = plan.into_bound()?;
        let cut_layout = Arc::clone(&layout);
        let pieces: Pieces<crate::Scalar> = Box::new(self.map(move |row| {
            let row = row?;
            let key = key.apply_scalar(&row)?;
            Ok(vec![(key, Serie::new([cut_layout.split_row(&row)]))])
        }));
        let partitions = Partitions::new(Arc::clone(&layout.serie_field), pieces, closing);
        let item_layout = Arc::clone(&layout);
        Ok(crate::StreamKeySerie::new(
            layout,
            partitions.map(move |closed| {
                closed.map(|(key, rows)| {
                    crate::KeySerie::new(key, Arc::clone(&item_layout), Serie::from(rows), None)
                })
            }),
        ))
    }
}

/// Native rows accumulate under the same closing and spill rules as columns.
struct PartitionHold {
    chunks: ChunkedSerie,
    rows: Vec<crate::Scalar>,
    row_bytes: usize,
}
impl PartitionHold {
    fn new(root: Arc<Field>) -> Self {
        Self {
            chunks: ChunkedSerie::from_landed(root, Vec::new()),
            rows: Vec::new(),
            row_bytes: 0,
        }
    }
    fn push(&mut self, piece: Serie) -> crate::Result<()> {
        match piece {
            Serie::Run(run) => {
                self.row_bytes = self.row_bytes.saturating_add(
                    run.as_slice()
                        .iter()
                        .map(crate::arrow::scalar_memory_size)
                        .sum::<usize>(),
                );
                self.rows.extend_from_slice(run.as_slice());
                Ok(())
            }
            column => {
                self.flush()?;
                self.chunks.push_landed(column)
            }
        }
    }
    fn flush(&mut self) -> crate::Result<()> {
        if !self.rows.is_empty() {
            let piece =
                Serie::from_scalars(self.chunks.field().clone(), std::mem::take(&mut self.rows))?;
            self.row_bytes = 0;
            self.chunks.push_landed(piece)?;
        }
        Ok(())
    }
    fn finish(mut self) -> crate::Result<ChunkedSerie> {
        self.flush()?;
        Ok(self.chunks)
    }
}
trait SpillHold {
    fn resident_size(&self) -> usize;
    fn spill(&mut self, options: &SpillOptions) -> crate::Result<()>;
}
impl SpillHold for ChunkedSerie {
    fn resident_size(&self) -> usize {
        ChunkedSerie::resident_size(self)
    }
    fn spill(&mut self, options: &SpillOptions) -> crate::Result<()> {
        ChunkedSerie::spill(self, options).map(|_| ())
    }
}
impl SpillHold for PartitionHold {
    fn resident_size(&self) -> usize {
        self.chunks.resident_size().saturating_add(self.row_bytes)
    }
    fn spill(&mut self, options: &SpillOptions) -> crate::Result<()> {
        self.flush()?;
        self.chunks.spill(options).map(|_| ())
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
    open: BTreeMap<K, PartitionHold>,
    closing: Closing,
    closed: VecDeque<(K, PartitionHold)>,
    resident: usize,
    spill: Option<&'static SpillOptions>,
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
            resident: 0,
            spill: None,
        }
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
                    self.resident = self.resident.saturating_sub(done.1.resident_size());
                    self.closed.push_back(done);
                }
            }
            let hold = self
                .open
                .entry(key)
                .or_insert_with(|| PartitionHold::new(Arc::clone(&self.root)));
            let before = hold.resident_size();
            hold.push(piece)?;
            self.resident = self
                .resident
                .saturating_sub(before)
                .saturating_add(hold.resident_size());
        }
        if let Some(max_open) = bound {
            while self.open.len() > max_open {
                let Some(lowest) = self.open.pop_first() else {
                    break;
                };
                self.resident = self.resident.saturating_sub(lowest.1.resident_size());
                self.closed.push_back(lowest);
            }
        }
        let options = match &self.spill {
            Some(options) => *options,
            None => *self.spill.insert(SpillOptions::from_env()?),
        };
        if !options.is_never() && self.resident as u64 > options.byte_size() {
            settle_holds_with_options(self.open.values_mut(), options)?;
            self.resident = self.open.values().map(SpillHold::resident_size).sum();
        }
        Ok(())
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
            if let Some((key, rows)) = self.closed.pop_front() {
                let result = rows.finish().map(|rows| (key, rows)).map_err(Into::into);
                if result.is_err() {
                    self.fuse();
                }
                return Some(result);
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
fn settle_holds_with_options<'a, H: SpillHold + 'a>(
    holds: impl IntoIterator<Item = &'a mut H>,
    options: &SpillOptions,
) -> crate::Result<()> {
    if options.is_never() {
        return Ok(());
    }
    let bound = options.byte_size();
    let bytes = |hold: &H| u64::try_from(hold.resident_size()).unwrap_or(u64::MAX);
    let mut holds: Vec<&mut H> = holds.into_iter().collect();
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
