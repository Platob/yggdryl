//! xxHash digests over bytes, values, and handles.
//!
//! One-shot functions answer a plain `int` at the algorithm's native width, so
//! a caller who wants a number gets one with no wrapper around it. The four
//! streaming classes are the resumable states, and [`PyDigest`] is the value
//! that carries its algorithm with it - which is what keeps `xxh64` and
//! `xxh3-64`, both 64 bits wide, from being confused for one another.
//!
//! Input is read in place wherever Python holds contiguous bytes: a `bytes` or
//! a `str` is borrowed, and any other buffer - `bytearray`, `memoryview`, an
//! Arrow buffer - is read through one bounded window, so nothing allocates
//! proportionally to the payload.

use arrow_array::RecordBatch as ArrowRecordBatch;
use arrow_pyarrow::{FromPyArrow, ToPyArrow};
use pyo3::buffer::PyBuffer;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyString, PyType};

use yggdryl::xxhash::{Xxh3, Xxh32, Xxh64, Xxh128};
use yggdryl::{Digest, DigestAlgorithm};

use crate::text::codec::PythonReader;
use crate::types::datatype::{arrow_array_from_pyarrow, arrow_array_to_pyarrow};
use crate::types::field::core_field_from_value;
use crate::types::scalar::PyScalar;
use crate::value_error;

