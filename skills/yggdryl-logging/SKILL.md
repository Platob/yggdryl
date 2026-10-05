---
name: yggdryl-logging
description: Configure and use yggdryl's logging - Python's logging owned by the Rust core - from Rust (yggdryl::logging), Python (stdlib logging hosts the core; yggdryl.logging adds handlers) and Node.js (the logging namespace). Covers loggers and levels (NOTSET 0, TRACE 5 ... CRITICAL 50), quieting or raising yggdryl's own output (getLogger("yggdryl")), the terminal line on stderr (basic_config / basicConfig, TerminalHandler, Formatter.terminal, NO_COLOR / FORCE_COLOR, thread names), FileHandler writing a log through any handle - local file, bucket object, ZIP member - with capacity and flush level, %-style formatters with date formats and zones, deduplicating repeats ("seen N times"), the Rust log facade, shutdown before exit, and where the core's records and warnings land (yggdryl.iceberg.table, yggdryl.aws.session, yggdryl.fix.*). Use when logging to a file or an object store, configuring an application's logging, or silencing, filtering or deduplicating yggdryl warnings.
---

# Yggdryl logging

`yggdryl::logging` is Python's `logging` owned by the core: one tree per
process of loggers named by dots, Python's numeric levels, handlers and
`%`-style formatters, behind the `log` facade the crate logs through. Python
does not get a copy - importing `yggdryl` makes Python's own `logging` the
tree's **host**, so a core logger is the Python logger of the same name.
JavaScript reaches the tree as the frozen `logging` namespace, Python's API
in camelCase. One fact to hold: **the core's records travel under their Rust
module path** - `yggdryl::iceberg::table` is the logger
`yggdryl.iceberg.table` - so `yggdryl` is the one switch for the whole
package, in every language.

| Level | Number | Rust `log` | Glyph |
| --- | --- | --- | --- |
| `NOTSET` | 0 | - | - |
| `TRACE` | 5 | `Trace` | `·` (faint) |
| `DEBUG` | 10 | `Debug` | `·` (cyan) |
| `INFO` | 20 | `Info` | `•` (green) |
| `WARNING` | 30 | `Warn` | `!` (yellow) |
| `ERROR` | 40 | `Error` | `✗` (red) |
| `CRITICAL` | 50 | - | `‼` (bold red) |

Any number `0..=255` is a level (`Level 15`, between `DEBUG` and `INFO`).
`NOTSET` on a logger takes the nearest ancestor's level; on a handler it
emits everything. The root is at `WARNING` until set.

## Choose the door

