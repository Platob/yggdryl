# Logging

`yggdryl::logging` is Python's `logging` owned by the core: named loggers in a dotted tree, Python's numeric levels, handlers and `%`-style formatters, behind the `log` facade the crate already logs through - and in Python it is `logging` itself.

## Contract

| Key | Value |
| --- | --- |
| Owner | `rust/src/logging/`, one tree per process. Python's `logging` is the tree's host and `yggdryl.logging` adds `FileHandler`, the terminal handler and the deduplication doors; JavaScript reaches the tree as the frozen `logging` namespace; the `yggdryl` command attaches a handler to its root ([Warnings](#warnings)) |
| Tree | One logger per dotted name, made the first time it is asked for; `""` and `"root"` are the root, named `root`; a logger hangs from its nearest existing ancestor and is re-hung when a nearer one is made ([Loggers and the tree](#loggers-and-the-tree)) |
| Levels | Python's numbers - `NOTSET` 0, `TRACE` 5, `DEBUG` 10, `INFO` 20, `WARNING` 30, `ERROR` 40, `CRITICAL` 50 - and any number `0..=255` between them ([Levels](#levels)) |
| Dispatch | A record at or above its logger's effective level passes the logger's filters, then reaches every handler up the propagating chain whose level it meets; a record no handler takes goes to the last resort, standard error at `WARNING` |
| Terminal | What a handler with no formatter, `basic_config` with none and the last resort write: the timestamp, the level's glyph and name, `[thread]`, the logger, the call site and the message, coloured only where `is_color_enabled` says ([Terminal](#terminal)) |
| Cached | Each logger's threshold and its deduplication, dropped by any level change, `disable`, `disabled`, a deduplication statement, a host change and a handler attached or detached - as Python's own cache is. A disabled record costs a few atomic loads, takes no lock and allocates nothing |
| Lazy | A record borrows its message and only a handler that writes it renders it, a deduplicating logger also rendering it into a hash and never into text; the facade refuses a record nothing handles before its message is built; a `FileHandler` opens its handle on its first publish |
| Deduplicated | Off unless a logger, or its nearest ancestor, states it: a record is counted by its hash where it is logged, said the first time and again at its 10th, 100th, 1000th occurrence with the count, and dropped otherwise, before any handler or host ([Deduplication](#deduplication)) |
| Parsed once | A format and a date format when the `Formatter` is built; a level name where it is given |
| Refused | An unknown key, a spec a key's kind cannot take, a width or a precision past 4096, a `%` naming no key, a format naming none, a date directive outside the list: `Error::Parse` at the byte. A level outside `0..=255` or an unknown name: `Error::Parse`. A `FileHandler` mode other than `append` and `overwrite`: `Error::InvalidRecord`. A `FileHandler`'s `flush` or `close` on the thread holding its handle: `Error::Io` of kind `Deadlock`, never a wait on itself. `install` beside another `log` backend: `Error::Conflict` |
| Never fails the caller | A handler that cannot emit, or a publish a store refuses, says so on standard error under `--- Logging error ---`; the code that logged never sees it, and a standard error that cannot take the report either - closed, a full disk, a reader gone - is left alone |
| Bindings | Python: `logging` itself - its loggers, levels, filters, formatters and handlers - with `yggdryl.logging.FileHandler`, `TerminalFormatter`, `TerminalHandler`, `deduplicate`, `is_deduplicating` and `Deduplicate`. JavaScript: loggers, levels, `Formatter` and `Formatter.terminal()`, `StreamHandler` and its `colored`, `FileHandler`, `NullHandler`, `basicConfig`, `disable`, `shutdown` and a logger's `deduplicating`; no filters and no handler of your own. Rust only: the `Handler` trait, `Record` and its `with_level_name`, `Host`, `Repeats`, `install`, `set_last_resort`, `set_foreign_level`, `set_thread_name`, `is_color_enabled` |

## Use

A logger, a file handler writing through a location, and the records it takes: each record is appended as it arrives.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::Holder;
    use yggdryl::local::LocalFolder;
    use yggdryl::logging::{self, FileHandler, Formatter, Handler, Level};

    let root = LocalFolder::temporary()?.path()?.join(format!("yggdryl-docs-logging-use-{}", std::process::id()));
    let path = root.join("logs").join("feed.log");

    let logger = logging::get_logger("docs.logging.use.feed");
    logger.set_level(Level::INFO);
    logger.set_propagating(false);

    // Any storage handle: a local file here, an object in a bucket or a buffer elsewhere.
    let file = Arc::new(FileHandler::new(Holder::local(&path)?));
    file.set_formatter(Formatter::from_str("%(levelname)s %(name)s %(message)s")?);
    let handler: Arc<dyn Handler> = file.clone();
    logger.add_handler(handler.clone());

    logger.info("opened 3 venues");
    logger.debug("below the logger's level");
    logger.warning("2 late fills");

    assert_eq!(
        std::fs::read_to_string(&path)?,
        "INFO docs.logging.use.feed opened 3 venues\nWARNING docs.logging.use.feed 2 late fills\n",
    );
    assert!(logger.remove_handler(&handler));
    file.close()?;
    std::fs::remove_dir_all(&root)?;
    ```

=== "Python"

    ```python
    import logging
    import pathlib
    import shutil
    import tempfile

    from yggdryl.logging import FileHandler

    folder = pathlib.Path(tempfile.mkdtemp())
    path = folder / "logs" / "feed.log"

    logger = logging.getLogger("docs.logging.use.feed")
    logger.setLevel(logging.INFO)
    logger.propagate = False

    # Any location: a path here, a `Url`, an `IOBase` or an object in a bucket elsewhere.
    handler = FileHandler(path)
    handler.setFormatter(logging.Formatter("%(levelname)s %(name)s %(message)s"))
    logger.addHandler(handler)

    logger.info("opened 3 venues")
    logger.debug("below the logger's level")
    logger.warning("2 late fills")

    assert path.read_text(encoding="utf-8") == (
        "INFO docs.logging.use.feed opened 3 venues\n"
        "WARNING docs.logging.use.feed 2 late fills\n"
    )
    logger.removeHandler(handler)
    handler.close()
    shutil.rmtree(folder)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { logging } = require('yggdryl')

    const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-logging-use-'))
    const location = path.join(folder, 'logs', 'feed.log')

    const logger = logging.getLogger('docs.logging.use.feed')
    logger.setLevel('INFO')
    logger.propagate = false

    // Any location: a path here, a `Url`, an `IOBase` or an object in a bucket elsewhere.
    const handler = new logging.FileHandler(location)
    handler.setFormatter(new logging.Formatter('%(levelname)s %(name)s %(message)s'))
    logger.addHandler(handler)

    logger.info('opened 3 venues')
    logger.debug("below the logger's level")
    logger.warning('2 late fills')

    assert.equal(
      fs.readFileSync(location, 'utf8'),
      'INFO docs.logging.use.feed opened 3 venues\nWARNING docs.logging.use.feed 2 late fills\n',
    )
    assert.equal(logger.removeHandler(handler), true)
    handler.close()
    fs.rmSync(folder, { recursive: true, force: true })
    ```

## Levels

A level is a number on Python's scale, and any number from `0` to `255` is one: a logger at `15` handles `INFO` and not `DEBUG`, and an unnamed level is spelled `Level 15`. `NOTSET` on a logger takes the nearest ancestor's level; on a handler it emits every record its logger passes.

| Level | Number | `log` facade |
| --- | --- | --- |
| `NOTSET` | 0 | - |
| `TRACE` | 5 | `log::Level::Trace` |
| `DEBUG` | 10 | `log::Level::Debug` |
| `INFO` | 20 | `log::Level::Info` |
| `WARNING` | 30 | `log::Level::Warn` |
| `ERROR` | 40 | `log::Level::Error` |
| `CRITICAL` | 50 | - |

A name reads in any case, with Python's `WARN` and `FATAL`, and so does its number as text. Python states the same numbers in `logging`, and names no `TRACE`: a Rust `trace` record reaches it as `Level 5`.

=== "Rust"

    ```rust
    use yggdryl::logging::Level;

    assert_eq!(Level::from_str("warn")?, Level::WARNING);
    assert_eq!(Level::from_str("FATAL")?, Level::CRITICAL);
    assert_eq!(Level::from_str("15")?, Level::new(15));
    assert_eq!(Level::new(15).to_string(), "Level 15");
    assert!(Level::DEBUG < Level::new(15) && Level::new(15) < Level::INFO);
    assert_eq!(Level::from(log::Level::Trace), Level::TRACE);

    let refused = Level::from_str("verbose").unwrap_err().to_string();
    assert!(refused.starts_with("invalid log level expression at byte 0:"), "{refused}");
    assert!(Level::from_str("256").is_err());
    ```

=== "Python"

    ```python
    import logging
    import pathlib
    import shutil
    import tempfile

    from yggdryl.logging import FileHandler

    # The tree's numbers are logging's own.
    assert (logging.DEBUG, logging.INFO, logging.WARNING, logging.ERROR) == (10, 20, 30, 40)
    assert logging.getLevelName(15) == "Level 15"

    # `flush_level` is read by the core: a name in any case, `WARN`, or a number.
    folder = pathlib.Path(tempfile.mkdtemp())
    handler = FileHandler(folder / "levels.log", capacity=4096, flush_level="warn")
    assert handler.flush_level == logging.WARNING
    try:
        FileHandler(folder / "never.log", flush_level="verbose")
    except ValueError as error:
        assert "invalid log level expression" in str(error)
    else:
        raise AssertionError("an unknown level is refused")
    handler.close()
    shutil.rmtree(folder)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { logging } = require('yggdryl')

    assert.deepEqual(
      [logging.NOTSET, logging.TRACE, logging.DEBUG, logging.INFO, logging.WARNING, logging.ERROR, logging.CRITICAL],
      [0, 5, 10, 20, 30, 40, 50],
    )
    const logger = logging.getLogger('docs.logging.levels')
    logger.setLevel('warn')
    assert.equal(logger.level, logging.WARNING)
    logger.setLevel(15)
    assert.equal(logger.isEnabledFor(logging.DEBUG), false)
    assert.equal(logger.isEnabledFor('info'), true)
    assert.throws(() => logger.setLevel('verbose'), /invalid log level expression at byte 0/)
    assert.throws(() => logger.setLevel(256), /level must be an unsigned 8-bit integer/)
    ```

## Loggers and the tree

A logger is named by dots, and its parent is its nearest ancestor that exists, whichever was asked for first; the root is `root`, at `WARNING` until set. A logger at `NOTSET` takes its nearest ancestor's level, a record reaches the handlers of its logger and of every ancestor until one does not propagate, a disabled logger drops what is logged on it - before its handlers and a [host](#python-hosted-by-logging) alike - and a logger's filters judge only the records logged on it.

Asking whether a level is enabled is the fast path: each logger caches its threshold, and every change that could move one - a level, `disable`, `disabled`, a deduplication statement, the host, a handler attached or detached - drops every cache at once, as Python's `_clear_cache` does. In Python the tree is `logging`'s own; JavaScript has no filters.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::logging::{self, FileHandler, Filter, Formatter, Handler, Level, Record};

    // Asked for after its child, the parent still becomes the child's parent.
    let leaf = logging::get_logger("docs.logging.tree.feed.orders");
    let feed = logging::get_logger("docs.logging.tree.feed");
    assert_eq!(leaf.parent(), Some(feed.clone()));
    assert_eq!(feed.child("orders"), leaf);
    assert_eq!(logging::get_logger("root"), logging::get_logger(""));
    assert_eq!(logging::get_logger("").name(), "root");

    feed.set_level(Level::DEBUG);
    feed.set_propagating(false);
    assert_eq!(leaf.level(), Level::NOTSET);
    assert_eq!(leaf.effective_level(), Level::DEBUG);
    assert!(leaf.is_enabled_for(Level::DEBUG));

    let file = Arc::new(FileHandler::new(Buffer::new()));
    file.set_formatter(Formatter::default());
    let handler: Arc<dyn Handler> = file.clone();
    feed.add_handler(handler.clone());
    let quiet: Filter = Arc::new(|record: &Record<'_>| record.message().to_string() != "heartbeat");
    leaf.add_filter(Arc::clone(&quiet));

    leaf.info("opened");
    leaf.info("heartbeat");
    leaf.set_disabled(true);
    assert!(!leaf.is_enabled_for(Level::CRITICAL));
    leaf.critical("dropped by a disabled logger");
    leaf.set_disabled(false);
    leaf.set_level(Level::ERROR);
    leaf.warning("under its own level");

    assert_eq!(file.io().read_all_bytes()?, b"opened\n");
    assert_eq!(leaf.to_string(), "docs.logging.tree.feed.orders (ERROR)");
    assert!(leaf.remove_filter(&quiet));
    assert!(feed.remove_handler(&handler));
    ```

=== "Python"

    ```python
    import logging
    import pathlib
    import shutil
    import tempfile

    from yggdryl.logging import FileHandler

    # Asked for after its child, the parent still becomes the child's parent.
    leaf = logging.getLogger("docs.logging.tree.feed.orders")
    feed = logging.getLogger("docs.logging.tree.feed")
    assert leaf.parent is feed
    assert feed.getChild("orders") is leaf
    assert logging.getLogger("root") is logging.getLogger("")
    assert logging.getLogger("").name == "root"

    feed.setLevel(logging.DEBUG)
    feed.propagate = False
    assert leaf.level == logging.NOTSET
    assert leaf.getEffectiveLevel() == logging.DEBUG
    assert leaf.isEnabledFor(logging.DEBUG)

    folder = pathlib.Path(tempfile.mkdtemp())
    path = folder / "tree.log"
    handler = FileHandler(path)
    handler.setFormatter(logging.Formatter())
    feed.addHandler(handler)
    quiet = lambda record: record.getMessage() != "heartbeat"
    leaf.addFilter(quiet)

    leaf.info("opened")
    leaf.info("heartbeat")
    leaf.disabled = True
    assert not leaf.isEnabledFor(logging.CRITICAL)
    leaf.critical("dropped by a disabled logger")
    leaf.disabled = False
    leaf.setLevel(logging.ERROR)
    leaf.warning("under its own level")

    assert path.read_text(encoding="utf-8") == "opened\n"
    leaf.removeFilter(quiet)
    feed.removeHandler(handler)
    handler.close()
    shutil.rmtree(folder)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { logging } = require('yggdryl')

    // Asked for after its child, the parent still becomes the child's parent.
    const leaf = logging.getLogger('docs.logging.tree.feed.orders')
    const feed = logging.getLogger('docs.logging.tree.feed')
    assert.ok(leaf.parent.equals(feed))
    assert.ok(feed.getChild('orders').equals(leaf))
    assert.ok(logging.getLogger('root').equals(logging.getLogger()))
    assert.equal(logging.getLogger().name, 'root')
    assert.equal(logging.getLogger().parent, null)

    feed.setLevel('DEBUG')
    feed.propagate = false
    assert.equal(leaf.level, logging.NOTSET)
    assert.equal(leaf.getEffectiveLevel(), logging.DEBUG)
    assert.equal(leaf.isEnabledFor('DEBUG'), true)

    const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-logging-tree-'))
    const location = path.join(folder, 'tree.log')
    const handler = new logging.FileHandler(location)
    handler.setFormatter(new logging.Formatter())
    feed.addHandler(handler)
    assert.equal(leaf.hasHandlers(), true)

    leaf.info('opened')
    leaf.disabled = true
    assert.equal(leaf.isEnabledFor('CRITICAL'), false)
    leaf.critical('dropped by a disabled logger')
    leaf.disabled = false
    leaf.setLevel('ERROR')
    leaf.warning('under its own level')

    assert.equal(fs.readFileSync(location, 'utf8'), 'opened\n')
    assert.equal(String(leaf), 'docs.logging.tree.feed.orders (ERROR)')
    assert.equal(feed.removeHandler(handler), true)
    handler.close()
    fs.rmSync(folder, { recursive: true, force: true })
    ```

## Handlers

A handler emits each record its logger hands it at or above the handler's own level, spelled by its formatter - the [terminal line](#terminal) until one is set - into a line buffer its thread reuses. In Python the handlers are `logging`'s own, and the core adds the file handler and the terminal handler. An example on this page that reads a handler's output back states its format, `%(message)s` where it asserts the bare message: `Formatter::default()`, Python's `logging.Formatter()`, `new logging.Formatter()`.

| Handler | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| A stream | `StreamHandler::stderr()`, `stdout()`, `new(writer)` | `logging.StreamHandler`; `yggdryl.logging.TerminalHandler` | `new logging.StreamHandler('stderr' \| 'stdout')` |
| Nothing at all | `NullHandler::new()` | `logging.NullHandler` | `new logging.NullHandler()` |
| A location | `FileHandler<H: IOBase>` | `yggdryl.logging.FileHandler` | `new logging.FileHandler(location, options)` |
| Your own | `impl Handler` | a `logging.Handler` subclass | not offered |

### Streams and the null handler

A stream handler writes one line per record and flushes after each; on standard error or standard output it is coloured when that stream is a colour terminal, a writer is plain ([Colour](#colour)). A writer that logs into its own handler while it writes has that line written after the one being written - one round, so what writing those lines logs waits for the next record - and a flush it asks for meanwhile does nothing; standard error and standard output lock re-entrantly anyway. A null handler takes every record and writes none, so a library's logger keeps the last resort from speaking for it.

=== "Rust"

    ```rust
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use yggdryl::logging::{self, Formatter, Handler, Level, NullHandler, StreamHandler};

    /// A writer whose bytes the example reads back.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let written = Shared::default();
    let stream = StreamHandler::new(written.clone());
    assert_eq!(stream.level(), Level::NOTSET);
    stream.set_level(Level::WARNING);
    stream.set_formatter(Formatter::from_str("%(levelname)s %(message)s")?);

    let logger = logging::get_logger("docs.logging.stream");
    logger.set_propagating(false);
    logger.set_level(Level::INFO);
    logger.add_handler(Arc::new(stream));
    logger.info("under the handler's level");
    logger.warning("late fill");
    assert_eq!(String::from_utf8(written.0.lock().unwrap().clone())?, "WARNING late fill\n");

    let library = logging::get_logger("docs.logging.null");
    library.add_handler(Arc::new(NullHandler::new()));
    assert!(library.has_handlers());
    ```

=== "Python"

    ```python
    import io
    import logging

    written = io.StringIO()
    stream = logging.StreamHandler(written)
    assert stream.level == logging.NOTSET
    stream.setLevel(logging.WARNING)
    stream.setFormatter(logging.Formatter("%(levelname)s %(message)s"))

    logger = logging.getLogger("docs.logging.stream")
    logger.propagate = False
    logger.setLevel(logging.INFO)
    logger.addHandler(stream)
    logger.info("under the handler's level")
    logger.warning("late fill")
    assert written.getvalue() == "WARNING late fill\n"

    library = logging.getLogger("docs.logging.null")
    library.addHandler(logging.NullHandler())
    assert library.hasHandlers()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { logging } = require('yggdryl')

    // Standard error or standard output: the stream is the process's own.
    const stream = new logging.StreamHandler('stdout')
    assert.equal(stream.level, logging.NOTSET)
    stream.setLevel('WARNING')
    stream.setFormatter(new logging.Formatter('%(levelname)s %(message)s'))
    assert.equal(stream.level, logging.WARNING)
    assert.equal(stream.formatter.format, '%(levelname)s %(message)s')
    assert.throws(() => new logging.StreamHandler('file'), /expected the stream stderr or stdout/)

    const logger = logging.getLogger('docs.logging.stream')
    logger.propagate = false
    logger.setLevel('INFO')
    logger.addHandler(stream)
    logger.info("under the handler's level")
    logger.warning('late fill') // `WARNING late fill` on standard output

    const library = logging.getLogger('docs.logging.null')
    library.addHandler(new logging.NullHandler())
    assert.equal(library.hasHandlers(), true)
    ```

### A handler of your own

Rust only: implement `state` and `emit`; the level, the formatter, the filters, `handle`, `flush`, `close` and `handle_error` are provided. A handler's filters judge every record handed to it, wherever it was logged. JavaScript offers no handler of its own: a record can be logged on any Rust thread, and a JavaScript function runs on its isolate's thread only.

```rust
use std::sync::{Arc, Mutex};

use yggdryl::logging::{self, Formatter, Handler, HandlerState, Level, Record};

#[derive(Default)]
struct Collect {
    state: HandlerState,
    lines: Mutex<Vec<String>>,
}

impl Handler for Collect {
    fn state(&self) -> &HandlerState {
        &self.state
    }

    fn emit(&self, _record: &Record<'_>, line: &str) -> yggdryl::Result<()> {
        self.lines.lock().unwrap().push(line.to_owned());
        Ok(())
    }
}

let collect = Arc::new(Collect::default());
collect.set_level(Level::WARNING);
collect.set_formatter(Formatter::from_str("%(levelname)s:%(name)s:%(message)s")?);
collect.add_filter(Arc::new(|record: &Record<'_>| record.name() != "docs.logging.custom.noisy"));

let logger = logging::get_logger("docs.logging.custom");
logger.set_propagating(false);
logger.set_level(Level::DEBUG);
logger.add_handler(collect.clone());

logger.info("under the handler's level");
logger.warning("late fill");
logger.child("noisy").error("refused by the handler's filter");
assert_eq!(*collect.lines.lock().unwrap(), ["WARNING:docs.logging.custom:late fill"]);
```

### FileHandler

A file handler writes each record as one line through any storage handle - a local file, an object in a bucket, a member of a [ZIP archive](holder/index.md#zip), a buffer - as Python's `FileHandler` does, with its `MemoryHandler`'s batching folded in. Every record is held in the handler's buffer and the buffer is published with one `append_bytes`: at once by default, or, under a capacity, once that many bytes are held or a record at the flush level arrives.

| Setting | Rust | Python | JavaScript | Default |
| --- | --- | --- | --- | --- |
| Location | any `H: IOBase` | `str`, `os.PathLike`, `Url`, `IOBase` | a path or URL text, `Url`, `Uri`, `Urn`, `Arn`, `IOBase` | - |
| Mode | `with_mode(IOMode)` | `mode=` | `{ mode }` | `append`; `overwrite` replaces the location with the first publish |
| Capacity | `with_capacity(bytes)` | `capacity=` | `{ capacity }` | `0`: each record published as it arrives |
| Flush level | `with_flush_level(level)` | `flush_level=` | `{ flushLevel }` | `ERROR` |
| Level | `set_level(level)` | `level=`, `setLevel` | `{ level }`, `setLevel` | `NOTSET` |

Each publish is a stated number of calls on the handle, pinned in `rust/tests/iobase_calls.rs`:

| Operation | Calls on the handle |
| --- | --- |
| construction, a record held back | none |
| the first publish | `open` + `append_bytes`; `open` + `write_all_bytes` under `overwrite`, once per handler |
| each later publish | `append_bytes` |
| `flush` holding nothing | none |
| `close` | the publish of what is held - and one more for what that publish raised - then `close` |

An append is a whole publish on every backend - one `PUT` of the object on an [object store](holder/index.md#object-stores), a rewrite of the member in an archive - so a handler over a remote store states a capacity: `with_capacity(1 << 20)` turns a `PUT` per record into one per mebibyte, and the flush level still publishes an `ERROR` at once. Records are held in memory until then, and a publish that fails loses the records it carried and says so on standard error.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::Buffer;
    use yggdryl::logging::{self, FileHandler, Formatter, Handler, Level};
    use yggdryl::{IOBase, IOMode};

    let file = Arc::new(
        FileHandler::new(Buffer::new())
            .with_capacity(64 * 1024)
            .with_flush_level(Level::ERROR),
    );
    file.set_formatter(Formatter::default());
    assert_eq!((file.mode(), file.capacity(), file.flush_level()), (IOMode::Append, 65_536, Level::ERROR));

    let logger = logging::get_logger("docs.logging.file");
    logger.set_propagating(false);
    logger.set_level(Level::INFO);
    logger.add_handler(file.clone());

    logger.info("opened");
    logger.info("filled 3");
    // Held under the capacity: no call has reached the buffer.
    assert_eq!(file.io().read_all_bytes()?, b"");
    // A record at the flush level publishes everything held, in one append.
    logger.error("rejected");
    assert_eq!(file.io().read_all_bytes()?, b"opened\nfilled 3\nrejected\n");
    logger.info("closed");
    file.flush()?;
    assert_eq!(file.io().read_all_bytes()?, b"opened\nfilled 3\nrejected\nclosed\n");
    ```

=== "Python"

    ```python
    import logging
    import pathlib
    import shutil
    import tempfile

    from yggdryl.logging import FileHandler

    folder = pathlib.Path(tempfile.mkdtemp())
    path = folder / "file.log"
    handler = FileHandler(path, capacity=64 * 1024, flush_level=logging.ERROR)
    handler.setFormatter(logging.Formatter())
    assert (handler.mode, handler.capacity, handler.flush_level) == ("append", 65_536, logging.ERROR)
    assert handler.url.startswith("file://")

    logger = logging.getLogger("docs.logging.file")
    logger.propagate = False
    logger.setLevel(logging.INFO)
    logger.addHandler(handler)

    logger.info("opened")
    logger.info("filled 3")
    # Held under the capacity: nothing has reached the location.
    assert not path.exists()
    # A record at the flush level publishes everything held, in one append.
    logger.error("rejected")
    assert path.read_text(encoding="utf-8") == "opened\nfilled 3\nrejected\n"
    logger.info("closed")
    handler.flush()
    assert path.read_text(encoding="utf-8") == "opened\nfilled 3\nrejected\nclosed\n"

    logger.removeHandler(handler)
    handler.close()
    shutil.rmtree(folder)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { logging } = require('yggdryl')

    const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-logging-file-'))
    const location = path.join(folder, 'file.log')
    const handler = new logging.FileHandler(location, { capacity: 64 * 1024, flushLevel: 'ERROR' })
    handler.setFormatter(new logging.Formatter())
    assert.deepEqual([handler.mode, handler.capacity, handler.flushLevel], ['append', 65536, logging.ERROR])
    assert.ok(handler.url.startsWith('file://'))

    const logger = logging.getLogger('docs.logging.file')
    logger.propagate = false
    logger.setLevel('INFO')
    logger.addHandler(handler)

    logger.info('opened')
    logger.info('filled 3')
    // Held under the capacity: nothing has reached the location.
    assert.equal(fs.existsSync(location), false)
    // A record at the flush level publishes everything held, in one append.
    logger.error('rejected')
    assert.equal(fs.readFileSync(location, 'utf8'), 'opened\nfilled 3\nrejected\n')
    logger.info('closed')
    handler.flush()
    assert.equal(fs.readFileSync(location, 'utf8'), 'opened\nfilled 3\nrejected\nclosed\n')

    logger.removeHandler(handler)
    handler.close()
    fs.rmSync(folder, { recursive: true, force: true })
    ```

`overwrite` replaces what the location holds with the handler's first publish, and only that one: a handler reopened after a close appends to what it wrote. Every other mode is refused, and the bindings take `append` and `overwrite` as written, never `a` or `w`.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::Buffer;
    use yggdryl::logging::{self, FileHandler, Formatter, Handler, Level};
    use yggdryl::{IOBase, IOMode};

    let mut yesterday = Buffer::new();
    yesterday.write_all_bytes(b"yesterday\n")?;
    let file = Arc::new(FileHandler::new(yesterday).with_mode(IOMode::Overwrite)?);
    file.set_formatter(Formatter::default());

    let logger = logging::get_logger("docs.logging.overwrite");
    logger.set_propagating(false);
    logger.set_level(Level::INFO);
    logger.add_handler(file.clone());
    // Nothing is replaced before a record is published.
    assert_eq!(file.io().read_all_bytes()?, b"yesterday\n");
    logger.info("today");
    logger.info("later");
    assert_eq!(file.io().read_all_bytes()?, b"today\nlater\n");

    let refused = FileHandler::new(Buffer::new()).with_mode(IOMode::Merge).unwrap_err();
    assert!(refused.to_string().contains("expected append or overwrite"), "{refused}");
    ```

=== "Python"

    ```python
    import logging
    import pathlib
    import shutil
    import tempfile

    from yggdryl import IOBase
    from yggdryl.logging import FileHandler

    folder = pathlib.Path(tempfile.mkdtemp())
    path = folder / "replaced.log"
    path.write_text("yesterday\n", encoding="utf-8")
    handler = FileHandler(IOBase(path), mode="overwrite")
    handler.setFormatter(logging.Formatter())

    logger = logging.getLogger("docs.logging.overwrite")
    logger.propagate = False
    logger.setLevel(logging.INFO)
    logger.addHandler(handler)
    # Nothing is replaced before a record is published.
    assert path.read_text(encoding="utf-8") == "yesterday\n"
    logger.info("today")
    logger.info("later")
    assert path.read_text(encoding="utf-8") == "today\nlater\n"

    for mode, reason in (("merge", "expected append or overwrite"), ("a", "invalid mode expression")):
        try:
            FileHandler(path, mode=mode)
        except ValueError as error:
            assert reason in str(error), error
        else:
            raise AssertionError(f"mode {mode!r} is refused")
    logger.removeHandler(handler)
    handler.close()
    shutil.rmtree(folder)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { logging, IOBase } = require('yggdryl')

    const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-logging-overwrite-'))
    const location = path.join(folder, 'replaced.log')
    fs.writeFileSync(location, 'yesterday\n')
    const handler = new logging.FileHandler(new IOBase(location), { mode: 'overwrite' })
    handler.setFormatter(new logging.Formatter())

    const logger = logging.getLogger('docs.logging.overwrite')
    logger.propagate = false
    logger.setLevel('INFO')
    logger.addHandler(handler)
    // Nothing is replaced before a record is published.
    assert.equal(fs.readFileSync(location, 'utf8'), 'yesterday\n')
    logger.info('today')
    logger.info('later')
    assert.equal(fs.readFileSync(location, 'utf8'), 'today\nlater\n')

    assert.throws(() => new logging.FileHandler(location, { mode: 'merge' }), /expected append or overwrite/)
    assert.throws(() => new logging.FileHandler(location, { mode: 'a' }), /invalid mode expression/)
    logger.removeHandler(handler)
    handler.close()
    fs.rmSync(folder, { recursive: true, force: true })
    ```

A record never waits on a publish in flight: the held lines sit under a short lock never held across a call on the handle, and the handle has a lock of its own.

| While the handle is held, by a publish or by `io()`'s guard, a record logged | Goes |
| --- | --- |
| on another thread - a store's own runtime driving the request, another handler's publish | held; the publishing thread takes it in and publishes it before it lets go of the handle, at most eight publishes for one call, and what remains goes with the next record, `flush` or `close` |
| on the thread holding the handle - a credential refresh or a retry its publish raises, or any record logged under `io()`'s guard | set aside and carried by the next publish, which it never starts on its own |

`flush` publishes everything held, waiting on a publish in flight, then what came due meanwhile. `close` publishes everything held and once more for the lines that publish raised about itself, then closes the handle; what the second publish or the close itself raises waits for the next publish, which reopens the handle. A `flush` or `close` on the thread already holding the handle - under its own `io()` guard, or from inside its own publish - is refused with `Error::Io` of kind `std::io::ErrorKind::Deadlock` rather than waiting on itself, and `shutdown` says such a refusal on standard error and closes the others. `io()` is the handle, locked: while its guard lives other threads' records are held, and what came due is published when it drops, the guard thread's own records waiting for the next publish; taking it again on the same thread waits on itself, as any lock does.

Rust only: no binding offers `io()`.

```rust
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::logging::{FileHandler, Formatter, Handler, Level, Record};

let handler = FileHandler::new(Buffer::new()).with_capacity(1024);
handler.set_formatter(Formatter::default());
handler.handle(&Record::new("docs.logging.guard", Level::INFO, &"held"));
{
    // This thread holds the handle: a flush would wait on itself, so it is refused.
    let held = handler.io();
    assert_eq!(held.read_all_bytes()?, b"");
    let refused = handler.flush().unwrap_err();
    assert!(
        matches!(&refused, yggdryl::Error::Io(io) if io.kind() == std::io::ErrorKind::Deadlock),
        "{refused}"
    );
}
handler.flush()?;
assert_eq!(handler.io().read_all_bytes()?, b"held\n");
```

## Formatter

A formatter is Python's `%`-style format over a record's attributes, parsed once when it is built, so a format that could not spell a record is refused there, at the byte, rather than on every record. In Python the formatter is `logging.Formatter` itself: the same language, Python's own implementation and checks.

| Key | Is |
| --- | --- |
| `name` | the logger's name |
| `levelno`, `levelname` | the level's number and name: the name the record states (`Record::with_level_name`), else the level's own, else `Level 15` for an unnamed one |
| `message` | the message |
| `asctime` | the creation instant as the date format spells it, `2026-10-03 14:05:09,123` when none is given |
| `created`, `msecs`, `relativeCreated` | seconds since the Unix epoch, their milliseconds, and milliseconds since the tree first answered |
| `pathname`, `filename`, `module`, `lineno` | where the record was raised; `(unknown file)` and `0` for a record that says nothing |
| `funcName` | the function a record states - a binding's call site, a JavaScript caller's function - else `(unknown function)`: Rust names no function |
| `thread`, `threadName`, `process`, `processName` | the emitting thread's number, from `1` per process; the thread a record states, else the name `set_thread_name` gave the emitting thread, else the one it was spawned with, else `Thread-N` ([Threads](#threads)); the process id; and `MainProcess` |
| `caller` | the call site as one token: `function:line` when the record states its function, else the Rust module path's last segment and the line (`table:227`), else the file's stem and the line; the place alone without a line, `-` when nothing is known |
| `levelglyph` | the level's glyph, `Level::glyph()`: `·` `•` `!` `✗` `‼` ([Terminal](#terminal)) |
| `levelcolor`, `dim`, `bold`, `reset` | the level's colour, faint, bold, and every style off: escape sequences `format_colored_into` spells and `format` and `format_into` spell as nothing ([Colour](#colour)) |

`caller`, `levelglyph` and the four styles are the core's own keys: Python's `logging.Formatter` names none of them, and `yggdryl.logging.TerminalFormatter` spells the line they build ([Terminal](#terminal)).

A key takes Python's conversion spec after it: flags `-` (left-align), `0` (zero-pad a number), `+` and space (sign a number) and `#` (`0x`, `0X` or `0o` before a hexadecimal or octal number, zero padding going after it, and `%g`'s trailing zeros kept), a width and a `.precision` of at most 4096 each, Python's length modifiers `h`, `l` and `L` read and ignored, and a conversion. The whole numbers `levelno`, `lineno`, `thread` and `process` take every conversion; the fractions `created`, `msecs` and `relativeCreated` every one but `x`, `X`, `o` and `c`; every other key is text and takes `s`, `r` and `a` alone. `%%` is a literal `%`.

| Conversion | Spells |
| --- | --- |
| `s` | the value as text; a float as Python does, `123.0` |
| `r` | Python's `repr`: quoted, with a backslash, the quote and every character Python does not print escaped |
| `a` | Python's `ascii`: `repr` with every character past ASCII escaped too |
| `d`, `i`, `u` | an integer; a float truncated toward zero, so a fraction between `-1` and `0` writes `0` |
| `f`, `F` | fixed point, six places unless the precision says |
| `e`, `E` | exponent notation, six places unless the precision says, the exponent signed and two digits at least: `%(created)e` is `1.700000e+09` |
| `g`, `G` | fixed point or exponent notation, as Python's rule picks for the precision, trailing zeros dropped: `%(created)g` is `1.7e+09`, `%(created).12g` is `1700000000.12` |
| `x`, `X`, `o` | a whole number in hexadecimal, its digits upper case under `X`, or in octal: `%(levelno)#06X` is `0X001E` |
| `c` | the character a whole number is the code point of, `U+FFFD` for none |

The date format takes `strftime` directives and renders the instant in UTC unless the formatter names a [zone](types/temporal/timezone.md) - `with_timezone` in Rust, `{ timezone }` in JavaScript - so a log read on another machine names the same instant. Python's formatter renders local time unless its `converter` is `time.gmtime`.

| Directives | Spell |
| --- | --- |
| `%Y`, `%y`, `%C` | the year, the year of the century, the century |
| `%G`, `%g` | the year the ISO 8601 week belongs to, and its last two digits |
| `%m`, `%d`, `%e`, `%j` | the month, the day, the day padded with a space, the day of the year |
| `%U`, `%W`, `%V` | the week of the year, its first Sunday or its first Monday starting week `01` and the days before it week `00`, and the ISO 8601 week |
| `%H`, `%k`, `%I`, `%l` | the hour, padded with a zero or a space; the 12-hour hour, padded with a zero or a space |
| `%p`, `%P` | `AM` or `PM`, `am` or `pm` |
| `%M`, `%S`, `%f` | the minute, the second, the microseconds |
| `%a`, `%A`, `%u`, `%w` | the weekday short and long, numbered from Monday `1` and from Sunday `0` |
| `%b`, `%h`, `%B` | the month short and long |
| `%z`, `%Z`, `%s` | the offset `+0100`, the zone's abbreviation `CET` (`UTC`), Unix seconds |
| `%F`, `%T`, `%D`, `%R` | `%Y-%m-%d`, `%H:%M:%S`, `%m/%d/%y`, `%H:%M` |
| `%c`, `%x`, `%X`, `%r` | `%a %b %e %H:%M:%S %Y`, `%m/%d/%y`, `%H:%M:%S`, `%I:%M:%S %p`: the C locale's, as Python's `time.strftime` writes them |
| `%n`, `%t`, `%%` | a newline, a tab, `%` |

=== "Rust"

    ```rust
    use yggdryl::Timezone;
    use yggdryl::logging::{Formatter, Level, Record};

    // 2023-11-14 22:13:20.123 UTC, raised at line 42.
    let record = Record::new("trades.feed", Level::WARNING, &"late fill")
        .with_location("src/feed.rs", 42)
        .with_created(1_700_000_000_123_000_000);

    let formatter = Formatter::from_str("%(asctime)s %(levelname)-8s %(name)s:%(lineno)d %(message)s")?;
    assert_eq!(formatter.format(&record), "2023-11-14 22:13:20,123 WARNING  trades.feed:42 late fill");

    let dated = Formatter::from_str("%(asctime)s %(msecs)03d %(message)r")?.with_datefmt("%Y-%m-%dT%H:%M:%SZ")?;
    assert_eq!(dated.format(&record), "2023-11-14T22:13:20Z 123 'late fill'");

    let paris = Formatter::from_str("%(asctime)s")?
        .with_datefmt("%H:%M %z %Z")?
        .with_timezone(Timezone::from_str("Europe/Paris")?);
    assert_eq!(paris.format(&record), "23:13 +0100 CET");

    let numbers = Formatter::from_str("%(levelno)#06X %(created)g")?;
    assert_eq!(numbers.format(&record), "0X001E 1.7e+09");
    let weeks = Formatter::from_str("%(asctime)s")?.with_datefmt("%G-W%V %a")?;
    assert_eq!(weeks.format(&record), "2023-W46 Tue");

    let refused = Formatter::from_str("%(name)d").unwrap_err().to_string();
    assert_eq!(refused, "invalid log format expression at byte 7: name is text and takes the s, r or a conversion");
    let refused = Formatter::from_str("%(message)99999s").unwrap_err().to_string();
    assert_eq!(refused, "invalid log format expression at byte 10: expected a width of at most 4096, got 99999");
    assert!(Formatter::from_str("plain text").is_err());
    let refused = Formatter::from_str("%(asctime)s")?.with_datefmt("%Y week %Q").unwrap_err().to_string();
    assert!(refused.starts_with("invalid log date format expression at byte 8: expected one of the directives"), "{refused}");
    ```

=== "Python"

    ```python
    import logging
    import time

    # 2023-11-14 22:13:20.123 UTC, raised at line 42.
    record = logging.LogRecord("trades.feed", logging.WARNING, "src/feed.py", 42, "late fill", None, None)
    record.created, record.msecs = 1_700_000_000.123, 123.0

    formatter = logging.Formatter("%(asctime)s %(levelname)-8s %(name)s:%(lineno)d %(message)s")
    formatter.converter = time.gmtime
    assert formatter.format(record) == "2023-11-14 22:13:20,123 WARNING  trades.feed:42 late fill"

    dated = logging.Formatter("%(asctime)s %(msecs)03d %(message)r", datefmt="%Y-%m-%dT%H:%M:%SZ")
    dated.converter = time.gmtime
    assert dated.format(record) == "2023-11-14T22:13:20Z 123 'late fill'"

    # Python checks a format names a key; a conversion its value cannot take fails on a record.
    try:
        logging.Formatter("plain text")
    except ValueError as error:
        assert "Invalid format" in str(error)
    else:
        raise AssertionError("a format naming no key is refused")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { logging } = require('yggdryl')

    const formatter = new logging.Formatter('%(asctime)s %(levelname)-8s %(name)s:%(lineno)d %(message)s', {
      datefmt: '%Y-%m-%dT%H:%M:%SZ',
    })
    assert.deepEqual([formatter.datefmt, formatter.timezone], ['%Y-%m-%dT%H:%M:%SZ', 'UTC'])
    assert.equal(new logging.Formatter().format, '%(message)s')
    assert.equal(new logging.Formatter('%(asctime)s', { timezone: 'Europe/Paris' }).timezone, 'Europe/Paris')

    const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-logging-formatter-'))
    const location = path.join(folder, 'formatted.log')
    const handler = new logging.FileHandler(location)
    handler.setFormatter(formatter)
    const logger = logging.getLogger('docs.logging.formatter')
    logger.propagate = false
    logger.addHandler(handler)
    // Dated now; `lineno` is the JavaScript line that logged it.
    logger.warning('late fill')
    assert.match(
      fs.readFileSync(location, 'utf8'),
      /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z WARNING {2}docs\.logging\.formatter:[1-9]\d* late fill\n$/,
    )

    assert.throws(
      () => new logging.Formatter('%(name)d'),
      /invalid log format expression at byte 7: name is text and takes the s, r or a conversion/,
    )
    assert.throws(
      () => new logging.Formatter('%(message)99999s'),
      /invalid log format expression at byte 10: expected a width of at most 4096, got 99999/,
    )
    assert.throws(() => new logging.Formatter('plain text'), /naming at least one %\(key\)/)
    assert.throws(
      () => new logging.Formatter('%(message)s', { datefmt: '%Y week %Q' }),
      /invalid log date format expression at byte 8: expected one of the directives/,
    )
    logger.removeHandler(handler)
    handler.close()
    fs.rmSync(folder, { recursive: true, force: true })
    ```

## Terminal

The terminal line is `Formatter::terminal()`, the prebuilt formatter over `Formatter::TERMINAL_FORMAT`: what the core writes where no format is stated - a handler with no formatter, `basic_config` given none, the last resort - and, in Python, whose handlers and `basicConfig` are `logging`'s own, what `TerminalHandler` and a `FileHandler` given no formatter write.

```text
2026-10-03 14:05:09,123 • INFO     [main] trades.feed open:42 › opened 3 venues
```

| Part | Key | On a colour terminal |
| --- | --- | --- |
| the timestamp, Python's `asctime`, in UTC unless the formatter names a zone | `asctime` | faint |
| the level's glyph, and its name padded to eight | `levelglyph`, `levelname` | the level's colour |
| the thread, in brackets ([Threads](#threads)) | `threadName` | faint |
| the logger | `name` | bold |
| the call site, then `›` | `caller` | faint |
| the message | `message` | plain |

| Level | Glyph | Colour |
| --- | --- | --- |
| below `DEBUG` | `·` | faint |
| `DEBUG` | `·` | cyan |
| `INFO` | `•` | green |
| `WARNING` | `!` | yellow |
| `ERROR` | `✗` | red |
| `CRITICAL` and above | `‼` | bold red |

A number between two named levels takes the lower one's glyph and colour: `Level 25` is a green `•`. `Level::glyph()` answers the glyph, and the `yggdryl` command marks its notes, warnings and failures with the same `·`, `!` and `✗`.

`Formatter::TERMINAL_FORMAT` is the line as a `%`-style format, the start of a format of one's own:

```text
%(dim)s%(asctime)s%(reset)s %(levelcolor)s%(levelglyph)s %(levelname)-8s%(reset)s %(dim)s[%(threadName)s]%(reset)s %(bold)s%(name)s%(reset)s %(dim)s%(caller)s ›%(reset)s %(message)s
```

A message's later lines start under its first, so a multi-line message or a traceback stays one block. The indent counts terminal columns: an East Asian wide or fullwidth character or an emoji takes two, a combining mark, a zero-width character or an escape sequence none. A line of the message is indented only once it has text, so a message ending in a newline leaves no trailing spaces. Only the prebuilt formatter hangs, and it keeps hanging under `with_datefmt` and `with_timezone`; `Formatter::from_str(Formatter::TERMINAL_FORMAT)` spells the same first line and does not.

| | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| The formatter | `Formatter::terminal()` | `yggdryl.logging.TerminalFormatter(color=False)`, a `logging.Formatter` rendering natively, one call per record | `logging.Formatter.terminal()` |
| A handler writing it | any handler with no formatter set | `TerminalHandler(stream=None, color=None)`, a `logging.StreamHandler` on standard error unless given; a `FileHandler` with no formatter set, plain | any handler with no formatter set |
| Colour | `StreamHandler::set_colored`, `Handler::is_colored` | `color=` | a `StreamHandler`'s `colored` |
| The call site | `Record::with_function`, `with_location` | `funcName:lineno` | the calling function and line, read only once the level is enabled |
| The thread | `set_thread_name`, `Record::with_thread` | Python's `threadName` | `main`, `worker-N` |

Python's `TerminalFormatter` reads the timestamp from `record.created` and its milliseconds from `record.msecs`, as Python's own `Formatter` does, so a record built by hand sets both; it names the level as `record.levelname` does, so a level `logging.addLevelName(25, "NOTICE")` registered prints `NOTICE` and a Rust `trace` record `Level 5`; and an exception or a stack the record carries is part of its message, hanging under it. `logging.basicConfig(level=logging.INFO, handlers=[TerminalHandler()])` is the whole setup of a Python application.

=== "Rust"

    ```rust
    use std::io::IsTerminal;

    use yggdryl::logging::{self, Formatter, Handler, Level, Record, StreamHandler};

    // 2023-11-14 22:13:20.5 UTC, raised in `open` at line 42 on the thread `main`.
    let record = Record::new("trades.feed", Level::INFO, &"opened 3 venues")
        .with_location("src/feed.rs", 42)
        .with_function("open")
        .with_thread("main")
        .with_created(1_700_000_000_500_000_000);
    let terminal = Formatter::terminal();
    assert_eq!(terminal.as_str(), Formatter::TERMINAL_FORMAT);
    assert_eq!(
        terminal.format(&record),
        "2023-11-14 22:13:20,500 • INFO     [main] trades.feed open:42 › opened 3 venues",
    );

    // What a colour terminal is sent: the same line, styled.
    let mut colored = String::new();
    terminal.format_colored_into(&record, &mut colored);
    assert_eq!(
        colored,
        "\x1b[2m2023-11-14 22:13:20,500\x1b[0m \x1b[32m• INFO    \x1b[0m \x1b[2m[main]\x1b[0m \x1b[1mtrades.feed\x1b[0m \x1b[2mopen:42 ›\x1b[0m opened 3 venues",
    );

    // No function stated: the file's stem; a later line starts under the first.
    let crossed = Record::new("trades.feed", Level::WARNING, &"crossed\nat 101.5")
        .with_location("src/feed.rs", 43)
        .with_thread("main")
        .with_created(1_700_000_000_500_000_000);
    let line = terminal.format(&crossed);
    let (first, second) = line.split_once('\n').expect("two lines");
    assert_eq!(first, "2023-11-14 22:13:20,500 ! WARNING  [main] trades.feed feed:43 › crossed");
    assert_eq!(second, format!("{}at 101.5", " ".repeat(first.chars().count() - "crossed".len())));

    // Standard error asks the colour rule once; a writer is plain until set.
    let stderr = StreamHandler::stderr();
    assert_eq!(stderr.is_colored(), logging::is_color_enabled(std::io::stderr().is_terminal()));
    let written = StreamHandler::new(Vec::new());
    assert!(!written.is_colored());
    written.set_colored(true);
    assert!(written.is_colored());

    // A thread names itself in every record it logs that names none.
    let threaded = Formatter::from_str("%(threadName)s")?;
    let named = std::thread::spawn(move || {
        logging::set_thread_name("ingest");
        threaded.format(&Record::new("trades.feed", Level::INFO, &"opened"))
    })
    .join()
    .expect("the thread ran");
    assert_eq!(named, "ingest");
    ```

=== "Python"

    ```python
    import io
    import logging
    import re

    from yggdryl.logging import TerminalFormatter, TerminalHandler

    # 2023-11-14 22:13:20.5 UTC, raised in `open` at line 42, on the thread Python names.
    record = logging.LogRecord(
        "trades.feed", logging.INFO, "src/feed.py", 42, "opened %d venues", (3,), None, func="open"
    )
    # The milliseconds are the record's own `msecs`, as Python's `Formatter` reads them.
    record.created, record.msecs = 1_700_000_000.5, 500.0
    assert record.threadName == "MainThread"
    assert TerminalFormatter().format(record) == (
        "2023-11-14 22:13:20,500 • INFO     [MainThread] trades.feed open:42 › opened 3 venues"
    )
    # What a colour terminal is sent: the same line, styled.
    assert TerminalFormatter(color=True).format(record).startswith(
        "\x1b[2m2023-11-14 22:13:20,500\x1b[0m \x1b[32m• INFO    \x1b[0m \x1b[2m[MainThread]\x1b[0m"
    )

    # A handler on any stream, coloured as the stream decides unless `color` says.
    written = io.StringIO()
    handler = TerminalHandler(written, color=False)
    logger = logging.getLogger("docs.logging.terminal")
    logger.propagate = False
    logger.setLevel(logging.INFO)
    logger.addHandler(handler)


    def open_feed() -> None:
        logger.info("opened %d venues", 3)


    open_feed()
    pattern = r"\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3} • INFO     \[MainThread\] docs\.logging\.terminal open_feed:\d+ › opened 3 venues\n"
    assert re.fullmatch(pattern, written.getvalue()), written.getvalue()
    logger.removeHandler(handler)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { logging } = require('yggdryl')

    const terminal = logging.Formatter.terminal()
    assert.equal(
      terminal.format,
      '%(dim)s%(asctime)s%(reset)s %(levelcolor)s%(levelglyph)s %(levelname)-8s%(reset)s %(dim)s[%(threadName)s]%(reset)s %(bold)s%(name)s%(reset)s %(dim)s%(caller)s ›%(reset)s %(message)s',
    )
    // Standard error asks the colour rule when the handler is built; a handler stating no formatter spells the terminal line.
    const stream = new logging.StreamHandler('stderr')
    assert.equal(typeof stream.colored, 'boolean')
    stream.colored = false
    assert.equal(stream.colored, false)
    assert.equal(stream.formatter.format, terminal.format)

    const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-logging-terminal-'))
    const location = path.join(folder, 'terminal.log')
    // No formatter set: the terminal line, plain - a file is no colour terminal.
    const handler = new logging.FileHandler(location)
    const logger = logging.getLogger('docs.logging.terminal')
    logger.propagate = false
    logger.setLevel('INFO')
    logger.addHandler(handler)

    function openFeed() {
      logger.info('opened 3 venues')
    }
    openFeed()
    assert.match(
      fs.readFileSync(location, 'utf8'),
      /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3} • INFO {5}\[main\] docs\.logging\.terminal openFeed:\d+ › opened 3 venues\n$/,
    )
    logger.removeHandler(handler)
    handler.close()
    fs.rmSync(folder, { recursive: true, force: true })
    ```

### Colour

`is_color_enabled(is_terminal)` is the one colour rule, the conventions every tool in a shell honours, read in this order:

| Environment | Colour |
| --- | --- |
| `NO_COLOR` set to anything but the empty text | off |
| `FORCE_COLOR` or `CLICOLOR_FORCE` set to anything but the empty text, `0` or `false` | on |
| `TERM=dumb` | off |
| otherwise | on for a terminal, off for a file or a pipe |

A handler is coloured when `Handler::is_colored` answers `true`, which no handler does unless it says so: `StreamHandler::stderr()` and `stdout()` ask the rule of their stream once, when built; `StreamHandler::new(writer)` and a `FileHandler` are plain; `StreamHandler::set_colored` overrides. A coloured handler spells its formatter through `format_colored_into`, so the style keys of any format - the terminal's or one's own - spell their escape sequences there and nothing anywhere else. Python's `TerminalHandler` asks the same rule of its stream's `isatty()`, the JavaScript `StreamHandler` of its stream, and the `yggdryl` command colours its standard output by it - a forced colour reaching a pipe too - while it draws boxes and its spinner only on a terminal that is also coloured, so a forced colour never sends spinner frames into a pipe.

### Threads

`threadName` is the thread a record states (`Record::with_thread`), else the name `set_thread_name` gave the emitting thread, else the name it was spawned with, else `Thread-N` as Python spells it, `N` being its `%(thread)d`; `set_thread_name("")` forgets the name. The Node addon names each isolate's thread once - `main`, and `worker-N` by its `threadId` in a worker - so every record logged there, the core's own included, names it. A record Python's `logging` makes carries Python's own `threadName`, `MainThread` on the main thread.

## Deduplication

A logger can say a record once and count the rest. A deduplicating logger counts each record where it is logged, before any handler or host, by `Record::stable_hash`: XXH3-64 of the logger's name, the level and the message, the message rendered into the hash and never into text, so a key allocates nothing and two records built differently but reading the same are one key. The instant and the location are no part of it.

| Occurrence of a key | `Repeat` | Said |
| --- | --- | --- |
| the first | `First` | as logged |
| the 10th, 100th, 1000th... | `Tenfold(n)` | again, as `message (seen N times)` |
| any other | `Repeated` | not at all: no handler, no host and no last resort sees it |
| a new key the full table cannot count | `Untracked` | on a deduplicating logger, as logged, uncounted: never lost; the [data warnings](#warnings) count such keys together instead |

A logger deduplicates when it or its nearest ancestor states so: `Some(true)` drops repeats on it and below, `Some(false)` passes every record on it and below, and `None` takes the ancestors' again; it is off at the root unless set. The loggers of the tree count in one process-wide, lock-free table of at most 4,096 keys (`Repeats::CAPACITY`), a bound that holds exactly however many threads race for its last room; a count is two atomic operations per record, no lock, and one allocation for the slots on the first count. Two threads counting one key see two counts, so exactly one of them sees the first. A repeat reaches neither the tree's handlers nor the host, so under Python it never takes the interpreter.

| Rust | Python | JavaScript |
| --- | --- | --- |
| `Logger::set_deduplicating(Some(true))`, `deduplicating()`, `is_deduplicating()` | `deduplicate(logger, enabled=True)` and `is_deduplicating(logger)` for the records the core logs; `Deduplicate(name="")`, a `logging.Filter`, for any record | `logger.deduplicating` (`true`, `false` or `null`), `logger.isDeduplicating()` |

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::logging::{self, FileHandler, Formatter, Handler, Level};

    let feed = logging::get_logger("docs.logging.dedup.feed");
    let orders = feed.child("orders");
    feed.set_level(Level::DEBUG);
    feed.set_propagating(false);
    let file = Arc::new(FileHandler::new(Buffer::new()));
    file.set_formatter(Formatter::default());
    feed.add_handler(file.clone());

    // Off at the root unless set; a logger takes its nearest ancestor's statement.
    assert_eq!((feed.deduplicating(), orders.is_deduplicating()), (None, false));
    feed.set_deduplicating(Some(true));
    assert!(orders.is_deduplicating());

    for _ in 0..100 {
        orders.warning("late fill");
    }
    // Another message, or the same one at another level, is another key.
    orders.warning("late fills");
    orders.error("late fill");
    assert_eq!(
        file.io().read_all_bytes()?,
        b"late fill\nlate fill (seen 10 times)\nlate fill (seen 100 times)\nlate fills\nlate fill\n",
    );

    // A nearer statement wins: this logger passes every record.
    orders.set_deduplicating(Some(false));
    orders.warning("late fills");
    orders.warning("late fills");
    assert!(!orders.is_deduplicating());
    assert!(file.io().read_all_bytes()?.ends_with(b"late fills\nlate fills\n"));
    orders.set_deduplicating(None);
    feed.set_deduplicating(None);
    ```

=== "Python"

    ```python
    import logging
    import pathlib
    import shutil
    import tempfile

    from yggdryl.logging import Deduplicate, FileHandler

    folder = pathlib.Path(tempfile.mkdtemp())
    path = folder / "dedup.log"

    feed = logging.getLogger("docs.logging.dedup.feed")
    orders = feed.getChild("orders")
    feed.setLevel(logging.DEBUG)
    feed.propagate = False

    # The filter counts the records of any logger, by the core's hash of the name, the level and the message.
    handler = FileHandler(path)
    handler.setFormatter(logging.Formatter())
    handler.addFilter(Deduplicate())
    feed.addHandler(handler)

    for _ in range(100):
        orders.warning("late %s", "fill")
    # Another message, or the same one at another level, is another key.
    orders.warning("late fills")
    orders.error("late fill")

    assert path.read_text(encoding="utf-8") == (
        "late fill\n"
        "late fill (seen 10 times)\n"
        "late fill (seen 100 times)\n"
        "late fills\n"
        "late fill\n"
    )
    feed.removeHandler(handler)
    handler.close()
    shutil.rmtree(folder)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { logging } = require('yggdryl')

    const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-logging-dedup-'))
    const location = path.join(folder, 'dedup.log')

    const feed = logging.getLogger('docs.logging.dedup.feed')
    const orders = feed.getChild('orders')
    feed.setLevel('DEBUG')
    feed.propagate = false
    const handler = new logging.FileHandler(location)
    handler.setFormatter(new logging.Formatter())
    feed.addHandler(handler)

    // Off at the root unless set; a logger takes its nearest ancestor's statement.
    assert.equal(feed.deduplicating, null)
    assert.equal(orders.isDeduplicating(), false)
    feed.deduplicating = true
    assert.equal(orders.isDeduplicating(), true)

    for (let at = 0; at < 100; at += 1) orders.warning('late fill')
    // Another message, or the same one at another level, is another key.
    orders.warning('late fills')
    orders.error('late fill')
    assert.equal(
      fs.readFileSync(location, 'utf8'),
      'late fill\nlate fill (seen 10 times)\nlate fill (seen 100 times)\nlate fills\nlate fill\n',
    )

    // A nearer statement wins: this logger passes every record.
    orders.deduplicating = false
    orders.warning('late fills')
    orders.warning('late fills')
    assert.equal(orders.isDeduplicating(), false)
    assert.ok(fs.readFileSync(location, 'utf8').endsWith('late fills\nlate fills\n'))
    orders.deduplicating = null
    feed.deduplicating = null

    feed.removeHandler(handler)
    handler.close()
    fs.rmSync(folder, { recursive: true, force: true })
    ```

The Python tab counts with the filter because a record Python logs is Python's, never the core's: `Deduplicate` counts any record by the core's hash in a table of its own, passes the first, rewrites a tenfold occurrence's `msg` as `message (seen N times)` with its `args` cleared, and drops every other repeat; a record outside its `name` passes uncounted. A call whose message cannot be formatted - `logger.warning("%d items", "x")` - passes uncounted and never raises into the caller: its handler's `handleError` reports it. A record the filter re-spelled keeps the message it was logged with (`record._yggdryl_message`), so filters on several handlers count that message and each says the 10th, 100th, 1000th. `deduplicate` states the flag on the core's logger of that name, so it governs the records the core logs.

Python only: the core's own repeats, dropped where they are logged and never handed to `logging`.

```python
import logging
import pathlib
import shutil
import tempfile

import pyarrow as pa

from yggdryl import IOBase
from yggdryl.iceberg import IcebergTable, assign_field_ids
from yggdryl.logging import deduplicate, is_deduplicating

said: list[str] = []


class Collect(logging.Handler):
    def emit(self, record: logging.LogRecord) -> None:
        said.append(record.getMessage())


package = logging.getLogger("yggdryl")
package.addHandler(Collect())
package.setLevel(logging.INFO)

schema = assign_field_ids(pa.schema([pa.field("id", pa.int64(), nullable=False)]))
folder = pathlib.Path(tempfile.mkdtemp())
table = IcebergTable.create(IOBase(folder / "trades"), schema)

# The core's records are counted where it logs them, a logger below the named one included.
assert not is_deduplicating("yggdryl.iceberg.table")
deduplicate("yggdryl.iceberg")
assert is_deduplicating("yggdryl.iceberg.table")

said.clear()
for _ in range(10):
    assert sum(batch.num_rows for batch in table.scan()) == 0
scans = [message for message in said if message.startswith("planned iceberg scan")]
assert len(scans) == 2 and scans[1].endswith("(seen 10 times)"), scans

deduplicate("yggdryl.iceberg", None)
assert not is_deduplicating("yggdryl.iceberg.table")
shutil.rmtree(folder)
```

Rust only: `Repeats` is the table on its own, counting any key a caller hashes, and `Counted` is a message said again with its count.

```rust
use yggdryl::logging::{Counted, Level, Record, Repeat, Repeats};

let repeats = Repeats::new();
let record = Record::new("docs.logging.dedup.table", Level::WARNING, &"late fill");
assert_eq!(repeats.count_record(&record), Repeat::First);
for _ in 2..10 {
    assert_eq!(repeats.count_record(&record), Repeat::Repeated);
}
assert_eq!(repeats.count_record(&record), Repeat::Tenfold(10));
assert_eq!(repeats.seen(record.stable_hash()), 10);
assert_eq!(Counted::new(10, &"late fill").to_string(), "late fill (seen 10 times)");
```

## Configuring an application

`basic_config` is Python's `basicConfig`: it installs the tree as the [facade's backend](#the-log-facade) and, on a root with no handler, attaches the handlers it is given - a `StreamHandler::stderr()` when none is - gives each one without a formatter the configured format, the [terminal line](#terminal) when none is, and states the level. A root that already has handlers is left alone unless `force` closes and removes them first. `Formatter::BASIC_FORMAT`, `%(levelname)s:%(name)s:%(message)s`, stays for a log that should read as Python's `basicConfig` writes it. In Python, `basicConfig` is Python's own and `handlers=[TerminalHandler()]` gives it the terminal line; in JavaScript, `basicConfig()` gives standard error the terminal line, and `{ datefmt }` without a `format` keeps it under that date format.

=== "Rust"

    ```{ .rust .no_run }
    use std::sync::Arc;

    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::logging::{self, BasicConfig, FileHandler, Formatter, Level};

    let file = Arc::new(FileHandler::new(Buffer::new()));
    logging::basic_config(
        BasicConfig::new()
            .with_level(Level::INFO)
            .with_formatter(Formatter::from_str("%(levelname)s %(name)s %(message)s")?)
            .with_handler(file.clone()),
    )?;

    log::info!(target: "trades::feed", "opened {} venues", 3);
    log::debug!(target: "trades::feed", "below the root's level");
    logging::get_logger("trades.book").warning("crossed");

    logging::shutdown();
    assert_eq!(
        file.io().read_all_bytes()?,
        b"INFO trades.feed opened 3 venues\nWARNING trades.book crossed\n"
    );
    ```

=== "Python"

    ```python
    import logging
    import pathlib
    import shutil
    import tempfile

    from yggdryl.logging import FileHandler

    folder = pathlib.Path(tempfile.mkdtemp())
    path = folder / "app.log"
    logging.basicConfig(
        level=logging.INFO,
        format="%(levelname)s %(name)s %(message)s",
        handlers=[FileHandler(path)],
    )

    logging.getLogger("trades.feed").info("opened %d venues", 3)
    logging.getLogger("trades.feed").debug("below the root's level")
    logging.getLogger("trades.book").warning("crossed")

    logging.shutdown()
    assert path.read_text(encoding="utf-8") == "INFO trades.feed opened 3 venues\nWARNING trades.book crossed\n"
    shutil.rmtree(folder)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { logging } = require('yggdryl')

    const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-logging-app-'))
    const location = path.join(folder, 'app.log')
    logging.basicConfig({
      level: 'INFO',
      format: '%(levelname)s %(name)s %(message)s',
      handlers: [new logging.FileHandler(location)],
    })

    logging.getLogger('trades.feed').info('opened 3 venues')
    logging.getLogger('trades.feed').debug("below the root's level")
    logging.getLogger('trades.book').warning('crossed')

    logging.shutdown()
    assert.equal(fs.readFileSync(location, 'utf8'), 'INFO trades.feed opened 3 venues\nWARNING trades.book crossed\n')
    fs.rmSync(folder, { recursive: true, force: true })
    ```

A record that reaches no handler on its propagating chain goes to the last resort: a `StreamHandler::stderr()` from `WARNING` up, writing the [terminal line](#terminal) coloured as the [rule](#colour) says - under the Node addon too - until `set_last_resort` replaces it or `None` drops such records. Under a host the last resort is silent: the host has its own, and Python's `logging.lastResort` is left as Python made it.

## The log facade

`install` makes the tree the backend of the [`log`](https://docs.rs/log) facade, which the crate's own records, its dependencies' and the application's go through: a record's target names its logger with `::` spelled `.`, so `yggdryl::iceberg::table` is `yggdryl.iceberg.table`. The facade's ceiling, `log::max_level`, follows the most verbose level any logger handles, so a record nothing handles is refused by the facade before its message is built.

| Target | Logger |
| --- | --- |
| `yggdryl::iceberg::table` | `yggdryl.iceberg.table` |
| `yggdryl` | `yggdryl` |
| `dependency::client` | `dependency.client`, under the foreign floor |

Installing twice is one installation, `basic_config` installs, and another `log` backend already installed is refused with `Error::Conflict`: the facade takes one per process, and a library does not replace what the application chose. `set_foreign_level(Level::WARNING)` admits a record whose target lies outside the `yggdryl` crate only at `WARNING` and above, whatever its logger's level; it is `NOTSET` - no floor - until set, and the Python binding and the Node addon set `WARNING`, so a crate the build depends on says what went wrong and never what it did.

The facade makes and remembers a logger for at most 4,096 distinct targets. Past that a new target makes no logger, so targets minted per tenant or per request cannot grow the tree: its records go to the nearest logger the tree already has - its own once `get_logger` made it, else its nearest existing ancestor, else the root - and keep their own name, the target with `::` spelled `.`. Static module paths never reach the bound.

Rust only: a binding installs the tree when it loads, so its records arrive with no call.

```rust
use std::cell::Cell;
use std::fmt;
use std::sync::Arc;

use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::logging::{self, FileHandler, Formatter, Handler, Level};

/// A message counting how often it is rendered.
struct Rendered<'a>(&'a Cell<u32>);

impl fmt::Display for Rendered<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.set(self.0.get() + 1);
        formatter.write_str("3 orders")
    }
}

logging::install()?;
logging::install()?;
assert!(logging::is_installed());

let feed = logging::get_logger("docs.logging.facade");
feed.set_propagating(false);
feed.set_level(Level::DEBUG);
let file = Arc::new(FileHandler::new(Buffer::new()));
file.set_formatter(Formatter::from_str("%(levelname)s %(name)s %(message)s")?);
feed.add_handler(file.clone());

let rendered = Cell::new(0);
log::debug!(target: "docs::logging::facade::orders", "{}", Rendered(&rendered));
// Below every level the logger handles: refused before the message is built.
log::trace!(target: "docs::logging::facade::orders", "{}", Rendered(&rendered));
assert_eq!(rendered.get(), 1);
assert!(log::log_enabled!(target: "docs::logging::facade::orders", log::Level::Debug));
assert!(!log::log_enabled!(target: "docs::logging::facade::orders", log::Level::Trace));
assert_eq!(file.io().read_all_bytes()?, b"DEBUG docs.logging.facade.orders 3 orders\n");
```

## Python: hosted by `logging`

Importing `yggdryl` installs the tree with Python's `logging` as its host: each Rust logger is the Python logger of the same dotted name, so `logging.getLogger("yggdryl")` is the one switch for the whole package and the records travel under their Rust module path.

- The levels are Python's: the tree asks a logger's `getEffectiveLevel()` once, raised past `logging.disable`, and keeps the answer until `logging` drops its own cache. The binding wraps `logging.Logger.manager._clear_cache`, which `setLevel`, `logging.disable`, `basicConfig` and `dictConfig` all call, so a level changed at any time applies to the next record; there is no refresh call.
- A native record is made by the logger's `makeRecord` - `pathname` the Rust file, `lineno` the Rust line, `funcName` `(unknown function)` - and handed to its `handle`, so Python's filters, handlers, propagation and `disabled` apply to it as to any record.
- The tree's own switches hold on the host's side too, under `logging` as under any Rust `Host`: a logger the tree disables hands the host nothing and answers `false` to `is_enabled_for`, and the tree's `disable` raises past what the host states.
- A `KeyboardInterrupt` a Python handler raises while it handles a native record is raised again in the main thread, so Ctrl-C still stops the program; any other exception is reported as unraisable.
- A record no Python logger handles never takes the interpreter, and a crate the build depends on reaches `logging` only at `WARNING` and above.
- `deduplicate("yggdryl")` drops the core's own repeats where they are logged, so a repeat never reaches `logging` at all ([Deduplication](#deduplication)).
- `yggdryl.logging.FileHandler(location, mode="append", capacity=0, flush_level=logging.ERROR, level=logging.NOTSET)` is a `logging.Handler` writing Python and native records alike through any [location](#filehandler), spelled by the formatter set on it - the [terminal line](#terminal), plain, until one is; it checks `mode`, `flush_level` and `level` before it takes an `IOBase`, so a refused one leaves the caller's handle usable; `logging.shutdown`, which Python runs at exit, publishes what it holds.
- `yggdryl.logging.TerminalHandler(stream=None, color=None)` and `TerminalFormatter(color=False)` write the [terminal line](#terminal), the handler coloured as the [rule](#colour) reads its stream's `isatty()` unless `color` says, the level and the milliseconds as the record names them; Python's `basicConfig()` without handlers and its `logging.lastResort` stay Python's own.

Python only: a native record reaches the Python logger its Rust module path names, at the level Python states.

```python
import logging
import pathlib
import shutil
import tempfile

import pyarrow as pa

from yggdryl import IOBase
from yggdryl.iceberg import IcebergTable, assign_field_ids

records: list[logging.LogRecord] = []


class Collect(logging.Handler):
    def emit(self, record: logging.LogRecord) -> None:
        records.append(record)


package = logging.getLogger("yggdryl")
package.addHandler(Collect())
# `setLevel` drops logging's level cache, and the native tree's with it.
package.setLevel(logging.INFO)

schema = assign_field_ids(pa.schema([pa.field("id", pa.int64(), nullable=False)]))
folder = pathlib.Path(tempfile.mkdtemp())
IcebergTable.create(IOBase(folder / "trades"), schema)

[made] = [record for record in records if record.name == "yggdryl.iceberg.table"]
assert made.levelno == logging.INFO
assert made.getMessage().startswith("created iceberg table at")
assert made.pathname.endswith("table.rs") and made.lineno > 0
assert made.funcName == "(unknown function)"

# A nearer logger's level wins from the next record on.
logging.getLogger("yggdryl.iceberg").setLevel(logging.WARNING)
records.clear()
IcebergTable.create(IOBase(folder / "quiet"), schema)
assert [record for record in records if record.name.startswith("yggdryl.iceberg")] == []
shutil.rmtree(folder)
```

## JavaScript

`const { logging } = require('yggdryl')` is the tree, Python's `logging` in camelCase: the level numbers `logging.NOTSET` to `logging.CRITICAL`, `getLogger`, `basicConfig`, `disable` (`CRITICAL` when no level is given) and `shutdown`, with `Logger`, `Formatter` and its `Formatter.terminal()`, `StreamHandler` and its `colored`, `FileHandler` and `NullHandler` reached through the namespace alone. The addon installs the tree when it loads, sets the foreign floor at `WARNING`, keeps the core's last resort - the [terminal line](#terminal) on standard error from `WARNING` up - names its thread `main`, or `worker-N` in a worker ([Threads](#threads)), and runs `shutdown` on the main thread's `process.on('exit')`, so what a handler holds is published - the tree is the process's, so a worker ending closes nothing; a level is a number or a name. `basicConfig()` attaches a `StreamHandler` on standard error writing the terminal line, and a `StreamHandler` decides `colored` from its stream when built ([Colour](#colour)).

A record logged from JavaScript carries its call site - `funcName` the calling function (`getFunctionName()`, else `getMethodName()`), `pathname` its file, `lineno` its line - read only once its level is enabled, so a disabled record never walks the stack, and the read restores `Error.prepareStackTrace` and `Error.stackTraceLimit` whatever happens, a stack overflowing as it is read included; at a script's top level, where no function is named, `%(caller)s` is the file's stem and the line. It goes through the tree like any other, so a logger's `deduplicating` - `true`, `false` or `null` - applies to it ([Deduplication](#deduplication)). Not offered: a handler calling back into JavaScript, because a record can be logged on any Rust thread and a JavaScript function runs on its isolate's thread only; a logger's `handlers` list (`hasHandlers` is); filters.

JavaScript only: the core's own warning reaches the logger its Rust module path names.

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { logging, fix } = require('yggdryl')

const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-logging-js-'))
const location = path.join(folder, 'core.log')
const handler = new logging.FileHandler(location)
handler.setFormatter(new logging.Formatter('%(levelname)s %(name)s %(message)s'))
const reader = logging.getLogger('yggdryl.fix')
reader.addHandler(handler)

// `52=bad` names no instant: the message is read and the reader says so once.
const codec = new fix.FixCodec(new fix.FixRegistry())
codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|52=bad|10=0|'))
assert.match(fs.readFileSync(location, 'utf8'), /^WARNING yggdryl\.fix\.\w+ FIX clock left unstated/)

reader.removeHandler(handler)
handler.close()
fs.rmSync(folder, { recursive: true, force: true })
```

## Shutdown

`shutdown` publishes and closes every handler attached anywhere in the tree, and the last resort - Python's `logging.shutdown`. A handler that fails - a `FileHandler` whose handle the calling thread holds, refused as a [deadlock](#filehandler), among them - says so on standard error and the others still close, and a handler logged to again reopens its location.

| Runtime | Who calls it |
| --- | --- |
| Rust | the application, before `main` returns: the process ends without dropping what a static holds |
| Python | `logging.shutdown`, which Python runs at exit, closing each `yggdryl.logging.FileHandler` |
| JavaScript | the addon, on the main thread's `process.on('exit')`, never a worker's; `logging.shutdown()` beside it |

Rust only: a `FileHandler` dropped by its owner publishes what it held, as `shutdown` does for every handler of the tree.

```rust
use yggdryl::holder::Holder;
use yggdryl::local::LocalFolder;
use yggdryl::logging::{FileHandler, Formatter, Handler, Level, Record};

let root = LocalFolder::temporary()?.path()?.join(format!("yggdryl-docs-logging-shutdown-{}", std::process::id()));
let path = root.join("held.log");
let handler = FileHandler::new(Holder::local(&path)?).with_capacity(1 << 20);
handler.set_formatter(Formatter::default());
handler.handle(&Record::new("docs.logging.shutdown", Level::WARNING, &"held until the end"));
assert!(!path.exists());
drop(handler);
assert_eq!(std::fs::read_to_string(&path)?, "held until the end\n");
std::fs::remove_dir_all(&root)?;
```

## Warnings

What a parse, a market projection, a lifecycle walk or a book walk passes over is said once as a deduplicated `WARN` record on the facade, located where it was raised: its logger the raising module's path - `yggdryl.fix.messages`, `yggdryl.fix.build` - its file and line those of the raise, so `%(caller)s` is the module's last segment and that line, and it reaches that logger and its ancestors like any other record ([Warnings](fix/capture.md#warnings)). Under the Node addon with nothing configured, the last resort writes it, and its tenfold counts as `... (sendingtime): seen 10 times`; in a worker the thread reads `[worker-1]`:

```text
2026-10-03 07:49:27,953 ! WARNING  [main] yggdryl.fix.build build:1387 › FIX clock left unstated: its text names no instant (sendingtime): ...; later occurrences are counted rather than repeated
```

The warnings are counted by the engine of [Deduplication](#deduplication) in a table of their own, keyed by where a warning is raised, what went wrong and its subject, so an application's deduplicating logger never takes room a data warning needs. Past 4,096 kinds a new kind is not said: they are counted together, and one line on `yggdryl.warning` - `4096 distinct warnings were logged; N of other kinds were not` - says how many at the 1st, 10th, 100th... of them. The `yggdryl` command installs the tree and attaches a handler to its root that keeps the core's warnings while a command's progress line is drawn, and prints them on standard output once it is done: a count prefixed `!` and one line each, or one workflow annotation each under `--annotate`.

## Edges

- `get_logger("")` and `get_logger("root")` -> the root, named `root`; `child("x")` on the root -> the logger `x`.
- A logger asked for before its parent -> hangs from its nearest existing ancestor, then from the parent once it is made.
- A logger at `NOTSET` -> the nearest ancestor's level; the root -> `WARNING` until set.
- A level name in any case, `WARN`, `FATAL`, or a number from `0` to `255` -> read; `verbose`, `256`, `-1`, `1.5` -> `invalid log level expression at byte 0: ...`; JavaScript `1.5` or `256` as a number -> `level must be an unsigned 8-bit integer`.
- `Logger::handle(&record)` -> a record built by hand reaches the handlers and the host whatever its level, as Python's `Logger.handle`; on a disabled logger -> nothing handed over.
- A handler added twice -> attached once; `remove_handler` -> by identity, answering whether it was attached, and never closing it.
- A logger's filters -> only records logged on that logger, never those propagating from a child; a handler's filters -> every record handed to it.
- A disabled logger -> drops what is logged on it, before its handlers and the host alike, and answers `false` to every level; a child's records still pass through it.
- A deduplicating logger -> counts a record by its logger, level and message where it is logged: the first as logged, the 10th, 100th, 1000th as `message (seen N times)`, every other reaching no handler, no host and no last resort; the instant and the location are no part of the key.
- `set_deduplicating(Some(false))` below a deduplicating logger -> every record on it and below passes; `None` -> the nearest ancestor's statement again, off at the root unless set.
- More than 4,096 distinct deduplication keys, however many threads race -> on a deduplicating logger each new one past the bound is said in full and uncounted (`Repeat::Untracked`), a key already counted keeps counting; past 4,096 kinds of data warning -> a new kind is not said, and one line on `yggdryl.warning` counts them at the 1st, 10th, 100th...
- Python `Deduplicate(name)` -> counts any record by the core's hash in a table of its own; a record outside `name` passes uncounted, and so does a call whose message cannot be formatted, for its handler's `handleError` to report; on several handlers -> each counts the message the record was logged with; `deduplicate(logger)` -> only the records the core logs.
- `disable(level)` -> every record at or below `level` dropped on every logger, under a host too; `disable(NOTSET)` lifts it.
- No handler on the propagating chain -> the last resort at `WARNING` and above, the terminal line; `set_last_resort(None)` -> dropped; under a host -> the host's own.
- A handler with no formatter set -> the terminal line, coloured only if the handler `is_colored`; `Formatter::default()` -> `%(message)s`, Python's default `Formatter()`.
- `NO_COLOR` set to anything but the empty text -> no colour, `FORCE_COLOR` included; an empty `NO_COLOR` -> unset; `FORCE_COLOR` or `CLICOLOR_FORCE` empty, `0` or `false` -> not forced; forced -> coloured under `TERM=dumb` and into a pipe.
- `%(levelcolor)s`, `%(dim)s`, `%(bold)s`, `%(reset)s` -> nothing through `format` and `format_into`, their escape sequences through `format_colored_into`.
- A multi-line message under `Formatter::terminal()` -> its later lines start under its first, counted in terminal columns - a wide character or an emoji two, a combining mark, a zero-width character or an escape sequence none - through `with_datefmt` and `with_timezone`; a message ending in a newline -> no trailing indent; under `Formatter::from_str(Formatter::TERMINAL_FORMAT)` or any other format -> not indented.
- `%(caller)s` of a record stating no function, no module and no file -> `-`; a function and no line -> the function alone.
- A thread nobody named, logging a record that names none -> `[Thread-N]`, `N` its `%(thread)d`; `set_thread_name("")` -> the name forgotten.
- A handler that fails to emit -> `--- Logging error ---` and the record on standard error; the caller never sees it, and a standard error that cannot take the report -> left alone, never a panic.
- A record logged by a handler while it spells another -> a line buffer of its own; a message whose `Display` logs -> rendered outside the formatter's lock, so a `set_formatter` meanwhile never deadlocks against it.
- A `StreamHandler::new(writer)` whose writer logs into the same handler while it writes -> that line written after the one being written; a flush the writer asks for while writing -> nothing.
- A line longer than 64 KiB -> spelled in a buffer of its own size, and the thread's buffer shrinks back.
- A `FileHandler` publish the store refuses -> the records it carried are lost and said on standard error.
- A `FileHandler` buffer grown past its capacity and 64 KiB -> released after the publish.
- `FileHandler` mode `merge`, `readonly` or `random` -> `invalid record value at $.mode: expected append or overwrite for a log handler, got merge`; Python and JavaScript `a` or `w` -> `invalid mode expression at byte 0: ...`.
- A record at or above the flush level -> publishes everything held at once, whatever the capacity.
- A record logged on another thread while a `FileHandler` publishes -> never waits ([FileHandler](#filehandler)).
- `FileHandler::io()` -> the handle, locked: a record arriving on another thread is held and published once the guard drops if it came due, one logged on the guard's own thread waits for the next publish; on that thread `io()` again waits on itself, and `flush` or `close` -> `Error::Io` of kind `Deadlock`, as from inside the handler's own publish.
- `FileHandler::write(line, level)` -> holds a line another formatter already spelled, as the Python handler does.
- `install` beside another `log` backend -> `Error::Conflict`; the Python binding and the Node addon then leave that backend in place, and the core's records go to it.
- A facade target outside `yggdryl` -> held to the foreign floor; `yggdryl_cli::shell` is outside, `yggdryl` and `yggdryl::...` are inside.
- More than 4,096 distinct facade targets -> each new one past the bound makes no logger: its records go to the nearest logger the tree has - its own once `get_logger` made it, else its nearest existing ancestor, else the root - under their own name.
- `%(name)d` -> refused at byte 7: `name is text and takes the s, r or a conversion`; Python's formatter accepts it and fails on the first record, through `handleError`.
- `%(created)x` -> refused: `created is a fraction and takes the s, r, a, d, i, u, f, F, e, E, g or G conversion`; `%(levelname)q` -> `expected one of the conversions s, r, a, d, i, u, o, x, X, e, E, f, F, g, G, c, got 'q'`.
- A width or a precision past 4096, or a number too large to read -> refused at its digits: `expected a width of at most 4096, got 99999`, `expected a precision of at most 4096, got ...`.
- A format naming no key -> refused (`plain text`), in Python too.
- `%(msecs)s` -> `123.0`; `%(created)f` -> six places; `%(levelname).4s` -> `WARN`; `%(created)d` of a fraction between `-1` and `0` -> `0`; `%(levelno)#06X` -> `0X001E`; `%(msecs)#g` -> `123.000`; the length modifiers `h`, `l`, `L` -> read and ignored.
- `relativeCreated` -> never negative: a record made before the tree first answered reads `0`.
- A date directive outside the list (`%Q`) -> `invalid log date format expression at byte N: expected one of the directives ...`.
- A zone with no rules -> rendered as UTC.
- Python `flush_level=` -> read by the core, any case; `level=` -> Python's `Handler` level, Python's names; a refused `mode`, `flush_level` or `level` -> the `IOBase` given left usable.
- A Rust `trace` record in Python -> `Level 5`, a level Python names nothing; a level `logging.addLevelName` registered -> that name in `TerminalFormatter`'s line.
- A Python handler raising `KeyboardInterrupt` over a native record -> raised again in the main thread; any other exception -> reported as unraisable.
- JavaScript `StreamHandler('file')` -> `expected the stream stderr or stdout, got "file"`.
- JavaScript `basicConfig({ datefmt })` without a `format` -> the terminal line with that date format, still hanging.
- A JavaScript record logged where no function is named -> `%(caller)s` the file's stem and the line, `%(funcName)s` `(unknown function)`.
- A JavaScript worker ending -> no `shutdown`: only the main thread's `exit` closes the tree's handlers.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test logging
    cargo test -p yggdryl --features internals --test logging warning
    cargo test -p yggdryl --features internals --test logging terminal
    cargo test -p yggdryl --test iobase_calls a_log_handler
    cargo test -p yggdryl --test allocations a_log_record
    cargo test -p yggdryl --doc logging
    cargo bench -p yggdryl --bench logging
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_logging.py -q
    python python/benchmarks/logger.py --iterations 20000   # against a release wheel
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/logging.test.js
    npm run --prefix node bench:logging                     # YGGDRYL_BENCH_ITERATIONS, 20000 by default
    ```

## Performance

Criterion medians of the `logging` target in a release build (thin LTO, one codegen unit), every fixture built outside the measured loop ([benchmarks](benchmarks.md)): Linux x86_64 on a 4-vCPU Intel Xeon at 2.10 GHz, rustc 1.97.0.

The allocation claims - a disabled record allocates nothing, an enabled one spelled into a handler allocates nothing once its thread's line buffer is warm, and a repeat on a deduplicating logger allocates nothing - are pinned in `rust/tests/allocations.rs` rather than timed. A file row times 4,096 records and the `flush` after them, the handler built outside the timer.

| Row | What it measures | Time |
| --- | --- | --- |
| `logging/disabled/logger` | `Logger::debug` on a logger at the root's `WARNING`: the cached threshold read, nothing built | 2.84 ns |
| `logging/disabled/facade` | `log::debug!` on a target nothing handles: refused at the facade's ceiling | 0.35 ns |
| `logging/format/std_fmt_baseline` | the next row's line written with `std::fmt` into a reused `String`: what a hand-rolled logger costs | 64.4 ns |
| `logging/format/asctime_levelname_name_message` | `Formatter::format_into` of `%(asctime)s %(levelname)-8s %(name)s: %(message)s` into a reused `String` | 232 ns |
| `logging/format/terminal` | `Formatter::terminal().format_into` of a record stating its function, line and thread: the plain terminal line | 337 ns |
| `logging/format/terminal_colored` | the same record through `format_colored_into`: the line a colour terminal is sent | 364 ns |
| `logging/enabled/facade_to_sink` | `log::info!` through the facade, spelled in the `asctime_levelname_name_message` row's format by a `StreamHandler` over `std::io::sink()` | 467 ns |
| `logging/repeated/deduplicated` | `Logger::info` of a repeat on a deduplicating logger: the message hashed, counted in the lock-free table and dropped before any handler | 83.5 ns |
| `logging/repeated/stable_hash_and_count` | `Repeats::count_record` of one record: its `stable_hash` and the table's count | 54.0 ns |
| `logging/file/append_each [append_bytes=1]` | 4,096 records spelled in the terminal format through a `FileHandler` over a counted `Buffer` at capacity `0`: one `append_bytes` per record | 1.85 ms (452 ns a record) |
| `logging/file/capacity_64k [append_bytes=1 per 64 KiB]` | the same 4,096 records held under a 64 KiB capacity: one `append_bytes` for all of them, at the closing `flush` | 1.54 ms (375 ns a record) |

```bash
cargo bench -p yggdryl --bench logging
```
