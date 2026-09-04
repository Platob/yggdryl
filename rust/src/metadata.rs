use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, btree_map, btree_map::Entry};
use std::fmt;
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::ops::{Bound, Index};
use std::str::FromStr;
use std::sync::{Arc, OnceLock};

use serde::de::{Error as DeError, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::{SmolStr, format_smolstr};

use crate::{AsciiEnum, Error, Result, Scheme, Url, stable_hash_display};

pub(crate) const ALIAS_KEY: &str = "alias";
pub(crate) const COMMENT_KEY: &str = "comment";
pub(crate) const DISPLAY_KEY: &str = "display";
pub(crate) const HTTP_ACCEPT_ENCODING_KEY: &str = "http:accept-encoding";
pub(crate) const HTTP_ACCEPT_KEY: &str = "http:accept";
pub(crate) const HTTP_ACCEPT_LANGUAGE_KEY: &str = "http:accept-language";
pub(crate) const HTTP_ACCEPT_RANGES_KEY: &str = "http:accept-ranges";
pub(crate) const HTTP_CACHE_CONTROL_KEY: &str = "http:cache-control";
pub(crate) const HTTP_CONTENT_DISPOSITION_KEY: &str = "http:content-disposition";
pub(crate) const HTTP_CONTENT_ENCODING_KEY: &str = "http:content-encoding";
pub(crate) const HTTP_CONTENT_LANGUAGE_KEY: &str = "http:content-language";
pub(crate) const HTTP_CONTENT_LENGTH_KEY: &str = "http:content-length";
pub(crate) const HTTP_CONTENT_LOCATION_KEY: &str = "http:content-location";
pub(crate) const HTTP_CONTENT_RANGE_KEY: &str = "http:content-range";
pub(crate) const HTTP_CONTENT_TYPE_KEY: &str = "http:content-type";
pub(crate) const HTTP_ETAG_KEY: &str = "http:etag";
pub(crate) const HTTP_EXPIRES_KEY: &str = "http:expires";
pub(crate) const HTTP_LAST_MODIFIED_KEY: &str = "http:last-modified";
pub(crate) const HTTP_LOCATION_KEY: &str = "http:location";
pub(crate) const HTTP_RANGE_KEY: &str = "http:range";
pub(crate) const HTTP_VARY_KEY: &str = "http:vary";
pub(crate) const LOCATION_KEY: &str = "location";
pub(crate) const FIELD_ENUM_KEY: &str = "field:enum";
pub(crate) const FIELD_INIT_KEY: &str = "field:init";
pub(crate) const FIELD_PARTITION_KEY: &str = "field:partition";
pub(crate) const PARQUET_FIELD_ID_KEY: &str = "PARQUET:field_id";

type MetadataMap = BTreeMap<String, String>;

/// Apply `$emit` to every protocol that has a named metadata view.
///
/// One list drives every generated accessor and every named view type, so a
/// protocol added here appears on the metadata snapshot, on [`crate::Field`]
/// and in [`crate::types::protocol`] in the same change rather than in
/// whichever of them someone remembered. `https` is deliberately absent: it
/// shares the canonical `http:` prefix, and one spelling of one namespace is
/// what keeps a header from being stored twice.
///
/// Every emitter matches all six tokens even where it ignores some, which is
/// what forces the accessors and the view types to grow together.
macro_rules! for_each_well_known_protocol {
    ($emit:ident) => {
        $emit!(
            as_http,
            as_http_mut,
            HTTP,
            HttpField,
            HttpFieldMut,
            "HTTP and HTTPS representation"
        );
        $emit!(
            as_file,
            as_file_mut,
            FILE,
            FileField,
            FileFieldMut,
            "file protocol"
        );
        $emit!(
            as_urn,
            as_urn_mut,
            URN,
            UrnField,
            UrnFieldMut,
            "uniform resource name"
        );
        $emit!(
            as_postgres,
            as_postgres_mut,
            POSTGRES,
            PostgresField,
            PostgresFieldMut,
            "short-spelling PostgreSQL"
        );
        $emit!(
            as_postgresql,
            as_postgresql_mut,
            POSTGRESQL,
            PostgresqlField,
            PostgresqlFieldMut,
            "long-spelling PostgreSQL"
        );
        $emit!(
            as_mysql,
            as_mysql_mut,
            MYSQL,
            MysqlField,
            MysqlFieldMut,
            "MySQL"
        );
        $emit!(
            as_arrow_properties,
            as_arrow_properties_mut,
            ARROW,
            ArrowPropertyField,
            ArrowPropertyFieldMut,
            "Arrow property"
        );
        $emit!(
            as_sql,
            as_sql_mut,
            SQL,
            SqlField,
            SqlFieldMut,
            "generic SQL"
        );
        $emit!(
            as_glue,
            as_glue_mut,
            GLUE,
            GlueField,
            GlueFieldMut,
            "AWS Glue"
        );
        $emit!(
            as_iceberg,
            as_iceberg_mut,
            ICEBERG,
            IcebergField,
            IcebergFieldMut,
            "Apache Iceberg"
        );
        $emit!(
            as_fix,
            as_fix_mut,
            FIX,
            FixField,
            FixFieldMut,
            "Financial Information eXchange"
        );
        $emit!(
            as_field_properties,
            as_field_properties_mut,
            FIELD,
            FieldPropertiesField,
            FieldPropertiesFieldMut,
            "Yggdryl field"
        );
        $emit!(as_s3, as_s3_mut, S3, S3Field, S3FieldMut, "Amazon S3");
        $emit!(
            as_gs,
            as_gs_mut,
            GS,
            GsField,
            GsFieldMut,
            "Google Cloud Storage"
        );
        $emit!(
            as_az,
            as_az_mut,
            AZ,
            AzField,
            AzFieldMut,
            "Azure Blob Storage"
        );
        $emit!(
            as_spark,
            as_spark_mut,
            SPARK,
            SparkField,
            SparkFieldMut,
            "Apache Spark"
        );
        $emit!(
            as_polars,
            as_polars_mut,
            POLARS,
            PolarsField,
            PolarsFieldMut,
            "Polars"
        );
        $emit!(
            as_pandas,
            as_pandas_mut,
            PANDAS,
            PandasField,
            PandasFieldMut,
            "pandas"
        );
    };
}

pub(crate) use for_each_well_known_protocol;

/// Emit one borrowed protocol view accessor on a metadata snapshot.
macro_rules! metadata_protocol_accessor {
    ($name:ident, $mutable:ident, $constant:ident, $view:ident, $view_mut:ident, $label:literal) => {
        #[doc = concat!("Returns the borrowed ", $label, " property view.")]
        ///
        /// This is [`Self::protocol`] with the protocol already chosen. A bare
        /// snapshot has no field behind it, so this answers a
        /// [`ProtocolMetadata`] rather than the specialized field view
        #[doc = concat!("[`", stringify!($view), "`](crate::types::protocol::", stringify!($view), "),")]
        /// and a typed vocabulary that view would carry is a
        /// [`get`](ProtocolMetadata::get) by bare name here.
        pub fn $name(&self) -> ProtocolMetadata<'_> {
            self.protocol(&Scheme::$constant)
        }
    };
}

