//! Snapshots, the named references that point at them, and their summaries.
//!
//! A snapshot is one complete version of a table's contents: an identifier, the
//! manifest list naming every manifest alive at that moment, and a summary of
//! what the commit did. A table's *current* snapshot is a pointer, which is why
//! a table with snapshots can still have no current one - a rolled-back or
//! freshly created table is exactly that, and reading it must yield no rows
//! rather than fail.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result, Scalar};

/// The reference every table's default branch is named by.
pub const MAIN_BRANCH: &str = "main";

/// One committed version of a table's contents.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// Identifier of this snapshot, unique within the table.
    pub snapshot_id: i64,
    /// The snapshot this one was produced from, when there was one.
    pub parent_snapshot_id: Option<i64>,
    /// Monotonic commit order, absent in v1 tables.
    pub sequence_number: Option<i64>,
    /// Wall-clock commit time in milliseconds since the Unix epoch.
    pub timestamp_ms: i64,
    /// Location of the Avro manifest list this snapshot's manifests are in.
    pub manifest_list: SmolStr,
    /// V1 manifest locations stored directly instead of a manifest list.
    pub manifests: Option<Vec<SmolStr>>,
    /// What the commit did, keyed by Iceberg's summary vocabulary.
    pub summary: Vec<(SmolStr, SmolStr)>,
    /// The schema in effect when the snapshot was written.
    pub schema_id: Option<i32>,
    /// V3 encryption key used by this snapshot, when encrypted.
    pub encryption_key_id: Option<SmolStr>,
    /// First assigned row identifier, added in v3 for row lineage.
    pub first_row_id: Option<i64>,
    /// Rows this snapshot added, added in v3 for row lineage.
    pub added_rows: Option<i64>,
}

#[derive(Eq, Hash, Ord, PartialEq, PartialOrd)]
struct SnapshotIdentity<'a> {
    snapshot_id: i64,
    parent_snapshot_id: Option<i64>,
    sequence_number: Option<i64>,
    timestamp_ms: i64,
    manifest_list: &'a SmolStr,
    manifests: &'a Option<Vec<SmolStr>>,
    summary: Vec<&'a (SmolStr, SmolStr)>,
    schema_id: Option<i32>,
    encryption_key_id: &'a Option<SmolStr>,
    first_row_id: Option<i64>,
    added_rows: Option<i64>,
}

impl Snapshot {
    /// Return a deterministic hash of this complete snapshot description.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