/// Register this module's classes and functions on the native module.
///
/// The registration lives beside the surface it names rather than in the
/// module root, so adding a class here cannot be forgotten there.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyDigest>()?;
    module.add_class::<PyXxh32>()?;
    module.add_class::<PyXxh64>()?;
    module.add_class::<PyXxh3>()?;
    module.add_class::<PyXxh128>()?;
    module.add_function(pyo3::wrap_pyfunction!(xxh32, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(xxh64, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(xxh3, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(xxh128, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(xxhash_digest, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(secret_minimum_length, module)?)?;
    module.add_class::<PyDigester>()?;
    module.add_function(pyo3::wrap_pyfunction!(row_digests, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(column_digests, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(is_secretable, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(is_seedable, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(algorithm_width, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(algorithm_bits, module)?)?;
    Ok(())
}

/// The bounded window a buffer that cannot be borrowed is read through.
const WINDOW: usize = 64 * 1024;

/// What a digest entry point refuses.
const CONTENT_REFUSAL: &str =
    "digest content must be str, bytes, or an object supporting the buffer protocol";

/// Feed the bytes `value` holds into `sink`, borrowing where Python does.
///
/// A `bytes` and a `str` are contiguous and immutable, so the slice handed on
/// is the object's own storage. Everything else goes through the buffer
/// protocol one window at a time: the payload is never copied whole, so
/// hashing a gigabyte `memoryview` costs one 64 KiB window.
pub(crate) fn feed_content(value: &Bound<'_, PyAny>, sink: &mut impl FnMut(&[u8])) -> PyResult<()> {
    if let Ok(text) = value.cast::<PyString>() {
        sink(text.to_str()?.as_bytes());
        return Ok(());
    }
    if let Ok(bytes) = value.cast::<PyBytes>() {
        sink(bytes.as_bytes());
        return Ok(());
    }
    let buffer = PyBuffer::<u8>::get(value).map_err(|_| PyTypeError::new_err(CONTENT_REFUSAL))?;
    let py = value.py();
    let cells = buffer
        .as_slice(py)
        .ok_or_else(|| PyValueError::new_err("digest content must be a contiguous buffer"))?;
    // Boxed rather than an array on the stack: 64 KiB is a page cache's worth
    // of stack, and this window lives for the whole feed.
    let mut window = vec![0_u8; WINDOW].into_boxed_slice();
    for chunk in cells.chunks(WINDOW) {
        for (slot, cell) in window.iter_mut().zip(chunk) {
            *slot = cell.get();
        }
        sink(&window[..chunk.len()]);
    }
    Ok(())
}

/// Parse an algorithm token, keeping the core's message.
/// Feed a Python readable to exhaustion through one bounded window.
///
/// The reader is drained by the core, which reuses one stream-sized window,
/// so memory stays flat in the source's length. A Python-side failure is
/// re-raised as itself rather than as the generic stream error wrapping it.
fn feed_reader(
    source: &Bound<'_, PyAny>,
    sink: &mut impl FnMut(&mut PythonReader<'_>) -> yggdryl::Result<u64>,
) -> PyResult<u64> {
    let mut reader = PythonReader::new(source);
    let written = sink(&mut reader);
    if let Some(error) = reader.take_error() {
        return Err(error);
    }
    written.map_err(value_error)
}

pub(crate) fn algorithm_from_str(value: &str) -> PyResult<DigestAlgorithm> {
    DigestAlgorithm::from_str(value).map_err(value_error)
}

/// The payload of a digest as one Python integer.
fn payload(digest: Digest) -> u128 {
    let bytes = digest.into_bytes();
    let mut wide = [0_u8; 16];
    wide[16 - bytes.len()..].copy_from_slice(&bytes);
    u128::from_be_bytes(wide)
}

/// Digest a complete value with XXH32.
#[pyfunction]
#[pyo3(name = "xxh32", signature = (data, seed = 0))]
pub(crate) fn xxh32(data: &Bound<'_, PyAny>, seed: u32) -> PyResult<u32> {
    let mut state = Xxh32::with_seed(seed);
    feed_content(data, &mut |bytes| state.write_bytes(bytes))?;
    Ok(state.as_u32())
}

/// Digest a complete value with XXH64.
#[pyfunction]
#[pyo3(name = "xxh64", signature = (data, seed = 0))]
pub(crate) fn xxh64(data: &Bound<'_, PyAny>, seed: u64) -> PyResult<u64> {
    let mut state = Xxh64::with_seed(seed);
    feed_content(data, &mut |bytes| state.write_bytes(bytes))?;
    Ok(state.as_u64())
}

/// Digest a complete value with XXH3, answering 64 bits.
#[pyfunction]
#[pyo3(name = "xxh3", signature = (data, seed = 0, secret = None))]
pub(crate) fn xxh3(data: &Bound<'_, PyAny>, seed: u64, secret: Option<&[u8]>) -> PyResult<u64> {
    let mut state = match secret {
        Some(secret) => Xxh3::from_seed_and_secret(seed, secret).map_err(value_error)?,
        None => Xxh3::with_seed(seed),
    };
    feed_content(data, &mut |bytes| state.write_bytes(bytes))?;
    Ok(state.as_u64())
}

/// Digest a complete value with XXH3, answering 128 bits.
#[pyfunction]
#[pyo3(name = "xxh128", signature = (data, seed = 0, secret = None))]
pub(crate) fn xxh128(data: &Bound<'_, PyAny>, seed: u64, secret: Option<&[u8]>) -> PyResult<u128> {
    let mut state = match secret {
        Some(secret) => Xxh128::from_seed_and_secret(seed, secret).map_err(value_error)?,
        None => Xxh128::with_seed(seed),
    };
    feed_content(data, &mut |bytes| state.write_bytes(bytes))?;
    Ok(state.as_u128())
}

/// Digest a complete value, carrying the algorithm with the answer.
#[pyfunction]
#[pyo3(name = "xxhash_digest")]
pub(crate) fn xxhash_digest(data: &Bound<'_, PyAny>, algorithm: &str) -> PyResult<PyDigest> {
    let algorithm = algorithm_from_str(algorithm)?;
    let mut digester = algorithm.digester();
    feed_content(data, &mut |bytes| digester.write_bytes(bytes))?;
    Ok(PyDigest::from_core(digester.as_digest()))
}

/// The shortest custom secret XXH3 accepts, in bytes.
#[pyfunction]
#[pyo3(name = "xxhash_secret_minimum_length")]
pub(crate) const fn secret_minimum_length() -> usize {
    yggdryl::xxhash::SECRET_MINIMUM_LENGTH
}

/// One digest: the algorithm that produced it and the value it produced.
///
/// Two digests of different algorithms are never equal, whatever their
/// payloads. `int(digest)` is the native value, `bytes(digest)` the canonical
/// big-endian representation, and `str(digest)` the `<algorithm>:<hex>`
/// spelling the constructor reads back.
#[pyclass(
    name = "Digest",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyDigest {
    inner: Digest,
}

impl PyDigest {
    pub(crate) const fn from_core(inner: Digest) -> Self {
        Self { inner }
    }

    /// The native digest this wrapper carries.
    pub(crate) const fn inner(&self) -> Digest {
        self.inner
    }
}

#[pymethods]
impl PyDigest {
    /// Parse the canonical `<algorithm>:<hex>` spelling.
    #[new]
    fn new(value: &str) -> PyResult<Self> {
        Digest::from_str(value)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// Rebuild a digest from its canonical big-endian bytes.
    #[classmethod]
    fn from_bytes(_cls: &Bound<'_, PyType>, algorithm: &str, data: &[u8]) -> PyResult<Self> {
        let algorithm = algorithm_from_str(algorithm)?;
        Digest::from_bytes(algorithm, data)
            .map(Self::from_core)
            .map_err(value_error)
    }

    /// Build a digest from an algorithm and its native value.
    #[classmethod]
    fn from_int(_cls: &Bound<'_, PyType>, algorithm: &str, value: u128) -> PyResult<Self> {
        let algorithm = algorithm_from_str(algorithm)?;
        Ok(Self::from_core(Digest::new(algorithm, value)))
    }

    /// The canonical algorithm token.
    #[getter]
    fn algorithm(&self) -> &'static str {
        self.inner.algorithm().as_str()
    }

    /// The digest width in bytes.
    #[getter]
    fn width(&self) -> usize {
        self.inner.algorithm().width()
    }

    /// The digest width in bits.
    #[getter]
    fn bits(&self) -> u32 {
        self.inner.algorithm().bits()
    }

    /// A deterministic cross-language hash of this digest.
    fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    fn __int__(&self) -> u128 {
        payload(self.inner)
    }

    fn __index__(&self) -> u128 {
        payload(self.inner)
    }

    fn __bytes__<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.inner.into_bytes())
    }

    fn __len__(&self) -> usize {
        self.inner.algorithm().width()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!("Digest({:?})", self.inner.to_string())
    }

    fn __hash__(&self) -> isize {
        crate::python_hash(self.inner.stable_hash())
    }

    fn __richcmp__(
        &self,
        other: &Bound<'_, PyAny>,
        operation: pyo3::basic::CompareOp,
    ) -> PyResult<Py<PyAny>> {
        let py = other.py();
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return Ok(py.NotImplemented());
        };
        Ok(crate::compare(self.inner.cmp(&other.inner), operation)
            .into_pyobject(py)?
            .to_owned()
            .into_any()
            .unbind())
    }

    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, (String,)) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (self.inner.to_string(),),
        )
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// Declare one streaming state class over one core state.
///
/// The four algorithms differ in the width of their seed and in whether they
/// accept a custom secret, and nothing else: the feed, the answer, and the
/// reset are the same three operations. The constructor - the only part that
/// really differs - is passed in; naming the rest once is what keeps the four
/// classes from drifting apart.
macro_rules! state {
    (
        $class:ident,
        $name:literal,
        $core:ty,
        $seed:ty,
        $algorithm:expr,
        $doc:literal,
        { $($constructor:tt)* }
    ) => {
        #[doc = $doc]
        #[pyclass(name = $name, module = "yggdryl._native", skip_from_py_object)]
        #[derive(Clone)]
        pub(crate) struct $class {
            inner: $core,
        }

        impl $class {
            /// This state as the runtime dispatcher, seed, secret, and fed
            /// bytes included.
            pub(crate) fn digester(&self) -> yggdryl::Digester {
                self.inner.clone().into()
            }
        }

        #[pymethods]
        impl $class {
            $($constructor)*

            /// The canonical algorithm token.
            #[getter]
            fn algorithm(&self) -> &'static str {
                $algorithm.as_str()
            }

            /// The seed this state was constructed with.
            #[getter]
            fn seed(&self) -> $seed {
                self.inner.seed()
            }

            /// Feed raw bytes, a string, or any buffer.
            fn write_bytes(&mut self, data: &Bound<'_, PyAny>) -> PyResult<()> {
                let inner = &mut self.inner;
                feed_content(data, &mut |bytes| inner.write_bytes(bytes))
            }

            /// Feed one value's canonical byte representation.
            fn write_scalar(&mut self, value: &PyScalar) {
                self.inner.write_scalar(&value.inner);
            }

            /// Fill default digest holders in one `PyArrow` `RecordBatch`.
            ///
            /// The root Field owns holder/component/path metadata. Existing
            /// non-default holders are retained, and this running state is not
            /// consumed or reset.
            #[pyo3(signature = (root, batch, *, force=false))]
            fn apply_arrow_batch<'py>(
                &self,
                py: Python<'py>,
                root: &Bound<'py, PyAny>,
                batch: &Bound<'py, PyAny>,
                force: bool,
            ) -> PyResult<Bound<'py, PyAny>> {
                let root = core_field_from_value(root)?;
                let batch = ArrowRecordBatch::from_pyarrow_bound(batch)?;
                self.inner
                    .apply_arrow_batch(&root, batch, force)
                    .map_err(value_error)?
                    .to_pyarrow(py)
            }

            /// Answer the digest of everything fed so far.
            ///
            /// Answering never consumes the state, so a running digest can be
            /// read at every commit boundary rather than only at the end.
            fn as_digest(&self) -> PyDigest {
                PyDigest::from_core(self.inner.as_digest())
            }

            /// Feed a readable to exhaustion, answering the bytes consumed.
            ///
            /// The source is drained through one reused window, so hashing a
            /// stream costs the window rather than the payload. A failure
            /// part way through leaves the bytes already fed in the state.
            fn write_reader(&mut self, source: &Bound<'_, PyAny>) -> PyResult<u64> {
                let inner = &mut self.inner;
                feed_reader(source, &mut |reader| inner.write_reader(reader))
            }

            /// The native-width number of everything fed so far.
            ///
            /// `as_digest` answers the same value carrying its algorithm;
            /// this is the plain integer beside it.
            fn as_int(&self) -> u128 {
                payload(self.inner.as_digest())
            }

            /// Reset to the constructed seed and secret, not to a fresh state.
            fn clear(&mut self) {
                self.inner.clear();
            }

            fn __repr__(&self) -> String {
                format!("{}(seed={})", $name, self.inner.seed())
            }

            // A state's answer changes as it is fed, so it has no stable hash.
            #[classattr]
            const __hash__: Option<Py<PyAny>> = None;

            fn __copy__(&self) -> Self {
                self.clone()
            }

            fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
                self.clone()
            }
        }
    };
}

state!(
    PyXxh32,
    "Xxh32",
    Xxh32,
    u32,
    DigestAlgorithm::Xxh32,
    "A resumable XXH32 state.",
    {
        /// Start a state, optionally seeded.
        ///
        /// XXH32 takes a seed and never a secret; only the XXH3 pair is
        /// secretable.
        #[new]
        #[pyo3(signature = (seed = 0))]
        fn new(seed: u32) -> Self {
            Self {
                inner: Xxh32::with_seed(seed),
            }
        }
    }
);
state!(
    PyXxh64,
    "Xxh64",
    Xxh64,
    u64,
    DigestAlgorithm::Xxh64,
    "A resumable XXH64 state.",
    {
        /// Start a state, optionally seeded.
        ///
        /// XXH64 takes a seed and never a secret; only the XXH3 pair is
        /// secretable.
        #[new]
        #[pyo3(signature = (seed = 0))]
        fn new(seed: u64) -> Self {
            Self {
                inner: Xxh64::with_seed(seed),
            }
        }
    }
);
state!(
    PyXxh3,
    "Xxh3",
    Xxh3,
    u64,
    DigestAlgorithm::Xxh3,
    "A resumable XXH3 state answering 64 bits.",
    {
        /// Start a state, optionally seeded and with a custom secret.
        ///
        /// A secret shorter than `SECRET_MINIMUM_LENGTH` is rejected by length
        /// whatever the payload: the reference only consults a secret past its
        /// 240-byte cutoff, and a secret that is sometimes used is worse than
        /// one that is refused.
        #[new]
        #[pyo3(signature = (seed = 0, secret = None))]
        fn new(seed: u64, secret: Option<&[u8]>) -> PyResult<Self> {
            let inner = match secret {
                Some(secret) => Xxh3::from_seed_and_secret(seed, secret).map_err(value_error)?,
                None => Xxh3::with_seed(seed),
            };
            Ok(Self { inner })
        }

        /// The custom secret this state was constructed with, if any.
        #[getter]
        fn secret<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
            self.inner.secret().map(|secret| PyBytes::new(py, secret))
        }
    }
);
state!(
    PyXxh128,
    "Xxh128",
    Xxh128,
    u64,
    DigestAlgorithm::Xxh128,
    "A resumable XXH3 state answering 128 bits.",
    {
        /// Start a state, optionally seeded and with a custom secret.
        ///
        /// A secret shorter than `SECRET_MINIMUM_LENGTH` is rejected by length
        /// whatever the payload, for the reason `Xxh3` states.
        #[new]
        #[pyo3(signature = (seed = 0, secret = None))]
        fn new(seed: u64, secret: Option<&[u8]>) -> PyResult<Self> {
            let inner = match secret {
                Some(secret) => Xxh128::from_seed_and_secret(seed, secret).map_err(value_error)?,
                None => Xxh128::with_seed(seed),
            };
            Ok(Self { inner })
        }

        /// The custom secret this state was constructed with, if any.
        #[getter]
        fn secret<'py>(&self, py: Python<'py>) -> Option<Bound<'py, PyBytes>> {
            self.inner.secret().map(|secret| PyBytes::new(py, secret))
        }
    }
);

