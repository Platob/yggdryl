//! Direct conversion between Python objects and the native byte codecs.
//!
//! Everything here is plumbing: bytes in, bytes out, and the stream adapters
//! that let the core read from and write to a caller-owned Python file object.
//! The value conversion itself belongs to [`crate::value`], so a document is
//! read and written through exactly one pair of functions.

use std::io::{self, BufWriter, Read, Write};
use std::path::Path;

use pyo3::exceptions::{PyOSError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyByteArray, PyBytes, PyIterator, PyMemoryView, PyString};
use yggdryl::text::{Format, Value};

use crate::value::{as_py, from_py};
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
pub(crate) fn codec_decode_inferred(
    py: Python<'_>,
    data: &Bound<'_, PyAny>,
) -> PyResult<Py<PyAny>> {
    let (_, value) = with_python_bytes(data, |data| {
        yggdryl::text::from_slice_inferred(data).map_err(value_error)
    })?;
    as_py(py, &value)
}

#[pyfunction(name = "_codec_decode_inferred_text")]
pub(crate) fn codec_decode_inferred_text(py: Python<'_>, data: &str) -> PyResult<Py<PyAny>> {
    let (_, value) = yggdryl::text::from_str_inferred(data).map_err(value_error)?;
    as_py(py, &value)
}

#[pyfunction(name = "_codec_encode")]
pub(crate) fn codec_encode<'py>(
    py: Python<'py>,
    value: &Bound<'py, PyAny>,
    format: &str,
) -> PyResult<Bound<'py, PyBytes>> {
    let value = from_py(value)?;
    let encoded = yggdryl::text::to_vec(&value, format_from_str(format)?).map_err(value_error)?;
    Ok(PyBytes::new(py, &encoded))
}

#[pyfunction(name = "_codec_encode_writer")]
pub(crate) fn codec_encode_writer(
    value: &Bound<'_, PyAny>,
    destination: &Bound<'_, PyAny>,
    format: &str,
) -> PyResult<()> {
    let value = from_py(value)?;
    let format = format_from_str(format)?;
    encode_value_to_python_writer(&value, destination, format)
}

fn encode_value_to_python_writer(
    value: &Value,
    destination: &Bound<'_, PyAny>,
    format: Format,
) -> PyResult<()> {
    let mut writer = PythonWriter::new(destination);
    let (encode_result, flush_result) = {
        let mut buffered = BufWriter::new(&mut writer);
        let encode_result = yggdryl::text::to_writer(&mut buffered, value, format);
        let flush_result = buffered.flush();
        (encode_result, flush_result)
    };
    writer.finish()?;
    encode_result.map_err(value_error)?;
    flush_result.map_err(PyOSError::new_err)?;
    Ok(())
}

#[pyfunction(name = "_codec_encode_path")]
pub(crate) fn codec_encode_path(
    py: Python<'_>,
    value: &Bound<'_, PyAny>,
    destination: &Bound<'_, PyAny>,
    format: &str,
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
    let result = encode_value_to_python_writer(&value, &stream, format);
    let close_result = stream.call_method0("close");
    match (result, close_result) {
        (Err(error), _) | (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(_)) => Ok(()),
    }
}

#[pyfunction(name = "_codec_decode")]
pub(crate) fn codec_decode(
    py: Python<'_>,
    data: &Bound<'_, PyAny>,
    format: &str,
) -> PyResult<Py<PyAny>> {
    let format = format_from_str(format)?;
    let value = with_python_bytes(data, |data| {
        yggdryl::text::from_slice(data, format).map_err(value_error)
    })?;
    as_py(py, &value)
}

#[pyfunction(name = "_codec_decode_text")]
pub(crate) fn codec_decode_text(py: Python<'_>, data: &str, format: &str) -> PyResult<Py<PyAny>> {
    let value = yggdryl::text::from_str(data, format_from_str(format)?).map_err(value_error)?;
    as_py(py, &value)
}

#[pyfunction(name = "_codec_decode_reader")]
pub(crate) fn codec_decode_reader(
    py: Python<'_>,
    source: &Bound<'_, PyAny>,
    format: &str,
) -> PyResult<Py<PyAny>> {
    let mut reader = PythonReader::new(source);
    let decoded = yggdryl::text::from_reader(&mut reader, format_from_str(format)?);
    if let Some(error) = reader.take_error() {
        return Err(error);
    }
    let value = decoded.map_err(value_error)?;
    as_py(py, &value)
}

#[pyfunction(name = "_codec_encode_all")]
pub(crate) fn codec_encode_all<'py>(
    py: Python<'py>,
    values: &Bound<'py, PyAny>,
    format: &str,
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
    yggdryl::text::to_writer_all(&mut output, &encoded_values, format_from_str(format)?)
        .map_err(value_error)?;
    Ok(PyBytes::new(py, &output))
}

#[pyfunction(name = "_codec_encode_all_writer")]
pub(crate) fn codec_encode_all_writer(
    values: &Bound<'_, PyAny>,
    destination: &Bound<'_, PyAny>,
    format: &str,
) -> PyResult<()> {
    let format = format_from_str(format)?;
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
                    yggdryl::yaml::to_writer(&mut buffered, &value).map_err(value_error)
                }),
                Format::Json | Format::JsonLines => {
                    yggdryl::text::to_writer(&mut buffered, &value, Format::JsonLines)
                        .map_err(value_error)
                }
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
pub(crate) fn codec_decode_all(
    py: Python<'_>,
    data: &Bound<'_, PyAny>,
    format: &str,
) -> PyResult<Vec<Py<PyAny>>> {
    let format = format_from_str(format)?;
    with_python_bytes(data, |data| {
        yggdryl::text::from_slice_all(data, format).map_err(value_error)
    })?
    .iter()
    .map(|value| as_py(py, value))
    .collect()
}

#[pyfunction(name = "_codec_decode_all_text")]
pub(crate) fn codec_decode_all_text(
    py: Python<'_>,
    data: &str,
    format: &str,
) -> PyResult<Vec<Py<PyAny>>> {
    yggdryl::text::from_str_all(data, format_from_str(format)?)
        .map_err(value_error)?
        .iter()
        .map(|value| as_py(py, value))
        .collect()
}

#[pyfunction(name = "_codec_decode_all_reader")]
pub(crate) fn codec_decode_all_reader(
    py: Python<'_>,
    source: &Bound<'_, PyAny>,
    format: &str,
) -> PyResult<Vec<Py<PyAny>>> {
    let mut reader = PythonReader::new(source);
    let decoded = yggdryl::text::from_reader_all(&mut reader, format_from_str(format)?);
    if let Some(error) = reader.take_error() {
        return Err(error);
    }
    decoded
        .map_err(value_error)?
        .iter()
        .map(|value| as_py(py, value))
        .collect()
}
