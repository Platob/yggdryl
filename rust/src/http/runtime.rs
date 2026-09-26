//! The one private runtime HTTP/2 and HTTP/3 connections are driven on, and
//! the wait the blocking API spends on their futures.
//!
//! The crate's HTTP API is synchronous: a call returns when its answer is in.
//! A multiplexed connection is not - its frames arrive for every stream at
//! once - so each connection's driver runs as a task on a small runtime of
//! its own, started on the first multiplexed request and shared by every
//! client in the process. A caller waits on the future it needs by polling it
//! on its own thread and parking between wakes, with the runtime entered so
//! a socket the future creates registers with it: no task, no channel and no
//! thread hop per chunk of a body, and nothing a caller has to bring.
//! [`wait`] works from any thread, a thread inside another runtime included,
//! where that runtime's own `block_on` would refuse.
//!
//! A deadline the waiting future holds is [`within`]'s, kept by the waiting
//! thread itself: it parks no longer than the nearest one its last poll
//! named. A runtime timer armed off the runtime's threads would wake a
//! worker to re-arm its driver - a thread hop per request for nothing.

use std::cell::Cell;
use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::Thread;
use std::time::{Duration, Instant};

use tokio::runtime::{Builder, Handle, Runtime};

/// The fewest and the most threads the runtime drives connections on.
const WORKERS: (usize, usize) = (2, 4);

/// The runtime, built on first use; the text of the failure when the
/// operating system refused its threads.
fn built() -> &'static std::result::Result<Runtime, String> {
    static RUNTIME: OnceLock<std::result::Result<Runtime, String>> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        let workers = std::thread::available_parallelism()
            .map_or(WORKERS.0, std::num::NonZero::get)
            .clamp(WORKERS.0, WORKERS.1);
        Builder::new_multi_thread()
            .worker_threads(workers)
            .thread_name("yggdryl-http")
            .enable_io()
            .enable_time()
            .build()
            .map_err(|error| error.to_string())
    })
}

/// The runtime's handle, for spawning a connection's driver.
///
/// # Errors
///
/// An [`std::io::Error`] when the runtime could not start its threads.
pub(crate) fn handle() -> std::io::Result<&'static Handle> {
    built()
        .as_ref()
        .map(Runtime::handle)
        .map_err(|error| std::io::Error::other(format!("the HTTP runtime did not start: {error}")))
}

thread_local! {
    /// The nearest deadline a [`within`] polled on this thread stands at,
    /// taken by the [`wait`] that polled it.
    static NEAREST: Cell<Option<Instant>> = const { Cell::new(None) };
    /// The waker of this thread's [`wait`], made once.
    static WAKER: Waker = Waker::from(Arc::new(Unpark(std::thread::current())));
}

/// Unparks the thread that is waiting on a future.
struct Unpark(Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

/// Poll `future` to completion on this thread, parked between wakes, with
/// the runtime entered.
///
/// # Errors
///
/// An [`std::io::Error`] when the runtime could not start its threads.
pub(crate) fn wait<F: Future>(future: F) -> std::io::Result<F::Output> {
    let _entered = handle()?.enter();
    let waker = WAKER
        .try_with(Waker::clone)
        .unwrap_or_else(|_| Waker::from(Arc::new(Unpark(std::thread::current()))));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        // A wait inside a future another wait polls leaves the outer one's
        // deadline to it.
        let outer = NEAREST.take();
        let polled = future.as_mut().poll(&mut context);
        let nearest = NEAREST.replace(outer);
        if let Poll::Ready(output) = polled {
            return Ok(output);
        }
        // A wake between the poll and the park leaves the token set, so the
        // park returns at once; a spurious return polls again, and so does
        // one at a deadline, which the `within` holding it then answers.
        match nearest {
            None => std::thread::park(),
            Some(deadline) => {
                let left = deadline.saturating_duration_since(Instant::now());
                if !left.is_zero() {
                    std::thread::park_timeout(left);
                }
            }
        }
    }
}

/// `future`, or `None` once `timeout` has passed since the call.
///
/// The deadline is checked each time the future is polled and handed to the
/// [`wait`] polling it, which parks no longer than that: a `within` is only
/// ever polled under a `wait`, never in a runtime task, where nothing would
/// poll it again at its deadline.
pub(crate) async fn within<F: Future>(timeout: Duration, future: F) -> Option<F::Output> {
    let deadline = Instant::now().checked_add(timeout);
    let mut future = pin!(future);
    poll_fn(|context| {
        if let Poll::Ready(output) = future.as_mut().poll(context) {
            return Poll::Ready(Some(output));
        }
        let Some(deadline) = deadline else {
            // Past what the clock can name: no deadline at all.
            return Poll::Pending;
        };
        if Instant::now() >= deadline {
            return Poll::Ready(None);
        }
        NEAREST.set(Some(
            NEAREST
                .get()
                .map_or(deadline, |nearest| nearest.min(deadline)),
        ));
        Poll::Pending
    })
    .await
}

/// [`wait`] for `future`, giving up after `timeout`: `None` when it ran out.
///
/// # Errors
///
/// As [`wait`].
pub(crate) fn wait_for<F: Future>(
    timeout: Duration,
    future: F,
) -> std::io::Result<Option<F::Output>> {
    wait(within(timeout, future))
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/http/runtime.rs` pins and a caller cannot reach.
    use std::time::Duration;

    /// Wait on a future a runtime task completes after `pause`, from this
    /// thread: the answer it sent.
    pub fn wait_on_a_task(pause: Duration, answer: u64) -> std::io::Result<u64> {
        let task = super::handle()?.spawn(async move {
            tokio::time::sleep(pause).await;
            answer
        });
        super::wait(task)?.map_err(std::io::Error::other)
    }

    /// Wait at most `timeout` for a timer of `pause`: whether it fired.
    pub fn wait_for_a_timer(timeout: Duration, pause: Duration) -> std::io::Result<bool> {
        // The timer is made inside the wait, where the runtime is entered.
        Ok(super::wait_for(timeout, async move { tokio::time::sleep(pause).await })?.is_some())
    }

    /// The name of the thread a runtime task runs on.
    pub fn worker_name() -> std::io::Result<Option<String>> {
        let task =
            super::handle()?.spawn(async { std::thread::current().name().map(str::to_owned) });
        super::wait(task)?.map_err(std::io::Error::other)
    }
}
