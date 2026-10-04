//! One table bucket as a warehouse catalog: its namespaces one level below
//! it, its Iceberg tables below them, and every commit to one published
//! through the control plane's metadata location.
//!
//! [`S3TablesCatalog`] and [`S3TablesNamespace`] answer the warehouse traits
//! ([`CatalogValue`], [`NamespaceValue`], [`ObjectValue`]) the way
//! [`IcebergCatalog`](crate::iceberg::IcebergCatalog) does for a folder. A
//! table either answers is an [`IcebergTable`] over a [`Handle`] on the
//! table's warehouse location - an `s3://...--table-s3` location reached
//! through the [`s3`](crate::s3) backend under the catalog's session and
//! properties - whose current document a [`MetadataPointer`] over
//! `GetTableMetadataLocation` and `UpdateTableMetadataLocation` names. A
//! warehouse location takes `PutObject` and `GetObject`; nothing here lists
//! it or deletes from it: a table's own `ls` is refused, and its `remove`
//! is the catalog's [`S3Tables::remove_table`], which drops it.
//!
//! # A location
//!
//! `s3tables://<bucket>[/<namespace>[/<table>]]` names the bucket's catalog,
//! one of its namespaces, or one of its tables - a trailing slash names
//! nothing, and more than two segments below the bucket are refused at
//! `$.url`. A table bucket's ARN names the catalog, and a table's ARN,
//! `arn:<partition>:s3tables:<region>:<account>:bucket/<name>/table/<id>`,
//! the table that identifier is. That ARN is read as itself: the location
//! it lowers to ([`Arn::locator`]) spells the identifier where a namespace
//! goes. One reading answers every door that takes a location -
//! [`Holder::from_url`](crate::holder::Holder::from_url), an identifier used
//! as a handle, [`IcebergTable::from_url`] and its two creating forms.
//!
//! # The request count
//!
//! | verb | requests |
//! | --- | --- |
//! | building the catalog, a namespace or a table description | none |
//! | the location of the bucket or of a namespace | none |
//! | the location of a table | 1 `GetTableMetadataLocation`, and no `GetNamespace` |
//! | a table's ARN | 1 `GetTable` |
//! | [`IcebergTable::create_from_url`] | [`create_table`](NamespaceValue::create_table)'s; under a namespace the bucket does not hold, the refused `CreateTable`, 1 `CreateNamespace` and those again |
//! | a table's [`remove`](crate::IOBase::remove) | 1 `DeleteTable` |
//! | the bucket's ARN, when the catalog was named by its location alone | 1 `ListTableBuckets` per page, once |
//! | [`children`](NamespaceValue::children) of the catalog | 1 `ListNamespaces` per page |
//! | [`get`](NamespaceValue::get) of a namespace | 1 `GetNamespace` |
//! | [`create_namespace`](NamespaceValue::create_namespace) | 1 `CreateNamespace` |
//! | [`children`](NamespaceValue::children) of a namespace | 1 `ListTables` per page, 1 `GetTableMetadataLocation` per table as its turn comes |
//! | [`get`](NamespaceValue::get) of a table | 1 `GetTableMetadataLocation` |
//! | [`create_table`](NamespaceValue::create_table) | `CreateTable`, `GetTableMetadataLocation`, 1 `PutObject` and `UpdateTableMetadataLocation` |
//! | a table's first read | 1 `GetTableMetadataLocation` and 1 `GetObject` |
//! | a commit | its files' `PutObject`s and 1 `UpdateTableMetadataLocation`; a refused one 1 `GetTableMetadataLocation` and 1 `GetObject` more |

use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use smol_str::{SmolStr, format_smolstr};

use super::S3Tables;
use super::namespace::check_namespace;
use super::table::{bucket_of, check_table};
use crate::aws::Session;
use crate::iceberg::{FormatVersion, IcebergTable, MetadataPointer, PartitionSpec, PointerState};
use crate::warehouse::{Handle, Site, extended, path_text};
use crate::{
    Arn, ArnPartition, Catalog, CatalogValue, Error, Field, IOKind, Namespace, NamespaceValue,
    Object, ObjectValue, Objects, Properties, Result, Scheme, Table, Uri, Url,
};

/// The properties a catalog reads the table bucket's ARN from, in order:
/// PyIceberg's S3 Tables catalog's name, then the REST catalog's.
const WAREHOUSE_PROPERTIES: [&str; 2] = ["s3tables.warehouse", "warehouse"];

/// The property naming the account a table bucket named by its location
/// alone belongs to, which saves the listing that finds it.
const ACCOUNT_PROPERTY: &str = "account_id";

