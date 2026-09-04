//! Shards read and written through one [`IOBase`] handle.
//!
//! A shard is a JSON array of the core field document - what
//! [`Field::into_value`] projects and [`Field::from_value`] reads back - so
//! the whole `fix:` namespace persists with no serializer here. The file is
//! the one thing this module composes, and it is rendered indented so the
//! tracked seed reads in a diff.
//!
//! The layout is `<root>/records/<branch>/<shard>.json`, because a shard
//! index is only unique inside one dictionary. The record is authoritative
//! and the folder is layout: a field whose `fix:branch` contradicts the
//! folder it was read from is a typed error naming both.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use smol_str::{SmolStr, format_smolstr};

use super::registry::canonical_id;
use super::{FixBranch, FixRegistry};
use crate::generic::Holder;
use crate::io::IOBase;
use crate::text::Formatting;
use crate::{Error, Field, Result, Scalar, Url};

/// The folder under a registry root that holds its branch folders.
const RECORDS: &str = "records";
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

/// The branch a folder under `records/` names.
fn branch_of(entry: &Holder) -> Result<FixBranch> {
    FixBranch::from_str(entry.url().and_then(Url::file_name).unwrap_or_default())
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
    /// Loads every shard under `<handle>/records`.
    ///
    /// The one loader: the process default resolves through it too. A folder
    /// that does not exist lists nothing and answers the empty registry,
    /// which is every handle's laziness contract rather than a failure.
    ///
    /// # Errors
    ///
    /// Returns a typed error naming the URL when `records/` holds a leaf
    /// rather than only branch folders, when a folder's name is not a
    /// branch, or when a shard is not a JSON array of field documents,
    /// holds a field without a `fix:tag`, with a tag another shard owns, with
    /// a branch its folder contradicts, or that the registry refuses.
    pub fn from_handle(handle: &dyn IOBase) -> Result<Self> {
        let mut registry = Self::new();
        let records = handle.child_by_path(RECORDS)?;
        for folder in records.ls(false, false) {
            let folder = folder?;
            if !folder.is_container() {
                return Err(Error::InvalidRecord {
                    path: records.url().map_or_else(
                        || SmolStr::new_static(RECORDS),
                        |url| format_smolstr!("{url}"),
                    ),
                    reason: crate::text::expected_got(
                        "only branch folders",
                        format_args!(
                            "the leaf {:?}",
                            folder.url().and_then(Url::file_name).unwrap_or_default()
                        ),
                    ),
                });
            }
            let branch =
                branch_of(&folder).map_err(|error| in_shard(error, At("folder", folder.url())))?;
            for entry in folder.ls(false, false) {
                let entry = entry?;
                let Some(shard) = shard_index(&entry) else {
                    continue;
                };
                registry
                    .load_shard(&entry, &branch, shard)
                    .map_err(|error| in_shard(error, At("shard", entry.url())))?;
            }
        }
        Ok(registry)
    }

    /// Read one shard's fields into this registry.
    fn load_shard(&mut self, entry: &Holder, branch: &FixBranch, shard: i32) -> Result<()> {
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
            self.insert(field)?;
        }
        Ok(())
    }

    /// Writes every populated shard under `<root>/records/<branch>` and
    /// removes the shards and branch folders no field populates any more.
    ///
    /// Each shard is written whole through the handle's ordinary byte write,
    /// creating the file and its parents, so a failed write leaves the prior
    /// shard intact. The cleanup afterwards is what keeps a reload from
    /// resurrecting a removed field.
    ///
    /// # Errors
    ///
    /// Returns the handle's write, listing or removal failure, the parse
    /// failure when a stored `fix:` property is malformed, or the encoder's.
    pub fn write_into(&self, root: &mut dyn IOBase) -> Result<()> {
        let mut shards: BTreeMap<(FixBranch, i32), Vec<&Field>> = BTreeMap::new();
        for field in self {
            let id = canonical_id(field)?;
            shards
                .entry((id.branch().clone(), shard_of(id.tag())))
                .or_default()
                .push(field);
        }
        for ((branch, shard), fields) in &shards {
            let document =
                Scalar::from_sequence(fields.iter().map(|field| (*field).clone().into_value()));
            let bytes =
                crate::json::into_bytes_with_formatting(&document, Formatting::indented(2))?;
            root.child_by_path(&format!("{RECORDS}/{branch}/{shard}.{EXTENSION}"))?
                .write_all_bytes(&bytes)?;
        }
        let held: BTreeSet<&FixBranch> = shards.keys().map(|(branch, _)| branch).collect();
        for folder in root.child_by_path(RECORDS)?.ls(false, false) {
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
                    .is_some_and(|shard| !shards.contains_key(&(branch.clone(), shard)))
                {
                    entry.remove(false)?;
                }
            }
        }
        Ok(())
    }
}
