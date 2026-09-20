//! One shared, allocation-free value path used by every recursive walk.
//!
//! Schema, record, Arrow, and compatibility walks all report where a failure
//! occurred. Before this module each owned a private spelling, so bindings
//! could not parse paths uniformly and a numeric index in one walk meant a
//! field name in another. [`Path`] is a borrowed cons-list: a recursive walker
//! carries it down without allocating per node and renders it only when an
//! error is actually produced.

use std::fmt;

use smol_str::{SmolStr, SmolStrBuilder};

use crate::text::elide_to;

/// Byte budget for one caller-supplied name inside a rendered path.
///
/// A path may contain many names, so each is bounded more tightly than a
/// standalone interpolated value.
const PATH_NAME_LIMIT: usize = 32;

/// One step from a parent value to a child value.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Segment<'a> {
    /// A named struct child.
    Field(&'a str),
    /// A positional element of a sequence.
    Index(usize),
    /// The element field of a list layout.
    Item,
    /// The key of one map entry.
    MapKey(usize),
    /// The value of one map entry.
    MapValue(usize),
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
pub(crate) enum Path<'a> {
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
    pub(crate) const fn root() -> Self {
        Self::Root
    }

    /// Borrow this path as the parent of one further step.
    pub(crate) const fn child(&'a self, segment: Segment<'a>) -> Self {
        Self::Child {
            parent: self,
            segment,
        }
    }

    /// Borrow this path as the parent of a named struct child.
    pub(crate) const fn field(&'a self, name: &'a str) -> Self {
        self.child(Segment::Field(name))
    }

    /// Render the canonical `$`-rooted text.
    pub(crate) fn render(&self) -> String {
        self.render_from("$")
    }

    /// Render the canonical `$`-rooted text as a [`SmolStr`].
    ///
    /// A compiled cast plan keeps every node's path for the one refusal that
    /// node may never raise, so the ordinary spelling - `$`, `$.id`,
    /// `$.symbol` - has to cost nothing to keep. Written straight into
    /// `SmolStrBuilder`, a path that fits inline never allocates; rendering
    /// through an owned `String` first always did, once per node per compile.
    pub(crate) fn render_compact(&self) -> SmolStr {
        let mut rendered = SmolStrBuilder::default();
        rendered.push_char('$');
        self.push_into(&mut rendered);
        rendered.finish()
    }

    /// Render the canonical text under an explicit root token.
    pub(crate) fn render_from(&self, root: &str) -> String {
        let mut rendered = String::from(root);
        self.push_into(&mut rendered);
        rendered
    }

    fn push_into<W: PathSink>(&self, target: &mut W) {
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

/// What one rendered path is written into.
///
/// Both implementations grow on demand and neither can fail, so the writers
/// below stay free of a `fmt::Result` no caller could act on. The supertrait is
/// what lets the two numeric steps reuse `write!`.
pub(crate) trait PathSink: fmt::Write {
    /// Append a rendered fragment.
    fn push_str(&mut self, value: &str);
    /// Append one rendered character.
    fn push_char(&mut self, value: char);
}

impl PathSink for String {
    fn push_str(&mut self, value: &str) {
        Self::push_str(self, value);
    }

    fn push_char(&mut self, value: char) {
        self.push(value);
    }
}

impl PathSink for SmolStrBuilder {
    fn push_str(&mut self, value: &str) {
        Self::push_str(self, value);
    }

    fn push_char(&mut self, value: char) {
        self.push(value);
    }
}

/// Append one rendered step to a path sink.
pub(crate) fn push_segment<W: PathSink>(path: &mut W, segment: Segment<'_>) {
    match segment {
        Segment::Field(name) => push_field_name(path, name),
        Segment::Index(index) => {
            path.push_char('[');
            push_usize(path, index);
            path.push_char(']');
        }
        Segment::Item => path.push_str("[]"),
        Segment::MapKey(index) => {
            path.push_char('[');
            push_usize(path, index);
            path.push_str("].key");
        }
        Segment::MapValue(index) => {
            path.push_char('[');
            push_usize(path, index);
            path.push_str("].value");
        }
        Segment::MapEntries => path.push_str(".entries"),
        Segment::UnionType(type_id) => {
            path.push_str("<union:");
            push_i8(path, type_id);
            path.push_char('>');
        }
        Segment::DictionaryValue => path.push_str(".dictionary_value"),
        Segment::RunEnds => path.push_str(".run_ends"),
        Segment::RunEndValues => path.push_str(".run_end_values"),
    }
}

/// Append a struct child name, bracketing and quoting it when it is not a
/// bare identifier.
pub(crate) fn push_field_name<W: PathSink>(path: &mut W, name: &str) {
    let mut characters = name.chars();
    let is_identifier = characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric());
    if is_identifier && name.len() <= PATH_NAME_LIMIT {
        path.push_char('.');
        path.push_str(name);
    } else {
        path.push_char('[');
        // Written straight through rather than through an owned `String` the
        // quoted spelling would only be copied out of.
        write_or_assert(path, format_args!("{:?}", elide_to(name, PATH_NAME_LIMIT)));
        path.push_char(']');
    }
}

fn push_usize<W: PathSink>(path: &mut W, value: usize) {
    write_or_assert(path, format_args!("{value}"));
}

fn push_i8<W: PathSink>(path: &mut W, value: i8) {
    write_or_assert(path, format_args!("{value}"));
}

/// Write one formatted fragment into a sink that cannot fail.
fn write_or_assert<W: PathSink>(path: &mut W, arguments: fmt::Arguments<'_>) {
    let result = path.write_fmt(arguments);
    debug_assert!(result.is_ok(), "writing into a path sink is infallible");
}

#[cfg(test)]
mod tests {
    use super::{Path, Segment};

    #[test]
    fn root_renders_as_the_root_token() {
        assert_eq!(Path::root().render(), "$");
    }

    #[test]
    fn identifiers_use_dots_and_other_names_use_quoted_brackets() {
        let root = Path::root();
        let plain = root.field("users");
        assert_eq!(plain.render(), "$.users");

        let dotted = root.field("a.b");
        assert_eq!(dotted.render(), "$[\"a.b\"]");

        let empty = root.field("");
        assert_eq!(empty.render(), "$[\"\"]");
    }

    #[test]
    fn nested_segments_render_outermost_first() {
        let root = Path::root();
        let users = root.field("users");
        let third = users.child(Segment::Index(3));
        let address = third.field("address");
        let zip = address.field("zip code");
        assert_eq!(zip.render(), "$.users[3].address[\"zip code\"]");
    }

    #[test]
    fn container_segments_have_stable_spellings() {
        let root = Path::root();
        assert_eq!(root.child(Segment::Item).render(), "$[]");
        assert_eq!(root.child(Segment::MapEntries).render(), "$.entries");
        assert_eq!(root.child(Segment::MapKey(2)).render(), "$[2].key");
        assert_eq!(root.child(Segment::MapValue(2)).render(), "$[2].value");
        assert_eq!(
            root.child(Segment::DictionaryValue).render(),
            "$.dictionary_value"
        );
        assert_eq!(root.child(Segment::RunEnds).render(), "$.run_ends");
        assert_eq!(
            root.child(Segment::RunEndValues).render(),
            "$.run_end_values"
        );
        assert_eq!(root.child(Segment::UnionType(1)).render(), "$<union:1>");
    }

    #[test]
    fn a_long_field_name_is_bounded() {
        let long = "n".repeat(512);
        let root = Path::root();
        let rendered = root.field(&long).render();
        assert!(rendered.len() < 64, "{}", rendered.len());
        assert!(rendered.contains('\u{2026}'), "{rendered}");
    }

    #[test]
    fn an_explicit_root_token_replaces_the_dollar() {
        let root = Path::root();
        let child = root.field("value");
        assert_eq!(child.render_from("record"), "record.value");
    }

    /// The compact rendering is the same text, sink or no sink.
    ///
    /// A compiled cast plan keeps `render_compact` rather than `render`, so the
    /// two spellings agreeing is what lets a refusal name the same path it
    /// always did. Every segment kind is covered, plus a name long enough to
    /// take the bounded branch and one short enough to stay inline.
    #[test]
    fn the_compact_rendering_matches_the_owned_one() {
        let root = Path::root();
        let long = "n".repeat(512);
        // Each step is bound: a `Path` borrows its parent, so a chained
        // expression would drop the parent while the child still names it.
        let users = root.field("users");
        let third = users.child(Segment::Index(3));
        let deep = third.field("zip code");
        let named = root.field(&long);
        let cases = [
            root,
            root.field("id"),
            root.field("a.b"),
            root.field(""),
            named,
            deep,
            root.child(Segment::Item),
            root.child(Segment::MapEntries),
            root.child(Segment::MapKey(2)),
            root.child(Segment::MapValue(2)),
            root.child(Segment::DictionaryValue),
            root.child(Segment::RunEnds),
            root.child(Segment::RunEndValues),
            root.child(Segment::UnionType(-1)),
        ];
        for case in cases {
            assert_eq!(case.render_compact().as_str(), case.render(), "{case}");
        }
    }
}
