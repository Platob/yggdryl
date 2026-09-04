//! The `fix:` vocabulary, on the field views that carry it.
//!
//! One type reads each property and one type writes it, and both reach the
//! metadata only through the view's own `get`, `insert` and `remove`, so
//! [`Field`](crate::Field)'s cache-aware mutation and metadata validation
//! apply to every write. The property names are private to this module: a
//! caller writes `set_tag(35)`, never `"fix:tag"`.

use std::fmt::Write as _;
use std::iter::FusedIterator;
use std::str::Split;

use smol_str::{SmolStr, format_smolstr};

use super::{FixId, FixNamespace};
use crate::{Error, FixField, FixFieldMut, Result};

/// The dictionary a field belongs to; absent means the standard one.
const NAMESPACE: &str = "namespace";
/// The full key the namespace is stored under, spelled once.
pub(super) const NAMESPACE_KEY: &str = "fix:namespace";
/// The canonical tag.
const TAG: &str = "tag";
/// The alternate tags, comma-separated, highest priority first.
const TAGS: &str = "tags";
/// The alternate names, comma-separated, highest priority first.
const ALIASES: &str = "aliases";
/// The specification's own wording.
const DESCRIPTION: &str = "description";
/// What separates the elements of a list-valued property.
const SEPARATOR: char = ',';

/// What a tag is, spelled once for every refusal.
const TAG_SHAPE: &str = "a FIX tag, a decimal integer from 0 to 2147483647";

/// What a namespace is, spelled once for every refusal.
const NAMESPACE_SHAPE: &str = "a FIX namespace, at most 23 ASCII letters, digits, hyphen, dot or underscore, starting with a letter";

/// Parse one tag strictly: decimal digits only, never negative, never signed.
///
/// `i32::from_str` would also accept `+35`, which the writer never emits, so
/// the digits are checked first and the width second. [`FixId::from_str`]
/// parses the tail of an identifier through this same one strict parse.
pub(super) fn parse_tag(text: &str) -> Option<i32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

impl<'field> FixField<'field> {
    /// Parses the dictionary this field belongs to.
    ///
    /// An absent property is [`FixNamespace::STANDARD`]: the FIX
    /// specification's own fields are the common case and state nothing.
    ///
    /// # Errors
    ///
    /// Returns an error naming the full `fix:namespace` key when the stored
    /// text is not a namespace: [`FixFieldMut::set_namespace`] never writes
    /// one, so this can only come from externally edited state.
    pub fn namespace(&self) -> Result<FixNamespace> {
        match self.get(NAMESPACE) {
            Some(stored) => FixNamespace::from_str(stored)
                .map_err(|_| self.invalid(NAMESPACE, NAMESPACE_SHAPE, stored)),
            None => Ok(FixNamespace::STANDARD),
        }
    }

    /// Builds this field's identity, absent exactly when `fix:tag` is.
    ///
    /// Derived from the namespace and the canonical tag on every ask; nothing
    /// stores it. Building it through [`FixId::from_parts`] is what refuses a
    /// hand-edited record that claims a specification tag for another
    /// dictionary at the door rather than after it is indexed.
    ///
    /// # Errors
    ///
    /// Returns the failure [`Self::tag`] or [`Self::namespace`] raises, or
    /// [`FixId::from_parts`]'s refusal.
    pub fn id(&self) -> Result<Option<FixId>> {
        let Some(tag) = self.tag()? else {
            return Ok(None);
        };
        FixId::from_parts(self.namespace()?, tag).map(Some)
    }

    /// Parses the canonical FIX tag.
    ///
    /// # Errors
    ///
    /// Returns an error naming the full `fix:tag` key when the stored text is
    /// not a tag: [`FixFieldMut::set_tag`] never writes one, so this can only
    /// come from externally edited state.
    pub fn tag(&self) -> Result<Option<i32>> {
        self.get(TAG)
            .map(|stored| parse_tag(stored).ok_or_else(|| self.invalid(TAG, TAG_SHAPE, stored)))
            .transpose()
    }

    /// Parses the alternate tags, highest priority first.
    ///
    /// An absent property is an empty list: a field states alternate tags
    /// only when it has them.
    ///
    /// # Errors
    ///
    /// Returns an error naming the full `fix:tags` key when the stored text
    /// holds an empty element, a duplicate, or anything that is not a tag.
    pub fn tags(&self) -> Result<Vec<i32>> {
        let Some(stored) = self.get(TAGS) else {
            return Ok(Vec::new());
        };
        let mut tags = Vec::new();
        for element in stored.split(SEPARATOR) {
            if element.is_empty() {
                return Err(self.invalid(TAGS, "no empty element among the tags", stored));
            }
            let tag = parse_tag(element)
                .ok_or_else(|| self.invalid(TAGS, "a comma-separated list of FIX tags", stored))?;
            if tags.contains(&tag) {
                return Err(self.invalid(TAGS, "each tag once", stored));
            }
            tags.push(tag);
        }
        Ok(tags)
    }

