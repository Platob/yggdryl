//! Digest roles and effective component selection on fields.

use std::collections::HashSet;
use std::slice;

use smol_str::{SmolStr, format_smolstr};

use crate::types::protocol::{DigestField, DigestFieldMut};
use crate::{DataType, DigestAlgorithm, Error, Field, Result, Scalar};

const ALGORITHM: &str = "algorithm";
const PATHS: &str = "paths";
const ROLE: &str = "role";
const DIGEST_PATHS_SHAPE: &str = "a JSON array of unique non-empty field path strings";

pub(crate) const DIGEST_ALGORITHM_KEY: &str = "digest:algorithm";
pub(crate) const DIGEST_PATHS_KEY: &str = "digest:paths";
pub(crate) const DIGEST_ROLE_KEY: &str = "digest:role";
pub(crate) const DIGEST_ROLE_COMPONENT: &str = "component";
pub(crate) const DIGEST_ROLE_HOLDER: &str = "holder";

/// Return whether a holder's physical datatype carries this algorithm exactly.
pub(crate) fn holder_accepts(field: &Field, algorithm: DigestAlgorithm) -> bool {
    match algorithm {
        DigestAlgorithm::Xxh32 => matches!(field.dtype(), DataType::UInt32),
        DigestAlgorithm::Xxh64 | DigestAlgorithm::Xxh3 => {
            matches!(field.dtype(), DataType::UInt64)
        }
        DigestAlgorithm::Xxh128 => matches!(field.dtype(), DataType::FixedSizeBinary(16)),
    }
}

/// Return the canonical datatype spelling an algorithm's holder requires.
pub(crate) const fn expected_holder_dtype(algorithm: DigestAlgorithm) -> &'static str {
    match algorithm {
        DigestAlgorithm::Xxh32 => "uint32",
        DigestAlgorithm::Xxh64 | DigestAlgorithm::Xxh3 => "uint64",
        DigestAlgorithm::Xxh128 => "fixed_size_binary[16]",
    }
}

/// Parse a stored digest algorithm and return its canonical token.
pub(crate) fn canonicalize_digest_algorithm(value: &str) -> Result<String> {
    parse_digest_algorithm(value).map(|algorithm| algorithm.as_str().to_owned())
}

fn parse_digest_algorithm(value: &str) -> Result<DigestAlgorithm> {
    DigestAlgorithm::from_str(value).map_err(|error| Error::InvalidMetadataValue {
        key: SmolStr::new_static(DIGEST_ALGORITHM_KEY),
        reason: SmolStr::new(error.to_string()),
    })
}

/// Parse the ordered field paths one digest holder selects.
fn parse_digest_paths(value: &str) -> Result<Vec<String>> {
    let document = crate::text::json::from_utf8(value).map_err(|error| {
        invalid_digest_paths(format_smolstr!(
            "expected {DIGEST_PATHS_SHAPE}, got invalid JSON: {}",
            crate::text::elide_display(&error)
        ))
    })?;
    let Some(values) = document.as_sequence() else {
        return Err(invalid_digest_paths(crate::text::expected_got(
            DIGEST_PATHS_SHAPE,
            format_args!("{:?}", crate::text::elide_to(value, 256)),
        )));
    };
    let mut paths = Vec::with_capacity(values.len());
    let mut seen = HashSet::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let Some(path) = value.as_str() else {
            let actual = crate::text::json::into_utf8(value)
                .unwrap_or_else(|_| "<unencodable JSON value>".to_owned());
            return Err(invalid_digest_paths(format_smolstr!(
                "expected a field path string at index {index}, got {:?}",
                crate::text::elide_to(&actual, 256)
            )));
        };
        if path.is_empty() {
            return Err(invalid_digest_paths(format_smolstr!(
                "expected a non-empty field path string at index {index}"
            )));
        }
        if !seen.insert(path) {
            return Err(invalid_digest_paths(format_smolstr!(
                "expected each field path once, got {path:?} twice"
            )));
        }
        paths.push(path.to_owned());
    }
    Ok(paths)
}

