use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::{SmolStr, SmolStrBuilder};

use crate::{Error, Result, hashing::stable_hash_display};

#[derive(Clone, Debug)]
enum SchemeWire {
    Http,
    Https,
    File,
    Urn,
    Arn,
    Postgres,
    Postgresql,
    Mysql,
    Arrow,
    Sql,
    Glue,
    Iceberg,
    Fix,
    Field,
    Digest,
    Identity,
    Partition,
    Transform,
    S3,
    S3a,
    S3n,
    S3Tables,
    Gs,
    Gcs,
    Az,
    Abfs,
    Abfss,
    Wasb,
    Wasbs,
    Spark,
    Polars,
    Pandas,
    Python,
    Custom(SmolStr),
}

/// A validated, canonical lowercase URI scheme and protocol namespace.
///
/// As a metadata namespace it is spelled upper case: a protocol's keys are
/// `SCHEME:name`, the way `ARROW:extension:name` and `PARQUET:field_id` are.
///
/// Common protocol flavors use allocation-free internal values. Any valid
/// RFC scheme remains supported through [`Self::from_str`].
#[derive(Clone, Debug)]
pub struct Scheme(SchemeWire);

impl Scheme {
    /// The HTTP protocol scheme.
    pub const HTTP: Self = Self(SchemeWire::Http);
    /// The secure HTTP protocol scheme.
    pub const HTTPS: Self = Self(SchemeWire::Https);
    /// The local or network file protocol scheme.
    pub const FILE: Self = Self(SchemeWire::File);
    /// The uniform resource name scheme.
    pub const URN: Self = Self(SchemeWire::Urn);
    /// The Amazon Resource Name scheme.
    pub const ARN: Self = Self(SchemeWire::Arn);
    /// The short PostgreSQL protocol spelling.
    pub const POSTGRES: Self = Self(SchemeWire::Postgres);
    /// The long PostgreSQL protocol spelling.
    pub const POSTGRESQL: Self = Self(SchemeWire::Postgresql);
    /// The MySQL protocol scheme.
    pub const MYSQL: Self = Self(SchemeWire::Mysql);
    /// The Arrow protocol and metadata namespace.
    pub const ARROW: Self = Self(SchemeWire::Arrow);
    /// The generic SQL metadata namespace.
    pub const SQL: Self = Self(SchemeWire::Sql);
    /// The AWS Glue metadata namespace.
    pub const GLUE: Self = Self(SchemeWire::Glue);
    /// The Apache Iceberg metadata namespace and table-format interchange.
    pub const ICEBERG: Self = Self(SchemeWire::Iceberg);
    /// The Financial Information eXchange metadata namespace.
    pub const FIX: Self = Self(SchemeWire::Fix);
    /// The Yggdryl field metadata namespace.
    pub const FIELD: Self = Self(SchemeWire::Field);
    /// The generic row-digest metadata namespace.
    pub const DIGEST: Self = Self(SchemeWire::Digest);
    /// The generic field identity metadata namespace.
    pub const IDENTITY: Self = Self(SchemeWire::Identity);
    /// The generic field partition metadata namespace.
    pub const PARTITION: Self = Self(SchemeWire::Partition);
    /// The `TRANSFORM:` field namespace: how a column is computed.
    pub const TRANSFORM: Self = Self(SchemeWire::Transform);
    /// The Amazon S3 object protocol scheme.
    pub const S3: Self = Self(SchemeWire::S3);
    /// The Hadoop `s3a` spelling of the same Amazon S3 protocol.
    pub const S3A: Self = Self(SchemeWire::S3a);
    /// The Hadoop `s3n` spelling of the same Amazon S3 protocol.
    pub const S3N: Self = Self(SchemeWire::S3n);
    /// The Amazon S3 Tables protocol scheme.
    pub const S3TABLES: Self = Self(SchemeWire::S3Tables);
    /// The Google Cloud Storage protocol scheme.
    pub const GS: Self = Self(SchemeWire::Gs);
    /// The `gcs` spelling of the same Google Cloud Storage protocol.
    pub const GCS: Self = Self(SchemeWire::Gcs);
    /// The Azure Blob Storage protocol scheme.
    pub const AZ: Self = Self(SchemeWire::Az);
    /// The Hadoop `abfs` spelling of the same Azure Blob Storage protocol.
    pub const ABFS: Self = Self(SchemeWire::Abfs);
    /// The Hadoop `abfss` spelling, which addresses the store over TLS.
    pub const ABFSS: Self = Self(SchemeWire::Abfss);
    /// The Hadoop `wasb` spelling of the same Azure Blob Storage protocol.
    pub const WASB: Self = Self(SchemeWire::Wasb);
    /// The Hadoop `wasbs` spelling, which addresses the store over TLS.
    pub const WASBS: Self = Self(SchemeWire::Wasbs);
    /// The Apache Spark SQL interchange namespace.
    pub const SPARK: Self = Self(SchemeWire::Spark);
    /// The Polars interchange namespace.
    pub const POLARS: Self = Self(SchemeWire::Polars);
    /// The pandas interchange namespace.
    pub const PANDAS: Self = Self(SchemeWire::Pandas);
    /// The Python runtime metadata namespace.
    pub const PYTHON: Self = Self(SchemeWire::Python);

