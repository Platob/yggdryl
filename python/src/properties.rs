//! Option properties given by name: the keywords every options-taking door
//! takes beside `options`, and the warning a name no property owns raises.
//!
//! Each property is set on a copy of the options by that property's own
//! setter - the fold lives beside each options class - so a keyword is
//! validated exactly as an assignment is. A name no setter owns is not an
//! error: it is skipped with an [`UnknownPropertyWarning`] naming it and the
//! closest property there is, so a typo is heard without failing a read.

use pyo3::exceptions::PyUserWarning;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyType};

pyo3::create_exception!(
    yggdryl,
    UnknownPropertyWarning,
    PyUserWarning,
    "A keyword naming no property of the options it was given for; it was ignored."
);

/// Whether any property was given: a value other than `...`, the project's
/// spelling for an argument that was not.
pub(crate) fn given(properties: Option<&Bound<'_, PyDict>>) -> bool {
    properties.is_some_and(|properties| {
        let ellipsis = properties.py().Ellipsis();
        properties.values().iter().any(|value| !value.is(&ellipsis))
    })
}

/// Warn that `name` names no settable property of `owner`, suggesting the
/// closest of `candidates`.
///
/// Raised through the `warnings` machinery, so a filter escalating
/// [`UnknownPropertyWarning`] to an error turns this into that error.
pub(crate) fn warn_unknown<S: AsRef<str>>(
    py: Python<'_>,
    owner: &str,
    name: &str,
    candidates: impl IntoIterator<Item = S>,
) -> PyResult<()> {
    // Quoted as Python quotes a name in its own messages.
    let message = match closest(name, candidates) {
        Some(closest) => format!(
            "{owner} has no settable property '{name}'; it is ignored; did you mean '{closest}'?"
        ),
        None => format!("{owner} has no settable property '{name}'; it is ignored"),
    };
    let message = std::ffi::CString::new(message)
        .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
    // Level one is the Python line that called into the extension: a native
    // frame is no Python frame to count.
    PyErr::warn(py, &py.get_type::<UnknownPropertyWarning>(), &message, 1)
}

/// The settable properties a native options class declares, read off the
/// class itself - asked only on the warning path, so the list is the class's
/// own and never a second copy of it.
pub(crate) fn class_properties(class: &Bound<'_, PyType>) -> PyResult<Vec<String>> {
    let mut names = Vec::new();
    for name in class.dir()? {
        let name = name.extract::<String>()?;
        if name.starts_with('_') {
            continue;
        }
        let member = class.getattr(name.as_str())?;
        if member.get_type().name()? == "getset_descriptor" {
            names.push(name);
        }
    }
    Ok(names)
}

/// The candidate closest to `name` by edit distance, when one is close
/// enough to be the name meant: within a third of its length, and at least
/// one edit.
fn closest<S: AsRef<str>>(name: &str, candidates: impl IntoIterator<Item = S>) -> Option<String> {
    let limit = (name.chars().count() / 3).max(1);
    let folded = name.to_ascii_lowercase();
    candidates
        .into_iter()
        .filter_map(|candidate| {
            let candidate = candidate.as_ref();
            let distance = edit_distance(&folded, &candidate.to_ascii_lowercase());
            (distance <= limit).then(|| (distance, candidate.to_owned()))
        })
        .min()
        .map(|(_, candidate)| candidate)
}

/// Levenshtein distance over characters, one row at a time.
fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];
    for (row, left) in left.chars().enumerate() {
        current[0] = row + 1;
        for (column, right) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(left != *right);
            current[column + 1] = substitution
                .min(previous[column + 1] + 1)
                .min(current[column] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

/// The `(name, value)` text pairs a string-keyed options door reads out of
/// `properties`, the keywords beside its mapping.
///
/// `...` and `None` are skipped as not given, a `bool` is spelled `true` or
/// `false`, anything else as `str()` spells it; a name `is_property` answers
/// `false` for is skipped with an `UnknownPropertyWarning` naming the closest
/// of `candidates`. The core reads the pairs, so what a name means - and
/// every refusal of a value - stays the core's.
pub(crate) fn property_pairs(
    owner: &str,
    properties: Option<&Bound<'_, PyDict>>,
    is_property: impl Fn(&str) -> bool,
    candidates: &[&str],
) -> PyResult<Vec<(String, String)>> {
    let Some(properties) = properties else {
        return Ok(Vec::new());
    };
    let py = properties.py();
    let ellipsis = py.Ellipsis();
    let mut pairs = Vec::with_capacity(properties.len());
    for (name, value) in properties.iter() {
        let name = name.cast::<pyo3::types::PyString>()?.to_str()?;
        if !is_property(name) {
            warn_unknown(py, owner, name, candidates)?;
            continue;
        }
        if value.is(&ellipsis) || value.is_none() {
            continue;
        }
        let text = if let Ok(flag) = value.cast::<pyo3::types::PyBool>() {
            if flag.is_true() { "true" } else { "false" }.to_owned()
        } else {
            value.str()?.to_str()?.to_owned()
        };
        pairs.push((name.to_owned(), text));
    }
    Ok(pairs)
}
