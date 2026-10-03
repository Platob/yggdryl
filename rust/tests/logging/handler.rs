//! `rust/src/logging/handler.rs`: a handler's shared state, its filters
//! and formatter, and the stream and null handlers.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use yggdryl::logging::{Filter, Formatter, Handler, Level, NullHandler, Record, StreamHandler};

use crate::support::Collect;

/// A writer whose bytes the test reads back.
#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Vec<u8>>>);

impl Shared {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().expect("the bytes").clone()).expect("utf-8")
    }
}

impl Write for Shared {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("the bytes").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A writer refusing every byte.
struct Refusing;

impl Write for Refusing {
    fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("the disk is full"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_handler_starts_at_notset_spelling_the_terminal_format_with_no_filter() {
    let collect = Collect::bare();
    assert_eq!(collect.level(), Level::NOTSET);
    assert!(!collect.has_formatter());
    assert_eq!(collect.formatter(), Formatter::terminal());
    let record = Record::new("trades", Level::DEBUG, &"opened")
        .with_location("src/feed.rs", 3)
        .with_thread("main")
        .with_created(1_700_000_000_123_000_000);
    assert!(collect.handle(&record));
    assert_eq!(
        collect.lines(),
        ["2023-11-14 22:13:20,123 · DEBUG    [main] trades feed:3 › opened"]
    );
}

#[test]
fn a_handler_spells_with_the_formatter_it_was_given() {
    let collect = Collect::shared();
    collect.set_level(Level::INFO);
    collect.set_formatter(Formatter::from_str("%(levelname)s %(message)s").expect("a format"));
    assert_eq!(collect.level(), Level::INFO);
    assert!(collect.has_formatter());
    collect.handle(&Record::new("trades", Level::ERROR, &"crossed"));
    assert_eq!(collect.lines(), ["ERROR crossed"]);
}

#[test]
fn a_filter_keeps_a_record_from_the_handler_until_removed() {
    let collect = Collect::shared();
    let quiet: Filter = Arc::new(|record: &Record<'_>| !record.name().starts_with("noisy"));
    collect.add_filter(Arc::clone(&quiet));
    assert!(!collect.handle(&Record::new("noisy.feed", Level::INFO, &"tick")));
    assert!(collect.handle(&Record::new("trades", Level::INFO, &"fill")));
    assert_eq!(collect.lines(), ["fill"]);

    let other: Filter = Arc::new(|_: &Record<'_>| false);
    assert!(!collect.remove_filter(&other), "a filter never added");
    assert!(collect.remove_filter(&quiet));
    assert!(collect.handle(&Record::new("noisy.feed", Level::INFO, &"tick")));
    assert_eq!(collect.lines(), ["tick"]);
}

#[test]
fn a_stream_handler_writes_one_line_per_record() {
    let written = Shared::default();
    let handler = StreamHandler::new(written.clone());
    handler.set_formatter(Formatter::from_str("%(name)s: %(message)s").expect("a format"));
    handler.handle(&Record::new("trades", Level::INFO, &"opened"));
    handler.handle(&Record::new(
        "trades.book",
        Level::WARNING,
        &format_args!("{} levels", 3),
    ));
    handler.flush().expect("a flush");
    assert_eq!(written.text(), "trades: opened\ntrades.book: 3 levels\n");
}

#[test]
fn a_failed_emit_is_said_and_never_reaches_the_caller() {
    let handler = StreamHandler::new(Refusing);
    assert!(handler.handle(&Record::new("trades", Level::ERROR, &"lost")));
}

#[test]
fn a_null_handler_takes_every_record_and_spells_none() {
    struct Loud;
    impl std::fmt::Display for Loud {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("a null handler spells nothing")
        }
    }
    let handler = NullHandler::new();
    assert!(handler.handle(&Record::new("trades", Level::CRITICAL, &Loud)));
    handler.close().expect("nothing to close");
}

#[test]
fn a_record_logged_while_a_handler_spells_gets_its_own_buffer() {
    struct Nested {
        state: yggdryl::logging::HandlerState,
        inner: Arc<Collect>,
    }
    impl Handler for Nested {
        fn state(&self) -> &yggdryl::logging::HandlerState {
            &self.state
        }
        fn emit(&self, _record: &Record<'_>, line: &str) -> yggdryl::Result<()> {
            self.inner
                .handle(&Record::new("inner", Level::INFO, &"nested"));
            self.inner.handle(&Record::new("outer", Level::INFO, &line));
            Ok(())
        }
    }
    let inner = Collect::shared();
    let nested = Nested {
        state: yggdryl::logging::HandlerState::new(),
        inner: Arc::clone(&inner),
    };
    nested.set_formatter(Formatter::default());
    nested.handle(&Record::new("outer", Level::INFO, &"outside"));
    assert_eq!(inner.lines(), ["nested", "outside"]);
}

#[test]
fn a_record_logged_from_a_thread_local_drop_is_spelled_after_the_buffers_are_gone() {
    /// Logs as the thread's storage is torn down: registered before the
    /// handler's buffers, so dropped after them.
    struct Farewell(Arc<Collect>);
    impl Drop for Farewell {
        fn drop(&mut self) {
            self.0
                .handle(&Record::new("teardown", Level::INFO, &"closing\tpool"));
        }
    }
    thread_local! {
        static FAREWELL: std::cell::RefCell<Option<Farewell>> = const { std::cell::RefCell::new(None) };
    }
    let collect = Arc::new(Collect::default());
    collect.set_formatter(Formatter::from_str("%(threadName)s %(message)r").expect("a format"));
    let held = Arc::clone(&collect);
    std::thread::spawn(move || {
        FAREWELL.with_borrow_mut(|farewell| *farewell = Some(Farewell(Arc::clone(&held))));
        yggdryl::logging::set_thread_name("pool");
        held.handle(&Record::new("teardown", Level::INFO, &"opened"));
    })
    .join()
    .expect("a thread whose storage logged on its way out");
    let lines = collect.lines();
    assert_eq!(lines[0], "pool 'opened'");
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[1].ends_with(" 'closing\\tpool'"), "{lines:?}");
}

#[test]
fn a_writer_that_logs_into_its_own_handler_writes_that_line_after_its_own() {
    use std::sync::{OnceLock, Weak};

    /// A writer that, writing its first line, logs a line of its own.
    struct Echo {
        bytes: Shared,
        handler: Arc<OnceLock<Weak<StreamHandler>>>,
        echoed: bool,
    }
    impl Write for Echo {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if !std::mem::replace(&mut self.echoed, true)
                && let Some(handler) = self.handler.get().and_then(Weak::upgrade)
            {
                handler.handle(&Record::new("echo", Level::INFO, &"nested"));
            }
            self.bytes
                .0
                .lock()
                .expect("the bytes")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let bytes = Shared::default();
    let slot = Arc::new(OnceLock::new());
    let handler = Arc::new(StreamHandler::new(Echo {
        bytes: bytes.clone(),
        handler: Arc::clone(&slot),
        echoed: false,
    }));
    handler.set_formatter(Formatter::default());
    slot.set(Arc::downgrade(&handler)).expect("one handler");
    handler.handle(&Record::new("echo", Level::INFO, &"outer"));
    assert_eq!(
        bytes.text(),
        "outer\nnested\n",
        "never waiting on its own lock"
    );
}