    fn identity(&self) -> SnapshotIdentity<'_> {
        SnapshotIdentity {
            snapshot_id: self.snapshot_id,
            parent_snapshot_id: self.parent_snapshot_id,
            sequence_number: self.sequence_number,
            timestamp_ms: self.timestamp_ms,
            manifest_list: &self.manifest_list,
            manifests: &self.manifests,
            summary: crate::generic::sorted_pairs(&self.summary),
            schema_id: self.schema_id,
            encryption_key_id: &self.encryption_key_id,
            first_row_id: self.first_row_id,
            added_rows: self.added_rows,
        }
    }

    /// Return one summary value by key.
    pub fn summary_value(&self, key: &str) -> Option<&str> {
        self.summary
            .iter()
            .find_map(|(name, value)| (name == key).then(|| value.as_str()))
    }

    /// Return the operation this snapshot recorded, defaulting to `append`.
    pub fn operation(&self) -> &str {
        self.summary_value("operation").unwrap_or("append")
    }

    /// Validate the fields this snapshot may carry in one table version.
    pub(crate) fn validate_for_version(&self, version: super::FormatVersion) -> Result<()> {
        if self.manifests.is_some() && !self.manifest_list.is_empty() {
            return Err(invalid(format_smolstr!(
                "expected either manifest-list or manifests on snapshot {}, got both",
                self.snapshot_id
            )));
        }
        if version != super::FormatVersion::V1 && self.manifests.is_some() {
            return Err(invalid(format_smolstr!(
                "expected no manifests array on an Iceberg v{} snapshot",
                version.number()
            )));
        }
        match (version, self.sequence_number) {
            (super::FormatVersion::V1, Some(_)) => {
                return Err(invalid(SmolStr::new_static(
                    "expected no sequence-number on an Iceberg v1 snapshot",
                )));
            }
            (super::FormatVersion::V2 | super::FormatVersion::V3, None) => {
                return Err(invalid(format_smolstr!(
                    "expected a sequence-number on Iceberg v{} snapshot {}",
                    version.number(),
                    self.snapshot_id
                )));
            }
            (super::FormatVersion::V2 | super::FormatVersion::V3, Some(sequence))
                if sequence < 0 =>
            {
                return Err(invalid(format_smolstr!(
                    "expected a non-negative sequence-number on snapshot {}, got {}",
                    self.snapshot_id,
                    sequence
                )));
            }
            _ => {}
        }
        if version < super::FormatVersion::V3 {
            let unsupported = if self.encryption_key_id.is_some() {
                Some("key-id")
            } else if self.first_row_id.is_some() {
                Some("first-row-id")
            } else if self.added_rows.is_some() {
                Some("added-rows")
            } else {
                None
            };
            if let Some(name) = unsupported {
                return Err(invalid(format_smolstr!(
                    "expected no {name} on an Iceberg v{} snapshot",
                    version.number()
                )));
            }
        }
        match (self.first_row_id, self.added_rows) {
            (Some(first), Some(added)) if first >= 0 && added >= 0 => Ok(()),
            (Some(first), Some(added)) => Err(invalid(format_smolstr!(
                "expected non-negative first-row-id and added-rows, got {first} and {added}"
            ))),
            (None, None) => Ok(()),
            _ => Err(invalid(SmolStr::new_static(
                "expected first-row-id and added-rows together",
            ))),
        }
    }

    /// Read one snapshot object.
    ///
    /// # Errors
    ///
    /// Returns an error when `snapshot-id`, `timestamp-ms`, or `manifest-list`
    /// is missing or not the type Iceberg declares.
    pub fn from_json(document: &Scalar) -> Result<Self> {
        let snapshot_id = document
            .get_key_str("snapshot-id")
            .and_then(Scalar::as_i64)
            .ok_or_else(|| invalid(SmolStr::new_static("expected a snapshot \"snapshot-id\"")))?;
        let timestamp_ms = document
            .get_key_str("timestamp-ms")
            .and_then(Scalar::as_i64)
            .ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected a \"timestamp-ms\" on snapshot {snapshot_id}"
                ))
            })?;
        let manifest_list = optional_string(document, "manifest-list", "snapshot")?;
        let manifests = match document.get_key_str("manifests") {
            None => None,
            Some(paths) if paths.is_null() => None,
            Some(paths) => Some(
                paths
                    .as_sequence()
                    .ok_or_else(|| {
                        invalid(format_smolstr!(
                            "expected a manifests array on snapshot {snapshot_id}"
                        ))
                    })?
                    .iter()
                    .enumerate()
                    .map(|(index, path)| {
                        path.as_str().map(SmolStr::new).ok_or_else(|| {
                            invalid(format_smolstr!(
                                "expected a string at manifests[{index}] on snapshot {snapshot_id}"
                            ))
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
            ),
        };
        let manifest_list = match (manifest_list, &manifests) {
            (Some(_), Some(_)) => {
                return Err(invalid(format_smolstr!(
                    "expected either manifest-list or manifests on snapshot {snapshot_id}, got both"
                )));
            }
            (None, None) => {
                return Err(invalid(format_smolstr!(
                    "expected a manifest-list string or manifests array on snapshot {snapshot_id}"
                )));
            }
            (Some(path), None) => path,
            (None, Some(_)) => SmolStr::new_static(""),
        };
        let summary = string_mapping(document, "summary", snapshot_id)?;

        let snapshot = Self {
            snapshot_id,
            parent_snapshot_id: optional_i64(document, "parent-snapshot-id", "snapshot")?,
            sequence_number: optional_i64(document, "sequence-number", "snapshot")?,
            timestamp_ms,
            manifest_list,
            manifests,
            summary,
            schema_id: optional_i32(document, "schema-id", "snapshot")?,
            encryption_key_id: optional_string(document, "key-id", "snapshot")?,
            first_row_id: optional_i64(document, "first-row-id", "snapshot")?,
            added_rows: optional_i64(document, "added-rows", "snapshot")?,
        };
        if let Some(sequence) = snapshot.sequence_number
            && sequence < 0
        {
            return Err(invalid(format_smolstr!(
                "expected a non-negative sequence-number on snapshot {snapshot_id}, got {}",
                sequence
            )));
        }
        match (snapshot.first_row_id, snapshot.added_rows) {
            (Some(first), Some(added)) if first >= 0 && added >= 0 => {}
            (Some(first), Some(added)) => {
                return Err(invalid(format_smolstr!(
                    "expected non-negative first-row-id and added-rows, got {first} and {added}"
                )));
            }
            (None, None) => {}
            _ => {
                return Err(invalid(SmolStr::new_static(
                    "expected first-row-id and added-rows together",
                )));
            }
        }
        Ok(snapshot)
    }

    /// Write this snapshot as the object a table metadata document holds.
    ///
    /// `version` selects what is emitted: a v1 snapshot carries no sequence
    /// number and a v3 one carries its row lineage.
    ///
    /// # Errors
    ///
    /// Returns an error only when the mapping cannot be built.
    pub fn into_json(self, version: super::FormatVersion) -> Result<Scalar> {
        self.validate_for_version(version)?;
        let mut entries = vec![(Scalar::from("snapshot-id"), Scalar::from(self.snapshot_id))];
        if let Some(parent) = self.parent_snapshot_id {
            entries.push((Scalar::from("parent-snapshot-id"), Scalar::from(parent)));
        }
        if version >= super::FormatVersion::V2 {
            if let Some(sequence) = self.sequence_number {
                entries.push((Scalar::from("sequence-number"), Scalar::from(sequence)));
            }
        }
        entries.push((
            Scalar::from("timestamp-ms"),
            Scalar::from(self.timestamp_ms),
        ));
        if let Some(manifests) = self.manifests {
            entries.push((
                Scalar::from("manifests"),
                Scalar::from_sequence(manifests.into_iter().map(Scalar::from)),
            ));
        } else {
            entries.push((
                Scalar::from("manifest-list"),
                Scalar::from(self.manifest_list.clone()),
            ));
        }
        entries.push((
            Scalar::from("summary"),
            Scalar::from_mapping(
                self.summary
                    .iter()
                    .map(|(key, value)| (Scalar::from(key.clone()), Scalar::from(value.clone()))),
            )?,
        ));
        if let Some(schema_id) = self.schema_id {
            entries.push((
                Scalar::from("schema-id"),
                Scalar::from(i64::from(schema_id)),
            ));
        }
        if let Some(key_id) = self.encryption_key_id {
            entries.push((Scalar::from("key-id"), Scalar::from(key_id)));
        }
        if version >= super::FormatVersion::V3 {
            if let Some(first_row_id) = self.first_row_id {
                entries.push((Scalar::from("first-row-id"), Scalar::from(first_row_id)));
            }
            if let Some(added_rows) = self.added_rows {
                entries.push((Scalar::from("added-rows"), Scalar::from(added_rows)));
            }
        }
        Scalar::from_mapping(entries)
    }
}

impl PartialEq for Snapshot {
    fn eq(&self, other: &Self) -> bool {
        self.identity() == other.identity()
    }
}

impl Eq for Snapshot {}

impl PartialOrd for Snapshot {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Snapshot {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.identity().cmp(&other.identity())
    }
}

impl Hash for Snapshot {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity().hash(state);
    }
}

/// A named pointer at one snapshot: a branch or a tag.
///
/// A branch moves on every commit and a tag never moves, which is the whole
/// difference between them. The optional retention fields tune snapshot
/// expiration: the two `*snapshot*` limits describe how much of a branch's
/// history to keep and so belong to branches alone, while `max_ref_age_ms`
/// bounds the life of the reference itself and applies to both kinds.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SnapshotRef {
    /// The snapshot this reference names.
    pub snapshot_id: i64,
    /// Either `branch` or `tag`; a branch moves and a tag does not.
    pub kind: SmolStr,
    /// Fewest snapshots expiration keeps on this branch, head included.
    pub min_snapshots_to_keep: Option<i32>,
    /// Oldest ancestor age expiration keeps on this branch, in milliseconds.
    pub max_snapshot_age_ms: Option<i64>,
    /// Age at which the reference itself expires, measured in milliseconds
    /// from its snapshot's commit time.
    pub max_ref_age_ms: Option<i64>,
}

