//! Work spread over threads, answered in the order it was asked.
//!
//! A capture is a stream of independent lines and a machine has cores, so
//! a door built over this reads its stream a chunk at a time, hands each
//! chunk's items to the threads and yields their answers in the items'
//! order: it answers exactly what its sequential twin answers, sooner, and
//! stays a stream - one chunk in hand, nothing collected. The threads are
//! scoped to the chunk, so an item borrowed from the caller crosses to a
//! thread and back without being made `'static`, and nothing outlives the
//! pull that spawned it. One thread is the sequential map, item by item,
//! and spawns nothing.
//!
//! What a thread reads as it works - the caches a [`Warm`] names - would
//! die with the chunk's thread, so it is taken out of each thread as it
//! finishes and put into the one spawned in its place for the next chunk:
//! a stream read on four threads warms four sets of caches once, and not
//! one set per chunk.

use std::collections::VecDeque;
use std::iter::Fuse;

/// What a thread keeps warm between chunks: the caches a worker fills as
/// it works, taken out of the thread that dies with its chunk and handed
/// to the one spawned for the next.
pub(crate) trait Warm: Send + Sized {
    /// The calling thread's caches, taken: the thread is left cold.
    fn take() -> Self;

    /// `self` made the calling thread's caches.
    fn restore(self);
}

/// Nothing kept warm: work that reads no cache.
impl Warm for () {
    fn take() -> Self {}

    fn restore(self) {}
}

/// The caches one thread of a run holds between chunks, by the run's
/// thread: none until the thread's first chunk, and the calling thread's
/// own are never among them.
pub(crate) type Slots<W> = Vec<Option<W>>;

/// `threads` slots, each cold.
pub(crate) fn slots<W: Warm>(threads: usize) -> Slots<W> {
    (0..threads.max(1)).map(|_| None).collect()
}

/// `work` over every item of `source`, on `threads` threads, in order.
///
/// A chunk holds `chunk` items, which is how far the stream is read ahead;
/// every item is answered once, on some thread, and the answers come out
/// in the items' order. One thread, or a chunk of one item, answers each
/// item as it is pulled. A thread that panics panics the pull. What each
/// thread reads as it works is kept warm between chunks as `W` names.
pub(crate) fn ordered<W, I, R, F>(
    source: I,
    threads: usize,
    chunk: usize,
    work: F,
) -> Ordered<W, I::IntoIter, R, F>
where
    W: Warm,
    I: IntoIterator,
    I::Item: Send,
    R: Send,
    F: Fn(I::Item) -> R + Sync,
{
    Ordered {
        source: source.into_iter().fuse(),
        threads: threads.max(1),
        chunk: chunk.max(1),
        work,
        answered: VecDeque::new(),
        slots: slots(threads),
    }
}

/// The stream [`ordered`] answers.
pub(crate) struct Ordered<W: Warm, I: Iterator, R, F> {
    source: Fuse<I>,
    threads: usize,
    chunk: usize,
    work: F,
    /// The chunk in hand, its answers in order.
    answered: VecDeque<R>,
    slots: Slots<W>,
}

impl<W, I, R, F> Iterator for Ordered<W, I, R, F>
where
    W: Warm,
    I: Iterator,
    I::Item: Send,
    R: Send,
    F: Fn(I::Item) -> R + Sync,
{
    type Item = R;

    fn next(&mut self) -> Option<R> {
        if self.threads == 1 {
            return self.source.next().map(&self.work);
        }
        if let Some(answer) = self.answered.pop_front() {
            return Some(answer);
        }
        let items: Vec<I::Item> = self.source.by_ref().take(self.chunk).collect();
        if items.is_empty() {
            return None;
        }
        let work = &self.work;
        let answered = over(items, self.threads, &mut self.slots, |part| {
            part.into_iter().map(work).collect::<Vec<R>>()
        });
        for answers in answered {
            self.answered.extend(answers);
        }
        self.answered.pop_front()
    }
}

/// `work` over the indices `0..count`, on `threads` threads, answered in
/// index order: what a batch already in hand is read by, each row on some
/// thread, the threads' caches kept warm in `slots` between runs.
pub(crate) fn mapped<W, R, F>(count: usize, threads: usize, slots: &mut Slots<W>, work: F) -> Vec<R>
where
    W: Warm,
    R: Send,
    F: Fn(usize) -> R + Sync,
{
    if threads <= 1 || count <= 1 {
        return (0..count).map(work).collect();
    }
    let indices: Vec<usize> = (0..count).collect();
    over(indices, threads, slots, |part| {
        part.into_iter().map(&work).collect::<Vec<R>>()
    })
    .into_iter()
    .flatten()
    .collect()
}

