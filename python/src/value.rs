//! The one conversion between a Python object and a core [`Value`].
//!
//! Both directions live in this module because they are one contract: what
//! [`from_py`] writes, [`as_py`] has to read back. Every `load` and `dump`
//! entry point routes through this pair, so the codecs cannot grow a second
//! spelling of the same value between them.
//!
//! Identity the core value model does not carry is dropped here rather than
//! smuggled across as a name over an untyped payload. A `set` arrives as a
//! sequence, a `uuid.UUID` as its text, a `pathlib.Path` as its string: the
//! shape survives, the class does not. `docs/extensions/python.md` lists every
//! loss, because a caller has to be able to predict them.

use std::collections::HashSet;

use pyo3::PyTypeInfo;
use pyo3::exceptions::{PyOverflowError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{
    PyAny, PyBool, PyByteArray, PyBytes, PyComplex, PyDict, PyFloat, PyFrozenSet, PyInt, PyList,
    PyMemoryView, PySet, PyString, PyTuple, PyType,
};
use yggdryl::text::{Float, Value};
use yggdryl::{TimeUnit, Timezone};

use crate::datatype::PyDataType;
use crate::field::PyField;
use crate::timezone::core_timezone_from_value;
use crate::uri::{PyUri, PyUrl, PyUrn};
use crate::value_error;

/// How deep a Python graph may nest before conversion refuses to recurse.
const MAX_PYTHON_DEPTH: usize = 128;

/// The day `date.toordinal` counts from, expressed as a Unix epoch offset.
///
/// `date(1970, 1, 1).toordinal()` is this number, so subtracting it turns
/// Python's proleptic Gregorian ordinal into the epoch day count a value holds.
const EPOCH_ORDINAL: i64 = 719_163;

/// The microseconds one whole day holds, the unit every temporal crosses in.
const MICROSECONDS_PER_DAY: i64 = 86_400_000_000;

/// Convert one Python object into a core value.
///
/// # Errors
///
/// Returns an error for a cyclic graph, a graph deeper than the codec limit, a
/// mapping whose distinct Python keys collide as values, or an object that has
/// no value shape at all.
pub(crate) fn from_py(value: &Bound<'_, PyAny>) -> PyResult<Value> {
    Encoder::default().convert(value, 0)
}

/// Convert one core value into the Python object that names it.
///
/// # Errors
///
/// Returns an error when a temporal carries a resolution finer than Python's
/// microsecond, when a date or time falls outside the range `datetime`
/// represents, or when distinct encoded mapping keys collide under Python
/// equality.
pub(crate) fn as_py(py: Python<'_>, value: &Value) -> PyResult<Py<PyAny>> {
    match value {
        Value::Null => Ok(py.None()),
        Value::Bool(value) => Ok(value.into_pyobject(py)?.to_owned().into_any().unbind()),
        Value::I64(value) => Ok(value.into_pyobject(py)?.into_any().unbind()),
        Value::U64(value) => Ok(value.into_pyobject(py)?.into_any().unbind()),
        Value::I128(value) => Ok(value.into_pyobject(py)?.into_any().unbind()),
        Value::U128(value) => Ok(value.into_pyobject(py)?.into_any().unbind()),
        Value::Float(value) => Ok(value.as_f64().into_pyobject(py)?.into_any().unbind()),
        Value::Decimal(unscaled, scale) => decimal_to_python(py, *unscaled, *scale),
        Value::String(value) => Ok(PyString::new(py, value.as_str()).into_any().unbind()),
        Value::Bytes(value) => Ok(PyBytes::new(py, value).into_any().unbind()),
        Value::Date(days) => date_to_python(py, *days),
        Value::Time(..) => time_to_python(py, value),
        Value::Timestamp(..) => timestamp_to_python(py, value),
        Value::Duration(..) => duration_to_python(py, value),
        Value::Sequence(items) => {
            let items = items
                .iter()
                .map(|item| as_py(py, item))
                .collect::<PyResult<Vec<_>>>()?;
            Ok(PyList::new(py, items)?.into_any().unbind())
        }
        Value::Mapping(entries) => mapping_to_python(py, entries),
    }
}

/// Build the Python dictionary one core mapping names.
fn mapping_to_python(py: Python<'_>, entries: &[(Value, Value)]) -> PyResult<Py<PyAny>> {
    let output = PyDict::new(py);
    for (key, value) in entries {
        let key = as_py_key(py, key)?;
        if output.contains(&key)? {
            return Err(PyValueError::new_err(
                "distinct encoded mapping keys collide under Python equality",
            ));
        }
        output.set_item(key, as_py(py, value)?)?;
    }
    Ok(output.into_any().unbind())
}

/// Convert one core value into a form Python can hash as a mapping key.
///
/// A sequence becomes a tuple and a mapping becomes a tuple of its entries,
/// because those are the hashable spellings of the same shapes. This is the one
/// place the two directions deliberately disagree, and it is why the records
/// layer reads a tuple of pairs back as a mapping.
fn as_py_key(py: Python<'_>, value: &Value) -> PyResult<Py<PyAny>> {
    match value {
        Value::Sequence(items) => {
            let items = items
                .iter()
                .map(|item| as_py_key(py, item))
                .collect::<PyResult<Vec<_>>>()?;
            Ok(PyTuple::new(py, items)?.into_any().unbind())
        }
        Value::Mapping(entries) => {
            let entries = entries
                .iter()
                .map(|(key, value)| Ok((as_py_key(py, key)?, as_py_key(py, value)?)))
                .collect::<PyResult<Vec<_>>>()?;
            Ok(PyTuple::new(py, entries)?.into_any().unbind())
        }
        _ => as_py(py, value),
    }
}

/// The Python-object-to-value walk, carrying the identities already on the stack.
#[derive(Default)]
struct Encoder {
    active: HashSet<usize>,
}

impl Encoder {
    /// Convert one object, refusing to recurse past the codec's depth limit.
    fn convert(&mut self, value: &Bound<'_, PyAny>, depth: usize) -> PyResult<Value> {
        if depth >= MAX_PYTHON_DEPTH {
            return Err(PyValueError::new_err(format!(
                "Python value exceeds the {MAX_PYTHON_DEPTH}-level codec limit"
            )));
        }
        if let Some(value) = self.convert_exact_builtin(value, depth)? {
            return Ok(value);
        }
        if let Some(value) = native_wrapper_to_value(value) {
            return Ok(value);
        }
        // A path is recognized by its protocol rather than by its class,
        // because every path-like object answers `__fspath__` and none of them
        // share a base class.
        if value.hasattr("__fspath__")? {
            return path_to_value(value);
        }
        let identity = type_identity(value)?;
        match identity.as_str() {
            "decimal.Decimal" => decimal_to_value(value),
            "uuid.UUID" => Ok(Value::String(value.str()?.to_str()?.into())),
            "datetime.datetime" => timestamp_to_value(value),
            "datetime.time" => time_to_value(value),
            "datetime.date" => date_to_value(value),
            "datetime.timedelta" => duration_to_value(value),
            "builtins.complex" => complex_to_value(value),
            "builtins.range" | "builtins.slice" => self.convert_triple(value, depth),
            "collections.deque" => self.convert_iterator(value, depth),
            "collections.OrderedDict" | "collections.Counter" | "collections.defaultdict" => {
                self.convert_dict(value, depth)
            }
            _ => self.convert_other(value, &identity, depth),
        }
    }

    /// Convert the builtin types whose exact class needs no further inspection.
    ///
    /// Exact-class matching comes first because it is the hot path and because
    /// a subclass can mean something its base does not: an `enum.IntEnum` is an
    /// `int` whose value is the number it names, not the member itself.
    fn convert_exact_builtin(
        &mut self,
        value: &Bound<'_, PyAny>,
        depth: usize,
    ) -> PyResult<Option<Value>> {
        if value.is_none() {
            return Ok(Some(Value::Null));
        }
        if is_exact_type::<PyBool>(value) {
            return value.extract::<bool>().map(Value::Bool).map(Some);
        }
        if is_exact_type::<PyInt>(value) {
            return integer_to_value(value).map(Some);
        }
        if is_exact_type::<PyFloat>(value) {
            return value
                .extract::<f64>()
                .map(Float::from_f64)
                .map(Value::Float)
                .map(Some);
        }
        if is_exact_type::<PyString>(value) {
            return Ok(Some(Value::String(
                value.cast::<PyString>()?.to_str()?.into(),
            )));
        }
        if is_exact_type::<PyBytes>(value) {
            return Ok(Some(Value::from(value.cast::<PyBytes>()?.as_bytes())));
        }
        if is_exact_type::<PyByteArray>(value) {
            return Ok(Some(Value::from(value.cast::<PyByteArray>()?.to_vec())));
        }
        if is_exact_type::<PyMemoryView>(value) {
            let bytes = value.call_method0("tobytes")?.cast_into::<PyBytes>()?;
            return Ok(Some(Value::from(bytes.as_bytes())));
        }
        if is_exact_type::<PyList>(value) {
            let items = value.cast::<PyList>()?;
            return self
                .convert_sequence(value, items.iter(), depth, false)
                .map(Some);
        }
        if is_exact_type::<PyTuple>(value) {
            let items = value.cast::<PyTuple>()?;
            return self
                .convert_sequence(value, items.iter(), depth, false)
                .map(Some);
        }
        if is_exact_type::<PySet>(value) {
            let items = value.cast::<PySet>()?;
            return self
                .convert_sequence(value, items.iter(), depth, true)
                .map(Some);
        }
        if is_exact_type::<PyFrozenSet>(value) {
            let items = value.cast::<PyFrozenSet>()?;
            return self
                .convert_sequence(value, items.iter(), depth, true)
                .map(Some);
        }
        if is_exact_type::<PyDict>(value) {
            return self.convert_dict(value, depth).map(Some);
        }
        Ok(None)
    }

    /// Convert anything left: a subclass, a dataclass, or a plain object.
    fn convert_other(
        &mut self,
        value: &Bound<'_, PyAny>,
        identity: &str,
        depth: usize,
    ) -> PyResult<Value> {
        if let Some(value) = self.convert_scalar_subclass(value, depth)? {
            return Ok(value);
        }
        if let Some(value) = self.convert_collection_subclass(value, depth)? {
            return Ok(value);
        }
        let py = value.py();
        let dataclasses = py.import("dataclasses")?;
        let is_dataclass = dataclasses
            .getattr("is_dataclass")?
            .call1((value,))?
            .is_truthy()?;
        if is_dataclass && value.cast::<PyType>().is_err() {
            return self.convert_dataclass(value, depth, &dataclasses);
        }
        if let Ok(attributes) = value.getattr("__dict__")
            && let Ok(attributes) = attributes.cast_into::<PyDict>()
        {
            return self.convert_entries(value, &attributes, depth);
        }
        Err(PyTypeError::new_err(format!(
            "unsupported value type {identity}; use a dataclass, mapping, or supported scalar"
        )))
    }

    /// Convert a subclass of a scalar type as the scalar it is.
    ///
    /// Every check here is an `isinstance`, so a subclass keeps its base's
    /// shape instead of decaying into whatever its instance dictionary holds.
    fn convert_scalar_subclass(
        &mut self,
        value: &Bound<'_, PyAny>,
        depth: usize,
    ) -> PyResult<Option<Value>> {
        let py = value.py();
        // An enum member is the value it names: the member's class is the
        // identity being dropped, and its value is what a schema declares.
        if value.is_instance(&py.import("enum")?.getattr("Enum")?)? {
            return self.convert(&value.getattr("value")?, depth + 1).map(Some);
        }
        if value.cast::<PyComplex>().is_ok() {
            return complex_to_value(value).map(Some);
        }
        if let Ok(bytes) = value.cast::<PyByteArray>() {
            return Ok(Some(Value::from(bytes.to_vec())));
        }
        if value.is_instance(&py.import("decimal")?.getattr("Decimal")?)? {
            return decimal_to_value(value).map(Some);
        }
        let datetime = py.import("datetime")?;
        // A `datetime` is a `date`, so the narrower class is asked first.
        if value.is_instance(&datetime.getattr("datetime")?)? {
            return timestamp_to_value(value).map(Some);
        }
        if value.is_instance(&datetime.getattr("date")?)? {
            return date_to_value(value).map(Some);
        }
        if value.is_instance(&datetime.getattr("time")?)? {
            return time_to_value(value).map(Some);
        }
        if value.is_instance(&datetime.getattr("timedelta")?)? {
            return duration_to_value(value).map(Some);
        }
        if value.is_instance_of::<PyInt>() {
            return integer_to_value(value).map(Some);
        }
        if value.is_instance_of::<PyFloat>() {
            return Ok(Some(Value::Float(Float::from_f64(value.extract()?))));
        }
        if let Ok(text) = value.cast::<PyString>() {
            return Ok(Some(Value::String(text.to_str()?.into())));
        }
        if let Ok(bytes) = value.cast::<PyBytes>() {
            return Ok(Some(Value::from(bytes.as_bytes())));
        }
        Ok(None)
    }

    /// Convert a subclass of a collection type as the collection it is.
    fn convert_collection_subclass(
        &mut self,
        value: &Bound<'_, PyAny>,
        depth: usize,
    ) -> PyResult<Option<Value>> {
        let py = value.py();
        if value.is_instance(&py.import("collections")?.getattr("deque")?)? {
            return self.convert_iterator(value, depth).map(Some);
        }
        if let Ok(items) = value.cast::<PyTuple>() {
            // A named tuple names its members, so the mapping is the shape that
            // keeps them; an ordinary tuple has only positions.
            if value.hasattr("_fields")? {
                return self.convert_named_tuple(value, depth).map(Some);
            }
            return self
                .convert_sequence(value, items.iter(), depth, false)
                .map(Some);
        }
        if let Ok(items) = value.cast::<PyList>() {
            return self
                .convert_sequence(value, items.iter(), depth, false)
                .map(Some);
        }
        if let Ok(items) = value.cast::<PySet>() {
            return self
                .convert_sequence(value, items.iter(), depth, true)
                .map(Some);
        }
        if let Ok(items) = value.cast::<PyFrozenSet>() {
            return self
                .convert_sequence(value, items.iter(), depth, true)
                .map(Some);
        }
        if value.cast::<PyDict>().is_ok() {
            return self.convert_dict(value, depth).map(Some);
        }
        Ok(None)
    }

    /// Convert a borrowed run of children into a sequence.
    ///
    /// A set is sorted on the way out, because a document written twice from
    /// one value has to be the same document both times.
    fn convert_sequence<'py, I>(
        &mut self,
        owner: &Bound<'py, PyAny>,
        items: I,
        depth: usize,
        sort: bool,
    ) -> PyResult<Value>
    where
        I: Iterator<Item = Bound<'py, PyAny>>,
    {
        self.with_cycle_check(owner, |encoder| {
            let mut values = items
                .map(|item| encoder.convert(&item, depth + 1))
                .collect::<PyResult<Vec<_>>>()?;
            if sort {
                values.sort_unstable();
            }
            Ok(Value::from_sequence(values))
        })
    }

    /// Convert anything that only knows how to iterate into a sequence.
    fn convert_iterator(&mut self, value: &Bound<'_, PyAny>, depth: usize) -> PyResult<Value> {
        self.with_cycle_check(value, |encoder| {
            let items = value
                .try_iter()?
                .map(|item| encoder.convert(&item?, depth + 1))
                .collect::<PyResult<Vec<_>>>()?;
            Ok(Value::from_sequence(items))
        })
    }

    /// Convert a `range` or a `slice` into its start, stop, and step.
    fn convert_triple(&mut self, value: &Bound<'_, PyAny>, depth: usize) -> PyResult<Value> {
        let values = ["start", "stop", "step"]
            .into_iter()
            .map(|name| self.convert(&value.getattr(name)?, depth + 1))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(Value::from_sequence(values))
    }

    /// Convert any dictionary, including a subclass, into a mapping.
    fn convert_dict(&mut self, value: &Bound<'_, PyAny>, depth: usize) -> PyResult<Value> {
        let entries = value.cast::<PyDict>()?;
        self.convert_entries(value, entries, depth)
    }

    /// Convert one dictionary's entries into a mapping, its keys included.
    fn convert_entries(
        &mut self,
        owner: &Bound<'_, PyAny>,
        entries: &Bound<'_, PyDict>,
        depth: usize,
    ) -> PyResult<Value> {
        self.with_cycle_check(owner, |encoder| {
            let entries = entries
                .iter()
                .map(|(key, item)| {
                    Ok((
                        encoder.convert(&key, depth + 1)?,
                        encoder.convert(&item, depth + 1)?,
                    ))
                })
                .collect::<PyResult<Vec<_>>>()?;
            Value::from_mapping(entries).map_err(value_error)
        })
    }

    /// Convert a named tuple into the mapping its field names describe.
    fn convert_named_tuple(&mut self, value: &Bound<'_, PyAny>, depth: usize) -> PyResult<Value> {
        let names = value.getattr("_fields")?;
        self.with_cycle_check(value, |encoder| {
            let entries = names
                .try_iter()?
                .map(|name| {
                    let name = name?.extract::<String>()?;
                    let item = value.getattr(name.as_str())?;
                    Ok((
                        Value::String(name.as_str().into()),
                        encoder.convert(&item, depth + 1)?,
                    ))
                })
                .collect::<PyResult<Vec<_>>>()?;
            Value::from_mapping(entries).map_err(value_error)
        })
    }

    /// Convert a dataclass instance into the mapping of its declared fields.
    ///
    /// The cached field tuple a Yggdryl record publishes is read first, because
    /// the encode path must not allocate a `dataclasses.fields` tuple per
    /// instance.
    fn convert_dataclass(
        &mut self,
        value: &Bound<'_, PyAny>,
        depth: usize,
        dataclasses: &Bound<'_, PyModule>,
    ) -> PyResult<Value> {
        self.with_cycle_check(value, |encoder| {
            let mut entries = Vec::new();
            let cached_fields = value
                .get_type()
                .getattr("__yggdryl_value_fields__")
                .ok()
                .and_then(|cached| cached.cast_into::<PyTuple>().ok())
                .and_then(|cached| cached.get_item(1).ok())
                .and_then(|fields| fields.cast_into::<PyTuple>().ok());
            if let Some(fields) = cached_fields {
                for field in fields.iter() {
                    push_dataclass_field(encoder, value, &field, depth, &mut entries)?;
                }
            } else {
                let marker = dataclasses.getattr("_FIELD")?;
                let fields = value
                    .get_type()
                    .getattr("__dataclass_fields__")?
                    .cast_into::<PyDict>()?;
                for (_, field) in fields.iter() {
                    if field.getattr("_field_type")?.is(&marker) {
                        push_dataclass_field(encoder, value, &field, depth, &mut entries)?;
                    }
                }
            }
            Value::from_mapping(entries).map_err(value_error)
        })
    }

    /// Run one conversion with this object marked as being on the stack.
    fn with_cycle_check<F>(&mut self, value: &Bound<'_, PyAny>, convert: F) -> PyResult<Value>
    where
        F: FnOnce(&mut Self) -> PyResult<Value>,
    {
        let identity = value.as_ptr() as usize;
        if !self.active.insert(identity) {
            return Err(PyValueError::new_err(
                "cyclic Python values cannot be serialized",
            ));
        }
        let converted = convert(self);
        self.active.remove(&identity);
        converted
    }
}

/// Append one dataclass field's name and converted value.
fn push_dataclass_field(
    encoder: &mut Encoder,
    value: &Bound<'_, PyAny>,
    field: &Bound<'_, PyAny>,
    depth: usize,
    entries: &mut Vec<(Value, Value)>,
) -> PyResult<()> {
    let name = field.getattr("name")?.extract::<String>()?;
    entries.push((
        Value::String(name.as_str().into()),
        encoder.convert(&value.getattr(name.as_str())?, depth + 1)?,
    ));
    Ok(())
}

/// Convert a native Yggdryl wrapper into its canonical text, when it is one.
///
/// Each of these round-trips through its own `from_str`, so the text is the
/// whole value; what a document loses is only which wrapper class held it.
fn native_wrapper_to_value(value: &Bound<'_, PyAny>) -> Option<Value> {
    if let Ok(value) = value.extract::<PyRef<'_, PyDataType>>() {
        return Some(Value::String(value.inner.to_string().into()));
    }
    if let Ok(value) = value.extract::<PyRef<'_, PyField>>() {
        return Some(Value::String(value.inner.to_string().into()));
    }
    if let Ok(value) = value.extract::<PyRef<'_, PyUri>>() {
        return Some(Value::String(value.inner.to_string().into()));
    }
    if let Ok(value) = value.extract::<PyRef<'_, PyUrl>>() {
        return Some(Value::String(value.inner.to_string().into()));
    }
    if let Ok(value) = value.extract::<PyRef<'_, PyUrn>>() {
        return Some(Value::String(value.inner.to_string().into()));
    }
    None
}

