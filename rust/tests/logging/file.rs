//! `rust/src/logging/file.rs`: a handler writing through a storage handle,
//! holding records back until its capacity or flush level says publish.

use yggdryl::holder::Buffer;
use yggdryl::logging::{FileHandler, Formatter, Handler, Level, Record};

/// `handler` spelling each record's message alone.
fn plain<H: IOBase>(handler: FileHandler<H>) -> FileHandler<H> {
    handler.set_formatter(Formatter::default());
    handler
}
use yggdryl::{IOBase, IOMode};

fn read(handler: &FileHandler<Buffer>) -> String {
    String::from_utf8(handler.io().read_all_bytes().expect("the bytes")).expect("utf-8")
}

#[test]
fn each_record_is_appended_as_it_arrives_by_default() {
    let mut buffer = Buffer::new();
    buffer.write_all_bytes(b"kept\n").expect("a write");
    let handler = plain(FileHandler::new(buffer));
    assert_eq!(handler.mode(), IOMode::Append);
    assert_eq!(handler.capacity(), 0);
    assert_eq!(handler.flush_level(), Level::ERROR);
    handler.handle(&Record::new("trades", Level::INFO, &"opened"));
    assert_eq!(read(&handler), "kept\nopened\n");
    handler.handle(&Record::new("trades", Level::DEBUG, &"tick"));
    assert_eq!(read(&handler), "kept\nopened\ntick\n");
}

#[test]
fn a_capacity_holds_records_until_it_is_reached_or_a_record_is_severe() {
    let handler = plain(
        FileHandler::new(Buffer::new())
            .with_capacity(16)
            .with_flush_level(Level::WARNING),
    );
    handler.handle(&Record::new("trades", Level::INFO, &"one"));
    handler.handle(&Record::new("trades", Level::INFO, &"two"));
    assert_eq!(read(&handler), "", "8 bytes held");
    handler.handle(&Record::new("trades", Level::INFO, &"three four"));
    assert_eq!(
        read(&handler),
        "one\ntwo\nthree four\n",
        "19 bytes published"
    );
    handler.handle(&Record::new("trades", Level::INFO, &"five"));
    handler.handle(&Record::new("trades", Level::WARNING, &"late"));
    assert_eq!(read(&handler), "one\ntwo\nthree four\nfive\nlate\n");
    handler.handle(&Record::new("trades", Level::INFO, &"held"));
    handler.flush().expect("a flush");
    assert!(read(&handler).ends_with("late\nheld\n"));
}

#[test]
fn an_overwrite_replaces_the_handle_once() {
    let mut buffer = Buffer::new();
    buffer.write_all_bytes(b"yesterday\n").expect("a write");
    let handler = plain(
        FileHandler::new(buffer)
            .with_mode(IOMode::Overwrite)
            .expect("an overwrite"),
    );
    assert_eq!(
        read(&handler),
        "yesterday\n",
        "nothing replaced before a record"
    );
    handler.handle(&Record::new("trades", Level::INFO, &"today"));
    assert_eq!(read(&handler), "today\n");
    handler.close().expect("a close");
    handler.handle(&Record::new("trades", Level::INFO, &"reopened"));
    assert_eq!(read(&handler), "today\nreopened\n");
}

#[test]
fn a_mode_writing_no_lines_is_refused() {
    for mode in [IOMode::Merge, IOMode::ReadOnly, IOMode::Random] {
        let error = FileHandler::new(Buffer::new())
            .with_mode(mode)
            .expect_err("a mode a log cannot take")
            .to_string();
        assert!(error.contains("expected append or overwrite"), "{error}");
    }
}

#[test]
fn the_handler_spells_with_its_formatter_and_takes_a_spelled_line() {
    let handler = plain(FileHandler::new(Buffer::new()));
    handler.set_formatter(Formatter::from_str("%(levelname)s %(message)s").expect("a format"));
    handler.handle(&Record::new("trades", Level::INFO, &"opened"));
    handler
        .write("spelled by a host", Level::INFO)
        .expect("a line");
    assert_eq!(read(&handler), "INFO opened\nspelled by a host\n");
}

