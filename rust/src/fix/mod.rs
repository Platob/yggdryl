//! FIX field definitions: the `fix:` vocabulary, a registry that resolves
//! them, the shards it persists to, the process-wide default, and the
//! message value that is typed against one.
//!
//! A FIX field is a [`Field`](crate::Field) whose metadata carries the `fix:`
//! namespace, read and written through [`FixField`](crate::FixField) and
//! [`FixFieldMut`](crate::FixFieldMut) - nobody spells `fix:` at a call site.
//! The canonical name is the field's own `name()`, the datatype its own
//! `dtype()`, and the display name the generic `display` key; the namespace
//! adds only what FIX states beyond a field:
//!
//! | property | key | type | meaning |
//! | --- | --- | --- | --- |
//! | tag | `fix:tag` | `i32` | canonical FIX tag |
//! | tags | `fix:tags` | ordered `i32` list | alternate tags, highest priority first |
//! | aliases | `fix:aliases` | ordered name list | alternate names, highest priority first |
//! | description | `fix:description` | text | the specification's own wording |
//!
//! Nesting needs no second type: a component is a Struct field whose
//! children are its members, a repeating group is a List field whose item is
//! that Struct, and the group's counter tag is the group field's own `fix:tag`.
//!
//! # Resolution
//!
//! [`FixRegistry`] answers a tag or a name through four tiers, and a later
//! tier is consulted only when every earlier one missed:
//!
//! 1. canonical tag, then alternate tags;
//! 2. canonical name folded, then aliases folded.
//!
//! Names fold ASCII case once, on the way in, so a query spelled in any case
//! finds the field and the answer is always the canonical spelling. A tag
//! query never consults names and a name query never consults tags, and
//! an alias can never take a name away from a field that claims it
//! canonically. [`FixKey`] carries either kind of key through the generic
//! [`FixRegistry::get_field`] / [`FixRegistry::field`] pair, which match once
//! and redirect to the specialized accessor for that kind.
//!
//! # Storage
//!
//! A registry reads and writes through one [`IOBase`](crate::io::IOBase)
//! folder handle. Shards live at `<root>/records/<shard>.json` with
//! `shard = tag / 100`, each a JSON array of the core field document ordered
//! by canonical tag, so a tag reaches exactly one shard by arithmetic and an
//! alternate tag never fans a field across shards. Every shard is loaded on
//! open: [`FixRegistry::from_handle`] lists `records/` and reads each shard
//! into the indexes, because a name has no numeric structure to pick a shard
//! with, and a dictionary is small enough that loading it whole costs less
//! than the machinery of loading it lazily. A folder that does not exist
//! loads as the empty registry, which is the laziness contract of every
//! handle; a shard that exists but does not parse is a typed error naming
//! its URL.
//!
//! # The process default
//!
//! [`FixRegistry::global`] is the registry every caller gets when it does not
//! name one, resolved once on the first call and never before. The order is
//! fixed: a registry installed by [`FixRegistry::install_global`], then the
//! folder `YGGDRYL_FIX_REGISTRY` names, then `~/.config/fix`, then the empty
//! registry - and only the third step treats absence as empty, because a
//! machine with no dictionary installed is an ordinary first-run state.
//! Every other failure is loud.
//!
//! ```
//! use yggdryl::{DataType, FixRegistry};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let mut symbol = DataType::Utf8.required_field("Symbol");
//! symbol.as_fix_mut().set_tag(55)?;
//! symbol.as_fix_mut().set_aliases(["Ticker"])?;
//!
//! let registry = FixRegistry::from_fields([symbol])?;
//! assert_eq!(registry.field_by_tag(55)?.name(), "Symbol");
//! assert_eq!(registry.field_by_name("ticker")?.name(), "Symbol");
//! assert_eq!(registry.field("SYMBOL")?.as_fix().tag()?, Some(55));
//! # Ok(())
//! # }
//! ```

use std::fmt;

mod field;
mod global;
mod msg;
mod registry;
mod store;
#[cfg(test)]
mod tests;

pub use field::FixAliases;
pub use msg::FixMsg;
pub use registry::{FixFieldIter, FixRegistry};

/// Either of the two ways a caller names one FIX field.
///
/// [`From`] carries every spelling a caller reaches for, exactly as
/// [`FieldKey`](crate::field::FieldKey) does, so `registry.field(35)` and
/// `registry.field("MsgType")` are one call rather than two.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FixKey<'a> {
    /// A canonical or alternate tag.
    Tag(i32),
    /// A canonical name or an alias, matched with ASCII case folded, or a
    /// dotted path whose first segment is one.
    Name(&'a str),
}

impl From<i32> for FixKey<'_> {
    fn from(tag: i32) -> Self {
        Self::Tag(tag)
    }
}

impl<'a> From<&'a str> for FixKey<'a> {
    fn from(name: &'a str) -> Self {
        Self::Name(name)
    }
}

impl<'a> From<&'a String> for FixKey<'a> {
    fn from(name: &'a String) -> Self {
        Self::Name(name.as_str())
    }
}

impl fmt::Display for FixKey<'_> {
    /// Renders the key the way an absence names it: `tag 35`, `name "MsgType"`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tag(tag) => write!(formatter, "tag {tag}"),
            Self::Name(name) => write!(formatter, "name {name:?}"),
        }
    }
}
