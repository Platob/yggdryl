//! The middle of the cascade: namespaces, nested to any depth.

use smol_str::{SmolStr, format_smolstr};

use super::catalogs::{Catalog, level_names};
use super::{
    NAMESPACE_DOCUMENT, Names, Occupant, Tables, classify, invalid, read_properties, resolve,
    update_properties, write_properties,
};
use crate::io::IOBase;
use crate::metadata::Metadata;
use crate::{IOKind, Result};

/// The namespaces below a catalog or a namespace, as a lazy view.
///
/// The view holds the catalog and the parent's dotted name and nothing else:
/// constructing one performs no I/O, membership and listing consult storage
/// when asked, and [`Self::get`] indexes to a [`Namespace`]. Two views over
/// the same catalog therefore observe each other's writes, and a view stays
/// valid across creation and deletion because every answer is storage's.
///
/// Names may be dotted: `namespaces.get("sales.eu")` descends, so the
/// resolution rule lives here and not in the caller.
#[derive(Debug)]
pub struct Namespaces<'catalog, H: IOBase> {
    /// The catalog every question is asked of.
    catalog: &'catalog Catalog<H>,
    /// The parent namespace's dotted name; `None` is the warehouse root.
    parent: Option<SmolStr>,
}

impl<'catalog, H: IOBase> Namespaces<'catalog, H> {
    /// The view at the warehouse root.
    pub(super) const fn at_root(catalog: &'catalog Catalog<H>) -> Self {
        Self {
            catalog,
            parent: None,
        }
    }

    /// The view one level below `parent`.
    pub(super) const fn under(catalog: &'catalog Catalog<H>, parent: SmolStr) -> Self {
        Self {
            catalog,
            parent: Some(parent),
        }
    }

    /// Spell one child's full dotted name.
    fn dotted(&self, name: &str) -> SmolStr {
        match &self.parent {
            Some(parent) => format_smolstr!("{parent}.{name}"),
            None => SmolStr::new(name),
        }
    }

    /// Open the named namespace, raised as a typed absence when nothing is
    /// there.
    ///
    /// A namespace exists when its folder does: the document
    /// `metadata/namespace.json` is what makes an *empty* one durable, and a
    /// folder a table write brought into being is a namespace all the same.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the dotted name, or a refusal when a
    /// table or a file occupies it.
    pub fn get(&self, name: &str) -> Result<Namespace<'catalog, H>> {
        let dotted = self.dotted(name);
        match classify(resolve(self.catalog.warehouse(), &dotted)?)? {
            Occupant::Namespace(_) => Ok(Namespace {
                catalog: self.catalog,
                name: dotted,
            }),
            Occupant::Table(_) => Err(invalid(format_smolstr!(
                "expected a namespace at {dotted:?}, got a table"
            ))),
            Occupant::Nothing(_) => Err(crate::Error::absent("namespace", dotted)),
            Occupant::File => Err(invalid(format_smolstr!(
                "expected a namespace folder at {dotted:?}, got a file"
            ))),
        }
    }

    /// Create the named namespace, writing its `metadata/namespace.json`.
    ///
    /// The document is what makes an empty namespace durable and what carries
    /// its properties - one artifact, both jobs. Writing it creates the
    /// folder and every missing ancestor, so nothing walks the ancestry
    /// first.
    ///
    /// # Errors
    ///
    /// Returns a typed conflict when a namespace - or a table - is already
    /// there, and the storage failure otherwise.
    pub fn create(&self, name: &str) -> Result<Namespace<'catalog, H>> {
        let dotted = self.dotted(name);
        match classify(resolve(self.catalog.warehouse(), &dotted)?)? {
            Occupant::Nothing(folder) => {
                write_properties(&folder, NAMESPACE_DOCUMENT, Vec::new())?;
                Ok(Namespace {
                    catalog: self.catalog,
                    name: dotted,
                })
            }
            Occupant::Namespace(_) => Err(crate::Error::conflict("namespace", "namespace", dotted)),
            Occupant::Table(_) => Err(crate::Error::conflict("namespace", "table", dotted)),
            Occupant::File => Err(crate::Error::conflict("namespace", "file", dotted)),
        }
    }

    /// Open the named namespace, creating it when absent.
    ///
    /// The same attempt as [`Self::create`] with the conflict absorbed: one
    /// classification, one code path, one round trip.
    ///
    /// # Errors
    ///
    /// Returns a refusal when a table or a file occupies the name.
    pub fn open_or_create(&self, name: &str) -> Result<Namespace<'catalog, H>> {
        let dotted = self.dotted(name);
        match classify(resolve(self.catalog.warehouse(), &dotted)?)? {
            Occupant::Namespace(_) => Ok(Namespace {
                catalog: self.catalog,
                name: dotted,
            }),
            Occupant::Nothing(folder) => {
                write_properties(&folder, NAMESPACE_DOCUMENT, Vec::new())?;
                Ok(Namespace {
                    catalog: self.catalog,
                    name: dotted,
                })
            }
            Occupant::Table(_) => Err(invalid(format_smolstr!(
                "expected a namespace at {dotted:?}, got a table"
            ))),
            Occupant::File => Err(invalid(format_smolstr!(
                "expected a namespace folder at {dotted:?}, got a file"
            ))),
        }
    }

    /// Return whether the named namespace exists, asked of storage now.
    ///
    /// A namespace is a folder that is not a table, so a table's name answers
    /// `false` here, and so does a location nothing occupies yet. This is an
    /// answer for a caller who asked; nothing in this module calls it on the
    /// way to doing something else.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is refused or the classification fails.
    pub fn contains(&self, name: &str) -> Result<bool> {
        Ok(matches!(
            classify(resolve(self.catalog.warehouse(), &self.dotted(name))?)?,
            Occupant::Namespace(_)
        ))
    }

    /// The namespace names one level down, one at a time, sorted.
    ///
    /// One classification per entry, lazily: taking three names classifies
    /// three, and a parent that does not exist lists nothing rather than
    /// failing.
    pub fn iter(&self) -> Names {
        let level = match &self.parent {
            Some(parent) => match resolve(self.catalog.warehouse(), parent) {
                Ok(folder) => folder.ls(false, false),
                Err(error) => return Names::failing(error),
            },
            None => self.catalog.warehouse().ls(false, false),
        };
        level_names(level, false)
    }

    /// Return how many namespaces are one level down, which drains the
    /// listing.
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

    /// Return whether no namespace is one level down, which costs one entry.
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

