//! Fields borrowed as one protocol.
//!
//! A view is a field plus the protocol it is being read through: one pointer
//! and a [`Scheme`], no duplicated state. Property access delegates to
//! [`ProtocolMetadata`], so lookup, iteration, prefix folding and key assembly
//! keep one implementation, and every write routes through [`Field`]'s own
//! cache-aware mutation.
//!
//! The named pairs are minted from the one protocol list [`Metadata`]'s own
//! snapshot accessors come from, so a protocol added there gains its accessor
//! and its two types in the same change. A named view carries the vocabulary
//! of a *foreign* protocol; state a field owns whatever key it is stored
//! under - [`Field::is_init`], [`Field::is_partition`], `alias`, `comment`,
//! `location` and `PARQUET:field_id` - stays on [`Field`].

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Deref, DerefMut, Index};

use super::Field;
use crate::metadata::{
    PropertyIter, ProtocolMetadata, for_each_well_known_protocol, property_key, property_name,
    protocol_metadata_prefix,
};
use crate::{Metadata, Result, Scheme};

mod http;

/// A field borrowed as one protocol: its properties by bare name, and the
/// field itself.
///
/// A protocol property is stored under a `scheme:name` key, and code that
/// spells that key by hand has to spell it right in every branch it appears
/// in. This view remembers the protocol once, so a caller writes `doc` where
/// it used to write `"iceberg:doc"`. Constructing one costs a `Scheme` clone
/// of a known protocol - which allocates nothing - and no map walk, so it is
/// built per call rather than stored.
///
/// `Deref<Target = Field>` puts the whole field surface on the view, but
/// `deref` borrows the *view*, so `field.as_iceberg().name()` is E0716 on a
/// temporary. [`Self::as_field`] answers the field with the view's own
/// lifetime and is the spelling that outlives it; every property read already
/// returns that lifetime and needs no hop.
///
/// Three names shadow [`Field`]'s through that deref, each deliberately:
/// [`Self::comment`] is the protocol form and a strict superset of
/// [`Field::comment`]; [`Self::merge_with`] takes one argument where
/// [`Field::merge_with`] takes two, so a wrong pick is a compile error rather
/// than a silent one; and `HttpField::location` reads `http:location` where
/// [`Field::location`] reads the namespace-free `location`.
///
/// ```
/// use yggdryl::DataType;
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut field = DataType::Int64.required_field("price");
/// field.as_iceberg_mut().insert("doc", "closing price")?;
///
/// // The value outlives the view it was read through.
/// let doc = field.as_iceberg().get("doc");
/// assert_eq!(doc, Some("closing price"));
///
/// assert_eq!(field.as_iceberg().as_field().name(), "price");
/// assert_eq!(&field.as_iceberg()["doc"], "closing price");
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct ProtocolField<'field> {
    field: &'field Field,
    scheme: Scheme,
}

impl<'field> ProtocolField<'field> {
    /// Borrows one protocol's properties on a field.
    pub(crate) fn new(field: &'field Field, scheme: Scheme) -> Self {
        Self { field, scheme }
    }

