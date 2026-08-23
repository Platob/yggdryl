//! Direct conversion between Python objects and the native byte codecs.
//!
//! Everything here is plumbing: bytes in, bytes out, and the stream adapters
//! that let the core read from and write to a caller-owned Python file object.
//! The value conversion itself belongs to [`crate::scalar`], so a document is
//! read and written through exactly one pair of functions.

use std::cell::RefCell;
use std::io::{self, BufWriter, Read, Write};
use std::path::Path;
use std::rc::Rc;

use pyo3::exceptions::{PyOSError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyByteArray, PyBytes, PyIterator, PyMemoryView, PyString};
use yggdryl::Field as CoreField;
use yggdryl::text::{Format, Formatting, Indent, Limits, Scalar};

use crate::field::core_field_from_value;
use crate::scalar::{PyScalar, as_py, as_py_with_field, from_py};
use crate::value_error;

/// How many documents one multi-document call may encode.
const MAX_PYTHON_DOCUMENTS: usize = 1_024;

/// Read bytes out of anything Python uses to hold them.
fn with_python_bytes<T>(
    value: &Bound<'_, PyAny>,
    operation: impl FnOnce(&[u8]) -> PyResult<T>,
) -> PyResult<T> {
    if let Ok(value) = value.cast::<PyBytes>() {
        return operation(value.as_bytes());
    }
    if let Ok(value) = value.cast::<PyByteArray>() {
        let value = value.to_vec();
        return operation(&value);
    }
    if let Ok(value) = value.cast::<PyMemoryView>() {
        let value = value.call_method0("tobytes")?.cast_into::<PyBytes>()?;
        return operation(value.as_bytes());
    }
    Err(PyTypeError::new_err(
        "codec content must be bytes, bytearray, or memoryview",
    ))
}

/// Parse one public codec alias through the core `Format` parser.
fn format_from_str(value: &str) -> PyResult<Format> {
    Format::from_str(value).map_err(value_error)
}

/// Build one core formatting value from the private boundary code.
///
/// `-2` is omitted/default, `-1` is explicitly unindented, `-3` is tabs,
/// and non-negative values are spaces per nesting level.
fn formatting_from_code(indent: i16) -> PyResult<Formatting> {
    let indent = match indent {
        -2 => Indent::Default,
        -1 => Indent::None,
        -3 => Indent::Tabs,
        0..=255 => Indent::Spaces(
            u8::try_from(indent).map_err(|error| PyValueError::new_err(error.to_string()))?,
        ),
        _ => {
            return Err(PyValueError::new_err(
                "indent must be None, a non-negative integer up to 255, or '\\t'",
            ));
        }
    };
    Ok(Formatting::default().with_indent(indent))
}

/// Resolve nullable Python parser bounds against the core defaults.
fn limits_from(
    max_depth: Option<usize>,
    max_input_bytes: Option<usize>,
    max_nodes: Option<usize>,
    max_documents: Option<usize>,
) -> Limits {
    let defaults = Limits::default();
    Limits::new(
        max_depth.unwrap_or(defaults.max_depth()),
        max_input_bytes.unwrap_or(defaults.max_input_bytes()),
        max_nodes.unwrap_or(defaults.max_nodes()),
        max_documents.unwrap_or(defaults.max_documents()),
    )
}

/// Adapt a caller-owned Python text or binary stream to Rust's byte reader.
///
/// Python text readers count characters rather than bytes, so one bounded
/// read can produce more UTF-8 bytes than the core requested. The excess is
/// retained for the next `Read` call instead of allocating the whole input.
struct PythonReader<'py> {
    source: Bound<'py, PyAny>,
    pending: Vec<u8>,
    pending_offset: usize,
    error: Option<PyErr>,
}

impl<'py> PythonReader<'py> {
    fn new(source: &Bound<'py, PyAny>) -> Self {
        Self {
            source: source.clone(),
            pending: Vec::new(),
            pending_offset: 0,
            error: None,
        }
    }

    fn copy_chunk(&mut self, chunk: &[u8], output: &mut [u8]) -> usize {
        let count = output.len().min(chunk.len());
        output[..count].copy_from_slice(&chunk[..count]);
        if count < chunk.len() {
            self.pending.clear();
            self.pending.extend_from_slice(&chunk[count..]);
            self.pending_offset = 0;
        }
        count
    }

    fn fail(&mut self, error: PyErr) -> io::Error {
        self.error = Some(error);
        io::Error::other("Python stream read failed")
    }

    fn take_error(&mut self) -> Option<PyErr> {
        self.error.take()
    }
}

