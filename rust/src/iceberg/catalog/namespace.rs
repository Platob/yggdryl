//! A namespace of an Iceberg catalog: a folder under the warehouse, read as
//! namespaces and tables, nested to any depth.

use std::any::Any;
use std::hash::{Hash, Hasher};

use smol_str::SmolStr;

use super::{
    NAMESPACE_DOCUMENT, child, create_namespace, create_table, level, read_document,
    update_document,
};
use crate::holder::Holder;
use crate::warehouse::{Handle, RegisteredNamespace, Site, path_text};
use crate::{
    Error, Field, IOBase, IOKind, IntoObjectPath, Namespace, NamespaceValue, Object, ObjectValue,
    Objects, Properties, Result, Table, Url,
};

/// One namespace of an Iceberg catalog: a folder under the warehouse.
///
/// A namespace exists when its folder does; the `metadata/namespace.json`
/// document is what makes an *empty* one durable and what carries its
/// stored properties, read as [`ObjectValue::properties`] between the
/// parent's and what was stated, and written by
/// [`ObjectValue::update_properties`]. Constructing one touches nothing.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IcebergNamespace {
    path: Vec<SmolStr>,
    handle: Handle,
    stated: Properties,
    inherited: Properties,
}

impl IcebergNamespace {
    /// The namespace at `path` - its catalog's name first, at least two
    /// parts - over the folder `url` names, touching nothing.
    ///
    /// # Errors
    ///
    /// Returns the identifier's own refusal when it names no location, and
    /// [`Error::InvalidRecord`] at `$.path` for a path of fewer than two
    /// parts.
    pub fn new(path: impl IntoObjectPath, url: impl Into<crate::Uri>) -> Result<Self> {
        let path = checked(path.into_object_path()?)?;
        let url = url.into().locator()?;
        Ok(Self {
            handle: Handle::at(Site::Url(url), false, &path, Properties::new()),
            path,
            stated: Properties::new(),
            inherited: Properties::new(),
        })
    }

    /// The namespace at `path` over a folder already in hand.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] at `$.path` for a path of fewer than
    /// two parts.
    pub fn bound(path: impl IntoObjectPath, folder: Holder) -> Result<Self> {
        let path = checked(path.into_object_path()?)?;
        Ok(Self {
            handle: Handle::bound(folder, false, &path, Properties::new()),
            path,
            stated: Properties::new(),
            inherited: Properties::new(),
        })
    }

    /// The namespace a listing found at `path`, carrying its parent's
    /// effective properties.
    pub(super) fn listed(path: Vec<SmolStr>, folder: Holder, inherited: Properties) -> Self {
        Self {
            handle: Handle::bound(folder, false, &path, inherited.clone()),
            path,
            stated: Properties::new(),
            inherited,
        }
    }

    /// Return this namespace with stated properties, which its folder and
    /// every object under it open with.
    #[must_use]
    pub fn with_properties(mut self, properties: Properties) -> Self {
        self.stated = properties;
        self.handle
            .set_properties(self.stated.inherit(&self.inherited));
        self
    }

    /// The namespace with its parent's effective properties pushed into it.
    pub(crate) fn inheriting(mut self, parent: &Properties) -> Self {
        self.inherited = parent.clone();
        self.handle
            .set_properties(self.stated.inherit(&self.inherited));
        self
    }

    /// The folder, opened on the first call under what is known without a
    /// read: the parent's properties, then the stated ones.
    fn handle(&self) -> Result<&Holder> {
        self.handle.get()
    }

    /// The effective properties: the parent's, then what the document
    /// keeps, then what was stated.
    fn effective(&self) -> Result<Properties> {
        let stored = read_document(self.handle()?, NAMESPACE_DOCUMENT)?;
        Ok(self.stated.inherit(&stored.inherit(&self.inherited)))
    }
}

/// A namespace path: its catalog's name and at least one part below it.
fn checked(path: Vec<SmolStr>) -> Result<Vec<SmolStr>> {
    if path.len() < 2 {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$.path"),
            reason: smol_str::format_smolstr!(
                "expected a namespace path of at least two parts, the catalog's name first, \
                 got {}",
                path_text(&path)
            ),
        });
    }
    Ok(path)
}

impl ObjectValue for IcebergNamespace {
    fn name(&self) -> &str {
        self.path.last().map_or("", SmolStr::as_str)
    }

    fn path(&self) -> &[SmolStr] {
        &self.path
    }

    fn kind(&self) -> IOKind {
        IOKind::Namespace
    }

    fn url(&self) -> Option<&Url> {
        self.handle.url()
    }

    fn modified(&self) -> Option<i64> {
        self.handle().ok()?.mtime()
    }

    fn properties(&self) -> Result<Properties> {
        self.effective()
    }

    fn update_properties(&self, updates: &Properties, removes: &[SmolStr]) -> Result<()> {
        update_document(self.handle()?, NAMESPACE_DOCUMENT, updates, removes)
    }
}

impl NamespaceValue for IcebergNamespace {
    fn children(&self) -> Objects {
        match self
            .handle()
            .and_then(|handle| Ok((handle, self.effective()?)))
        {
            Ok((handle, effective)) => level(handle, &self.path, effective),
            Err(error) => Objects::failing(error),
        }
    }

    fn get(&self, name: &str) -> Result<Object> {
        child(self.handle()?, &self.path, &self.effective()?, name)
    }

    fn create_namespace(&self, name: &str, properties: &Properties) -> Result<Namespace> {
        create_namespace(
            self.handle()?,
            &self.path,
            &self.effective()?,
            name,
            properties,
        )
    }

    fn create_table(&self, name: &str, field: &Field, properties: &Properties) -> Result<Table> {
        create_table(
            self.handle()?,
            &self.path,
            &self.effective()?,
            name,
            field,
            properties,
        )
    }
}

impl RegisteredNamespace for IcebergNamespace {
    fn implementation_name(&self) -> &'static str {
        "IcebergNamespace"
    }

    fn clone_box(&self) -> Box<dyn RegisteredNamespace> {
        Box::new(self.clone())
    }

    fn dyn_eq(&self, other: &dyn RegisteredNamespace) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| self == other)
    }

    fn dyn_hash(&self, mut state: &mut dyn Hasher) {
        Hash::hash(self, &mut state);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn with_properties(self: Box<Self>, properties: Properties) -> Box<dyn RegisteredNamespace> {
        Box::new((*self).with_properties(properties))
    }

    fn inheriting(self: Box<Self>, parent: &Properties) -> Box<dyn RegisteredNamespace> {
        Box::new((*self).inheriting(parent))
    }
}

impl From<IcebergNamespace> for Namespace {
    fn from(namespace: IcebergNamespace) -> Self {
        Self::Registered(Box::new(namespace))
    }
}

impl From<IcebergNamespace> for Object {
    fn from(namespace: IcebergNamespace) -> Self {
        Self::Namespace(Namespace::from(namespace))
    }
}
