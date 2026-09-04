//! Shared hierarchy traversal for [`IOBase`](super::IOBase).

use super::IOBase;
use crate::holder::Holder;
use crate::{Error, Result, Url};

/// Resolve a chain of fixed names below `base`, without touching anything.
///
/// Returns `None` for an empty chain, so a caller can tell "descend nowhere"
/// from "descend to here".
pub(super) fn descend(base: &(impl IOBase + ?Sized), names: &[&str]) -> Result<Option<Holder>> {
    let Some((first, rest)) = names.split_first() else {
        return Ok(None);
    };
    let mut holder = base.child_by_path(first)?;
    for name in rest {
        holder = holder.child_by_path(name)?;
    }
    Ok(Some(holder))
}

/// Answer whether a container reads as one table, without listing the tree.
///
/// A folder reads as the table beneath it, so its leaves decide - and one leaf
/// is enough, because a partitioned tree is one table in one encoding. The walk
/// therefore descends towards the first leaf it can reach: every entry a level
/// already listed is checked before anything deeper is listed at all, so a lake
/// answers from the first partition that holds a file. Nothing is capped or
/// sampled; a container holding no tabular leaf anywhere is walked exactly as a
/// recursive listing would walk it, and answers `false` at the end of it.
///
/// A listing failure answers `false` rather than propagating: this is a
/// predicate, and a container nobody can list holds no rows anyone can read.
pub(crate) fn container_is_tabular(handle: &(impl IOBase + ?Sized)) -> bool {
    #[cfg(feature = "iceberg")]
    // A folder holding a table format is one tabular value however its files
    // are named, and asking costs one lookup of the metadata directory.
    if matches!(crate::media::iceberg::located(handle), Ok(Some(_))) {
        return true;
    }
    let mut level = handle.ls(false, false);
    // The frontier: the containers a level named and this walk has not opened
    // yet. It is bounded by the tree's width at the levels already listed, and
    // the walk stops at the first tabular leaf, so it is never the result.
    let mut deeper: Vec<Holder> = Vec::new();
    loop {
        for entry in level {
            let Ok(entry) = entry else {
                return false;
            };
            // The media type answers first because it is free, and no
            // container reports a tabular one - asking whether an entry is a
            // container is what costs a call into the backing store.
            if entry.media_type().is_tabular() {
                return true;
            }
            if entry.is_container() {
                deeper.push(entry);
            }
        }
        let Some(next) = deeper.pop() else {
            return false;
        };
        level = next.ls(false, false);
    }
}

/// Report a resource that cannot contain children.
pub(super) fn no_children(url: Option<&Url>, name: &str) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::NotADirectory,
        match url {
            Some(url) => format!("expected a container to resolve {name:?} against, got {url}"),
            None => format!("expected a container to resolve {name:?} against, got a buffer"),
        },
    ))
}
