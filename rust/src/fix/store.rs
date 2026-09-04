//! Shards read and written through one [`IOBase`] handle.
//!
//! A shard is a JSON array of the core field document - what
//! [`Field::into_value`] projects and [`Field::from_value`] reads back - so
//! the whole `fix:` namespace persists with no serializer here. The file is
//! the one thing this module composes, and it is rendered indented so the
//! tracked seed reads in a diff.
//!
//! The layout is `<root>/primitive/<branch>/<shard>.json` and
//! `<root>/nested/<branch>/<shard>.json`: a shard index is only unique inside
//! one dictionary, and the two trees separate the scalar fields a transcriber
//! resolves per wire tag from the components and repeating groups that carry a
//! whole subtree. Either tree may be absent - a dictionary of only scalars
//! writes no `nested/`, and a root with neither loads as the empty registry.
//! The record is authoritative and the folder is layout: a field whose
//! `fix:branch` contradicts the branch folder it was read from, or whose
//! datatype contradicts the tree, is a typed error naming both.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use smol_str::{SmolStr, format_smolstr};

use super::registry::{canonical_id, is_nested};
use super::{FixBranch, FixRegistry};
use crate::generic::Holder;
use crate::io::IOBase;
use crate::text::Formatting;
use crate::{Error, Field, Result, Scalar, Url};

/// The tree under a registry root holding the branch folders of the fields
/// whose datatype is one scalar value.
const PRIMITIVE: &str = "primitive";
/// The tree holding the branch folders of the components and repeating
/// groups.
const NESTED: &str = "nested";
/// Both trees, in the order a load walks them and a write cleans them.
const TREES: [&str; 2] = [PRIMITIVE, NESTED];
/// The single folder the two trees replaced.
///
/// It is named here for one reason: a root still holding it is a registry
/// written by a retired layout, and answering that with an empty dictionary
/// would turn every later lookup into a wrong answer instead of a failure.
const RETIRED: &str = "records";
/// How many consecutive tags one shard holds.
const SHARD_WIDTH: i32 = 100;
/// The extension every shard carries.
const EXTENSION: &str = "json";

/// The one shard that can hold `tag`, within its branch.
///
/// Tags are non-negative, which [`FixFieldMut::set_tag`](crate::FixFieldMut)
/// and the parse behind [`FixField::tag`](crate::FixField) guarantee, so the
/// division is total and never names a negative shard.
pub(super) const fn shard_of(tag: i32) -> i32 {
    tag / SHARD_WIDTH
}

/// The shard index a listing entry carries, when it is a shard.
///
/// A shard is a leaf named `<n>.json` with a decimal `n`; any other entry is
/// not one and is left alone by both the reader and the writer's cleanup.
/// This tolerance holds only *inside* a branch folder: the level above it
/// admits nothing but branch folders.
fn shard_index(entry: &Holder) -> Option<i32> {
    if entry.is_container() {
        return None;
    }
    let url = entry.url()?;
    if url.extension() != Some(EXTENSION) {
        return None;
    }
    let stem = url.stem()?;
    if stem.is_empty() || !stem.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    stem.parse().ok()
}

/// The branch a folder under a tree root names.
fn branch_of(entry: &Holder) -> Result<FixBranch> {
    FixBranch::from_str(entry.url().and_then(Url::file_name).unwrap_or_default())
}

/// The tree a field's definition belongs in.
///
/// One predicate decides both the registry half and the file, so a field
/// cannot be indexed as one thing and written as another.
fn tree_of(field: &Field) -> &'static str {
    if is_nested(field) { NESTED } else { PRIMITIVE }
}

/// Where something was read from, for the failure that names it.
struct At<'entry>(&'static str, Option<&'entry Url>);

impl fmt::Display for At<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.1 {
            Some(url) => write!(formatter, "{} {url}", self.0),
            None => write!(formatter, "an unnamed {}", self.0),
        }
    }
}

