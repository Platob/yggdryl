//! Python views over the core XML codecs.
//!
//! This boundary owns no XML model: the document walk, the mapping, the
//! escaping and the schema all remain native. Python values enter and leave
//! through the same shared `Scalar` conversion the structured codecs use, and
//! a schema crosses as the `Field` the core answers.

use pyo3::exceptions::PyTypeError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyByteArray, PyBytes};
use yggdryl::media::xml;
use yggdryl::text::{Formatting, Indent, Limits};

use crate::types::field::PyField;
use crate::types::scalar::{PyScalar, from_py};
use crate::value_error;

/// Decode one XML document into a natural Python value.
///
/// Attributes and child elements are one namespace of names, a leaf is its
/// characters, and a repeated child is a list. Every leaf is text, because
/// text is all a document proves; a `Field` is what types them.
#[pyfunction]
#[pyo3(
    name = "xml_loads",
    signature = (data, *, max_depth = None, max_input_bytes = None, max_nodes = None)
)]
pub(crate) fn xml_loads(
    py: Python<'_>,
    data: &Bound<'_, PyAny>,
    max_depth: Option<usize>,
    max_input_bytes: Option<usize>,
    max_nodes: Option<usize>,
) -> PyResult<Py<PyAny>> {
    let bytes = bytes_from_value(data)?;
    let limits = decode_limits(max_depth, max_input_bytes, max_nodes);
    let value = py
        .detach(|| xml::from_bytes_with_limits(&bytes, limits))
        .map_err(value_error)?;
    crate::types::scalar::as_py(py, &value)
}

/// Encode one value as a whole XML document rooted at `name`.
///
/// XML has no anonymous document, so the root's name is an argument rather
/// than a default nobody chose.
#[pyfunction]
#[pyo3(name = "xml_dumps", signature = (value, name, *, indent = None))]
pub(crate) fn xml_dumps<'py>(
    py: Python<'py>,
    value: &Bound<'_, PyAny>,
    name: &str,
    indent: Option<u8>,
) -> PyResult<Bound<'py, PyBytes>> {
    let value = from_py(value)?;
    let formatting = Formatting::new().with_indent(match indent {
        Some(0) => Indent::None,
        Some(width) => Indent::Spaces(width),
        None => Indent::Default,
    });
    let encoded = py
        .detach(|| xml::into_bytes_with_formatting(name, &value, formatting))
        .map_err(value_error)?;
    Ok(PyBytes::new(py, &encoded))
}

/// Read an XML Schema document as the field it declares.
///
/// The field is what a record read takes, so a schema types a document
/// through the ordinary declared-field path rather than a second surface.
#[pyfunction]
#[pyo3(
    name = "xml_schema",
    signature = (data, *, root = None, max_depth = None, max_input_bytes = None, max_nodes = None)
)]
pub(crate) fn xml_schema(
    py: Python<'_>,
    data: &Bound<'_, PyAny>,
    root: Option<&str>,
    max_depth: Option<usize>,
    max_input_bytes: Option<usize>,
    max_nodes: Option<usize>,
) -> PyResult<PyField> {
    let bytes = bytes_from_value(data)?;
    let limits = decode_limits(max_depth, max_input_bytes, max_nodes);
    let field = py
        .detach(|| xml::field_from_xsd(&bytes, limits, root))
        .map_err(value_error)?;
    Ok(PyField::from_inner(field))
}

/// Write one field as the XML Schema that declares it.
#[pyfunction]
#[pyo3(name = "xml_schema_dumps", signature = (field, *, indent = None))]
pub(crate) fn xml_schema_dumps<'py>(
    py: Python<'py>,
    field: &Bound<'_, PyAny>,
    indent: Option<u8>,
) -> PyResult<Bound<'py, PyBytes>> {
    let field = crate::types::field::core_field_from_value(field)?;
    let formatting = Formatting::new().with_indent(match indent {
        Some(0) => Indent::None,
        Some(width) => Indent::Spaces(width),
        None => Indent::Default,
    });
    let encoded = py
        .detach(|| xml::field_into_xsd(&field, formatting))
        .map_err(value_error)?;
    Ok(PyBytes::new(py, &encoded))
}

/// Decode one XML document under a field, as the typed value it declares.
#[pyfunction]
#[pyo3(
    name = "xml_loads_with_field",
    signature = (data, field, *, max_depth = None, max_input_bytes = None, max_nodes = None)
)]
pub(crate) fn xml_loads_with_field(
    py: Python<'_>,
    data: &Bound<'_, PyAny>,
    field: &Bound<'_, PyAny>,
    max_depth: Option<usize>,
    max_input_bytes: Option<usize>,
    max_nodes: Option<usize>,
) -> PyResult<PyScalar> {
    let bytes = bytes_from_value(data)?;
    let field = crate::types::field::core_field_from_value(field)?;
    let limits = decode_limits(max_depth, max_input_bytes, max_nodes);
    let value = py
        .detach(|| {
            xml::from_bytes_with_limits(&bytes, limits)
                .and_then(|value| field.from_natural_value(value))
        })
        .map_err(value_error)?;
    Ok(PyScalar::from_inner(value))
}

/// Resolve the decode budget from the keywords a caller gave.
fn decode_limits(
    max_depth: Option<usize>,
    max_input_bytes: Option<usize>,
    max_nodes: Option<usize>,
) -> Limits {
    let defaults = Limits::default();
    Limits::new(
        max_depth.unwrap_or_else(|| defaults.max_depth()),
        max_input_bytes.unwrap_or_else(|| defaults.max_input_bytes()),
        max_nodes.unwrap_or_else(|| defaults.max_nodes()),
        defaults.max_documents(),
    )
}

/// Read every documented spelling of a byte payload exactly once.
fn bytes_from_value(value: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    if let Ok(text) = value.extract::<String>() {
        return Ok(text.into_bytes());
    }
    if let Ok(bytes) = value.cast::<PyBytes>() {
        return Ok(bytes.as_bytes().to_vec());
    }
    if let Ok(bytes) = value.cast::<PyByteArray>() {
        return Ok(bytes.to_vec());
    }
    if value.hasattr("tobytes")? {
        let owned = value.call_method0("tobytes")?;
        if let Ok(bytes) = owned.cast::<PyBytes>() {
            return Ok(bytes.as_bytes().to_vec());
        }
    }
    Err(PyTypeError::new_err(
        "XML input must be str, bytes, bytearray, or memoryview",
    ))
}