/// Convert any path-like object into the string its file system uses.
fn path_to_value(value: &Bound<'_, PyAny>) -> PyResult<Value> {
    let py = value.py();
    let os = py.import("os")?;
    let path = os.getattr("fspath")?.call1((value,))?;
    // A bytes path is still a path, so decoding it with the file system's own
    // rules keeps it readable rather than turning it into a byte blob.
    let path = if path.cast::<PyBytes>().is_ok() {
        os.getattr("fsdecode")?.call1((path,))?
    } else {
        path
    };
    Ok(Value::String(path.cast::<PyString>()?.to_str()?.into()))
}

/// Convert a Python integer into the narrowest native integer that holds it.
fn integer_to_value(value: &Bound<'_, PyAny>) -> PyResult<Value> {
    if let Ok(value) = value.extract::<i64>() {
        return Ok(Value::I64(value));
    }
    if let Ok(value) = value.extract::<u64>() {
        return Ok(Value::U64(value));
    }
    if let Ok(value) = value.extract::<i128>() {
        return Ok(Value::I128(value));
    }
    if let Ok(value) = value.extract::<u128>() {
        return Ok(Value::U128(value));
    }
    // Python's integers are unbounded and no native integer is. The decimal
    // text is the only exact shape left, so the magnitude survives and the type
    // does not: an integer wider than 128 bits reads back as a string.
    Ok(Value::String(value.str()?.to_str()?.into()))
}