impl Read for PythonReader<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.error.is_some() {
            return Err(io::Error::other("Python stream read already failed"));
        }
        if output.is_empty() {
            return Ok(0);
        }
        if self.pending_offset < self.pending.len() {
            let available = &self.pending[self.pending_offset..];
            let count = output.len().min(available.len());
            output[..count].copy_from_slice(&available[..count]);
            self.pending_offset += count;
            if self.pending_offset == self.pending.len() {
                self.pending.clear();
                self.pending_offset = 0;
            }
            return Ok(count);
        }

        let value = match self.source.call_method1("read", (output.len(),)) {
            Ok(value) => value,
            Err(error) => return Err(self.fail(error)),
        };
        if let Ok(value) = value.cast::<PyBytes>() {
            if value.as_bytes().len() > output.len() {
                return Err(self.fail(PyOSError::new_err(
                    "binary file read() returned more data than requested",
                )));
            }
            return Ok(self.copy_chunk(value.as_bytes(), output));
        }
        if let Ok(value) = value.cast::<PyString>() {
            let value = match value.to_str() {
                Ok(value) => value,
                Err(error) => return Err(self.fail(error)),
            };
            if value.chars().count() > output.len() {
                return Err(self.fail(PyOSError::new_err(
                    "text file read() returned more characters than requested",
                )));
            }
            return Ok(self.copy_chunk(value.as_bytes(), output));
        }
        if let Ok(value) = value.cast::<PyByteArray>() {
            let value = value.to_vec();
            if value.len() > output.len() {
                return Err(self.fail(PyOSError::new_err(
                    "binary file read() returned more data than requested",
                )));
            }
            return Ok(self.copy_chunk(&value, output));
        }
        if let Ok(value) = value.cast::<PyMemoryView>() {
            let value = match value.call_method0("tobytes") {
                Ok(value) => value,
                Err(error) => return Err(self.fail(error)),
            };
            let value = match value.cast::<PyBytes>() {
                Ok(value) => value,
                Err(error) => return Err(self.fail(error.into())),
            };
            if value.as_bytes().len() > output.len() {
                return Err(self.fail(PyOSError::new_err(
                    "binary file read() returned more data than requested",
                )));
            }
            return Ok(self.copy_chunk(value.as_bytes(), output));
        }
        Err(self.fail(PyTypeError::new_err(
            "file read() must return str or bytes-like data",
        )))
    }
}

/// The owned reader used by a Python iterator that outlives one extension call.
///
/// The core parsers own this adapter, so JSON Lines and YAML framing remains in
/// Rust while Python contributes only its native `read`/`readline` protocol.
/// A saved Python exception crosses back unchanged on the pull that observed
/// it; parser errors continue through the core's byte-position diagnostics.
struct OwnedPythonReader {
    source: Py<PyAny>,
    method: &'static str,
    pending: Vec<u8>,
    pending_offset: usize,
    error: Rc<RefCell<Option<PyErr>>>,
}

enum OwnedPythonChunk {
    Binary(Vec<u8>),
    Text { bytes: Vec<u8>, characters: usize },
}

impl OwnedPythonReader {
    fn new(source: Py<PyAny>, method: &'static str, error: Rc<RefCell<Option<PyErr>>>) -> Self {
        Self {
            source,
            method,
            pending: Vec::new(),
            pending_offset: 0,
            error,
        }
    }

    fn fail(&self, error: PyErr) -> io::Error {
        *self.error.borrow_mut() = Some(error);
        io::Error::other("Python stream read failed")
    }

    fn copy_chunk(&mut self, chunk: &[u8], output: &mut [u8]) -> usize {
        let count = output.len().min(chunk.len());
        output[..count].copy_from_slice(&chunk[..count]);
        if count < chunk.len() {
            self.pending.clear();
            self.pending.extend_from_slice(&chunk[count..]);
            self.pending_offset = 0;
        }
        count
    }

    fn read_python_chunk(&self, requested: usize) -> PyResult<OwnedPythonChunk> {
        Python::attach(|py| {
            let value = self
                .source
                .bind(py)
                .call_method1(self.method, (requested,))?;
            if let Ok(value) = value.cast::<PyBytes>() {
                return Ok(OwnedPythonChunk::Binary(value.as_bytes().to_vec()));
            }
            if let Ok(value) = value.cast::<PyString>() {
                let value = value.to_str()?;
                return Ok(OwnedPythonChunk::Text {
                    bytes: value.as_bytes().to_vec(),
                    characters: value.chars().count(),
                });
            }
            if let Ok(value) = value.cast::<PyByteArray>() {
                return Ok(OwnedPythonChunk::Binary(value.to_vec()));
            }
            if let Ok(value) = value.cast::<PyMemoryView>() {
                let value = value.call_method0("tobytes")?.cast_into::<PyBytes>()?;
                return Ok(OwnedPythonChunk::Binary(value.as_bytes().to_vec()));
            }
            Err(PyTypeError::new_err(format!(
                "file {}() must return str or bytes-like data",
                self.method
            )))
        })
    }
}

