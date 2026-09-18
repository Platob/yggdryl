//! Union: one of several field variants per row.

use std::cmp::Ordering;
use std::fmt;
use std::ops::Index;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::types::structure::cmp_fields;
use crate::types::dtype::invalid;
use smol_str::format_smolstr;
use crate::{
    DataType, Field, Result, UnionMode,

};

/// Union members paired with their non-negative Arrow type IDs.
#[derive(Clone, Default, Eq, PartialEq, Hash)]
pub struct UnionFields(pub(crate) Option<Arc<[(i8, Field)]>>);

impl UnionFields {
    /// Builds union members and rejects duplicate or negative type IDs.
    pub fn from_fields<I>(values: I) -> Result<Self>
    where
        I: IntoIterator<Item = (i8, Field)>,
    {
        let values = values.into_iter().collect::<Vec<_>>();
        validate_union_values(&values, true)?;
        Ok(Self::from_vec(values))
    }

    /// Returns union members in their declared order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (i8, &Field)> {
        self.as_ref()
            .iter()
            .map(|(type_id, field)| (*type_id, field))
    }

    /// Returns the number of members.
    pub fn len(&self) -> usize {
        self.as_ref().len()
    }

    /// Returns whether the union has no members.
    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    /// Returns the member at `index`.
    pub fn get(&self, index: usize) -> Option<(i8, &Field)> {
        self.as_ref().get(index).map(|(id, field)| (*id, field))
    }

    /// Finds a member by exact field name.
    pub fn get_by_name(&self, name: &str) -> Option<(i8, &Field)> {
        self.iter().find(|(_, field)| field.name() == name)
    }

    /// Returns the shared member slice without allocating.
    pub fn as_fields(&self) -> &[(i8, Field)] {
        self.as_ref()
    }

    pub(crate) fn shares_storage_with(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    /// Consumes the collection and returns owned type-ID/field pairs.
    pub fn into_fields(self) -> Vec<(i8, Field)> {
        self.as_ref().to_vec()
    }

    fn as_ref(&self) -> &[(i8, Field)] {
        self.0.as_deref().unwrap_or_default()
    }

    fn from_vec(values: Vec<(i8, Field)>) -> Self {
        if values.is_empty() {
            Self(None)
        } else {
            Self(Some(values.into()))
        }
    }

    pub(crate) fn from_imported_fields(values: Vec<(i8, Field)>) -> Result<Self> {
        validate_union_values(&values, false)?;
        Ok(Self::from_vec(values))
    }
}

impl fmt::Debug for UnionFields {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

impl Ord for UnionFields {
    fn cmp(&self, other: &Self) -> Ordering {
        if matches!((&self.0, &other.0), (Some(left), Some(right)) if Arc::ptr_eq(left, right)) {
            return Ordering::Equal;
        }
        let mut left = self.as_ref().iter();
        let mut right = other.as_ref().iter();
        loop {
            match (left.next(), right.next()) {
                (Some((left_id, left_field)), Some((right_id, right_field))) => {
                    let order = left_id
                        .cmp(right_id)
                        .then_with(|| cmp_fields(left_field, right_field));
                    if order != Ordering::Equal {
                        return order;
                    }
                }
                (None, None) => return Ordering::Equal,
                (None, Some(_)) => return Ordering::Less,
                (Some(_), None) => return Ordering::Greater,
            }
        }
    }
}

impl PartialOrd for UnionFields {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Index<usize> for UnionFields {
    type Output = (i8, Field);

    fn index(&self, index: usize) -> &Self::Output {
        &self.as_ref()[index]
    }
}

impl IntoIterator for UnionFields {
    type Item = (i8, Field);
    type IntoIter = std::vec::IntoIter<(i8, Field)>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_fields().into_iter()
    }
}

impl Serialize for UnionFields {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.as_ref().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for UnionFields {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let fields = Vec::<(i8, Field)>::deserialize(deserializer)?;
        Self::from_fields(fields).map_err(serde::de::Error::custom)
    }
}

impl DataType {

    /// Creates a union after validating field names and type IDs.
    pub fn union<I>(fields: I, mode: UnionMode) -> Result<Self>
    where
        I: IntoIterator<Item = (i8, Field)>,
    {
        Ok(Self::Union(UnionFields::from_fields(fields)?, mode))
    }

    /// Creates a finite dense union with sequential type IDs.
    ///
    /// Members retain their input order and receive IDs `0..`. The result is
    /// the canonical [`DataType::Union`] representation rather than a second
    /// logical datatype, so display, serialization, Arrow projection, and
    /// record materialization all reuse the union contract.
    ///
    /// This used to be spelled `variant`, after the input sugar the parser
    /// still accepts - `variant(a: int32, b: utf8)` remains that sugar, and
    /// its canonical display remains `union(dense, ...)`. The name moved
    /// because bare `variant` is now a datatype of its own - the
    /// self-describing semi-structured value Iceberg v3, Parquet, and Doris
    /// share - and one word cannot name both a union of declared members and
    /// a value that declares itself.
    pub fn dense_union<I>(fields: I) -> Result<Self>
    where
        I: IntoIterator<Item = Field>,
    {
        let fields = fields
            .into_iter()
            .enumerate()
            .map(|(index, field)| {
                i8::try_from(index)
                    .map(|type_id| (type_id, field))
                    .map_err(|_| {
                        invalid("Variant", "a variant cannot contain more than 128 members")
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        Self::union(fields, UnionMode::Dense)
    }
}

impl<'a> IntoIterator for &'a UnionFields {
    type Item = (i8, &'a Field);
    type IntoIter =
        std::iter::Map<std::slice::Iter<'a, (i8, Field)>, fn(&(i8, Field)) -> (i8, &Field)>;

    fn into_iter(self) -> Self::IntoIter {
        fn borrow_member((type_id, field): &(i8, Field)) -> (i8, &Field) {
            (*type_id, field)
        }
        self.as_ref().iter().map(borrow_member)
    }
}

pub(crate) fn validate_union_fields(fields: &UnionFields) -> Result<()> {
    validate_union_values(fields.as_fields(), true)
}

pub(crate) fn validate_union_values(values: &[(i8, Field)], validate_children: bool) -> Result<()> {
    let mut seen = 0_u128;
    for (index, (type_id, field)) in values.iter().enumerate() {
        if *type_id < 0 {
            return Err(invalid(
                "Union",
                format_smolstr!("type id must be non-negative: {type_id}"),
            ));
        }
        let mask = 1_u128 << *type_id;
        if seen & mask != 0 {
            return Err(invalid(
                "Union",
                format_smolstr!("duplicate type id: {type_id}"),
            ));
        }
        seen |= mask;
        if values[..index]
            .iter()
            .any(|(_, previous)| previous.name() == field.name())
        {
            return Err(invalid(
                "Union",
                format_smolstr!("duplicate field name {:?}", field.name()),
            ));
        }
        if validate_children {
            field.validate()?;
        }
    }
    Ok(())
}
