//! The core's logging tree, Python's `logging` spelled in camelCase: named
//! loggers, numeric levels, handlers writing to standard error or through
//! any location, and `%`-style formatters.

use std::sync::Arc;

use napi::bindgen_prelude::{ClassInstance, Either, Either3, Result};
use napi_derive::napi;
use yggdryl::Timezone;
use yggdryl::holder::Holder;
use yggdryl::logging::{
    self as core_logging, BasicConfig, FileHandler, Formatter, Handler, Level, Logger, NullHandler,
    Record, StreamHandler,
};

use crate::iobase::{LocationInput, located_from_input};
use crate::{exact_u8, napi_error};

/// A level given as its number or its name.
type LoggingLevelInput = Either<f64, String>;

/// `level` read as the argument `name` - `level`, `flushLevel` - so a
/// refusal names what was wrong.
fn level_from_input(level: LoggingLevelInput, name: &str) -> Result<Level> {
    match level {
        Either::A(number) => exact_u8(number, name)
            .map(Level::new)
            .map_err(|error| napi_error(format!("{}, got {number}", error.reason))),
        Either::B(text) => {
            Level::from_str(&text).map_err(|error| napi_error(format!("{name}: {error}")))
        }
    }
}

/// Any of the three handler classes, where a handler is taken.
type LoggingHandlerInput<'a> = Either3<
    ClassInstance<'a, JsStreamHandler>,
    ClassInstance<'a, JsFileHandler>,
    ClassInstance<'a, JsNullHandler>,
>;

fn handler_from_input(handler: &LoggingHandlerInput<'_>) -> Arc<dyn Handler> {
    match handler {
        Either3::A(stream) => Arc::clone(&stream.inner) as Arc<dyn Handler>,
        Either3::B(file) => Arc::clone(&file.inner) as Arc<dyn Handler>,
        Either3::C(null) => Arc::clone(&null.inner) as Arc<dyn Handler>,
    }
}

/// A `%`-style format and the date format `asctime` takes.
#[napi(js_name = "Formatter")]
#[derive(Clone)]
pub struct JsFormatter {
    pub(crate) inner: Formatter,
}

/// What a `Formatter` takes beside its format.
#[napi(object)]
pub struct FormatterOptions {
    /// The `strftime` date format `asctime` is spelled in.
    pub datefmt: Option<String>,
    /// The zone instants are rendered in, UTC when absent.
    pub timezone: Option<String>,
}

fn formatter_from(
    format: Option<String>,
    datefmt: Option<String>,
    timezone: Option<String>,
) -> Result<Formatter> {
    let mut formatter = match format {
        Some(format) => Formatter::from_str(&format).map_err(napi_error)?,
        None => Formatter::default(),
    };
    if let Some(datefmt) = datefmt {
        formatter = formatter.with_datefmt(&datefmt).map_err(napi_error)?;
    }
    if let Some(timezone) = timezone {
        formatter = formatter.with_timezone(Timezone::from_str(&timezone).map_err(napi_error)?);
    }
    Ok(formatter)
}

#[napi]
impl JsFormatter {
    /// A formatter over `format` - `%(message)s` when absent.
    #[napi(constructor)]
    pub fn new(format: Option<String>, options: Option<FormatterOptions>) -> Result<Self> {
        let (datefmt, timezone) =
            options.map_or((None, None), |options| (options.datefmt, options.timezone));
        formatter_from(format, datefmt, timezone).map(|inner| Self { inner })
    }

    /// The prebuilt terminal formatter: the timestamp, the level's glyph and
    /// name in its colour, the thread, the logger, the call site and the
    /// message - the default of `basicConfig` and of the last resort.
    #[napi(factory)]
    pub fn terminal() -> Self {
        Self {
            inner: Formatter::terminal(),
        }
    }

    /// The format, as it was spelled.
    #[napi(getter)]
    pub fn format(&self) -> String {
        self.inner.as_str().to_owned()
    }

    /// The date format, `null` for the default.
    #[napi(getter)]
    pub fn datefmt(&self) -> Option<String> {
        self.inner.datefmt().map(str::to_owned)
    }