/// The prefix PyIceberg's S3 Tables catalog spells its client's settings
/// under: `s3tables.region`, `s3tables.endpoint`, `s3tables.profile-name`.
const PREFIX: &str = "s3tables.";

/// The property a catalog named by a location is called by, as
/// [`Catalog::from_url`] reads it; the bucket's own name otherwise.
const NAME_PROPERTY: &str = "name";

/// What a location in a table bucket spells, as a refusal names it.
const GRAMMAR: &str = "s3tables://<bucket>[/<namespace>[/<table>]]";

/// What a location names below its table bucket.
enum Below {
    /// Nothing: the bucket itself.
    Bucket,
    /// One of its namespaces.
    Namespace(SmolStr),
    /// A table, by its namespace and its name.
    Table(SmolStr, SmolStr),
    /// A table, by the ARN the service identifies it with.
    Identified(Arn),
}

/// A location in a table bucket, read once: the bucket, its ARN where the
/// location was given as one, and what it names below the bucket.
struct Place {
    /// `s3tables://<bucket>`.
    bucket: Url,
    /// The bucket's ARN: the location's own, or the one a table's ARN is
    /// under.
    arn: Option<Arn>,
    below: Below,
}

impl Place {
    /// Read `location`: an `s3tables:` URL, a table bucket's ARN, or a
    /// table's ARN - read as the ARN, never as the location it lowers to.
    fn read(location: &Uri) -> Result<Self> {
        if location.scheme() == &Scheme::ARN {
            let arn = Arn::from_uri(location.clone())?;
            if arn.service() == super::client::SERVICE && arn.table().is_some() {
                let bucket = bucket_of(&arn)?;
                return Ok(Self {
                    bucket: bucket_location(&bucket)?,
                    arn: Some(bucket),
                    below: Below::Identified(arn),
                });
            }
            return Ok(Self {
                bucket: bucket_location(&arn)?,
                arn: Some(arn),
                below: Below::Bucket,
            });
        }
        let url = location.locator()?;
        let refuse = |reason: SmolStr| Error::InvalidRecord {
            path: SmolStr::new_static("$.url"),
            reason: format_smolstr!(
                "expected {GRAMMAR}, got {}: {reason}",
                crate::fs::mask_uri(&url.to_string())
            ),
        };
        if !url.scheme().is_s3_tables() {
            return Err(refuse(SmolStr::new_static(
                "the location is not a table bucket's",
            )));
        }
        // A name the service has no namespace or table of is refused here,
        // at no request: a table's ARN lowered to a location spells its
        // identifier where a namespace goes, and is such a name.
        let named = |kind: &str, name: &str, checked: Result<()>| {
            checked.map(|()| SmolStr::new(name)).map_err(|error| {
                refuse(format_smolstr!(
                    "{name:?} names no {kind} ({error}); a table's ARN is read as the ARN, \
                     never as the location it lowers to"
                ))
            })
        };
        let mut segments = url.path_segments().filter(|segment| !segment.is_empty());
        let below = match (segments.next(), segments.next(), segments.next()) {
            (None, ..) => Below::Bucket,
            (Some(namespace), None, _) => {
                Below::Namespace(named("namespace", namespace, check_namespace(namespace))?)
            }
            (Some(namespace), Some(table), None) => Below::Table(
                named("namespace", namespace, check_namespace(namespace))?,
                named("table", table, check_table(table))?,
            ),
            (Some(_), Some(_), Some(_)) => {
                return Err(refuse(SmolStr::new_static(
                    "a table bucket holds namespaces one level deep, and tables below them",
                )));
            }
        };
        Ok(Self {
            bucket: Url::from_str(&format!("s3tables://{}", url.authority().host()))?,
            arn: None,
            below,
        })
    }

    /// The bucket's catalog under `properties`: called what the `name`
    /// property says, else what the bucket is.
    fn catalog(&self, properties: &Properties) -> Result<S3TablesCatalog> {
        let name = match properties.get(NAME_PROPERTY) {
            Some(name) if !name.is_empty() => SmolStr::new(name),
            _ => bucket_name(&self.bucket),
        };
        S3TablesCatalog::from_location(name, &self.bucket, self.arn.clone(), properties)
    }
}

