# yggdryl-logging in Rust

`yggdryl::logging` - `Level`, `get_logger` and `Logger`, the handlers
(`StreamHandler`, `NullHandler`, `FileHandler<H>` over any `IOBase`, and the
`Handler` trait over a `HandlerState`), `Formatter`, `Record`,
`BasicConfig`/`basic_config`, `install`, `disable`, `shutdown` and `Repeats`.
No feature flag is needed; the crate logs through the `log` facade, so a
`log` dependency is how an application's own `log::info!` reaches the tree.

The tree is one per process. The recipes below name their loggers
`skills.logging.*`, stop their propagation, and put back every level and
handler they changed - do the same in tests, which share the process.

## Set up an application

`basic_config` is the one call an application makes: it installs the tree
as the `log` backend and gives a root with no handler the handlers it is
given - `StreamHandler::stderr()` writing the terminal line when none is -
and the level. `shutdown()` before `main` returns publishes what any
handler holds back: the process drops no static. This block configures the
process's root, so it is shown here rather than run beside the others.

```{ .rust .ignore }
use std::sync::Arc;

use yggdryl::local::LocalFile;
use yggdryl::logging::{self, BasicConfig, FileHandler, Formatter, Handler, Level, StreamHandler};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The terminal line on standard error, and a file holding 64 KiB per append.
    let file = Arc::new(FileHandler::new(LocalFile::new("logs/app.log")?).with_capacity(64 * 1024));
    file.set_formatter(Formatter::from_str("%(asctime)s %(levelname)s %(name)s %(message)s")?);
    logging::basic_config(
        BasicConfig::new()
            .with_level(Level::INFO)
            .with_handler(Arc::new(StreamHandler::stderr()))
            .with_handler(file),
    )?;
    // One part of the core, one level down: the AWS credential walk.
    logging::get_logger("yggdryl.aws.session").set_level(Level::DEBUG);

    log::info!("started");
    logging::get_logger("trades.feed").warning("2 late fills");

    logging::shutdown();
    Ok(())
}
```

## Levels

A level is a number on Python's scale; a name reads in any case, with
`WARN` and `FATAL`, and so does its number as text.

```rust
use yggdryl::logging::Level;

assert_eq!(Level::from_str("warn")?, Level::WARNING);
assert_eq!(Level::from_str("FATAL")?, Level::CRITICAL);
assert_eq!(Level::from_str("15")?, Level::new(15));
assert_eq!(Level::new(15).to_string(), "Level 15");
assert!(Level::DEBUG < Level::new(15) && Level::new(15) < Level::INFO);
assert_eq!(Level::from(log::Level::Trace), Level::TRACE);
assert_eq!((Level::INFO.get(), Level::INFO.name(), Level::INFO.glyph()), (20, Some("INFO"), "•"));

let refused = Level::from_str("verbose").unwrap_err().to_string();
assert!(refused.starts_with("invalid log level expression at byte 0:"), "{refused}");
assert!(Level::from_str("256").is_err());
```

## A logger, a handler, a level

A logger hangs from its nearest existing ancestor, takes its level while at
`NOTSET`, and hands its records to its own handlers and every ancestor's
until one does not propagate. A logger's filter judges the records logged on
it. A message is any `Display`, rendered only by a handler that writes it.