impl Read for OwnedPythonReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.error.borrow().is_some() {
            return Err(io::Error::other("Python stream read already failed"));
        }
        if output.is_empty() {
            return Ok(0);
        }
        if self.pending_offset < self.pending.len() {
            let available = &self.pending[self.pending_offset..];
            let count = output.len().min(available.len());
            output[..count].copy_from_slice(&available[..count]);
            self.pending_offset += count;
            if self.pending_offset == self.pending.len() {
                self.pending.clear();
                self.pending_offset = 0;
            }
            return Ok(count);
        }

        let chunk = self
            .read_python_chunk(output.len())
            .map_err(|error| self.fail(error))?;
        match chunk {
            OwnedPythonChunk::Binary(bytes) => {
                if bytes.len() > output.len() {
                    return Err(self.fail(PyOSError::new_err(format!(
                        "binary file {}() returned more data than requested",
                        self.method
                    ))));
                }
                Ok(self.copy_chunk(&bytes, output))
            }
            OwnedPythonChunk::Text { bytes, characters } => {
                if characters > output.len() {
                    return Err(self.fail(PyOSError::new_err(format!(
                        "text file {}() returned more characters than requested",
                        self.method
                    ))));
                }
                Ok(self.copy_chunk(&bytes, output))
            }
        }
    }
}

/// Lazy native iterator over a caller-owned Python byte or text reader.
#[pyclass(name = "_CodecScalarIterator", module = "yggdryl._native", unsendable)]
pub(crate) struct PyCodecScalarIterator {
    inner: Box<dyn Iterator<Item = yggdryl::Result<Scalar>>>,
    field: Option<CoreField>,
    native_scalar: bool,
    reader_error: Rc<RefCell<Option<PyErr>>>,
    finished: bool,
}