impl SnapshotRef {
    /// Return a deterministic hash of this complete snapshot reference.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

    /// Point a branch at one snapshot.
    pub fn branch(snapshot_id: i64) -> Self {
        Self {
            snapshot_id,
            kind: SmolStr::new_static("branch"),
            min_snapshots_to_keep: None,
            max_snapshot_age_ms: None,
            max_ref_age_ms: None,
        }
    }

    /// Point a tag at one snapshot.
    pub fn tag(snapshot_id: i64) -> Self {
        Self {
            snapshot_id,
            kind: SmolStr::new_static("tag"),
            min_snapshots_to_keep: None,
            max_snapshot_age_ms: None,
            max_ref_age_ms: None,
        }
    }

    /// Return whether this reference is a branch, the kind that moves.
    pub fn is_branch(&self) -> bool {
        self.kind == "branch"
    }

    /// Return whether this reference is a tag, the kind that never moves.
    pub fn is_tag(&self) -> bool {
        self.kind == "tag"
    }

    /// Keep at least this many snapshots on the branch, head included.
    ///
    /// # Errors
    ///
    /// Returns an error when `count` is not positive or when this reference is
    /// not a branch, because only a branch has ancestors to retain.
    pub fn with_min_snapshots_to_keep(mut self, count: i32) -> Result<Self> {
        self.expect_branch("min-snapshots-to-keep")?;
        if count <= 0 {
            return Err(invalid(format_smolstr!(
                "expected a positive min-snapshots-to-keep, got {count}"
            )));
        }
        self.min_snapshots_to_keep = Some(count);
        Ok(self)
    }

