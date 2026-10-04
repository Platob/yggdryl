//! Catalogs and namespaces of registered objects, kept in order with no
//! storage behind them: what a [`Warehouse`](super::Warehouse) builds along
//! a registered path.

use smol_str::{SmolStr, format_smolstr};

use super::object::{extended, path_text};
use super::{CatalogValue, NamespaceValue, Object, ObjectValue, Objects, Properties};
use crate::{Error, IOKind, Result, Url};

/// Registered objects of any implementation, in registration order, under
/// one catalog name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MemoryCatalog {
    path: Vec<SmolStr>,
    description: Option<String>,
    stated: Properties,
    children: Vec<Object>,
}

impl MemoryCatalog {
    /// An empty catalog called `name`.
    pub fn new(name: impl Into<SmolStr>) -> Self {
        Self {
            path: vec![name.into()],
            description: None,
            stated: Properties::new(),
            children: Vec::new(),
        }
    }

    /// Return this catalog with a description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Return this catalog with stated properties, which every object
    /// registered under it inherits.
    #[must_use]
    pub fn with_properties(mut self, properties: Properties) -> Self {
        self.stated = properties;
        self
    }

    /// Return this catalog with `object` registered directly under it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Conflict`] when the name is taken, and
    /// [`Error::InvalidRecord`] when the object's path does not sit one
    /// level below this catalog.
    pub fn with_object(mut self, object: impl Into<Object>) -> Result<Self> {
        self.register(object.into())?;
        Ok(self)
    }

    /// The objects registered directly under this catalog, as registered.
    #[must_use]
    pub fn registered(&self) -> &[Object] {
        &self.children
    }

    /// Register `object` directly under this catalog.
    pub(crate) fn register(&mut self, object: Object) -> Result<()> {
        register_under(&self.path, &mut self.children, object)
    }

    /// Replace the object of `object`'s name under this catalog, answering
    /// the one it replaced.
    pub(crate) fn replace(&mut self, object: Object) -> Result<Option<Object>> {
        replace_under(&self.path, &mut self.children, object)
    }

    /// Remove the object called `name`, answering it.
    pub(crate) fn unregister(&mut self, name: &str) -> Result<Object> {
        unregister_under(&self.path, &mut self.children, name)
    }

    /// The registered object called `name`, mutably, when there is one.
    pub(crate) fn child_mut(&mut self, name: &str) -> Option<&mut Object> {
        self.children.iter_mut().find(|child| child.name() == name)
    }
}

impl ObjectValue for MemoryCatalog {
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

    fn properties(&self) -> Result<Properties> {
        Ok(self.stated.clone())
    }
}

impl NamespaceValue for MemoryCatalog {
    fn children(&self) -> Objects {
        children_of(&self.children, &self.stated)
    }

    fn get(&self, name: &str) -> Result<Object> {
        get_under(&self.path, &self.children, &self.stated, name)
    }
}

impl CatalogValue for MemoryCatalog {
    fn namespace_levels(&self) -> Option<usize> {
        None
    }
}

/// Registered objects of any implementation, in registration order, under
/// one namespace path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MemoryNamespace {
    path: Vec<SmolStr>,
    description: Option<String>,
    stated: Properties,
    inherited: Properties,
    children: Vec<Object>,
}