| Task | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| import | `use yggdryl::logging::{self, BasicConfig, Level};` | `import logging` + `from yggdryl.logging import FileHandler, TerminalHandler` | `const { logging } = require('yggdryl')` |
| set up an application: the terminal line on standard error | `logging::basic_config(BasicConfig::new().with_level(Level::INFO))?` | `logging.basicConfig(level=logging.INFO, handlers=[TerminalHandler()])` | `logging.basicConfig({ level: 'INFO' })` |
| a logger; the root | `logging::get_logger("trades.feed")`; `get_logger("")` | `logging.getLogger("trades.feed")`; `getLogger()` | `logging.getLogger('trades.feed')`; `getLogger()` |
| log a record | `logger.info("opened")`, `logger.log(level, msg)`, or `log::info!` once installed | `logger.info("opened %d venues", 3)` | `logger.info('opened')`, `logger.log(level, msg)` |
| a level | `logger.set_level(Level::DEBUG)`, `effective_level()`, `is_enabled_for(l)` | `setLevel`, `getEffectiveLevel`, `isEnabledFor` | `setLevel('debug')`, `getEffectiveLevel()`, `isEnabledFor('info')` |
| read a level from text | `Level::from_str("warn")?`, `Level::new(15)` | Python's numbers; `flush_level="warn"` is read by the core | a number or a name, any case, wherever a level is taken |
| quiet or raise yggdryl | `get_logger("yggdryl").set_level(Level::ERROR)` | `logging.getLogger("yggdryl").setLevel(logging.ERROR)` | `logging.getLogger('yggdryl').setLevel('ERROR')` |
| keep records off the ancestors; drop a logger's | `set_propagating(false)`; `set_disabled(true)` | `propagate = False`; `disabled = True` | `propagate = false`; `disabled = true` |
| drop everything up to a level, every logger | `logging::disable(Level::INFO)`; `NOTSET` lifts | `logging.disable(logging.INFO)` | `logging.disable('INFO')` (`CRITICAL` when absent) |
| a stream handler | `StreamHandler::stderr()`, `stdout()`, `new(writer)` | `logging.StreamHandler(s)`; `TerminalHandler(stream=None, color=None)` | `new logging.StreamHandler('stderr' \| 'stdout')` |
| take records, write none | `NullHandler::new()` | `logging.NullHandler()` | `new logging.NullHandler()` |
| log through any location | `FileHandler::new(handle)` over any `IOBase` | `FileHandler(location)`: a path, `Url`, `IOBase` | `new logging.FileHandler(location)`: a path, URL text, `Url`, `IOBase` |
| batch publishes (a remote store) | `.with_capacity(1 << 20).with_flush_level(Level::ERROR)` | `capacity=1 << 20, flush_level=logging.ERROR` | `{ capacity: 1 << 20, flushLevel: 'ERROR' }` |
| replace what the location holds | `.with_mode(IOMode::Overwrite)?` | `mode="overwrite"` | `{ mode: 'overwrite' }` |
| attach, detach | `add_handler(arc)`, `remove_handler(&arc)`, `has_handlers()` | `addHandler`, `removeHandler`, `hasHandlers` | `addHandler`, `removeHandler`, `hasHandlers()` |
| a format | `handler.set_formatter(Formatter::from_str("%(levelname)s %(message)s")?)` | `handler.setFormatter(logging.Formatter(fmt, datefmt))` | `handler.setFormatter(new logging.Formatter(fmt, { datefmt, timezone }))` |
| date format, zone | `.with_datefmt("%Y-%m-%dT%H:%M:%SZ")?`, `.with_timezone(tz)` (UTC otherwise) | `datefmt=`; `formatter.converter = time.gmtime` | `{ datefmt, timezone }` (UTC otherwise) |
| the terminal line as a formatter | `Formatter::terminal()`, `Formatter::TERMINAL_FORMAT` | `TerminalFormatter(color=False)` | `logging.Formatter.terminal()` |
| colour | `logging::is_color_enabled(is_tty)`, `StreamHandler::set_colored` | `TerminalHandler(color=...)` | a `StreamHandler`'s `colored` |
| name the thread | `logging::set_thread_name("ingest")` (else the spawned name, else `Thread-N`; `""` forgets it) | Python's own `threadName` (`MainThread`) | fixed: `main`, `worker-N` |
| deduplicate repeats | `logger.set_deduplicating(Some(true))`, `None` inherits | `deduplicate("yggdryl")` (the core's records), `Deduplicate()` filter (any record) | `logger.deduplicating = true`, `null` inherits |
| filters | `logger.add_filter(Arc::new(\|r\| ..))`, `Handler::add_filter` | `addFilter` | not offered |
| a handler of your own | `impl Handler` (`state`, `emit`) | a `logging.Handler` subclass | not offered |
| route the `log` facade | `logging::install()?`, `set_foreign_level(Level::WARNING)` | done at import, foreign floor `WARNING` | done at load, foreign floor `WARNING` |
| publish and close before exit | `logging::shutdown()` | `logging.shutdown()` (Python runs it at exit) | `logging.shutdown()` (run at the main thread's exit) |
| the last resort | `logging::set_last_resort(None)` drops what no handler takes | Python's `logging.lastResort` | the core's, not replaceable |
| count keys yourself | `Repeats::new().count(key)`, `Counted::new(n, &msg)` | Rust only | Rust only |

## Rules for fast, correct use

1. **Configure once, at the application; a library only logs.** Rust:
   `basic_config` installs the tree as the `log` backend and gives a bare root
   the terminal handler (a root that has handlers is left alone unless
   `with_force(true)` / `force=True` / `{ force: true }`). Python: the stdlib
   `basicConfig`; give it `handlers=[TerminalHandler()]` for the core's line.
   A library attaches a `NullHandler` at most.
2. **`yggdryl` is the one switch.** Every core record is under
   `yggdryl.<module path>`: `yggdryl.iceberg.table` narrates creates and
   commits at `INFO`, `yggdryl.aws.session` walks the AWS credential chain
   (`DEBUG` each source asked, `INFO` the one that answered, `WARNING` a set
   passed over, key ids masked to their first and last four characters),
   `yggdryl.fix.*` and the market readers say what they passed over. The
   core logs one record per operation, never per row.
3. **Dependencies speak only at `WARNING`.** The bindings set the foreign
   floor so a crate the build depends on says what went wrong, never what it
   did; Rust states it with `set_foreign_level`, `NOTSET` (none) until set.
4. **Data warnings are said once.** A parse, a market projection, a
   lifecycle or book walk that passes over a value warns at `WARNING` under
   the raising module (`yggdryl.fix.messages`, `yggdryl.fix.build`), keyed by
   where, what and the column or tag - never the value - so it is said the
   first time with its detail, then as `... seen N times` at 10, 100,
   1000; past 4,096 kinds one line on `yggdryl.warning` counts the rest. The
   `yggdryl` command prints them on standard output after its progress line.
5. **Nothing configured still says warnings.** A record no handler takes goes
   to the last resort: the terminal line on standard error from `WARNING`
   up (Rust and JavaScript); under Python it is Python's own `lastResort`.
6. **A handler with no formatter writes the terminal line** -
   `2026-10-03 14:05:09,123 • INFO     [main] trades.feed open:42 › opened 3 venues`:
   time, glyph and level, `[thread]`, logger, call site, message. To assert
   or parse output, set a format: `Formatter::default()`,
   `logging.Formatter()` and `new logging.Formatter()` are `%(message)s`.
7. **A level change applies at the next record.** Every logger caches its
   threshold and every change drops the caches (Python's `setLevel`,
   `disable`, `basicConfig`, `dictConfig` included), so there is no refresh
   call; a disabled record costs a few atomic loads and allocates nothing.
   Rust: pass a `Display` (`format_args!(..)`), not a `format!` string - the
   message is rendered only by a handler that writes it.
8. **`FileHandler` is one `append_bytes` per publish**, through any handle.
   On an object store an append is a whole `PUT`, and in a ZIP a member
   rewrite, so state a `capacity`: records are held in memory until that
   many bytes are, or a record at the flush level (`ERROR`) arrives. A
   publish the store refuses loses the records it carried and says so on
   standard error; logging never fails the code that logs.
9. **Shut down before exit.** Rust: the process ends without dropping
   statics, so call `logging::shutdown()` before `main` returns (a handler's
   own `flush()` and `close()` publish too, and so does dropping its last
   owner - a logger holding it is one) or held records are lost. Python
   runs `logging.shutdown` at exit; Node runs it on the main thread's
   `exit` only.
10. **Deduplication is off until a logger states it**, and a logger takes its
    nearest ancestor's statement (`None`/`null` inherits). The key is the
    logger, the level and the message - never the time or place - counted
    lock-free in one table of 4,096 keys before any handler or host.
    Python: `deduplicate(name)` drops the *core's* repeats before they reach
    `logging`; a record Python logs is counted by a `Deduplicate()` filter.
11. **Colour follows the shell's conventions**: `NO_COLOR` set and non-empty
    is off, else `FORCE_COLOR` or `CLICOLOR_FORCE` (not empty, `0`/`false`/
    `no`/`off`) on, else `TERM=dumb` off, else on for a terminal only. A
    `FileHandler` is always plain; a `StreamHandler` over a writer is plain
    until `set_colored(true)` (Rust) or `colored = true` (JavaScript) says so.
12. **Formats are parsed once.** Rust and JavaScript refuse a bad format at
    the byte when it is built (`%(name)d`: `name is text and takes the s, r
    or a conversion`); Python's `logging.Formatter` fails on the first record
    instead. Rust and JavaScript spell dates in UTC unless the formatter
    names a zone; Python's formatter uses local time unless its `converter`
    is `time.gmtime`.

## Pitfalls

| Wrong | Right |
| --- | --- |
| a Rust `main` returning with a `with_capacity` handler and no `logging::shutdown()` | call `logging::shutdown()` before `main` returns, or `flush()` / `close()` the handler - statics are never dropped, and a handler still attached to a logger is never dropped by its owner letting go |
| a `FileHandler` over S3 at the default capacity `0` | `with_capacity(1 << 20)` / `capacity=1 << 20` / `{ capacity: 1 << 20 }`: one `PUT` per mebibyte, and `ERROR` still at once |
| `handler.flush()` or `close()` while this thread holds `handler.io()` | refused as `Error::Io` of kind `Deadlock`; drop the guard first (Rust only: no binding offers `io()`) |
| `FileHandler(path, mode="a")` / `{ mode: 'w' }` | `"append"` (the default) or `"overwrite"`; `merge` and the one-letter modes are refused |
| `logging.basicConfig(level=logging.INFO)` in Python expecting the core's line | add `handlers=[TerminalHandler()]`; without it Python's own format is written |
| asserting a handler's text with no formatter set | the terminal line carries a timestamp and a call site: set `%(message)s` or your own format |
| `deduplicate("trades")` in Python expecting it to count what Python code logs | it governs only the records the core logs; put a `Deduplicate()` filter on a handler for Python's own (it counts any record, after `logging` has it) |
| raising `logging.getLogger("yggdryl")` to `DEBUG` to see a dependency's debug lines | the bindings admit crates outside yggdryl only from `WARNING`; Rust lowers it with `set_foreign_level` |
| expecting `logging.TRACE` in Python | Python names no `TRACE`: a Rust `trace` record reaches it as `Level 5` |
| a JavaScript callback as a handler, `logger.handlers`, `addFilter` | not offered (a record can be logged on any Rust thread): use `StreamHandler`, `FileHandler`, `NullHandler`; `hasHandlers()` answers |
| relying on a Node worker's exit to publish a `FileHandler` | only the main thread's exit runs `shutdown`; call `logging.shutdown()` or `handler.close()` |
| `logging::install()` beside `env_logger` | the facade takes one backend: `Error::Conflict`. Either works - the core logs through `log` - but only the tree gives named loggers, levels and `FileHandler` |
| a second `basic_config` / `basicConfig` expecting a new level or format | a root with handlers is left alone: state `force` |
| `logger.info(format!("{n} rows"))` on a hot path | `logger.info(format_args!("{n} rows"))`: nothing is built while the level is disabled |

## Language references

- `references/rust.md` - read when writing Rust.
- `references/python.md` - read when writing Python.
- `references/javascript.md` - read when writing JavaScript / TypeScript.

## Deeper

- Contract, levels and the tree: https://platob.github.io/yggdryl/logging/,
  https://platob.github.io/yggdryl/logging/#levels,
  https://platob.github.io/yggdryl/logging/#loggers-and-the-tree
- Handlers, `FileHandler` and its call counts: https://platob.github.io/yggdryl/logging/#handlers,
  https://platob.github.io/yggdryl/logging/#filehandler
- Format keys, conversions and date directives: https://platob.github.io/yggdryl/logging/#formatter
- The terminal line, colour and threads: https://platob.github.io/yggdryl/logging/#terminal,
  https://platob.github.io/yggdryl/logging/#colour,
  https://platob.github.io/yggdryl/logging/#threads
- Deduplication: https://platob.github.io/yggdryl/logging/#deduplication
- Application setup, the `log` facade, shutdown: https://platob.github.io/yggdryl/logging/#configuring-an-application,
  https://platob.github.io/yggdryl/logging/#the-log-facade,
  https://platob.github.io/yggdryl/logging/#shutdown
- Python hosting and the JavaScript namespace: https://platob.github.io/yggdryl/logging/#python-hosted-by-logging,
  https://platob.github.io/yggdryl/logging/#javascript
- The core's warnings: https://platob.github.io/yggdryl/logging/#warnings,
  https://platob.github.io/yggdryl/fix/capture/#warnings; every edge:
  https://platob.github.io/yggdryl/logging/#edges; measured costs:
  https://platob.github.io/yggdryl/logging/#performance
- Sibling skills: `yggdryl` (install, conventions), `yggdryl-storage` (the
  handles a `FileHandler` writes through, the AWS credential walk
  `yggdryl.aws.session` reports), `yggdryl-records` (the Iceberg tables
  `yggdryl.iceberg.table` narrates), `yggdryl-fix` (the FIX warnings and the
  `yggdryl` command's warning summary).