#[pymethods]
impl PyCodecScalarIterator {
    // Consumption changes decoder and stream state.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        if self.finished {
            return Ok(None);
        }
        match self.inner.next() {
            None => {
                self.finished = true;
                Ok(None)
            }
            Some(Ok(value)) => {
                let value = match self.field.as_ref() {
                    Some(field) => field.from_natural_value(value),
                    None => Ok(value),
                };
                let value = match value {
                    Ok(value) => value,
                    Err(error) => {
                        self.finished = true;
                        return Err(value_error(error));
                    }
                };
                match decoded_into_py(py, value, self.field.as_ref(), self.native_scalar) {
                    Ok(value) => Ok(Some(value)),
                    Err(error) => {
                        self.finished = true;
                        Err(error)
                    }
                }
            }
            Some(Err(error)) => {
                self.finished = true;
                if let Some(error) = self.reader_error.borrow_mut().take() {
                    Err(error)
                } else {
                    Err(value_error(error))
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WriterMode {
    Unknown,
    Binary,
    Text,
}

/// Adapt a caller-owned Python writer without buffering the encoded document.
///
/// Bytes are attempted first. An initial `TypeError` switches the adapter to
/// an incremental UTF-8 text writer, retaining at most an incomplete code
/// point between core writes.
struct PythonWriter<'py> {
    destination: Bound<'py, PyAny>,
    mode: WriterMode,
    pending_utf8: Vec<u8>,
    error: Option<PyErr>,
}

impl<'py> PythonWriter<'py> {
    fn new(destination: &Bound<'py, PyAny>) -> Self {
        Self {
            destination: destination.clone(),
            mode: WriterMode::Unknown,
            pending_utf8: Vec::new(),
            error: None,
        }
    }

    fn fail(&mut self, error: PyErr) -> io::Error {
        self.error = Some(error);
        io::Error::other("Python stream write failed")
    }

    fn checked_written(
        &mut self,
        result: &Bound<'_, PyAny>,
        maximum: usize,
        unit: &str,
    ) -> io::Result<usize> {
        if result.is_none() {
            return Ok(maximum);
        }
        if result.is_instance_of::<PyBool>() {
            return Err(self.fail(PyTypeError::new_err(
                "file write() must return an integer or None",
            )));
        }
        let Ok(written) = result.extract::<usize>() else {
            return Err(self.fail(PyTypeError::new_err(
                "file write() must return an integer or None",
            )));
        };
        if written == 0 || written > maximum {
            return Err(self.fail(PyOSError::new_err(format!(
                "file write() returned an invalid {unit} count"
            ))));
        }
        Ok(written)
    }

    fn write_binary(&mut self, input: &[u8]) -> io::Result<usize> {
        let py = self.destination.py();
        let value = PyBytes::new(py, input);
        let result = self.destination.call_method1("write", (value,));
        match result {
            Ok(result) => {
                self.mode = WriterMode::Binary;
                self.checked_written(&result, input.len(), "byte")
            }
            Err(error)
                if self.mode == WriterMode::Unknown && error.is_instance_of::<PyTypeError>(py) =>
            {
                self.mode = WriterMode::Text;
                self.write_text_bytes(input)?;
                Ok(input.len())
            }
            Err(error) => Err(self.fail(error)),
        }
    }

    fn write_text_bytes(&mut self, input: &[u8]) -> io::Result<()> {
        self.pending_utf8.extend_from_slice(input);
        let (valid_length, incomplete) = match std::str::from_utf8(&self.pending_utf8) {
            Ok(_) => (self.pending_utf8.len(), false),
            Err(error) if error.error_len().is_none() => (error.valid_up_to(), true),
            Err(_) => {
                return Err(self.fail(PyValueError::new_err("native codec emitted invalid UTF-8")));
            }
        };
        if valid_length != 0 {
            let result = {
                let text = std::str::from_utf8(&self.pending_utf8[..valid_length])
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                let destination = self.destination.clone();
                write_python_text(&destination, text)
            };
            if let Err(error) = result {
                return Err(self.fail(error));
            }
            self.pending_utf8.drain(..valid_length);
        }
        if !incomplete && !self.pending_utf8.is_empty() {
            return Err(self.fail(PyValueError::new_err("native codec emitted invalid UTF-8")));
        }
        Ok(())
    }

    fn finish(&mut self) -> PyResult<()> {
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        if self.mode == WriterMode::Text && !self.pending_utf8.is_empty() {
            return Err(PyValueError::new_err(
                "native codec emitted incomplete UTF-8",
            ));
        }
        Ok(())
    }
}

fn write_python_text(destination: &Bound<'_, PyAny>, mut input: &str) -> PyResult<()> {
    let mut remaining_characters = input.chars().count();
    while !input.is_empty() {
        let result = destination.call_method1("write", (input,))?;
        let written = if result.is_none() {
            remaining_characters
        } else if result.is_instance_of::<PyBool>() {
            return Err(PyTypeError::new_err(
                "file write() must return an integer or None",
            ));
        } else {
            result
                .extract::<usize>()
                .map_err(|_| PyTypeError::new_err("file write() must return an integer or None"))?
        };
        if written == 0 || written > remaining_characters {
            return Err(PyOSError::new_err(
                "file write() returned an invalid character count",
            ));
        }
        if written == remaining_characters {
            return Ok(());
        }
        let byte_offset = input
            .char_indices()
            .nth(written)
            .map_or(input.len(), |(offset, _)| offset);
        input = &input[byte_offset..];
        remaining_characters -= written;
    }
    Ok(())
}

impl Write for PythonWriter<'_> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        if self.error.is_some() {
            return Err(io::Error::other("Python stream write already failed"));
        }
        if input.is_empty() {
            return Ok(0);
        }
        match self.mode {
            WriterMode::Unknown | WriterMode::Binary => self.write_binary(input),
            WriterMode::Text => {
                self.write_text_bytes(input)?;
                Ok(input.len())
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[pyfunction(name = "_codec_infer")]
pub(crate) fn codec_infer(data: &Bound<'_, PyAny>) -> PyResult<&'static str> {
    with_python_bytes(data, |data| {
        yggdryl::text::infer_format(data)
            .map(Format::as_str)
            .map_err(value_error)
    })
}

#[pyfunction(name = "_codec_infer_text")]
pub(crate) fn codec_infer_text(data: &str) -> PyResult<&'static str> {
    yggdryl::text::infer_format(data.as_bytes())
        .map(Format::as_str)
        .map_err(value_error)
}

/// Normalize one public codec alias through the core `Format` parser.
#[pyfunction(name = "_codec_normalize_format")]
pub(crate) fn codec_normalize_format(value: &str) -> PyResult<&'static str> {
    Format::from_str(value)
        .map(Format::as_str)
        .map_err(value_error)
}

/// Infer one codec from a path through the core extension table.
#[pyfunction(name = "_codec_infer_path")]
pub(crate) fn codec_infer_path(value: &str) -> PyResult<&'static str> {
    Format::from_path(Path::new(value))
        .map(Format::as_str)
        .map_err(value_error)
}

#[pyfunction(name = "_codec_decode_inferred")]
#[pyo3(signature = (
    data,
    field = None,
    native_scalar = false,
    max_depth = None,
    max_input_bytes = None,
    max_nodes = None,
    max_documents = None,
))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn codec_decode_inferred(
    py: Python<'_>,
    data: &Bound<'_, PyAny>,
    field: Option<&Bound<'_, PyAny>>,
    native_scalar: bool,
    max_depth: Option<usize>,
    max_input_bytes: Option<usize>,
    max_nodes: Option<usize>,
    max_documents: Option<usize>,
) -> PyResult<Py<PyAny>> {
    let field = field.map(core_field_from_value).transpose()?;
    let limits = limits_from(max_depth, max_input_bytes, max_nodes, max_documents);
    let (_, value) = with_python_bytes(data, |data| {
        let (format, value) =
            yggdryl::text::from_bytes_inferred_with_limits(data, limits).map_err(value_error)?;
        let value = field
            .as_ref()
            .map_or(Ok(value.clone()), |field| field.from_natural_value(value))
            .map_err(value_error)?;
        Ok((format, value))
    })?;
    decoded_into_py(py, value, field.as_ref(), native_scalar)
}