    /// Iterates the aliases, highest priority first.
    ///
    /// The iterator is lazy and allocates nothing: every alias is a slice of
    /// the stored text, which the field already owns, so reading them costs
    /// the same whether one is taken or all are. An absent property yields
    /// nothing.
    pub fn aliases(&self) -> FixAliases<'field> {
        FixAliases::over(self.get(ALIASES))
    }

    /// Returns the specification's own wording for this field.
    pub fn description(&self) -> Option<&'field str> {
        self.get(DESCRIPTION)
    }

    /// Name the full key a stored value failed under, and what it should be.
    fn invalid(&self, name: &str, expected: &str, actual: &str) -> Error {
        Error::InvalidMetadataValue {
            key: SmolStr::new(self.key(name)),
            reason: format_smolstr!("expected {expected}, got {actual:?}"),
        }
    }
}

impl FixFieldMut<'_> {
    /// Records the dictionary this field belongs to.
    ///
    /// [`FixNamespace::STANDARD`] removes the property rather than writing
    /// `"standard"`, exactly as an empty tag or alias list removes its own, so
    /// one declaration has one stored form. The canonical tag and every
    /// alternate tag are held to the standard-tag rule against the new
    /// namespace before anything is written.
    ///
    /// # Errors
    ///
    /// Returns [`FixId::from_parts`]'s refusal when a tag this field holds is
    /// one the FIX specification assigns, the parse failure when a stored
    /// `fix:` property is malformed, or the property write's refusal. Any of
    /// them leaves the field unchanged.
    pub fn set_namespace(&mut self, namespace: &FixNamespace) -> Result<()> {
        let view = self.as_protocol();
        if let Some(tag) = view.tag()? {
            FixId::from_parts(namespace.clone(), tag)?;
        }
        for tag in view.tags()? {
            FixId::from_parts(namespace.clone(), tag)?;
        }
        self.put_namespace(namespace)?;
        Ok(())
    }

    /// Records both halves of an identity at once.
    ///
    /// Moving a field between namespaces one property at a time works in only
    /// one order and refuses the other, because each setter holds the field to
    /// the standard-tag rule as it stands. This writes the namespace, then the
    /// tag, and restores the prior namespace entry if the tag write fails, so
    /// either move succeeds and a failure leaves the field unchanged. A
    /// [`FixId`] is already legal, so nothing beyond the tag's own shape is
    /// re-checked.
    ///
    /// # Errors
    ///
    /// Returns the tag write's refusal, having restored the namespace.
    pub fn set_id(&mut self, id: &FixId) -> Result<()> {
        let prior = self.put_namespace(id.namespace())?;
        match self.set_tag(id.tag()) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.restore_namespace(prior);
                Err(error)
            }
        }
    }

    /// Records the canonical FIX tag.
    ///
    /// # Errors
    ///
    /// Returns an error when the tag is negative, when this field's namespace
    /// may not claim it - a tag below [`FixId::STANDARD_TAG_LIMIT`] is the FIX
    /// specification's own - or when the property write fails the validation
    /// every metadata write goes through. Any of them leaves the field
    /// unchanged.
    pub fn set_tag(&mut self, tag: i32) -> Result<()> {
        if tag < 0 {
            return Err(self.rejected(TAG, format_smolstr!("expected {TAG_SHAPE}, got {tag}")));
        }
        FixId::from_parts(self.as_protocol().namespace()?, tag)?;
        self.store(TAG, tag.to_string())
    }

    /// Records the alternate tags in the given order, highest priority first.
    ///
    /// An empty slice removes the property.
    ///
    /// # Errors
    ///
    /// Returns an error when a tag is negative, repeated, or one this field's
    /// namespace may not claim, leaving the field unchanged. An alternate tag
    /// resolves exactly as a canonical one does, so it is held to the same
    /// standard-tag rule.
    pub fn set_tags(&mut self, tags: &[i32]) -> Result<()> {
        if tags.is_empty() {
            self.remove(TAGS);
            return Ok(());
        }
        let namespace = self.as_protocol().namespace()?;
        let mut rendered = String::new();
        for (index, tag) in tags.iter().enumerate() {
            if *tag < 0 {
                return Err(self.rejected(TAGS, format_smolstr!("expected {TAG_SHAPE}, got {tag}")));
            }
            FixId::from_parts(namespace.clone(), *tag)?;
            if tags[..index].contains(tag) {
                return Err(self.rejected(
                    TAGS,
                    format_smolstr!("expected each tag once, got {tag} twice"),
                ));
            }
            if index > 0 {
                rendered.push(SEPARATOR);
            }
            // Writing into a `String` cannot fail.
            let _ = write!(rendered, "{tag}");
        }
        self.store(TAGS, rendered)
    }

    /// Records the aliases in the given order, highest priority first.
    ///
    /// Empty input removes the property.
    ///
    /// # Errors
    ///
    /// Returns an error when an alias is empty, contains the separator, or
    /// repeats an earlier one with ASCII case folded, leaving the field
    /// unchanged.
    pub fn set_aliases<I, S>(&mut self, aliases: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut rendered = String::new();
        let mut count = 0;
        for alias in aliases {
            let alias = alias.as_ref();
            if alias.is_empty() {
                return Err(self.rejected(ALIASES, "expected a non-empty alias, got \"\"".into()));
            }
            if alias.contains(SEPARATOR) {
                return Err(self.rejected(
                    ALIASES,
                    format_smolstr!("expected an alias without {SEPARATOR:?}, got {alias:?}"),
                ));
            }
            if rendered
                .split(SEPARATOR)
                .any(|held| held.eq_ignore_ascii_case(alias))
            {
                return Err(self.rejected(
                    ALIASES,
                    format_smolstr!("expected each alias once, got {alias:?} twice"),
                ));
            }
            if count > 0 {
                rendered.push(SEPARATOR);
            }
            rendered.push_str(alias);
            count += 1;
        }
        if count == 0 {
            self.remove(ALIASES);
            return Ok(());
        }
        self.store(ALIASES, rendered)
    }

    /// Records the specification's own wording for this field.
    ///
    /// # Errors
    ///
    /// Returns an error when the property write fails the validation every
    /// metadata write goes through, leaving the field unchanged.
    pub fn set_description(&mut self, value: impl Into<String>) -> Result<()> {
        self.store(DESCRIPTION, value)
    }

    /// Write one property, dropping the prior value a generic insert answers.
    fn store(&mut self, name: &str, value: impl Into<String>) -> Result<()> {
        self.insert(name, value)?;
        Ok(())
    }

    /// Put the namespace entry in the one form that declaration has, and
    /// answer what stood there before.
    fn put_namespace(&mut self, namespace: &FixNamespace) -> Result<Option<String>> {
        if namespace.is_standard() {
            Ok(self.remove(NAMESPACE))
        } else {
            self.insert(NAMESPACE, namespace.as_str())
        }
    }

    /// Put back what [`Self::put_namespace`] answered.
    ///
    /// The value was read out of this very field, so re-inserting it cannot
    /// fail validation; a failure here would be reported instead of the one
    /// being unwound, which is why the result is dropped.
    fn restore_namespace(&mut self, prior: Option<String>) {
        match prior {
            Some(value) => {
                let _ = self.insert(NAMESPACE, value);
            }
            None => {
                self.remove(NAMESPACE);
            }
        }
    }

    /// Name the full key a value was refused under.
    fn rejected(&self, name: &str, reason: SmolStr) -> Error {
        Error::InvalidMetadataValue {
            key: SmolStr::new(self.key(name)),
            reason,
        }
    }
}