/// What a location names in an Amazon S3 Tables table bucket, under
/// `properties`: the one reading every door that takes a location answers
/// through.
///
/// `s3tables://<bucket>` and a table bucket's ARN are the bucket's
/// [`S3TablesCatalog`], and `s3tables://<bucket>/<namespace>` one of its
/// namespaces - a description each, and no request: a namespace the bucket
/// does not hold says so on its first verb.
/// `s3tables://<bucket>/<namespace>/<table>` is the table, at one
/// `GetTableMetadataLocation`; a table's ARN is the table that identifier
/// is, at one `GetTable`, whose answer names its namespace, its name and its
/// warehouse location. Neither answer primes the table's pointer: an object
/// may be read long after it is built, so its first read asks where its
/// document is again.
///
/// The properties are read as [`S3TablesCatalog::from_location`] reads them,
/// and the catalog is called what the `name` property says, else what the
/// bucket is.
///
/// # Errors
///
/// Returns [`Error::InvalidRecord`] at `$.url` for a location that spells
/// more than a namespace and a table below its bucket, or a name the service
/// has no namespace or table of; [`Error::Parse`] for an ARN that names
/// neither a table bucket nor a table; [`Error::Absent`] when the bucket
/// keeps no such table; and the service's own refusal.
pub(crate) fn locate(location: &Uri, properties: &Properties) -> Result<Object> {
    let place = Place::read(location)?;
    let catalog = place.catalog(properties)?;
    match &place.below {
        Below::Bucket => Ok(Object::Catalog(Catalog::S3Tables(Box::new(catalog)))),
        Below::Namespace(namespace) => Ok(Object::Namespace(Namespace::S3Tables(Box::new(
            catalog.namespace(namespace),
        )))),
        Below::Table(namespace, name) => {
            catalog.namespace(namespace).table(name).map(Object::Table)
        }
        Below::Identified(arn) => {
            let described = catalog.client().get_table_by_arn(arn)?;
            catalog
                .namespace(described.namespace())
                .described(described.name(), described.warehouse_location())
                .map(|table| Object::Table(Table::Iceberg(Box::new(table))))
        }
    }
}

/// Create the table `s3tables://<bucket>/<namespace>/<table>` names, under
/// `properties`: what [`IcebergTable::create_from_url`] answers for a table
/// bucket's location.
///
/// The table is laid out as a create that states neither a version nor a
/// spec lays one out (`iceberg::create_layout`) and created as
/// [`NamespaceValue::create_table`] creates one, with one repair that is
/// this door's alone: a `CreateTable` the service answers
/// `NotFoundException` - the namespace is not there - is followed by one
/// `CreateNamespace`, whose conflict is another creator's success, and sent
/// once more. The table states nothing of its own: the properties are its
/// catalog's, less the ones the session read.
///
/// # Errors
///
/// Returns [`Error::InvalidRecord`] at `$.url` for a location that names a
/// bucket or a namespace rather than a table, and for a table's ARN, which
/// names a table that exists; the schema's and the `format-version`
/// property's refusals; [`Error::Conflict`] when the namespace already holds
/// the table; and the service's own refusal.
pub(crate) fn create(
    location: &Uri,
    properties: &Properties,
    version: Option<FormatVersion>,
    schema: &Field,
    spec: Option<PartitionSpec>,
) -> Result<IcebergTable<Handle>> {
    let place = Place::read(location)?;
    let Below::Table(namespace, name) = &place.below else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$.url"),
            reason: format_smolstr!(
                "expected the location to create a table at, \
                 s3tables://<bucket>/<namespace>/<table>, got {}{}",
                crate::fs::mask_uri(&location.to_string()),
                match place.below {
                    Below::Identified(_) => "; a table's ARN names a table that exists",
                    _ => "",
                }
            ),
        });
    };
    let (schema, spec, version) = crate::iceberg::create_layout(schema, properties, version, spec)?;
    let namespace = place.catalog(properties)?.namespace(namespace);
    match namespace.register(name) {
        // What is missing is above the table: the namespace, made here, or
        // the bucket, whose absence the namespace's creation then answers.
        Err(Error::Remote { code, .. }) if code == "NotFoundException" => {
            let bucket = &namespace.bucket;
            match bucket
                .client
                .create_namespace(bucket.arn()?, namespace.namespace())
            {
                Ok(()) => {}
                Err(error) if error.is_conflict() => {}
                Err(error) => return Err(error),
            }
            namespace.register(name)?;
        }
        registered => registered?,
    }
    namespace.publish_first(name, schema, spec, version, &Properties::new())
}

/// What every object below one table bucket shares: the client, where the
/// bucket is, and its ARN once known - resolved at most once for the
/// catalog and everything it answered.
#[derive(Debug)]
struct Bucket {
    client: S3Tables,
    /// `s3tables://<name>`, the location the catalog answers as its own.
    url: Url,
    name: SmolStr,
    /// The ARN as it was stated, when it was.
    stated: Option<Arn>,
    /// The account a bucket named by its location belongs to, when stated.
    account: Option<String>,
    /// The ARN a bucket named by its location resolved to.
    resolved: OnceLock<Arn>,
}