#[pyfunction(name = "_codec_decode_inferred_text")]
#[pyo3(signature = (
    data,
    field = None,
    native_scalar = false,
    max_depth = None,
    max_input_bytes = None,
    max_nodes = None,
    max_documents = None,
))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn codec_decode_inferred_text(
    py: Python<'_>,
    data: &str,
    field: Option<&Bound<'_, PyAny>>,
    native_scalar: bool,
    max_depth: Option<usize>,
    max_input_bytes: Option<usize>,
    max_nodes: Option<usize>,
    max_documents: Option<usize>,
) -> PyResult<Py<PyAny>> {
    let field = field.map(core_field_from_value).transpose()?;
    let limits = limits_from(max_depth, max_input_bytes, max_nodes, max_documents);
    let (_, value) =
        yggdryl::text::from_utf8_inferred_with_limits(data, limits).map_err(value_error)?;
    let value = field
        .as_ref()
        .map_or(Ok(value.clone()), |field| field.from_natural_value(value))
        .map_err(value_error)?;
    decoded_into_py(py, value, field.as_ref(), native_scalar)
}

#[pyfunction(name = "_codec_encode")]
#[pyo3(signature = (value, format, indent = -2))]
pub(crate) fn codec_encode<'py>(
    py: Python<'py>,
    value: &Bound<'py, PyAny>,
    format: &str,
    indent: i16,
) -> PyResult<Bound<'py, PyBytes>> {
    let value = from_py(value)?;
    let encoded = yggdryl::text::into_bytes_with_formatting(
        &value,
        format_from_str(format)?,
        formatting_from_code(indent)?,
    )
    .map_err(value_error)?;
    Ok(PyBytes::new(py, &encoded))
}

#[pyfunction(name = "_codec_encode_writer")]
#[pyo3(signature = (value, destination, format, indent = -2))]
pub(crate) fn codec_encode_writer(
    value: &Bound<'_, PyAny>,
    destination: &Bound<'_, PyAny>,
    format: &str,
    indent: i16,
) -> PyResult<()> {
    let value = from_py(value)?;
    let format = format_from_str(format)?;
    encode_value_to_python_writer(&value, destination, format, formatting_from_code(indent)?)
}

fn encode_value_to_python_writer(
    value: &Scalar,
    destination: &Bound<'_, PyAny>,
    format: Format,
    formatting: Formatting,
) -> PyResult<()> {
    let mut writer = PythonWriter::new(destination);
    let (encode_result, flush_result) = {
        let mut buffered = BufWriter::new(&mut writer);
        let encode_result =
            yggdryl::text::into_writer_with_formatting(value, &mut buffered, format, formatting);
        let flush_result = buffered.flush();
        (encode_result, flush_result)
    };
    writer.finish()?;
    encode_result.map_err(value_error)?;
    flush_result.map_err(PyOSError::new_err)?;
    Ok(())
}

#[pyfunction(name = "_codec_encode_path")]
#[pyo3(signature = (value, destination, format, indent = -2))]
pub(crate) fn codec_encode_path(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    destination: &Bound<'_, PyAny>,
    format: &str,
    indent: i16,
) -> PyResult<()> {
    let format = format_from_str(format)?;
    let value = from_py(value)?;

    if format == Format::Toml {
        yggdryl::toml::validate_for_write(&value).map_err(value_error)?;
    }

    // Delegate path interpretation and OSError construction to Python while
    // delaying file creation until conversion and option validation succeed.
    let stream = py
        .import("builtins")?
        .getattr("open")?
        .call1((destination, "wb"))?;
    let result =
        encode_value_to_python_writer(&value, &stream, format, formatting_from_code(indent)?);
    let close_result = stream.call_method0("close");
    match (result, close_result) {
        (Err(error), _) | (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(_)) => Ok(()),
    }
}

/// Assemble the read-side options from the boundary's two switches.
///
/// Substitution is on when the caller supplied variables *or* asked for the
/// environment, and off - meaning no walk and no `std::env` call at all - when
/// they did neither. The two are separate because a document that resolves
/// `{{ AWS_SECRET_ACCESS_KEY }}` into a value that is then dumped has leaked
/// it, so reaching the process environment is its own decision.
fn loading_from(
    placeholders: Option<&Bound<'_, PyAny>>,
    environment: bool,
    field: Option<CoreField>,
    limits: Limits,
) -> PyResult<yggdryl::text::Loading> {
    let mut loading = yggdryl::text::Loading::new().with_limits(limits);
    if placeholders.is_some() || environment {
        let variables = match placeholders {
            Some(mapping) => yggdryl::text::Placeholders::from_variables(&from_py(mapping)?)
                .map_err(value_error)?,
            None => yggdryl::text::Placeholders::new(),
        };
        loading = loading.with_placeholders(variables.with_environment(environment));
    }
    if let Some(field) = field {
        loading = loading.with_field(field);
    }
    Ok(loading)
}