/// The aliases a field declares, in stored priority order.
///
/// Answered by [`FixField::aliases`]. It walks the stored comma-separated
/// text as it goes and hands back slices of it, so nothing is parsed ahead
/// of the alias being asked for and nothing is allocated. An empty element,
/// which the writer never produces, is skipped rather than reported: the
/// typed rejection belongs to the write, and a read stays cheap.
#[derive(Clone, Debug)]
pub struct FixAliases<'field> {
    parts: Option<Split<'field, char>>,
}

impl<'field> FixAliases<'field> {
    /// Walk one stored `fix:aliases` value, or nothing for an absent one.
    fn over(stored: Option<&'field str>) -> Self {
        Self {
            parts: stored.map(|stored| stored.split(SEPARATOR)),
        }
    }
}

impl<'field> Iterator for FixAliases<'field> {
    type Item = &'field str;

    fn next(&mut self) -> Option<Self::Item> {
        self.parts.as_mut()?.find(|alias| !alias.is_empty())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.parts {
            Some(parts) => (0, parts.size_hint().1),
            None => (0, Some(0)),
        }
    }
}

impl DoubleEndedIterator for FixAliases<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.parts.as_mut()?.rfind(|alias| !alias.is_empty())
    }
}

impl FusedIterator for FixAliases<'_> {}
