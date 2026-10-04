//! Amazon Resource Names.

use super::*;

/// A validated AWS ARN backed directly by a canonical [`Uri`].
///
/// An ARN is a name in six fields - `arn:partition:service:region:account:resource` -
/// and the last of them carries its own `/` and `:` structure. The first five
/// are what AWS decides, so they are read as fields; the sixth is what the
/// service decides, so it is read as a path and the filename accessors work
/// over it exactly as they work over a [`Urn`]'s namespace-specific string.
///
/// `partition`, `service`, and `region` fold to lower case, the way a URN's
/// namespace does. `region` and `account` may be empty, which is how a global
/// service - IAM, Amazon S3, CloudFront - spells "every region" and "no owning
/// account". A policy wildcard (`*`, `?`) is a value a field may hold.
///
/// Two services name a place rather than a thing, and those two
/// [`locate`](Self::locator): Amazon S3, whose resource is a bucket and a key,
/// and Amazon S3 Tables, whose resource is a table bucket and a
/// [`table`](Self::table).
///
/// ```
/// use yggdryl::Arn;
///
/// # fn main() -> yggdryl::Result<()> {
/// let arn = Arn::from_str("arn:aws:s3:::trades/2026/part.parquet")?;
///
/// assert_eq!(arn.partition(), "aws");
/// assert_eq!(arn.service(), "s3");
/// assert_eq!(arn.region(), None);
/// assert_eq!(arn.account(), None);
/// assert_eq!(arn.resource(), "trades/2026/part.parquet");
/// assert_eq!(arn.bucket(), Some("trades"));
/// assert_eq!(arn.key(), Some("2026/part.parquet"));
/// assert_eq!(arn.extension(), Some("parquet"));
///
/// // An Amazon S3 ARN names what an `s3:` URL locates.
/// assert_eq!(arn.locator()?.to_string(), "s3://trades/2026/part.parquet");
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Arn(Uri);

/// The count of fields an ARN's path holds, its resource included.
const FIELDS: usize = 5;

/// The count of leading fields AWS decides, and therefore folds to lower case.
const FOLDED_FIELDS: usize = 3;

/// The byte offset of an ARN's first field: past `arn:`.
const FIELDS_OFFSET: usize = 4;

impl Arn {
    /// Parse and validate an ARN.
    ///
    /// # Errors
    ///
    /// As [`Self::from_uri`], plus the URI parser's own refusal.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Build a validated ARN from its five fields.
    ///
    /// `region` and `account` are written as the empty string when the service
    /// names neither.
    ///
    /// ```
    /// use yggdryl::Arn;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let arn = Arn::from_parts("aws", "iam", "", "123456789012", "user/David")?;
    /// assert_eq!(arn.to_string(), "arn:aws:iam::123456789012:user/David");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a parse error when a field is empty where the syntax requires
    /// one, holds a byte the field cannot carry, or spells an ARN that is not
    /// valid URI text.
    pub fn from_parts(
        partition: &str,
        service: &str,
        region: &str,
        account: &str,
        resource: &str,
    ) -> Result<Self> {
        let mut path = SmolStrBuilder::new();
        for field in [partition, service, region, account] {
            path.push_str(field);
            path.push(':');
        }
        path.push_str(resource);
        let path: SmolStr = path.into();
        Self::from_uri(Uri::from_parts(
            Scheme::ARN,
            Authority(SmolStr::new("")),
            UriPath::from_str(path.as_str())?,
            None,
            None,
        )?)
    }