    /// The zone instants are rendered in.
    #[napi(getter)]
    pub fn timezone(&self) -> String {
        self.inner.timezone().as_str().to_owned()
    }

    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }
}

/// Writes each record as one line to standard error or standard output.
#[napi(js_name = "StreamHandler")]
pub struct JsStreamHandler {
    inner: Arc<StreamHandler>,
}

#[napi]
impl JsStreamHandler {
    /// A handler on `stream`: `stderr`, the default, or `stdout`.
    #[napi(constructor)]
    pub fn new(stream: Option<String>) -> Result<Self> {
        let handler = match stream.as_deref() {
            None | Some("stderr") => StreamHandler::stderr(),
            Some("stdout") => StreamHandler::stdout(),
            Some(other) => {
                return Err(napi_error(format!(
                    "expected the stream stderr or stdout, got {other:?}"
                )));
            }
        };
        Ok(Self {
            inner: Arc::new(handler),
        })
    }

    /// The level the handler emits from.
    #[napi(getter)]
    pub fn level(&self) -> u8 {
        self.inner.level().get()
    }

    /// Emits records at `level` and above.
    #[napi]
    pub fn set_level(&self, level: LoggingLevelInput) -> Result<()> {
        self.inner.set_level(level_from_input(level, "level")?);
        Ok(())
    }

    /// The formatter records are spelled with.
    #[napi(getter)]
    pub fn formatter(&self) -> JsFormatter {
        JsFormatter {
            inner: self.inner.formatter(),
        }
    }

    /// Spells records with `formatter`.
    #[napi]
    pub fn set_formatter(&self, formatter: &JsFormatter) {
        self.inner.set_formatter(formatter.inner.clone());
    }

    /// Whether the formatter's styles are spelled: decided from the stream
    /// when the handler was built - a colour terminal, `NO_COLOR`,
    /// `FORCE_COLOR`, `CLICOLOR_FORCE`, `TERM=dumb` - until set.
    #[napi(getter)]
    pub fn colored(&self) -> bool {
        self.inner.is_colored()
    }

    #[napi(setter)]
    pub fn set_colored(&mut self, colored: bool) {
        self.inner.set_colored(colored);
    }

    /// Flushes the stream.
    #[napi]
    pub fn flush(&self) -> Result<()> {
        self.inner.flush().map_err(napi_error)
    }

    /// Flushes the stream; a stream is never closed.
    #[napi]
    pub fn close(&self) -> Result<()> {
        self.inner.close().map_err(napi_error)
    }
}

/// What a `FileHandler` takes beside its location.
#[napi(object)]
pub struct FileHandlerOptions {
    /// `append`, the default, or `overwrite`.
    pub mode: Option<String>,
    /// The bytes held back before a publish; `0`, the default, publishes
    /// each record as it arrives.
    pub capacity: Option<f64>,
    /// The level that publishes what is held at once; `ERROR` by default.
    pub flush_level: Option<LoggingLevelInput>,
    /// The level the handler emits from; `NOTSET` by default.
    pub level: Option<LoggingLevelInput>,
}

/// Writes each record as one line through a location's handle, one append
/// per publish.
#[napi(js_name = "FileHandler")]
pub struct JsFileHandler {
    inner: Arc<FileHandler<Holder>>,
    url: Option<String>,
}

#[napi]
impl JsFileHandler {
    /// A handler writing to `location` - a path, a `Url` or an `IOBase`.
    #[napi(constructor)]
    pub fn new(location: LocationInput<'_>, options: Option<FileHandlerOptions>) -> Result<Self> {
        let holder = located_from_input(location)?;
        let on_javascript = yggdryl::IOBase::bound_location(&holder).is_some_and(|bound| {
            bound
                .filesystem()
                .as_any()
                .downcast_ref::<crate::holder::fs::JsFileSystem>()
                .is_some()
        });
        if on_javascript {
            return Err(napi_error(
                "a FileHandler writes from any thread, and a JavaScript file system answers on its own isolate thread only",
            ));
        }
        let url = yggdryl::IOBase::url(&holder).map(ToString::to_string);
        let mut handler = FileHandler::new(holder);
        let mut level = None;
        if let Some(options) = options {
            if let Some(mode) = options.mode {
                let mode = yggdryl::IOMode::from_str(&mode).map_err(napi_error)?;
                handler = handler.with_mode(mode).map_err(napi_error)?;
            }
            if let Some(capacity) = options.capacity {
                let capacity = crate::exact_u32(capacity, "capacity")?;
                handler = handler.with_capacity(capacity as usize);
            }
            if let Some(flush_level) = options.flush_level {
                handler = handler.with_flush_level(level_from_input(flush_level, "flushLevel")?);
            }
            if let Some(stated) = options.level {
                level = Some(level_from_input(stated, "level")?);
            }
        }
        if let Some(level) = level {
            handler.set_level(level);
        }
        Ok(Self {
            inner: Arc::new(handler),
            url,
        })
    }

