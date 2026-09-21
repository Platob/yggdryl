//! Work spread over threads, answered in the order it was asked.
//!
//! A capture is a stream of independent lines and a machine has cores, so
//! a door built over this reads its stream a chunk at a time, hands each
//! chunk to a worker and yields the answers in the items' order: it
//! answers exactly what its sequential twin answers, sooner, and stays a
//! stream - a few chunks in hand, nothing collected. One thread is the
//! sequential map, item by item, and spawns nothing.
//!
//! The workers live for the whole stream. Each owns one lane - the chunks
//! it was handed, in order - and the chunks go round the lanes in turn, so
//! chunk `k` is worked by lane `k % threads` and drained from it: every
//! lane keeps its own order and the round keeps the lanes', which is the
//! items' order with no sorting and no sequence number. A lane holds at
//! most [`LANE_DEPTH`] chunks, the one being worked and the one behind it,
//! so the thread that pulls reads the next chunk off the source while the
//! workers work the ones before it, and what a consumer does with each
//! answer overlaps the work on the answers after it. What a worker reads
//! as it works - the thread-local caches a message read fills - lives as
//! long as the worker does: a stream read on four threads warms four sets
//! of caches once.
//!
//! An item crosses to a worker by value and its answer comes back the
//! same way, so both are owned: a door that reads borrowed lines makes
//! them its own before it spreads them, exactly as it would to keep one.
//! A worker that panics panics the pull that drains its chunk, with the
//! panic it raised.

use std::any::Any;
use std::collections::VecDeque;
use std::iter::Fuse;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

/// How many line chunks one lane holds: the chunk its worker is working,
/// and the one waiting behind it, so the worker never idles while the puller
/// reads the next. Whole-batch jobs override this with one.
pub(crate) const LANE_DEPTH: usize = 2;

/// `work` over every item of `source`, on `threads` threads, in order.
///
/// A chunk holds `chunk` items; up to `threads * LANE_DEPTH` chunks are
/// read ahead. Every item is answered once, on some thread, and the
/// answers come out in the items' order. One thread answers each item as
/// it is pulled and reads nothing ahead. A thread that panics panics the
/// pull that drains its chunk.
pub fn ordered<I, R, F>(
    source: I,
    threads: usize,
    chunk: usize,
    work: F,
) -> Ordered<I::IntoIter, R, F>
where
    I: IntoIterator,
    I::Item: Send + 'static,
    R: Send + 'static,
    F: Fn(I::Item) -> R + Send + Sync + 'static,
{
    Ordered {
        source: source.into_iter().fuse(),
        threads: threads.max(1),
        chunk: chunk.max(1),
        lane_depth: LANE_DEPTH,
        work: Arc::new(work),
        answered: VecDeque::new(),
        lanes: None,
        exhausted: false,
    }
}

/// The stream [`ordered`] answers.
pub struct Ordered<I: Iterator, R, F> {
    source: Fuse<I>,
    threads: usize,
    chunk: usize,
    lane_depth: usize,
    work: Arc<F>,
    /// The chunk drained last, its answers still to be yielded, in order.
    answered: VecDeque<R>,
    /// The workers, spawned on the first pull that needs them.
    lanes: Option<Lanes<I::Item, R>>,
    /// Whether the source answered its last item.
    exhausted: bool,
}

impl<I: Iterator, R, F> Ordered<I, R, F> {
    /// Limits the chunks each worker may hold. Batch jobs use one: each job
    /// already owns a whole input batch, while line chunks retain the normal
    /// read-ahead depth.
    #[must_use]
    pub fn with_lane_depth(mut self, lane_depth: usize) -> Self {
        self.lane_depth = lane_depth.max(1);
        self
    }
}

