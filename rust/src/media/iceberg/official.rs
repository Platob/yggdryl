//! Boundary to Apache `iceberg-rust` metadata.
//!
//! The official crate owns Iceberg JSON normalization and validation. Its
//! Arrow 58 types never cross this module; Yggdryl keeps Arrow 59, `IOBase`,
//! and data-file writes.

use std::collections::BTreeMap;

use iceberg_official::spec::{Schema as OfficialSchema, TableMetadata as OfficialTableMetadata};
use smol_str::{SmolStr, format_smolstr};

use crate::{Error, Result, Scalar};

/// Direct manifest paths retained by v1 snapshots while the official metadata
/// model handles every field it represents.
#[derive(Default)]
pub(super) struct V1SnapshotManifests(BTreeMap<i64, Vec<SmolStr>>);

impl V1SnapshotManifests {
    pub(super) fn insert(&mut self, snapshot_id: i64, manifests: Vec<SmolStr>) {
        self.0.insert(snapshot_id, manifests);
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn into_entries(self) -> impl Iterator<Item = (i64, Vec<SmolStr>)> {
        self.0.into_iter()
    }
}

/// Parse, normalize, and serialize one table metadata document with the
/// official implementation.
pub(super) fn normalize_table_metadata(document: &Scalar) -> Result<Scalar> {
    let (metadata, v1_manifests) = parse_table_metadata(document)?;
    table_metadata_document(&metadata, &v1_manifests)
}

/// Validate one table metadata document with the official implementation.
pub(super) fn validate_table_metadata(document: &Scalar) -> Result<()> {
    let _ = parse_table_metadata(document)?;
    Ok(())
}

/// Parse metadata through Apache Iceberg, temporarily representing the one v1
/// snapshot shape its current model rejects.
pub(super) fn parse_table_metadata(
    document: &Scalar,
) -> Result<(OfficialTableMetadata, V1SnapshotManifests)> {
    let (bridged, v1_manifests) = bridge_v1_manifests(document)?;
    let bytes = crate::text::json::into_bytes(&bridged)?;
    let metadata = serde_json::from_slice(&bytes)?;
    Ok((metadata, v1_manifests))
}

/// Serialize official metadata and restore the direct v1 manifest arrays that
/// have no lossless representation in the official model.
pub(super) fn table_metadata_document(
    metadata: &OfficialTableMetadata,
    v1_manifests: &V1SnapshotManifests,
) -> Result<Scalar> {
    let document = crate::text::json::from_bytes(&serde_json::to_vec(metadata)?)?;
    restore_v1_manifests(&document, v1_manifests)
}

/// Replace each direct v1 manifest array with a private manifest-list path for
/// the duration of an official metadata operation.
fn bridge_v1_manifests(document: &Scalar) -> Result<(Scalar, V1SnapshotManifests)> {
    let mut v1_manifests = V1SnapshotManifests::default();
    let Some(snapshots) = document.get_key_str("snapshots") else {
        return Ok((document.clone(), v1_manifests));
    };
    let Some(snapshots) = snapshots.as_sequence() else {
        return Ok((document.clone(), v1_manifests));
    };
    let mut bridged = Vec::with_capacity(snapshots.len());
    for snapshot in snapshots {
        let Some(paths) = snapshot.get_key_str("manifests") else {
            bridged.push(snapshot.clone());
            continue;
        };
        let snapshot_id = snapshot
            .get_key_str("snapshot-id")
            .and_then(Scalar::as_i64)
            .ok_or_else(|| invalid("expected a snapshot-id beside v1 direct manifests"))?;
        let paths = paths
            .as_sequence()
            .ok_or_else(|| invalid("expected v1 direct manifests to be an array"))?
            .iter()
            .enumerate()
            .map(|(index, path)| {
                path.as_str().map(SmolStr::new).ok_or_else(|| {
                    invalid(format_smolstr!(
                        "expected a string at v1 direct manifests[{index}]"
                    ))
                })
            })
            .collect::<Result<Vec<_>>>()?;
        v1_manifests.insert(snapshot_id, paths);
        let snapshot = without_name(snapshot, "manifests")?;
        bridged.push(with_name(
            &snapshot,
            "manifest-list",
            v1_manifest_list(snapshot_id),
        )?);
    }
    Ok((
        with_name(document, "snapshots", Scalar::from_sequence(bridged))?,
        v1_manifests,
    ))
}

fn restore_v1_manifests(document: &Scalar, v1_manifests: &V1SnapshotManifests) -> Result<Scalar> {
    if v1_manifests.is_empty() {
        return Ok(document.clone());
    }
    let version = document
        .get_key_str("format-version")
        .and_then(Scalar::as_i64)
        .unwrap_or_default();
    if version != 1 {
        return Err(invalid(
            "cannot upgrade v1 snapshots with direct manifests without writing manifest lists",
        ));
    }
    let Some(snapshots) = document
        .get_key_str("snapshots")
        .and_then(Scalar::as_sequence)
    else {
        return Ok(document.clone());
    };
    let mut restored = Vec::with_capacity(snapshots.len());
    for snapshot in snapshots {
        let Some(snapshot_id) = snapshot.get_key_str("snapshot-id").and_then(Scalar::as_i64) else {
            restored.push(snapshot.clone());
            continue;
        };
        let Some(paths) = v1_manifests.0.get(&snapshot_id) else {
            restored.push(snapshot.clone());
            continue;
        };
        let snapshot = without_name(snapshot, "manifest-list")?;
        restored.push(with_name(
            &snapshot,
            "manifests",
            Scalar::from_sequence(paths.iter().cloned().map(Scalar::from)),
        )?);
    }
    with_name(document, "snapshots", Scalar::from_sequence(restored))
}

pub(super) fn v1_manifest_list(snapshot_id: i64) -> String {
    format!("file:///__iceberg_v1_direct_manifests/{snapshot_id}.avro")
}

fn with_name(document: &Scalar, name: &str, value: impl Into<Scalar>) -> Result<Scalar> {
    if document.as_record().is_some() {
        document.with_field(name, value)
    } else {
        document.with_key(name, value)
    }
}

fn without_name(document: &Scalar, name: &str) -> Result<Scalar> {
    if document.as_record().is_some() {
        document.without_field(name)
    } else {
        document.without_key(name)
    }
}

fn invalid(reason: impl Into<SmolStr>) -> Error {
    Error::Codec {
        format: "iceberg",
        position: 0,
        reason: reason.into(),
    }
}

/// Parse, normalize, and serialize one schema document with the official
/// implementation.
pub(super) fn normalize_schema(document: &Scalar) -> Result<Scalar> {
    let bytes = crate::text::json::into_bytes(document)?;
    let schema: OfficialSchema = serde_json::from_slice(&bytes)?;
    let mut identifier_ids: Vec<i32> = schema.identifier_field_ids().collect();
    identifier_ids.sort_unstable();

    let normalized = crate::text::json::from_bytes(&serde_json::to_vec(&schema)?)?;
    if identifier_ids.is_empty() {
        return Ok(normalized);
    }
    normalized.with_field(
        "identifier-field-ids",
        Scalar::from_sequence(identifier_ids.into_iter().map(i64::from).map(Scalar::from)),
    )
}

/// Validate one schema document with the official implementation.
pub(super) fn validate_schema(document: &Scalar) -> Result<()> {
    let bytes = crate::text::json::into_bytes(document)?;
    let _: OfficialSchema = serde_json::from_slice(&bytes)?;
    Ok(())
}