#[test]
fn a_dropped_handler_publishes_what_it_held_to_a_local_file() {
    let root = yggdryl::local::LocalFolder::temporary()
        .and_then(|folder| folder.path())
        .expect("a temporary folder")
        .join(format!("yggdryl-logging-file-{}", std::process::id()));
    let path = root.join("logs").join("feed.log");
    let handler = plain(
        FileHandler::new(yggdryl::holder::Holder::local(&path).expect("a location"))
            .with_capacity(1024),
    );
    handler.handle(&Record::new("trades", Level::INFO, &"held"));
    assert!(!path.exists(), "nothing written while held");
    drop(handler);
    assert_eq!(std::fs::read_to_string(&path).expect("the log"), "held\n");
    std::fs::remove_dir_all(&root).expect("a cleanup");
}

/// A handle whose every append logs, as a store's credential refresh does.
#[derive(Debug)]
struct Talkative {
    handle: Buffer,
}

impl yggdryl::IOMedia for Talkative {
    yggdryl::impl_default_iomedia!();
}

impl IOBase for Talkative {
    yggdryl::delegate_iobase!(handle: pread, read_all_bytes, read_range_bytes, pwrite, size,
        capacity, reserve, truncate, uri, url, media_type, set_media_type, flush, open, opened,
        close, parent, child_by_path, ls, kind);

    fn append_bytes(&mut self, bytes: &[u8]) -> yggdryl::Result<u64> {
        yggdryl::logging::get_logger("file.talkative").warning("kept the token in hand");
        self.handle.append_bytes(bytes)
    }
}

#[test]
fn a_record_its_own_publish_logs_waits_for_the_next_publish_instead_of_itself() {
    use std::sync::Arc;

    let _tree = crate::support::serial();
    let logger = yggdryl::logging::get_logger("file.talkative");
    logger.set_propagating(false);
    let handler = Arc::new(plain(FileHandler::new(Talkative {
        handle: Buffer::new(),
    })));
    logger.add_handler(handler.clone());

    logger.warning("first");
    let read = |handler: &FileHandler<Talkative>| {
        String::from_utf8(handler.io().read_all_bytes().expect("the bytes")).expect("utf-8")
    };
    assert_eq!(
        read(&handler),
        "first\n",
        "the warning its publish raised is held aside"
    );
    logger.warning("second");
    assert_eq!(
        read(&handler),
        "first\nkept the token in hand\nsecond\n",
        "carried by the next publish, in order"
    );

    // Logging while holding the handle is the same thread asking twice.
    {
        let _held = handler.io();
        logger.error("while held");
    }
    handler.flush().expect("a flush");
    assert_eq!(
        read(&handler),
        "first\nkept the token in hand\nsecond\nkept the token in hand\nwhile held\n",
        "the flush's own warning waits for the next publish"
    );
    logger.remove_handler(&(handler as Arc<dyn Handler>));
}

#[test]
fn a_closed_handler_publishes_each_record_at_once() {
    let handler = plain(FileHandler::new(Buffer::new()).with_capacity(1 << 20));
    handler.handle(&Record::new("trades", Level::INFO, &"held"));
    handler.close().expect("a close");
    assert_eq!(read(&handler), "held\n");
    handler.handle(&Record::new("trades", Level::INFO, &"after the close"));
    assert_eq!(read(&handler), "held\nafter the close\n");
}

#[test]
fn a_handler_stating_no_formatter_writes_the_terminal_line_plain() {
    let handler = FileHandler::new(Buffer::new());
    let record = Record::new("trades", Level::ERROR, &"rejected")
        .with_location("src/feed.rs", 12)
        .with_function("submit")
        .with_thread("main")
        .with_created(1_700_000_000_123_000_000);
    handler.handle(&record);
    assert_eq!(
        read(&handler),
        "2023-11-14 22:13:20,123 ✗ ERROR    [main] trades submit:12 › rejected\n"
    );
}