    /// Every schema-compatibility target, in normalization-cost order.
    ///
    /// [`Self::ARROW`] is the identity target; the rest are progressively more
    /// conservative subsets of the pinned Arrow model. [`Self::ICEBERG`] is the
    /// table-format subset, so it is both a metadata namespace and a target.
    pub const COMPATIBILITY_TARGETS: [Self; 5] = [
        Self::ARROW,
        Self::SPARK,
        Self::POLARS,
        Self::PANDAS,
        Self::ICEBERG,
    ];

    /// Parse and validate a URI scheme or metadata protocol namespace.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// The prefix this scheme's metadata keys carry: its spelling in upper
    /// case - `FIX:tag`, `PARTITION:sources`, `ICEBERG:doc` - so the crate's
    /// own keys read like the `ARROW:extension:name` and `PARQUET:field_id`
    /// every catalog already carries. HTTPS shares HTTP's one namespace. A
    /// known scheme answers a static string; only a custom one allocates.
    pub(crate) fn metadata_prefix(&self) -> Cow<'_, str> {
        Cow::Borrowed(match &self.0 {
            SchemeWire::Http | SchemeWire::Https => "HTTP",
            SchemeWire::File => "FILE",
            SchemeWire::Urn => "URN",
            SchemeWire::Arn => "ARN",
            SchemeWire::Postgres => "POSTGRES",
            SchemeWire::Postgresql => "POSTGRESQL",
            SchemeWire::Mysql => "MYSQL",
            SchemeWire::Arrow => "ARROW",
            SchemeWire::Sql => "SQL",
            SchemeWire::Glue => "GLUE",
            SchemeWire::Iceberg => "ICEBERG",
            SchemeWire::Fix => "FIX",
            SchemeWire::Field => "FIELD",
            SchemeWire::Digest => "DIGEST",
            SchemeWire::Identity => "IDENTITY",
            SchemeWire::Partition => "PARTITION",
            SchemeWire::Transform => "TRANSFORM",
            SchemeWire::S3 => "S3",
            SchemeWire::S3a => "S3A",
            SchemeWire::S3n => "S3N",
            SchemeWire::S3Tables => "S3TABLES",
            SchemeWire::Gs => "GS",
            SchemeWire::Gcs => "GCS",
            SchemeWire::Az => "AZ",
            SchemeWire::Abfs => "ABFS",
            SchemeWire::Abfss => "ABFSS",
            SchemeWire::Wasb => "WASB",
            SchemeWire::Wasbs => "WASBS",
            SchemeWire::Spark => "SPARK",
            SchemeWire::Polars => "POLARS",
            SchemeWire::Pandas => "PANDAS",
            SchemeWire::Python => "PYTHON",
            SchemeWire::Custom(value) => return Cow::Owned(value.to_ascii_uppercase()),
        })
    }

    /// Return the canonical lowercase spelling without allocating.
    pub fn as_str(&self) -> &str {
        match &self.0 {
            SchemeWire::Http => "http",
            SchemeWire::Https => "https",
            SchemeWire::File => "file",
            SchemeWire::Urn => "urn",
            SchemeWire::Arn => "arn",
            SchemeWire::Postgres => "postgres",
            SchemeWire::Postgresql => "postgresql",
            SchemeWire::Mysql => "mysql",
            SchemeWire::Arrow => "arrow",
            SchemeWire::Sql => "sql",
            SchemeWire::Glue => "glue",
            SchemeWire::Iceberg => "iceberg",
            SchemeWire::Fix => "fix",
            SchemeWire::Field => "field",
            SchemeWire::Digest => "digest",
            SchemeWire::Identity => "identity",
            SchemeWire::Partition => "partition",
            SchemeWire::Transform => "transform",
            SchemeWire::S3 => "s3",
            SchemeWire::S3a => "s3a",
            SchemeWire::S3n => "s3n",
            SchemeWire::S3Tables => "s3tables",
            SchemeWire::Gs => "gs",
            SchemeWire::Gcs => "gcs",
            SchemeWire::Az => "az",
            SchemeWire::Abfs => "abfs",
            SchemeWire::Abfss => "abfss",
            SchemeWire::Wasb => "wasb",
            SchemeWire::Wasbs => "wasbs",
            SchemeWire::Spark => "spark",
            SchemeWire::Polars => "polars",
            SchemeWire::Pandas => "pandas",
            SchemeWire::Python => "python",
            SchemeWire::Custom(value) => value.as_str(),
        }
    }

    /// Return whether this scheme uses a static, allocation-free protocol value.
    pub const fn is_known(&self) -> bool {
        !matches!(self.0, SchemeWire::Custom(_))
    }

    /// Return the IANA-registered default port for the scheme, when it has one.
    ///
    /// A scheme that names a metadata namespace or an object-storage protocol
    /// without a fixed listening port returns `None`. A URL omitting the port
    /// is understood to address this port.
    pub const fn default_port(&self) -> Option<u16> {
        match self.0 {
            SchemeWire::Http => Some(80),
            SchemeWire::Https => Some(443),
            SchemeWire::Postgres | SchemeWire::Postgresql => Some(5432),
            SchemeWire::Mysql => Some(3306),
            _ => None,
        }
    }

    /// Return whether the scheme addresses Amazon S3.
    ///
    /// `s3`, `s3a`, and `s3n` name one protocol: the two Hadoop spellings
    /// differ only in the connector that once read them, and every store,
    /// bucket, and key they address is the same. This is what a caller asks
    /// before selecting the S3 backend, so the three never drift apart into
    /// separate scheme comparisons.
    pub const fn is_s3(&self) -> bool {
        matches!(self.0, SchemeWire::S3 | SchemeWire::S3a | SchemeWire::S3n)
    }

    /// Return whether the scheme addresses Amazon S3 Tables.
    ///
    /// A table bucket holds tables rather than objects, and no byte-level
    /// backend opens one, so this is deliberately not part of
    /// [`Self::is_object_store`]: it is the question a reader asks before
    /// speaking the S3 Tables catalog, not before selecting a byte store.
    pub const fn is_s3_tables(&self) -> bool {
        matches!(self.0, SchemeWire::S3Tables)
    }

    /// Return whether the scheme addresses Google Cloud Storage.
    ///
    /// `gs` and `gcs` name one protocol, for the reason [`Self::is_s3`] gives:
    /// the second spelling is what some tools write, and every bucket and
    /// object either addresses is the same.
    pub const fn is_gs(&self) -> bool {
        matches!(self.0, SchemeWire::Gs | SchemeWire::Gcs)
    }

    /// Return whether the scheme addresses Azure Blob Storage.
    ///
    /// `az` is this crate's spelling; `abfs`, `abfss`, `wasb`, and `wasbs` are
    /// the Hadoop connectors' four, which differ in the driver that once read
    /// them and in whether the endpoint they imply is TLS. Every container and
    /// blob the five address is the same.
    pub const fn is_az(&self) -> bool {
        matches!(
            self.0,
            SchemeWire::Az
                | SchemeWire::Abfs
                | SchemeWire::Abfss
                | SchemeWire::Wasb
                | SchemeWire::Wasbs
        )
    }

    /// Return whether the scheme addresses an object store.
    ///
    /// The three stores one backend serves, under every spelling each is
    /// written as. This is what selects that backend, so a location written by
    /// another tool reaches it rather than falling through to a filesystem.
    pub const fn is_object_store(&self) -> bool {
        self.is_s3() || self.is_gs() || self.is_az()
    }

    /// Return whether a location under this scheme names a container.
    ///
    /// Every object store does - a bucket on Amazon S3 and Google Cloud
    /// Storage, a container on Azure Blob Storage - and so does Amazon S3
    /// Tables, whose table bucket holds tables. It is one position in the
    /// location whichever store it is, so this is what [`Uri::bucket`] and
    /// [`Uri::key`] read, while [`Self::is_object_store`] stays the question
    /// of which backend opens it.
    ///
    /// [`Uri::bucket`]: crate::Uri::bucket
    /// [`Uri::key`]: crate::Uri::key
    pub const fn has_container(&self) -> bool {
        self.is_object_store() || self.is_s3_tables()
    }

    /// Return whether the scheme addresses a byte-oriented storage location.
    ///
    /// These are the schemes a filesystem abstraction can open, as opposed to
    /// metadata namespaces and compatibility targets.
    pub const fn is_storage(&self) -> bool {
        matches!(
            self.0,
            SchemeWire::File
                | SchemeWire::Http
                | SchemeWire::Https
                | SchemeWire::S3
                | SchemeWire::S3a
                | SchemeWire::S3n
                | SchemeWire::Gs
                | SchemeWire::Gcs
                | SchemeWire::Az
                | SchemeWire::Abfs
                | SchemeWire::Abfss
                | SchemeWire::Wasb
                | SchemeWire::Wasbs
        )
    }

    /// Return whether the scheme names a schema-compatibility target.
    ///
    /// Only these values are accepted by `into_scheme_compat`.
    pub const fn is_compatibility_target(&self) -> bool {
        matches!(
            self.0,
            SchemeWire::Arrow
                | SchemeWire::Spark
                | SchemeWire::Polars
                | SchemeWire::Pandas
                | SchemeWire::Iceberg
        )
    }

    /// Return a deterministic cross-language hash of the canonical scheme.
    pub fn stable_hash(&self) -> u64 {
        stable_hash_display(self)
    }
}

