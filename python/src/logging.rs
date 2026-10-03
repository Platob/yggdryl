//! The core's logging tree hosted by Python's `logging`: the same loggers,
//! the levels `logging` states, and a file handler writing through any
//! location.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyCFunction, PyDict, PyTuple};
use yggdryl::holder::Holder;
use yggdryl::logging::{
    self as core_logging, Counted, FileHandler, Formatter, Handler as _, Host, Level, Record,
    Repeat, Repeats,
};

use crate::iobase::{PyIOBase, located_holder};
use crate::uri::core_url_from_value;
use crate::value_error;

/// The most Python loggers the host keeps a reference to. A logger is a
/// Rust module path in practice, so the bound is never reached by code that
/// does not mint targets; past it a logger is looked up on each record.
const LOGGERS: usize = 4096;

/// Python's `logging` as the tree's host: each Rust logger is the Python
/// logger of the same dotted name.
struct PythonHost {
    get_logger: Py<PyAny>,
    manager: Py<PyAny>,
    logger_class: Py<PyAny>,
    root: Py<PyAny>,
    loggers: Mutex<HashMap<Box<str>, Py<PyAny>>>,
}

impl PythonHost {
    fn new(logging: &Bound<'_, PyModule>) -> PyResult<Self> {
        let logger_class = logging.getattr("Logger")?;
        Ok(Self {
            get_logger: logging.getattr("getLogger")?.unbind(),
            manager: logger_class.getattr("manager")?.unbind(),
            root: logging.getattr("root")?.unbind(),
            logger_class: logger_class.unbind(),
            loggers: Mutex::default(),
        })
    }

    /// The Python logger named `name`, kept once asked for.
    ///
    /// The cache's lock is never held while Python runs: `getLogger` may
    /// let the interpreter go, and a thread holding the interpreter must
    /// never wait on a lock a thread without it holds.
    fn logger<'py>(&self, py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyAny>> {
        if let Some(logger) = self
            .loggers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
        {
            return Ok(logger.bind(py).clone());
        }
        let logger = self.get_logger.bind(py).call1((name,))?;
        let mut loggers = self.loggers.lock().unwrap_or_else(PoisonError::into_inner);
        if loggers.len() < LOGGERS {
            loggers
                .entry(name.into())
                .or_insert_with(|| logger.clone().unbind());
        }
        Ok(logger)
    }

    /// The level Python's `logging.disable` drops records at and below.
    fn disabled(&self, py: Python<'_>) -> PyResult<i64> {
        self.manager.bind(py).getattr("disable")?.extract()
    }
}

/// The lowest level a logger effective at `effective` handles under
/// `logging.disable(disabled)`; `None` past the highest level the tree
/// counts to.
fn threshold(effective: i64, disabled: i64) -> Option<Level> {
    u8::try_from(effective.max(disabled.saturating_add(1)).max(0))
        .ok()
        .map(Level::new)
}

/// Says a Python failure met while reading `logging` the way Python says an
/// exception nothing can catch, rather than leaving it pending on a thread
/// that never looks.
fn unraisable(py: Python<'_>, error: PyErr) {
    // Ctrl-C in a handler is raised again in the main thread at its next
    // check, so it still stops the program the native record came from.
    if error.is_instance_of::<pyo3::exceptions::PyKeyboardInterrupt>(py)
        && py
            .import("_thread")
            .and_then(|thread| thread.call_method0("interrupt_main"))
            .is_ok()
    {
        return;
    }
    error.write_unraisable(py, None);
}

impl Host for PythonHost {
    fn threshold(&self, name: &str) -> Option<Level> {
        Python::try_attach(|py| {
            let read = || -> PyResult<Option<Level>> {
                let effective = self
                    .logger(py, name)?
                    .call_method0("getEffectiveLevel")?
                    .extract()?;
                Ok(threshold(effective, self.disabled(py)?))
            };
            read().unwrap_or_else(|error| {
                unraisable(py, error);
                None
            })
        })
        .flatten()
    }

    fn lowest_threshold(&self) -> Option<Level> {
        Python::try_attach(|py| {
            let read = || -> PyResult<Option<Level>> {
                // A logger's effective level is its own or an ancestor's,
                // the root's last: the lowest is among the stated ones.
                let mut lowest: i64 = self.root.bind(py).getattr("level")?.extract()?;
                let logger_class = self.logger_class.bind(py);
                let loggers = self.manager.bind(py).getattr("loggerDict")?;
                let loggers = loggers.cast::<PyDict>()?.values();
                for logger in loggers.iter() {
                    if !logger.is_instance(logger_class)? {
                        continue;
                    }
                    let level: i64 = logger.getattr("level")?.extract()?;
                    if level != 0 {
                        lowest = lowest.min(level);
                    }
                }
                Ok(threshold(lowest, self.disabled(py)?))
            };
            read().unwrap_or_else(|error| {
                unraisable(py, error);
                None
            })
        })
        .flatten()
    }