/// Render ordered digest paths through the canonical compact JSON codec.
fn render_digest_paths<I, P>(paths: I) -> Result<String>
where
    I: IntoIterator<Item = P>,
    P: AsRef<str>,
{
    let paths: Vec<String> = paths
        .into_iter()
        .map(|path| path.as_ref().to_owned())
        .collect();
    let mut seen = HashSet::with_capacity(paths.len());
    for (index, path) in paths.iter().enumerate() {
        if path.is_empty() {
            return Err(invalid_digest_paths(format_smolstr!(
                "expected a non-empty field path string at index {index}"
            )));
        }
        if !seen.insert(path.as_str()) {
            return Err(invalid_digest_paths(format_smolstr!(
                "expected each field path once, got {path:?} twice"
            )));
        }
    }
    let document = Scalar::from_sequence(paths.into_iter().map(Scalar::from));
    crate::text::json::into_utf8(&document).map_err(|error| {
        invalid_digest_paths(format_smolstr!(
            "could not encode canonical field paths: {}",
            crate::text::elide_display(&error)
        ))
    })
}

/// Restate externally supplied path JSON in its one stored spelling.
pub(crate) fn canonicalize_digest_paths(value: &str) -> Result<String> {
    render_digest_paths(parse_digest_paths(value)?)
}

fn invalid_digest_paths(reason: SmolStr) -> Error {
    Error::InvalidMetadataValue {
        key: SmolStr::new_static(DIGEST_PATHS_KEY),
        reason,
    }
}

impl DigestField<'_> {
    /// Returns whether this field holds a digest rather than contributing to it.
    pub fn is_holder(&self) -> bool {
        self.get(ROLE) == Some(DIGEST_ROLE_HOLDER)
    }

    /// Returns whether this field is an explicitly selected digest component.
    pub fn is_component(&self) -> bool {
        self.get(ROLE) == Some(DIGEST_ROLE_COMPONENT)
    }

    /// Parses the algorithm this holder declares.
    ///
    /// # Errors
    ///
    /// Returns an error naming `digest:algorithm` when externally supplied
    /// metadata is not a [`DigestAlgorithm`] token.
    pub fn algorithm(&self) -> Result<Option<DigestAlgorithm>> {
        self.get(ALGORITHM).map(parse_digest_algorithm).transpose()
    }

    /// Parses the ordered paths this holder selects relative to its Struct.
    ///
    /// `None` retains component-role fallback selection. `Some([])` is an
    /// explicit empty selection, so absence and an empty JSON array remain
    /// distinct.
    ///
    /// # Errors
    ///
    /// Returns an error naming `digest:paths` when stored metadata is not a
    /// JSON array of unique non-empty strings.
    pub fn paths(&self) -> Result<Option<Vec<String>>> {
        self.get(PATHS).map(parse_digest_paths).transpose()
    }
}

impl DigestFieldMut<'_> {
    /// Marks this field as holding a digest rather than contributing to it.
    pub fn set_holder(&mut self) -> Result<()> {
        self.insert(ROLE, DIGEST_ROLE_HOLDER).map(|_| ())
    }

    /// Marks this field as an explicitly selected digest component.
    ///
    /// # Errors
    ///
    /// Returns an error when holder-owned algorithm or paths metadata is
    /// present, leaving the field unchanged. Remove both before changing the
    /// role.
    pub fn set_component(&mut self) -> Result<()> {
        if self.has_holder_properties() {
            return Err(self.rejected(
                ROLE,
                "cannot set component while digest:algorithm or digest:paths is present".into(),
            ));
        }
        self.insert(ROLE, DIGEST_ROLE_COMPONENT).map(|_| ())
    }

    /// Records the algorithm this holder carries in canonical spelling.
    ///
    /// # Errors
    ///
    /// Returns an error when this field is not a holder, leaving it unchanged.
    pub fn set_algorithm(&mut self, algorithm: DigestAlgorithm) -> Result<()> {
        if !self.as_protocol().is_holder() {
            return Err(self.rejected(ALGORITHM, "requires digest:role=holder".into()));
        }
        if !holder_accepts(self.as_field(), algorithm) {
            return Err(self.rejected(
                ALGORITHM,
                format_smolstr!(
                    "algorithm {algorithm} requires {}, got {}",
                    expected_holder_dtype(algorithm),
                    self.as_field().dtype()
                ),
            ));
        }
        self.insert(ALGORITHM, algorithm.as_str()).map(|_| ())
    }

    /// Removes this holder's explicit digest algorithm.
    pub fn remove_algorithm(&mut self) -> Option<String> {
        self.remove(ALGORITHM)
    }

    /// Records the ordered paths this holder selects relative to its Struct.
    ///
    /// Order is hash-significant. Empty input stores `[]`, which explicitly
    /// selects the empty sequence; [`Self::remove_paths`] restores fallback
    /// selection instead.
    ///
    /// # Errors
    ///
    /// Returns an error when this field is not a holder, or when a path is
    /// empty or repeated, leaving the field unchanged.
    pub fn set_paths<I, P>(&mut self, paths: I) -> Result<()>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<str>,
    {
        if !self.as_protocol().is_holder() {
            return Err(self.rejected(PATHS, "requires digest:role=holder".into()));
        }
        self.insert(PATHS, render_digest_paths(paths)?).map(|_| ())
    }

    /// Removes the explicit holder path selection and restores fallback.
    pub fn remove_paths(&mut self) -> Option<String> {
        self.remove(PATHS)
    }

    /// Removes this field's explicit digest role.
    ///
    /// # Errors
    ///
    /// Returns an error when holder-owned algorithm or paths metadata is
    /// present, leaving the field unchanged. Remove both first.
    pub fn remove_role(&mut self) -> Result<Option<String>> {
        if self.has_holder_properties() {
            return Err(self.rejected(
                ROLE,
                "cannot remove holder role while digest:algorithm or digest:paths is present"
                    .into(),
            ));
        }
        Ok(self.remove(ROLE))
    }

    fn has_holder_properties(&self) -> bool {
        self.contains_key(ALGORITHM) || self.contains_key(PATHS)
    }

    /// Name the full digest key a typed mutation was refused under.
    fn rejected(&self, name: &str, reason: SmolStr) -> Error {
        Error::InvalidMetadataValue {
            key: SmolStr::new(self.key(name)),
            reason,
        }
    }
}

