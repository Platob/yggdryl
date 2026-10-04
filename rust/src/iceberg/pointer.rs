//! Where a table's current metadata document is named, when something
//! other than the table's own folder names it.
//!
//! A table laid out as `HadoopTables` lays one out names its current
//! document itself: `metadata/version-hint.text`, else the highest number a
//! listing of `metadata/` shows. A table a catalog service keeps - an Amazon
//! S3 Tables table, a REST catalog's - is named by the service instead, and
//! the folder beside it may take neither a listing nor a delete. A
//! [`MetadataPointer`] is that service, reduced to the two questions a table
//! asks of it: which document is current now, and may this one replace it.
//!
//! An [`IcebergTable`](super::IcebergTable) opened or created with a pointer
//! ([`IcebergTable::open_pointed`](super::IcebergTable::open_pointed),
//! [`IcebergTable::create_pointed`](super::IcebergTable::create_pointed))
//! reads the one document the pointer names and nothing else, writes each
//! next document as `metadata/{version:05}-{uuid}.metadata.json` - the name
//! Iceberg's own catalogs give one, the first numbered `00000` - and
//! publishes it under the token the pointer last answered. No read and no
//! commit lists the folder, writes a hint or removes a file: a document or a
//! file a failed commit wrote stays where it is, unreferenced, for the
//! store's own maintenance to collect. The table's own
//! [`ls`](crate::IOBase::ls) and [`remove`](crate::IOBase::remove) are
//! refused, touching nothing: the catalog that keeps the pointer keeps the
//! table, and drops it.
//!
//! ```
//! use std::sync::{Arc, Mutex};
//!
//! use yggdryl::iceberg::{FormatVersion, IcebergTable, MetadataPointer, PartitionSpec, PointerState};
//! use yggdryl::local::LocalFolder;
//! use yggdryl::{DataType, Field, StructType, Url};
//!
//! /// A pointer kept in memory: a location and a counter for a token.
//! #[derive(Debug, Default)]
//! struct Memory(Mutex<(Option<Url>, u64)>);
//!
//! impl MetadataPointer for Memory {
//!     fn current(&self) -> yggdryl::Result<PointerState> {
//!         let held = self.0.lock().unwrap();
//!         Ok(PointerState::new(held.0.clone(), held.1.to_string()))
//!     }
//!
//!     fn publish(&self, token: &str, location: &Url) -> yggdryl::Result<PointerState> {
//!         let mut held = self.0.lock().unwrap();
//!         if token != held.1.to_string() {
//!             return Err(yggdryl::Error::conflict("table version", "table version", location));
//!         }
//!         *held = (Some(location.clone()), held.1 + 1);
//!         Ok(PointerState::new(held.0.clone(), held.1.to_string()))
//!     }
//! }
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
//!     .required_field("row");
//! let root = std::env::temp_dir().join(format!("yggdryl-pointer-doc-{}", std::process::id()));
//! let pointer = Arc::new(Memory::default());
//!
//! // The first document is version 0, and the pointer names it.
//! let table = IcebergTable::create_pointed(
//!     LocalFolder::new(&root)?,
//!     FormatVersion::V2,
//!     schema,
//!     PartitionSpec::unpartitioned(),
//!     pointer.clone(),
//! )?;
//! assert_eq!(table.metadata_version()?, 0);
//! let named = pointer.current()?.location().cloned().expect("a published document");
//! assert_eq!(table.metadata_location()?, named.to_string());
//!
//! // Opening reads the one document the pointer names: no hint, no listing.
//! let opened = IcebergTable::open_pointed(LocalFolder::new(&root)?, pointer)?;
//! assert_eq!(opened.metadata_file_name()?, table.metadata_file_name()?);
//! assert!(!root.join("metadata/version-hint.text").exists());
//! # std::fs::remove_dir_all(&root)?;
//! # Ok(())
//! # }
//! ```

use smol_str::SmolStr;

use crate::{Result, Url};

/// Where a table's current metadata document is named, and how the next one
/// is published.
///
/// The pointer is the table's one compare-and-swap: [`Self::publish`] names
/// a document on condition the pointer still stands at the token a reading
/// answered, so of two writers committing on one token one wins and the
/// other is told - which plain storage, with no such primitive, cannot do.
pub trait MetadataPointer: std::fmt::Debug + Send + Sync {
    /// The document the table names now and the token a publication is
    /// conditioned on; a table with no document yet states no location.
    ///
    /// # Errors
    ///
    /// Returns the store's failure to answer, and its absence of the table.
    fn current(&self) -> Result<PointerState>;

    /// Name `location` the current document, on condition the pointer still
    /// stands at `token`; a pointer that moved is a commit conflict.
    ///
    /// Answers where the pointer stands once the document is named: the
    /// location, and the token the next publication is conditioned on.
    ///
    /// # Errors
    ///
    /// Returns a failure for which [`Error::is_conflict`](crate::Error::is_conflict)
    /// holds when the pointer no longer stands at `token`, and the store's
    /// own failure otherwise - which leaves the publication in doubt until
    /// [`Self::current`] is read again.
    fn publish(&self, token: &str, location: &Url) -> Result<PointerState>;
}

/// Where a [`MetadataPointer`] stands: the document it names, when it names
/// one, and the token the next publication is conditioned on.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PointerState {
    location: Option<Url>,
    token: SmolStr,
}

impl PointerState {
    /// The pointer naming `location` - none for a table with no document
    /// yet - at `token`.
    pub fn new(location: Option<Url>, token: impl Into<SmolStr>) -> Self {
        Self {
            location,
            token: token.into(),
        }
    }

    /// The document the pointer names, when it names one.
    pub fn location(&self) -> Option<&Url> {
        self.location.as_ref()
    }

    /// The token the next publication is conditioned on.
    pub fn token(&self) -> &str {
        &self.token
    }
}
