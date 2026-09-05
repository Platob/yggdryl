use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::{SmolStr, SmolStrBuilder};

use crate::{Error, Result, stable_hash_display};

#[derive(Clone, Debug)]
enum SchemeValue {
    Http,
    Https,
    File,
    Urn,
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
    S3,
    Gs,
    Az,
    Spark,
    Polars,
    Pandas,
    Custom(SmolStr),
}

/// A validated, canonical lowercase URI scheme and protocol namespace.
///
/// Common protocol flavors use allocation-free internal values. Any valid
/// RFC scheme remains supported through [`Self::from_str`].
#[derive(Clone, Debug)]
pub struct Scheme(SchemeValue);

impl Scheme {
    /// The HTTP protocol scheme.
    pub const HTTP: Self = Self(SchemeValue::Http);
    /// The secure HTTP protocol scheme.
    pub const HTTPS: Self = Self(SchemeValue::Https);
    /// The local or network file protocol scheme.
    pub const FILE: Self = Self(SchemeValue::File);
    /// The uniform resource name scheme.
    pub const URN: Self = Self(SchemeValue::Urn);
    /// The short PostgreSQL protocol spelling.
    pub const POSTGRES: Self = Self(SchemeValue::Postgres);
    /// The long PostgreSQL protocol spelling.
    pub const POSTGRESQL: Self = Self(SchemeValue::Postgresql);
    /// The MySQL protocol scheme.
    pub const MYSQL: Self = Self(SchemeValue::Mysql);
    /// The Arrow protocol and metadata namespace.
    pub const ARROW: Self = Self(SchemeValue::Arrow);
    /// The generic SQL metadata namespace.
    pub const SQL: Self = Self(SchemeValue::Sql);
    /// The AWS Glue metadata namespace.
    pub const GLUE: Self = Self(SchemeValue::Glue);
    /// The Apache Iceberg metadata namespace and table-format interchange.
    pub const ICEBERG: Self = Self(SchemeValue::Iceberg);
    /// The Financial Information eXchange metadata namespace.
    pub const FIX: Self = Self(SchemeValue::Fix);
    /// The Yggdryl field metadata namespace.
    pub const FIELD: Self = Self(SchemeValue::Field);
    /// The generic row-digest metadata namespace.
    pub const DIGEST: Self = Self(SchemeValue::Digest);
    /// The generic field identity metadata namespace.
    pub const IDENTITY: Self = Self(SchemeValue::Identity);
    /// The generic field partition metadata namespace.
    pub const PARTITION: Self = Self(SchemeValue::Partition);
    /// The Amazon S3 object protocol scheme.
    pub const S3: Self = Self(SchemeValue::S3);
    /// The Google Cloud Storage protocol scheme.
    pub const GS: Self = Self(SchemeValue::Gs);
    /// The Azure Blob Storage protocol scheme.
    pub const AZ: Self = Self(SchemeValue::Az);
    /// The Apache Spark SQL interchange namespace.
    pub const SPARK: Self = Self(SchemeValue::Spark);
    /// The Polars interchange namespace.
    pub const POLARS: Self = Self(SchemeValue::Polars);
    /// The pandas interchange namespace.
    pub const PANDAS: Self = Self(SchemeValue::Pandas);

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

    /// Return the canonical lowercase spelling without allocating.
    pub fn as_str(&self) -> &str {
        match &self.0 {
            SchemeValue::Http => "http",
            SchemeValue::Https => "https",
            SchemeValue::File => "file",
            SchemeValue::Urn => "urn",
            SchemeValue::Postgres => "postgres",
            SchemeValue::Postgresql => "postgresql",
            SchemeValue::Mysql => "mysql",
            SchemeValue::Arrow => "arrow",
            SchemeValue::Sql => "sql",
            SchemeValue::Glue => "glue",
            SchemeValue::Iceberg => "iceberg",
            SchemeValue::Fix => "fix",
            SchemeValue::Field => "field",
            SchemeValue::Digest => "digest",
            SchemeValue::Identity => "identity",
            SchemeValue::Partition => "partition",
            SchemeValue::S3 => "s3",
            SchemeValue::Gs => "gs",
            SchemeValue::Az => "az",
            SchemeValue::Spark => "spark",
            SchemeValue::Polars => "polars",
            SchemeValue::Pandas => "pandas",
            SchemeValue::Custom(value) => value.as_str(),
        }
    }

    /// Return whether this scheme uses a static, allocation-free protocol value.
    pub const fn is_known(&self) -> bool {
        !matches!(self.0, SchemeValue::Custom(_))
    }

    /// Return the IANA-registered default port for the scheme, when it has one.
    ///
    /// A scheme that names a metadata namespace or an object-storage protocol
    /// without a fixed listening port returns `None`. A URL omitting the port
    /// is understood to address this port.
    pub const fn default_port(&self) -> Option<u16> {
        match self.0 {
            SchemeValue::Http => Some(80),
            SchemeValue::Https => Some(443),
            SchemeValue::Postgres | SchemeValue::Postgresql => Some(5432),
            SchemeValue::Mysql => Some(3306),
            _ => None,
        }
    }

    /// Return whether the scheme addresses a byte-oriented storage location.
    ///
    /// These are the schemes a filesystem abstraction can open, as opposed to
    /// metadata namespaces and compatibility targets.
    pub const fn is_storage(&self) -> bool {
        matches!(
            self.0,
            SchemeValue::File
                | SchemeValue::Http
                | SchemeValue::Https
                | SchemeValue::S3
                | SchemeValue::Gs
                | SchemeValue::Az
        )
    }

    /// Return whether the scheme names a schema-compatibility target.
    ///
    /// Only these values are accepted by `into_scheme_compat`.
    pub const fn is_compatibility_target(&self) -> bool {
        matches!(
            self.0,
            SchemeValue::Arrow
                | SchemeValue::Spark
                | SchemeValue::Polars
                | SchemeValue::Pandas
                | SchemeValue::Iceberg
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
            3 if value.eq_ignore_ascii_case("urn") => Some(Self::URN),
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
            7 if value.eq_ignore_ascii_case("iceberg") => Some(Self::ICEBERG),
            8 if value.eq_ignore_ascii_case("postgres") => Some(Self::POSTGRES),
            8 if value.eq_ignore_ascii_case("identity") => Some(Self::IDENTITY),
            9 if value.eq_ignore_ascii_case("partition") => Some(Self::PARTITION),
            10 if value.eq_ignore_ascii_case("postgresql") => Some(Self::POSTGRESQL),
            _ => None,
        };
        if let Some(known) = known {
            return Ok(known);
        }

        if value.bytes().all(|byte| !byte.is_ascii_uppercase()) {
            return Ok(Self(SchemeValue::Custom(SmolStr::new(value))));
        }
        let mut normalized = SmolStrBuilder::new();
        for byte in value.bytes() {
            normalized.push(char::from(byte.to_ascii_lowercase()));
        }
        Ok(Self(SchemeValue::Custom(normalized.into())))
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
