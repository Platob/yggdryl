//! [`NodePath`] — a parsed path addressing a node inside a nested column / value tree.
//!
//! A `NodePath` is a sequence of [`PathSegment`]s — a **name** (a struct field, the list item, or a
//! map's `key` / `value` child) or a numeric **index** (a positional child). It is a full value
//! type: it parses from a compact textual form, renders back to a canonical string, compares and
//! hashes by content, and round-trips through a byte codec. It is the parsed form the `get_by_path`
//! resolvers on [`AnySerie`](crate::io::AnySerie) / [`AnyField`](crate::io::AnyField) /
//! [`AnyScalar`](crate::io::AnyScalar) walk.
//!
//! DESIGN: the grammar is deliberately small and modeled on [`Uri::parse`](crate::io::Uri) — one
//! centralized tokenizer over a single **breaking-char set** ([`BREAKING_CHARS`]), guided errors that
//! name the offending position and the fix, and a canonical [`Display`] that always re-parses to the
//! same segments (so `serialize_bytes` is the canonical string and equality agrees with it).

use core::fmt;

/// One step of a [`NodePath`]: a named child or a positional (indexed) child.
///
/// ```
/// use yggdryl_core::io::PathSegment;
///
/// assert_eq!(PathSegment::name("a").as_name(), Some("a"));
/// assert_eq!(PathSegment::index(3).as_index(), Some(3));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PathSegment {
    /// A named child — a struct field name, the list item name, or a map's `key` / `value`.
    Name(String),
    /// A positional child — the zero-based index of a child.
    Index(usize),
}

impl PathSegment {
    /// A [`Name`](PathSegment::Name) segment from any string-like value.
    pub fn name(name: impl Into<String>) -> Self {
        Self::Name(name.into())
    }

    /// An [`Index`](PathSegment::Index) segment.
    pub fn index(index: usize) -> Self {
        Self::Index(index)
    }

    /// The name, if this is a [`Name`](PathSegment::Name) segment.
    pub fn as_name(&self) -> Option<&str> {
        match self {
            Self::Name(name) => Some(name),
            Self::Index(_) => None,
        }
    }

    /// The index, if this is an [`Index`](PathSegment::Index) segment.
    pub fn as_index(&self) -> Option<usize> {
        match self {
            Self::Index(index) => Some(*index),
            Self::Name(_) => None,
        }
    }
}

impl fmt::Display for PathSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name(name) => f.write_str(&render_name(name)),
            Self::Index(index) => write!(f, "[{index}]"),
        }
    }
}

/// A **path into a nested tree** — an ordered list of [`PathSegment`]s. A full value type:
/// `Debug`/`Clone`/`PartialEq`/`Eq`/`Hash`, a canonical [`Display`], a byte codec, and the pure
/// `Vec` combinators ([`parent`](NodePath::parent) / [`child`](NodePath::child) /
/// [`push`](NodePath::push)) — it carries **no** reference to any column, so it is a plain owned
/// value with no lifetime.
///
/// ```
/// use yggdryl_core::io::{NodePath, PathSegment};
///
/// let path = NodePath::parse("a[0].b").unwrap();
/// assert_eq!(path.len(), 3);
/// assert_eq!(path.segments()[0], PathSegment::Name("a".into()));
/// assert_eq!(path.segments()[1], PathSegment::Index(0));
/// assert_eq!(path.segments()[2], PathSegment::Name("b".into()));
/// // The canonical render round-trips through `parse`.
/// assert_eq!(path.to_string(), "a[0].b");
/// assert_eq!(NodePath::parse(&path.to_string()).unwrap(), path);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct NodePath {
    // Value identity is these segments (`#[derive]`). `serialize_bytes` is the canonical `Display`
    // string, which is injective over segment-space (every distinct segment vector renders to a
    // distinct string that re-parses to it), so "equal iff canonical bytes equal" holds and equality
    // and the byte codec never disagree.
    segments: Vec<PathSegment>,
}

impl NodePath {
    /// The empty path — zero segments, addressing the root node.
    pub fn new() -> Self {
        Self {
            segments: Vec::new(),
        }
    }

    /// A path from an explicit segment list.
    pub fn from_segments(segments: Vec<PathSegment>) -> Self {
        Self { segments }
    }