/// One namespace of a catalog: identity, properties, and its two collections.
///
/// The view borrows the catalog and holds only the namespace's dotted name;
/// its tables are [`Self::tables`] and its child namespaces are
/// [`Self::namespaces`], each resolving against the warehouse at the moment
/// an operation runs, so the namespace is as lazy as the catalog itself.
#[derive(Debug)]
pub struct Namespace<'catalog, H: IOBase> {
    pub(super) catalog: &'catalog Catalog<H>,
    pub(super) name: SmolStr,
}

impl<'catalog, H: IOBase> Namespace<'catalog, H> {
    /// The namespace's dotted name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The role this view plays: [`IOKind::Namespace`].
    ///
    /// A namespace is a folder that is not a table, which is all storage can
    /// see; that it is a namespace rather than any other folder is what the
    /// catalog framing adds, so the view is what answers it.
    pub const fn kind(&self) -> IOKind {
        IOKind::Namespace
    }

    /// This namespace's tables, as a lazy map-oriented view.
    pub fn tables(&self) -> Tables<'catalog, H> {
        Tables::under(self.catalog, self.name.clone())
    }

    /// The namespaces one level below this one, as the same view shape the
    /// catalog itself answers - the cascade that reaches a nested namespace.
    pub fn namespaces(&self) -> Namespaces<'catalog, H> {
        Namespaces::under(self.catalog, self.name.clone())
    }

    /// The namespace's properties, from `metadata/namespace.json`.
    ///
    /// Absent means empty - a namespace a table write brought into being
    /// carries no document and answers no properties, and that is not a
    /// failure.
    ///
    /// # Errors
    ///
    /// Returns an error when a document is there but is not the expected
    /// shape.
    pub fn properties(&self) -> Result<Metadata> {
        let folder = resolve(self.catalog.warehouse(), &self.name)?;
        read_properties(&folder, NAMESPACE_DOCUMENT)
    }

    /// Apply property updates and removals in one transactional write.
    ///
    /// A failure leaves the stored value unchanged, and keys under the
    /// reserved `iceberg:` prefix are refused by name. Writing the document
    /// is also what makes this namespace durable when it was only implicit.
    ///
    /// # Errors
    ///
    /// Returns a refusal for a reserved key, or the read or write failure.
    pub fn update_properties(
        &self,
        updates: impl IntoIterator<Item = (String, String)>,
        removes: impl IntoIterator<Item = String>,
    ) -> Result<()> {
        let folder = resolve(self.catalog.warehouse(), &self.name)?;
        update_properties(&folder, NAMESPACE_DOCUMENT, updates, removes)
    }
}