impl Bucket {
    /// The bucket's ARN: as stated, else built from the account stated and
    /// the client's region, else found by name among the caller's own
    /// table buckets in that region - one listing, once.
    fn arn(&self) -> Result<&Arn> {
        if let Some(arn) = &self.stated {
            return Ok(arn);
        }
        if let Some(arn) = self.resolved.get() {
            return Ok(arn);
        }
        let arn = match &self.account {
            Some(account) => {
                let region = self.client.region_of(None)?;
                Arn::from_str(&format!(
                    "arn:{}:s3tables:{region}:{account}:bucket/{}",
                    ArnPartition::from_region(&region).as_str(),
                    self.name
                ))?
            }
            None => self
                .client
                .table_buckets()
                .find_map(|bucket| match bucket {
                    Ok(bucket) if bucket.name() == self.name => Some(Ok(bucket.arn().clone())),
                    Ok(_) => None,
                    Err(error) => Some(Err(error)),
                })
                .unwrap_or_else(|| Err(Error::absent("table bucket", &self.url)))?,
        };
        Ok(self.resolved.get_or_init(|| arn))
    }

    /// What identifies the bucket: its location and, when stated, its ARN
    /// and account.
    fn identity(&self) -> (&Url, Option<&Arn>, Option<&str>) {
        (&self.url, self.stated.as_ref(), self.account.as_deref())
    }
}

/// An Amazon S3 Tables table bucket, read as a warehouse catalog: its
/// namespaces one level below it ([`CatalogValue::namespace_levels`] is
/// `Some(1)`), each holding Iceberg tables.
///
/// Constructing one touches nothing. Equality and the hash read the
/// description - the path, the bucket's location, its ARN and account where
/// stated, the description and the stated properties - never the session.
/// The service keeps no properties for a bucket or a namespace, so
/// [`ObjectValue::properties`] answers what was stated, and every table's
/// storage opens under it: the [`s3`](crate::s3) backend reads the store's
/// own names (`s3.endpoint`, `s3.region`, ...) off it.
///
/// ```
/// use yggdryl::aws::{Credentials, Session};
/// use yggdryl::s3tables::{S3Tables, S3TablesCatalog};
/// use yggdryl::{Arn, CatalogValue, ObjectValue};
///
/// # fn main() -> yggdryl::Result<()> {
/// let session = Session::new()
///     .with_environment(false)
///     .with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "a-secret"));
/// let lake = Arn::from_str("arn:aws:s3tables:eu-west-3:123456789012:bucket/lake")?;
/// let catalog = S3TablesCatalog::new("lake", S3Tables::new(session), lake)?;
/// assert_eq!(catalog.url().map(ToString::to_string).as_deref(), Some("s3tables://lake"));
/// assert_eq!(catalog.namespace_levels(), Some(1));
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct S3TablesCatalog {
    path: Vec<SmolStr>,
    bucket: Arc<Bucket>,
    description: Option<String>,
    stated: Properties,
}

impl S3TablesCatalog {
    /// The catalog `name` over the table bucket `bucket`, reached through
    /// `client`, touching nothing.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when `bucket` is not a table bucket's ARN -
    /// `arn:<partition>:s3tables:<region>:<account>:bucket/<name>`.
    pub fn new(name: impl Into<SmolStr>, client: S3Tables, bucket: Arn) -> Result<Self> {
        let url = bucket_location(&bucket)?;
        Ok(Self::over(
            name.into(),
            client,
            url.clone(),
            bucket_name(&url),
            Some(bucket),
            None,
        ))
    }