static EMPTY_METADATA: OnceLock<Arc<MetadataMap>> = OnceLock::new();

fn empty_metadata() -> Arc<MetadataMap> {
    Arc::clone(EMPTY_METADATA.get_or_init(|| Arc::new(BTreeMap::new())))
}

/// Immutable, deterministic field metadata shared copy-on-write by [`crate::Field`].
///
/// Keys and values are always owned, non-null strings. Empty values remain
/// valid for arbitrary Arrow metadata, while typed reserved values are
/// validated by the same path used by every constructor and mutation.
#[derive(Clone)]
pub struct Metadata(Arc<MetadataMap>);

impl Metadata {
    /// Returns the shared empty metadata value without a per-call allocation.
    pub fn new() -> Self {
        Self(empty_metadata())
    }

    /// Constructs metadata from unique string entries.
    pub fn from_entries<I, K, V>(values: I) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let mut entries = BTreeMap::new();
        for (key, value) in values {
            let (key, value) = validate_entry(key.into(), value.into())?;
            match entries.entry(key) {
                Entry::Vacant(entry) => {
                    entry.insert(value);
                }
                Entry::Occupied(entry) => {
                    return Err(Error::DuplicateMetadataKey(entry.key().as_str().into()));
                }
            }
        }
        Ok(Self::from_map(entries))
    }

    /// Imports Arrow's borrowed metadata map.
    pub fn from_arrow(values: &HashMap<String, String>) -> Result<Self> {
        Self::from_entries(values)
    }

    /// Deserializes and validates deterministic structural JSON metadata.
    pub fn from_json(value: &str) -> Result<Self> {
        serde_json::from_str(value).map_err(Error::from)
    }

    /// Returns the number of entries.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether there are no entries.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns a borrowed value by key.
    ///
    /// Canonical metadata lookup is exact. HTTP field names additionally use
    /// their protocol-defined ASCII case-insensitive comparison, while the
    /// stored key remains one lowercase `http:<field-name>` spelling. The
    /// canonical spelling takes one exact allocation-free tree lookup; only a
    /// noncanonical HTTP lookup allocates its lowercase search key.
    pub fn get(&self, key: &str) -> Option<&str> {
        let key = canonical_http_lookup_key(key);
        self.0.get(key.as_ref()).map(String::as_str)
    }

    /// Returns whether a key exists without allocating.
    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Iterates in lexical key order without allocating.
    pub fn iter(&self) -> MetadataIter<'_> {
        MetadataIter(self.0.iter())
    }

    /// Returns whether both values share the same immutable backing map.
    pub(crate) fn shares_storage_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Returns the first entry after `after_key`, or the first entry for `None`.
    ///
    /// This cursor form lets owning FFI iterators advance without repeated
    /// indexed scans or rebuilding a key collection.
    pub fn next_entry(&self, after_key: Option<&str>) -> Option<(&str, &str)> {
        let entry = match after_key {
            Some(key) => {
                let key = canonical_http_lookup_key(key);
                self.0
                    .range::<str, _>((Bound::Excluded(key.as_ref()), Bound::Unbounded))
                    .next()
            }
            None => self.0.first_key_value(),
        }?;
        Some((entry.0.as_str(), entry.1.as_str()))
    }

    /// Looks up a protocol-prefixed property without allocating a full key.
    ///
    /// The lookup key is assembled inline - a `scheme:name` pair short enough
    /// for [`SmolStr`]'s inline storage never allocates - and answered by one
    /// exact tree lookup rather than by scanning the protocol's range. HTTP
    /// keeps its protocol-defined ASCII case-insensitive comparison, because
    /// [`Self::get`] canonicalizes an `http:` key before it looks it up.
    pub fn get_property(&self, scheme: &Scheme, name: &str) -> Option<&str> {
        self.get(&property_lookup_key(scheme, name))
    }

    /// Returns whether a protocol-prefixed property exists.
    pub fn has_property(&self, scheme: &Scheme, name: &str) -> bool {
        self.get_property(scheme, name).is_some()
    }

    /// Iterates over one protocol's property names and values without allocating.
    pub fn property_iter<'metadata, 'scheme>(
        &'metadata self,
        scheme: &'scheme Scheme,
    ) -> PropertyIter<'metadata, 'scheme> {
        let prefix = protocol_metadata_prefix(scheme);
        PropertyIter {
            entries: self
                .0
                .range::<str, _>((Bound::Included(prefix), Bound::Unbounded)),
            prefix,
            finished: false,
        }
    }

    /// Returns the first protocol property after `after_name`.
    pub fn next_property_entry<'metadata>(
        &'metadata self,
        scheme: &Scheme,
        after_name: Option<&str>,
    ) -> Option<(&'metadata str, &'metadata str)> {
        let Some(after_name) = after_name else {
            return self.property_iter(scheme).next();
        };
        let prefix = protocol_metadata_prefix(scheme);
        let after_name = if prefix == Scheme::HTTP.as_str()
            && after_name.bytes().any(|byte| byte.is_ascii_uppercase())
        {
            Cow::Owned(after_name.to_ascii_lowercase())
        } else {
            Cow::Borrowed(after_name)
        };
        let lower = format_smolstr!("{prefix}:{after_name}");
        for (key, value) in self
            .0
            .range::<str, _>((Bound::Excluded(lower.as_str()), Bound::Unbounded))
        {
            match property_key_position(key, prefix) {
                PropertyKeyPosition::Match(name) => return Some((name, value)),
                PropertyKeyPosition::Before => {}
                PropertyKeyPosition::After => return None,
            }
        }
        None
    }

    /// Returns the metadata carrying both this snapshot's entries and `other`'s.
    ///
    /// The union of the two, and this snapshot wins a key they disagree on.
    /// That asymmetry is the whole rule: a merge that silently took the other
    /// side would make a receiver's own declarations conditional on what it
    /// was merged against, and one that refused outright would make combining
    /// two descriptions of the same column impossible whenever they annotate
    /// it differently.
    ///
    /// Every entry goes through the validation an ordinary write uses, so a
    /// merge cannot assemble metadata that would have been refused.
    ///
    /// # Errors
    ///
    /// Returns an error when a merged entry fails that validation.
    pub fn merge_with(&self, other: &Self) -> Result<Self> {
        // `other` lays the entries down and this snapshot overwrites them, so
        // a shared key keeps this value while every key only `other` has still
        // arrives. The map resolves the clash because `from_entries` refuses a
        // duplicate key rather than picking a winner.
        let mut entries: BTreeMap<&str, &str> = other.iter().collect();
        entries.extend(self.iter());
        Self::from_entries(entries)
    }

    /// Returns a borrowed view of one protocol's properties.
    ///
    /// The view remembers the protocol, so every read spells the bare property
    /// name and the `scheme:` prefix is applied once, by the view. Nothing is
    /// copied: the value borrows this snapshot and answers from the same tree.
    ///
    /// ```
    /// use yggdryl::{Metadata, Scheme};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let metadata = Metadata::from_entries([("iceberg:doc", "closing price")])?;
    ///
    /// assert_eq!(metadata.protocol(&Scheme::ICEBERG).get("doc"), Some("closing price"));
    /// assert_eq!(metadata.as_iceberg().get("doc"), Some("closing price"));
    /// assert_eq!(metadata.as_iceberg().key("doc"), "iceberg:doc");
    /// # Ok(())
    /// # }
    /// ```
    pub fn protocol(&self, scheme: &Scheme) -> ProtocolMetadata<'_> {
        ProtocolMetadata {
            metadata: self,
            scheme: scheme.clone(),
        }
    }

    for_each_well_known_protocol!(metadata_protocol_accessor);

    /// Returns the shared human-readable comment.
    ///
    /// The one straight description, belonging to no protocol. Every protocol
    /// view falls back to it, so one sentence written once is what every
    /// catalog reads.
    pub fn comment(&self) -> Option<&str> {
        self.get(COMMENT_KEY)
    }

    /// Returns the shared human-readable display name.
    ///
    /// The one straight name a reader is shown, belonging to no protocol.
    /// Every protocol view falls back to it, so one name written once is what
    /// every catalog shows.
    pub fn display(&self) -> Option<&str> {
        self.get(DISPLAY_KEY)
    }

    /// Consumes and serializes this snapshot as deterministic structural JSON.
    pub fn into_json(self) -> Result<String> {
        serde_json::to_string(&self).map_err(Error::from)
    }

    /// Returns a deterministic cross-language hash of canonical display output.
    pub fn stable_hash(&self) -> u64 {
        stable_hash_display(self)
    }

    /// Consumes this snapshot and projects it to Arrow metadata.
    ///
    /// When uniquely owned, key and value allocations move directly into the
    /// Arrow map. Shared snapshots clone only because Arrow's map is mutable.
    pub fn into_arrow(self) -> HashMap<String, String> {
        self.into_map().into_iter().collect()
    }

    pub(crate) fn insert(&mut self, key: String, value: String) -> Result<(Option<String>, bool)> {
        let (key, value) = validate_entry(key, value)?;
        if self.get(&key) == Some(value.as_str()) {
            return Ok((Some(value), false));
        }
        Ok((Arc::make_mut(&mut self.0).insert(key, value), true))
    }

    pub(crate) fn insert_validated(
        &mut self,
        key: String,
        value: String,
    ) -> (Option<String>, bool) {
        if self.get(&key) == Some(value.as_str()) {
            return (Some(value), false);
        }
        (Arc::make_mut(&mut self.0).insert(key, value), true)
    }

    pub(crate) fn remove(&mut self, key: &str) -> Option<String> {
        let key = canonical_http_lookup_key(key);
        self.get(key.as_ref())?;
        let previous = Arc::make_mut(&mut self.0).remove(key.as_ref());
        if self.0.is_empty() {
            self.0 = empty_metadata();
        }
        previous
    }

    pub(crate) fn update(&mut self, overlay: Self) -> bool {
        if overlay.is_empty()
            || overlay
                .iter()
                .all(|(key, value)| self.get(key) == Some(value))
        {
            return false;
        }
        let target = Arc::make_mut(&mut self.0);
        target.extend(overlay.into_map());
        true
    }

    pub(crate) fn clear(&mut self) -> bool {
        if self.is_empty() {
            false
        } else {
            self.0 = empty_metadata();
            true
        }
    }

    pub(crate) fn remove_properties(&mut self, scheme: &Scheme) -> bool {
        if self.property_iter(scheme).next().is_none() {
            return false;
        }
        let prefix = protocol_metadata_prefix(scheme);
        Arc::make_mut(&mut self.0).retain(|key, _| property_name(key, prefix).is_none());
        if self.0.is_empty() {
            self.0 = empty_metadata();
        }
        true
    }

    pub(crate) fn matches_arrow(&self, values: &HashMap<String, String>) -> bool {
        self.len() == values.len()
            && self
                .iter()
                .all(|(key, value)| values.get(key).is_some_and(|other| other == value))
    }

    fn from_map(values: MetadataMap) -> Self {
        if values.is_empty() {
            Self::new()
        } else {
            Self(Arc::new(values))
        }
    }

    fn into_map(self) -> MetadataMap {
        Arc::try_unwrap(self.0).unwrap_or_else(|values| values.as_ref().clone())
    }
}

