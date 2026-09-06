//! One field as a given version spelled and typed it, resolved once.
//!
//! Reading a message at a version renames and retypes every field it holds
//! to what that version called it, and both answers come from walking the
//! field's [lineage](super::lineage). That walk is around a microsecond and
//! the message it serves takes a few - so a capture of ten million rows pays
//! it ten million times for an answer that changed on none of them.
//!
//! It is a pure function of a field and a version, so it is cached on the
//! [reader](super::FixReader) that spans the rows rather than recomputed
//! inside each one. A capture is one session at one version, which is why the
//! cache is one generation deep: a version it was not built for replaces it
//! rather than growing a second, so a reader whose version genuinely varies
//! per row is no worse off than it was and the common one pays a hash.

use std::collections::HashMap;
use std::sync::RwLock;

use crate::{DataType, Field, Version};

use super::FixId;

/// Projected fields, all of one version.
#[derive(Default)]
struct Generation {
    /// `None` is the undated generation, which projects nothing.
    version: Option<Version>,
    fields: HashMap<FixId, Field>,
}

/// A reader's memory of what a version calls the fields it has seen.
///
/// Shared behind one lock rather than one per field: the read is a hash and a
/// clone, the write happens once per field per run, and a capture read across
/// threads shares the work instead of repeating it per thread.
#[derive(Default)]
pub(super) struct Projections {
    held: RwLock<Generation>,
}

impl Projections {
    /// One registry field as `version` spells and types it.
    ///
    /// Fields the cache cannot key - a nested member carrying no identity of
    /// its own - are projected directly, which is correct and merely uncached.
    pub(super) fn field(&self, known: &Field, version: Option<Version>) -> Field {
        let Some(id) = known.as_fix().id().ok().flatten() else {
            return project(known, version);
        };
        if let Ok(held) = self.held.read() {
            if held.version == version {
                if let Some(field) = held.fields.get(&id) {
                    return field.clone();
                }
            }
        }
        let projected = project(known, version);
        if let Ok(mut held) = self.held.write() {
            if held.version != version {
                held.version = version;
                held.fields.clear();
            }
            held.fields.insert(id, projected.clone());
        }
        projected
    }
}

/// One registry field renamed and retyped to what `version` said.
///
/// A field the dictionary knows keeps its metadata across a retype, because
/// the lineage and the code set are what a later read of it resolves through.
/// A repeating group keeps its shape, because its lineage dates its counter.
fn project(known: &Field, version: Option<Version>) -> Field {
    let mut field = known.clone();
    let Some(at) = version else {
        return field;
    };
    let view = known.as_fix();
    if let Some(name) = view.name_at(at) {
        if name != known.name() {
            field.set_name(name);
        }
    }
    // A nested field is a repeating group, and its lineage dates the counter -
    // `NumInGroup` - rather than the group. Retyping it would collapse the List
    // the row builds its occurrences into, so a group is projected by name only.
    if super::registry::is_nested(known) {
        return field;
    }
    if let Ok(Some(dtype)) = view.dtype_at(at) {
        if dtype != *known.dtype() {
            field = Field::new(field.name(), dtype, field.is_nullable());
            let _ = field.set_metadata(known.as_metadata().iter());
        }
    }
    field
}

/// Whether a datatype is one the FIX layer reads a wire spelling for.
///
/// Kept beside the projection because both answer "what is this field, at
/// this version" and neither belongs in the generic value contract.
pub(super) const fn is_binary(dtype: &DataType) -> bool {
    matches!(dtype, DataType::Binary | DataType::LargeBinary)
}