    /// The catalog a `s3tables://<bucket>` location names, under
    /// `properties`: what [`Catalog::from_url`](crate::Catalog::from_url)
    /// answers for one, `located` the table bucket's ARN when the location
    /// was given as it.
    ///
    /// Who signs is [`Session::from_properties`] over them - with
    /// PyIceberg's `s3tables.`-prefixed names read after the bare ones, so
    /// `s3tables.profile-name` is a profile - and `s3tables.region` and
    /// `s3tables.endpoint` are the client's region and endpoint. The
    /// bucket's ARN is the one the location was given as, else the
    /// `s3tables.warehouse` or `warehouse` property where one names the
    /// bucket - the two refused where they disagree - else built from the
    /// `account_id` property and the client's region, else found by name
    /// among the caller's own table buckets on first use, since a bare
    /// location states no account and no region.
    ///
    /// The catalog keeps, and hands every namespace and table under it, the
    /// properties less the ones the session read: who signs is the session
    /// from here on, so no credential travels on in a bag a `Debug` or a
    /// `properties` listing prints, and the store a table's files are opened
    /// on signs as this session and no other.
    pub(crate) fn from_location(
        name: SmolStr,
        url: &Url,
        located: Option<Arn>,
        properties: &Properties,
    ) -> Result<Self> {
        if !url.path_segments().all(str::is_empty) {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.url"),
                reason: format_smolstr!(
                    "expected an S3 Tables table bucket's location, s3tables://<bucket>, got {url}; \
                     a namespace or a table below it is no catalog"
                ),
            });
        }
        let bucket = bucket_name(url);
        let prefixed = properties
            .iter()
            .filter_map(|(key, value)| Some((key.strip_prefix(PREFIX)?, value)));
        let session = Session::from_properties(properties.iter().chain(prefixed))?;
        let mut client = S3Tables::new(session);
        if let Some(region) = properties.get("s3tables.region") {
            client = client.with_region(region);
        }
        if let Some(endpoint) = properties.get("s3tables.endpoint") {
            client = client.try_with_endpoint_url(endpoint)?;
        }
        let mut stated = None;
        for key in WAREHOUSE_PROPERTIES {
            let Some(text) = properties.get(key) else {
                continue;
            };
            let refuse = |reason: SmolStr| Error::InvalidRecord {
                path: format_smolstr!("$.with.{key}"),
                reason,
            };
            let arn = Arn::from_str(text).map_err(|error| {
                refuse(format_smolstr!(
                    "expected the table bucket's ARN, arn:<partition>:s3tables:<region>:<account>:bucket/{bucket}, got {text:?}: {error}"
                ))
            })?;
            if bucket_location(&arn).ok().as_ref() != Some(url) {
                return Err(refuse(format_smolstr!(
                    "expected the ARN of the table bucket {bucket}, got {text:?}"
                )));
            }
            if let Some(located) = located.as_ref().filter(|located| **located != arn) {
                return Err(refuse(format_smolstr!(
                    "expected the ARN the location states, {located}, got {text:?}"
                )));
            }
            stated = Some(arn);
            break;
        }
        let stated = located.or(stated);
        let account = properties
            .get(ACCOUNT_PROPERTY)
            .map(str::trim)
            .filter(|account| !account.is_empty())
            .map(str::to_owned);
        let kept: Properties = properties
            .iter()
            .filter(|(name, _)| !Session::is_property(name.strip_prefix(PREFIX).unwrap_or(name)))
            .collect();
        Ok(Self::over(name, client, url.clone(), bucket, stated, account).with_properties(kept))
    }

    fn over(
        name: SmolStr,
        client: S3Tables,
        url: Url,
        bucket: SmolStr,
        stated: Option<Arn>,
        account: Option<String>,
    ) -> Self {
        Self {
            path: vec![name],
            bucket: Arc::new(Bucket {
                client,
                url,
                name: bucket,
                stated,
                account,
                resolved: OnceLock::new(),
            }),
            description: None,
            stated: Properties::new(),
        }
    }

    /// Return this catalog with a description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Return this catalog with stated properties, which every table's
    /// storage below it opens with.
    #[must_use]
    pub fn with_properties(mut self, properties: Properties) -> Self {
        self.stated = properties;
        self
    }

    /// The client every request of the catalog is sent through.
    pub fn client(&self) -> &S3Tables {
        &self.bucket.client
    }

    /// The table bucket's ARN: as stated, else resolved once - see
    /// [`S3TablesCatalog`]'s request count.
    ///
    /// # Errors
    ///
    /// Returns the region's refusal, the listing's failure, and
    /// [`Error::Absent`] when the caller has no table bucket of the name in
    /// the region.
    pub fn bucket_arn(&self) -> Result<&Arn> {
        self.bucket.arn()
    }

    /// The namespace `name` of the bucket, described: no request.
    fn namespace(&self, name: &str) -> S3TablesNamespace {
        namespace_of(&self.bucket, extended(&self.path, name), &self.stated)
    }
}

impl PartialEq for S3TablesCatalog {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
            && self.bucket.identity() == other.bucket.identity()
            && self.description == other.description
            && self.stated == other.stated
    }
}

impl Eq for S3TablesCatalog {}

impl Hash for S3TablesCatalog {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.path.hash(state);
        self.bucket.identity().hash(state);
        self.description.hash(state);
        self.stated.hash(state);
    }
}

impl ObjectValue for S3TablesCatalog {
    fn name(&self) -> &str {
        &self.path[0]
    }

