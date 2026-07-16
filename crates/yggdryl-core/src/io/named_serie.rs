//! [`NamedSerie`] — a **self-describing column**: an erased column ([`AnySerie`]) plus the *name* and
//! *metadata* it should carry into a schema. It is a **build-input carrier**, not a column: it
//! deliberately does **not** implement [`AnySerie`], so it can never be stored as a child column (a
//! name byte can never reach a column data frame). A struct column is built from a `Vec<NamedSerie>`
//! — each is unwrapped ([`into_inner`](NamedSerie::into_inner)) into the stored column, and its
//! name/metadata contribute only the schema [`AnyField`], which lives outside the data.

use super::{AnyField, AnySerie, Headers};

/// A **named, self-describing column** — an erased [`AnySerie`] together with the `name` and
/// `metadata` it should be addressed by in a schema.
///
/// Because name and metadata are **excluded** from a column's data identity and byte codec, a
/// `NamedSerie` is always *unwrapped* before storage: it exists only to carry the naming into a
/// [`field`](NamedSerie::field). It intentionally does not implement [`AnySerie`], so it cannot be
/// nested as a child.
///
/// ```
/// use yggdryl_core::io::fixed::Serie;
/// use yggdryl_core::io::{AnySerie, Headers, NamedSerie};
///
/// // Name a column in one line via `AnySerie::named`.
/// let column = Serie::from_values(&[1i32, 2, 3]).named("x");
/// assert_eq!(column.name(), "x");
/// assert_eq!(column.len(), 3);
/// assert_eq!(column.field().name(), "x");
///
/// // Attach schema metadata that rides along into the field.
/// let column = Serie::from_values(&[1i32, 2]).named("y").with_metadata(
///     Headers::new().with("origin", "sensor-a"),
/// );
/// assert_eq!(column.field().metadata().get("origin"), Some("sensor-a"));
/// ```
#[derive(Debug, Clone)]
pub struct NamedSerie {
    inner: Box<dyn AnySerie>,
    name: String,
    metadata: Headers,
}

impl NamedSerie {
    /// A named column from an erased column and its `name` (empty metadata). The
    /// [`AnySerie::named`](crate::io::AnySerie::named) shorthand builds this for a concrete `Serie`;
    /// pass a `Box<dyn AnySerie>` here directly.
    pub fn new(inner: Box<dyn AnySerie>, name: &str) -> Self {
        Self {
            inner,
            name: name.to_string(),
            metadata: Headers::new(),
        }
    }

    /// A fresh named column carrying the given schema `metadata` (replacing any existing) — the
    /// one-line, non-mutating builder.
    pub fn with_metadata(mut self, metadata: Headers) -> Self {
        self.metadata = metadata;
        self
    }

    /// The column name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The number of rows (delegates to the wrapped column).
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Whether the wrapped column has no rows.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// The wrapped erased column, borrowed.
    pub fn inner(&self) -> &(dyn AnySerie + 'static) {
        self.inner.as_ref()
    }

    /// Consumes the carrier, yielding the wrapped erased column for storage (dropping the name and
    /// metadata, which live only on the schema [`field`](NamedSerie::field)).
    pub fn into_inner(self) -> Box<dyn AnySerie> {
        self.inner
    }

    /// The schema [`AnyField`] this column contributes — the wrapped column's inferred field named
    /// [`name`](NamedSerie::name), with this carrier's `metadata` overlaid (user entries win, the
    /// intrinsic type-recovery keys are never clobbered — see
    /// [`AnyField::with_metadata_overlay`]). With empty metadata the overlay is a no-op, so the field
    /// is byte-identical to `inner.field(name)`.
    pub fn field(&self) -> AnyField {
        self.inner
            .field(&self.name)
            .with_metadata_overlay(&self.metadata)
    }
}