    /// Validate and wrap an existing URI as an ARN.
    ///
    /// # Errors
    ///
    /// Returns a parse error naming the field that refused, at its byte
    /// position in the ARN text.
    pub fn from_uri(mut value: Uri) -> Result<Self> {
        if value.scheme() != &Scheme::ARN {
            return Err(parse_error("arn", 0, "ARN scheme must be `arn`"));
        }
        if value.has_authority() || !value.authority().is_empty() {
            return Err(parse_error(
                "arn",
                FIELDS_OFFSET,
                "ARN must not contain an authority",
            ));
        }
        let path = value.path().as_str();
        let tail = FIELDS_OFFSET + path.len();
        // A `?` or a `#` in a resource is that byte, never a component: the URI
        // parser has already split one off, so an ARN carrying either was
        // written with a byte its resource has to escape.
        if value.query.is_some() {
            return Err(parse_error(
                "arn",
                tail,
                "ARN must not contain a query; percent-encode a `?` the resource holds",
            ));
        }
        if value.fragment.is_some() {
            return Err(parse_error(
                "arn",
                tail,
                "ARN must not contain a fragment; percent-encode a `#` the resource holds",
            ));
        }

        let fields: Vec<&str> = path.splitn(FIELDS, ':').collect();
        if fields.len() < FIELDS {
            return Err(parse_error(
                "arn",
                tail,
                "ARN must contain a partition, service, region, account, and resource",
            ));
        }
        let mut position = FIELDS_OFFSET;
        for (index, field) in fields.iter().enumerate() {
            validate_field(field, index, position)?;
            position += field.len() + 1;
        }

        // The three AWS decides are case-insensitive and written lower case, so
        // they fold here for the reason a URN's namespace folds: two spellings
        // of one name must compare, order, and hash as one value. The account
        // and the resource are the owner's and the service's, and stay as
        // written.
        if fields[..FOLDED_FIELDS]
            .iter()
            .any(|field| field.bytes().any(|byte| byte.is_ascii_uppercase()))
        {
            let mut folded = SmolStrBuilder::new();
            for (index, field) in fields.iter().enumerate() {
                if index > 0 {
                    folded.push(':');
                }
                if index < FOLDED_FIELDS {
                    folded.push_str(&field.to_ascii_lowercase());
                } else {
                    folded.push_str(field);
                }
            }
            value.state_path(UriPath(folded.into()));
        }
        Ok(Self(value))
    }

    /// Deserialize an ARN from structural JSON.
    ///
    /// # Errors
    ///
    /// As [`Uri::from_json`], plus this type's own validation.
    pub fn from_json(value: &str) -> Result<Self> {
        serde_json::from_str(value).map_err(Error::from)
    }

    /// Consume this ARN and serialize it as structural JSON.
    ///
    /// # Errors
    ///
    /// As [`Uri::into_json`].
    pub fn into_json(self) -> Result<String> {
        serde_json::to_string(&self).map_err(Error::from)
    }

    /// Consume this ARN and return its URI without allocating.
    pub fn into_uri(self) -> Uri {
        self.0
    }

    /// Return the required `arn` scheme.
    pub fn scheme(&self) -> &Scheme {
        self.0.scheme()
    }

    /// Return the concrete empty ARN authority.
    pub fn authority(&self) -> &Authority {
        self.0.authority()
    }

    /// Return the concrete ARN path: the five fields as written.
    pub fn path(&self) -> &UriPath {
        self.0.path()
    }

