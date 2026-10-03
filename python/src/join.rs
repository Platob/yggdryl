//! Python's native view of the shared [`JoinOptions`]: the facts beside a
//! join's keys and kind, and the one door every `join_with` reads its
//! arguments through.
//!
//! [`PyJoinOptions`] owns only the core value and is immutable, so it
//! compares, hashes and pickles by what the core's equality reads. A join's
//! `how` and a build side are words the core's own parsers read, its keys
//! one `Scalar` the core's key reader reads ([`JoinKeys::from_scalar`]), so
//! nothing here parses a key or a kind.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use pyo3::prelude::*;
use pyo3::types::PyBool;
use yggdryl::expression::JoinKeys;
use yggdryl::{DEFAULT_JOIN_SUFFIX, DEFAULT_PUSHDOWN_KEYS, JoinKind, JoinOptions, JoinSide};

use crate::scalar::from_py;
use crate::spill::{PySpillOptions, hash_spill_options};
use crate::{python_hash, value_error};

/// The facts beside a join's keys and kind. Immutable.
#[pyclass(
    name = "JoinOptions",
    module = "yggdryl._native",
    frozen,
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct PyJoinOptions {
    pub(crate) inner: JoinOptions,
}

impl PyJoinOptions {
    /// Wrap a value the core built.
    pub(crate) const fn from_core(inner: JoinOptions) -> Self {
        Self { inner }
    }
}

/// The keys a join takes, resolved once: any value `Scalar` holds, read as
/// the core reads one - the text of a key list (`"id, venue = market"`), a
/// list of key texts or of `[left, right]` term pairs, or a mapping of left
/// terms to right terms.
pub(crate) fn join_keys_of(by: &Bound<'_, PyAny>) -> PyResult<JoinKeys> {
    JoinKeys::from_scalar(&from_py(by)?).map_err(value_error)
}

/// The kind a join's `how` names, as the core reads the word.
pub(crate) fn join_kind_of(how: &str) -> PyResult<JoinKind> {
    JoinKind::from_str(how).map_err(value_error)
}

/// The keywords every `join_with` takes beside `options`, each `...` when
/// it was not given.
pub(crate) struct JoinKeywords {
    pub(crate) coalesce: Py<PyAny>,
    pub(crate) suffix: Py<PyAny>,
    pub(crate) build: Py<PyAny>,
    pub(crate) prune: Py<PyAny>,
    pub(crate) spill: Py<PyAny>,
    pub(crate) pushdown_keys: Py<PyAny>,
}

impl JoinKeywords {
    /// `options` - else `JoinOptions()` - with every keyword given set on a
    /// copy by its own setter: `...` is a keyword not given, and `None` is a
    /// value, which clears `build` and `spill`.
    pub(crate) fn resolve(
        self,
        py: Python<'_>,
        options: Option<&PyJoinOptions>,
    ) -> PyResult<JoinOptions> {
        let ellipsis = py.Ellipsis();
        let given = |value: &Py<PyAny>| {
            let value = value.bind(py);
            (!value.is(&ellipsis)).then(|| value.clone())
        };
        let mut options = options.map_or_else(JoinOptions::new, |options| options.inner.clone());
        if let Some(coalesce) = given(&self.coalesce) {
            options = options.with_coalesce(coalesce.extract()?);
        }
        if let Some(suffix) = given(&self.suffix) {
            options = options.with_suffix(suffix.extract::<String>()?);
        }
        if let Some(build) = given(&self.build) {
            options = options.with_build(build_of(build.extract()?)?);
        }
        if let Some(prune) = given(&self.prune) {
            options = options.with_prune(prune.extract()?);
        }
        if let Some(spill) = given(&self.spill) {
            options = with_spill(options, spill.extract()?);
        }
        if let Some(keys) = given(&self.pushdown_keys) {
            options = options.with_pushdown_keys(keys.extract()?);
        }
        Ok(options)
    }
}

/// The build side a word names, as the core reads it, `None` for neither.
fn build_of(build: Option<&str>) -> PyResult<Option<JoinSide>> {
    build
        .map(|word| JoinSide::from_str(word).map_err(value_error))
        .transpose()
}

/// `options` settling under `spill`, or under the process default for
/// `None`.
fn with_spill(options: JoinOptions, spill: Option<PyRef<'_, PySpillOptions>>) -> JoinOptions {
    match spill {
        Some(spill) => options.with_spill(spill.inner.clone()),
        // The core states a bound and never clears one, so every other fact
        // is carried onto options that state none.
        None => JoinOptions::new()
            .with_coalesce(options.coalesce())
            .with_suffix(options.suffix())
            .with_build(options.build())
            .with_prune(options.prune())
            .with_pushdown_keys(options.pushdown_keys()),
    }
}