impl Default for Metadata {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Metadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_map().entries(self.iter()).finish()
    }
}

impl fmt::Display for Metadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("{")?;
        for (index, (key, value)) in self.iter().enumerate() {
            if index != 0 {
                formatter.write_str(",")?;
            }
            write_json_string(formatter, key)?;
            formatter.write_str(":")?;
            write_json_string(formatter, value)?;
        }
        formatter.write_str("}")
    }
}

impl PartialEq for Metadata {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0 == other.0
    }
}

impl Eq for Metadata {}

impl PartialOrd for Metadata {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Metadata {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        if Arc::ptr_eq(&self.0, &other.0) {
            std::cmp::Ordering::Equal
        } else {
            self.0.cmp(&other.0)
        }
    }
}

impl Hash for Metadata {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl Serialize for Metadata {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.len()))?;
        for (key, value) in self.iter() {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Metadata {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MetadataVisitor;

        impl<'de> Visitor<'de> for MetadataVisitor {
            type Value = Metadata;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object with unique, non-empty string keys")
            }

            fn visit_map<A>(self, mut map: A) -> std::result::Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut entries = Vec::with_capacity(map.size_hint().unwrap_or_default());
                while let Some((key, value)) = map.next_entry::<String, String>()? {
                    entries.push((key, value));
                }
                Metadata::from_entries(entries).map_err(A::Error::custom)
            }
        }

        deserializer.deserialize_map(MetadataVisitor)
    }
}