    /// Return the ARN path as text, decoding its escapes when asked.
    ///
    /// # Errors
    ///
    /// As [`Uri::path_text`].
    pub fn path_text(&self, decode: bool) -> Result<Cow<'_, str>> {
        self.0.path_text(decode)
    }

    /// Return the partition: `aws`, `aws-cn`, `aws-us-gov`, or another AWS
    /// names; [`ArnPartition::from_arn`] reads it as one AWS runs.
    pub fn partition(&self) -> &str {
        self.field(0)
    }

    /// Return the service namespace: `s3`, `iam`, `lambda`, and the rest.
    pub fn service(&self) -> &str {
        self.field(1)
    }

    /// Return the region, or `None` for a service that spans every region.
    pub fn region(&self) -> Option<&str> {
        non_empty(self.field(2))
    }

    /// Return the owning account, or `None` when the ARN names none.
    ///
    /// The value is the twelve-digit account identifier for a resource an
    /// account owns, and `aws` for one AWS itself owns - an AWS-managed IAM
    /// policy reads `arn:aws:iam::aws:policy/AdministratorAccess`.
    pub fn account(&self) -> Option<&str> {
        non_empty(self.field(3))
    }

    /// Return the resource field whole, its own `/` and `:` structure kept.
    pub fn resource(&self) -> &str {
        self.field(FIELDS - 1)
    }

    /// Return the separator the resource field uses, if it carries one.
    ///
    /// A service writes its resource as `type/id` or as `type:id`, and which
    /// one it writes is the service's choice rather than a canonical form, so
    /// this reports it rather than normalizing it away.
    pub fn resource_separator(&self) -> Option<char> {
        resource_split(self.resource()).map(|(_, separator, _)| separator)
    }

    /// Return the resource type, when the resource names one.
    ///
    /// This is the syntactic split at the resource's first `/` or `:`. What
    /// that leading part *means* is the service's own business - for Amazon S3
    /// it is the bucket, which [`bucket`](Self::bucket) is the door for.
    pub fn resource_type(&self) -> Option<&str> {
        resource_split(self.resource()).map(|(resource_type, _, _)| resource_type)
    }

    /// Return the resource identifier: what follows the type, or the whole
    /// resource when it names no type.
    pub fn resource_id(&self) -> &str {
        resource_split(self.resource())
            .map_or_else(|| self.resource(), |(_, _, resource_id)| resource_id)
    }

    /// Return the container an ARN names, when its service has one.
    ///
    /// The bucket on Amazon S3 and the table bucket on Amazon S3 Tables: one
    /// name, because it is one position in the location, which is what
    /// [`Uri::bucket`] reads it as. Only the container form answers - an ARN
    /// naming an access point, a job, or another service's resource has none,
    /// and says so rather than reading its first resource part as one.
    pub fn bucket(&self) -> Option<&str> {
        self.store_location().map(|(_, bucket, _)| bucket)
    }

    /// Return the object key an Amazon S3 ARN names, below its bucket.
    ///
    /// The key is spelled as the ARN spells it - escapes stay escaped and a
    /// trailing slash stays - for the reason [`Uri::key`] gives. An ARN naming
    /// the bucket alone answers `""`. An Amazon S3 Tables ARN holds a table
    /// rather than an object below its container, which [`table`](Self::table)
    /// is the door for.
    pub fn key(&self) -> Option<&str> {
        (self.service() == "s3")
            .then(|| self.store_location().map(|(_, _, key)| key))
            .flatten()
    }

    /// Return the table an Amazon S3 Tables ARN names, below its table bucket.
    ///
    /// A table bucket ARN names the container alone and answers `None`; the
    /// table form is `bucket/<table-bucket>/table/<table>`, which is what the
    /// S3 Tables catalog addresses one by.
    ///
    /// ```
    /// use yggdryl::Arn;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let table = Arn::from_str("arn:aws:s3tables:us-east-1:123456789012:bucket/lake/table/t-a1")?;
    /// assert_eq!(table.bucket(), Some("lake"));
    /// assert_eq!(table.table(), Some("t-a1"));
    /// assert_eq!(table.key(), None);
    ///
    /// let container = Arn::from_str("arn:aws:s3tables:us-east-1:123456789012:bucket/lake")?;
    /// assert_eq!(container.bucket(), Some("lake"));
    /// assert_eq!(container.table(), None);
    /// # Ok(())
    /// # }
    /// ```
    pub fn table(&self) -> Option<&str> {
        (self.service() == "s3tables")
            .then(|| self.store_location().map(|(_, _, table)| table))
            .flatten()
            .filter(|table| !table.is_empty())
    }

    /// Return the table bucket and the identifier of the table this ARN
    /// names, when it is an Amazon S3 Tables table's own ARN:
    /// `bucket/<name>/table/<id>`, each one segment, and nothing else.
    ///
    /// The strict reading beside [`table`](Self::table), which answers
    /// whatever the resource spells below its bucket the way the location
    /// the ARN lowers to does. A door that addresses the service by the ARN
    /// asks this one: the service identifies a table by exactly that shape.
    #[cfg(feature = "s3tables")]
    pub(crate) fn identified_table(&self) -> Option<(&str, &str)> {
        if self.service() != "s3tables" {
            return None;
        }
        let (bucket, below) = self.table_bucket_resource()?;
        let id = below.strip_prefix("table/")?;
        (!id.is_empty() && !id.contains('/')).then_some((bucket, id))
    }

    /// Return the table bucket this ARN names, when it is an Amazon S3
    /// Tables table bucket's own ARN: `bucket/<name>`, one segment and
    /// nothing below it.
    ///
    /// The strict reading beside [`bucket`](Self::bucket), which answers the
    /// container of any resource spelling one. A door that addresses the
    /// service by a table bucket asks this one: the service identifies a
    /// table bucket by exactly that shape, and a trailing slash or an empty
    /// identifier below the name is nothing it has.
    #[cfg(feature = "s3tables")]
    pub(crate) fn table_bucket(&self) -> Option<&str> {
        if self.service() != "s3tables" {
            return None;
        }
        // Read off the resource itself: the split every location reads by
        // folds `bucket/<name>/` into the name, and that slash is a shape the
        // service has not.
        let name = self.resource().strip_prefix("bucket/")?;
        (!name.is_empty() && !name.contains('/')).then_some(name)
    }

    /// Return the location this name addresses, as a URL.
    ///
    /// An Amazon S3 ARN names a bucket and, below it, a key, which is exactly
    /// what an `s3:` URL locates; an Amazon S3 Tables ARN names a table bucket
    /// and, below it, a table, which is what an `s3tables:` URL locates. Every
    /// other service addresses something no URL locates, and is refused by
    /// name.
    ///
    /// ```
    /// use yggdryl::Arn;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let arn = Arn::from_str("arn:aws:s3:::trades/2026/part.parquet")?;
    /// assert_eq!(arn.locator()?.to_string(), "s3://trades/2026/part.parquet");
    ///
    /// let table = Arn::from_str("arn:aws:s3tables:us-east-1:123456789012:bucket/lake/table/t-a1")?;
    /// assert_eq!(table.locator()?.to_string(), "s3tables://lake/t-a1");
    ///
    /// let user = Arn::from_str("arn:aws:iam::123456789012:user/David")?;
    /// assert!(user.locator().is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a parse error when the ARN names no location, or when its
    /// container and what it holds do not spell a valid URL.
    pub fn locator(&self) -> Result<Url> {
        let Some((scheme, container, name)) = self.store_location() else {
            return Err(parse_error(
                "arn",
                FIELDS_OFFSET,
                "only an Amazon S3 bucket or an Amazon S3 Tables ARN names a location a URL can address",
            ));
        };
        let mut path = SmolStrBuilder::new();
        if !name.is_empty() {
            path.push('/');
            path.push_str(name);
        }
        Url::from_uri(Uri::from_parts(
            scheme,
            Authority::from_str(container)?,
            UriPath(path.into()),
            None,
            None,
        )?)
    }

    /// Iterate over non-empty ARN path segments without allocating.
    pub fn path_segments(&self) -> PathSegments<'_> {
        self.0.path_segments()
    }

    /// Return the next path segment and cursor for an owning FFI iterator.
    pub fn next_path_segment(&self, cursor: usize) -> Option<(usize, &str)> {
        self.0.next_path_segment(cursor)
    }

    /// Return the last filename segment in the resource.
    pub fn file_name(&self) -> Option<&str> {
        file_name_from_path(self.resource())
    }

    /// Return the resource filename without its final extension.
    pub fn stem(&self) -> Option<&str> {
        self.file_name().map(stem_from_file_name)
    }

    /// Return the final resource filename extension.
    pub fn extension(&self) -> Option<&str> {
        self.file_name().and_then(extension_from_file_name)
    }

    /// Iterate over the resource filename's extensions without allocating.
    pub fn extensions(&self) -> Extensions<'_> {
        extensions_from_file_name(self.file_name())
    }

    /// Infer the final resource MIME type, defaulting to octet-stream.
    pub fn mime_type(&self) -> MimeType {
        self.extension()
            .and_then(|extension| MimeType::from_extension(extension).ok())
            .unwrap_or_default()
    }

    /// Infer the resource media type and its transparent encodings.
    pub fn media_type(&self) -> MediaType {
        MediaType::from_file_name(self.file_name().unwrap_or(""))
    }

    /// Replace the final resource filename segment.
    ///
    /// An error leaves the ARN unchanged.
    ///
    /// # Errors
    ///
    /// Returns the refusal of the ARN the value spells.
    pub fn set_file_name(&mut self, value: &str) -> Result<()> {
        let mut resource = self.resource_path();
        resource.set_file_name(value)?;
        self.replace_resource(&resource)
    }

    /// Replace the resource filename stem, preserving its final extension.
    ///
    /// An error leaves the ARN unchanged.
    ///
    /// # Errors
    ///
    /// Returns the refusal of the ARN the value spells.
    pub fn set_stem(&mut self, value: &str) -> Result<()> {
        let mut resource = self.resource_path();
        resource.set_stem(value)?;
        self.replace_resource(&resource)
    }

    /// Add or replace the final resource filename extension.
    ///
    /// An error leaves the ARN unchanged.
    ///
    /// # Errors
    ///
    /// Returns the refusal of the ARN the value spells.
    pub fn set_extension(&mut self, value: &str) -> Result<()> {
        let mut resource = self.resource_path();
        resource.set_extension(value)?;
        self.replace_resource(&resource)
    }

    /// Replace the complete compound resource extension chain.
    ///
    /// An error leaves the ARN unchanged.
    ///
    /// # Errors
    ///
    /// Returns the refusal of the ARN the values spell.
    pub fn set_extensions<I, S>(&mut self, values: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut resource = self.resource_path();
        resource.set_extensions(values)?;
        self.replace_resource(&resource)
    }

    /// Add or replace the final resource suffix from a MIME type.
    ///
    /// An error leaves the ARN unchanged.
    ///
    /// # Errors
    ///
    /// Returns the refusal of the ARN the type spells.
    pub fn set_mime_type(&mut self, value: MimeType) -> Result<()> {
        let mut resource = self.resource_path();
        resource.set_mime_type(value)?;
        self.replace_resource(&resource)
    }

    /// Replace the resource suffix chain from a media type.
    ///
    /// An error leaves the ARN unchanged.
    ///
    /// # Errors
    ///
    /// Returns the refusal of the ARN the type spells.
    pub fn set_media_type(&mut self, value: MediaType) -> Result<()> {
        let mut resource = self.resource_path();
        resource.set_media_type(value)?;
        self.replace_resource(&resource)
    }

    /// Remove the final resource extension and report whether one existed.
    pub fn remove_extension(&mut self) -> bool {
        let mut resource = self.resource_path();
        if !resource.remove_extension() {
            return false;
        }
        self.replace_resource(&resource).is_ok()
    }

    /// Remove every resource extension and report whether any existed.
    pub fn clear_extensions(&mut self) -> bool {
        let mut resource = self.resource_path();
        if !resource.clear_extensions() {
            return false;
        }
        self.replace_resource(&resource).is_ok()
    }

    /// Return a deterministic cross-language hash of the canonical ARN.
    pub fn stable_hash(&self) -> u64 {
        stable_hash_display(self)
    }

    /// Return the field at `index`, which validation proved is there.
    fn field(&self, index: usize) -> &str {
        field_of(self.0.path().as_str(), index)
    }

    /// Return the table bucket an Amazon S3 Tables resource names and what
    /// the resource spells below it: `bucket/<name>[/<below>]`.
    fn table_bucket_resource(&self) -> Option<(&str, &str)> {
        let below = self.resource().strip_prefix("bucket/")?;
        let (bucket, below) = below.split_once('/').unwrap_or((below, ""));
        (!bucket.is_empty()).then_some((bucket, below))
    }

    /// Return the scheme, the container, and the name below it this ARN spells.
    ///
    /// Two services name a location. The Amazon S3 bucket form carries no
    /// region and no account, which is what separates it from an access point
    /// or a job ARN, whose resource is a type and an identifier rather than a
    /// container and a key. The Amazon S3 Tables form writes the container
    /// under a `bucket` resource type and the table under a `table` one, so
    /// only a resource spelling both is a location.
    fn store_location(&self) -> Option<(Scheme, &str, &str)> {
        match self.service() {
            "s3" if self.region().is_none() && self.account().is_none() => {
                let resource = self.resource();
                let (bucket, key) = resource.split_once('/').unwrap_or((resource, ""));
                (!bucket.is_empty()).then_some((Scheme::S3, bucket, key))
            }
            "s3tables" => {
                let (bucket, below) = self.table_bucket_resource()?;
                let table = below.strip_prefix("table/").unwrap_or(below);
                Some((Scheme::S3TABLES, bucket, table))
            }
            _ => None,
        }
    }

    /// Answer the resource field as a path, for the accessors that edit one.
    fn resource_path(&self) -> UriPath {
        UriPath(SmolStr::new(self.resource()))
    }

    /// Rewrite the resource field, re-validating the whole ARN.
    fn replace_resource(&mut self, resource: &UriPath) -> Result<()> {
        let mut path = SmolStrBuilder::new();
        for index in 0..FIELDS - 1 {
            path.push_str(self.field(index));
            path.push(':');
        }
        path.push_str(resource.as_str());
        let mut candidate = self.0.clone();
        candidate.state_path(UriPath(path.into()));
        *self = Self::from_uri(candidate)?;
        Ok(())
    }
}