    /// Borrows the whole field this view reads, with the view's lifetime.
    pub const fn as_field(&self) -> &'field Field {
        self.field
    }

    /// Borrows this protocol's properties on the field's metadata snapshot.
    pub fn as_properties(&self) -> ProtocolMetadata<'field> {
        // Reading the borrow out through a binding copies it; calling on the
        // place `self.field` would reborrow it for `&self` and collapse every
        // `'field` return to the view's own lifetime.
        let field: &'field Field = self.field;
        field.as_metadata().protocol(&self.scheme)
    }

    /// Returns the protocol this view remembers.
    pub const fn scheme(&self) -> &Scheme {
        &self.scheme
    }

    /// Returns the canonical key prefix this view applies.
    ///
    /// [`ProtocolMetadata::prefix`] carries the HTTPS folding.
    pub fn prefix(&self) -> &str {
        protocol_metadata_prefix(&self.scheme)
    }

    /// Returns the full metadata key one property name is stored under.
    pub fn key(&self, name: &str) -> String {
        property_key(&self.scheme, name)
    }

    /// Returns one property value by its bare name.
    pub fn get(&self, name: &str) -> Option<&'field str> {
        self.as_properties().get(name)
    }

    /// Returns whether one property exists.
    pub fn contains_key(&self, name: &str) -> bool {
        self.field.has_property(&self.scheme, name)
    }

    /// Returns the number of properties this protocol holds.
    ///
    /// [`ProtocolMetadata::len`] carries the cost.
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// Returns whether this protocol holds no properties.
    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }

    /// Iterates this protocol's names and values in lexical order.
    pub fn iter(&self) -> PropertyIter<'field, '_> {
        // Not routed through `as_properties`: the iterator's second lifetime
        // is cut from the `Scheme` it is built over, so a temporary view would
        // hand back an iterator borrowing an already-dead one. This reaches
        // the same single implementation one link further down.
        let field: &'field Field = self.field;
        field.property_iter(&self.scheme)
    }

    /// Returns the first property after `after_name`, or the first for `None`.
    ///
    /// This is the cursor form an owning FFI iterator advances with.
    pub fn next_entry(&self, after_name: Option<&str>) -> Option<(&'field str, &'field str)> {
        self.as_properties().next_entry(after_name)
    }

    /// Returns this protocol's comment, falling back to the straight one.
    ///
    /// [`ProtocolMetadata::comment`] carries the rule.
    pub fn comment(&self) -> Option<&'field str> {
        self.as_properties().comment()
    }

    /// Collects this protocol's properties as a standalone snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error only when a property fails the validation it already
    /// passed, which externally corrupted serialized state can produce.
    pub fn into_metadata(self) -> Result<Metadata> {
        self.as_properties().into_metadata()
    }

    /// Returns this protocol's properties merged with another view's.
    ///
    /// [`ProtocolMetadata::merge_with`] carries the direction and the keying.
    ///
    /// # Errors
    ///
    /// Returns an error when a merged property fails the validation every
    /// write goes through.
    pub fn merge_with(&self, other: &ProtocolField<'_>) -> Result<Metadata> {
        self.as_properties().merge_with(&other.as_properties())
    }
}

impl Deref for ProtocolField<'_> {
    type Target = Field;

    fn deref(&self) -> &Field {
        self.field
    }
}

impl AsRef<Field> for ProtocolField<'_> {
    fn as_ref(&self) -> &Field {
        self.field
    }
}

/// Subscripting a protocol view by name reaches one property value.
///
/// The concrete impl is what keeps the operator on properties: [`Field`]'s own
/// `Index<&str>` answers a child field, and the operator autoderefs, so
/// omitting this would silently descend the schema instead.
///
/// # Panics
///
/// Panics when this protocol carries no property of that name.
impl Index<&str> for ProtocolField<'_> {
    type Output = str;

    fn index(&self, name: &str) -> &Self::Output {
        self.get(name)
            .unwrap_or_else(|| missing_property(&self.key(name)))
    }
}

impl fmt::Debug for ProtocolField<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ProtocolField")
            .field(&self.prefix())
            .field(&format_args!("{self}"))
            .finish()
    }
}

impl fmt::Display for ProtocolField<'_> {
    /// Renders this protocol's own names as a deterministic JSON object.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.as_properties(), formatter)
    }
}

impl PartialEq for ProtocolField<'_> {
    /// Compares the properties two views expose, not the fields behind them.
    fn eq(&self, other: &Self) -> bool {
        self.as_properties() == other.as_properties()
    }
}

impl Eq for ProtocolField<'_> {}

impl PartialOrd for ProtocolField<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ProtocolField<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_properties().cmp(&other.as_properties())
    }
}

impl Hash for ProtocolField<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_properties().hash(state);
    }
}

