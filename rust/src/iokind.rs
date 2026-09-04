//! What kind of resource a byte handle addresses.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::format_smolstr;

use crate::{Error, Result};

/// The role a resource plays, independent of where it lives.
///
/// Every backend has the same three roles - bytes with no location, a leaf that
/// holds bytes, a container that holds other resources - plus the honest answer
/// for a location that does not exist yet. A generic handle such as
/// [`crate::holder::local::Path`] reads this to decide which specialized implementation
/// to work through, so adding a backend means answering this question rather
/// than inventing new vocabulary.
///
/// A table format names three container roles of its own, because the folders
/// it owns are not ordinary folders and answering "directory" about them loses
/// what a caller most needs to know. A [`Table`](Self::Table) is one tabular
/// value spread over many files, so it is read through the record surface
/// rather than by listing it; a [`Namespace`](Self::Namespace) holds tables and
/// further namespaces; a [`Catalog`](Self::Catalog) is the warehouse those
/// namespaces live under. All three contain others, so
/// [`is_container`](Self::is_container) stays the one question a walk asks.
///
/// ```
/// use yggdryl::{IOBase, holder::Buffer};
/// use yggdryl::IOKind;
///
/// assert_eq!(Buffer::new().kind(), IOKind::Memory);
///
/// // A table format's roles are containers too, so one walk covers them all.
/// assert!(IOKind::Table.is_container());
/// assert!(IOKind::Namespace.is_container());
/// assert!(IOKind::Catalog.is_container());
/// assert_eq!(IOKind::from_str("catalog")?, IOKind::Catalog);
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum IOKind {
    /// Bytes held in memory, with no location of their own.
    Memory,
    /// A leaf resource holding bytes: a file, an object, a blob.
    #[default]
    File,
    /// A container holding other resources: a directory, a key prefix.
    Directory,
    /// A container that is one tabular value: an Iceberg table's folder.
    ///
    /// Its files are the table's storage, not its contents, so a caller reads
    /// it through the record surface and never by listing what is under it.
    Table,
    /// A container of tables and further namespaces.
    Namespace,
    /// A container of namespaces: the warehouse a catalog resolves names in.
    Catalog,
    /// A location that does not exist yet, so its role is not decided.
    ///
    /// A read of an unknown location yields nothing and a write creates it,
    /// exactly as the laziness contract requires.
    Unknown,
}

impl IOKind {
    /// Every kind in canonical order.
    ///
    /// The order widens: memory, then a leaf, then the containers from the
    /// plainest to the one every other lives under, then the undecided answer.
    pub const ALL: [Self; 7] = [
        Self::Memory,
        Self::File,
        Self::Directory,
        Self::Table,
        Self::Namespace,
        Self::Catalog,
        Self::Unknown,
    ];

    /// Parse a canonical kind name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] naming the accepted vocabulary and the input.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        <Self as FromStr>::from_str(value)
    }

    /// Return the canonical lowercase name without allocating.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::File => "file",
            Self::Directory => "directory",
            Self::Table => "table",
            Self::Namespace => "namespace",
            Self::Catalog => "catalog",
            Self::Unknown => "unknown",
        }
    }

    /// Return whether this kind holds other resources.
    ///
    /// A table format's roles hold resources exactly as a directory does - a
    /// table holds its files, a namespace its tables, a catalog its namespaces
    /// - so everything that asks "can this hold others?" keeps asking it once.
    pub const fn is_container(self) -> bool {
        matches!(
            self,
            Self::Directory | Self::Table | Self::Namespace | Self::Catalog
        )
    }

    /// Return whether this kind holds bytes of its own.
    pub const fn is_leaf(self) -> bool {
        matches!(self, Self::Memory | Self::File)
    }

    /// Return whether the resource exists yet.
    pub const fn is_known(self) -> bool {
        !matches!(self, Self::Unknown)
    }
}

impl FromStr for IOKind {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let normalized = value.trim();
        Self::ALL
            .into_iter()
            .find(|kind| normalized.eq_ignore_ascii_case(kind.as_str()))
            .ok_or_else(|| Error::Parse {
                target: "io kind",
                position: 0,
                reason: format_smolstr!(
                    "expected one of {}, got {value:?}",
                    Self::ALL
                        .iter()
                        .map(|kind| kind.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            })
    }
}

impl fmt::Display for IOKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for IOKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for IOKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = <&str>::deserialize(deserializer)?;
        Self::from_str(value).map_err(serde::de::Error::custom)
    }
}