    /// Keep the branch's ancestors younger than this many milliseconds.
    ///
    /// # Errors
    ///
    /// Returns an error when `age_ms` is not positive or when this reference
    /// is not a branch, because only a branch has ancestors to retain.
    pub fn with_max_snapshot_age_ms(mut self, age_ms: i64) -> Result<Self> {
        self.expect_branch("max-snapshot-age-ms")?;
        if age_ms <= 0 {
            return Err(invalid(format_smolstr!(
                "expected a positive max-snapshot-age-ms, got {age_ms}"
            )));
        }
        self.max_snapshot_age_ms = Some(age_ms);
        Ok(self)
    }

    /// Let the reference itself expire this many milliseconds after its
    /// snapshot was committed.
    ///
    /// # Errors
    ///
    /// Returns an error when `age_ms` is not positive.
    pub fn with_max_ref_age_ms(mut self, age_ms: i64) -> Result<Self> {
        if age_ms <= 0 {
            return Err(invalid(format_smolstr!(
                "expected a positive max-ref-age-ms, got {age_ms}"
            )));
        }
        self.max_ref_age_ms = Some(age_ms);
        Ok(self)
    }

    /// Refuse a branch-only retention field on anything but a branch.
    fn expect_branch(&self, key: &str) -> Result<()> {
        if self.is_branch() {
            return Ok(());
        }
        Err(invalid(format_smolstr!(
            "expected a branch for {key}, got a {:?}",
            crate::text::elide_to(&self.kind, 64)
        )))
    }