    /// Parses `s` into a `NodePath`. **One** tokenizer over the [`BREAKING_CHARS`] set.
    ///
    /// # Grammar
    /// Segments are joined by a separator (`.` or `-`). A segment is one of:
    /// - a **bareword** run of characters *not* in the breaking set → [`Name`](PathSegment::Name);
    /// - a **backtick-quoted** ``` `…` ``` where every breaking char inside is literal and a doubled
    ///   backtick (` `` `) escapes one → [`Name`](PathSegment::Name) (addresses a field literally
    ///   named e.g. `` a.b-c ``);
    /// - a **bracketed** accessor opened by any of `[` `(` `{`, closed by its match `]` `)` `}`,
    ///   whose body is an integer → [`Index`](PathSegment::Index).
    ///
    /// A bracket accessor may follow a name with no separator (`a[0]`); the empty string parses to
    /// the empty (root) path.
    ///
    /// # Errors
    /// A guided [`PathError`] naming the position and the fix for an empty segment (`a..b`), a
    /// trailing/leading separator, two adjacent segments with no separator, an unmatched or
    /// mismatched bracket, an unterminated quote, or a non-integer index body.
    ///
    /// ```
    /// use yggdryl_core::io::{NodePath, PathSegment};
    ///
    /// // Dotted, hyphenated, and bracket-indexed all tokenize the same way.
    /// assert_eq!(NodePath::parse("a.b").unwrap(), NodePath::parse("a-b").unwrap());
    /// // A name containing a breaking char is backtick-quoted; a doubled backtick is one literal.
    /// let quoted = NodePath::parse("`a.b-c`.`x``y`").unwrap();
    /// assert_eq!(quoted.segments()[0], PathSegment::Name("a.b-c".into()));
    /// assert_eq!(quoted.segments()[1], PathSegment::Name("x`y".into()));
    /// // The three bracket styles are equivalent index accessors.
    /// assert_eq!(NodePath::parse("[7]").unwrap(), NodePath::parse("(7)").unwrap());
    /// ```
    pub fn parse(s: &str) -> Result<NodePath, PathError> {
        let chars: Vec<char> = s.chars().collect();
        let n = chars.len();
        let mut segments = Vec::new();
        let mut i = 0;
        // Whether the previous token completed a segment (so the next token must be a separator, or a
        // bracket accessor that attaches with no separator, or the end).
        let mut after_segment = false;
        while i < n {
            let c = chars[i];
            if is_separator(c) {
                if !after_segment {
                    return Err(PathError::EmptySegment { position: i });
                }
                after_segment = false;
                i += 1;
            } else if matching_close(c).is_some() {
                // An opener (`[` `(` `{`) — a bracket accessor always begins a new segment, and may
                // attach directly to a preceding segment (`a[0]`).
                let (index, next) = parse_index(&chars, i)?;
                segments.push(PathSegment::Index(index));
                i = next;
                after_segment = true;
            } else if c == '`' {
                if after_segment {
                    return Err(PathError::MissingSeparator { position: i });
                }
                let (name, next) = parse_quoted(&chars, i)?;
                segments.push(PathSegment::Name(name));
                i = next;
                after_segment = true;
            } else if matches!(c, ')' | ']' | '}') {
                return Err(PathError::UnmatchedClose {
                    position: i,
                    close: c,
                });
            } else {
                if after_segment {
                    return Err(PathError::MissingSeparator { position: i });
                }
                let (name, next) = parse_bareword(&chars, i);
                segments.push(PathSegment::Name(name));
                i = next;
                after_segment = true;
            }
        }
        // A path that ended on a separator (with at least one segment) has a dangling empty segment.
        if !after_segment && !segments.is_empty() {
            return Err(PathError::TrailingSeparator { position: n });
        }
        Ok(NodePath { segments })
    }

    /// The segments, in order.
    pub fn segments(&self) -> &[PathSegment] {
        &self.segments
    }

    /// The number of segments.
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Whether the path is empty (the root).
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// The parent path — a clone with the last segment dropped, or `None` if already empty. A pure
    /// `Vec` operation: it holds **no** graph reference, so it never needs a column to compute.
    ///
    /// ```
    /// use yggdryl_core::io::NodePath;
    ///
    /// let path = NodePath::parse("a[0].b").unwrap();
    /// assert_eq!(path.parent().unwrap().to_string(), "a[0]");
    /// assert!(NodePath::new().parent().is_none());
    /// ```
    pub fn parent(&self) -> Option<NodePath> {
        if self.segments.is_empty() {
            return None;
        }
        Some(NodePath {
            segments: self.segments[..self.segments.len() - 1].to_vec(),
        })
    }