#[pymethods]
impl PyJoinOptions {
    /// Coalesce a key stated as one bare column on both sides into one
    /// column; suffix a colliding right name with `suffix`; hold and hash
    /// the `build` side - `"left"`, `"right"`, or `None` to pick the held
    /// side over a stream, else the smaller; drop probe rows the build keys
    /// cannot match when `prune`; settle under `spill`, the process default
    /// for `None`; push up to `pushdown_keys` keys into a probe source.
    #[new]
    #[pyo3(
        signature = (
            coalesce = true,
            suffix = DEFAULT_JOIN_SUFFIX,
            build = None,
            prune = true,
            spill = None,
            pushdown_keys = DEFAULT_PUSHDOWN_KEYS,
        ),
        // The core's defaults, spelled for `inspect`: an expression default
        // renders as `...`. The binding's tests pin them to the constants.
        text_signature = "(coalesce=True, suffix='_right', build=None, prune=True, spill=None, pushdown_keys=10000)"
    )]
    fn new(
        coalesce: bool,
        suffix: &str,
        build: Option<&str>,
        prune: bool,
        spill: Option<PyRef<'_, PySpillOptions>>,
        pushdown_keys: usize,
    ) -> PyResult<Self> {
        let options = JoinOptions::new()
            .with_coalesce(coalesce)
            .with_suffix(suffix)
            .with_build(build_of(build)?)
            .with_prune(prune)
            .with_pushdown_keys(pushdown_keys);
        Ok(Self::from_core(with_spill(options, spill)))
    }

    /// Whether a key stated as the same bare column on both sides appears
    /// once, under the left name, left value else right.
    #[getter]
    fn coalesce(&self) -> bool {
        self.inner.coalesce()
    }

    /// What a right column whose name collides with a left one is suffixed
    /// with.
    #[getter]
    fn suffix(&self) -> &str {
        self.inner.suffix()
    }

    /// Which side is held and hashed: `"left"`, `"right"`, or `None` to
    /// pick.
    #[getter]
    fn build(&self) -> Option<&'static str> {
        self.inner.build().map(JoinSide::as_str)
    }

    /// Whether probe rows and batches the build keys cannot match are
    /// dropped before they are hashed.
    #[getter]
    fn prune(&self) -> bool {
        self.inner.prune()
    }

    /// The bound the build side and every output batch settle under, `None`
    /// for the process default.
    #[getter]
    fn spill(&self) -> Option<PySpillOptions> {
        self.inner.spill().cloned().map(PySpillOptions::from_core)
    }

    /// The largest distinct build key set pushed into a probe source's
    /// filter.
    #[getter]
    fn pushdown_keys(&self) -> usize {
        self.inner.pushdown_keys()
    }

    fn __eq__(&self, py: Python<'_>, other: &Bound<'_, PyAny>) -> Py<PyAny> {
        let Ok(other) = other.extract::<PyRef<'_, Self>>() else {
            return py.NotImplemented();
        };
        PyBool::new(py, self.inner == other.inner)
            .to_owned()
            .into_any()
            .unbind()
    }

    /// Hashes every fact the equality reads.
    fn __hash__(&self) -> isize {
        let mut state = DefaultHasher::new();
        self.inner.coalesce().hash(&mut state);
        self.inner.suffix().hash(&mut state);
        self.inner.build().hash(&mut state);
        self.inner.prune().hash(&mut state);
        if let Some(spill) = self.inner.spill() {
            hash_spill_options(spill, &mut state);
        }
        self.inner.pushdown_keys().hash(&mut state);
        python_hash(state.finish())
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let spill = match self.inner.spill() {
            Some(spill) => Py::new(py, PySpillOptions::from_core(spill.clone()))?
                .bind(py)
                .repr()?
                .to_string(),
            None => "None".to_owned(),
        };
        Ok(format!(
            "JoinOptions(coalesce={}, suffix={:?}, build={}, prune={}, spill={spill}, pushdown_keys={})",
            python_bool(self.inner.coalesce()),
            self.inner.suffix(),
            self.inner
                .build()
                .map_or_else(|| "None".to_owned(), |side| format!("{:?}", side.as_str())),
            python_bool(self.inner.prune()),
            self.inner.pushdown_keys(),
        ))
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    fn __reduce__(&self, py: Python<'_>) -> (Py<PyAny>, ReduceArguments) {
        (
            py.get_type::<Self>().into_any().unbind(),
            (
                self.inner.coalesce(),
                self.inner.suffix().to_owned(),
                self.inner.build().map(JoinSide::as_str),
                self.inner.prune(),
                self.inner.spill().cloned().map(PySpillOptions::from_core),
                self.inner.pushdown_keys(),
            ),
        )
    }
}

/// What `JoinOptions(...)` takes back from a pickle, in its order.
type ReduceArguments = (
    bool,
    String,
    Option<&'static str>,
    bool,
    Option<PySpillOptions>,
    usize,
);

/// A boolean as Python spells it.
const fn python_bool(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}
