//! `rust/src/parallel.rs`: the ordered worker pool no caller can name.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use yggdryl::internals::parallel::ordered;

#[test]
fn answers_come_out_in_the_order_they_were_asked_whatever_the_threads() {
    let asked: Vec<u64> = (0..1_000).collect();
    let sequential: Vec<u64> = ordered(asked.clone(), 1, 7, |held| held * 3).collect();
    let spread: Vec<u64> = ordered(asked.clone(), 4, 7, |held| held * 3).collect();
    assert_eq!(
        sequential,
        asked.iter().map(|held| held * 3).collect::<Vec<_>>()
    );
    assert_eq!(spread, sequential);
    assert_eq!(
        ordered(0..0_u64, 3, 7, |held| held).collect::<Vec<_>>(),
        Vec::<u64>::new()
    );
    // Fewer items than lanes, and exactly one chunk.
    assert_eq!(
        ordered(0..2_u64, 4, 7, |held| held + 1).collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(
        ordered(0..7_u64, 4, 7, |held| held).collect::<Vec<_>>(),
        (0..7).collect::<Vec<_>>()
    );
}

thread_local! {
    static COUNTED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[test]
fn what_a_worker_read_stays_with_it_for_the_whole_stream() {
    // Two workers over ten chunks of ten: each worker's count reaches
    // its share of every chunk, because the worker lives for the
    // stream and its thread-local cache with it.
    let counted: Vec<u64> = ordered(0..100_u64, 2, 10, |_| {
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
fn a_stream_is_read_a_lane_depth_ahead_and_no_further() {
    let pulled = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = std::sync::Arc::clone(&pulled);
    let source = (0..100).inspect(move |_| {
        counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    });
    let mut answers = ordered(source, 2, 10, |held| held);
    assert_eq!(answers.next(), Some(0));
    // Two lanes, two chunks each: forty items read for the first answer.
    assert_eq!(pulled.load(std::sync::atomic::Ordering::Relaxed), 40);
    assert_eq!(answers.nth(8), Some(9));
    assert_eq!(pulled.load(std::sync::atomic::Ordering::Relaxed), 40);
    // The first chunk drained, one more is read to keep its lane full.
    assert_eq!(answers.next(), Some(10));
    assert_eq!(pulled.load(std::sync::atomic::Ordering::Relaxed), 50);
}

#[test]
fn a_shallow_pool_completes_out_of_order_and_yields_in_order() {
    let gate = Arc::new((Mutex::new(0_usize), Condvar::new()));
    let trace = Arc::new(Mutex::new(Vec::new()));
    let waited = Arc::clone(&gate);
    let completed = Arc::clone(&gate);
    let traced = Arc::clone(&trace);
    let output: Vec<_> = ordered(0..3, 3, 1, move |item| {
        if item == 0 {
            let (later, wake) = &*waited;
            let (later, _) = wake
                .wait_timeout_while(
                    later.lock().expect("the gate is live"),
                    Duration::from_secs(10),
                    |later| *later < 2,
                )
                .expect("the gate is live");
            assert_eq!(*later, 2, "later jobs must complete before the first");
            traced.lock().expect("the trace is live").push(item);
        } else {
            traced.lock().expect("the trace is live").push(item);
            let (later, wake) = &*completed;
            *later.lock().expect("the gate is live") += 1;
            wake.notify_all();
        }
        item
    })
    .with_lane_depth(1)
    .collect();
    assert_eq!(output, vec![0, 1, 2]);
    let trace = trace.lock().expect("the trace is live");
    assert_eq!(trace.last(), Some(&0));
    assert!(trace[..2].contains(&1) && trace[..2].contains(&2));
}

#[test]
fn a_shallow_pool_reads_at_most_one_batch_per_worker_ahead() {
    let pulled = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&pulled);
    let source = (0..100).inspect(move |_| {
        counted.fetch_add(1, Ordering::Relaxed);
    });
    let mut answers = ordered(source, 3, 1, |item| item).with_lane_depth(1);
    assert_eq!(answers.next(), Some(0));
    assert_eq!(pulled.load(Ordering::Relaxed), 3);
}

#[test]
fn a_worker_that_panics_panics_the_pull_with_its_own_panic() {
    let caught = std::panic::catch_unwind(|| {
        ordered(0..100_u64, 3, 4, |held| {
            assert!(held != 17, "item seventeen refuses");
            held
        })
        .collect::<Vec<_>>()
    });
    let panic = caught.expect_err("the pull panics");
    let text = panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|held| (*held).to_owned()))
        .unwrap_or_default();
    assert!(text.contains("item seventeen refuses"), "{text}");
}

#[test]
fn dropping_the_stream_early_ends_its_workers() {
    let mut answers = ordered(0..1_000_u64, 3, 8, |held| held);
    assert_eq!(answers.next(), Some(0));
    drop(answers);
}

#[test]
fn dropping_a_shallow_pool_joins_its_dispatched_workers() {
    let joined = Arc::new(AtomicUsize::new(0));
    let completed = Arc::clone(&joined);
    let mut answers = ordered(0..1_000_u64, 2, 1, move |held| {
        completed.fetch_add(1, Ordering::Relaxed);
        held
    })
    .with_lane_depth(1);
    assert_eq!(answers.next(), Some(0));
    drop(answers);
    assert_eq!(joined.load(Ordering::Relaxed), 2);
}