/// A streaming digest state whose algorithm is chosen at run time.
///
/// This is to the algorithm token what the four named states are to the four
/// algorithms: one place a value like `"xxh3-64"` read from a configuration
/// becomes a running state. A caller who writes the algorithm as a literal
/// uses the named class and pays no dispatch.
#[pyclass(name = "Digester", module = "yggdryl._native", skip_from_py_object)]
#[derive(Clone)]
pub(crate) struct PyDigester {
    inner: yggdryl::Digester,
}

impl PyDigester {
    /// The native dispatcher this wrapper carries.
    pub(crate) fn digester(&self) -> yggdryl::Digester {
        self.inner.clone()
    }
}

#[pymethods]
impl PyDigester {
    /// Start a state for one algorithm, optionally seeded.
    ///
    /// The seed is one shape for four algorithms, so XXH32 - whose seed is
    /// 32 bits wide - uses its low half. An algorithm that takes a custom
    /// secret is constructed through its own class, which is where a secret
    /// can be spelled.
    #[new]
    #[pyo3(signature = (algorithm = "xxh3-64", *, seed = None))]
    fn new(algorithm: &str, seed: Option<u64>) -> PyResult<Self> {
        let algorithm = algorithm_from_str(algorithm)?;
        Ok(Self {
            inner: match seed {
                Some(seed) => algorithm.digester_with_seed(seed),
                None => algorithm.digester(),
            },
        })
    }

