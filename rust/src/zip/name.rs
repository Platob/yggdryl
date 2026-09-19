//! Member paths, which are the archive's own names rather than URL text.
//!
//! A ZIP member is named by a `/` separated relative path with no drive, no
//! leading separator, and no `.` or `..` left in it. Normalizing here once is
//! what lets every role compare names by equality and lets a prefix decide
//! containment with a string test rather than a walk.

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result};

/// Resolve `path` against the member prefix `base`.
///
/// `base` is `""` for the archive root and otherwise a normalized prefix with
/// no trailing separator. The result is normalized the same way, so the two
/// are directly comparable.
///
/// # Errors
///
/// Returns [`Error::Parse`] when the path climbs above the archive root, which
/// is the one thing a member name can never spell.
pub(super) fn resolve(base: &str, path: &str) -> Result<SmolStr> {
    let mut parts: Vec<&str> = if base.is_empty() {
        Vec::new()
    } else {
        base.split('/').collect()
    };
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(Error::Parse {
                        target: "zip member path",
                        position: 0,
                        reason: format_smolstr!(
                            "expected a path inside the archive, got {path:?} under {base:?}"
                        ),
                    });
                }
            }
            other => parts.push(other),
        }
    }
    Ok(SmolStr::new(parts.join("/")))
}

/// Return `name` relative to the prefix `base`, when it is under it.
///
/// The root prefix contains everything; any other prefix contains exactly the
/// names that continue it at a separator, so `lake` never captures `lakeside`.
pub(super) fn under<'name>(base: &str, name: &'name str) -> Option<&'name str> {
    if base.is_empty() {
        return Some(name);
    }
    name.strip_prefix(base)?.strip_prefix('/')
}

/// Return the first segment of `relative`, and whether more follow it.
///
/// This is what turns a flat index into one level of a listing: a name that
/// still has a separator names a directory this level holds, and one that does
/// not names a member.
pub(super) fn head(relative: &str) -> (&str, bool) {
    match relative.split_once('/') {
        Some((head, rest)) => (head, !rest.is_empty()),
        None => (relative, false),
    }
}

/// Return the last segment of `name`, which is what its representation is
/// inferred from.
pub(super) fn base_name(name: &str) -> &str {
    let name = name.trim_end_matches('/');
    name.rsplit_once('/').map_or(name, |(_, last)| last)
}

/// Return the prefix `name` lives under, or `None` at the archive root.
pub(super) fn parent(name: &str) -> Option<&str> {
    name.trim_end_matches('/')
        .rsplit_once('/')
        .map(|(up, _)| up)
}

/// Whether any segment of `name` is private, which a listing hides by default.
pub(super) fn is_private(name: &str) -> bool {
    name.split('/').any(|part| part.starts_with('.'))
}

/// Spell the directory record for prefix `base`, which always ends with `/`.
pub(super) fn directory_name(base: &str) -> SmolStr {
    format_smolstr!("{base}/")
}