```rust
use std::sync::Arc;

use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::logging::{self, FileHandler, Filter, Formatter, Handler, Level, Record};

// Asked for after its child, the parent still becomes the child's parent.
let orders = logging::get_logger("skills.logging.tree.orders");
let tree = logging::get_logger("skills.logging.tree");
assert_eq!(orders.parent(), Some(tree.clone()));
assert_eq!(tree.child("orders"), orders);
assert_eq!(logging::get_logger("").name(), "root");

tree.set_level(Level::INFO);
tree.set_propagating(false);
assert_eq!((orders.level(), orders.effective_level()), (Level::NOTSET, Level::INFO));
assert!(!orders.is_enabled_for(Level::DEBUG));

let file = Arc::new(FileHandler::new(Buffer::new()));
file.set_formatter(Formatter::from_str("%(levelname)s %(name)s %(message)s")?);
let handler: Arc<dyn Handler> = file.clone();
tree.add_handler(handler.clone());
let quiet: Filter = Arc::new(|record: &Record<'_>| record.message().to_string() != "heartbeat");
orders.add_filter(quiet.clone());

orders.info("opened");
orders.info("heartbeat");
orders.debug("below the level");
tree.warning(format_args!("{} late fills", 2));
assert_eq!(
    file.io().read_all_bytes()?,
    b"INFO skills.logging.tree.orders opened\nWARNING skills.logging.tree 2 late fills\n"
);
// A logger displays its name and effective level.
assert_eq!(orders.to_string(), "skills.logging.tree.orders (INFO)");

// Put back what the example changed: the tree is the process's.
assert!(orders.remove_filter(&quiet));
assert!(tree.remove_handler(&handler));
tree.set_level(Level::NOTSET);
tree.set_propagating(true);
```

## Write a log through any handle

`FileHandler::new(handle)` takes any `IOBase` - `LocalFile`, `Buffer`, an
S3 object, a ZIP member - and publishes with one `append_bytes`: each
record at once by default, or, under `with_capacity`, once that many bytes
are held or a record at `with_flush_level` (`ERROR` by default) arrives. It
opens its handle on its first publish, creating the file and its parents. A
record logged on another thread while a publish is in flight is held, never
waited on: the publishing thread takes it in.

```rust
use std::sync::Arc;

use yggdryl::holder::Buffer;
use yggdryl::local::{LocalFile, LocalFolder};
use yggdryl::logging::{self, FileHandler, Formatter, Handler, Level};
use yggdryl::{IOBase, IOMode};

let logger = logging::get_logger("skills.logging.file");
logger.set_level(Level::INFO);
logger.set_propagating(false);

// A local file: each record appended as it arrives.
let folder = format!("yggdryl-skills-logging-file-{}", std::process::id());
let root = LocalFolder::temporary()?.path()?.join(folder);
let location = root.join("logs").join("feed.log");
let local = Arc::new(FileHandler::new(LocalFile::new(&location)?));
local.set_formatter(Formatter::from_str("%(levelname)s %(message)s")?);
let local_handler: Arc<dyn Handler> = local.clone();
logger.add_handler(local_handler.clone());
logger.info("opened 3 venues");
assert_eq!(std::fs::read_to_string(&location)?, "INFO opened 3 venues\n");
assert!(logger.remove_handler(&local_handler));
local.close()?;

// Records held back, as a handler over an object store states: nothing
// reaches the handle until the capacity fills or an ERROR arrives.
let held = Arc::new(FileHandler::new(Buffer::new()).with_capacity(64 * 1024));
held.set_formatter(Formatter::default());
assert_eq!((held.mode(), held.capacity(), held.flush_level()), (IOMode::Append, 65_536, Level::ERROR));
let held_handler: Arc<dyn Handler> = held.clone();
logger.add_handler(held_handler.clone());
logger.info("filled 3");
assert!(held.io().read_all_bytes()?.is_empty());
logger.error("rejected");
assert_eq!(held.io().read_all_bytes()?, b"filled 3\nrejected\n");
logger.info("closed");
held.flush()?;
assert_eq!(held.io().read_all_bytes()?, b"filled 3\nrejected\nclosed\n");

assert!(logger.remove_handler(&held_handler));
logger.set_level(Level::NOTSET);
logger.set_propagating(true);
std::fs::remove_dir_all(&root)?;
```

## Overwrite, close, and what a drop publishes

`with_mode(IOMode::Overwrite)` replaces what the handle holds with the
handler's first publish, and only that one; every mode but `Append` and
`Overwrite` is refused. A handler that is closed or dropped publishes what it
held, as `logging::shutdown()` does for every handler in the tree. `io()` is
the handle, locked - drop its guard before a `flush` or `close` on the same
thread, which is otherwise refused as `Error::Io` of kind `Deadlock`.