impl<'view, 'field> IntoIterator for &'view ProtocolField<'field> {
    type Item = (&'field str, &'field str);
    type IntoIter = PropertyIter<'field, 'view>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// A field mutably borrowed as one protocol.
///
/// This is [`ProtocolField`] with the field's mutations added. It is a
/// separate value for the reason every mutable view in Rust is: it borrows the
/// field exclusively, so a read-only view can be handed out freely while this
/// one exists only where a change is actually being made.
///
/// Every write routes through [`Field`]'s own cache-aware mutation, so a
/// protocol write invalidates a populated Arrow projection exactly as a direct
/// metadata write does, and a rejected value leaves the field untouched.
///
/// There is deliberately no `DerefMut<Target = Field>` and no `as_field_mut`:
/// either would put the field's whole mutator surface back on a view named for
/// a foreign protocol, which is the ambiguity these views exist to remove.
///
/// ```
/// use yggdryl::DataType;
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut field = DataType::Int64.required_field("price");
///
/// field.as_iceberg_mut().insert("doc", "closing price")?;
/// field.as_iceberg_mut().insert("schema-id", "3")?;
///
/// assert_eq!(field.as_iceberg().get("doc"), Some("closing price"));
/// assert_eq!(field.get_metadata("iceberg:doc"), Some("closing price"));
///
/// assert_eq!(
///     field.as_iceberg_mut().remove("doc").as_deref(),
///     Some("closing price"),
/// );
/// field.as_iceberg_mut().clear();
/// assert!(field.as_iceberg().is_empty());
/// # Ok(())
/// # }
/// ```
pub struct ProtocolFieldMut<'field> {
    field: &'field mut Field,
    scheme: Scheme,
}

impl<'field> ProtocolFieldMut<'field> {
    /// Borrows one protocol's properties on a field for reading and writing.
    pub(crate) fn new(field: &'field mut Field, scheme: Scheme) -> Self {
        Self { field, scheme }
    }

    /// Borrows the whole field this view writes.
    pub const fn as_field(&self) -> &Field {
        self.field
    }

    /// Borrows the read-only view of the same protocol.
    pub fn as_protocol(&self) -> ProtocolField<'_> {
        ProtocolField::new(self.field, self.scheme.clone())
    }

    /// Returns the protocol this view remembers.
    pub const fn scheme(&self) -> &Scheme {
        &self.scheme
    }

    /// Returns the canonical key prefix this view applies.
    ///
    /// [`ProtocolMetadata::prefix`] carries the HTTPS folding.
    pub fn prefix(&self) -> &str {
        protocol_metadata_prefix(&self.scheme)
    }

    /// Returns the full metadata key one property name is stored under.
    pub fn key(&self, name: &str) -> String {
        property_key(&self.scheme, name)
    }

    /// Returns one property value by its bare name.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.field.get_property(&self.scheme, name)
    }

    /// Returns whether one property exists.
    pub fn contains_key(&self, name: &str) -> bool {
        self.field.has_property(&self.scheme, name)
    }

    /// Returns the number of properties this protocol holds.
    ///
    /// [`ProtocolMetadata::len`] carries the cost.
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// Returns whether this protocol holds no properties.
    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }

    /// Iterates this protocol's names and values in lexical order.
    pub fn iter(&self) -> PropertyIter<'_, '_> {
        self.field.property_iter(&self.scheme)
    }

    /// Returns the first property after `after_name`, or the first for `None`.
    ///
    /// [`ProtocolField::next_entry`] carries what the cursor form is for.
    pub fn next_entry(&self, after_name: Option<&str>) -> Option<(&str, &str)> {
        self.field.next_property_entry(&self.scheme, after_name)
    }

    /// Returns this protocol's comment, falling back to the straight one.
    ///
    /// [`ProtocolMetadata::comment`] carries the rule.
    pub fn comment(&self) -> Option<&str> {
        self.as_protocol().comment()
    }

    /// Inserts or replaces one property and returns its prior value.
    ///
    /// # Errors
    ///
    /// Returns an error when the name or value fails the validation the
    /// protocol namespace applies, leaving the field unchanged.
    pub fn insert(&mut self, name: &str, value: impl Into<String>) -> Result<Option<String>> {
        self.field.set_property(&self.scheme, name, value)
    }

    /// Overlays several properties, keeping the ones not named.
    ///
    /// The whole overlay is validated before any of it is applied, so a
    /// rejected entry leaves every other entry unwritten too.
    ///
    /// # Errors
    ///
    /// Returns an error when any name or value fails validation.
    pub fn update<I, N, V>(&mut self, entries: I) -> Result<()>
    where
        I: IntoIterator<Item = (N, V)>,
        N: AsRef<str>,
        V: Into<String>,
    {
        let overlay: Vec<(String, String)> = entries
            .into_iter()
            .map(|(name, value)| (self.key(name.as_ref()), value.into()))
            .collect();
        self.field.update_metadata(overlay)
    }

    /// Replaces this protocol's properties with exactly these, atomically.
    ///
    /// Properties of other protocols and every shared key are untouched, which
    /// is what makes this a protocol-scoped `set` rather than a metadata one.
    ///
    /// # Errors
    ///
    /// Returns an error when any name or value fails validation, leaving the
    /// field unchanged.
    pub fn set<I, N, V>(&mut self, entries: I) -> Result<()>
    where
        I: IntoIterator<Item = (N, V)>,
        N: AsRef<str>,
        V: Into<String>,
    {
        let prefix = protocol_metadata_prefix(&self.scheme);
        let mut replacement: Vec<(String, String)> = self
            .field
            .metadata_iter()
            .filter(|(key, _)| property_name(key, prefix).is_none())
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect();
        for (name, value) in entries {
            replacement.push((self.key(name.as_ref()), value.into()));
        }
        self.field.set_metadata(replacement)
    }

    /// Merges another protocol view's properties into this one, in place.
    ///
    /// A name this field already carries keeps its value, so the merge only
    /// ever adds - the same direction [`Metadata::merge_with`] resolves in,
    /// seen from the receiving side. Properties of other protocols are
    /// untouched.
    ///
    /// # Errors
    ///
    /// Returns an error when a merged property fails validation, leaving the
    /// field unchanged.
    pub fn merge_with(&mut self, other: &ProtocolField<'_>) -> Result<()> {
        let held: Vec<String> = self.iter().map(|(name, _)| name.to_owned()).collect();
        let additions: Vec<(String, String)> = other
            .iter()
            .filter(|(name, _)| !held.iter().any(|kept| kept == name))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        self.update(additions)
    }

    /// Removes one property and returns its prior value.
    pub fn remove(&mut self, name: &str) -> Option<String> {
        self.field.remove_property(&self.scheme, name)
    }

    /// Removes every property of this protocol.
    pub fn clear(&mut self) {
        self.field.clear_properties(&self.scheme);
    }
}