/// Convert a complex number into the pair of floats it is.
fn complex_to_value(value: &Bound<'_, PyAny>) -> PyResult<Value> {
    let complex = value.cast::<PyComplex>()?;
    Ok(Value::from_sequence([
        Value::Float(Float::from_f64(complex.real())),
        Value::Float(Float::from_f64(complex.imag())),
    ]))
}

/// Convert a `decimal.Decimal` into the exact coefficient and scale it names.
fn decimal_to_value(value: &Bound<'_, PyAny>) -> PyResult<Value> {
    let parts = value.call_method0("as_tuple")?;
    let Ok(exponent) = parts.getattr("exponent")?.extract::<i32>() else {
        // A non-finite decimal spells its exponent `n`, `N`, or `F`. No exact
        // decimal names an infinity or a NaN, so the float that does is the
        // honest shape and the document says so.
        return Ok(Value::Float(Float::from_f64(value.extract::<f64>()?)));
    };
    let scale = exponent
        .checked_neg()
        .and_then(|scale| i8::try_from(scale).ok())
        .ok_or_else(|| {
            PyOverflowError::new_err(format!(
                "decimal exponent {exponent} has no scale in -128..=127"
            ))
        })?;
    let mut unscaled: i128 = 0;
    for digit in parts.getattr("digits")?.try_iter()? {
        let digit = i128::from(digit?.extract::<u8>()?);
        unscaled = unscaled
            .checked_mul(10)
            .and_then(|unscaled| unscaled.checked_add(digit))
            .ok_or_else(|| {
                PyOverflowError::new_err(
                    "decimal coefficient exceeds the 128 bits an exact decimal holds",
                )
            })?;
    }
    if parts.getattr("sign")?.extract::<i32>()? == 1 {
        unscaled = -unscaled;
    }
    Ok(Value::decimal(unscaled, scale))
}

