//! [`ListScalar`] — one **list value**: a nullable row of a list column, its elements carried as an
//! erased sub-[`Serie`](crate::io::AnySerie). It is what [`ListSerie::row_scalar`](super::ListSerie::row_scalar)
//! yields.

use super::ListType;
use crate::io::field_carrier::field_accessors;
use crate::io::fixed::Field;
use crate::io::{AnyField, AnySerie, DataTypeId, ScalarType};

/// A single **list value** — a row: the list's element (item) field, the row's elements as an erased
/// sub-column (`Box<dyn AnySerie>`), and whether the list value itself is null. A list scalar "falls
/// back on our [`Serie`](crate::io::AnySerie)" — its elements *are* a (usually short) erased column,
/// so it needs no dependency on a bespoke value container.
///
/// It is a hashable value type: two list values are equal iff they have the same item field and
/// either are both null, or hold equal elements. A **null** list's phantom elements are ignored (two
/// same-typed null lists are equal, like `Scalar::null() == Scalar::null()`).
///
/// ```
/// use yggdryl_core::io::fixed::Serie;
/// use yggdryl_core::io::AnySerie;
/// use yggdryl_core::io::nested::ListSerie;
///
/// let items = Serie::from_values(&[1i32, 2, 3]).named("item");
/// let list = ListSerie::from_values(items, &[0, 2, 3], None).unwrap();
/// let row = list.row_scalar(0);
/// assert!(!row.is_null());
/// assert_eq!(row.len(), 2);
/// assert_eq!(row.items().value(0).bytes(), Some(&1i32.to_le_bytes()[..]));
/// ```
#[derive(Debug, Clone)]
pub struct ListScalar {
    item: AnyField,
    items: Box<dyn AnySerie>,
    null: bool,
    /// The value's **own-header** field (`List` type_id) — its name, declared nullability, and
    /// metadata. Excluded from value identity (the item field + elements are the identity).
    field: Field,
}

impl ListScalar {
    /// A present list value from its element (item) field and its elements as an erased sub-column.
    pub fn new(item: AnyField, items: Box<dyn AnySerie>) -> Self {
        Self {
            item,
            items,
            null: false,
            field: Field::of("", DataTypeId::List, 0, false),
        }
    }

    /// A null list value carrying its (logically-absent) elements.
    pub fn null(item: AnyField, items: Box<dyn AnySerie>) -> Self {
        Self {
            item,
            items,
            null: true,
            field: Field::of("", DataTypeId::List, 0, false),
        }
    }

    field_accessors!();

    /// The erased [`AnyField`] this list value contributes — a `List` field over its item field,
    /// with **effective** nullability `self.nullable() || self.is_null()` and the held metadata.
    pub fn field(&self) -> AnyField {
        AnyField::list_(
            self.name(),
            self.item.clone(),
            self.nullable() || self.is_null(),
        )
        .with_metadata_overlay(self.metadata())
    }

    /// Like [`field`](ListScalar::field) but **consumes** the value.
    pub fn into_field(self) -> AnyField {
        AnyField::list_(
            self.name(),
            self.item.clone(),
            self.nullable() || self.is_null(),
        )
        .with_metadata_overlay(self.metadata())
    }

    /// Whether the list value is null.
    pub fn is_null(&self) -> bool {
        self.null
    }

    /// The number of elements.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the list value has no elements.
    pub fn is_empty(&self) -> bool {
        self.items.len() == 0
    }

    /// The row's elements as an erased sub-column ([`AnySerie`](crate::io::AnySerie), downcast with
    /// `.as_serie::<T>()`).
    pub fn items(&self) -> &(dyn AnySerie + 'static) {
        self.items.as_ref()
    }

    /// The element (item) field descriptor.
    pub fn item_field(&self) -> &AnyField {
        &self.item
    }

    /// The element [`DataTypeId`] — always [`List`](DataTypeId::List).
    pub fn type_id(&self) -> DataTypeId {
        DataTypeId::List
    }

    /// The typed [`ListType`] descriptor of this value.
    pub fn data_type(&self) -> ListType {
        ListType::new(self.item.clone())
    }
}

impl PartialEq for ListScalar {
    fn eq(&self, other: &Self) -> bool {
        if self.null != other.null || self.item != other.item {
            return false;
        }
        // A null list's elements are logically absent, so they do not affect identity.
        self.null || self.items.eq_any(other.items.as_ref())
    }
}

impl Eq for ListScalar {}

impl core::hash::Hash for ListScalar {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.item.hash(state);
        self.null.hash(state);
        if !self.null {
            // Stay in lock-step with `PartialEq`: equal erased columns are byte-canonical, so hashing
            // the sub-column's frame keeps "equal values hash equal". A list value is a whole (short)
            // column, so this one allocation is acceptable (see `AnyScalar::hash`).
            self.items.serialize_bytes().hash(state);
        }
    }
}

impl ScalarType for ListScalar {
    type Data = ListType;

    fn data_type(&self) -> ListType {
        self.data_type()
    }

    fn is_null(&self) -> bool {
        self.null
    }
}
