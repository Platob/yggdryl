//! Depth-aware splitting helpers shared by the nested [`DataType`](crate::DataType)
//! and [`Field`](crate::Field) string grammars. A dependency-free foundation (the
//! analogue of the core crate's `encoding` module): it understands only the
//! bracket structure `name(params)<body>`, never the types themselves.

/// A type string split into its leading `name`, an optional parenthesised
/// `params` group, and an optional angle-bracketed `body` group — the shape every
/// `DataType` form takes, e.g. `fixed_size_list(3)<item: int64>` →
/// `{ name: "fixed_size_list", params: Some("3"), body: Some("item: int64") }`.
pub(crate) struct Head<'a> {
    /// The leading identifier (the run before the first `(` or `<`).
    pub name: &'a str,
    /// The contents of the `(…)` group, if present.
    pub params: Option<&'a str>,
    /// The contents of the `<…>` group, if present.
    pub body: Option<&'a str>,
}

/// Splits a type string into its [`Head`]. Returns `None` on unbalanced brackets
/// or trailing garbage after the `<body>` (a malformed input).
pub(crate) fn split_head(input: &str) -> Option<Head<'_>> {
    let s = input.trim();
    let name_end = s.find(['(', '<']).unwrap_or(s.len());
    let name = s[..name_end].trim();
    let mut rest = &s[name_end..];

    let mut params = None;
    if let Some(after) = rest.strip_prefix('(') {
        let close = matching_close(after, '(', ')')?;
        params = Some(after[..close].trim());
        rest = after[close + 1..].trim_start();
    }

    let mut body = None;
    if let Some(after) = rest.strip_prefix('<') {
        let close = matching_close(after, '<', '>')?;
        body = Some(after[..close].trim());
        rest = after[close + 1..].trim_start();
    }

    if !rest.is_empty() {
        return None;
    }
    Some(Head { name, params, body })
}

/// Splits `input` on every top-level occurrence of `sep`, treating `<…>`, `(…)`
/// and `[…]` as opaque groups whose inner separators are ignored. Pieces are
/// trimmed; a `sep`-free input yields the whole (trimmed) string as one piece.
pub(crate) fn split_top_level(input: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in input.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            _ if c == sep && depth == 0 => {
                parts.push(input[start..i].trim());
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(input[start..].trim());
    parts
}

/// Finds the byte index of the first top-level `target` (one outside any `<…>` /
/// `(…)` / `[…]` group), or `None` if there is none.
pub(crate) fn find_top_level(input: &str, target: char) -> Option<usize> {
    let mut depth = 0i32;
    for (i, c) in input.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            _ if c == target && depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// Given the slice *after* an opening `open`, returns the byte index of its
/// matching `close` (honouring nesting of the same pair), or `None` if unbalanced.
fn matching_close(s: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 1i32;
    for (i, c) in s.char_indices() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_splits_name_params_body() {
        let h = split_head("fixed_size_list(3)<item: int64>").unwrap();
        assert_eq!(h.name, "fixed_size_list");
        assert_eq!(h.params, Some("3"));
        assert_eq!(h.body, Some("item: int64"));

        let h = split_head("timestamp(us, UTC)").unwrap();
        assert_eq!(h.name, "timestamp");
        assert_eq!(h.params, Some("us, UTC"));
        assert_eq!(h.body, None);

        let h = split_head("int64").unwrap();
        assert_eq!((h.name, h.params, h.body), ("int64", None, None));
    }

    #[test]
    fn head_rejects_malformed() {
        assert!(split_head("list<int64").is_none()); // unbalanced
        assert!(split_head("list<int64> trailing").is_none()); // trailing garbage
    }

    #[test]
    fn split_respects_nesting() {
        let parts = split_top_level("a: int64, b: list<c: int64>, d: utf8", ',');
        assert_eq!(parts, ["a: int64", "b: list<c: int64>", "d: utf8"]);
        // The ':' inside the nested body is not top-level.
        let body = "b: list<c: int64>";
        assert_eq!(find_top_level(body, ':'), Some(1));
    }

    #[test]
    fn empty_body_yields_single_empty_piece() {
        assert_eq!(split_top_level("", ','), [""]);
    }
}