```rust
use yggdryl::holder::Buffer;
use yggdryl::logging::{FileHandler, Formatter, Handler, Level, Record};
use yggdryl::{IOBase, IOMode};

let mut yesterday = Buffer::new();
yesterday.write_all_bytes(b"yesterday\n")?;
let fresh = FileHandler::new(yesterday).with_mode(IOMode::Overwrite)?;
fresh.set_formatter(Formatter::default());
// Nothing is replaced before a record is published.
assert_eq!(fresh.io().read_all_bytes()?, b"yesterday\n");
// `handle` hands a record straight to a handler; the level is a logger's to check.
fresh.handle(&Record::new("skills.logging.overwrite", Level::INFO, &"today"));
fresh.handle(&Record::new("skills.logging.overwrite", Level::INFO, &"later"));
assert_eq!(fresh.io().read_all_bytes()?, b"today\nlater\n");

let refused = FileHandler::new(Buffer::new()).with_mode(IOMode::Merge).unwrap_err();
assert!(refused.to_string().contains("expected append or overwrite"), "{refused}");

let held = FileHandler::new(Buffer::new()).with_capacity(1024);
held.set_formatter(Formatter::default());
held.handle(&Record::new("skills.logging.overwrite", Level::WARNING, &"held"));
{
    let guard = held.io();
    assert_eq!(guard.read_all_bytes()?, b"");
    let deadlock = held.flush().unwrap_err();
    assert!(matches!(&deadlock, yggdryl::Error::Io(io) if io.kind() == std::io::ErrorKind::Deadlock));
}
held.close()?;
assert_eq!(held.io().read_all_bytes()?, b"held\n");
```

## Formats and the terminal line

`Formatter::from_str` is Python's `%`-style format, parsed once and refused
at the byte. Dates are UTC unless `with_timezone` names a zone.
`Formatter::terminal()` is the line every handler with no formatter writes;
a coloured handler spells its styles through `format_colored_into`.

```rust
use std::io::IsTerminal;

use yggdryl::Timezone;
use yggdryl::logging::{self, Formatter, Handler, Level, Record, StreamHandler};

// 2023-11-14 22:13:20.123 UTC, raised in `open` at line 42 on the thread `main`.
let record = Record::new("trades.feed", Level::INFO, &"opened 3 venues")
    .with_location("src/feed.rs", 42)
    .with_function("open")
    .with_thread("main")
    .with_created(1_700_000_000_123_000_000);

let plain = Formatter::from_str("%(asctime)s %(levelname)-8s %(name)s:%(lineno)d %(message)s")?;
assert_eq!(plain.format(&record), "2023-11-14 22:13:20,123 INFO     trades.feed:42 opened 3 venues");
let paris = Formatter::from_str("%(asctime)s %(message)s")?
    .with_datefmt("%H:%M %Z")?
    .with_timezone(Timezone::from_str("Europe/Paris")?);
assert_eq!(paris.format(&record), "23:13 CET opened 3 venues");
assert_eq!(Formatter::default().format(&record), "opened 3 venues");
let basic = Formatter::from_str(Formatter::BASIC_FORMAT)?;
assert_eq!(basic.format(&record), "INFO:trades.feed:opened 3 venues");

let terminal = Formatter::terminal();
assert_eq!(terminal.as_str(), Formatter::TERMINAL_FORMAT);
assert_eq!(
    terminal.format(&record),
    "2023-11-14 22:13:20,123 • INFO     [main] trades.feed open:42 › opened 3 venues"
);
let mut colored = String::new();
terminal.format_colored_into(&record, &mut colored);
assert!(
    colored.starts_with("\x1b[2m2023-11-14 22:13:20,123\x1b[0m \x1b[32m• INFO    \x1b[0m"),
    "{colored}"
);

let refused = Formatter::from_str("%(name)d").unwrap_err().to_string();
assert_eq!(
    refused,
    "invalid log format expression at byte 7: name is text and takes the s, r or a conversion"
);

// Standard error asks the colour rule once (NO_COLOR, FORCE_COLOR,
// CLICOLOR_FORCE, TERM=dumb, a terminal); a writer is plain until set.
assert_eq!(
    StreamHandler::stderr().is_colored(),
    logging::is_color_enabled(std::io::stderr().is_terminal())
);
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

## A handler of your own, and the null handler

Rust only: implement `state` and `emit`; the level, the formatter, the
filters, `handle`, `flush` and `close` are provided. A `NullHandler` takes
every record and writes none, so a library's logger keeps the last resort
from speaking for it.

```rust
use std::sync::{Arc, Mutex};

