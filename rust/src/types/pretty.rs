//! A readable, indented rendering of a schema - the alternate to [`Display`].
//!
//! [`Display`] on [`Field`] and [`DataType`] is the compact constructor form -
//! `field("price",decimal(9,2),nullable=false,...)` - and it stays exactly as
//! it is, because it round-trips through the parsers and is what `__repr__`,
//! the error messages, and the documentation depend on. It is also unreadable
//! the moment a struct nests three levels deep, which is the problem this
//! solves.
//!
//! The readable form is the **alternate**: `{:#}` on either type, backed by a
//! named [`Field::into_pretty_str`] / [`DataType::into_pretty_str`] spelling so
//! a caller who dislikes format flags has one too, and so one implementation
//! sits behind both.
//!
//! Each level shows the name, the datatype, nullability, and only the
//! attributes that are actually set - a `dictionary_id` of `0` or empty
//! metadata is noise, and the compact form already omits them. Nested `Struct`,
//! `List`, and `Map` children recurse one indent deeper, and metadata renders
//! as indented key-value lines rather than one braced blob. The output is
//! stable across runs: nothing here iterates a hash map.
//!
//! ```
//! use yggdryl::{DataType, Field};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let order = DataType::from_fields([
//!     DataType::Int64.required_field("id"),
//!     DataType::from_fields([DataType::Float64.required_field("price")])?
//!         .nullable_field("line"),
//! ])?
//! .required_field("order");
//!
//! // Compact stays compact, and still parses back.
//! assert_eq!(Field::from_str(&format!("{order}"))?, order);
//!
//! // Readable is the alternate, or the named adapter.
//! let readable = format!("{order:#}");
//! assert_eq!(readable, order.into_pretty_str());
//! assert_eq!(
//!     readable,
//!     "\
//! order: struct[2], required
//!   id: int64, required
//!   line: struct[1], nullable
//!     price: float64, required",
//! );
//! # Ok(())
//! # }
//! ```

use std::fmt;

use crate::types::enums::EnumType;
use crate::types::sequence::SequenceType;
use crate::{DataType, Field};

/// How many columns one nesting level is indented in the readable form.
const WIDTH: usize = 2;

impl Field {
    /// A readable, indented rendering of this field and everything under it.
    ///
    /// The named spelling of `{:#}`; both run the one implementation, and
    /// [`Pretty`] documents the shape they produce.
    #[must_use]
    #[allow(
        clippy::wrong_self_convention,
        reason = "the rendering borrows; the string is new"
    )]
    pub fn into_pretty_str(&self) -> String {
        self.pretty_view().to_string()
    }

    /// The borrowing adapter behind `{:#}` and [`Self::into_pretty_str`].
    #[must_use]
    pub const fn pretty_view(&self) -> Pretty<'_> {
        Pretty::Field(self)
    }
}

impl DataType {
    /// A readable, indented rendering of this datatype and its children.
    ///
    /// The named spelling of `{:#}`; both run the one implementation.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let rows = DataType::list(
    ///     DataType::from_fields([DataType::utf8().nullable_field("venue")])?.nullable_field("item"),
    /// );
    ///
    /// assert_eq!(
    ///     rows.into_pretty_str(),
    ///     "\
    /// list
    ///   item: struct[1], nullable
    ///     venue: utf8, nullable",
    /// );
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[allow(
        clippy::wrong_self_convention,
        reason = "the rendering borrows; the string is new"
    )]
    pub fn into_pretty_str(&self) -> String {
        self.pretty_view().to_string()
    }

    /// The borrowing adapter behind `{:#}` and [`Self::into_pretty_str`].
    #[must_use]
    pub const fn pretty_view(&self) -> Pretty<'_> {
        Pretty::DataType(self)
    }
}

/// A [`std::fmt::Display`] adapter rendering a schema node readably.
///
/// Built by [`Field::pretty_view`] and [`DataType::pretty_view`]; borrowing, so
/// building one allocates nothing and the rendering happens on write.
#[derive(Clone, Copy, Debug)]
pub enum Pretty<'node> {
    /// A field and everything under it.
    Field(&'node Field),
    /// A datatype and its children.
    DataType(&'node DataType),
}

impl fmt::Display for Pretty<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Field(field) => write_field(formatter, field, 0),
            Self::DataType(dtype) => write_dtype(formatter, dtype, 0, None),
        }
    }
}

/// Write one field's line, then everything below it.
fn write_field(formatter: &mut fmt::Formatter<'_>, field: &Field, columns: usize) -> fmt::Result {
    write_indent(formatter, columns)?;
    formatter.write_str(field.name())?;
    formatter.write_str(": ")?;
    write_head(formatter, field.dtype())?;
    formatter.write_str(if field.is_nullable() {
        ", nullable"
    } else {
        ", required"
    })?;
    // Only what is actually set: an unset attribute is noise, and the compact
    // form already omits it.
    if let Some(id) = field.dictionary_id().filter(|id| *id != 0) {
        write!(formatter, ", dictionary_id={id}")?;
    }
    if field.dictionary_is_ordered() == Some(true) {
        formatter.write_str(", dictionary_is_ordered")?;
    }
    let inner = columns + WIDTH;
    for (key, value) in field.metadata_iter() {
        formatter.write_str("\n")?;
        write_indent(formatter, inner)?;
        write!(formatter, "@{key} = {value}")?;
    }
    write_children(formatter, field.dtype(), inner)
}

