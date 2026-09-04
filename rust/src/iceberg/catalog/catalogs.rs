//! The top of the cascade: a folder of warehouses, and one warehouse framed.

use super::super::Table;
use super::{
    CATALOG_DOCUMENT, Names, Namespace, Namespaces, Occupant, Tables, classify, invalid, resolve,
    write_properties,
};
use crate::IOBase;
use crate::holder::Holder;
use crate::metadata::Metadata;
use crate::{IOKind, Result};

/// A folder of warehouses, as a lazy map-oriented view.
///
/// This is the level the hierarchy was missing above the warehouse: a
/// deployment with more than one lake addresses them as
/// `catalogs.get("lake")?.namespaces()...` instead of a caller-side
/// convention. Constructing the view performs no I/O; a warehouse exists when
/// its folder does, and [`Self::create`] is what writes the
/// `metadata/catalog.json` document that makes an empty one durable.
#[derive(Debug)]
pub struct Catalogs<H: IOBase> {
    /// The folder every warehouse lives under.
    root: H,
}

impl<H: IOBase> Catalogs<H> {
    /// Describe the collection over a root folder, touching nothing.
    pub const fn new(root: H) -> Self {
        Self { root }
    }

    /// Borrow the folder the warehouses live under.
    pub const fn root(&self) -> &H {
        &self.root
    }

    /// Open the named catalog, raised as a typed absence when nothing is there.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the catalog when its folder holds
    /// nothing, and a refusal when a table or a file occupies the name.
    pub fn get(&self, name: &str) -> Result<Catalog<Holder>> {
        match classify(resolve(&self.root, name)?)? {
            // A warehouse is a folder; what makes it a catalog is this
            // framing, so any occupied non-table folder answers.
            Occupant::Namespace(folder) => Ok(Catalog::new(folder)),
            Occupant::Table(_) => Err(invalid(smol_str::format_smolstr!(
                "expected a catalog at {name:?}, got a table"
            ))),
            Occupant::Nothing(_) => Err(crate::Error::absent("catalog", name)),
            Occupant::File => Err(invalid(smol_str::format_smolstr!(
                "expected a catalog folder at {name:?}, got a file"
            ))),
        }
    }

    /// Create the named catalog, writing its `metadata/catalog.json`.
    ///
    /// The write is what creates the folder and its ancestry - nothing walks
    /// or prepares anything first.
    ///
    /// # Errors
    ///
    /// Returns a typed conflict when something already occupies the name.
    pub fn create(&self, name: &str) -> Result<Catalog<Holder>> {
        match classify(resolve(&self.root, name)?)? {
            Occupant::Nothing(folder) => {
                write_properties(&folder, CATALOG_DOCUMENT, Vec::new())?;
                Ok(Catalog::new(folder))
            }
            Occupant::Namespace(_) => Err(crate::Error::conflict("catalog", "catalog", name)),
            Occupant::Table(_) => Err(crate::Error::conflict("catalog", "table", name)),
            Occupant::File => Err(crate::Error::conflict("catalog", "file", name)),
        }
    }

    /// Open the named catalog, creating it when absent.
    ///
    /// The same attempt as [`Self::create`] with the conflict absorbed: one
    /// classification, one code path.
    ///
    /// # Errors
    ///
    /// Returns a refusal when a table or a file occupies the name.
    pub fn open_or_create(&self, name: &str) -> Result<Catalog<Holder>> {
        match classify(resolve(&self.root, name)?)? {
            Occupant::Namespace(folder) => Ok(Catalog::new(folder)),
            Occupant::Nothing(folder) => {
                write_properties(&folder, CATALOG_DOCUMENT, Vec::new())?;
                Ok(Catalog::new(folder))
            }
            Occupant::Table(_) => Err(invalid(smol_str::format_smolstr!(
                "expected a catalog at {name:?}, got a table"
            ))),
            Occupant::File => Err(invalid(smol_str::format_smolstr!(
                "expected a catalog folder at {name:?}, got a file"
            ))),
        }
    }

    /// Return whether the named catalog exists, asked of storage now.
    ///
    /// This is an answer for a caller who asked; nothing in this module calls
    /// it on the way to doing something else.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is refused or the classification fails.
    pub fn contains(&self, name: &str) -> Result<bool> {
        Ok(matches!(
            classify(resolve(&self.root, name)?)?,
            Occupant::Namespace(_)
        ))
    }

    /// The catalog names one level down, one at a time.
    pub fn iter(&self) -> Names {
        level_names(self.root.ls(false, false), false)
    }

    /// Return how many catalogs are here, which drains the listing.
    ///
    /// # Errors
    ///
    /// Returns the first listing failure.
    pub fn len(&self) -> Result<usize> {
        let mut count = 0;
        for name in self.iter() {
            name?;
            count += 1;
        }
        Ok(count)
    }

    /// Return whether no catalog is here, which costs one entry.
    ///
    /// # Errors
    ///
    /// Returns the first listing failure.
    pub fn is_empty(&self) -> Result<bool> {
        match self.iter().next() {
            Some(entry) => entry.map(|_| false),
            None => Ok(true),
        }
    }
}

