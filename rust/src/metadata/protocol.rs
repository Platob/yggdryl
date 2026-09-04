//! Borrowed protocol metadata views.

use super::*;

/// One protocol's properties, addressed by their bare names.
///
/// A protocol property is stored under a `scheme:name` key, and code that
/// spells that key by hand has to spell it right in every branch it appears in.
/// This view remembers the protocol once and applies it to every operation, so
/// a caller writes `doc` where it used to write `"iceberg:doc"`.
///
/// The view is a borrow, not a copy: it holds the same snapshot the metadata
/// holds and answers reads out of the same tree, so constructing one costs a
/// `Scheme` clone of a known protocol - which allocates nothing - and no map
/// walk at all. It is therefore cheap enough to build per call rather than
/// stored, which is what the named accessors such as
/// [`Metadata::as_iceberg`] do.
///
/// ```
/// use yggdryl::Metadata;
///
/// # fn main() -> yggdryl::Result<()> {
/// let metadata = Metadata::from_entries([
///     ("iceberg:doc", "closing price"),
///     ("iceberg:schema-id", "3"),
///     ("postgres:table", "trades"),
/// ])?;
///
/// let iceberg = metadata.as_iceberg();
/// assert_eq!(iceberg.len(), 2);
/// assert_eq!(iceberg.get("schema-id"), Some("3"));
/// assert!(!iceberg.contains_key("table"));
/// assert_eq!(
///     iceberg.iter().collect::<Vec<_>>(),
///     [("doc", "closing price"), ("schema-id", "3")],
/// );
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct ProtocolMetadata<'metadata> {
    pub(super) metadata: &'metadata Metadata,
    pub(super) scheme: Scheme,
}

impl<'metadata> ProtocolMetadata<'metadata> {
    /// Returns the protocol this view remembers.
    pub const fn scheme(&self) -> &Scheme {
        &self.scheme
    }

    /// Returns the canonical key prefix this view applies.
    ///
    /// This is the scheme's own spelling for every protocol but HTTPS, which
    /// shares HTTP's one namespace.
    pub fn prefix(&self) -> &str {
        protocol_metadata_prefix(&self.scheme)
    }

    /// Returns the full metadata key one property name is stored under.
    pub fn key(&self, name: &str) -> String {
        property_key(&self.scheme, name)
    }

    /// Returns one property value by its bare name.
    pub fn get(&self, name: &str) -> Option<&'metadata str> {
        self.metadata.get_property(&self.scheme, name)
    }

    /// Returns whether one property exists.
    pub fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Returns the number of properties this protocol holds.
    ///
    /// The count walks this protocol's contiguous key range rather than the
    /// whole snapshot, so it costs the properties it counts and not the
    /// metadata around them.
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// Returns whether this protocol holds no properties.
    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }

    /// Iterates this protocol's names and values in lexical order.
    pub fn iter(&self) -> PropertyIter<'metadata, '_> {
        self.metadata.property_iter(&self.scheme)
    }

    /// Returns the first property after `after_name`, or the first for `None`.
    ///
    /// This is the cursor form an owning FFI iterator advances with.
    pub fn next_entry(&self, after_name: Option<&str>) -> Option<(&'metadata str, &'metadata str)> {
        self.metadata.next_property_entry(&self.scheme, after_name)
    }

    /// Returns the complete snapshot this view reads from.
    pub const fn as_metadata(&self) -> &'metadata Metadata {
        self.metadata
    }

    /// Collects this protocol's properties as a standalone snapshot.
    ///
    /// Keys keep their `scheme:` prefix, so the result is a metadata value that
    /// merges back into any other with [`crate::Field::update_metadata`].
    ///
    /// # Errors
    ///
    /// Returns an error only when a property fails the validation it already
    /// passed, which externally corrupted serialized state can produce.
    pub fn into_metadata(self) -> Result<Metadata> {
        Metadata::from_entries(self.iter().map(|(name, value)| (self.key(name), value)))
    }

    /// Returns this protocol's comment, falling back to the straight one.
    ///
    /// A protocol that names its own `comment` answers it; one that does not
    /// answers the field's straight `comment`, so a description written once
    /// without a namespace is what every protocol reads.
    ///
    /// The fallback lives here rather than in [`Self::get`] on purpose: `get`,
    /// [`Self::iter`] and [`Self::len`] stay literal about what this protocol
    /// actually carries, so the view never reports a property that iterating
    /// it would not yield.
    pub fn comment(&self) -> Option<&'metadata str> {
        self.get(COMMENT_KEY).or_else(|| self.metadata.comment())
    }

    /// Returns this protocol's display name, falling back to the straight one.
    ///
    /// A protocol that names its own `display` answers it; one that does not
    /// answers the field's straight `display`, so a name written once without
    /// a namespace is what every protocol shows. [`Self::comment`] carries why
    /// the fallback lives here rather than in [`Self::get`].
    pub fn display(&self) -> Option<&'metadata str> {
        self.get(DISPLAY_KEY).or_else(|| self.metadata.display())
    }

    /// Returns this protocol's properties merged with `other`'s.
    ///
    /// Both views contribute their own bare names and the result is keyed
    /// under *this* view's protocol, so merging an `iceberg` view with a
    /// `glue` one answers Iceberg properties. This view wins a name they
    /// disagree on, exactly as [`Metadata::merge_with`] does.
    ///
    /// # Errors
    ///
    /// Returns an error when a merged property fails the validation every
    /// write goes through.
    pub fn merge_with(&self, other: &ProtocolMetadata<'_>) -> Result<Metadata> {
        let mut names: BTreeMap<&str, &str> = other.iter().collect();
        names.extend(self.iter());
        Metadata::from_entries(
            names
                .into_iter()
                .map(|(name, value)| (self.key(name), value)),
        )
    }
}

impl fmt::Debug for ProtocolMetadata<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ProtocolMetadata")
            .field(&self.prefix())
            .field(&format_args!("{self}"))
            .finish()
    }
}

impl fmt::Display for ProtocolMetadata<'_> {
    /// Renders this protocol's own names as a deterministic JSON object.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("{")?;
        for (index, (name, value)) in self.iter().enumerate() {
            if index != 0 {
                formatter.write_str(",")?;
            }
            write_json_string(formatter, name)?;
            formatter.write_str(":")?;
            write_json_string(formatter, value)?;
        }
        formatter.write_str("}")
    }
}

impl PartialEq for ProtocolMetadata<'_> {
    /// Compares the properties two views hold, not the snapshots behind them.
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}

impl Eq for ProtocolMetadata<'_> {}

impl PartialOrd for ProtocolMetadata<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ProtocolMetadata<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.iter().cmp(other.iter())
    }
}

impl Hash for ProtocolMetadata<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.len().hash(state);
        for entry in self.iter() {
            entry.hash(state);
        }
    }
}

impl Index<&str> for ProtocolMetadata<'_> {
    type Output = str;

    fn index(&self, name: &str) -> &Self::Output {
        self.get(name).unwrap_or_else(|| {
            panic!(
                "metadata property {:?} is not present",
                self.key(name).as_str()
            )
        })
    }
}

impl<'metadata, 'view> IntoIterator for &'view ProtocolMetadata<'metadata> {
    type Item = (&'metadata str, &'metadata str);
    type IntoIter = PropertyIter<'metadata, 'view>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