    /// The canonical algorithm token this state computes.
    #[getter]
    fn algorithm(&self) -> &'static str {
        self.inner.algorithm().as_str()
    }

    /// Feed raw bytes, a string, or any buffer.
    fn write_bytes(&mut self, data: &Bound<'_, PyAny>) -> PyResult<()> {
        let inner = &mut self.inner;
        feed_content(data, &mut |bytes| inner.write_bytes(bytes))
    }

    /// Feed one value's canonical byte representation.
    fn write_scalar(&mut self, value: &PyScalar) {
        self.inner.write_scalar(&value.inner);
    }

    /// Feed a readable to exhaustion, answering the bytes consumed.
    fn write_reader(&mut self, source: &Bound<'_, PyAny>) -> PyResult<u64> {
        let inner = &mut self.inner;
        feed_reader(source, &mut |reader| inner.write_reader(reader))
    }

    /// Fill default digest holders in one `PyArrow` `RecordBatch`.
    #[pyo3(signature = (root, batch, *, force = false))]
    fn apply_arrow_batch<'py>(
        &self,
        py: Python<'py>,
        root: &Bound<'py, PyAny>,
        batch: &Bound<'py, PyAny>,
        force: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let root = core_field_from_value(root)?;
        let batch = ArrowRecordBatch::from_pyarrow_bound(batch)?;
        self.inner
            .apply_arrow_batch(&root, batch, force)
            .map_err(value_error)?
            .to_pyarrow(py)
    }

    /// Answer the digest of everything fed so far, without consuming it.
    fn as_digest(&self) -> PyDigest {
        PyDigest::from_core(self.inner.as_digest())
    }

    /// The native-width number of everything fed so far.
    fn as_int(&self) -> u128 {
        payload(self.inner.as_digest())
    }

    /// Reset to the constructed seed, not to a fresh state.
    fn clear(&mut self) {
        self.inner.clear();
    }

    fn __repr__(&self) -> String {
        format!("Digester({:?})", self.inner.algorithm().as_str())
    }

    // A state's answer changes as it is fed, so it has no stable hash.
    #[classattr]
    const __hash__: Option<Py<PyAny>> = None;

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }
}

