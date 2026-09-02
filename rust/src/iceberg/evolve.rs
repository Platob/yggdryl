//! Column-level schema evolution: the next schema, derived from the current.
//!
//! Iceberg identifies a column by its field id, not by its name or position,
//! which is what makes evolution safe: a rename keeps the id, a drop retires
//! the id forever, and an added column takes a fresh id above
//! `last-column-id`. [`SchemaUpdate`] holds that rule so a caller cannot
//! break it - it captures a table's current schema, records column
//! operations, and produces the evolved root that
//! [`TableMetadata::add_schema`] then numbers into the table's history.
//!
//! Type changes are the other half of the contract. A reader must be able to
//! widen every stored value into the new type, so only the promotions the
//! Iceberg specification lists are legal, and [`can_promote`] is the one
//! place that list lives.
//!
//! ```
//! use yggdryl::iceberg::{
//!     FormatVersion, PartitionSpec, SchemaUpdate, TableMetadata, assign_field_ids,
//! };
//! use yggdryl::DataType;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut schema = DataType::from_fields([
//!     DataType::Int32.required_field("id"),
//!     DataType::Utf8.nullable_field("symbol"),
//! ])?
//! .required_field("row");
//! assign_field_ids(&mut schema, 1)?;
//! let mut metadata = TableMetadata::new(
//!     FormatVersion::V2,
//!     "file:///tmp/trades",
//!     schema,
//!     PartitionSpec::unpartitioned(),
//! )?;
//!
//! let mut update = SchemaUpdate::from_metadata(&metadata)?;
//! update.add_column("", DataType::Int64.nullable_field("quantity"));
//! update.update_type("id", DataType::Int64);
//! let evolved = update.into_field()?;
//!
//! // The added column is numbered above every id the table has assigned.
//! assert_eq!(evolved.get_field_by_name("quantity").unwrap().parquet_field_id()?, Some(3));
//!
//! let schema_id = metadata.add_schema(evolved)?;
//! metadata.set_current_schema(schema_id)?;
//! assert_eq!(metadata.last_column_id(), 3);
//! # Ok(())
//! # }
//! ```

use smol_str::{SmolStr, format_smolstr};

use super::TableMetadata;
use crate::text::elide_to;
use crate::{DataType, Error, Field, Result};

/// How many bytes of a caller-supplied path an error message shows.
const PATH_LIMIT: usize = 64;

/// Check one type change against the promotions Iceberg allows.
///
/// A promotion is legal when every value already stored reads back losslessly
/// as the new type: `int` to `long`, `float` to `double`, and a decimal to a
/// decimal of higher precision at the same scale - across the physical
/// `decimal32`/`decimal64`/`decimal128` widths, staying within Iceberg's
/// precision ceiling of 38. Identical types pass, because a no-op cannot lose
/// anything.
///
/// ```
/// use yggdryl::DataType;
/// use yggdryl::iceberg::can_promote;
///
/// # fn main() -> yggdryl::Result<()> {
/// can_promote(&DataType::Int32, &DataType::Int64)?;
/// can_promote(&DataType::decimal64(10, 2)?, &DataType::decimal128(20, 2)?)?;
/// assert!(can_promote(&DataType::Int64, &DataType::Int32).is_err());
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error naming both types for every other change.
pub fn can_promote(from: &DataType, to: &DataType) -> Result<()> {
    if from == to {
        return Ok(());
    }
    let legal = match (from, to) {
        (DataType::Int32, DataType::Int64) | (DataType::Float32, DataType::Float64) => true,
        _ => match (decimal_parts(from), decimal_parts(to)) {
            (Some((precision, scale)), Some((to_precision, to_scale))) => {
                to_scale == scale && to_precision >= precision && to_precision <= 38
            }
            _ => false,
        },
    };
    if legal {
        return Ok(());
    }
    Err(invalid(format_smolstr!(
        "expected an Iceberg-legal promotion, got {from} to {to}"
    )))
}