    /// Validate the reference kind and retention contract.
    pub(crate) fn validate(&self) -> Result<()> {
        if !self.is_branch() && !self.is_tag() {
            return Err(invalid(format_smolstr!(
                "expected a snapshot reference type of branch or tag, got {:?}",
                crate::text::elide_to(&self.kind, 64)
            )));
        }
        if let Some(count) = self.min_snapshots_to_keep {
            self.expect_branch("min-snapshots-to-keep")?;
            if count <= 0 {
                return Err(invalid(format_smolstr!(
                    "expected a positive min-snapshots-to-keep, got {count}"
                )));
            }
        }
        if let Some(age_ms) = self.max_snapshot_age_ms {
            self.expect_branch("max-snapshot-age-ms")?;
            if age_ms <= 0 {
                return Err(invalid(format_smolstr!(
                    "expected a positive max-snapshot-age-ms, got {age_ms}"
                )));
            }
        }
        if let Some(age_ms) = self.max_ref_age_ms
            && age_ms <= 0
        {
            return Err(invalid(format_smolstr!(
                "expected a positive max-ref-age-ms, got {age_ms}"
            )));
        }
        Ok(())
    }

    /// Read one reference object.
    ///
    /// # Errors
    ///
    /// Returns an error when a required value is missing or malformed.
    pub fn from_json(document: &Scalar) -> Result<Self> {
        let reference = Self {
            snapshot_id: document
                .get_key_str("snapshot-id")
                .and_then(Scalar::as_i64)
                .ok_or_else(|| {
                    invalid(SmolStr::new_static(
                        "expected a snapshot reference \"snapshot-id\"",
                    ))
                })?,
            kind: SmolStr::new(
                document
                    .get_key_str("type")
                    .and_then(Scalar::as_str)
                    .ok_or_else(|| {
                        invalid(SmolStr::new_static(
                            "expected a snapshot reference \"type\"",
                        ))
                    })?,
            ),
            min_snapshots_to_keep: optional_i32(
                document,
                "min-snapshots-to-keep",
                "snapshot reference",
            )?,
            max_snapshot_age_ms: optional_i64(
                document,
                "max-snapshot-age-ms",
                "snapshot reference",
            )?,
            max_ref_age_ms: optional_i64(document, "max-ref-age-ms", "snapshot reference")?,
        };
        reference.validate()?;
        Ok(reference)
    }

    /// Write one reference object, omitting the retention fields it does not
    /// carry.
    ///
    /// # Errors
    ///
    /// Returns an error only when the mapping cannot be built.
    pub fn into_json(self) -> Result<Scalar> {
        self.validate()?;
        let mut entries = vec![
            (Scalar::from("snapshot-id"), Scalar::from(self.snapshot_id)),
            (Scalar::from("type"), Scalar::from(self.kind.clone())),
        ];
        if let Some(count) = self.min_snapshots_to_keep {
            entries.push((
                Scalar::from("min-snapshots-to-keep"),
                Scalar::from(i64::from(count)),
            ));
        }
        if let Some(age_ms) = self.max_snapshot_age_ms {
            entries.push((Scalar::from("max-snapshot-age-ms"), Scalar::from(age_ms)));
        }
        if let Some(age_ms) = self.max_ref_age_ms {
            entries.push((Scalar::from("max-ref-age-ms"), Scalar::from(age_ms)));
        }
        Scalar::from_mapping(entries)
    }
}

fn optional_i64(document: &Scalar, key: &str, context: &str) -> Result<Option<i64>> {
    match document.get_key_str(key) {
        None => Ok(None),
        Some(value) if value.is_null() => Ok(None),
        Some(value) => value.as_i64().map(Some).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a 64-bit integer {key:?} on {context}"
            ))
        }),
    }
}

fn optional_i32(document: &Scalar, key: &str, context: &str) -> Result<Option<i32>> {
    optional_i64(document, key, context)?
        .map(|value| {
            i32::try_from(value).map_err(|_| {
                invalid(format_smolstr!(
                    "expected a 32-bit integer {key:?} on {context}, got {value}"
                ))
            })
        })
        .transpose()
}

fn optional_string(document: &Scalar, key: &str, context: &str) -> Result<Option<SmolStr>> {
    match document.get_key_str(key) {
        None => Ok(None),
        Some(value) if value.is_null() => Ok(None),
        Some(value) => value
            .as_str()
            .map(SmolStr::new)
            .map(Some)
            .ok_or_else(|| invalid(format_smolstr!("expected a string {key:?} on {context}"))),
    }
}