    /// The location written to, `null` for one with no URL.
    #[napi(getter)]
    pub fn url(&self) -> Option<String> {
        self.url.clone()
    }

    /// `append` or `overwrite`.
    #[napi(getter)]
    pub fn mode(&self) -> &'static str {
        self.inner.mode().as_str()
    }

    /// The bytes held back before a publish.
    #[napi(getter)]
    pub fn capacity(&self) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let capacity = self.inner.capacity() as f64;
        capacity
    }

    /// The level that publishes what is held at once.
    #[napi(getter)]
    pub fn flush_level(&self) -> u8 {
        self.inner.flush_level().get()
    }

    /// The level the handler emits from.
    #[napi(getter)]
    pub fn level(&self) -> u8 {
        self.inner.level().get()
    }

    /// Emits records at `level` and above.
    #[napi]
    pub fn set_level(&self, level: LoggingLevelInput) -> Result<()> {
        self.inner.set_level(level_from_input(level, "level")?);
        Ok(())
    }

    /// The formatter records are spelled with.
    #[napi(getter)]
    pub fn formatter(&self) -> JsFormatter {
        JsFormatter {
            inner: self.inner.formatter(),
        }
    }

    /// Spells records with `formatter`.
    #[napi]
    pub fn set_formatter(&self, formatter: &JsFormatter) {
        self.inner.set_formatter(formatter.inner.clone());
    }

    /// Publishes what is held.
    #[napi]
    pub fn flush(&self) -> Result<()> {
        self.inner.flush().map_err(napi_error)
    }

    /// Publishes what is held and lets go of the handle; a later record
    /// reopens it.
    #[napi]
    pub fn close(&self) -> Result<()> {
        self.inner.close().map_err(napi_error)
    }
}

/// Takes every record and writes none.
#[napi(js_name = "NullHandler")]
pub struct JsNullHandler {
    inner: Arc<NullHandler>,
}

#[napi]
impl JsNullHandler {
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(NullHandler::new()),
        }
    }

    /// The level the handler takes records from.
    #[napi(getter)]
    pub fn level(&self) -> u8 {
        self.inner.level().get()
    }

    /// Takes records at `level` and above.
    #[napi]
    pub fn set_level(&self, level: LoggingLevelInput) -> Result<()> {
        self.inner.set_level(level_from_input(level, "level")?);
        Ok(())
    }
}

impl Default for JsNullHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// A named logger of the process's tree; `logging.getLogger` answers one.
#[napi(js_name = "Logger")]
pub struct JsLogger {
    inner: Logger,
}