impl FromStr for Metadata {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::from_json(value)
    }
}

impl Index<&str> for Metadata {
    type Output = str;

    fn index(&self, key: &str) -> &Self::Output {
        self.get(key)
            .unwrap_or_else(|| panic!("metadata key {key:?} is not present"))
    }
}

impl TryFrom<HashMap<String, String>> for Metadata {
    type Error = Error;

    fn try_from(values: HashMap<String, String>) -> Result<Self> {
        Self::from_entries(values)
    }
}

impl TryFrom<&HashMap<String, String>> for Metadata {
    type Error = Error;

    fn try_from(values: &HashMap<String, String>) -> Result<Self> {
        Self::from_arrow(values)
    }
}

impl TryFrom<BTreeMap<String, String>> for Metadata {
    type Error = Error;

    fn try_from(values: BTreeMap<String, String>) -> Result<Self> {
        Self::from_entries(values)
    }
}

impl From<Metadata> for HashMap<String, String> {
    fn from(value: Metadata) -> Self {
        value.into_arrow()
    }
}

impl From<Metadata> for BTreeMap<String, String> {
    fn from(value: Metadata) -> Self {
        value.into_map()
    }
}

impl AsRef<BTreeMap<String, String>> for Metadata {
    fn as_ref(&self) -> &BTreeMap<String, String> {
        &self.0
    }
}