use yggdryl::logging::{self, Formatter, Handler, HandlerState, Level, NullHandler, Record};

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
let handler: Arc<dyn Handler> = collect.clone();

let logger = logging::get_logger("skills.logging.custom");
logger.set_level(Level::DEBUG);
logger.set_propagating(false);
logger.add_handler(handler.clone());
logger.info("under the handler's level");
logger.warning("late fill");
assert_eq!(*collect.lines.lock().unwrap(), ["WARNING:skills.logging.custom:late fill"]);

let null: Arc<dyn Handler> = Arc::new(NullHandler::new());
let library = logging::get_logger("skills.logging.library");
library.add_handler(null.clone());
assert!(library.has_handlers());

assert!(library.remove_handler(&null));
assert!(logger.remove_handler(&handler));
logger.set_level(Level::NOTSET);
logger.set_propagating(true);
```

## Deduplicate repeats

A deduplicating logger counts each record by its logger, level and message
where it is logged: the first goes on as logged, the 10th, 100th, 1000th as
`message (seen N times)`, every other reaches no handler. A logger takes its
nearest ancestor's statement; `None` inherits again. `Repeats` is the same
lock-free table on its own, counting any key a caller hashes.

```rust
use std::sync::Arc;

use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::logging::{self, Counted, FileHandler, Formatter, Handler, Level, Record, Repeat, Repeats};

let feed = logging::get_logger("skills.logging.dedup");
let orders = feed.child("orders");
feed.set_level(Level::INFO);
feed.set_propagating(false);
let file = Arc::new(FileHandler::new(Buffer::new()));
file.set_formatter(Formatter::default());
let handler: Arc<dyn Handler> = file.clone();
feed.add_handler(handler.clone());

assert_eq!((feed.deduplicating(), orders.is_deduplicating()), (None, false));
feed.set_deduplicating(Some(true));
assert!(orders.is_deduplicating());
for _ in 0..100 {
    orders.warning("late fill");
}
// Another message, or the same one at another level, is another key.
orders.error("late fill");
assert_eq!(
    file.io().read_all_bytes()?,
    b"late fill\nlate fill (seen 10 times)\nlate fill (seen 100 times)\nlate fill\n",
);

feed.set_deduplicating(None);
assert!(feed.remove_handler(&handler));
feed.set_level(Level::NOTSET);
feed.set_propagating(true);

let repeats = Repeats::new();
let record = Record::new("skills.logging.dedup.table", Level::WARNING, &"late fill");
assert_eq!(repeats.count_record(&record), Repeat::First);
for _ in 2..10 {
    assert_eq!(repeats.count_record(&record), Repeat::Repeated);
}
assert_eq!(repeats.count_record(&record), Repeat::Tenfold(10));
assert_eq!(repeats.seen(record.stable_hash()), 10);
assert_eq!(Counted::new(10, &"late fill").to_string(), "late fill (seen 10 times)");
```

## Route the `log` facade

`install()` makes the tree the `log` backend - the crate's own records, its
dependencies' and the application's - with a record's target naming its
logger, `::` spelled `.`. Until a backend is installed - this one through
`install()` or `basic_config`, or another such as `env_logger` - the
crate's own records go nowhere. Installing twice is one installation;
another backend already installed is `Error::Conflict`, which `basic_config`
returns with nothing configured. The facade's ceiling follows the tree, so
a record nothing handles is refused before its message is built.
`set_foreign_level(Level::WARNING)` - what the bindings state - admits a
target outside `yggdryl` only from that level.

```rust
use std::sync::Arc;