impl Field {
    /// Returns the struct children that contribute to a row digest.
    ///
    /// One or more children marked as digest components form the exact input
    /// set. With no explicit component, every child except a digest holder is
    /// selected. Declaration order is retained in both cases.
    pub fn digest_fields(&self) -> DigestFields<'_> {
        DigestFields::new(self.fields())
    }

    /// Returns the names of the effective row-digest components.
    pub fn digest_field_names(&self) -> DigestFieldNames<'_> {
        DigestFieldNames(self.digest_fields())
    }

    /// Returns the number of effective row-digest components.
    pub fn digest_field_len(&self) -> usize {
        self.digest_fields().count()
    }

    /// Returns whether any child explicitly declares the digest-component role.
    pub fn has_digest_components(&self) -> bool {
        has_explicit_components(self.fields())
    }

    /// Returns this struct root holding only its effective digest components.
    ///
    /// # Errors
    ///
    /// Returns an error when this is not a struct, or when the selected
    /// children do not form a valid datatype.
    pub fn only_digest_fields(&self) -> Result<Self> {
        self.require_struct()?;
        let kept: Vec<Self> = self.digest_fields().cloned().collect();
        Self::from_parts(
            self.name(),
            DataType::from_fields(kept)?,
            self.is_nullable(),
            self.metadata_iter(),
        )
    }
}

pub(crate) fn has_explicit_components(fields: &[Field]) -> bool {
    fields.iter().any(|field| field.as_digest().is_component())
}

pub(crate) fn is_effective_component(field: &Field, explicit: bool) -> bool {
    let digest = field.as_digest();
    !digest.is_holder() && (!explicit || digest.is_component())
}

/// A borrowed iterator over the effective row-digest components.
#[derive(Clone)]
pub struct DigestFields<'field> {
    fields: slice::Iter<'field, Field>,
    explicit: bool,
}

impl<'field> DigestFields<'field> {
    pub(crate) fn new(fields: &'field [Field]) -> Self {
        Self {
            fields: fields.iter(),
            explicit: has_explicit_components(fields),
        }
    }
}

impl<'field> Iterator for DigestFields<'field> {
    type Item = &'field Field;

    fn next(&mut self) -> Option<Self::Item> {
        self.fields
            .find(|field| is_effective_component(field, self.explicit))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(self.fields.len()))
    }
}

impl DoubleEndedIterator for DigestFields<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.fields
            .rfind(|field| is_effective_component(field, self.explicit))
    }
}

impl std::iter::FusedIterator for DigestFields<'_> {}

/// A borrowed iterator over the effective row-digest component names.
#[derive(Clone)]
pub struct DigestFieldNames<'field>(DigestFields<'field>);

impl<'field> Iterator for DigestFieldNames<'field> {
    type Item = &'field str;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(Field::name)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl DoubleEndedIterator for DigestFieldNames<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(Field::name)
    }
}

impl std::iter::FusedIterator for DigestFieldNames<'_> {}