    fn path(&self) -> &[SmolStr] {
        &self.path
    }

    fn kind(&self) -> IOKind {
        IOKind::Catalog
    }

    fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    fn url(&self) -> Option<&Url> {
        Some(&self.bucket.url)
    }

    /// What was stated: the service keeps no properties for a bucket.
    fn properties(&self) -> Result<Properties> {
        Ok(self.stated.clone())
    }
}

impl NamespaceValue for S3TablesCatalog {
    /// The bucket's namespaces, one `ListNamespaces` per page as drained.
    fn children(&self) -> Objects {
        let arn = match self.bucket.arn() {
            Ok(arn) => arn.clone(),
            Err(error) => return Objects::failing(error),
        };
        let (bucket, path, effective) = (
            Arc::clone(&self.bucket),
            self.path.clone(),
            self.stated.clone(),
        );
        Objects::new(self.bucket.client.namespaces(&arn).map(move |summary| {
            let summary = summary?;
            Ok(Object::Namespace(Namespace::S3Tables(Box::new(
                namespace_of(&bucket, extended(&path, summary.name()), &effective),
            ))))
        }))
    }

    /// The namespace `name`, one `GetNamespace`.
    fn get(&self, name: &str) -> Result<Object> {
        let below = extended(&self.path, name);
        match self.bucket.client.get_namespace(self.bucket.arn()?, name) {
            Ok(_) => Ok(Object::Namespace(Namespace::S3Tables(Box::new(
                namespace_of(&self.bucket, below, &self.stated),
            )))),
            Err(error) if error.is_absent() => Err(Error::absent("namespace", path_text(&below))),
            Err(error) => Err(error),
        }
    }

    /// Create the namespace `name`, one `CreateNamespace`. The service keeps
    /// no properties for it: `properties` are stated on the namespace
    /// answered, and every table's storage below it opens with them.
    fn create_namespace(&self, name: &str, properties: &Properties) -> Result<Namespace> {
        let below = extended(&self.path, name);
        match self
            .bucket
            .client
            .create_namespace(self.bucket.arn()?, name)
        {
            Ok(()) => {}
            Err(error) if error.is_conflict() => {
                return Err(Error::conflict("namespace", "namespace", path_text(&below)));
            }
            Err(error) => return Err(error),
        }
        Ok(Namespace::S3Tables(Box::new(
            namespace_of(&self.bucket, below, &self.stated).with_properties(properties.clone()),
        )))
    }
}

impl CatalogValue for S3TablesCatalog {
    fn namespace_levels(&self) -> Option<usize> {
        Some(1)
    }
}

/// One namespace of a table bucket: the Iceberg tables it holds.
///
/// Constructing one touches nothing; the service keeps nothing for it but
/// its name, so its effective properties are its catalog's, then what was
/// stated for it.
#[derive(Clone, Debug)]
pub struct S3TablesNamespace {
    path: Vec<SmolStr>,
    bucket: Arc<Bucket>,
    stated: Properties,
    inherited: Properties,
}

impl S3TablesNamespace {
    /// Return this namespace with stated properties, which every table's
    /// storage below it opens with.
    #[must_use]
    pub fn with_properties(mut self, properties: Properties) -> Self {
        self.stated = properties;
        self
    }

    /// The namespace with its parent's effective properties pushed into it.
    pub(crate) fn inheriting(mut self, parent: &Properties) -> Self {
        self.inherited = parent.clone();
        self
    }

    /// The namespace's own name: its path's last part.
    fn namespace(&self) -> &str {
        self.path.last().map_or("", SmolStr::as_str)
    }

    fn effective(&self) -> Properties {
        self.stated.inherit(&self.inherited)
    }

    /// The table `name` as `GetTableMetadataLocation` describes it: one
    /// request, the table's document read on its first use.
    fn table(&self, name: &str) -> Result<Table> {
        let location = match self.bucket.client.get_table_metadata_location(
            self.bucket.arn()?,
            self.namespace(),
            name,
        ) {
            Ok(location) => location,
            Err(error) if error.is_absent() => {
                return Err(Error::absent(
                    "table",
                    path_text(&extended(&self.path, name)),
                ));
            }
            Err(error) => return Err(error),
        };
        self.described(name, location.warehouse_location())
            .map(|table| Table::Iceberg(Box::new(table)))
    }

    /// The table `name` whose files are under `warehouse`, described: no
    /// request, its pointer asked where its document is on its first use.
    fn described(&self, name: &str, warehouse: &Url) -> Result<IcebergTable<Handle>> {
        let below = extended(&self.path, name);
        let pointer = TablePointer::shared(
            &self.bucket,
            self.bucket.arn()?,
            self.namespace(),
            name,
            None,
        );
        let root = self.root(warehouse, &below, &Properties::new())?;
        Ok(IcebergTable::at(below, root)
            .pointed(pointer)
            .inheriting(&self.effective()))
    }

