//! Immutable group routing and nullable output projection.

use std::collections::HashMap;
use std::sync::Arc;

use crate::{DataType, Error, Field, Result, Scalar, StructType};

const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug)]
pub struct GroupPlan {
    field: Field,
    columns: Vec<Field>,
    tags: HashMap<i32, Option<usize>>,
    groups: HashMap<i32, Option<usize>>,
    nested: Vec<NestedGroup>,
    delimiter: Option<i32>,
}

#[derive(Clone, Debug)]
struct NestedGroup {
    column: usize,
    path: Vec<usize>,
    plan: Arc<GroupPlan>,
}

impl GroupPlan {
    pub fn from_field(field: &Field) -> Result<Self> {
        Self::from_projection(nullable_layout(field, false, 0)?, 0)
    }

    fn from_projection(field: Field, depth: usize) -> Result<Self> {
        check_depth(&field, depth)?;
        let item = super::catalog::occurrence_of(&field).ok_or_else(|| Error::InvalidRecord {
            path: field.name().into(),
            reason: "expected a FIX group List, LargeList or Map".into(),
        })?;
        let mut columns = Vec::new();
        let mut paths = Vec::new();
        Self::columns(item, &mut Vec::new(), &mut columns, &mut paths, depth + 1)?;
        let mut tags = HashMap::new();
        let mut groups = HashMap::new();
        let mut nested = Vec::new();
        let mut delimiter = None;
        for (index, column) in columns.iter().enumerate() {
            if let Some(tag) = column.as_fix().tag()? {
                delimiter.get_or_insert(tag);
                tags.entry(tag)
                    .and_modify(|held| *held = None)
                    .or_insert(Some(index));
            }
            if let Some(tag) = column.as_fix().counter()? {
                let position = nested.len();
                groups
                    .entry(tag)
                    .and_modify(|held| *held = None)
                    .or_insert(Some(position));
                nested.push(NestedGroup {
                    column: index,
                    path: paths[index].clone(),
                    plan: Arc::new(Self::from_projection(
                        column.clone(),
                        depth + paths[index].len() + 1,
                    )?),
                });
            }
        }
        Ok(Self {
            field,
            columns,
            tags,
            groups,
            nested,
            delimiter,
        })
    }

    fn columns(
        field: &Field,
        path: &mut Vec<usize>,
        columns: &mut Vec<Field>,
        paths: &mut Vec<Vec<usize>>,
        depth: usize,
    ) -> Result<()> {
        check_depth(field, depth)?;
        for (index, child) in field.fields().iter().enumerate() {
            path.push(index);
            if matches!(child.dtype(), DataType::Struct(_)) {
                Self::columns(child, path, columns, paths, depth + 1)?;
            } else {
                columns.push(child.clone());
                paths.push(path.clone());
            }
            path.pop();
        }
        Ok(())
    }

    pub const fn field(&self) -> &Field {
        &self.field
    }
    pub fn column(&self, index: usize) -> &Field {
        &self.columns[index]
    }
    pub fn columns_len(&self) -> usize {
        self.columns.len()
    }
    pub fn tag_index(&self, tag: i32) -> Option<usize> {
        self.tags.get(&tag).copied().flatten()
    }
    pub const fn delimiter(&self) -> Option<i32> {
        self.delimiter
    }
    pub fn nested(&self, tag: i32) -> Option<(usize, &Self)> {
        let nested = &self.nested[self.groups.get(&tag).copied().flatten()?];
        Some((nested.column, &nested.plan))
    }
    pub(super) fn nested_plans(&self) -> impl Iterator<Item = (&[usize], &Arc<Self>)> {
        self.nested
            .iter()
            .map(|nested| (nested.path.as_slice(), &nested.plan))
    }
    pub fn row(&self, values: Vec<Scalar>) -> Scalar {
        let item =
            super::catalog::occurrence_of(&self.field).expect("a compiled group has an occurrence");
        component_value(item, &mut values.into_iter(), true)
    }
}

fn check_depth(field: &Field, depth: usize) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(Error::InvalidRecord {
            path: field.name().into(),
            reason: "numeric FIX group layouts are nested at most 64 levels".into(),
        });
    }
    Ok(())
}

fn nullable_layout(field: &Field, nullable: bool, depth: usize) -> Result<Field> {
    check_depth(field, depth)?;
    let mut held = field.clone();
    let dtype = match field.dtype() {
        DataType::Struct(fields) => DataType::from(StructType::from_fields(
            fields
                .iter()
                .map(|child| nullable_layout(child, true, depth + 1))
                .collect::<Result<Vec<_>>>()?,
        )?),
        DataType::List(item) => DataType::list(nullable_layout(item, false, depth + 1)?),
        DataType::LargeList(item) => DataType::large_list(nullable_layout(item, false, depth + 1)?),
        // Native maps are already complete values, not sparse wire groups:
        // their entry and key nullability must remain exactly as declared.
        _ => field.dtype().clone(),
    };
    held.set_dtype(dtype)?;
    held.set_nullable(nullable);
    Ok(held)
}

fn component_value(field: &Field, values: &mut std::vec::IntoIter<Scalar>, root: bool) -> Scalar {
    let children: Vec<_> = field
        .fields()
        .iter()
        .map(|child| {
            if matches!(child.dtype(), DataType::Struct(_)) {
                component_value(child, values, false)
            } else {
                values
                    .next()
                    .expect("a compiled component has one value per column")
            }
        })
        .collect();
    if !root && children.iter().all(Scalar::is_null) {
        Scalar::Null
    } else {
        Scalar::from_sequence(children)
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/fix/group_plan.rs` pins and a caller cannot reach.
    //!
    //! A plan is compiled once per group definition and borrowed by every
    //! message and every registry clone reading it; a caller sees the row it
    //! lays out and never the layout. `fix::group_plan` is a private module of
    //! a published one, so `GroupPlan` being `pub` reaches nobody: this door is
    //! the only path to it, and it exists under the `internals` feature alone.
    pub use super::GroupPlan;

    /// Whether the plan declares no numeric wire layout at all, which is what
    /// a crate-owned Map group is.
    #[must_use]
    pub fn tags_is_empty(plan: &GroupPlan) -> bool {
        plan.tags.is_empty()
    }
}