#[napi]
impl JsLogger {
    /// The logger's dotted name; `root` for the root.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_owned()
    }

    /// The logger this one hangs from; `null` for the root.
    #[napi(getter)]
    pub fn parent(&self) -> Option<JsLogger> {
        self.inner.parent().map(|inner| JsLogger { inner })
    }

    /// The logger named `suffix` below this one.
    #[napi]
    pub fn get_child(&self, suffix: String) -> Self {
        Self {
            inner: self.inner.child(&suffix),
        }
    }

    /// The level stated on this logger; `0` when it takes its ancestors'.
    #[napi(getter)]
    pub fn level(&self) -> u8 {
        self.inner.level().get()
    }

    /// States `level` on this logger; `NOTSET` takes the ancestors' again.
    #[napi]
    pub fn set_level(&self, level: LoggingLevelInput) -> Result<()> {
        self.inner.set_level(level_from_input(level, "level")?);
        Ok(())
    }

    /// The level this logger handles from.
    #[napi]
    pub fn get_effective_level(&self) -> u8 {
        self.inner.effective_level().get()
    }

    /// Whether a record at `level` would be handled.
    #[napi]
    pub fn is_enabled_for(&self, level: LoggingLevelInput) -> Result<bool> {
        Ok(self.inner.is_enabled_for(level_from_input(level, "level")?))
    }

    /// Whether records go on to the ancestors' handlers.
    #[napi(getter)]
    pub fn propagate(&self) -> bool {
        self.inner.is_propagating()
    }

    #[napi(setter)]
    pub fn set_propagate(&mut self, propagate: bool) {
        self.inner.set_propagating(propagate);
    }

    /// Whether the logger drops every record logged on it.
    #[napi(getter)]
    pub fn disabled(&self) -> bool {
        self.inner.is_disabled()
    }

    #[napi(setter)]
    pub fn set_disabled(&mut self, disabled: bool) {
        self.inner.set_disabled(disabled);
    }

    /// Whether this logger states its own deduplication: `true` drops
    /// repeated records, `false` passes every record, `null` takes the
    /// nearest ancestor's.
    #[napi(getter)]
    pub fn deduplicating(&self) -> Option<bool> {
        self.inner.deduplicating()
    }

    #[napi(setter)]
    pub fn set_deduplicating(&mut self, deduplicating: Option<bool>) {
        self.inner.set_deduplicating(deduplicating);
    }

    /// Whether records logged here are deduplicated, as stated here or by
    /// the nearest ancestor: the first occurrence is said, the 10th, 100th,
    /// 1000th as `message (seen N times)`, every other reaches no handler.
    #[napi]
    pub fn is_deduplicating(&self) -> bool {
        self.inner.is_deduplicating()
    }

    /// Attaches `handler`, once.
    #[napi]
    pub fn add_handler(&self, handler: LoggingHandlerInput<'_>) {
        self.inner.add_handler(handler_from_input(&handler));
    }

    /// Detaches `handler`; answers whether it was attached.
    #[napi]
    pub fn remove_handler(&self, handler: LoggingHandlerInput<'_>) -> bool {
        self.inner.remove_handler(&handler_from_input(&handler))
    }

    /// Whether this logger or an ancestor its records reach has a handler.
    #[napi]
    pub fn has_handlers(&self) -> bool {
        self.inner.has_handlers()
    }

    /// Logs `message` at `level` when the level is enabled.
    #[napi]
    pub fn log(&self, level: LoggingLevelInput, message: String) -> Result<()> {
        self.logged(level_from_input(level, "level")?, &message);
        Ok(())
    }

    /// Logs `message` at `DEBUG`.
    #[napi]
    pub fn debug(&self, message: String) {
        self.logged(Level::DEBUG, &message);
    }

    /// Logs `message` at `INFO`.
    #[napi]
    pub fn info(&self, message: String) {
        self.logged(Level::INFO, &message);
    }

    /// Logs `message` at `WARNING`.
    #[napi]
    pub fn warning(&self, message: String) {
        self.logged(Level::WARNING, &message);
    }

    /// Logs `message` at `ERROR`.
    #[napi]
    pub fn error(&self, message: String) {
        self.logged(Level::ERROR, &message);
    }

    /// Logs `message` at `CRITICAL`.
    #[napi]
    pub fn critical(&self, message: String) {
        self.logged(Level::CRITICAL, &message);
    }

    /// Hands over a record at `level` located at the JavaScript call site the
    /// binding read, when the level is enabled.
    #[napi(js_name = "_record", skip_typescript)]
    pub fn record_at(
        &self,
        level: LoggingLevelInput,
        message: String,
        function: Option<String>,
        file: Option<String>,
        line: Option<u32>,
    ) -> Result<()> {
        let level = level_from_input(level, "level")?;
        if !self.inner.is_enabled_for(level) {
            return Ok(());
        }
        let mut record = Record::new(self.inner.name(), level, &message);
        if let Some(file) = file.as_deref() {
            record = record.with_location(file, line.unwrap_or(0));
        }
        if let Some(function) = function.as_deref() {
            record = record.with_function(function);
        }
        self.inner.handle(&record);
        Ok(())
    }

    /// Whether `other` is this very logger.
    #[napi]
    pub fn equals(&self, other: &JsLogger) -> bool {
        self.inner == other.inner
    }

    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }
}