impl FromStr for Scheme {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let bytes = value.as_bytes();
        if bytes.first().is_none_or(|byte| !byte.is_ascii_alphabetic()) {
            return Err(Error::Parse {
                target: "scheme",
                position: 0,
                reason: "scheme must start with an ASCII letter".into(),
            });
        }
        if let Some(position) = bytes
            .iter()
            .position(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.')))
        {
            return Err(Error::Parse {
                target: "scheme",
                position,
                reason: "scheme may contain only ASCII letters, digits, plus, hyphen, or dot"
                    .into(),
            });
        }

        let known = match value.len() {
            2 if value.eq_ignore_ascii_case("s3") => Some(Self::S3),
            2 if value.eq_ignore_ascii_case("gs") => Some(Self::GS),
            2 if value.eq_ignore_ascii_case("az") => Some(Self::AZ),
            3 if value.eq_ignore_ascii_case("gcs") => Some(Self::GCS),
            4 if value.eq_ignore_ascii_case("abfs") => Some(Self::ABFS),
            4 if value.eq_ignore_ascii_case("wasb") => Some(Self::WASB),
            5 if value.eq_ignore_ascii_case("abfss") => Some(Self::ABFSS),
            5 if value.eq_ignore_ascii_case("wasbs") => Some(Self::WASBS),
            3 if value.eq_ignore_ascii_case("s3a") => Some(Self::S3A),
            3 if value.eq_ignore_ascii_case("s3n") => Some(Self::S3N),
            8 if value.eq_ignore_ascii_case("s3tables") => Some(Self::S3TABLES),
            3 if value.eq_ignore_ascii_case("urn") => Some(Self::URN),
            3 if value.eq_ignore_ascii_case("arn") => Some(Self::ARN),
            3 if value.eq_ignore_ascii_case("sql") => Some(Self::SQL),
            3 if value.eq_ignore_ascii_case("fix") => Some(Self::FIX),
            4 if value.eq_ignore_ascii_case("http") => Some(Self::HTTP),
            4 if value.eq_ignore_ascii_case("file") => Some(Self::FILE),
            4 if value.eq_ignore_ascii_case("glue") => Some(Self::GLUE),
            5 if value.eq_ignore_ascii_case("https") => Some(Self::HTTPS),
            5 if value.eq_ignore_ascii_case("mysql") => Some(Self::MYSQL),
            5 if value.eq_ignore_ascii_case("arrow") => Some(Self::ARROW),
            5 if value.eq_ignore_ascii_case("field") => Some(Self::FIELD),
            5 if value.eq_ignore_ascii_case("spark") => Some(Self::SPARK),
            6 if value.eq_ignore_ascii_case("digest") => Some(Self::DIGEST),
            6 if value.eq_ignore_ascii_case("polars") => Some(Self::POLARS),
            6 if value.eq_ignore_ascii_case("pandas") => Some(Self::PANDAS),
            6 if value.eq_ignore_ascii_case("python") => Some(Self::PYTHON),
            7 if value.eq_ignore_ascii_case("iceberg") => Some(Self::ICEBERG),
            8 if value.eq_ignore_ascii_case("postgres") => Some(Self::POSTGRES),
            8 if value.eq_ignore_ascii_case("identity") => Some(Self::IDENTITY),
            9 if value.eq_ignore_ascii_case("partition") => Some(Self::PARTITION),
            9 if value.eq_ignore_ascii_case("transform") => Some(Self::TRANSFORM),
            10 if value.eq_ignore_ascii_case("postgresql") => Some(Self::POSTGRESQL),
            _ => None,
        };
        if let Some(known) = known {
            return Ok(known);
        }

        if value.bytes().all(|byte| !byte.is_ascii_uppercase()) {
            return Ok(Self(SchemeWire::Custom(SmolStr::new(value))));
        }
        let mut normalized = SmolStrBuilder::new();
        for byte in value.bytes() {
            normalized.push(char::from(byte.to_ascii_lowercase()));
        }
        Ok(Self(SchemeWire::Custom(normalized.into())))
    }
}

impl PartialEq for Scheme {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for Scheme {}

impl PartialOrd for Scheme {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Scheme {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl Hash for Scheme {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl AsRef<str> for Scheme {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Scheme {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for Scheme {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Scheme {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(D::Error::custom)
    }
}
