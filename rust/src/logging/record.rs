//! One logged event, as every handler and host reads it.

use std::fmt;

use super::Level;

/// One logged event: the logger it was logged on, its level, its message
/// and where it was raised - Python's `LogRecord`.
///
/// A record borrows everything it holds: the message is the caller's value,
/// rendered only by a handler that writes it, so a record no handler takes
/// costs no text. A record from the `log` facade carries the Rust target
/// (`yggdryl::iceberg::table`), the module path, the file and the line; its
/// logger's name is the target with `::` spelled `.`.
///
/// ```
/// use yggdryl::logging::{Level, Record};
///
/// let record = Record::new("trades.feed", Level::INFO, &"opened").with_location("feed.rs", 7);
/// assert_eq!(record.name(), "trades.feed");
/// assert_eq!(record.message().to_string(), "opened");
/// assert_eq!((record.file(), record.line()), (Some("feed.rs"), Some(7)));
/// assert!(record.created() > 0);
/// ```
#[derive(Clone, Copy)]
pub struct Record<'a> {
    name: &'a str,
    level: Level,
    level_name: Option<&'a str>,
    message: &'a dyn fmt::Display,
    target: &'a str,
    module_path: Option<&'a str>,
    file: Option<&'a str>,
    line: Option<u32>,
    function: Option<&'a str>,
    thread: Option<&'a str>,
    created: i64,
}

impl<'a> Record<'a> {
    /// A record of `message` at `level` on the logger `name`, created now,
    /// its target the logger's name and its location unknown.
    pub fn new(name: &'a str, level: Level, message: &'a dyn fmt::Display) -> Self {
        Self {
            name,
            level,
            level_name: None,
            message,
            target: name,
            module_path: None,
            file: None,
            line: None,
            function: None,
            thread: None,
            created: now(),
        }
    }

    /// The record naming its level `name` - what a binding states when its
    /// runtime names a level its own way: Python's `addLevelName`, and its
    /// `Level 5` where the tree says `TRACE`.
    #[must_use]
    pub const fn with_level_name(mut self, name: &'a str) -> Self {
        self.level_name = Some(name);
        self
    }

    /// The record raised at `line` of `file`.
    #[must_use]
    pub const fn with_location(mut self, file: &'a str, line: u32) -> Self {
        self.file = Some(file);
        self.line = Some(line);
        self
    }

    /// The record raised in the function or method `function` - what a
    /// binding states from its own runtime's call site.
    #[must_use]
    pub const fn with_function(mut self, function: &'a str) -> Self {
        self.function = Some(function);
        self
    }

    /// The record raised on the thread named `thread` - what a binding
    /// states when its runtime names its threads its own way; the emitting
    /// thread otherwise.
    #[must_use]
    pub const fn with_thread(mut self, thread: &'a str) -> Self {
        self.thread = Some(thread);
        self
    }

    /// The record with the Rust target and module path it was logged under.
    #[must_use]
    pub const fn with_target(mut self, target: &'a str, module_path: Option<&'a str>) -> Self {
        self.target = target;
        self.module_path = module_path;
        self
    }

    /// The record saying `message` instead.
    #[must_use]
    pub const fn with_message(mut self, message: &'a dyn fmt::Display) -> Self {
        self.message = message;
        self
    }

    /// The record dated `created`, nanoseconds since the Unix epoch.
    #[must_use]
    pub const fn with_created(mut self, created: i64) -> Self {
        self.created = created;
        self
    }

    /// The name of the logger the record was logged on: `yggdryl.fix.build`.
    pub const fn name(&self) -> &'a str {
        self.name
    }

    /// The level the record was logged at.
    pub const fn level(&self) -> Level {
        self.level
    }

    /// The level's name the record states, when it states one; a handler
    /// otherwise spells [`Level`]'s own.
    pub const fn level_name(&self) -> Option<&'a str> {
        self.level_name
    }

    /// The message, rendered by whoever writes it.
    pub const fn message(&self) -> &'a dyn fmt::Display {
        self.message
    }

    /// The Rust target the record was logged under - `yggdryl::fix::build` -
    /// or the logger's name for a record logged on a [`Logger`](super::Logger).
    pub const fn target(&self) -> &'a str {
        self.target
    }

    /// The Rust module path the record was raised in, when known.
    pub const fn module_path(&self) -> Option<&'a str> {
        self.module_path
    }

    /// The source file the record was raised in, when known.
    pub const fn file(&self) -> Option<&'a str> {
        self.file
    }

    /// The line of [`Self::file`] the record was raised at, when known.
    pub const fn line(&self) -> Option<u32> {
        self.line
    }

    /// The function or method the record was raised in, when stated.
    pub const fn function(&self) -> Option<&'a str> {
        self.function
    }

    /// The thread the record was raised on, when stated; a formatter reads
    /// the emitting thread's name otherwise.
    pub const fn thread(&self) -> Option<&'a str> {
        self.thread
    }

    /// When the record was created: nanoseconds since the Unix epoch, UTC.
    pub const fn created(&self) -> i64 {
        self.created
    }
}

impl fmt::Debug for Record<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Record")
            .field("name", &self.name)
            .field("level", &self.level)
            .field("message", &format_args!("{}", self.message))
            .field("target", &self.target)
            .field("file", &self.file)
            .field("line", &self.line)
            .field("function", &self.function)
            .field("thread", &self.thread)
            .field("created", &self.created)
            .finish_non_exhaustive()
    }
}

/// The wall clock, nanoseconds since the Unix epoch.
pub(crate) fn now() -> i64 {
    crate::holder::system_time_ns(std::time::SystemTime::now()).unwrap_or(0)
}