#[pyfunction(name = "_codec_decode")]
#[pyo3(signature = (
    data,
    format,
    placeholders = None,
    environment = false,
    field = None,
    native_scalar = false,
    max_depth = None,
    max_input_bytes = None,
    max_nodes = None,
    max_documents = None,
))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn codec_decode(
    py: Python<'_>,
    data: &Bound<'_, PyAny>,
    format: &str,
    placeholders: Option<&Bound<'_, PyAny>>,
    environment: bool,
    field: Option<&Bound<'_, PyAny>>,
    native_scalar: bool,
    max_depth: Option<usize>,
    max_input_bytes: Option<usize>,
    max_nodes: Option<usize>,
    max_documents: Option<usize>,
) -> PyResult<Py<PyAny>> {
    let format = format_from_str(format)?;
    let field = field.map(core_field_from_value).transpose()?;
    let loading = loading_from(
        placeholders,
        environment,
        field.clone(),
        limits_from(max_depth, max_input_bytes, max_nodes, max_documents),
    )?;
    let value = with_python_bytes(data, |data| {
        yggdryl::text::from_bytes_with(data, format, &loading).map_err(value_error)
    })?;
    decoded_into_py(py, value, field.as_ref(), native_scalar)
}

#[pyfunction(name = "_codec_decode_text")]
#[pyo3(signature = (
    data,
    format,
    placeholders = None,
    environment = false,
    field = None,
    native_scalar = false,
    max_depth = None,
    max_input_bytes = None,
    max_nodes = None,
    max_documents = None,
))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn codec_decode_text(
    py: Python<'_>,
    data: &str,
    format: &str,
    placeholders: Option<&Bound<'_, PyAny>>,
    environment: bool,
    field: Option<&Bound<'_, PyAny>>,
    native_scalar: bool,
    max_depth: Option<usize>,
    max_input_bytes: Option<usize>,
    max_nodes: Option<usize>,
    max_documents: Option<usize>,
) -> PyResult<Py<PyAny>> {
    let field = field.map(core_field_from_value).transpose()?;
    let loading = loading_from(
        placeholders,
        environment,
        field.clone(),
        limits_from(max_depth, max_input_bytes, max_nodes, max_documents),
    )?;
    let value = yggdryl::text::from_utf8_with(data, format_from_str(format)?, &loading)
        .map_err(value_error)?;
    decoded_into_py(py, value, field.as_ref(), native_scalar)
}

#[pyfunction(name = "_codec_decode_reader")]
#[pyo3(signature = (
    source,
    format,
    placeholders = None,
    environment = false,
    field = None,
    native_scalar = false,
    max_depth = None,
    max_input_bytes = None,
    max_nodes = None,
    max_documents = None,
))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn codec_decode_reader(
    py: Python<'_>,
    source: &Bound<'_, PyAny>,
    format: &str,
    placeholders: Option<&Bound<'_, PyAny>>,
    environment: bool,
    field: Option<&Bound<'_, PyAny>>,
    native_scalar: bool,
    max_depth: Option<usize>,
    max_input_bytes: Option<usize>,
    max_nodes: Option<usize>,
    max_documents: Option<usize>,
) -> PyResult<Py<PyAny>> {
    let field = field.map(core_field_from_value).transpose()?;
    let loading = loading_from(
        placeholders,
        environment,
        field.clone(),
        limits_from(max_depth, max_input_bytes, max_nodes, max_documents),
    )?;
    let mut reader = PythonReader::new(source);
    let decoded = yggdryl::text::from_reader_with(&mut reader, format_from_str(format)?, &loading);
    if let Some(error) = reader.take_error() {
        return Err(error);
    }
    let value = decoded.map_err(value_error)?;
    decoded_into_py(py, value, field.as_ref(), native_scalar)
}