/// One of the partitions AWS runs: a set of regions with DNS names and ARNs
/// of its own, named by an ARN's first field.
///
/// The table is botocore's `partitions.json`, the one every AWS SDK resolves
/// endpoints through: each partition's name, the prefix its regions share,
/// its DNS suffix and its dual-stack one, and the region a global service
/// answers in. It is what turns a region into a host, so a partition's
/// suffixes are spelled here once and every client - STS, IAM Identity
/// Center, the Sign-In service, Amazon S3 - builds its hosts from them.
///
/// ```
/// use yggdryl::{Arn, ArnPartition};
///
/// # fn main() -> yggdryl::Result<()> {
/// let role = Arn::from_str("arn:aws-cn:iam::123456789012:role/lake-reader")?;
/// let partition = ArnPartition::from_arn(&role).expect("a partition AWS runs");
/// assert_eq!(partition, ArnPartition::AwsCn);
/// assert_eq!(partition.dns_suffix(), "amazonaws.com.cn");
/// assert_eq!(partition.global_region(), "cn-northwest-1");
///
/// // A region is in the partition its prefix names; one nobody claims is in `aws`.
/// assert_eq!(ArnPartition::from_region("us-gov-west-1"), ArnPartition::AwsUsGov);
/// assert_eq!(ArnPartition::from_region("eu-west-3"), ArnPartition::Aws);
/// assert_eq!(
///     ArnPartition::from_region("cn-north-1").service_host("sts", "cn-north-1", false, true),
///     "sts.cn-north-1.api.amazonwebservices.com.cn"
/// );
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ArnPartition {
    /// `aws`, the commercial regions.
    Aws,
    /// `aws-cn`, the regions in China.
    AwsCn,
    /// `aws-us-gov`, AWS GovCloud (US).
    AwsUsGov,
    /// `aws-iso`, the US ISO regions.
    AwsIso,
    /// `aws-iso-b`, the US ISOB regions.
    AwsIsoB,
    /// `aws-iso-e`, the EU ISOE regions.
    AwsIsoE,
    /// `aws-iso-f`, the US ISOF regions.
    AwsIsoF,
    /// `aws-eusc`, the AWS European Sovereign Cloud.
    AwsEusc,
}

