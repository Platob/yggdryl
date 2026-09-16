//! Immutable group routing and nullable output projection.

use std::collections::HashMap;
use std::sync::Arc;

use crate::{DataType, Error, Field, Result, Scalar};

const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug)]
pub(super) struct GroupPlan {
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
    pub(super) fn from_field(field: &Field) -> Result<Self> {
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

    pub(super) const fn field(&self) -> &Field {
        &self.field
    }
    pub(super) fn column(&self, index: usize) -> &Field {
        &self.columns[index]
    }
    pub(super) fn columns_len(&self) -> usize {
        self.columns.len()
    }
    pub(super) fn tag_index(&self, tag: i32) -> Option<usize> {
        self.tags.get(&tag).copied().flatten()
    }
    pub(super) const fn delimiter(&self) -> Option<i32> {
        self.delimiter
    }
    pub(super) fn nested(&self, tag: i32) -> Option<(usize, &Self)> {
        let nested = &self.nested[self.groups.get(&tag).copied().flatten()?];
        Some((nested.column, &nested.plan))
    }
    pub(super) fn nested_plans(&self) -> impl Iterator<Item = (&[usize], &Arc<Self>)> {
        self.nested
            .iter()
            .map(|nested| (nested.path.as_slice(), &nested.plan))
    }
    pub(super) fn row(&self, values: Vec<Scalar>) -> Scalar {
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
        DataType::Struct(fields) => DataType::from_fields(
            fields
                .iter()
                .map(|child| nullable_layout(child, true, depth + 1))
                .collect::<Result<Vec<_>>>()?,
        )?,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FixCategory, FixRegistry, MsgType};

    fn tagged(name: &str, tag: i32, dtype: DataType) -> Field {
        let mut field = dtype.required_field(name);
        field.as_fix_mut().set_tag(tag).unwrap();
        field
    }

    fn parties() -> Field {
        let subparty = DataType::from_fields([tagged("PartySubID", 523, DataType::utf8())])
            .unwrap()
            .required_field("SubParty");
        let mut nested = DataType::large_list(subparty).required_field("SubParties");
        nested.as_fix_mut().set_counter(802).unwrap();
        let attribution = DataType::from_fields([tagged("PartyRole", 452, DataType::Int32)])
            .unwrap()
            .required_field("Attribution");
        let item = DataType::from_fields([
            tagged("PartyID", 448, DataType::utf8()),
            attribution,
            tagged("NoPartySubIDs", 802, DataType::Int32),
            nested,
        ])
        .unwrap()
        .required_field("Party");
        let mut group = DataType::list(item).required_field("Parties");
        group.as_fix_mut().set_counter(453).unwrap();
        group
    }

    #[test]
    fn plans_flatten_components_preserve_list_width_and_project_nullable_members() {
        let source = parties();
        let plan = GroupPlan::from_field(&source).unwrap();
        assert_eq!(plan.columns_len(), 4);
        assert_eq!(plan.delimiter(), Some(448));
        assert_eq!(plan.tag_index(452), Some(1));
        assert_eq!(plan.tag_index(802), Some(2));
        assert!(plan.column(1).is_nullable());
        let (column, nested) = plan.nested(802).unwrap();
        assert_eq!(column, 3);
        assert!(matches!(nested.field().dtype(), DataType::LargeList(_)));
        assert_eq!(nested.tag_index(523), Some(0));
        let row = plan.row(vec![
            Scalar::from("broker"),
            Scalar::Null,
            Scalar::from(0_i32),
            Scalar::Null,
        ]);
        assert_eq!(
            row,
            Scalar::from_sequence([
                Scalar::from("broker"),
                Scalar::Null,
                Scalar::from(0_i32),
                Scalar::Null,
            ])
        );
        let DataType::List(item) = source.dtype() else {
            panic!("list")
        };
        assert!(!item.fields()[0].is_nullable());
    }

    #[test]
    fn message_and_nested_scope_borrow_one_precompiled_plan() {
        let mut field = DataType::from_fields([parties()])
            .unwrap()
            .required_field("Report");
        field.as_fix_mut().set_msgtype("R").unwrap();
        let message = MsgType::from_field(field).unwrap();
        let outer = message.get_group_plan_by_tag(453).unwrap();
        let nested = message.get_group_plan_by_tag(802).unwrap();
        assert!(std::ptr::eq(outer.nested(802).unwrap().1, nested));
        assert!(std::ptr::eq(
            outer,
            message.get_group_plan_by_tag(453).unwrap(),
        ));
        let cloned = message.clone();
        assert!(std::ptr::eq(
            outer,
            cloned.get_group_plan_by_tag(453).unwrap(),
        ));
    }

    #[test]
    fn registry_clones_share_plans_and_replacements_recompile_once() {
        let mut registry =
            FixRegistry::from_fields([tagged("NoPartyIDs", 453, DataType::Int32)]).unwrap();
        registry
            .insert_definition(FixCategory::Groups, parties())
            .unwrap();
        let snapshot = registry.clone();
        let original = snapshot.get_group_plan_by_tag(453).unwrap();
        assert!(std::ptr::eq(
            original,
            registry.get_group_plan_by_tag(453).unwrap()
        ));
        let item = DataType::from_fields([tagged("PartyRole", 452, DataType::Int32)])
            .unwrap()
            .required_field("Party");
        let mut replacement = DataType::list(item).required_field("Parties");
        replacement.as_fix_mut().set_counter(453).unwrap();
        registry
            .update_definition(FixCategory::Groups, replacement)
            .unwrap();
        let current = registry.get_group_plan_by_tag(453).unwrap();
        assert!(!std::ptr::eq(original, current));
        assert_eq!(original.delimiter(), Some(448));
        assert_eq!(current.delimiter(), Some(452));
    }

    #[test]
    fn maps_keep_native_key_shape_and_declare_no_numeric_wire_layout() {
        let mut field = DataType::map_of(DataType::utf8(), DataType::utf8(), true)
            .unwrap()
            .nullable_field("nativeids");
        field.as_fix_mut().set_tag(65_090).unwrap();
        field.as_fix_mut().set_counter(65_090).unwrap();
        let plan = GroupPlan::from_field(&field).unwrap();
        let DataType::Map(map) = plan.field().dtype() else {
            panic!("the native Map layout is preserved")
        };
        assert!(map.keys_sorted());
        assert!(!map.entries().is_nullable());
        assert!(!map.entries().fields()[0].is_nullable());
        assert_eq!(plan.delimiter(), None);
        assert!(plan.tags.is_empty());
        assert_eq!(
            plan.row(vec![Scalar::from("orderid"), Scalar::from("O-1")]),
            Scalar::from_sequence([Scalar::from("orderid"), Scalar::from("O-1")]),
        );
        let mut registry = FixRegistry::new();
        registry
            .insert_definition(FixCategory::Groups, field)
            .unwrap();
        assert!(registry.get_group_by_tag(65_090).is_some());
        assert!(registry.get_group_plan_by_tag(65_090).is_none());
    }
}