/// Build the `decimal.Decimal` one coefficient and scale name.
fn decimal_to_python(py: Python<'_>, unscaled: i128, scale: i8) -> PyResult<Py<PyAny>> {
    // `<coefficient>E<-scale>` is exact and keeps the written precision, so a
    // value at scale 2 comes back as `10.50`. `Decimal(unscaled).scaleb(-scale)`
    // would instead round the coefficient at the active context's precision.
    let text = format!("{unscaled}E{}", -i32::from(scale));
    py.import("decimal")?
        .getattr("Decimal")?
        .call1((text,))
        .map(Bound::unbind)
}

/// Convert a `datetime.date` into its day count since the Unix epoch.
fn date_to_value(value: &Bound<'_, PyAny>) -> PyResult<Value> {
    let ordinal = value.call_method0("toordinal")?.extract::<i64>()?;
    let days = i32::try_from(ordinal - EPOCH_ORDINAL)
        .map_err(|_| PyOverflowError::new_err("date is outside the days a date value counts"))?;
    Ok(Value::date(days))
}

/// Build the `datetime.date` one epoch day count names.
fn date_to_python(py: Python<'_>, days: i32) -> PyResult<Py<PyAny>> {
    py.import("datetime")?
        .getattr("date")?
        .getattr("fromordinal")?
        .call1((i64::from(days) + EPOCH_ORDINAL,))
        .map(Bound::unbind)
}