/// Return the precision and scale of a decimal Iceberg can spell.
///
/// `decimal256` is deliberately absent: Iceberg's decimal stops at precision
/// 38, so a 256-bit decimal is not a promotion target the format can store.
const fn decimal_parts(data_type: &DataType) -> Option<(u8, i8)> {
    match data_type {
        DataType::Decimal32 { precision, scale }
        | DataType::Decimal64 { precision, scale }
        | DataType::Decimal128 { precision, scale } => Some((*precision, *scale)),
        _ => None,
    }
}

/// A recorded set of column operations against a table's current schema.
///
/// Built by [`SchemaUpdate::from_metadata`], which captures the current schema
/// and `last-column-id`. The recording methods store operations without
/// touching anything; [`SchemaUpdate::into_field`] then plays them back in call
/// order, numbers every added column above the captured `last-column-id`, and
/// returns the evolved root ready for [`TableMetadata::add_schema`].
///
/// A path is dotted: `"quote.price"` names the column `price` inside the
/// struct column `quote`, and the empty parent `""` names the root itself.
#[derive(Clone, Debug)]
pub struct SchemaUpdate {
    /// The schema the update starts from.
    schema: Field,
    /// The identifier numbering continues from, one above `last-column-id`.
    next_id: i32,
    /// The recorded operations, in call order.
    ops: Vec<Op>,
}

/// One recorded column operation.
#[derive(Clone, Debug)]
enum Op {
    /// Append a column to the root or to a nested struct.
    AddColumn { parent: SmolStr, field: Field },
    /// Remove a column, retiring its identifier forever.
    DropColumn { path: SmolStr },
    /// Rename a column, keeping its identifier.
    RenameColumn { path: SmolStr, name: SmolStr },
    /// Set a column's `iceberg:doc` documentation string.
    UpdateDoc { path: SmolStr, doc: SmolStr },
    /// Relax a required column to optional.
    MakeNullable { path: SmolStr },
    /// Promote a column's type, gated by [`can_promote`].
    UpdateType { path: SmolStr, data_type: DataType },
}

impl SchemaUpdate {
    /// Start an update from a table's current schema and `last-column-id`.
    ///
    /// # Errors
    ///
    /// Returns an error when the metadata's current schema id resolves to no
    /// schema, or when `last-column-id` cannot grow.
    pub fn from_metadata(metadata: &TableMetadata) -> Result<Self> {
        let schema = metadata.current_schema()?.clone();
        let next_id = metadata.last_column_id.checked_add(1).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a last-column-id below {}, got {}",
                i32::MAX,
                metadata.last_column_id
            ))
        })?;
        Ok(Self {
            schema,
            next_id,
            ops: Vec::new(),
        })
    }

    /// Record a new column under `parent` - `""` for the root, a dotted path
    /// for a nested struct.
    ///
    /// On apply the column and every child it has are numbered fresh above
    /// the captured `last-column-id`, depth first, so a retired identifier is
    /// never reused; identifiers already on the given field are discarded.
    pub fn add_column(&mut self, parent: &str, field: Field) {
        self.ops.push(Op::AddColumn {
            parent: SmolStr::new(parent),
            field,
        });
    }

    /// Record the removal of the column at `path`, retiring its identifier.
    pub fn drop_column(&mut self, path: &str) {
        self.ops.push(Op::DropColumn {
            path: SmolStr::new(path),
        });
    }

    /// Record a rename of the column at `path`; its identifier is kept.
    pub fn rename_column(&mut self, path: &str, name: impl Into<SmolStr>) {
        self.ops.push(Op::RenameColumn {
            path: SmolStr::new(path),
            name: name.into(),
        });
    }

    /// Record a new `iceberg:doc` documentation string on the column at
    /// `path`, through the field's Iceberg protocol view.
    pub fn update_doc(&mut self, path: &str, doc: impl Into<SmolStr>) {
        self.ops.push(Op::UpdateDoc {
            path: SmolStr::new(path),
            doc: doc.into(),
        });
    }

    /// Record that the column at `path` becomes optional.
    ///
    /// Required to optional is the only direction nullability can evolve: a
    /// row written before the change never holds a null, but the reverse
    /// would declare rows that may already hold one as never-null, so no
    /// method offers it.
    pub fn make_nullable(&mut self, path: &str) {
        self.ops.push(Op::MakeNullable {
            path: SmolStr::new(path),
        });
    }

    /// Record a type promotion on the column at `path`, checked against
    /// [`can_promote`] when the update is applied.
    pub fn update_type(&mut self, path: &str, data_type: DataType) {
        self.ops.push(Op::UpdateType {
            path: SmolStr::new(path),
            data_type,
        });
    }

    /// Play the recorded operations back, in call order, and return the
    /// evolved schema root, validated and ready for
    /// [`TableMetadata::add_schema`].
    ///
    /// # Errors
    ///
    /// Returns the first operation's failure - a path that does not resolve,
    /// a name collision, an illegal promotion - or the final root's
    /// validation failure. The path errors name the missing segment and the
    /// columns that exist beside it.
    pub fn into_field(self) -> Result<Field> {
        let Self {
            mut schema,
            mut next_id,
            ops,
        } = self;
        for op in ops {
            match op {
                Op::AddColumn { parent, field } => {
                    apply_add(&mut schema, &parent, field)?;
                    next_id = schema.assign_parquet_field_ids(next_id)?;
                }
                Op::DropColumn { path } => apply_drop(&mut schema, &path)?,
                Op::RenameColumn { path, name } => apply_rename(&mut schema, &path, name)?,
                Op::UpdateDoc { path, doc } => apply_doc(&mut schema, &path, &doc)?,
                Op::MakeNullable { path } => apply_nullable(&mut schema, &path)?,
                Op::UpdateType { path, data_type } => apply_type(&mut schema, &path, data_type)?,
            }
        }
        schema.validate_struct_root()?;
        Ok(schema)
    }
}