impl Deref for ProtocolFieldMut<'_> {
    type Target = Field;

    fn deref(&self) -> &Field {
        self.field
    }
}

impl AsRef<Field> for ProtocolFieldMut<'_> {
    fn as_ref(&self) -> &Field {
        self.field
    }
}

/// Subscripting a mutable protocol view by name reaches one property value.
///
/// [`Index<&str>`] on [`ProtocolField`] carries why the impl is mandatory.
///
/// # Panics
///
/// Panics when this protocol carries no property of that name.
impl Index<&str> for ProtocolFieldMut<'_> {
    type Output = str;

    fn index(&self, name: &str) -> &Self::Output {
        self.get(name)
            .unwrap_or_else(|| missing_property(&self.key(name)))
    }
}

/// Report the full key a subscript found nothing under.
#[cold]
fn missing_property(key: &str) -> ! {
    panic!("metadata property {key:?} is not present")
}

impl fmt::Debug for ProtocolFieldMut<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ProtocolFieldMut")
            .field(&self.prefix())
            .field(&format_args!("{}", self.as_protocol()))
            .finish()
    }
}

impl fmt::Display for ProtocolFieldMut<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.as_protocol(), formatter)
    }
}

/// Mint one protocol's named borrowed and mutable field views.
macro_rules! protocol_field_types {
    ($name:ident, $mutable:ident, $constant:ident, $view:ident, $view_mut:ident, $label:literal) => {
        #[doc = concat!("A field borrowed as its ", $label, " protocol.")]
        ///
        /// Dereferences to [`ProtocolField`] for the property surface and
        /// through it to [`crate::Field`] for the field surface.
        #[derive(Clone)]
        pub struct $view<'field>(ProtocolField<'field>);

        impl<'field> $view<'field> {
            #[doc = concat!("Borrows a field as its ", $label, " protocol.")]
            pub(crate) fn new(field: &'field Field) -> Self {
                Self(ProtocolField::new(field, Scheme::$constant))
            }
        }

        impl<'field> Deref for $view<'field> {
            type Target = ProtocolField<'field>;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl AsRef<Field> for $view<'_> {
            fn as_ref(&self) -> &Field {
                self.0.as_field()
            }
        }

        impl fmt::Debug for $view<'_> {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_tuple(stringify!($view))
                    .field(&format_args!("{}", self.0))
                    .finish()
            }
        }

        impl fmt::Display for $view<'_> {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, formatter)
            }
        }

        impl PartialEq for $view<'_> {
            fn eq(&self, other: &Self) -> bool {
                self.0 == other.0
            }
        }

        impl Eq for $view<'_> {}

        impl PartialOrd for $view<'_> {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }

        impl Ord for $view<'_> {
            fn cmp(&self, other: &Self) -> Ordering {
                self.0.cmp(&other.0)
            }
        }

        impl Hash for $view<'_> {
            fn hash<H: Hasher>(&self, state: &mut H) {
                self.0.hash(state);
            }
        }

        // `for` selects on the exact type and never derefs, so the base
        // impl alone would not put a named view in a loop.
        impl<'view, 'field> IntoIterator for &'view $view<'field> {
            type Item = (&'field str, &'field str);
            type IntoIter = PropertyIter<'field, 'view>;

            fn into_iter(self) -> Self::IntoIter {
                self.0.iter()
            }
        }

        #[doc = concat!("A field mutably borrowed as its ", $label, " protocol.")]
        ///
        /// Dereferences to [`ProtocolFieldMut`], which is what carries the
        /// write surface; there is no path from here to [`crate::Field`]'s
        /// own mutators.
        pub struct $view_mut<'field>(ProtocolFieldMut<'field>);

        impl<'field> $view_mut<'field> {
            #[doc = concat!("Borrows a field mutably as its ", $label, " protocol.")]
            pub(crate) fn new(field: &'field mut Field) -> Self {
                Self(ProtocolFieldMut::new(field, Scheme::$constant))
            }

            #[doc = concat!("Borrows the read-only ", $label, " view of the same field.")]
            ///
            /// This refines [`ProtocolFieldMut::as_protocol`] to the named
            /// type, which is what lets a typed remover read its own typed
            /// prior value.
            pub fn as_protocol(&self) -> $view<'_> {
                $view::new(&*self.0.field)
            }
        }

        impl<'field> Deref for $view_mut<'field> {
            type Target = ProtocolFieldMut<'field>;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl<'field> DerefMut for $view_mut<'field> {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }

        impl AsRef<Field> for $view_mut<'_> {
            fn as_ref(&self) -> &Field {
                self.0.as_field()
            }
        }

        impl fmt::Debug for $view_mut<'_> {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_tuple(stringify!($view_mut))
                    .field(&format_args!("{}", self.0))
                    .finish()
            }
        }

        impl fmt::Display for $view_mut<'_> {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, formatter)
            }
        }
    };
}