/// Convert a `datetime.time` into its microsecond count since midnight.
///
/// A `tzinfo` and a `fold` on a time of day are dropped: the value model gives
/// a time no zone, and no way to say which reading of a repeated hour it is.
fn time_to_value(value: &Bound<'_, PyAny>) -> PyResult<Value> {
    Ok(Value::time(
        microseconds_of_day(value)?,
        TimeUnit::Microsecond,
    ))
}

/// Build the `datetime.time` one microsecond count since midnight names.
fn time_to_python(py: Python<'_>, value: &Value) -> PyResult<Py<PyAny>> {
    let count = exact_microseconds(value)?;
    if !(0..MICROSECONDS_PER_DAY).contains(&count) {
        return Err(PyValueError::new_err(format!(
            "a time of day must be within one day of midnight, got {count} microseconds"
        )));
    }
    let (hour, minute, second, microsecond) = split_day(count);
    py.import("datetime")?
        .getattr("time")?
        .call1((hour, minute, second, microsecond))
        .map(Bound::unbind)
}

/// Convert a `datetime.timedelta` into its elapsed microsecond count.
fn duration_to_value(value: &Bound<'_, PyAny>) -> PyResult<Value> {
    Ok(Value::duration(
        timedelta_microseconds(value)?,
        TimeUnit::Microsecond,
    ))
}

