//! `rust/src/http/runtime.rs`: the one runtime HTTP/2 and HTTP/3 connections
//! are driven on, and the wait a blocking call spends on a future.

use std::time::{Duration, Instant};

use yggdryl::internals::http_runtime::{wait_for_a_timer, wait_on_a_task, worker_name};

#[test]
fn a_thread_waits_on_a_task_the_runtime_completes() {
    assert_eq!(wait_on_a_task(Duration::from_millis(20), 7).unwrap(), 7);
    assert_eq!(worker_name().unwrap().as_deref(), Some("yggdryl-http"));
}

#[test]
fn a_wait_gives_up_at_its_timeout_and_not_before() {
    let started = Instant::now();
    assert!(!wait_for_a_timer(Duration::from_millis(50), Duration::from_secs(5)).unwrap());
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(50) && waited < Duration::from_secs(2),
        "{waited:?}"
    );
    assert!(wait_for_a_timer(Duration::from_secs(5), Duration::from_millis(10)).unwrap());
}

#[test]
fn a_thread_inside_another_runtime_waits_too() {
    // Where that runtime's own `block_on` would refuse, the wait parks.
    let other = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a runtime");
    let answer = other.block_on(async { wait_on_a_task(Duration::from_millis(5), 11) });
    assert_eq!(answer.unwrap(), 11);
}