/// The names of one listing level that classify as `tables` asks.
///
/// One classification per entry, lazily: a caller that takes three names from
/// a level of ten thousand classifies three. Leaves and the reserved
/// `metadata` folder are skipped; a container is a table when
/// [`Table::locate`] finds a current document and a namespace otherwise.
pub(super) fn level_names(level: crate::Listing, tables: bool) -> Names {
    Names::new(level.filter_map(move |entry| {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => return Some(Err(error)),
        };
        if !entry.is_container() {
            return None;
        }
        let name = entry
            .url()
            .and_then(|url| url.file_name().map(ToOwned::to_owned))?;
        if name == super::METADATA_DIR {
            return None;
        }
        match Table::locate(entry) {
            Ok(located) => (located.is_some() == tables).then_some(Ok(name)),
            Err(error) => Some(Err(error)),
        }
    }))
}

/// A warehouse folder of namespaces of Iceberg tables.
///
/// The catalog is a description of where tables live, not proof that any do:
/// constructing one touches nothing, and every operation resolves its dotted
/// name against the warehouse handle at the moment it runs. There is no
/// service in between, so two catalogs over the same folder see the same
/// tables. Its collections are [`Self::namespaces`] and [`Self::tables`];
/// the two dotted entry points - [`Self::table`] and [`Self::namespace`] -
/// are kept because a dotted identifier is a real Iceberg spelling and
/// deserves one call.
#[derive(Debug)]
pub struct Catalog<H: IOBase> {
    /// The folder every namespace and table lives under.
    warehouse: H,
}

impl<H: IOBase> Catalog<H> {
    /// Describe a catalog over a warehouse folder, touching nothing.
    pub const fn new(warehouse: H) -> Self {
        Self { warehouse }
    }

    /// Borrow the warehouse folder the catalog resolves names against.
    pub const fn warehouse(&self) -> &H {
        &self.warehouse
    }

    /// The catalog's name: its warehouse folder's own name.
    ///
    /// This is the identity [`Catalogs::iter`] lists the warehouse under -
    /// the same answer [`Namespace::name`] gives one level down - read off
    /// the warehouse handle's URL, so an in-memory warehouse answers its
    /// identity segment and a handle reporting no URL has no name to give.
    pub fn name(&self) -> Option<&str> {
        self.warehouse.url().and_then(crate::Url::file_name)
    }

    /// The role this value plays: [`IOKind::Catalog`].
    ///
    /// The warehouse handle underneath answers [`IOKind::Directory`], because
    /// a warehouse is a folder and storage has no way to tell that one folder
    /// is a catalog while the next is not. Framing it as a catalog is what
    /// this value adds, so it is this value that says so - the same way
    /// [`Namespace::kind`] and [`Table`]'s own [`IOBase::kind`] do one level
    /// down each.
    pub const fn kind(&self) -> IOKind {
        IOKind::Catalog
    }

    /// The catalog's namespaces, as a lazy map-oriented view.
    ///
    /// Constructing the view performs no I/O; every question it answers is
    /// asked of storage when it is asked of the view, and its names may be
    /// dotted - `namespaces.get("sales.eu")` descends.
    pub const fn namespaces(&self) -> Namespaces<'_, H> {
        Namespaces::at_root(self)
    }

    /// The catalog's tables, as a lazy map-oriented view over dotted names.
    ///
    /// `tables.get("sales.eu.orders")` descends; an un-dotted name addresses
    /// a table directly under the warehouse root.
    pub const fn tables(&self) -> Tables<'_, H> {
        Tables::at_root(self)
    }

    /// Open the table a dotted name addresses: [`Tables::get`], one call.
    ///
    /// # Errors
    ///
    /// Returns what [`Tables::get`] returns.
    pub fn table(&self, name: &str) -> Result<Table<Holder>> {
        self.tables().get(name)
    }

    /// Open the namespace a dotted name addresses: [`Namespaces::get`].
    ///
    /// # Errors
    ///
    /// Returns what [`Namespaces::get`] returns.
    pub fn namespace(&self, name: &str) -> Result<Namespace<'_, H>> {
        self.namespaces().get(name)
    }

    /// The catalog's own properties, from `metadata/catalog.json`.
    ///
    /// Absent means empty - never an error, never a missing-file failure a
    /// caller has to catch.
    ///
    /// # Errors
    ///
    /// Returns an error when a document is there but is not the expected
    /// shape.
    pub fn properties(&self) -> Result<Metadata> {
        let document = self.warehouse.child_by_path(CATALOG_DOCUMENT)?;
        super::read_properties_from(&document)
    }

    /// Apply property updates and removals in one transactional write.
    ///
    /// A failure leaves the stored value unchanged, and keys under the
    /// reserved `iceberg:` prefix are refused by name. Writing the document
    /// is also what makes an empty warehouse durable.
    ///
    /// # Errors
    ///
    /// Returns a refusal for a reserved key, or the read or write failure.
    pub fn update_properties(
        &self,
        updates: impl IntoIterator<Item = (String, String)>,
        removes: impl IntoIterator<Item = String>,
    ) -> Result<()> {
        let mut document = self.warehouse.child_by_path(CATALOG_DOCUMENT)?;
        super::update_properties_at(&mut document, updates, removes)
    }
}