/// Build the `datetime.timedelta` one elapsed microsecond count names.
fn duration_to_python(py: Python<'_>, value: &Value) -> PyResult<Py<PyAny>> {
    let kwargs = PyDict::new(py);
    kwargs.set_item("microseconds", exact_microseconds(value)?)?;
    py.import("datetime")?
        .getattr("timedelta")?
        .call((), Some(&kwargs))
        .map(Bound::unbind)
}

/// Convert a `datetime.datetime` into a UTC-relative count and its zone.
///
/// The count Arrow defines is always relative to UTC, so an aware value moves
/// by the offset in force at that instant - which is exactly the offset Python
/// computes, daylight saving and `fold` included. A naive value has no offset
/// to apply and carries no zone.
fn timestamp_to_value(value: &Bound<'_, PyAny>) -> PyResult<Value> {
    let days = value.call_method0("toordinal")?.extract::<i64>()? - EPOCH_ORDINAL;
    let time_of_day = microseconds_of_day(value)?;
    let local = days
        .checked_mul(MICROSECONDS_PER_DAY)
        .and_then(|days| days.checked_add(time_of_day))
        .ok_or_else(overflowing_timestamp)?;
    let offset = value.call_method0("utcoffset")?;
    if offset.is_none() {
        return Ok(Value::timestamp_in(local, TimeUnit::Microsecond, None));
    }
    let count = local
        .checked_sub(timedelta_microseconds(&offset)?)
        .ok_or_else(overflowing_timestamp)?;
    let zone = core_timezone_from_value(&value.getattr("tzinfo")?)?;
    Ok(Value::timestamp_in(
        count,
        TimeUnit::Microsecond,
        Some(zone),
    ))
}