/// Start one lazy core decoder over a caller-owned Python reader.
///
/// Python selects the language protocol (`read` or `readline`) and performs
/// target-class materialization after each pull. Document boundaries, limits,
/// and cumulative byte positions all remain properties of the Rust parser.
#[pyfunction(name = "_codec_decode_iter")]
#[pyo3(signature = (
    source,
    format,
    max_depth = None,
    max_input_bytes = None,
    max_nodes = None,
    max_documents = None,
    field = None,
    native_scalar = false,
))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn codec_decode_iter(
    source: &Bound<'_, PyAny>,
    format: &str,
    max_depth: Option<usize>,
    max_input_bytes: Option<usize>,
    max_nodes: Option<usize>,
    max_documents: Option<usize>,
    field: Option<&Bound<'_, PyAny>>,
    native_scalar: bool,
) -> PyResult<PyCodecScalarIterator> {
    let format = format_from_str(format)?;
    if format == Format::Toml {
        return Err(PyValueError::new_err(
            "TOML supports exactly one document; use loads()",
        ));
    }
    let methods = if format == Format::JsonLines {
        ["readline", "read"]
    } else {
        ["read", "readline"]
    };
    let method = if source.hasattr(methods[0])? {
        methods[0]
    } else if source.hasattr(methods[1])? {
        methods[1]
    } else {
        return Err(PyTypeError::new_err(
            "source must provide read(size) or readline(size)",
        ));
    };

    let limits = limits_from(max_depth, max_input_bytes, max_nodes, max_documents);
    let reader_error = Rc::new(RefCell::new(None));
    let reader = OwnedPythonReader::new(source.clone().unbind(), method, Rc::clone(&reader_error));
    let inner: Box<dyn Iterator<Item = yggdryl::Result<Scalar>>> = match format {
        Format::Json => Box::new(yggdryl::json::Reader::with_limits(reader, limits)),
        Format::JsonLines => Box::new(yggdryl::json::LinesReader::with_limits(reader, limits)),
        Format::Yaml => Box::new(yggdryl::yaml::Reader::with_limits(reader, limits)),
        Format::Toml => unreachable!("TOML was rejected before reader construction"),
    };
    Ok(PyCodecScalarIterator {
        inner,
        field: field.map(core_field_from_value).transpose()?,
        native_scalar,
        reader_error,
        finished: false,
    })
}

#[pyfunction(name = "_codec_encode_all")]
#[pyo3(signature = (values, format, indent = -2))]
pub(crate) fn codec_encode_all<'py>(
    py: Python<'py>,
    values: &Bound<'py, PyAny>,
    format: &str,
    indent: i16,
) -> PyResult<Bound<'py, PyBytes>> {
    let iterator = values.try_iter()?;
    let closer = iterator.clone();
    let mut encoded_values = Vec::new();
    for (index, value) in iterator.enumerate() {
        if index >= MAX_PYTHON_DOCUMENTS {
            close_iterator(&closer);
            return Err(PyValueError::new_err(format!(
                "codec collection exceeds the {MAX_PYTHON_DOCUMENTS}-document limit"
            )));
        }
        let value = match value.and_then(|value| from_py(&value)) {
            Ok(value) => value,
            Err(error) => {
                close_iterator(&closer);
                return Err(error);
            }
        };
        encoded_values.push(value);
    }
    let mut output = Vec::new();
    yggdryl::text::into_writer_all_with_formatting(
        &encoded_values,
        &mut output,
        format_from_str(format)?,
        formatting_from_code(indent)?,
    )
    .map_err(value_error)?;
    Ok(PyBytes::new(py, &output))
}

#[pyfunction(name = "_codec_encode_all_writer")]
#[pyo3(signature = (values, destination, format, indent = -2))]
pub(crate) fn codec_encode_all_writer(
    values: &Bound<'_, PyAny>,
    destination: &Bound<'_, PyAny>,
    format: &str,
    indent: i16,
) -> PyResult<()> {
    let format = format_from_str(format)?;
    let formatting = formatting_from_code(indent)?;
    let iterator = values.try_iter()?;
    let closer = iterator.clone();
    let mut writer = PythonWriter::new(destination);
    let encode_result = {
        let mut buffered = BufWriter::new(&mut writer);
        let mut result = Ok(());
        for (index, item) in iterator.enumerate() {
            if index >= MAX_PYTHON_DOCUMENTS {
                close_iterator(&closer);
                result = Err(PyValueError::new_err(format!(
                    "codec collection exceeds the {MAX_PYTHON_DOCUMENTS}-document limit"
                )));
                break;
            }
            let value = match item.and_then(|item| from_py(&item)) {
                Ok(value) => value,
                Err(error) => {
                    close_iterator(&closer);
                    result = Err(error);
                    break;
                }
            };
            let write_result: PyResult<()> = match format {
                Format::Yaml => if index != 0 {
                    buffered.write_all(b"---\n").map_err(PyOSError::new_err)
                } else {
                    Ok(())
                }
                .and_then(|()| {
                    yggdryl::yaml::into_writer_with_formatting(&value, &mut buffered, formatting)
                        .map_err(value_error)
                }),
                Format::Json | Format::JsonLines => yggdryl::text::into_writer_with_formatting(
                    &value,
                    &mut buffered,
                    Format::JsonLines,
                    formatting,
                )
                .map_err(value_error),
                Format::Toml => Err(PyValueError::new_err(
                    "TOML supports exactly one document; use dump()",
                )),
            };
            let write_result =
                write_result.and_then(|()| buffered.flush().map_err(PyOSError::new_err));
            if let Err(error) = write_result {
                close_iterator(&closer);
                result = Err(error);
                break;
            }
        }
        if result.is_ok()
            && let Err(error) = buffered.flush()
        {
            result = Err(PyOSError::new_err(error.to_string()));
        }
        result
    };
    writer.finish()?;
    encode_result
}