impl<'metadata> IntoIterator for &'metadata Metadata {
    type Item = (&'metadata str, &'metadata str);
    type IntoIter = MetadataIter<'metadata>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl IntoIterator for Metadata {
    type Item = (String, String);
    type IntoIter = MetadataIntoIter;

    fn into_iter(self) -> Self::IntoIter {
        MetadataIntoIter(self.into_map().into_iter())
    }
}

/// A borrowed lexical metadata iterator.
#[derive(Clone)]
pub struct MetadataIter<'a>(btree_map::Iter<'a, String, String>);

impl<'a> Iterator for MetadataIter<'a> {
    type Item = (&'a str, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        self.0
            .next()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl DoubleEndedIterator for MetadataIter<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0
            .next_back()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }
}

impl ExactSizeIterator for MetadataIter<'_> {}
impl std::iter::FusedIterator for MetadataIter<'_> {}

/// A consuming lexical metadata iterator.
pub struct MetadataIntoIter(btree_map::IntoIter<String, String>);

impl Iterator for MetadataIntoIter {
    type Item = (String, String);

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl DoubleEndedIterator for MetadataIntoIter {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back()
    }
}

impl ExactSizeIterator for MetadataIntoIter {}
impl std::iter::FusedIterator for MetadataIntoIter {}

/// A borrowed iterator over the suffixes of one protocol's metadata keys.
#[derive(Clone)]
pub struct PropertyIter<'metadata, 'scheme> {
    entries: btree_map::Range<'metadata, String, String>,
    prefix: &'scheme str,
    finished: bool,
}

impl<'metadata> Iterator for PropertyIter<'metadata, '_> {
    type Item = (&'metadata str, &'metadata str);

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        for (key, value) in self.entries.by_ref() {
            match property_key_position(key, self.prefix) {
                PropertyKeyPosition::Match(name) => return Some((name, value)),
                PropertyKeyPosition::Before => {}
                PropertyKeyPosition::After => {
                    self.finished = true;
                    return None;
                }
            }
        }
        self.finished = true;
        None
    }
}

impl DoubleEndedIterator for PropertyIter<'_, '_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        while let Some((key, value)) = self.entries.next_back() {
            match property_key_position(key, self.prefix) {
                PropertyKeyPosition::Match(name) => return Some((name, value)),
                PropertyKeyPosition::After => {}
                PropertyKeyPosition::Before => {
                    self.finished = true;
                    return None;
                }
            }
        }
        self.finished = true;
        None
    }
}

impl std::iter::FusedIterator for PropertyIter<'_, '_> {}

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
    metadata: &'metadata Metadata,
    scheme: Scheme,
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

/// Return the full `scheme:name` key one property is stored under.
pub(crate) fn property_key(scheme: &Scheme, name: &str) -> String {
    let prefix = protocol_metadata_prefix(scheme);
    let mut key = String::with_capacity(prefix.len() + 1 + name.len());
    key.push_str(prefix);
    key.push(':');
    key.push_str(name);
    key
}

/// Return the same key without allocating for a name of ordinary length.
fn property_lookup_key(scheme: &Scheme, name: &str) -> SmolStr {
    format_smolstr!("{}:{name}", protocol_metadata_prefix(scheme))
}

pub(crate) fn property_name<'a>(key: &'a str, scheme: &str) -> Option<&'a str> {
    key.strip_prefix(scheme)?.strip_prefix(':')
}