    /// This path with `segment` appended, consuming and returning it — the chainable builder.
    ///
    /// ```
    /// use yggdryl_core::io::{NodePath, PathSegment};
    ///
    /// let path = NodePath::new().child(PathSegment::name("a")).child(PathSegment::index(0));
    /// assert_eq!(path.to_string(), "a[0]");
    /// ```
    pub fn child(mut self, segment: PathSegment) -> NodePath {
        self.segments.push(segment);
        self
    }

    /// Appends `segment` in place.
    pub fn push(&mut self, segment: PathSegment) {
        self.segments.push(segment);
    }

    /// The canonical string as UTF-8 bytes — equal to [`Display`](NodePath) and the exact inverse of
    /// [`deserialize_bytes`](NodePath::deserialize_bytes).
    ///
    /// ```
    /// use yggdryl_core::io::NodePath;
    ///
    /// let path = NodePath::parse("a[0].b").unwrap();
    /// assert_eq!(path.serialize_bytes(), b"a[0].b");
    /// assert_eq!(NodePath::deserialize_bytes(&path.serialize_bytes()).unwrap(), path);
    /// ```
    pub fn serialize_bytes(&self) -> Vec<u8> {
        self.to_string().into_bytes()
    }

    /// Decodes a `NodePath` from the UTF-8 bytes produced by
    /// [`serialize_bytes`](NodePath::serialize_bytes) — the exact inverse.
    ///
    /// # Errors
    /// [`PathError::NonUtf8`] if the bytes are not UTF-8, or any [`parse`](NodePath::parse) error.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<NodePath, PathError> {
        let text =
            core::str::from_utf8(bytes).map_err(|_| PathError::NonUtf8 { len: bytes.len() })?;
        NodePath::parse(text)
    }
}

impl fmt::Display for NodePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (position, segment) in self.segments.iter().enumerate() {
            match segment {
                PathSegment::Name(name) => {
                    // A name after the first segment is introduced by the canonical `.` separator; an
                    // index attaches with no separator.
                    if position > 0 {
                        f.write_str(".")?;
                    }
                    f.write_str(&render_name(name))?;
                }
                PathSegment::Index(index) => write!(f, "[{index}]")?,
            }
        }
        Ok(())
    }
}

// -------------------------------------------------------------------------------------
// Tokenizer helpers — one breaking-char set, one function per segment shape.
// -------------------------------------------------------------------------------------

/// The **breaking-char set**: the characters that cannot appear literally in a bareword segment — the
/// two separators (`.` / `-`), the three bracket pairs, and the backtick quote. A name containing any
/// of them must be backtick-quoted. Defined **once** here so the tokenizer and the renderer agree.
pub const BREAKING_CHARS: [char; 9] = ['.', '-', '(', ')', '[', ']', '{', '}', '`'];

/// Whether `c` is in the [`BREAKING_CHARS`] set.
fn is_breaking(c: char) -> bool {
    BREAKING_CHARS.contains(&c)
}

/// Whether `c` is a segment separator (`.` or `-`).
fn is_separator(c: char) -> bool {
    c == '.' || c == '-'
}

/// The closing bracket matching an opener, or `None` if `c` is not an opener.
fn matching_close(c: char) -> Option<char> {
    match c {
        '[' => Some(']'),
        '(' => Some(')'),
        '{' => Some('}'),
        _ => None,
    }
}

/// Renders a name to its canonical token: bare if it is non-empty and holds no breaking char,
/// otherwise backtick-quoted with every internal backtick doubled — so [`Display`](NodePath) always
/// re-parses to the same name.
fn render_name(name: &str) -> String {
    if name.is_empty() || name.chars().any(is_breaking) {
        let mut out = String::with_capacity(name.len() + 2);
        out.push('`');
        for c in name.chars() {
            if c == '`' {
                out.push('`'); // double an internal backtick
            }
            out.push(c);
        }
        out.push('`');
        out
    } else {
        name.to_string()
    }
}

/// Parses a bracketed integer accessor starting at the opener `chars[start]`, returning the index and
/// the position just past the closing bracket.
fn parse_index(chars: &[char], start: usize) -> Result<(usize, usize), PathError> {
    let open = chars[start];
    let close = matching_close(open).expect("caller only enters with an opener");
    let mut body = String::new();
    let mut j = start + 1;
    while j < chars.len() {
        let c = chars[j];
        if c == close {
            let index = body
                .parse::<usize>()
                .map_err(|_| PathError::NonIntegerIndex {
                    position: start,
                    body: body.clone(),
                })?;
            return Ok((index, j + 1));
        }
        body.push(c);
        j += 1;
    }
    Err(PathError::UnmatchedBracket {
        position: start,
        open,
    })
}