/// Attach the location to a failure raised while reading it.
fn in_shard(error: Error, at: At<'_>) -> Error {
    match error {
        Error::Codec {
            format,
            position,
            reason,
        } => Error::Codec {
            format,
            position,
            reason: format_smolstr!("{reason} in {at}"),
        },
        Error::Parse {
            target,
            position,
            reason,
        } => Error::Parse {
            target,
            position,
            reason: format_smolstr!("{reason} in {at}"),
        },
        Error::Conflict {
            expected,
            actual,
            path,
        } => Error::Conflict {
            expected,
            actual,
            path: format_smolstr!("{path} in {at}"),
        },
        Error::Absent { expected, path } => Error::Absent {
            expected,
            path: format_smolstr!("{path} in {at}"),
        },
        Error::InvalidRecord { path, reason } => Error::InvalidRecord {
            path,
            reason: format_smolstr!("{reason} in {at}"),
        },
        Error::InvalidMetadataValue { key, reason } => Error::InvalidMetadataValue {
            key,
            reason: format_smolstr!("{reason} in {at}"),
        },
        other => Error::Codec {
            format: "json",
            position: 0,
            reason: format_smolstr!("{other} in {at}"),
        },
    }
}

impl FixRegistry {
    /// Loads every shard under `<handle>/primitive` and `<handle>/nested`.
    ///
    /// The one loader: the process default resolves through it too. A tree
    /// that does not exist lists nothing, so a dictionary of only scalars and
    /// a root with neither tree both load without a failure - every handle's
    /// laziness contract. Both trees are read whole, because a name has no
    /// numeric structure to pick a shard with; what the split buys the loader
    /// is that the nested definitions are a contiguous, skippable half.
    ///
    /// # Errors
    ///
    /// Returns a typed error naming the URL when a tree holds a leaf rather
    /// than only branch folders, when a folder's name is not a branch, or
    /// when a shard is not a JSON array of field documents, holds a field
    /// without a `fix:tag`, with a tag another shard owns, with a branch its
    /// folder contradicts, with a datatype its tree contradicts, or that the
    /// registry refuses.
    pub fn from_handle(handle: &dyn IOBase) -> Result<Self> {
        let mut registry = Self::new();
        // A root written before the trees existed holds `records/` and neither
        // of them, so every read below would answer nothing and the caller
        // would get a working-looking empty dictionary instead of a failure.
        // Absence is the laziness contract; a retired layout sitting there is
        // not absence, and this module never falls back to empty on one.
        let retired = handle.child_by_path(RETIRED)?;
        if retired.is_container() {
            return Err(Error::InvalidRecord {
                path: retired
                    .url()
                    .map_or_else(|| SmolStr::new(RETIRED), |url| format_smolstr!("{url}")),
                reason: crate::text::expected_got(
                    format_args!("the {PRIMITIVE} and {NESTED} trees"),
                    format_args!("the retired {RETIRED:?} folder"),
                ),
            });
        }
        for tree in TREES {
            let root = handle.child_by_path(tree)?;
            for folder in root.ls(false, false) {
                let folder = folder?;
                if !folder.is_container() {
                    return Err(Error::InvalidRecord {
                        path: root
                            .url()
                            .map_or_else(|| SmolStr::new(tree), |url| format_smolstr!("{url}")),
                        reason: crate::text::expected_got(
                            "only branch folders",
                            format_args!(
                                "the leaf {:?}",
                                folder.url().and_then(Url::file_name).unwrap_or_default()
                            ),
                        ),
                    });
                }
                let branch = branch_of(&folder)
                    .map_err(|error| in_shard(error, At("folder", folder.url())))?;
                for entry in folder.ls(false, false) {
                    let entry = entry?;
                    let Some(shard) = shard_index(&entry) else {
                        continue;
                    };
                    registry
                        .load_shard(&entry, tree, &branch, shard)
                        .map_err(|error| in_shard(error, At("shard", entry.url())))?;
                }
            }
        }
        Ok(registry)
    }