enum PropertyKeyPosition<'a> {
    Before,
    Match(&'a str),
    After,
}

fn property_key_position<'a>(key: &'a str, scheme: &str) -> PropertyKeyPosition<'a> {
    let Some(suffix) = key.strip_prefix(scheme) else {
        return PropertyKeyPosition::After;
    };
    let Some(first) = suffix.as_bytes().first() else {
        return PropertyKeyPosition::Before;
    };
    match first.cmp(&b':') {
        std::cmp::Ordering::Less => PropertyKeyPosition::Before,
        std::cmp::Ordering::Equal => PropertyKeyPosition::Match(&suffix[1..]),
        std::cmp::Ordering::Greater => PropertyKeyPosition::After,
    }
}

fn validate_entry(key: String, value: String) -> Result<(String, String)> {
    let key = canonicalize_metadata_key(key)?;
    if key.is_empty() {
        return Err(Error::EmptyMetadataKey);
    }
    let value = match key.as_str() {
        ALIAS_KEY | COMMENT_KEY | DISPLAY_KEY => {
            validate_reserved_text(&key, &value)?;
            value
        }
        LOCATION_KEY => Url::from_str(&value)?.to_string(),
        FIELD_ENUM_KEY => parse_ascii_enum(&value)?.into_json(),
        FIELD_INIT_KEY => parse_reserved_bool(FIELD_INIT_KEY, &value)?.to_string(),
        FIELD_PARTITION_KEY => parse_reserved_bool(FIELD_PARTITION_KEY, &value)?.to_string(),
        PARQUET_FIELD_ID_KEY => parse_field_id(&value)?.to_string(),
        _ => {
            if key.starts_with("http:") {
                validate_http_header_value(&key, &value)?;
                if key == HTTP_CONTENT_LENGTH_KEY {
                    return Ok((key, parse_content_length(&value)?.to_string()));
                }
            }
            if let Some((prefix, name)) = key.split_once(':') {
                if Scheme::from_str(prefix).is_ok_and(|scheme| scheme.as_str() == prefix) {
                    validate_property_part(&key, "property name", name)?;
                }
            }
            value
        }
    };
    Ok((key, value))
}

fn canonicalize_metadata_key(mut key: String) -> Result<String> {
    if let Some((prefix, name)) = http_header_parts(&key) {
        validate_http_header_name(&key, name)?;
        let prefix_len = prefix.len();
        key.replace_range(..prefix_len, Scheme::HTTP.as_str());
        key.make_ascii_lowercase();
    }
    Ok(key)
}

fn canonical_http_lookup_key(key: &str) -> Cow<'_, str> {
    let Some((prefix, _)) = http_header_parts(key) else {
        return Cow::Borrowed(key);
    };
    if prefix == Scheme::HTTP.as_str() && !key.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Cow::Borrowed(key);
    }
    let mut canonical = key.to_owned();
    canonical.replace_range(..prefix.len(), Scheme::HTTP.as_str());
    canonical.make_ascii_lowercase();
    Cow::Owned(canonical)
}

fn http_header_parts(key: &str) -> Option<(&str, &str)> {
    let (prefix, name) = key.split_once(':')?;
    is_http_metadata_prefix(prefix).then_some((prefix, name))
}

fn is_http_metadata_prefix(prefix: &str) -> bool {
    prefix.eq_ignore_ascii_case(Scheme::HTTP.as_str())
        || prefix.eq_ignore_ascii_case(Scheme::HTTPS.as_str())
}

pub(crate) fn protocol_metadata_prefix(scheme: &Scheme) -> &str {
    if scheme == &Scheme::HTTPS {
        Scheme::HTTP.as_str()
    } else {
        scheme.as_str()
    }
}

fn validate_http_header_name(key: &str, name: &str) -> Result<()> {
    if !name.is_empty() && name.bytes().all(is_http_token_byte) {
        return Ok(());
    }
    Err(Error::InvalidMetadataValue {
        key: SmolStr::new(key),
        reason: SmolStr::new_static(
            "HTTP field name must be a non-empty ASCII token without a colon",
        ),
    })
}

const fn is_http_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

fn validate_http_header_value(key: &str, value: &str) -> Result<()> {
    if value
        .bytes()
        .all(|byte| byte == b'\t' || (byte >= b' ' && byte != 0x7f))
    {
        return Ok(());
    }
    Err(Error::InvalidMetadataValue {
        key: SmolStr::new(key),
        reason: SmolStr::new_static(
            "HTTP field value must not contain CR, LF, NUL, DEL, or controls other than HTAB",
        ),
    })
}

pub(crate) fn parse_content_length(value: &str) -> Result<u64> {
    if value.is_empty() || value.bytes().any(|byte| !byte.is_ascii_digit()) {
        return Err(invalid_content_length());
    }
    value.parse().map_err(|_| invalid_content_length())
}