impl ArnPartition {
    /// Every partition, in the order botocore's table lists them.
    pub const ALL: [Self; 8] = [
        Self::Aws,
        Self::AwsCn,
        Self::AwsEusc,
        Self::AwsIso,
        Self::AwsIsoB,
        Self::AwsIsoE,
        Self::AwsIsoF,
        Self::AwsUsGov,
    ];

    /// The partition's name, as an ARN's first field spells it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Aws => "aws",
            Self::AwsCn => "aws-cn",
            Self::AwsUsGov => "aws-us-gov",
            Self::AwsIso => "aws-iso",
            Self::AwsIsoB => "aws-iso-b",
            Self::AwsIsoE => "aws-iso-e",
            Self::AwsIsoF => "aws-iso-f",
            Self::AwsEusc => "aws-eusc",
        }
    }

    /// The prefix every region of the partition starts with; `aws` has none
    /// of its own, holding every region no other partition claims.
    const fn region_prefix(self) -> Option<&'static str> {
        match self {
            Self::Aws => None,
            Self::AwsCn => Some("cn-"),
            Self::AwsUsGov => Some("us-gov-"),
            Self::AwsIso => Some("us-iso-"),
            Self::AwsIsoB => Some("us-isob-"),
            Self::AwsIsoE => Some("eu-isoe-"),
            Self::AwsIsoF => Some("us-isof-"),
            Self::AwsEusc => Some("eusc-de-"),
        }
    }

    /// The partition `region` is in: the one whose regions share its
    /// prefix, else `aws`, which is where botocore resolves a region its
    /// table does not know. Surrounding blanks and case are ignored.
    pub fn from_region(region: &str) -> Self {
        let region = region.trim();
        Self::ALL
            .into_iter()
            .find(|partition| {
                partition.region_prefix().is_some_and(|prefix| {
                    region
                        .get(..prefix.len())
                        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
                })
            })
            .unwrap_or(Self::Aws)
    }

    /// The partition `arn` names, when it names one AWS runs.
    pub fn from_arn(arn: &Arn) -> Option<Self> {
        arn.partition().parse().ok()
    }

    /// The DNS suffix the partition's hosts end with: `amazonaws.com`,
    /// `amazonaws.com.cn`, `c2s.ic.gov`, ...
    pub const fn dns_suffix(self) -> &'static str {
        match self {
            Self::Aws | Self::AwsUsGov => "amazonaws.com",
            Self::AwsCn => "amazonaws.com.cn",
            Self::AwsIso => "c2s.ic.gov",
            Self::AwsIsoB => "sc2s.sgov.gov",
            Self::AwsIsoE => "cloud.adc-e.uk",
            Self::AwsIsoF => "csp.hci.ic.gov",
            Self::AwsEusc => "amazonaws.eu",
        }
    }

    /// The DNS suffix the partition's dual-stack hosts end with: `api.aws`,
    /// `api.amazonwebservices.com.cn`, ...
    pub const fn dualstack_dns_suffix(self) -> &'static str {
        match self {
            Self::Aws | Self::AwsUsGov => "api.aws",
            Self::AwsCn => "api.amazonwebservices.com.cn",
            Self::AwsIso => "api.aws.ic.gov",
            Self::AwsIsoB => "api.aws.scloud",
            Self::AwsIsoE => "api.cloud-aws.adc-e.uk",
            Self::AwsIsoF => "api.aws.hci.ic.gov",
            Self::AwsEusc => "api.amazonwebservices.eu",
        }
    }

    /// The region a global service of the partition answers in, and the one
    /// a request names when nothing names another: `us-east-1` on `aws`.
    pub const fn global_region(self) -> &'static str {
        match self {
            Self::Aws => "us-east-1",
            Self::AwsCn => "cn-northwest-1",
            Self::AwsUsGov => "us-gov-west-1",
            Self::AwsIso => "us-iso-east-1",
            Self::AwsIsoB => "us-isob-east-1",
            Self::AwsIsoE => "eu-isoe-west-1",
            Self::AwsIsoF => "us-isof-south-1",
            Self::AwsEusc => "eusc-de-east-1",
        }
    }

    /// The host `service` answers at in `region`, in the form the endpoint
    /// rules of most services share: `{service}.{region}.{suffix}`, the
    /// service written `{service}-fips` under `fips`, and the dual-stack
    /// suffix under `dualstack`. A service whose rules spell its hosts
    /// otherwise - the Sign-In service, Amazon S3's own dual-stack form -
    /// builds them from [`Self::dns_suffix`] and
    /// [`Self::dualstack_dns_suffix`] instead.
    pub fn service_host(self, service: &str, region: &str, fips: bool, dualstack: bool) -> String {
        let service = if fips {
            format!("{service}-fips")
        } else {
            service.to_owned()
        };
        let suffix = if dualstack {
            self.dualstack_dns_suffix()
        } else {
            self.dns_suffix()
        };
        format!("{service}.{}.{suffix}", region.trim())
    }
}