fn string_mapping(
    document: &Scalar,
    key: &str,
    snapshot_id: i64,
) -> Result<Vec<(SmolStr, SmolStr)>> {
    let Some(value) = document.get_key_str(key) else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    if let Some(record) = value.as_record() {
        return record
            .iter()
            .map(|(name, value)| {
                value
                    .as_str()
                    .map(|value| (name.clone(), SmolStr::new(value)))
                    .ok_or_else(|| {
                        invalid(format_smolstr!(
                            "expected a string summary value for {name:?} on snapshot {snapshot_id}"
                        ))
                    })
            })
            .collect();
    }
    let entries = value.as_mapping().ok_or_else(|| {
        invalid(format_smolstr!(
            "expected a summary object on snapshot {snapshot_id}"
        ))
    })?;
    let mut seen = HashSet::with_capacity(entries.len());
    let mut result = Vec::with_capacity(entries.len());
    for (name, value) in entries {
        let name = name.as_str().ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a string summary key on snapshot {snapshot_id}"
            ))
        })?;
        if !seen.insert(name) {
            return Err(invalid(format_smolstr!(
                "expected unique summary keys on snapshot {snapshot_id}, got {name:?} more than once"
            )));
        }
        let value = value.as_str().ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a string summary value for {name:?} on snapshot {snapshot_id}"
            ))
        })?;
        result.push((SmolStr::new(name), SmolStr::new(value)));
    }
    Ok(result)
}

/// Report a malformed Iceberg snapshot document.
fn invalid(reason: SmolStr) -> Error {
    Error::Codec {
        format: "iceberg",
        position: 0,
        reason,
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod strict_json_tests {
    use super::{Snapshot, SnapshotRef};
    use crate::iceberg::FormatVersion;

    #[test]
    fn snapshot_optional_values_are_exact_when_present() {
        for text in [
            r#"{"snapshot-id":1,"timestamp-ms":2,"manifest-list":3}"#,
            r#"{"snapshot-id":1,"timestamp-ms":2,"manifest-list":"m","schema-id":2147483648}"#,
            r#"{"snapshot-id":1,"timestamp-ms":2,"manifest-list":"m","summary":{"operation":3}}"#,
            r#"{"snapshot-id":1,"timestamp-ms":2,"manifest-list":"m","sequence-number":-1}"#,
        ] {
            let document = crate::json::from_utf8(text).unwrap();
            assert!(Snapshot::from_json(&document).is_err(), "{text}");
        }
    }

    #[test]
    fn snapshot_requires_one_manifest_location_shape() {
        let missing = crate::json::from_utf8(r#"{"snapshot-id":1,"timestamp-ms":2}"#).unwrap();
        assert!(Snapshot::from_json(&missing).is_err());

        let both = crate::json::from_utf8(
            r#"{"snapshot-id":1,"timestamp-ms":2,"manifest-list":"m","manifests":[]}"#,
        )
        .unwrap();
        assert!(Snapshot::from_json(&both).is_err());

        let v1 =
            crate::json::from_utf8(r#"{"snapshot-id":1,"timestamp-ms":2,"manifests":["m.avro"]}"#)
                .unwrap();
        let snapshot = Snapshot::from_json(&v1).unwrap();
        snapshot.validate_for_version(FormatVersion::V1).unwrap();
    }

    #[test]
    fn reference_json_requires_type_ranges_and_positive_retention() {
        for text in [
            r#"{"snapshot-id":1}"#,
            r#"{"snapshot-id":1,"type":"branch","min-snapshots-to-keep":2147483648}"#,
            r#"{"snapshot-id":1,"type":"branch","min-snapshots-to-keep":0}"#,
            r#"{"snapshot-id":1,"type":"branch","max-snapshot-age-ms":-1}"#,
            r#"{"snapshot-id":1,"type":"tag","min-snapshots-to-keep":1}"#,
            r#"{"snapshot-id":1,"type":"other"}"#,
        ] {
            let document = crate::json::from_utf8(text).unwrap();
            assert!(SnapshotRef::from_json(&document).is_err(), "{text}");
        }
    }
}