/// Digest every row of one `PyArrow` `RecordBatch`.
///
/// A row is the ordered sequence of its non-holder columns in schema order,
/// framed as a sequence so a two-column row and a nested one never collide.
/// The answer is the exact Arrow width the algorithm names: `uint32` for
/// XXH32, `uint64` for the two 64-bit algorithms, `fixed_size_binary(16)`
/// for XXH128.
#[pyfunction]
#[pyo3(name = "xxhash_row_digests", signature = (batch, algorithm = "xxh3-64"))]
pub(crate) fn row_digests<'py>(
    py: Python<'py>,
    batch: &Bound<'py, PyAny>,
    algorithm: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let batch = ArrowRecordBatch::from_pyarrow_bound(batch)?;
    let digests = yggdryl::xxhash::arrow::row_digests(&batch, algorithm_from_str(algorithm)?)
        .map_err(value_error)?;
    arrow_array_to_pyarrow(py, &digests, None)
}

/// Digest every cell of one `PyArrow` array under the field that declares it.
///
/// There is no row framing here, so a column digest is the value's own feed;
/// a null feeds the null tag, which is what keeps a null and an empty string
/// apart. The array is reconciled to the field first, because the answer is
/// the value model's rather than the layout's - an `int32` column read under
/// an `int64` declaration is the same numbers - and strictly, so a value the
/// declaration cannot hold is named rather than nulled into a digest.
#[pyfunction]
#[pyo3(name = "xxhash_column_digests", signature = (array, field, algorithm = "xxh3-64"))]
pub(crate) fn column_digests<'py>(
    py: Python<'py>,
    array: &Bound<'py, PyAny>,
    field: &Bound<'py, PyAny>,
    algorithm: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let values = arrow_array_from_pyarrow(array)?;
    let field = core_field_from_value(field)?;
    let digests =
        yggdryl::xxhash::arrow::column_digests(values, &field, algorithm_from_str(algorithm)?)
            .map_err(value_error)?;
    arrow_array_to_pyarrow(py, &digests, None)
}

/// Return whether an algorithm accepts a custom secret.
#[pyfunction]
#[pyo3(name = "xxhash_is_secretable")]
pub(crate) fn is_secretable(algorithm: &str) -> PyResult<bool> {
    Ok(algorithm_from_str(algorithm)?.is_secretable())
}

/// Return whether an algorithm accepts a seed.
#[pyfunction]
#[pyo3(name = "xxhash_is_seedable")]
pub(crate) fn is_seedable(algorithm: &str) -> PyResult<bool> {
    Ok(algorithm_from_str(algorithm)?.is_seedable())
}

/// The digest width of an algorithm, in bytes.
#[pyfunction]
#[pyo3(name = "xxhash_width")]
pub(crate) fn algorithm_width(algorithm: &str) -> PyResult<usize> {
    Ok(algorithm_from_str(algorithm)?.width())
}

/// The digest width of an algorithm, in bits.
#[pyfunction]
#[pyo3(name = "xxhash_bits")]
pub(crate) fn algorithm_bits(algorithm: &str) -> PyResult<u32> {
    Ok(algorithm_from_str(algorithm)?.bits())
}