impl FromStr for ArnPartition {
    type Err = Error;

    /// The partition a name spells, its case ignored.
    fn from_str(value: &str) -> Result<Self> {
        let value = value.trim();
        Self::ALL
            .into_iter()
            .find(|partition| partition.as_str().eq_ignore_ascii_case(value))
            .ok_or_else(|| {
                parse_error(
                    "arn",
                    0,
                    "expected a partition AWS runs - aws, aws-cn, aws-us-gov, aws-iso, aws-iso-b, aws-iso-e, aws-iso-f or aws-eusc",
                )
            })
    }
}

impl fmt::Display for ArnPartition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The field at `index` of an ARN's path - the five fields as written - and
/// the empty text where the path holds no such field.
fn field_of(path: &str, index: usize) -> &str {
    path.splitn(FIELDS, ':').nth(index).unwrap_or("")
}

/// The service field of an identifier under the `arn` scheme, as written:
/// what a door reads to route an ARN before it validates one, so neither
/// folded nor proven to be there.
#[cfg(feature = "s3tables")]
pub(super) fn service_of(arn: &Uri) -> &str {
    field_of(arn.path().as_str(), 1)
}

/// Answer `value` unless it is the empty field, which names nothing.
fn non_empty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

/// Split a resource into its type, its separator, and its identifier.
fn resource_split(resource: &str) -> Option<(&str, char, &str)> {
    let position = resource.find(['/', ':'])?;
    let separator = resource[position..].chars().next()?;
    Some((
        &resource[..position],
        separator,
        &resource[position + separator.len_utf8()..],
    ))
}