impl MemoryNamespace {
    /// An empty namespace at `path`, which names its catalog first.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when the path has fewer than two
    /// parts: a namespace sits under a catalog.
    pub fn new(path: impl super::IntoObjectPath) -> Result<Self> {
        let path = path.into_object_path()?;
        if path.len() < 2 {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$.{}", path_text(&path)),
                reason: format_smolstr!(
                    "expected a namespace path of at least two parts, a catalog's name first, \
                     got {} part(s)",
                    path.len()
                ),
            });
        }
        Ok(Self {
            path,
            description: None,
            stated: Properties::new(),
            inherited: Properties::new(),
            children: Vec::new(),
        })
    }

    /// Return this namespace with a description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Return this namespace with stated properties, which every object
    /// registered under it inherits.
    #[must_use]
    pub fn with_properties(mut self, properties: Properties) -> Self {
        self.stated = properties;
        self
    }

    /// Return this namespace with `object` registered directly under it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Conflict`] when the name is taken, and
    /// [`Error::InvalidRecord`] when the object's path does not sit one
    /// level below this namespace.
    pub fn with_object(mut self, object: impl Into<Object>) -> Result<Self> {
        self.register(object.into())?;
        Ok(self)
    }

    /// The objects registered directly under this namespace, as registered.
    #[must_use]
    pub fn registered(&self) -> &[Object] {
        &self.children
    }

    pub(crate) fn register(&mut self, object: Object) -> Result<()> {
        register_under(&self.path, &mut self.children, object)
    }

    pub(crate) fn replace(&mut self, object: Object) -> Result<Option<Object>> {
        replace_under(&self.path, &mut self.children, object)
    }

    pub(crate) fn unregister(&mut self, name: &str) -> Result<Object> {
        unregister_under(&self.path, &mut self.children, name)
    }

    pub(crate) fn child_mut(&mut self, name: &str) -> Option<&mut Object> {
        self.children.iter_mut().find(|child| child.name() == name)
    }

    pub(crate) fn inheriting(mut self, parent: &Properties) -> Self {
        self.inherited = parent.clone();
        self
    }
}

impl ObjectValue for MemoryNamespace {
    fn name(&self) -> &str {
        self.path.last().map_or("", SmolStr::as_str)
    }

    fn path(&self) -> &[SmolStr] {
        &self.path
    }

    fn kind(&self) -> IOKind {
        IOKind::Namespace
    }

    fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    fn url(&self) -> Option<&Url> {
        None
    }

    fn properties(&self) -> Result<Properties> {
        Ok(self.stated.inherit(&self.inherited))
    }
}

impl NamespaceValue for MemoryNamespace {
    fn children(&self) -> Objects {
        children_of(&self.children, &self.stated.inherit(&self.inherited))
    }

    fn get(&self, name: &str) -> Result<Object> {
        get_under(
            &self.path,
            &self.children,
            &self.stated.inherit(&self.inherited),
            name,
        )
    }
}

/// The registered children, each carrying `effective` as what it inherits.
fn children_of(children: &[Object], effective: &Properties) -> Objects {
    let effective = effective.clone();
    let children: Vec<Object> = children.to_vec();
    Objects::new(
        children
            .into_iter()
            .map(move |child| Ok(child.inheriting(&effective))),
    )
}

/// The registered child called `name`, carrying `effective`.
fn get_under(
    path: &[SmolStr],
    children: &[Object],
    effective: &Properties,
    name: &str,
) -> Result<Object> {
    children
        .iter()
        .find(|child| child.name() == name)
        .map(|child| child.clone().inheriting(effective))
        .ok_or_else(|| Error::absent("child", path_text(&extended(path, name))))
}

/// Check that `object` sits exactly one level below `parent`.
fn placed_under(parent: &[SmolStr], object: &Object) -> Result<()> {
    let path = object.path();
    if path.len() == parent.len() + 1 && path[..parent.len()] == *parent {
        return Ok(());
    }
    Err(Error::InvalidRecord {
        path: format_smolstr!("$.{}", path_text(path)),
        reason: format_smolstr!(
            "expected an object one level below `{}`, got `{}`",
            path_text(parent),
            path_text(path)
        ),
    })
}

fn register_under(parent: &[SmolStr], children: &mut Vec<Object>, object: Object) -> Result<()> {
    placed_under(parent, &object)?;
    if let Some(existing) = children.iter().find(|child| child.name() == object.name()) {
        return Err(Error::conflict(
            object.kind().as_str(),
            existing.kind().as_str(),
            path_text(object.path()),
        ));
    }
    children.push(object);
    Ok(())
}

fn replace_under(
    parent: &[SmolStr],
    children: &mut Vec<Object>,
    object: Object,
) -> Result<Option<Object>> {
    placed_under(parent, &object)?;
    match children
        .iter()
        .position(|child| child.name() == object.name())
    {
        Some(at) => Ok(Some(std::mem::replace(&mut children[at], object))),
        None => {
            children.push(object);
            Ok(None)
        }
    }
}

fn unregister_under(parent: &[SmolStr], children: &mut Vec<Object>, name: &str) -> Result<Object> {
    match children.iter().position(|child| child.name() == name) {
        Some(at) => Ok(children.remove(at)),
        None => Err(Error::absent("child", path_text(&extended(parent, name)))),
    }
}