/// Parses a backtick-quoted name starting at `chars[start]` (the opening backtick), returning the
/// decoded name and the position just past the closing backtick. A doubled backtick is one literal.
fn parse_quoted(chars: &[char], start: usize) -> Result<(String, usize), PathError> {
    let n = chars.len();
    let mut out = String::new();
    let mut j = start + 1;
    while j < n {
        let c = chars[j];
        if c == '`' {
            if j + 1 < n && chars[j + 1] == '`' {
                out.push('`'); // escaped backtick
                j += 2;
            } else {
                return Ok((out, j + 1)); // closing backtick
            }
        } else {
            out.push(c);
            j += 1;
        }
    }
    Err(PathError::UnterminatedQuote { position: start })
}

/// Parses a bareword (a run of non-breaking chars) starting at `chars[start]`, returning the name and
/// the position of the first breaking char (or the end). The caller only enters here on a non-breaking
/// char, so the name is non-empty.
fn parse_bareword(chars: &[char], start: usize) -> (String, usize) {
    let mut out = String::new();
    let mut j = start;
    while j < chars.len() && !is_breaking(chars[j]) {
        out.push(chars[j]);
        j += 1;
    }
    (out, j)
}

// -------------------------------------------------------------------------------------
// PathError — the guided parse / resolution failures.
// -------------------------------------------------------------------------------------

/// An error raised while parsing a [`NodePath`] or resolving one against a nested tree.
///
/// Every variant names the offending position (a **character** index) or segment and how to fix it;
/// in the bindings it surfaces as a Python `ValueError` / a thrown `Error`.
///
/// ```
/// use yggdryl_core::io::{NodePath, PathError};
///
/// assert!(matches!(NodePath::parse("a..b").unwrap_err(), PathError::EmptySegment { .. }));
/// assert!(matches!(NodePath::parse("a[x]").unwrap_err(), PathError::NonIntegerIndex { .. }));
/// assert!(matches!(NodePath::parse("`a").unwrap_err(), PathError::UnterminatedQuote { .. }));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PathError {
    /// A segment was empty — two separators in a row, or a leading separator. Remove the extra
    /// separator, or backtick-quote a field literally named with a separator.
    EmptySegment {
        /// The character position of the offending separator.
        position: usize,
    },
    /// The path ended on a separator, leaving a dangling empty segment. Drop the trailing `.`/`-`.
    TrailingSeparator {
        /// The character position just past the end of the path.
        position: usize,
    },
    /// Two segments sat next to each other with no separator (`` `a`b `` / `a[0]b`). Insert a `.` (or
    /// `-`) between them; only a bracket accessor may attach to a name with no separator.
    MissingSeparator {
        /// The character position of the second segment.
        position: usize,
    },
    /// A bracket accessor was opened but never closed by its matching bracket. Close it with the
    /// matching `]` / `)` / `}`.
    UnmatchedBracket {
        /// The character position of the unclosed opener.
        position: usize,
        /// The opener that was not closed.
        open: char,
    },
    /// A closing bracket appeared with no matching opener. Remove it, or add the opener before it.
    UnmatchedClose {
        /// The character position of the stray closer.
        position: usize,
        /// The stray closing bracket.
        close: char,
    },
    /// A bracket accessor's body was not a non-negative integer. Put a decimal index inside the
    /// brackets (`[0]`), or backtick-quote a field name instead of bracketing it.
    NonIntegerIndex {
        /// The character position of the opener.
        position: usize,
        /// The offending (non-integer) body text.
        body: String,
    },
    /// A backtick-quoted name was never closed. Add the closing backtick (double an internal one).
    UnterminatedQuote {
        /// The character position of the opening backtick.
        position: usize,
    },
    /// The bytes handed to [`deserialize_bytes`](NodePath::deserialize_bytes) are not valid UTF-8, so
    /// they cannot be a path string. Pass the UTF-8 bytes of a path (as `serialize_bytes` produces).
    NonUtf8 {
        /// The number of bytes supplied.
        len: usize,
    },
    /// A [`Name`](PathSegment::Name) segment named a child the node does not have. Address a child
    /// that exists (the node exposes `num_children` children), or fix the name.
    NoChildNamed {
        /// The zero-based depth (segment position) that failed to resolve.
        depth: usize,
        /// The child name that was not found.
        name: String,
        /// How many children the node at that depth has.
        num_children: usize,
    },
    /// An [`Index`](PathSegment::Index) segment addressed a child position outside the node's
    /// children. Use an index in `0..num_children`.
    ChildIndexOutOfRange {
        /// The zero-based depth (segment position) that failed to resolve.
        depth: usize,
        /// The out-of-range child index.
        index: usize,
        /// How many children the node at that depth has.
        num_children: usize,
    },
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySegment { position } => write!(
                f,
                "empty path segment at position {position}: a `.`/`-` separator has nothing before \
                 it; remove the extra separator (or backtick-quote a name that contains one)"
            ),
            Self::TrailingSeparator { position } => write!(
                f,
                "trailing path separator at position {position}: the path ends on a `.`/`-` with no \
                 segment after it; drop the trailing separator"
            ),
            Self::MissingSeparator { position } => write!(
                f,
                "missing separator at position {position}: two segments are adjacent with no `.`/`-` \
                 between them; insert one (only a `[i]` accessor may attach without a separator)"
            ),
            Self::UnmatchedBracket { position, open } => write!(
                f,
                "unmatched `{open}` bracket opened at position {position}: it is never closed; add \
                 the matching closing bracket"
            ),
            Self::UnmatchedClose { position, close } => write!(
                f,
                "stray `{close}` at position {position}: a closing bracket with no matching opener; \
                 remove it or add the opener before it"
            ),
            Self::NonIntegerIndex { position, body } => write!(
                f,
                "non-integer index {body:?} in the bracket accessor at position {position}: put a \
                 non-negative decimal index inside the brackets (e.g. `[0]`), or backtick-quote a \
                 field name instead of bracketing it"
            ),
            Self::UnterminatedQuote { position } => write!(
                f,
                "unterminated backtick quote opened at position {position}: add the closing backtick \
                 (double an internal backtick to make it literal)"
            ),
            Self::NonUtf8 { len } => write!(
                f,
                "cannot decode a path from {len} bytes: the bytes are not valid UTF-8; pass the \
                 UTF-8 bytes of a path (as produced by `serialize_bytes`)"
            ),
            Self::NoChildNamed {
                depth,
                name,
                num_children,
            } => write!(
                f,
                "no child named {name:?} at path depth {depth}: the node has {num_children} \
                 child(ren); address a child that exists or fix the name"
            ),
            Self::ChildIndexOutOfRange {
                depth,
                index,
                num_children,
            } => write!(
                f,
                "child index {index} is out of range at path depth {depth}: the node has \
                 {num_children} child(ren); use an index in 0..{num_children}"
            ),
        }
    }
}