impl<I, R, F> Iterator for Ordered<I, R, F>
where
    I: Iterator,
    I::Item: Send + 'static,
    R: Send + 'static,
    F: Fn(I::Item) -> R + Send + Sync + 'static,
{
    type Item = R;

    fn next(&mut self) -> Option<R> {
        if self.threads == 1 {
            return self.source.next().map(|item| (self.work)(item));
        }
        loop {
            if let Some(answer) = self.answered.pop_front() {
                return Some(answer);
            }
            let (threads, chunk) = (self.threads, self.chunk);
            let work = &self.work;
            let lanes = self
                .lanes
                .get_or_insert_with(|| Lanes::spawn(threads, work));
            // Every lane full, while the source lasts: the puller reads
            // ahead exactly what the workers can hold, and no further.
            while !self.exhausted && lanes.in_flight() < threads.saturating_mul(self.lane_depth) {
                let items: Vec<I::Item> = self.source.by_ref().take(chunk).collect();
                if items.is_empty() {
                    self.exhausted = true;
                    break;
                }
                lanes.dispatch(items);
            }
            if lanes.in_flight() == 0 {
                return None;
            }
            self.answered.extend(lanes.drain());
        }
    }
}

/// What one worker answers for one chunk: the answers in the chunk's
/// order, or the panic it raised working them.
type Answered<R> = Result<Vec<R>, Box<dyn Any + Send>>;

/// The workers of one stream, each behind its lane.
struct Lanes<T, R> {
    lanes: Vec<Lane<T, R>>,
    /// How many chunks were handed out, which names the lane the next goes
    /// to.
    dispatched: usize,
    /// How many chunks were drained, which names the lane the next comes
    /// from.
    drained: usize,
}

/// One worker: the chunks handed to it, in order, and its answers to them.
struct Lane<T, R> {
    tasks: Option<Sender<Vec<T>>>,
    answers: Receiver<Answered<R>>,
    worker: Option<JoinHandle<()>>,
}

impl<T, R> Lanes<T, R>
where
    T: Send + 'static,
    R: Send + 'static,
{
    fn spawn<F>(threads: usize, work: &Arc<F>) -> Self
    where
        F: Fn(T) -> R + Send + Sync + 'static,
    {
        let lanes = (0..threads)
            .map(|_| {
                let (tasks, chunks) = channel::<Vec<T>>();
                let (answers, drained) = channel::<Answered<R>>();
                let work = Arc::clone(work);
                let worker = std::thread::spawn(move || {
                    while let Ok(items) = chunks.recv() {
                        let answered = catch_unwind(AssertUnwindSafe(|| {
                            items.into_iter().map(&*work).collect::<Vec<R>>()
                        }));
                        let panicked = answered.is_err();
                        if answers.send(answered).is_err() || panicked {
                            return;
                        }
                    }
                });
                Lane {
                    tasks: Some(tasks),
                    answers: drained,
                    worker: Some(worker),
                }
            })
            .collect();
        Self {
            lanes,
            dispatched: 0,
            drained: 0,
        }
    }

    /// How many chunks are handed out and not yet drained.
    const fn in_flight(&self) -> usize {
        self.dispatched - self.drained
    }

    /// Hands one chunk to the next lane round.
    ///
    /// A lane whose worker died refuses the chunk; the panic that killed
    /// it is raised by the drain of the chunk it died on, which comes
    /// first.
    fn dispatch(&mut self, items: Vec<T>) {
        let at = self.dispatched % self.lanes.len();
        if let Some(tasks) = &self.lanes[at].tasks {
            let _ = tasks.send(items);
        }
        self.dispatched += 1;
    }

    /// The answers of the oldest chunk still out, in its order.
    fn drain(&mut self) -> Vec<R> {
        let at = self.drained % self.lanes.len();
        self.drained += 1;
        match self.lanes[at].answers.recv() {
            Ok(Ok(answers)) => answers,
            Ok(Err(panic)) => resume_unwind(panic),
            // A worker answers every chunk it takes, panic included, and
            // leaves only after; a lane that went quiet lost its worker to
            // something no unwind reports.
            Err(_) => panic!("a worker thread ended without answering its chunk"),
        }
    }
}

impl<T, R> Drop for Lane<T, R> {
    fn drop(&mut self) {
        // Closing the lane is what ends the worker: it drains what it was
        // handed and returns. Joined, so nothing outlives the stream.
        drop(self.tasks.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/parallel.rs` pins and a caller cannot reach.
    //!
    //! The ordered worker pool is how codec parsing spreads per-event work
    //! without reordering it, and its read-ahead, its panics and its joins
    //! are only observable from the pull itself. The crate root declares
    //! `parallel` privately, so these reach nobody without the feature.
    pub use super::{Ordered, ordered};
}