use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::logging::{self, FileHandler, Formatter, Handler, Level};

logging::install()?;
logging::install()?;
assert!(logging::is_installed());

let facade = logging::get_logger("skills.logging.facade");
facade.set_level(Level::DEBUG);
facade.set_propagating(false);
let file = Arc::new(FileHandler::new(Buffer::new()));
file.set_formatter(Formatter::from_str("%(levelname)s %(name)s %(message)s")?);
let handler: Arc<dyn Handler> = file.clone();
facade.add_handler(handler.clone());

log::debug!(target: "skills::logging::facade::orders", "{} orders", 3);
log::trace!(target: "skills::logging::facade::orders", "under the level: never built");
assert!(!log::log_enabled!(target: "skills::logging::facade::orders", log::Level::Trace));
assert_eq!(file.io().read_all_bytes()?, b"DEBUG skills.logging.facade.orders 3 orders\n");

assert!(facade.remove_handler(&handler));
facade.set_level(Level::NOTSET);
facade.set_propagating(true);
```

## Read what the core reports

The core logs one record per operation under its module path:
`yggdryl.iceberg.table` at `INFO` for a create or a commit,
`yggdryl.aws.session` for each credential source the AWS chain asks
(`DEBUG`), the one that answered (`INFO`) and a set passed over
(`WARNING`), and the data warnings - `yggdryl.fix.messages`,
`yggdryl.fix.build` - once per kind, then at each tenfold count. A handler on
the logger, or on `yggdryl` for everything, takes them.

```rust
use std::sync::Arc;

use yggdryl::holder::Buffer;
use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec, assign_field_ids};
use yggdryl::local::LocalFolder;
use yggdryl::logging::{self, FileHandler, Formatter, Handler, Level};
use yggdryl::{DataType, IOBase, StructType};

logging::install()?;
let watcher = logging::get_logger("yggdryl.iceberg.table");
watcher.set_level(Level::INFO);
let held = Arc::new(FileHandler::new(Buffer::new()));
held.set_formatter(Formatter::from_str("%(levelname)s %(name)s %(message)s")?);
let handler: Arc<dyn Handler> = held.clone();
watcher.add_handler(handler.clone());

let mut schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
    .required_field("row");
assign_field_ids(&mut schema, 1)?;
let folder = format!("yggdryl-skills-logging-core-{}", std::process::id());
let path = LocalFolder::temporary()?.path()?.join(&folder);
let _ = std::fs::remove_dir_all(&path);
let table = IcebergTable::create(
    LocalFolder::new(&path)?,
    FormatVersion::V2,
    schema,
    PartitionSpec::unpartitioned(),
)?;

// Tables other code creates meanwhile are narrated too: read this folder's line.
let said = String::from_utf8(held.io().read_all_bytes()?)?;
assert!(
    said.lines().any(|line| {
        line.starts_with("INFO yggdryl.iceberg.table created iceberg table at") && line.contains(&folder)
    }),
    "{said}"
);

assert!(watcher.remove_handler(&handler));
watcher.set_level(Level::NOTSET);
drop(table);
std::fs::remove_dir_all(&path)?;
```

## Gotchas in Rust

- `remove_handler(&handler)` and `remove_filter(&filter)` compare by `Arc`
  identity: keep the `Arc<dyn Handler>` or `Filter` you added. Removing never
  closes the handler, and adding it twice attaches it once.
- `Logger::handle(&record)` hands a record you built to the chain's handlers
  whatever the logger's level (a disabled logger still hands nothing, and
  each handler still checks its own level), and `Handler::handle` filters,
  spells and emits a record whatever the handler's own level: the logger
  checks it, as in Python.
- A logger holds the handlers attached to it, so dropping your own `Arc` does
  not publish them: `flush()` or `close()` the handler, or call
  `logging::shutdown()`, before `main` returns.
- A handler with no formatter set writes the terminal line, timestamp and
  call site included: set `Formatter::default()` (`%(message)s`) or your own
  format before asserting on its output.