fn close_iterator(iterator: &Bound<'_, PyIterator>) {
    if let Ok(close) = iterator.getattr("close") {
        let _ = close.call0();
    }
}

#[pyfunction(name = "_codec_decode_all")]
#[pyo3(signature = (
    data,
    format,
    field = None,
    native_scalar = false,
    max_depth = None,
    max_input_bytes = None,
    max_nodes = None,
    max_documents = None,
))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn codec_decode_all(
    py: Python<'_>,
    data: &Bound<'_, PyAny>,
    format: &str,
    field: Option<&Bound<'_, PyAny>>,
    native_scalar: bool,
    max_depth: Option<usize>,
    max_input_bytes: Option<usize>,
    max_nodes: Option<usize>,
    max_documents: Option<usize>,
) -> PyResult<Vec<Py<PyAny>>> {
    let format = format_from_str(format)?;
    let field = field.map(core_field_from_value).transpose()?;
    let limits = limits_from(max_depth, max_input_bytes, max_nodes, max_documents);
    with_python_bytes(data, |data| {
        field
            .as_ref()
            .map_or_else(
                || yggdryl::text::from_bytes_all_with_limits(data, format, limits),
                |field| {
                    yggdryl::text::from_bytes_all_with_field_and_limits(data, format, field, limits)
                },
            )
            .map_err(value_error)
    })?
    .into_iter()
    .map(|value| decoded_into_py(py, value, field.as_ref(), native_scalar))
    .collect()
}

#[pyfunction(name = "_codec_decode_all_text")]
#[pyo3(signature = (
    data,
    format,
    field = None,
    native_scalar = false,
    max_depth = None,
    max_input_bytes = None,
    max_nodes = None,
    max_documents = None,
))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn codec_decode_all_text(
    py: Python<'_>,
    data: &str,
    format: &str,
    field: Option<&Bound<'_, PyAny>>,
    native_scalar: bool,
    max_depth: Option<usize>,
    max_input_bytes: Option<usize>,
    max_nodes: Option<usize>,
    max_documents: Option<usize>,
) -> PyResult<Vec<Py<PyAny>>> {
    let format = format_from_str(format)?;
    let field = field.map(core_field_from_value).transpose()?;
    let limits = limits_from(max_depth, max_input_bytes, max_nodes, max_documents);
    field
        .as_ref()
        .map_or_else(
            || yggdryl::text::from_utf8_all_with_limits(data, format, limits),
            |field| yggdryl::text::from_utf8_all_with_field_and_limits(data, format, field, limits),
        )
        .map_err(value_error)?
        .into_iter()
        .map(|value| decoded_into_py(py, value, field.as_ref(), native_scalar))
        .collect()
}

#[pyfunction(name = "_codec_decode_all_reader")]
#[pyo3(signature = (
    source,
    format,
    field = None,
    native_scalar = false,
    max_depth = None,
    max_input_bytes = None,
    max_nodes = None,
    max_documents = None,
))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn codec_decode_all_reader(
    py: Python<'_>,
    source: &Bound<'_, PyAny>,
    format: &str,
    field: Option<&Bound<'_, PyAny>>,
    native_scalar: bool,
    max_depth: Option<usize>,
    max_input_bytes: Option<usize>,
    max_nodes: Option<usize>,
    max_documents: Option<usize>,
) -> PyResult<Vec<Py<PyAny>>> {
    let format = format_from_str(format)?;
    let field = field.map(core_field_from_value).transpose()?;
    let limits = limits_from(max_depth, max_input_bytes, max_nodes, max_documents);
    let mut reader = PythonReader::new(source);
    let decoded = match field.as_ref() {
        Some(field) => {
            yggdryl::text::from_reader_all_with_field_and_limits(&mut reader, format, field, limits)
        }
        None => yggdryl::text::from_reader_all_with_limits(&mut reader, format, limits),
    };
    if let Some(error) = reader.take_error() {
        return Err(error);
    }
    decoded
        .map_err(value_error)?
        .into_iter()
        .map(|value| decoded_into_py(py, value, field.as_ref(), native_scalar))
        .collect()
}

/// Cross one decoded core value without materializing a lossy Python view.
pub(crate) fn decoded_into_py(
    py: Python<'_>,
    value: Scalar,
    field: Option<&CoreField>,
    native_scalar: bool,
) -> PyResult<Py<PyAny>> {
    if native_scalar {
        return Ok(Py::new(py, PyScalar::from_inner(value))?.into_any());
    }
    decoded_as_py(py, &value, field)
}

pub(crate) fn decoded_as_py(
    py: Python<'_>,
    value: &Scalar,
    field: Option<&CoreField>,
) -> PyResult<Py<PyAny>> {
    match field {
        Some(field) => as_py_with_field(py, value, field),
        None => as_py(py, value),
    }
}