/// Append one column, stripped of any stale identifiers, under `parent`.
fn apply_add(schema: &mut Field, parent: &str, mut field: Field) -> Result<()> {
    strip_ids(&mut field)?;
    let segments: Vec<&str> = if parent.is_empty() {
        Vec::new()
    } else {
        parent.split('.').collect()
    };
    edit_children(schema, &segments, parent, |children| {
        if children.iter().any(|child| child.name() == field.name()) {
            return Err(invalid(format_smolstr!(
                "expected a column name not already under {:?}, got {:?}",
                elide_to(parent, PATH_LIMIT),
                elide_to(field.name(), PATH_LIMIT)
            )));
        }
        children.push(field);
        Ok(())
    })
}

/// Remove the column at `path`.
fn apply_drop(schema: &mut Field, path: &str) -> Result<()> {
    let (segments, name) = split_column_path(path)?;
    edit_children(schema, &segments, path, |children| {
        let Some(index) = children.iter().position(|child| child.name() == name) else {
            return Err(missing_column(name, children, path));
        };
        children.remove(index);
        Ok(())
    })
}

/// Rename the column at `path`, keeping its identifier.
fn apply_rename(schema: &mut Field, path: &str, name: SmolStr) -> Result<()> {
    let (segments, target) = split_column_path(path)?;
    edit_children(schema, &segments, path, |children| {
        if children
            .iter()
            .any(|child| child.name() == name && child.name() != target)
        {
            return Err(invalid(format_smolstr!(
                "expected an unused name for {:?}, got {:?} which a sibling already carries",
                elide_to(path, PATH_LIMIT),
                elide_to(&name, PATH_LIMIT)
            )));
        }
        let Some(index) = children.iter().position(|child| child.name() == target) else {
            return Err(missing_column(target, children, path));
        };
        children[index].set_name(name);
        Ok(())
    })
}

/// Set the `iceberg:doc` property on the column at `path`.
fn apply_doc(schema: &mut Field, path: &str, doc: &str) -> Result<()> {
    let (segments, target) = split_column_path(path)?;
    edit_children(schema, &segments, path, |children| {
        let Some(index) = children.iter().position(|child| child.name() == target) else {
            return Err(missing_column(target, children, path));
        };
        children[index]
            .iceberg_mut()
            .insert(super::schema::DOC, doc)?;
        Ok(())
    })
}