    fn handle(&self, record: &Record<'_>) {
        // Spelled before the interpreter is taken, so the message's own
        // rendering never runs while other Python threads wait.
        let message = record.message().to_string();
        Python::try_attach(|py| {
            let handled = || -> PyResult<()> {
                let logger = self.logger(py, record.name())?;
                let made = logger.call_method1(
                    "makeRecord",
                    (
                        record.name(),
                        record.level().get(),
                        record.file().unwrap_or("(unknown file)"),
                        record.line().unwrap_or(0),
                        message,
                        PyTuple::empty(py),
                        py.None(),
                        record.function().unwrap_or("(unknown function)"),
                    ),
                )?;
                logger.call_method1("handle", (made,))?;
                Ok(())
            };
            if let Err(error) = handled() {
                unraisable(py, error);
            }
        });
    }
}

/// Makes the core's tree the `log` facade's backend hosted by `logging`,
/// unless the process already chose another logger: records then travel
/// under their Rust module path - `yggdryl.iceberg.table` - at the levels
/// `logging` states, and a dependency of the build passes only at
/// `WARNING` and above.
///
/// `logging` drops its own level cache through `Manager._clear_cache`
/// whenever `setLevel` or `disable` runs; the tree's cache is dropped by
/// the same call, so a level changed at any time applies to the next
/// record.
pub(crate) fn install(py: Python<'_>) -> PyResult<()> {
    if core_logging::install().is_err() {
        // An embedder installed a logger first, and an extension has no
        // business replacing it: the core's records go to that logger.
        return Ok(());
    }
    core_logging::set_foreign_level(Level::WARNING);
    let logging = py.import("logging")?;
    let host = PythonHost::new(&logging)?;
    let manager = host.manager.bind(py).clone();
    let clear_cache = manager.getattr("_clear_cache")?.unbind();
    let hook = PyCFunction::new_closure(
        py,
        Some(c"_clear_cache"),
        Some(c"Drops logging's level cache and the yggdryl tree's with it."),
        move |arguments: &Bound<'_, PyTuple>, keywords: Option<&Bound<'_, PyDict>>| {
            let cleared = clear_cache.bind(arguments.py()).call(arguments, keywords);
            core_logging::invalidate();
            cleared.map(Bound::unbind)
        },
    )?;
    manager.setattr("_clear_cache", hook)?;
    core_logging::set_host(Some(Arc::new(host)));
    Ok(())
}

/// A record's own level, which `logging` lets be any number: past the
/// tree's 255 it reads as 255, below 0 as 0, so no record is refused for its
/// level.
fn record_level(levelno: i64) -> Level {
    Level::new(u8::try_from(levelno.clamp(0, 255)).unwrap_or(u8::MAX))
}

/// A level given as a number or as a name.
fn level_from_value(value: &Bound<'_, PyAny>) -> PyResult<Level> {
    if let Ok(number) = value.extract::<i64>() {
        return u8::try_from(number).map(Level::new).map_err(|_| {
            PyValueError::new_err(format!("expected a level from 0 to 255, got {number}"))
        });
    }
    Level::from_str(&value.extract::<String>()?).map_err(value_error)
}

/// The native half of `yggdryl.logging.FileHandler`: records spelled by
/// Python's formatter, held and published through a location's handle.
#[pyclass(name = "LogFile", module = "yggdryl._native", frozen)]
pub(crate) struct PyLogFile {
    inner: FileHandler<Holder>,
    url: Option<String>,
}

#[pymethods]
impl PyLogFile {
    #[new]
    #[pyo3(signature = (location, mode = "append", capacity = 0, flush_level = None))]
    fn new(
        location: &Bound<'_, PyAny>,
        mode: &str,
        capacity: usize,
        flush_level: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        // Every argument is read before the handle is taken, so a refusal
        // leaves the caller's handle as it was.
        let mode = yggdryl::IOMode::from_str(mode).map_err(value_error)?;
        // The handler's own refusal of a mode, asked of one over nothing.
        FileHandler::new(yggdryl::holder::Buffer::new())
            .with_mode(mode)
            .map_err(value_error)?;
        let flush_level = flush_level.map(level_from_value).transpose()?;
        // A handle is taken whole - its store options, coding and media
        // type kept - and the object answers nothing more.
        let holder = match location.extract::<PyRefMut<'_, PyIOBase>>() {
            Ok(mut handle) => handle.take()?,
            Err(_) => located_holder(&core_url_from_value(location)?)?,
        };
        let url = yggdryl::IOBase::url(&holder).map(ToString::to_string);
        let mut inner = FileHandler::new(holder)
            .with_mode(mode)
            .map_err(value_error)?
            .with_capacity(capacity);
        if let Some(level) = flush_level {
            inner = inner.with_flush_level(level);
        }
        Ok(Self { inner, url })
    }

    /// Holds one spelled record logged at `level`, publishing when the
    /// capacity or the flush level says so.
    fn write(&self, py: Python<'_>, line: &str, level: i64) -> PyResult<()> {
        let level = record_level(level);
        py.detach(|| self.inner.write(line, level))
            .map_err(crate::holder::fs::storage_error)
    }

    /// Publishes what is held.
    fn flush(&self, py: Python<'_>) -> PyResult<()> {
        py.detach(|| self.inner.flush())
            .map_err(crate::holder::fs::storage_error)
    }

    /// Publishes what is held and lets go of the handle; a later record
    /// reopens it.
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        py.detach(|| self.inner.close())
            .map_err(crate::holder::fs::storage_error)
    }

    /// The location written to, `None` for one with no URL.
    #[getter]
    fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    /// `append` or `overwrite`.
    #[getter]
    fn mode(&self) -> &'static str {
        self.inner.mode().as_str()
    }

    /// The bytes held back before a publish.
    #[getter]
    fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    /// The level that publishes what is held at once.
    #[getter]
    fn flush_level(&self) -> u8 {
        self.inner.flush_level().get()
    }

    fn __repr__(&self) -> String {
        format!(
            "LogFile({}, mode={:?}, capacity={})",
            self.url.as_deref().unwrap_or("<memory>"),
            self.mode(),
            self.capacity()
        )
    }
}

