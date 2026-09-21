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
    /// The element field of a list layout.
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
        self.push_into(&mut rendered);
        rendered
    }

    fn push_into(&self, target: &mut String) {
        // Walk to the root first so segments render outermost-first without
        // allocating an intermediate vector for shallow paths.
        match self {
            Self::Root => {}
            Self::Child { parent, segment } => {
                parent.push_into(target);
                push_segment(target, *segment);
            }
        }
    }
}

impl fmt::Display for Path<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render())
    }
}

/// Append one rendered step to an owned path string.
pub(crate) fn push_segment(path: &mut String, segment: Segment<'_>) {
    match segment {
        Segment::Field(name) => push_field_name(path, name),
        Segment::Index(index) => {
            path.push('[');
            push_usize(path, index);
            path.push(']');
        }
        Segment::Item => path.push_str("[]"),
        Segment::MapEntries => path.push_str(".entries"),
        Segment::UnionType(type_id) => {
            path.push_str("<union:");
            push_i8(path, type_id);
            path.push('>');
        }
        Segment::DictionaryValue => path.push_str(".dictionary_value"),
        Segment::RunEnds => path.push_str(".run_ends"),
        Segment::RunEndValues => path.push_str(".run_end_values"),
    }
}

/// Append a struct child name, bracketing and quoting it when it is not a
/// bare identifier.
pub(crate) fn push_field_name(path: &mut String, name: &str) {
    let mut characters = name.chars();
    let is_identifier = characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric());
    if is_identifier && name.len() <= PATH_NAME_LIMIT {
        path.push('.');
        path.push_str(name);
    } else {
        path.push('[');
        path.push_str(&format!("{:?}", elide_to(name, PATH_NAME_LIMIT)));
        path.push(']');
    }
}

fn push_usize(path: &mut String, value: usize) {
    use fmt::Write as _;
    let result = write!(path, "{value}");
    debug_assert!(result.is_ok(), "writing into a String is infallible");
}

fn push_i8(path: &mut String, value: i8) {
    use fmt::Write as _;
    let result = write!(path, "{value}");
    debug_assert!(result.is_ok(), "writing into a String is infallible");
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
