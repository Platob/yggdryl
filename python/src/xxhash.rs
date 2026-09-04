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

use pyo3::buffer::PyBuffer;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyString, PyType};

use yggdryl::xxhash::{Xxh3_64, Xxh3_128, Xxh32, Xxh64};
use yggdryl::{Digest, DigestAlgorithm};

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
    module.add_class::<PyXxh3_64>()?;
    module.add_class::<PyXxh3_128>()?;
    module.add_function(pyo3::wrap_pyfunction!(xxh32, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(xxh64, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(xxh3_64, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(xxh3_128, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(xxhash_digest, module)?)?;
    module.add_function(pyo3::wrap_pyfunction!(secret_minimum_length, module)?)?;
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
fn feed_content(value: &Bound<'_, PyAny>, sink: &mut impl FnMut(&[u8])) -> PyResult<()> {
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
#[pyo3(name = "xxh3_64", signature = (data, seed = 0, secret = None))]
pub(crate) fn xxh3_64(data: &Bound<'_, PyAny>, seed: u64, secret: Option<&[u8]>) -> PyResult<u64> {
    let mut state = match secret {
        Some(secret) => Xxh3_64::from_seed_and_secret(seed, secret).map_err(value_error)?,
        None => Xxh3_64::with_seed(seed),
    };
    feed_content(data, &mut |bytes| state.write_bytes(bytes))?;
    Ok(state.as_u64())
}

/// Digest a complete value with XXH3, answering 128 bits.
#[pyfunction]
#[pyo3(name = "xxh3_128", signature = (data, seed = 0, secret = None))]
pub(crate) fn xxh3_128(
    data: &Bound<'_, PyAny>,
    seed: u64,
    secret: Option<&[u8]>,
) -> PyResult<u128> {
    let mut state = match secret {
        Some(secret) => Xxh3_128::from_seed_and_secret(seed, secret).map_err(value_error)?,
        None => Xxh3_128::with_seed(seed),
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

            /// Answer the digest of everything fed so far.
            ///
            /// Answering never consumes the state, so a running digest can be
            /// read at every commit boundary rather than only at the end.
            fn as_digest(&self) -> PyDigest {
                PyDigest::from_core(self.inner.as_digest())
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
    PyXxh3_64,
    "Xxh3_64",
    Xxh3_64,
    u64,
    DigestAlgorithm::Xxh3_64,
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
                Some(secret) => Xxh3_64::from_seed_and_secret(seed, secret).map_err(value_error)?,
                None => Xxh3_64::with_seed(seed),
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
    PyXxh3_128,
    "Xxh3_128",
    Xxh3_128,
    u64,
    DigestAlgorithm::Xxh3_128,
    "A resumable XXH3 state answering 128 bits.",
    {
        /// Start a state, optionally seeded and with a custom secret.
        ///
        /// A secret shorter than `SECRET_MINIMUM_LENGTH` is rejected by length
        /// whatever the payload, for the reason `Xxh3_64` states.
        #[new]
        #[pyo3(signature = (seed = 0, secret = None))]
        fn new(seed: u64, secret: Option<&[u8]>) -> PyResult<Self> {
            let inner = match secret {
                Some(secret) => {
                    Xxh3_128::from_seed_and_secret(seed, secret).map_err(value_error)?
                }
                None => Xxh3_128::with_seed(seed),
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