fn invalid_content_length() -> Error {
    Error::InvalidMetadataValue {
        key: SmolStr::new_static(HTTP_CONTENT_LENGTH_KEY),
        reason: SmolStr::new_static("must be an unsigned 64-bit decimal integer"),
    }
}

/// Parse the enum document a field's ASCII values are named by.
///
/// The stored spelling is the one [`AsciiEnum::into_json`] renders, so a
/// document that reaches storage reads back as the enum that wrote it.
pub(crate) fn parse_ascii_enum(value: &str) -> Result<AsciiEnum> {
    AsciiEnum::from_json(value).map_err(|error| Error::InvalidMetadataValue {
        key: SmolStr::new_static(FIELD_ENUM_KEY),
        reason: SmolStr::new(error.to_string()),
    })
}

pub(crate) fn parse_field_id(value: &str) -> Result<i32> {
    value.parse().map_err(|_| Error::InvalidMetadataValue {
        key: SmolStr::new_static(PARQUET_FIELD_ID_KEY),
        reason: SmolStr::new_static("must be a signed 32-bit decimal integer"),
    })
}

fn validate_reserved_text(key: &str, value: &str) -> Result<()> {
    validate_property_part(key, "value", value)
}

/// Parse a reserved boolean metadata value.
///
/// Reserved booleans are stored in exactly one canonical spelling so a reader
/// never has to guess between `true`, `True`, `1`, and `yes`.
pub(crate) fn parse_reserved_bool(key: &str, value: &str) -> Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(Error::InvalidMetadataValue {
            key: SmolStr::new(key),
            reason: crate::text::expected_got("true or false", format_args!("{other:?}")),
        }),
    }
}

fn validate_property_part(key: &str, label: &str, value: &str) -> Result<()> {
    let reason = if value.is_empty() {
        Some(format!("{label} must not be empty"))
    } else if value.chars().any(char::is_control) {
        Some(format!("{label} must not contain control characters"))
    } else {
        None
    };
    if let Some(reason) = reason {
        return Err(Error::InvalidMetadataValue {
            key: SmolStr::new(key),
            reason: SmolStr::new(reason),
        });
    }
    Ok(())
}

pub(crate) fn write_json_string(formatter: &mut fmt::Formatter<'_>, value: &str) -> fmt::Result {
    formatter.write_str("\"")?;
    for character in value.chars() {
        match character {
            '"' => formatter.write_str("\\\"")?,
            '\\' => formatter.write_str("\\\\")?,
            '\n' => formatter.write_str("\\n")?,
            '\r' => formatter.write_str("\\r")?,
            '\t' => formatter.write_str("\\t")?,
            '\u{08}' => formatter.write_str("\\b")?,
            '\u{0c}' => formatter.write_str("\\f")?,
            character if character.is_control() => {
                write!(formatter, "\\u{:04x}", u32::from(character))?;
            }
            character => formatter.write_char(character)?,
        }
    }
    formatter.write_str("\"")
}

#[cfg(test)]
mod tests {
    use std::hash::{Hash, Hasher};
    use std::sync::Arc;

    use super::Metadata;
    use crate::Scheme;

    #[test]
    fn empty_and_cloned_metadata_share_their_backing_map() {
        let empty = Metadata::new();
        let other_empty = Metadata::new();
        assert!(Arc::ptr_eq(&empty.0, &other_empty.0));

        let metadata = Metadata::from_entries([("source", "orders")]).unwrap();
        let clone = metadata.clone();
        assert!(Arc::ptr_eq(&metadata.0, &clone.0));
    }

    #[test]
    fn unique_arrow_projection_moves_string_allocations() {
        let key = "protocol-property-key-longer-than-inline-storage";
        let value = "protocol-property-value-longer-than-inline-storage";
        let metadata = Metadata::from_entries([(key, value)]).unwrap();
        let (stored_key, stored_value) = metadata.iter().next().unwrap();
        let key_pointer = stored_key.as_ptr();
        let value_pointer = stored_value.as_ptr();

        let arrow = metadata.into_arrow();
        let (arrow_key, arrow_value) = arrow.iter().next().unwrap();
        assert_eq!(arrow_key.as_ptr(), key_pointer);
        assert_eq!(arrow_value.as_ptr(), value_pointer);
    }

    #[test]
    fn protocol_iteration_is_exact_sorted_double_ended_and_cursor_compatible() {
        let metadata = Metadata::from_entries([
            ("postgre", "before"),
            ("postgres", "plain"),
            ("postgres-prefix", "before-colon"),
            ("postgres:alpha", "a"),
            ("postgres:middle", "m"),
            ("postgres:omega", "z"),
            ("postgres0", "before-colon"),
            ("postgresql:alpha", "different-scheme"),
            ("z:last", "after"),
        ])
        .unwrap();

        let mut properties = metadata.property_iter(&Scheme::POSTGRES);
        assert_eq!(properties.next(), Some(("alpha", "a")));
        assert_eq!(properties.next_back(), Some(("omega", "z")));
        assert_eq!(properties.next(), Some(("middle", "m")));
        assert_eq!(properties.next(), None);
        assert_eq!(properties.next_back(), None);

        assert_eq!(
            metadata.next_property_entry(&Scheme::POSTGRES, Some("alpha")),
            Some(("middle", "m"))
        );
        assert_eq!(
            metadata.next_property_entry(&Scheme::POSTGRES, Some("omega")),
            None
        );
        assert_eq!(metadata.get_property(&Scheme::POSTGRES, "alpha"), Some("a"));
        assert_eq!(metadata.get_property(&Scheme::POSTGRES, "missing"), None);
    }