/// `items` split into at most `threads` runs, each run answered on its own
/// scoped thread, the answers in the runs' order; the thread of run `i`
/// starts with the caches `slots[i]` holds and leaves its own there.
fn over<W, T, R, F>(items: Vec<T>, threads: usize, slots: &mut Slots<W>, work: F) -> Vec<R>
where
    W: Warm,
    T: Send,
    R: Send,
    F: Fn(Vec<T>) -> R + Sync,
{
    let share = items.len().div_ceil(threads.max(1)).max(1);
    let mut parts = Vec::with_capacity(items.len().div_ceil(share));
    let mut rest = items;
    while !rest.is_empty() {
        let tail = rest.split_off(rest.len().min(share));
        parts.push(rest);
        rest = tail;
    }
    if parts.len() == 1 {
        return parts.into_iter().map(work).collect();
    }
    if slots.len() < parts.len() {
        slots.resize_with(parts.len(), || None);
    }
    std::thread::scope(|scope| {
        let work = &work;
        let handles: Vec<_> = parts
            .into_iter()
            .zip(slots.iter_mut())
            .map(|(part, slot)| {
                let warmth = slot.take();
                let handle = scope.spawn(move || {
                    if let Some(warmth) = warmth {
                        warmth.restore();
                    }
                    (work(part), W::take())
                });
                (slot, handle)
            })
            .collect();
        handles
            .into_iter()
            .map(|(slot, handle)| {
                let (answer, warmth) = handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
                *slot = Some(warmth);
                answer
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::{Warm, mapped, ordered, slots};

    #[test]
    fn answers_come_out_in_the_order_they_were_asked_whatever_the_threads() {
        let asked: Vec<u64> = (0..1_000).collect();
        let sequential: Vec<u64> =
            ordered::<(), _, _, _>(asked.clone(), 1, 7, |held| held * 3).collect();
        let spread: Vec<u64> =
            ordered::<(), _, _, _>(asked.clone(), 4, 7, |held| held * 3).collect();
        assert_eq!(
            sequential,
            asked.iter().map(|held| held * 3).collect::<Vec<_>>()
        );
        assert_eq!(spread, sequential);
        assert_eq!(
            mapped(1_000, 3, &mut slots::<()>(3), |at| at * 2),
            (0..2_000).step_by(2).collect::<Vec<_>>()
        );
        assert_eq!(
            mapped(0, 3, &mut slots::<()>(3), |at| at),
            Vec::<usize>::new()
        );
    }

    thread_local! {
        static COUNTED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    }

    /// One thread's count of the items it answered, kept between chunks.
    struct Counted(u64);

    impl Warm for Counted {
        fn take() -> Self {
            Self(COUNTED.with(|held| held.replace(0)))
        }

        fn restore(self) {
            COUNTED.with(|held| held.set(self.0));
        }
    }

    #[test]
    fn what_a_thread_read_is_kept_warm_between_chunks() {
        // Two threads over ten chunks of ten: each thread's count reaches
        // its chunk share summed over every chunk, not one chunk's.
        let counted: Vec<u64> = ordered::<Counted, _, _, _>(0..100_u64, 2, 10, |_| {
            COUNTED.with(|held| {
                held.set(held.get() + 1);
                held.get()
            })
        })
        .collect();
        assert_eq!(counted.iter().max(), Some(&50));
        assert_eq!(counted.iter().filter(|held| **held == 1).count(), 2);
        assert_eq!(
            COUNTED.with(std::cell::Cell::get),
            0,
            "the caller's own thread is never among them"
        );
    }

    #[test]
    fn a_stream_is_read_a_chunk_ahead_and_no_further() {
        let pulled = std::sync::atomic::AtomicUsize::new(0);
        let source = (0..100).inspect(|_| {
            pulled.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        });
        let mut answers = ordered::<(), _, _, _>(source, 2, 10, |held| held);
        assert_eq!(answers.next(), Some(0));
        assert_eq!(pulled.load(std::sync::atomic::Ordering::Relaxed), 10);
        assert_eq!(answers.nth(8), Some(9));
        assert_eq!(pulled.load(std::sync::atomic::Ordering::Relaxed), 10);
        assert_eq!(answers.next(), Some(10));
        assert_eq!(pulled.load(std::sync::atomic::Ordering::Relaxed), 20);
    }
}