/// Relax the column at `path` to optional.
fn apply_nullable(schema: &mut Field, path: &str) -> Result<()> {
    let (segments, target) = split_column_path(path)?;
    edit_children(schema, &segments, path, |children| {
        let Some(index) = children.iter().position(|child| child.name() == target) else {
            return Err(missing_column(target, children, path));
        };
        children[index].set_nullable(true);
        Ok(())
    })
}

/// Promote the type of the column at `path`.
fn apply_type(schema: &mut Field, path: &str, data_type: DataType) -> Result<()> {
    let (segments, target) = split_column_path(path)?;
    edit_children(schema, &segments, path, |children| {
        let Some(index) = children.iter().position(|child| child.name() == target) else {
            return Err(missing_column(target, children, path));
        };
        can_promote(children[index].data_type(), &data_type).map_err(|error| match error {
            Error::Codec {
                format,
                position,
                reason,
            } => Error::Codec {
                format,
                position,
                reason: format_smolstr!("{reason} at {:?}", elide_to(path, PATH_LIMIT)),
            },
            other => other,
        })?;
        children[index].set_data_type(data_type)
    })
}

/// Split a dotted column path into its parent segments and the column name.
fn split_column_path(path: &str) -> Result<(Vec<&str>, &str)> {
    if path.is_empty() {
        return Err(invalid(SmolStr::new_static(
            "expected a dotted column path, got \"\"",
        )));
    }
    let mut segments: Vec<&str> = path.split('.').collect();
    let name = segments.pop().unwrap_or_default();
    Ok((segments, name))
}

/// Walk `segments` down nested structs and edit the children at the end.
///
/// The mutation rebuilds every struct on the way back up, because a field's
/// children live behind its shared datatype; an error on the way down leaves
/// the schema untouched.
fn edit_children<F>(node: &mut Field, segments: &[&str], path: &str, edit: F) -> Result<()>
where
    F: FnOnce(&mut Vec<Field>) -> Result<()>,
{
    let mut children = node.fields().to_vec();
    let Some((first, rest)) = segments.split_first() else {
        edit(&mut children)?;
        return node.set_data_type(DataType::from_fields(children)?);
    };
    let Some(index) = children.iter().position(|child| child.name() == *first) else {
        return Err(missing_column(first, &children, path));
    };
    if !children[index].is_struct() {
        return Err(invalid(format_smolstr!(
            "expected a struct column {:?} at {:?}, got {}",
            elide_to(first, PATH_LIMIT),
            elide_to(path, PATH_LIMIT),
            crate::text::elide_display(children[index].data_type())
        )));
    }
    edit_children(&mut children[index], rest, path, edit)?;
    node.set_data_type(DataType::from_fields(children)?)
}

/// Report a path segment that names no column, and the columns that exist.
fn missing_column(segment: &str, siblings: &[Field], path: &str) -> Error {
    let names: Vec<&str> = siblings.iter().map(Field::name).collect();
    invalid(format_smolstr!(
        "expected a column {:?} at {:?}, got {names:?}",
        elide_to(segment, PATH_LIMIT),
        elide_to(path, PATH_LIMIT)
    ))
}

/// Remove every identifier from a field tree, over all of its layouts.
///
/// An added column may be a copy of one that already lived in a schema, and
/// carrying that identifier in would resurrect a retired id; numbering is
/// [`Field::assign_parquet_field_ids`]'s job alone.
fn strip_ids(field: &mut Field) -> Result<()> {
    field.remove_parquet_field_id()?;
    let count = field.data_type().field_len();
    if count == 0 {
        return Ok(());
    }
    let mut children = Vec::with_capacity(count);
    for index in 0..count {
        let Some(child) = field.data_type().get_field(index) else {
            continue;
        };
        let mut child = child.clone();
        strip_ids(&mut child)?;
        children.push(child);
    }
    field.set_data_type(field.data_type().with_fields(children)?)
}

/// Report a malformed Iceberg schema evolution.
fn invalid(reason: SmolStr) -> Error {
    Error::Codec {
        format: "iceberg",
        position: 0,
        reason,
    }
}

#[cfg(test)]
mod tests;