    #[test]
    fn protocol_cursor_visits_every_wide_property_once() {
        let metadata = Metadata::from_entries(
            (0..1_024).map(|index| (format!("postgres:key-{index:04}"), index.to_string())),
        )
        .unwrap();
        let mut after = None;
        let mut count = 0;
        while let Some((name, value)) = metadata.next_property_entry(&Scheme::POSTGRES, after) {
            assert_eq!(name, format!("key-{count:04}"));
            assert_eq!(value, count.to_string());
            after = Some(name);
            count += 1;
        }
        assert_eq!(count, 1_024);
    }

    #[test]
    fn protocol_views_order_and_hash_the_properties_they_expose() {
        fn hash(value: &impl Hash) -> u64 {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            value.hash(&mut hasher);
            hasher.finish()
        }

        let first = Metadata::from_entries([("postgres:a", "1")]).unwrap();
        let equal =
            Metadata::from_entries([("postgres:a", "1"), ("s3:bucket", "ignored")]).unwrap();
        let later = Metadata::from_entries([("postgres:b", "1")]).unwrap();
        assert_eq!(first.as_postgres(), equal.as_postgres());
        assert_eq!(hash(&first.as_postgres()), hash(&equal.as_postgres()));
        assert!(first.as_postgres() < later.as_postgres());
    }

    #[test]
    fn http_keys_are_canonical_case_insensitive_and_collision_safe() {
        let mut metadata = Metadata::from_entries([
            ("HTTP:Content-Type", "text/plain; charset=utf-8"),
            ("HtTpS:X-Custom", "preserved"),
            ("http:Content-Length", "00042"),
        ])
        .unwrap();

        assert_eq!(
            metadata.iter().collect::<Vec<_>>(),
            [
                ("http:content-length", "42"),
                ("http:content-type", "text/plain; charset=utf-8"),
                ("http:x-custom", "preserved"),
            ]
        );
        assert_eq!(
            metadata.get("HTTP:CONTENT-TYPE"),
            Some("text/plain; charset=utf-8")
        );
        assert_eq!(
            metadata.get("HTTPS:CONTENT-TYPE"),
            Some("text/plain; charset=utf-8")
        );
        assert_eq!(
            metadata.get_property(&Scheme::HTTPS, "X-CUSTOM"),
            Some("preserved")
        );
        assert_eq!(
            metadata.property_iter(&Scheme::HTTPS).collect::<Vec<_>>(),
            [
                ("content-length", "42"),
                ("content-type", "text/plain; charset=utf-8"),
                ("x-custom", "preserved"),
            ]
        );
        assert_eq!(metadata.get("http:content-length"), Some("42"));
        assert_eq!(
            metadata.remove("HTTPS:CONTENT-TYPE").as_deref(),
            Some("text/plain; charset=utf-8")
        );
        assert!(!metadata.contains_key("http:content-type"));

        assert!(
            Metadata::from_entries([
                ("HTTPS:Content-Type", "text/plain"),
                ("HTTP:content-type", "application/json"),
            ])
            .is_err()
        );
    }

    #[test]
    fn http_values_reject_injection_but_allow_horizontal_tab() {
        let metadata = Metadata::from_entries([("HTTPS:X-Trace", "one\ttwo")]).unwrap();
        assert_eq!(metadata.get("http:x-trace"), Some("one\ttwo"));

        for value in ["a\0b", "a\nb", "a\rb", "a\u{1f}b", "a\u{7f}b"] {
            assert!(
                Metadata::from_entries([("https:x-trace", value)]).is_err(),
                "accepted HTTP control value {value:?}"
            );
        }
        for key in ["http:", "http:bad name", "HTTP:bad:name", "http:café"] {
            assert!(
                Metadata::from_entries([(key, "value")]).is_err(),
                "accepted invalid HTTP field name {key:?}"
            );
        }
    }

    #[test]
    fn content_length_requires_ascii_digits_and_u64_range() {
        assert_eq!(
            Metadata::from_entries([("http:content-length", u64::MAX.to_string())])
                .unwrap()
                .get("http:content-length"),
            Some("18446744073709551615")
        );
        for value in ["", "+1", "-1", " 1", "1 ", "١", "18446744073709551616"] {
            assert!(
                Metadata::from_entries([("http:content-length", value)]).is_err(),
                "accepted invalid Content-Length {value:?}"
            );
        }
    }
}