    /// Register the table `name` with the service, with no document: one
    /// `CreateTable`, a table already there a conflict by its warehouse
    /// path.
    fn register(&self, name: &str) -> Result<()> {
        match self
            .bucket
            .client
            .create_table(self.bucket.arn()?, self.namespace(), name, None)
        {
            Ok(_) => Ok(()),
            Err(error) if error.is_conflict() => Err(Error::conflict(
                "table",
                "table",
                path_text(&extended(&self.path, name)),
            )),
            Err(error) => Err(error),
        }
    }

    /// Write and publish the first document of the registered table `name`:
    /// `schema` - as Iceberg states it, and numbered - under `spec` at
    /// `version`, the table stating `properties`.
    ///
    /// `GetTableMetadataLocation` answers the warehouse the document is
    /// written into and the token it is named current under. A creation
    /// whose first document is not published removes the registration again
    /// under that token, which a publication that took has moved past.
    fn publish_first(
        &self,
        name: &str,
        schema: Field,
        spec: PartitionSpec,
        version: FormatVersion,
        properties: &Properties,
    ) -> Result<IcebergTable<Handle>> {
        let below = extended(&self.path, name);
        let arn = self.bucket.arn()?;
        let client = &self.bucket.client;
        let location = client.get_table_metadata_location(arn, self.namespace(), name)?;
        let state = PointerState::new(
            location.metadata_location().cloned(),
            location.version_token(),
        );
        let pointer = TablePointer::shared(&self.bucket, arn, self.namespace(), name, Some(state));
        let root = self.root(location.warehouse_location(), &below, properties)?;
        match IcebergTable::create_pointed(root, version, schema, spec, pointer) {
            Ok(table) => Ok(table
                .placed(below)
                .with_properties(properties.clone())
                .inheriting(&self.effective())),
            Err(error) => {
                drop(client.remove_table(
                    arn,
                    self.namespace(),
                    name,
                    Some(location.version_token()),
                ));
                Err(error)
            }
        }
    }

    /// The handle on a table's warehouse location: the store opened under
    /// the catalog's session, in the bucket's region, and under the
    /// effective properties.
    fn root(&self, warehouse: &Url, path: &[SmolStr], stated: &Properties) -> Result<Handle> {
        let region = self.bucket.client.region_of(Some(self.bucket.arn()?))?;
        Ok(Handle::at(
            Site::Store {
                url: warehouse.clone(),
                session: self.bucket.client.session().clone(),
                region,
            },
            false,
            path,
            stated.inherit(&self.effective()),
        ))
    }
}

impl PartialEq for S3TablesNamespace {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
            && self.bucket.identity() == other.bucket.identity()
            && self.stated == other.stated
            && self.inherited == other.inherited
    }
}

impl Eq for S3TablesNamespace {}

impl Hash for S3TablesNamespace {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.path.hash(state);
        self.bucket.identity().hash(state);
        self.stated.hash(state);
        self.inherited.hash(state);
    }
}

impl ObjectValue for S3TablesNamespace {
    fn name(&self) -> &str {
        self.namespace()
    }

    fn path(&self) -> &[SmolStr] {
        &self.path
    }

    fn kind(&self) -> IOKind {
        IOKind::Namespace
    }

    /// The catalog's, then what was stated: the service keeps nothing.
    fn properties(&self) -> Result<Properties> {
        Ok(self.effective())
    }
}

impl NamespaceValue for S3TablesNamespace {
    /// The namespace's tables: one `ListTables` per page, and one
    /// `GetTableMetadataLocation` per table as its turn comes.
    fn children(&self) -> Objects {
        let arn = match self.bucket.arn() {
            Ok(arn) => arn.clone(),
            Err(error) => return Objects::failing(error),
        };
        let namespace = self.clone();
        Objects::new(
            self.bucket
                .client
                .tables(&arn, Some(self.namespace()))
                .map(move |summary| Ok(Object::Table(namespace.table(summary?.name())?))),
        )
    }

    /// The table `name`, one `GetTableMetadataLocation`.
    fn get(&self, name: &str) -> Result<Object> {
        self.table(name).map(Object::Table)
    }

