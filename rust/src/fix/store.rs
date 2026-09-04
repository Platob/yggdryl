//! Shards read and written through one [`IOBase`] handle.
//!
//! A shard is a JSON array of the core field document - what
//! [`Field::into_value`] projects and [`Field::from_value`] reads back - so
//! the whole `fix:` namespace persists with no serializer here. The file is
//! the one thing this module composes, and it is rendered indented so the
//! tracked seed reads in a diff.

use std::collections::BTreeMap;
use std::fmt;

use smol_str::format_smolstr;

use super::FixRegistry;
use super::registry::canonical_tag;
use crate::generic::Holder;
use crate::io::IOBase;
use crate::text::Formatting;
use crate::{Error, Field, Result, Scalar, Url};

/// The folder under a registry root that holds its shards.
const RECORDS: &str = "records";
/// How many consecutive tags one shard holds.
const SHARD_WIDTH: i32 = 100;
/// The extension every shard carries.
const EXTENSION: &str = "json";

/// The one shard that can hold `tag`.
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

/// Where a shard was read from, for the failure that names it.
struct Shard<'entry>(Option<&'entry Url>);

impl fmt::Display for Shard<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(url) => write!(formatter, "shard {url}"),
            None => formatter.write_str("an unnamed shard"),
        }
    }
}

/// Attach the shard's location to a failure raised while reading it.
fn in_shard(error: Error, url: Shard<'_>) -> Error {
    match error {
        Error::Codec {
            format,
            position,
            reason,
        } => Error::Codec {
            format,
            position,
            reason: format_smolstr!("{reason} in {url}"),
        },
        Error::Conflict {
            expected,
            actual,
            path,
        } => Error::Conflict {
            expected,
            actual,
            path: format_smolstr!("{path} in {url}"),
        },
        Error::Absent { expected, path } => Error::Absent {
            expected,
            path: format_smolstr!("{path} in {url}"),
        },
        Error::InvalidRecord { path, reason } => Error::InvalidRecord {
            path,
            reason: format_smolstr!("{reason} in {url}"),
        },
        Error::InvalidMetadataValue { key, reason } => Error::InvalidMetadataValue {
            key,
            reason: format_smolstr!("{reason} in {url}"),
        },
        other => Error::Codec {
            format: "json",
            position: 0,
            reason: format_smolstr!("{other} in {url}"),
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
    /// Returns a typed error naming the shard's URL when a shard is not a
    /// JSON array of field documents, holds a field without a `fix:tag` or
    /// with a tag another shard owns, or holds a field the registry refuses.
    pub fn from_handle(handle: &dyn IOBase) -> Result<Self> {
        let mut registry = Self::new();
        for entry in handle.child_by_path(RECORDS)?.ls(false, false) {
            let entry = entry?;
            let Some(shard) = shard_index(&entry) else {
                continue;
            };
            registry
                .load_shard(&entry, shard)
                .map_err(|error| in_shard(error, Shard(entry.url())))?;
        }
        Ok(registry)
    }

    /// Read one shard's fields into this registry.
    fn load_shard(&mut self, entry: &Holder, shard: i32) -> Result<()> {
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
            let tag = canonical_tag(&field)?;
            if shard_of(tag) != shard {
                return Err(Error::InvalidRecord {
                    path: field.name().into(),
                    reason: crate::text::expected_got(
                        format_args!(
                            "a tag of shard {shard}, from {} to {}",
                            shard * SHARD_WIDTH,
                            shard * SHARD_WIDTH + SHARD_WIDTH - 1
                        ),
                        format_args!("tag {tag}"),
                    ),
                });
            }
            self.insert(field)?;
        }
        Ok(())
    }

    /// Writes every populated shard under `<root>/records` and removes the
    /// shards no field populates any more.
    ///
    /// Each shard is written whole through the handle's ordinary byte write,
    /// creating the file and its parents, so a failed write leaves the prior
    /// shard intact. The cleanup afterwards is what keeps a reload from
    /// resurrecting a removed field.
    ///
    /// # Errors
    ///
    /// Returns the handle's write, listing or removal failure, or the
    /// encoder's.
    pub fn write_into(&self, root: &mut dyn IOBase) -> Result<()> {
        let mut shards: BTreeMap<i32, Vec<&Field>> = BTreeMap::new();
        for field in self {
            shards
                .entry(shard_of(canonical_tag(field)?))
                .or_default()
                .push(field);
        }
        for (shard, fields) in &shards {
            let document =
                Scalar::from_sequence(fields.iter().map(|field| (*field).clone().into_value()));
            let bytes =
                crate::json::into_bytes_with_formatting(&document, Formatting::indented(2))?;
            root.child_by_path(&format!("{RECORDS}/{shard}.{EXTENSION}"))?
                .write_all_bytes(&bytes)?;
        }
        for entry in root.child_by_path(RECORDS)?.ls(false, false) {
            let mut entry = entry?;
            if shard_index(&entry).is_some_and(|shard| !shards.contains_key(&shard)) {
                entry.remove(false)?;
            }
        }
        Ok(())
    }
}
