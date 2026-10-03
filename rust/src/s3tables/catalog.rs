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
//! it or deletes from it.
//!
//! # The request count
//!
//! | verb | requests |
//! | --- | --- |
//! | building the catalog, a namespace or a table description | none |
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
use crate::aws::Session;
use crate::iceberg::{IcebergTable, MetadataPointer, PartitionSpec, PointerState};
use crate::warehouse::{Handle, Site, extended, path_text};
use crate::{
    Arn, ArnPartition, CatalogValue, Error, Field, IOKind, Namespace, NamespaceValue, Object,
    ObjectValue, Objects, Properties, Result, Table, Url,
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
    /// answers for one.
    ///
    /// Who signs is [`Session::from_properties`] over them - with
    /// PyIceberg's `s3tables.`-prefixed names read after the bare ones, so
    /// `s3tables.profile-name` is a profile - and `s3tables.region` and
    /// `s3tables.endpoint` are the client's region and endpoint. A location
    /// states no account and no region, so the bucket's ARN is the
    /// `s3tables.warehouse` or `warehouse` property where one names the
    /// bucket, else built from the `account_id` property and the client's
    /// region, else found by name among the caller's own table buckets on
    /// first use.
    pub(crate) fn from_location(name: SmolStr, url: &Url, properties: &Properties) -> Result<Self> {
        if !url.path_segments().all(str::is_empty) {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.url"),
                reason: format_smolstr!(
                    "expected an S3 Tables table bucket's location, s3tables://<bucket>, got {url}; \
                     a table is reached through the catalog of its bucket"
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
            stated = Some(arn);
            break;
        }
        let account = properties
            .get(ACCOUNT_PROPERTY)
            .map(str::trim)
            .filter(|account| !account.is_empty())
            .map(str::to_owned);
        Ok(
            Self::over(name, client, url.clone(), bucket, stated, account)
                .with_properties(properties.clone()),
        )
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
        let below = extended(&self.path, name);
        let arn = self.bucket.arn()?;
        let location =
            match self
                .bucket
                .client
                .get_table_metadata_location(arn, self.namespace(), name)
            {
                Ok(location) => location,
                Err(error) if error.is_absent() => {
                    return Err(Error::absent("table", path_text(&below)));
                }
                Err(error) => return Err(error),
            };
        let pointer = TablePointer::shared(&self.bucket, arn, self.namespace(), name, None);
        let root = self.root(location.warehouse_location(), &below, &Properties::new())?;
        Ok(Table::Iceberg(Box::new(
            IcebergTable::at(below, root)
                .pointed(pointer)
                .inheriting(&self.effective()),
        )))
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
        let below = extended(&self.path, name);
        let mut schema = field.clone().into_scheme_compat(&crate::Scheme::ICEBERG)?;
        let start = crate::iceberg::last_column_id(&schema)?.saturating_add(1);
        crate::iceberg::assign_field_ids(&mut schema, start)?;
        let spec = PartitionSpec::from_schema(0, &schema)?;
        let version = crate::iceberg::format_version_for(&schema, properties)?;
        let arn = self.bucket.arn()?;
        let client = &self.bucket.client;
        match client.create_table(arn, self.namespace(), name, None) {
            Ok(_) => {}
            Err(error) if error.is_conflict() => {
                return Err(Error::conflict("table", "table", path_text(&below)));
            }
            Err(error) => return Err(error),
        }
        let location = client.get_table_metadata_location(arn, self.namespace(), name)?;
        let state = PointerState::new(
            location.metadata_location().cloned(),
            location.version_token(),
        );
        let pointer = TablePointer::shared(&self.bucket, arn, self.namespace(), name, Some(state));
        let root = self.root(location.warehouse_location(), &below, properties)?;
        match IcebergTable::create_pointed(root, version, schema, spec, pointer) {
            Ok(table) => Ok(Table::Iceberg(Box::new(
                table
                    .placed(below)
                    .with_properties(properties.clone())
                    .inheriting(&self.effective()),
            ))),
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