/// Validate one ARN field against what AWS lets it carry.
///
/// The resource is whatever the service writes and the URI parser has already
/// proved it is path text, so only its emptiness is decided here. The four
/// AWS decides carry ASCII letters, digits, hyphens, and underscores, plus the
/// `*` and `?` a policy writes to match a set of them.
fn validate_field(value: &str, index: usize, position: usize) -> Result<()> {
    if value.is_empty() {
        return match index {
            0 => Err(parse_error(
                "arn",
                position,
                "ARN partition must not be empty",
            )),
            1 => Err(parse_error(
                "arn",
                position,
                "ARN service must not be empty",
            )),
            4 => Err(parse_error(
                "arn",
                position,
                "ARN resource must not be empty",
            )),
            // A global service names no region, and an AWS-owned resource no
            // account: both are written as the empty field.
            _ => Ok(()),
        };
    }
    if index == FIELDS - 1 {
        return Ok(());
    }
    if let Some(offset) = value.bytes().position(|byte| {
        !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'*' | b'?'))
    }) {
        return Err(parse_error(
            "arn",
            position + offset,
            "ARN partition, service, region, and account carry only ASCII letters, digits, hyphens, underscores, and the `*` and `?` wildcards",
        ));
    }
    Ok(())
}

impl FromStr for Arn {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::from_uri(Uri::from_str(value)?)
    }
}

impl TryFrom<Uri> for Arn {
    type Error = Error;

    fn try_from(value: Uri) -> Result<Self> {
        Self::from_uri(value)
    }
}

impl TryFrom<&Uri> for Arn {
    type Error = Error;

    fn try_from(value: &Uri) -> Result<Self> {
        Self::from_uri(value.clone())
    }
}

impl From<Arn> for Uri {
    fn from(value: Arn) -> Self {
        value.into_uri()
    }
}

impl From<&Arn> for Uri {
    fn from(value: &Arn) -> Self {
        value.clone().into_uri()
    }
}

impl AsRef<Uri> for Arn {
    fn as_ref(&self) -> &Uri {
        &self.0
    }
}

impl AsRef<UriPath> for Arn {
    fn as_ref(&self) -> &UriPath {
        self.path()
    }
}

impl fmt::Display for Arn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl<'a> IntoIterator for &'a Arn {
    type Item = &'a str;
    type IntoIter = PathSegments<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.path_segments()
    }
}

impl Serialize for Arn {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Arn {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_uri(Uri::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}