impl std::error::Error for PathError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(path: &NodePath) -> Vec<PathSegment> {
        path.segments().to_vec()
    }

    #[test]
    fn bareword_dotted_and_hyphen_forms_tokenize_alike() {
        let dotted = NodePath::parse("a.b.c").unwrap();
        let hyphen = NodePath::parse("a-b-c").unwrap();
        assert_eq!(dotted, hyphen);
        assert_eq!(
            names(&dotted),
            vec![
                PathSegment::name("a"),
                PathSegment::name("b"),
                PathSegment::name("c")
            ]
        );
    }

    #[test]
    fn bracket_index_forms_are_equivalent_and_chain() {
        assert_eq!(
            NodePath::parse("[3]").unwrap(),
            NodePath::parse("(3)").unwrap()
        );
        assert_eq!(
            NodePath::parse("[3]").unwrap(),
            NodePath::parse("{3}").unwrap()
        );
        let chained = NodePath::parse("a[0][1].b").unwrap();
        assert_eq!(
            names(&chained),
            vec![
                PathSegment::name("a"),
                PathSegment::index(0),
                PathSegment::index(1),
                PathSegment::name("b"),
            ]
        );
    }

    #[test]
    fn backtick_quoting_makes_breaking_chars_literal() {
        let path = NodePath::parse("`a.b-c`.`x[0]`").unwrap();
        assert_eq!(
            names(&path),
            vec![PathSegment::name("a.b-c"), PathSegment::name("x[0]")]
        );
    }

    #[test]
    fn doubled_backtick_escapes_a_literal_backtick() {
        let path = NodePath::parse("`x``y`").unwrap();
        assert_eq!(names(&path), vec![PathSegment::name("x`y")]);
        // A lone backtick name round-trips through Display -> parse.
        let tick = NodePath::from_segments(vec![PathSegment::name("`")]);
        assert_eq!(NodePath::parse(&tick.to_string()).unwrap(), tick);
    }

    #[test]
    fn empty_path_is_the_root() {
        let root = NodePath::parse("").unwrap();
        assert!(root.is_empty());
        assert_eq!(root.len(), 0);
        assert_eq!(root.to_string(), "");
        assert_eq!(NodePath::parse(&root.to_string()).unwrap(), root);
    }

    #[test]
    fn display_round_trips_through_parse_for_every_shape() {
        for input in [
            "a",
            "a.b.c",
            "a[0].b",
            "[0][1][2]",
            "`a.b`.c[9]",
            "`x``y`.z",
            "outer[3].`in ner`.leaf",
        ] {
            let path = NodePath::parse(input).unwrap();
            let rendered = path.to_string();
            assert_eq!(
                NodePath::parse(&rendered).unwrap(),
                path,
                "Display of {input:?} = {rendered:?} did not re-parse to the same path"
            );
        }
    }

    #[test]
    fn value_type_round_trips_and_hashes() {
        use std::collections::HashSet;

        let a = NodePath::parse("a[0].b").unwrap();
        let b = NodePath::parse("a-0-b"); // NOT equal: `-0-` is names, not an index
        assert!(b.is_ok());
        assert_ne!(a, b.unwrap());

        // serialize/deserialize is the exact inverse.
        assert_eq!(
            NodePath::deserialize_bytes(&a.serialize_bytes()).unwrap(),
            a
        );

        // Equal paths (from different textual forms) hash equal.
        let dotted = NodePath::parse("a.b").unwrap();
        let hyphen = NodePath::parse("a-b").unwrap();
        let set: HashSet<NodePath> = [dotted.clone(), hyphen.clone()].into_iter().collect();
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn parent_child_push_are_pure_vec_ops() {
        let path = NodePath::parse("a[0].b").unwrap();
        assert_eq!(path.parent().unwrap().to_string(), "a[0]");
        assert_eq!(path.parent().unwrap().parent().unwrap().to_string(), "a");
        assert!(NodePath::parse("a").unwrap().parent().unwrap().is_empty());
        assert!(NodePath::new().parent().is_none());

        let built = NodePath::new()
            .child(PathSegment::name("a"))
            .child(PathSegment::index(0))
            .child(PathSegment::name("b"));
        assert_eq!(built, path);

        let mut p = NodePath::new();
        p.push(PathSegment::name("a"));
        p.push(PathSegment::index(0));
        assert_eq!(p.to_string(), "a[0]");
    }

    #[test]
    fn parse_errors_name_the_offending_position() {
        assert!(matches!(
            NodePath::parse("a..b").unwrap_err(),
            PathError::EmptySegment { position: 2 }
        ));
        assert!(matches!(
            NodePath::parse(".a").unwrap_err(),
            PathError::EmptySegment { position: 0 }
        ));
        assert!(matches!(
            NodePath::parse("a.").unwrap_err(),
            PathError::TrailingSeparator { .. }
        ));
        assert!(matches!(
            NodePath::parse("a[0]b").unwrap_err(),
            PathError::MissingSeparator { .. }
        ));
        assert!(matches!(
            NodePath::parse("`a`b").unwrap_err(),
            PathError::MissingSeparator { .. }
        ));
        assert!(matches!(
            NodePath::parse("a[1").unwrap_err(),
            PathError::UnmatchedBracket { open: '[', .. }
        ));
        assert!(matches!(
            NodePath::parse("a]").unwrap_err(),
            PathError::UnmatchedClose { close: ']', .. }
        ));
        assert!(matches!(
            NodePath::parse("a[x]").unwrap_err(),
            PathError::NonIntegerIndex { .. }
        ));
        assert!(matches!(
            NodePath::parse("a[]").unwrap_err(),
            PathError::NonIntegerIndex { .. }
        ));
        assert!(matches!(
            NodePath::parse("`abc").unwrap_err(),
            PathError::UnterminatedQuote { position: 0 }
        ));
    }

    #[test]
    fn deserialize_rejects_non_utf8() {
        assert!(matches!(
            NodePath::deserialize_bytes(&[0xff, 0xfe]).unwrap_err(),
            PathError::NonUtf8 { len: 2 }
        ));
    }
}