for_each_well_known_protocol!(protocol_field_types);

#[cfg(test)]
mod tests {
    use crate::metadata::for_each_well_known_protocol;
    use crate::{DataType, Scheme};

    /// Assert one named pair reaches the key its own scheme spells.
    macro_rules! assert_protocol_prefix {
        ($name:ident, $mutable:ident, $constant:ident, $view:ident, $view_mut:ident, $label:literal) => {
            let mut field = DataType::Int64.required_field("probe");
            let key = format!("{}:x", Scheme::$constant.as_str());

            field.$mutable().insert("x", "1").unwrap();
            // The literal key is what proves the accessor, the newtype and the
            // scheme constant of one list entry agree; both `prefix` calls
            // would still agree if all three drifted together.
            assert_eq!(field.get_metadata(&key), Some("1"));
            assert_eq!(field.$name().prefix(), Scheme::$constant.as_str());
            assert_eq!(field.$mutable().prefix(), Scheme::$constant.as_str());
            assert_eq!(field.$name().key("x"), key);

            let view = field.$name();
            let mut looped = Vec::new();
            for (name, value) in &view {
                looped.push((name, value));
            }
            assert_eq!(looped, [("x", "1")]);
        };
    }

    #[test]
    fn every_named_protocol_view_spells_its_own_scheme_prefix() {
        for_each_well_known_protocol!(assert_protocol_prefix);
    }
}