    /// Create the Iceberg table `name` from `field`: the schema as Iceberg
    /// states it - a layout the format does not state rewritten to the one
    /// it does, the columns numbered above the highest identifier present -
    /// partitioned as it declares ([`PartitionSpec::from_schema`]), at the
    /// format version `properties` state under `format-version`, else the
    /// lowest that states the schema: 3 for a nanosecond timestamp, a
    /// variant or an unknown column, else 2.
    ///
    /// `CreateTable` registers the table with no document, and its first
    /// document - `metadata/00000-{uuid}.metadata.json` under the warehouse
    /// location `GetTableMetadataLocation` answers - is written and named
    /// current under that answer's token, so the schema, the spec and the
    /// sort order the table keeps are the crate's own. A creation whose
    /// first document is not published removes the table again under that
    /// token, which a publication that took has moved past.
    fn create_table(&self, name: &str, field: &Field, properties: &Properties) -> Result<Table> {
        let (schema, spec, version) = crate::iceberg::create_layout(field, properties, None, None)?;
        self.register(name)?;
        self.publish_first(name, schema, spec, version, properties)
            .map(|table| Table::Iceberg(Box::new(table)))
    }
}

/// The namespace at `path` of `bucket`, carrying `inherited`.
fn namespace_of(
    bucket: &Arc<Bucket>,
    path: Vec<SmolStr>,
    inherited: &Properties,
) -> S3TablesNamespace {
    S3TablesNamespace {
        path,
        bucket: Arc::clone(bucket),
        stated: Properties::new(),
        inherited: inherited.clone(),
    }
}

/// The metadata location of one table, as the control plane keeps it.
#[derive(Debug)]
struct TablePointer {
    client: S3Tables,
    bucket: Arn,
    namespace: String,
    name: String,
    /// The answer a creation already read, taken by the first `current`
    /// instead of a second request: the creation publishes under it at
    /// once, so it cannot have gone stale.
    primed: Mutex<Option<PointerState>>,
}

impl TablePointer {
    /// The pointer of the table `name`, shared by every clone of it.
    fn shared(
        bucket: &Bucket,
        arn: &Arn,
        namespace: &str,
        name: &str,
        primed: Option<PointerState>,
    ) -> Arc<dyn MetadataPointer> {
        Arc::new(Self {
            client: bucket.client.clone(),
            bucket: arn.clone(),
            namespace: namespace.to_owned(),
            name: name.to_owned(),
            primed: Mutex::new(primed),
        })
    }
}

impl MetadataPointer for TablePointer {
    /// One `GetTableMetadataLocation`, or the answer a creation primed.
    fn current(&self) -> Result<PointerState> {
        if let Some(state) = self.primed.lock().ok().and_then(|mut primed| primed.take()) {
            return Ok(state);
        }
        let answer =
            self.client
                .get_table_metadata_location(&self.bucket, &self.namespace, &self.name)?;
        Ok(PointerState::new(
            answer.metadata_location().cloned(),
            answer.version_token(),
        ))
    }

    /// One `UpdateTableMetadataLocation`; the service's `ConflictException`,
    /// a token the table has moved past, is the commit conflict.
    fn publish(&self, token: &str, location: &Url) -> Result<PointerState> {
        if let Ok(mut primed) = self.primed.lock() {
            *primed = None;
        }
        match self.client.update_table_metadata_location(
            &self.bucket,
            &self.namespace,
            &self.name,
            token,
            location,
        ) {
            Ok(version) => Ok(PointerState::new(
                Some(location.clone()),
                version.version_token(),
            )),
            Err(Error::Remote { code, .. }) if code == "ConflictException" => Err(Error::conflict(
                "table version",
                "a newer table version",
                format!("{}/{}/{}", self.bucket, self.namespace, self.name),
            )),
            Err(error) => Err(error),
        }
    }

    /// One `DeleteTable`, under no version token: the table is dropped
    /// wherever it stands, and one already gone is dropped.
    fn remove(&self) -> Result<()> {
        self.client
            .remove_table(&self.bucket, &self.namespace, &self.name, None)
    }
}

/// `s3tables://<name>` of a table bucket's ARN, refusing any other ARN.
fn bucket_location(bucket: &Arn) -> Result<Url> {
    if bucket.service() != "s3tables" || bucket.table().is_some() || bucket.bucket().is_none() {
        return Err(Error::Parse {
            target: "s3tables bucket",
            position: 0,
            reason: format_smolstr!(
                "expected a table bucket's ARN, arn:<partition>:s3tables:<region>:<account>:bucket/<name>, got {bucket}"
            ),
        });
    }
    bucket.locator()
}

/// The bucket a `s3tables://<name>` location names.
fn bucket_name(url: &Url) -> SmolStr {
    SmolStr::new(url.authority().host())
}