    /// Read one shard's fields into this registry.
    fn load_shard(
        &mut self,
        entry: &Holder,
        tree: &'static str,
        branch: &FixBranch,
        shard: i32,
    ) -> Result<()> {
        let document = crate::from_json_scalar(entry.read_all_bytes()?)?;
        let Some(fields) = document.as_sequence() else {
            return Err(Error::Codec {
                format: "json",
                position: 0,
                reason: crate::text::expected_got(
                    "a JSON array of field documents",
                    document.kind(),
                ),
            });
        };
        for value in fields {
            let field = Field::from_value(value.clone())?;
            let id = canonical_id(&field)?;
            if shard_of(id.tag()) != shard {
                return Err(Error::InvalidRecord {
                    path: field.name().into(),
                    reason: crate::text::expected_got(
                        format_args!(
                            "a tag of shard {shard}, from {} to {}",
                            shard * SHARD_WIDTH,
                            shard * SHARD_WIDTH + SHARD_WIDTH - 1
                        ),
                        format_args!("tag {}", id.tag()),
                    ),
                });
            }
            if id.branch() != branch {
                return Err(Error::InvalidRecord {
                    path: field.name().into(),
                    reason: crate::text::expected_got(
                        format_args!("the branch {:?} its folder names", branch.as_str()),
                        format_args!("{:?}", id.branch().as_str()),
                    ),
                });
            }
            if tree_of(&field) != tree {
                return Err(Error::InvalidRecord {
                    path: field.name().into(),
                    reason: crate::text::expected_got(
                        format_args!("a datatype of the {tree} tree it was read from"),
                        format_args!("{}", field.dtype()),
                    ),
                });
            }
            self.insert(field)?;
        }
        Ok(())
    }

    /// Writes every populated shard under `<root>/<tree>/<branch>` and removes
    /// the shards, branch folders and whole trees no field populates any more.
    ///
    /// Each field is routed to its tree by the one predicate the registry
    /// indexes it with, so a field whose datatype changed from scalar to
    /// nested is written to its new tree and the file it left is removed by
    /// the same cleanup that removes an emptied shard. Each shard is written
    /// whole through the handle's ordinary byte write, creating the file and
    /// its parents, so a failed write leaves the prior shard intact. The
    /// cleanup afterwards is what keeps a reload from resurrecting a removed
    /// field.
    ///
    /// # Errors
    ///
    /// Returns the handle's write, listing or removal failure, the parse
    /// failure when a stored `fix:` property is malformed, or the encoder's.
    pub fn write_into(&self, root: &mut dyn IOBase) -> Result<()> {
        let mut shards: BTreeMap<(&'static str, FixBranch, i32), Vec<&Field>> = BTreeMap::new();
        for field in self {
            let id = canonical_id(field)?;
            shards
                .entry((tree_of(field), id.branch().clone(), shard_of(id.tag())))
                .or_default()
                .push(field);
        }
        for ((tree, branch, shard), fields) in &shards {
            let document =
                Scalar::from_sequence(fields.iter().map(|field| (*field).clone().into_value()));
            let bytes =
                crate::json::into_bytes_with_formatting(&document, Formatting::indented(2))?;
            root.child_by_path(&format!("{tree}/{branch}/{shard}.{EXTENSION}"))?
                .write_all_bytes(&bytes)?;
        }
        for tree in TREES {
            let held: BTreeSet<&FixBranch> = shards
                .keys()
                .filter(|(populated, _, _)| *populated == tree)
                .map(|(_, branch, _)| branch)
                .collect();
            let mut root = root.child_by_path(tree)?;
            if held.is_empty() {
                // A tree no field populates goes whole, so a dictionary of
                // only scalars leaves no empty `nested/` behind. Absence is
                // never a removal failure, so an unwritten tree needs no
                // separate check.
                root.remove(true)?;
                continue;
            }
            for folder in root.ls(false, false) {
                let mut folder = folder?;
                // A leaf here is what `from_handle` refuses, and a folder whose
                // name is not a branch is nothing this registry wrote: the
                // writer neither owns either nor deletes them.
                if !folder.is_container() {
                    continue;
                }
                let Ok(branch) = branch_of(&folder) else {
                    continue;
                };
                if !held.contains(&branch) {
                    folder.remove(true)?;
                    continue;
                }
                for entry in folder.ls(false, false) {
                    let mut entry = entry?;
                    if shard_index(&entry)
                        .is_some_and(|shard| !shards.contains_key(&(tree, branch.clone(), shard)))
                    {
                        entry.remove(false)?;
                    }
                }
            }
        }
        Ok(())
    }
}
