//! The [`ToOutput`] rendering trait — the inverse of each type's `from_str` /
//! `from_mapping` parsers.

use crate::Mapping;

/// The output forms produced by [`ToOutput`].
pub enum Output {
    /// A rendered string, e.g. `"https://example.com"`.
    Str(String),
    /// A [`Mapping`] of components.
    Mapping(Mapping),
}

/// The inverse of a type's `from_str` / `from_mapping` parsers: render a value
/// back into a string or a component [`Mapping`]. Implemented by [`Uri`](crate::Uri),
/// [`Url`](crate::Url) and [`Version`](crate::Version).
///
/// `to_mapping` is the inverse of each type's `from_mapping`, so
/// `T::from_mapping(&value.to_mapping())` round-trips.
pub trait ToOutput {
    /// Renders to a string. `encode` controls percent-encoding where relevant.
    fn to_str(&self, encode: bool) -> String;

    /// Renders to a component [`Mapping`]. The default wraps the string form under
    /// a `"str"` key; [`Uri`](crate::Uri), [`Url`](crate::Url) and
    /// [`Version`](crate::Version) override it with real component maps that avoid
    /// a useless string serialization.
    fn to_mapping(&self) -> Mapping {
        Mapping::from([("str".to_string(), self.to_str(true))])
    }

    /// Renders to any [`Output`] form: a [`Mapping`] when `as_mapping`, otherwise
    /// the string form (whose encoding is controlled by `encode`).
    fn to_(&self, as_mapping: bool, encode: bool) -> Output {
        if as_mapping {
            Output::Mapping(self.to_mapping())
        } else {
            Output::Str(self.to_str(encode))
        }
    }
}