/// States the core logger `name`'s deduplication: `True` counts each record
/// logged there and below and drops repeats before they reach `logging`,
/// `False` passes every record, `None` takes the ancestors' again.
#[pyfunction]
#[pyo3(signature = (name, enabled = Some(true)))]
pub(crate) fn log_deduplicate(name: &str, enabled: Option<bool>) {
    core_logging::get_logger(name).set_deduplicating(enabled);
}

/// Whether the core logger `name` deduplicates, as stated or inherited.
#[pyfunction]
pub(crate) fn log_is_deduplicating(name: &str) -> bool {
    core_logging::get_logger(name).is_deduplicating()
}

/// The native half of `yggdryl.logging.Deduplicate`: one table of repeated
/// records, counted by the core's hash of the logger, the level and the
/// message.
#[pyclass(name = "LogRepeats", module = "yggdryl._native", frozen)]
pub(crate) struct PyLogRepeats {
    inner: Repeats,
}

#[pymethods]
impl PyLogRepeats {
    #[new]
    fn new() -> Self {
        Self {
            inner: Repeats::new(),
        }
    }

    /// What to say of one occurrence of `message` at `level` on `name`:
    /// `None` for a repeat said nothing of, the message itself the first
    /// time or when the table is full, `message (seen N times)` at a
    /// tenfold count.
    fn said(&self, name: &str, level: i64, message: &str) -> Option<String> {
        let level = record_level(level);
        let record = Record::new(name, level, &message);
        match self.inner.count_record(&record) {
            Repeat::First | Repeat::Untracked => Some(message.to_owned()),
            Repeat::Tenfold(count) => Some(Counted::new(count, &message).to_string()),
            Repeat::Repeated => None,
        }
    }

    /// How many distinct records are counted.
    fn __len__(&self) -> usize {
        self.inner.len()
    }
}

/// One record as the core's terminal format spells it - the timestamp,
/// the level's glyph and name, the thread, the logger, the call site, the
/// message - with its colours when `colored`.
#[pyfunction]
#[pyo3(signature = (name, level, levelname, message, created, pathname = None, lineno = None, function = None, thread = None, colored = false))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn log_terminal(
    name: &str,
    level: i64,
    levelname: Option<&str>,
    message: &str,
    created: i64,
    pathname: Option<&str>,
    lineno: Option<u32>,
    function: Option<&str>,
    thread: Option<&str>,
    colored: bool,
) -> String {
    let mut record = Record::new(name, record_level(level), &message).with_created(created);
    if let Some(levelname) = levelname {
        record = record.with_level_name(levelname);
    }
    if let Some(pathname) = pathname {
        record = record.with_location(pathname, lineno.unwrap_or(0));
    }
    if let Some(function) = function {
        record = record.with_function(function);
    }
    if let Some(thread) = thread {
        record = record.with_thread(thread);
    }
    let mut line = String::new();
    if colored {
        Formatter::terminal().format_colored_into(&record, &mut line);
    } else {
        Formatter::terminal().format_into(&record, &mut line);
    }
    line
}

/// Whether a stream answering `is_terminal` is written in colour, by the
/// core's rule: `NO_COLOR`, `FORCE_COLOR`, `CLICOLOR_FORCE`, `TERM=dumb`.
#[pyfunction]
pub(crate) fn log_is_color_enabled(is_terminal: bool) -> bool {
    core_logging::is_color_enabled(is_terminal)
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyLogFile>()?;
    module.add_class::<PyLogRepeats>()?;
    module.add_function(wrap_pyfunction!(log_deduplicate, module)?)?;
    module.add_function(wrap_pyfunction!(log_is_deduplicating, module)?)?;
    module.add_function(wrap_pyfunction!(log_terminal, module)?)?;
    module.add_function(wrap_pyfunction!(log_is_color_enabled, module)?)
}
