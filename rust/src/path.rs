//! One shared, allocation-free value path used by every recursive walk.
//!
//! Schema, record, Arrow, and compatibility walks all report where a failure
//! occurred. Before this module each owned a private spelling, so bindings
//! could not parse paths uniformly and a numeric index in one walk meant a
//! field name in another. [`Path`] is a borrowed cons-list: a recursive walker
//! carries it down without allocating per node and renders it only when an
//! error is actually produced.

use std::fmt;

use crate::text::elide_to;

/// Byte budget for one caller-supplied name inside a rendered path.
///
/// A path may contain many names, so each is bounded more tightly than a
/// standalone interpolated value.
const PATH_NAME_LIMIT: usize = 32;

/// One step from a parent value to a child value.
#[derive(Clone, Copy, Debug)]
pub enum Segment<'a> {
    /// A named struct child.
    Field(&'a str),
    /// A positional element of a sequence.
    Index(usize),
    /// The element field of a serie layout.
    Item,
    /// The entries struct of a map layout.
    MapEntries,
    /// One union alternative, by Arrow type id.
    UnionType(i8),
    /// The value type behind a dictionary encoding.
    DictionaryValue,
    /// The run-ends child of a run-end encoding.
    RunEnds,
    /// The values child of a run-end encoding.
    RunEndValues,
}

/// A borrowed, allocation-free path accumulated during a recursive walk.
///
/// Renders as `$`-rooted dot/bracket text, such as
/// `$.users[3].address["zip code"]`.
#[derive(Clone, Copy, Debug)]
pub enum Path<'a> {
    /// The value the walk started from.
    Root,
    /// A child reached from `parent` through `segment`.
    Child {
        /// The value this step descends from.
        parent: &'a Path<'a>,
        /// The step taken.
        segment: Segment<'a>,
    },
}

impl<'a> Path<'a> {
    /// The path of the value a walk starts from.
    pub const fn root() -> Self {
        Self::Root
    }

    /// Borrow this path as the parent of one further step.
    pub const fn child(&'a self, segment: Segment<'a>) -> Self {
        Self::Child {
            parent: self,
            segment,
        }
    }

    /// Borrow this path as the parent of a named struct child.
    pub const fn field(&'a self, name: &'a str) -> Self {
        self.child(Segment::Field(name))
    }

    /// Render the canonical `$`-rooted text.
    pub fn render(&self) -> String {
        self.render_from("$")
    }

    /// Render the canonical text under an explicit root token.
    pub fn render_from(&self, root: &str) -> String {
        let mut rendered = String::from(root);
        let result = self.write_into(&mut rendered);
        debug_assert!(result.is_ok(), "writing into a String is infallible");
        rendered
    }

    /// Stream the steps below the root, outermost first, into `target`.
    ///
    /// The walk climbs to the root before it writes, so a shallow path
    /// renders with no intermediate vector, and a writer that keeps short
    /// text inline - a `SmolStr` builder - takes it with no heap at all.
    fn write_into<W: fmt::Write>(&self, target: &mut W) -> fmt::Result {
        match self {
            Self::Root => Ok(()),
            Self::Child { parent, segment } => {
                parent.write_into(target)?;
                write_segment(target, *segment)
            }
        }
    }
}

impl fmt::Display for Path<'_> {
    /// The text [`Self::render`] answers, streamed rather than built.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("$")?;
        self.write_into(formatter)
    }
}

/// Append one rendered step to an owned path string.
pub(crate) fn push_segment(path: &mut String, segment: Segment<'_>) {
    let result = write_segment(path, segment);
    debug_assert!(result.is_ok(), "writing into a String is infallible");
}

/// Append a struct child name, bracketing and quoting it when it is not a
/// bare identifier.
pub(crate) fn push_field_name(path: &mut String, name: &str) {
    let result = write_field_name(path, name);
    debug_assert!(result.is_ok(), "writing into a String is infallible");
}

/// Write one rendered step.
fn write_segment<W: fmt::Write>(path: &mut W, segment: Segment<'_>) -> fmt::Result {
    match segment {
        Segment::Field(name) => write_field_name(path, name),
        Segment::Index(index) => write!(path, "[{index}]"),
        Segment::Item => path.write_str("[]"),
        Segment::MapEntries => path.write_str(".entries"),
        Segment::UnionType(type_id) => write!(path, "<union:{type_id}>"),
        Segment::DictionaryValue => path.write_str(".dictionary_value"),
        Segment::RunEnds => path.write_str(".run_ends"),
        Segment::RunEndValues => path.write_str(".run_end_values"),
    }
}

/// Write a struct child name, bracketing and quoting it when it is not a
/// bare identifier.
fn write_field_name<W: fmt::Write>(path: &mut W, name: &str) -> fmt::Result {
    let mut characters = name.chars();
    let is_identifier = characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric());
    if is_identifier && name.len() <= PATH_NAME_LIMIT {
        path.write_char('.')?;
        path.write_str(name)
    } else {
        write!(path, "[{:?}]", elide_to(name, PATH_NAME_LIMIT))
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/path.rs` pins and a caller cannot reach.
    //!
    //! A value path is the spelling every recursive walk names its place
    //! with, and a caller only ever sees it inside a rendered failure. The
    //! crate root declares `path` privately, so these reach nobody without
    //! the feature.
    pub use super::{Path, Segment};
}