/// The error a datetime beyond a 64-bit microsecond count reports.
fn overflowing_timestamp() -> PyErr {
    PyOverflowError::new_err("datetime is outside the microseconds a timestamp counts")
}

/// Build the `datetime.datetime` one UTC-relative count and zone name.
fn timestamp_to_python(py: Python<'_>, value: &Value) -> PyResult<Py<PyAny>> {
    let count = exact_microseconds(value)?;
    let (_, _, zone) = value.as_timestamp_in().ok_or_else(|| {
        PyValueError::new_err(format!("expected a timestamp, got {}", value.kind()))
    })?;
    let datetime = py.import("datetime")?;
    let date = datetime
        .getattr("date")?
        .getattr("fromordinal")?
        .call1((count.div_euclid(MICROSECONDS_PER_DAY) + EPOCH_ORDINAL,))?;
    let (hour, minute, second, microsecond) = split_day(count.rem_euclid(MICROSECONDS_PER_DAY));
    let arguments = (
        date.getattr("year")?.extract::<i64>()?,
        date.getattr("month")?.extract::<i64>()?,
        date.getattr("day")?.extract::<i64>()?,
        hour,
        minute,
        second,
        microsecond,
    );
    let Some(zone) = zone else {
        return datetime
            .getattr("datetime")?
            .call1(arguments)
            .map(Bound::unbind);
    };
    // The count is UTC, so the value is built in UTC and then moved into the
    // zone it was written in. Building it in that zone directly would need the
    // local reading, which is exactly what the offset was taken out of.
    let kwargs = PyDict::new(py);
    kwargs.set_item("tzinfo", datetime.getattr("timezone")?.getattr("utc")?)?;
    let instant = datetime
        .getattr("datetime")?
        .call(arguments, Some(&kwargs))?;
    let tzinfo = zone_to_tzinfo(py, zone, count.div_euclid(1_000_000))?;
    instant
        .call_method1("astimezone", (tzinfo,))
        .map(Bound::unbind)
}

