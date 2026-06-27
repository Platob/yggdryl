//! Parsing of a child **node path** like `a.b.c` for [`NestedSerie::child_path`](crate::NestedSerie::child_path).
//!
//! Segments are split on top-level `.`; a segment may be **wrapped** in `[...]`,
//! `"..."`, `'...'` or `` `...` `` to match a name **exactly** (and to contain dots),
//! a bare numeric segment is a child **index**, and any other bare segment matches a
//! child name case-sensitively then case-insensitively.

/// One resolved path segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Segment {
    /// A child index (`0`, `1`, …).
    Index(usize),
    /// An exact, case-sensitive name match (from a wrapped segment).
    Exact(String),
    /// A name match: case-sensitive first, then case-insensitive (a bare segment).
    Loose(String),
}

/// Splits `path` into [`Segment`]s on top-level `.`, honouring the wrapper characters
/// (`[]` / `"` / `'` / `` ` ``) so a wrapped segment can contain dots.
pub(crate) fn parse_path(path: &str) -> Vec<Segment> {
    let mut raw: Vec<String> = Vec::new();
    let mut buf = String::new();
    let mut close: Option<char> = None; // the closing char we are waiting for
    for c in path.chars() {
        match close {
            Some(end) => {
                buf.push(c);
                if c == end {
                    close = None;
                }
            }
            None => match c {
                '"' | '\'' | '`' => {
                    close = Some(c);
                    buf.push(c);
                }
                '[' => {
                    close = Some(']');
                    buf.push(c);
                }
                '.' => raw.push(std::mem::take(&mut buf)),
                _ => buf.push(c),
            },
        }
    }
    raw.push(buf);
    raw.into_iter().map(classify).collect()
}

/// Classifies one raw segment into a [`Segment`].
fn classify(raw: String) -> Segment {
    let trimmed = raw.trim();
    if let Some(name) = unwrap(trimmed) {
        return Segment::Exact(name);
    }
    if let Ok(index) = trimmed.parse::<usize>() {
        return Segment::Index(index);
    }
    Segment::Loose(trimmed.to_string())
}

/// Strips one matching wrapper pair (`[]` / `"` / `'` / `` ` ``) from `s`, returning
/// the inner name, or `None` if `s` is not wrapped.
fn unwrap(s: &str) -> Option<String> {
    let mut chars = s.chars();
    let first = chars.next()?;
    let last = chars.next_back()?; // requires at least two chars
    let wrapped = matches!(
        (first, last),
        ('"', '"') | ('\'', '\'') | ('`', '`') | ('[', ']')
    );
    // `first` / `last` are single-byte ASCII wrappers, so byte slicing is safe.
    wrapped.then(|| s[first.len_utf8()..s.len() - last.len_utf8()].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_and_classifies() {
        assert_eq!(
            parse_path("a.b.c"),
            vec![
                Segment::Loose("a".into()),
                Segment::Loose("b".into()),
                Segment::Loose("c".into())
            ]
        );
        assert_eq!(
            parse_path("items.0"),
            vec![Segment::Loose("items".into()), Segment::Index(0)]
        );
        // wrappers protect dots and force an exact match
        assert_eq!(
            parse_path(r#""a.b".c"#),
            vec![Segment::Exact("a.b".into()), Segment::Loose("c".into())]
        );
        assert_eq!(parse_path("[a.b]"), vec![Segment::Exact("a.b".into())]);
        assert_eq!(parse_path("`x`"), vec![Segment::Exact("x".into())]);
    }
}