/// A handle whose first append waits for another thread to log into the
/// same handler, as a store's runtime does while it drives the request.
struct Driven {
    handle: Buffer,
    handler: std::sync::Arc<std::sync::OnceLock<std::sync::Weak<FileHandler<Driven>>>>,
    driven: bool,
}

impl std::fmt::Debug for Driven {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Driven")
    }
}

impl yggdryl::IOMedia for Driven {
    yggdryl::impl_default_iomedia!();
}

impl IOBase for Driven {
    yggdryl::delegate_iobase!(handle: pread, read_all_bytes, read_range_bytes, pwrite, size,
        capacity, reserve, truncate, uri, url, media_type, set_media_type, flush, open, opened,
        close, parent, child_by_path, ls, kind);

    fn append_bytes(&mut self, bytes: &[u8]) -> yggdryl::Result<u64> {
        if !std::mem::replace(&mut self.driven, true) {
            let handler = self.handler.get().and_then(std::sync::Weak::upgrade);
            let (said, logged) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                if let Some(handler) = handler {
                    handler.handle(&Record::new("runtime", Level::INFO, &"driven"));
                }
                let _ = said.send(());
            });
            // The publish waits on the other thread, as a request waits on
            // the runtime driving it: a record that waited on the publish
            // would never come back.
            assert!(
                logged
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .is_ok(),
                "a record on another thread waited on the publish in flight"
            );
        }
        self.handle.append_bytes(bytes)
    }
}

#[test]
fn a_record_on_another_thread_never_waits_on_a_publish_and_goes_with_it() {
    use std::sync::{Arc, OnceLock};

    let slot = Arc::new(OnceLock::new());
    let handler = Arc::new(plain(FileHandler::new(Driven {
        handle: Buffer::new(),
        handler: Arc::clone(&slot),
        driven: false,
    })));
    slot.set(Arc::downgrade(&handler)).expect("one handler");
    handler.handle(&Record::new("feed", Level::INFO, &"first"));
    let written =
        String::from_utf8(handler.io().read_all_bytes().expect("the bytes")).expect("utf-8");
    assert_eq!(
        written, "first\ndriven\n",
        "published by the thread that was publishing, before it let go"
    );
}

#[test]
fn a_flush_or_a_close_under_the_threads_own_guard_is_refused_not_waited_on() {
    let handler = plain(FileHandler::new(Buffer::new()).with_capacity(1024));
    handler.handle(&Record::new("trades", Level::INFO, &"held"));
    {
        let _held = handler.io();
        for refused in [handler.flush(), handler.close()] {
            let error = refused.expect_err("the thread holds the handle");
            assert!(
                matches!(&error, yggdryl::Error::Io(io) if io.kind() == std::io::ErrorKind::Deadlock),
                "{error}"
            );
        }
        assert!(format!("{handler:?}").contains("pending: 5"), "{handler:?}");
    }
    handler.flush().expect("a flush once let go");
    assert_eq!(read(&handler), "held\n");
}

#[test]
fn a_close_publishes_what_its_own_publish_raised() {
    use std::sync::Arc;

    let _tree = crate::support::serial();
    let logger = yggdryl::logging::get_logger("file.talkative");
    logger.set_propagating(false);
    let handler = Arc::new(plain(
        FileHandler::new(Talkative {
            handle: Buffer::new(),
        })
        .with_capacity(1024),
    ));
    logger.add_handler(handler.clone());
    logger.warning("held");
    handler.close().expect("a close");
    logger.remove_handler(&(handler.clone() as Arc<dyn Handler>));
    assert_eq!(
        String::from_utf8(handler.io().read_all_bytes().expect("the bytes")).expect("utf-8"),
        "held\nkept the token in hand\n",
        "the closing publish's own warning, published before the handle closed"
    );
}