/// Write a datatype at the root of the rendering, with no field line above it.
fn write_dtype(
    formatter: &mut fmt::Formatter<'_>,
    dtype: &DataType,
    columns: usize,
    name: Option<&str>,
) -> fmt::Result {
    write_indent(formatter, columns)?;
    if let Some(name) = name {
        formatter.write_str(name)?;
        formatter.write_str(": ")?;
    }
    write_head(formatter, dtype)?;
    write_children(formatter, dtype, columns + WIDTH)
}

/// Write the one-line head of a datatype: its family and its parameters.
///
/// A nested datatype's *children* are lines of their own, so the head names the
/// family alone - `struct`, `list` - while a leaf spells itself in full.
fn write_head(formatter: &mut fmt::Formatter<'_>, dtype: &DataType) -> fmt::Result {
    use DataType as D;
    match dtype {
        D::Structure(fields) => write!(formatter, "struct[{}]", fields.len()),
        D::Sequence(SequenceType::List(_)) => formatter.write_str("list"),
        D::Sequence(SequenceType::ListView(_)) => formatter.write_str("list_view"),
        D::Sequence(SequenceType::LargeList(_)) => formatter.write_str("large_list"),
        D::Sequence(SequenceType::LargeListView(_)) => formatter.write_str("large_list_view"),
        D::Sequence(SequenceType::FixedSizeList(_, length)) => {
            write!(formatter, "fixed_size_list[{length}]")
        }
        D::Mapping(map) => {
            formatter.write_str("map")?;
            if map.keys_sorted() {
                formatter.write_str("[keys_sorted]")?;
            }
            Ok(())
        }
        D::Union(fields, mode) => write!(formatter, "union[{mode},{}]", fields.len()),
        D::RunEndEncoded(_) => formatter.write_str("run_end_encoded"),
        D::Enum(EnumType::Dictionary(dictionary)) => {
            write!(formatter, "dictionary[{}]", dictionary.key())
        }
        // Everything else is one token, and the compact spelling is already
        // the readable one.
        other => write!(formatter, "{other}"),
    }
}

/// Write the children of a nested datatype, one indent deeper.
fn write_children(
    formatter: &mut fmt::Formatter<'_>,
    dtype: &DataType,
    columns: usize,
) -> fmt::Result {
    use DataType as D;
    match dtype {
        D::Structure(fields) => {
            for field in fields.as_fields() {
                formatter.write_str("\n")?;
                write_field(formatter, field, columns)?;
            }
            Ok(())
        }
        D::Sequence(SequenceType::List(field))
        | D::Sequence(SequenceType::ListView(field))
        | D::Sequence(SequenceType::LargeList(field))
        | D::Sequence(SequenceType::LargeListView(field))
        | D::Sequence(SequenceType::FixedSizeList(field, _)) => {
            formatter.write_str("\n")?;
            write_field(formatter, field, columns)
        }
        D::Mapping(map) => {
            formatter.write_str("\n")?;
            write_field(formatter, map.entries(), columns)
        }
        D::Union(fields, _) => {
            for (type_id, field) in fields.iter() {
                formatter.write_str("\n")?;
                write_indent(formatter, columns)?;
                write!(formatter, "#{type_id} ")?;
                // The tag is written above, so the member's own line continues
                // it rather than re-indenting.
                write_field_inline(formatter, field, columns)?;
            }
            Ok(())
        }
        D::RunEndEncoded(encoded) => {
            formatter.write_str("\n")?;
            write_field(formatter, encoded.run_ends(), columns)?;
            formatter.write_str("\n")?;
            write_field(formatter, encoded.values(), columns)
        }
        D::Enum(EnumType::Dictionary(dictionary)) => {
            formatter.write_str("\n")?;
            write_dtype(formatter, dictionary.value(), columns, Some("value"))
        }
        _ => Ok(()),
    }
}

/// Write a field's line without its leading indent, for a continued line.
fn write_field_inline(
    formatter: &mut fmt::Formatter<'_>,
    field: &Field,
    columns: usize,
) -> fmt::Result {
    formatter.write_str(field.name())?;
    formatter.write_str(": ")?;
    write_head(formatter, field.dtype())?;
    formatter.write_str(if field.is_nullable() {
        ", nullable"
    } else {
        ", required"
    })?;
    write_children(formatter, field.dtype(), columns + WIDTH)
}

/// Write `columns` spaces.
fn write_indent(formatter: &mut fmt::Formatter<'_>, columns: usize) -> fmt::Result {
    for _ in 0..columns {
        formatter.write_str(" ")?;
    }
    Ok(())
}