impl JsLogger {
    /// Python's `Logger.log`: the level asked, then a record handed over -
    /// located nowhere, since the JavaScript caller is not a Rust line.
    fn logged(&self, level: Level, message: &str) {
        if self.inner.is_enabled_for(level) {
            self.inner
                .handle(&Record::new(self.inner.name(), level, &message));
        }
    }
}

/// The logger named `name`, made the first time; the root when absent.
#[napi(js_name = "_loggingGetLogger", skip_typescript)]
#[allow(dead_code)] // Reached through NAPI's registration inventory.
pub fn logging_get_logger(name: Option<String>) -> JsLogger {
    JsLogger {
        inner: core_logging::get_logger(name.as_deref().unwrap_or_default()),
    }
}

/// Configures the root: Python's `basicConfig`.
#[napi(js_name = "_loggingBasicConfig", skip_typescript)]
#[allow(dead_code)] // Reached through NAPI's registration inventory.
pub fn logging_basic_config(
    level: Option<LoggingLevelInput>,
    format: Option<String>,
    datefmt: Option<String>,
    handlers: Option<Vec<LoggingHandlerInput<'_>>>,
    force: Option<bool>,
) -> Result<()> {
    let mut config = BasicConfig::new().with_force(force.unwrap_or(false));
    if let Some(level) = level {
        config = config.with_level(level_from_input(level, "level")?);
    }
    match (format, datefmt) {
        (Some(format), datefmt) => {
            config = config.with_formatter(formatter_from(Some(format), datefmt, None)?);
        }
        (None, Some(datefmt)) => {
            let formatter = Formatter::terminal()
                .with_datefmt(&datefmt)
                .map_err(napi_error)?;
            config = config.with_formatter(formatter);
        }
        (None, None) => {}
    }
    for handler in handlers.iter().flatten() {
        config = config.with_handler(handler_from_input(handler));
    }
    core_logging::basic_config(config).map_err(napi_error)
}

/// Drops every record at or below `level`; Python's `logging.disable`.
#[napi(js_name = "_loggingDisable", skip_typescript)]
#[allow(dead_code)] // Reached through NAPI's registration inventory.
pub fn logging_disable(level: Option<LoggingLevelInput>) -> Result<()> {
    let level = match level {
        Some(level) => level_from_input(level, "level")?,
        None => Level::CRITICAL,
    };
    core_logging::disable(level);
    Ok(())
}

/// Publishes and closes every handler of the tree.
#[napi(js_name = "_loggingShutdown", skip_typescript)]
#[allow(dead_code)] // Reached through NAPI's registration inventory.
pub fn logging_shutdown() {
    core_logging::shutdown();
}

/// Installs the tree as the process's logger when the addon loads, unless
/// the process installed one first: a dependency of the build passes at
/// `WARNING` and above, and a record no handler takes is written to
/// standard error by the core's last resort - the terminal format, from
/// `WARNING` up.
pub(crate) fn install() {
    if core_logging::install().is_ok() {
        core_logging::set_foreign_level(Level::WARNING);
    }
}

/// Names the calling isolate's thread in every record logged from it: the
/// binding calls it once per isolate, `main` or `worker-N`.
#[napi(js_name = "_nameThread", skip_typescript)]
#[allow(dead_code)] // Reached through NAPI's registration inventory.
pub fn name_thread(name: String) {
    core_logging::set_thread_name(&name);
}

/// Every named level and its number, from the core's one table.
#[napi(js_name = "_loggingLevels", skip_typescript)]
#[allow(dead_code)] // Reached through NAPI's registration inventory.
pub fn logging_levels() -> std::collections::HashMap<String, u32> {
    Level::ALL
        .iter()
        .filter_map(|level| Some((level.name()?.to_owned(), u32::from(level.get()))))
        .collect()
}