/// Return the Python `tzinfo` that answers for one core zone.
fn zone_to_tzinfo<'py>(
    py: Python<'py>,
    zone: &Timezone,
    epoch_seconds: i64,
) -> PyResult<Bound<'py, PyAny>> {
    let datetime = py.import("datetime")?;
    if zone.is_utc() {
        return datetime.getattr("timezone")?.getattr("utc");
    }
    // A place names rules rather than an offset, and `zoneinfo` is where Python
    // keeps them, so the value comes back carrying the same live zone it went
    // in with rather than a frozen offset.
    if !zone.is_fixed()
        && let Ok(tzinfo) = py
            .import("zoneinfo")?
            .getattr("ZoneInfo")?
            .call1((zone.as_str(),))
    {
        return Ok(tzinfo);
    }
    // A fixed offset, or a platform with no time zone database: this build's
    // own rules answer for the instant, which is all a fixed offset can carry.
    let offset = zone.offset_at(epoch_seconds).ok_or_else(|| {
        PyValueError::new_err(format!(
            "no offset rules for time zone {:?} in this build or in Python's database",
            zone.as_str()
        ))
    })?;
    let delta = datetime.getattr("timedelta")?.call1((0, offset))?;
    datetime.getattr("timezone")?.call1((delta, zone.as_str()))
}

/// Split a microsecond count within one day into its clock fields.
const fn split_day(count: i64) -> (i64, i64, i64, i64) {
    let hour = count / 3_600_000_000;
    let rest = count % 3_600_000_000;
    let minute = rest / 60_000_000;
    let rest = rest % 60_000_000;
    (hour, minute, rest / 1_000_000, rest % 1_000_000)
}

/// Read the microseconds elapsed since midnight from any clock-shaped object.
fn microseconds_of_day(value: &Bound<'_, PyAny>) -> PyResult<i64> {
    let hour = value.getattr("hour")?.extract::<i64>()?;
    let minute = value.getattr("minute")?.extract::<i64>()?;
    let second = value.getattr("second")?.extract::<i64>()?;
    let microsecond = value.getattr("microsecond")?.extract::<i64>()?;
    Ok(((hour * 60 + minute) * 60 + second) * 1_000_000 + microsecond)
}

/// Read a `datetime.timedelta` as one microsecond count.
fn timedelta_microseconds(value: &Bound<'_, PyAny>) -> PyResult<i64> {
    let days = i128::from(value.getattr("days")?.extract::<i64>()?);
    let seconds = i128::from(value.getattr("seconds")?.extract::<i64>()?);
    let microseconds = i128::from(value.getattr("microseconds")?.extract::<i64>()?);
    // A Python timedelta spans about 2.7 million years and a 64-bit microsecond
    // count spans about 292 thousand, so the wide arithmetic is what makes the
    // overflow reportable instead of wrapping.
    let total = (days * 86_400 + seconds) * 1_000_000 + microseconds;
    i64::try_from(total).map_err(|_| {
        PyOverflowError::new_err("timedelta exceeds the microseconds a duration counts")
    })
}

/// Restate one temporal at Python's microsecond resolution, or refuse.
fn exact_microseconds(value: &Value) -> PyResult<i64> {
    value.temporal_count_at(TimeUnit::Microsecond).ok_or_else(|| {
        PyValueError::new_err(format!(
            "a {} at this resolution has no exact microsecond count, which is all datetime holds",
            value.kind()
        ))
    })
}

/// Return the dotted `module.qualname` of a value's class.
fn type_identity(value: &Bound<'_, PyAny>) -> PyResult<String> {
    let value_type = value.get_type();
    let module = value_type.getattr("__module__")?.extract::<String>()?;
    let qualname = value_type.getattr("__qualname__")?.extract::<String>()?;
    Ok(format!("{module}.{qualname}"))
}

/// Return whether a value's class is exactly `T` rather than a subclass of it.
fn is_exact_type<T: PyTypeInfo>(value: &Bound<'_, PyAny>) -> bool {
    value.get_type().is(value.py().get_type::<T>())
}
