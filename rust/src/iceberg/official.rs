//! Boundary to Apache `iceberg-rust` metadata.
//!
//! The official crate owns Iceberg JSON normalization and validation. Its
//! Arrow 58 types never cross this module; Yggdryl keeps Arrow 59, `IOBase`,
//! and data-file writes.
//!
//! Two shapes the official 0.10.1 model does not represent are bridged
//! across it rather than refused: a v1 snapshot's direct `manifests` array
//! travels as a private manifest-list path ([`V1SnapshotManifests`]), and
//! the v3 column types `unknown` and `variant` - which its `PrimitiveType`
//! has no variant for - travel as `binary` under their own field identifiers
//! ([`V3Types`]). Both are restored on the way back, so the crate's own
//! schema serde is what spells the two names and the official model still
//! validates everything else about the column: its identifier, its name,
//! its requiredness, and its place in the tree.

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

/// The v3 column types the official model has no spelling for, keyed by the
/// schema they were read from and the identifier of the slot that held them.
///
/// The slot is a field's `id`, a list's `element-id`, or a map's `key-id` or
/// `value-id`. The schema half of the key is what keeps a promotion honest: a
/// later schema that promoted an `unknown` column to `binary` - which v3
/// allows, `unknown` promotes to anything - must read back as `binary`, and
/// only the schema that spelled `unknown` gets it back.
#[derive(Default)]
pub(super) struct V3Types(BTreeMap<(Option<i64>, i64), SmolStr>);

impl V3Types {
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What a bridged v3 type crosses the official boundary as.
///
/// A binary placeholder is accepted wherever a primitive is, and the two
/// bridged types are never partition sources, sort sources, or identifier
/// columns, so nothing the official model validates reads the placeholder's
/// own semantics.
const V3_PLACEHOLDER: &str = "binary";

/// Return whether one bare type name is a v3 spelling the official crate
/// cannot parse.
fn is_bridged_v3_type(name: &str) -> bool {
    matches!(name, "unknown" | "variant")
}

/// Parse, normalize, and serialize one table metadata document with the
/// official implementation.
pub(super) fn normalize_table_metadata(document: &Scalar) -> Result<Scalar> {
    let (metadata, v1_manifests, v3_types) = parse_table_metadata(document)?;
    table_metadata_document(&metadata, &v1_manifests, &v3_types)
}

/// Validate one table metadata document with the official implementation.
pub(super) fn validate_table_metadata(document: &Scalar) -> Result<()> {
    let _ = parse_table_metadata(document)?;
    Ok(())
}

/// Parse metadata through Apache Iceberg, temporarily representing the v1
/// snapshot shape and the v3 column types its current model rejects.
pub(super) fn parse_table_metadata(
    document: &Scalar,
) -> Result<(OfficialTableMetadata, V1SnapshotManifests, V3Types)> {
    let (bridged, v1_manifests) = bridge_v1_manifests(document)?;
    let (bridged, v3_types) = bridge_v3_types(&bridged)?;
    let bytes = crate::json::into_bytes(&bridged)?;
    let metadata = serde_json::from_slice(&bytes)?;
    Ok((metadata, v1_manifests, v3_types))
}

/// Serialize official metadata and restore the direct v1 manifest arrays and
/// the v3 column types that have no representation in the official model.
pub(super) fn table_metadata_document(
    metadata: &OfficialTableMetadata,
    v1_manifests: &V1SnapshotManifests,
    v3_types: &V3Types,
) -> Result<Scalar> {
    let document = crate::json::from_bytes(&serde_json::to_vec(metadata)?)?;
    let document = restore_v1_manifests(&document, v1_manifests)?;
    restore_v3_types(&document, v3_types)
}

/// Replace every `unknown` and `variant` column of every schema in a metadata
/// document with the placeholder, remembering where each was.
fn bridge_v3_types(document: &Scalar) -> Result<(Scalar, V3Types)> {
    let mut types = V3Types::default();
    let mut bridged = document.clone();
    if let Some(schemas) = document
        .get_key_str("schemas")
        .and_then(Scalar::as_sequence)
    {
        let mut replaced = Vec::with_capacity(schemas.len());
        for schema in schemas {
            replaced.push(bridge_schema(schema, &mut types)?);
        }
        bridged = with_name(&bridged, "schemas", Scalar::from_sequence(replaced))?;
    }
    if let Some(schema) = document.get_key_str("schema") {
        let replaced = bridge_schema(schema, &mut types)?;
        bridged = with_name(&bridged, "schema", replaced)?;
    }
    Ok((bridged, types))
}

/// Put every remembered `unknown` and `variant` back in place.
fn restore_v3_types(document: &Scalar, types: &V3Types) -> Result<Scalar> {
    if types.is_empty() {
        return Ok(document.clone());
    }
    let mut restored = document.clone();
    if let Some(schemas) = document
        .get_key_str("schemas")
        .and_then(Scalar::as_sequence)
    {
        let mut replaced = Vec::with_capacity(schemas.len());
        for schema in schemas {
            replaced.push(restore_schema(schema, types)?);
        }
        restored = with_name(&restored, "schemas", Scalar::from_sequence(replaced))?;
    }
    if let Some(schema) = document.get_key_str("schema") {
        let replaced = restore_schema(schema, types)?;
        restored = with_name(&restored, "schema", replaced)?;
    }
    Ok(restored)
}

/// Bridge one schema object: the placeholder goes in, the spelling is kept.
fn bridge_schema(schema: &Scalar, types: &mut V3Types) -> Result<Scalar> {
    let schema_id = schema.get_key_str("schema-id").and_then(Scalar::as_i64);
    walk_v3_types(schema, &mut |id, name| {
        if is_bridged_v3_type(name) {
            types.0.insert((schema_id, id), SmolStr::new(name));
            return Some(SmolStr::new_static(V3_PLACEHOLDER));
        }
        None
    })
}

/// Restore one schema object: the spelling comes back where it was.
fn restore_schema(schema: &Scalar, types: &V3Types) -> Result<Scalar> {
    let schema_id = schema.get_key_str("schema-id").and_then(Scalar::as_i64);
    walk_v3_types(schema, &mut |id, name| {
        if name != V3_PLACEHOLDER {
            return None;
        }
        types.0.get(&(schema_id, id)).cloned()
    })
}

/// Rewrite the bare type names of one struct object's field tree.
///
/// `rename` sees every typed slot with its identifier - a field, a list
/// element, a map key or value - and answers the name to write in its place,
/// or `None` to leave the slot as it is. Nested structs recurse; an object
/// without `fields` is returned untouched.
fn walk_v3_types(
    object: &Scalar,
    rename: &mut dyn FnMut(i64, &str) -> Option<SmolStr>,
) -> Result<Scalar> {
    let Some(fields) = object.get_key_str("fields").and_then(Scalar::as_sequence) else {
        return Ok(object.clone());
    };
    let mut replaced = Vec::with_capacity(fields.len());
    for field in fields {
        let (Some(id), Some(type_json)) = (
            field.get_key_str("id").and_then(Scalar::as_i64),
            field.get_key_str("type"),
        ) else {
            replaced.push(field.clone());
            continue;
        };
        let type_json = walk_v3_type(type_json, id, rename)?;
        replaced.push(with_name(field, "type", type_json)?);
    }
    with_name(object, "fields", Scalar::from_sequence(replaced))
}

/// Rewrite one typed slot, recursing into the nested types.
fn walk_v3_type(
    type_json: &Scalar,
    id: i64,
    rename: &mut dyn FnMut(i64, &str) -> Option<SmolStr>,
) -> Result<Scalar> {
    if let Some(name) = type_json.as_str() {
        return Ok(match rename(id, name) {
            Some(renamed) => Scalar::from(renamed),
            None => type_json.clone(),
        });
    }
    match type_json.get_key_str("type").and_then(Scalar::as_str) {
        Some("struct") => walk_v3_types(type_json, rename),
        Some("list") => {
            let (Some(element_id), Some(element)) = (
                type_json.get_key_str("element-id").and_then(Scalar::as_i64),
                type_json.get_key_str("element"),
            ) else {
                return Ok(type_json.clone());
            };
            let element = walk_v3_type(element, element_id, rename)?;
            with_name(type_json, "element", element)
        }
        Some("map") => {
            let mut rewritten = type_json.clone();
            if let (Some(key_id), Some(key)) = (
                type_json.get_key_str("key-id").and_then(Scalar::as_i64),
                type_json.get_key_str("key"),
            ) {
                rewritten = with_name(&rewritten, "key", walk_v3_type(key, key_id, rename)?)?;
            }
            if let (Some(value_id), Some(value)) = (
                type_json.get_key_str("value-id").and_then(Scalar::as_i64),
                type_json.get_key_str("value"),
            ) {
                rewritten = with_name(&rewritten, "value", walk_v3_type(value, value_id, rename)?)?;
            }
            Ok(rewritten)
        }
        _ => Ok(type_json.clone()),
    }
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
    if document.as_struct().is_some() {
        document.with_field(name, value)
    } else {
        document.with_key(name, value)
    }
}

fn without_name(document: &Scalar, name: &str) -> Result<Scalar> {
    if document.as_struct().is_some() {
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
    let mut types = V3Types::default();
    let bridged = bridge_schema(document, &mut types)?;
    let bytes = crate::json::into_bytes(&bridged)?;
    let schema: OfficialSchema = serde_json::from_slice(&bytes)?;
    let mut identifier_ids: Vec<i32> = schema.identifier_field_ids().collect();
    identifier_ids.sort_unstable();

    let normalized = crate::json::from_bytes(&serde_json::to_vec(&schema)?)?;
    let normalized = restore_schema(&normalized, &types)?;
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
    parse_schema(document).map(|_| ())
}

/// The placeholder reading of one schema document, when it needs one.
///
/// A manifest carries the table schema in its Avro header, and the official
/// manifest reader parses it; this is what that reader is handed instead of
/// a document spelling `unknown` or `variant`. `None` says the document
/// already reads as it is.
pub(super) fn bridged_schema(document: &Scalar) -> Result<Option<Scalar>> {
    let mut types = V3Types::default();
    let bridged = bridge_schema(document, &mut types)?;
    Ok((!types.is_empty()).then_some(bridged))
}

/// Parse one schema document into the official model, the two v3 types
/// bridged as placeholders.
///
/// What comes back is the official crate's own reading, so a caller asking it
/// about a bridged column sees `binary`; every other question - identifiers,
/// names, requiredness, defaults, nesting - is answered as it is.
pub(super) fn parse_schema(document: &Scalar) -> Result<OfficialSchema> {
    let bridged = bridge_schema(document, &mut V3Types::default())?;
    let bytes = crate::json::into_bytes(&bridged)?;
    Ok(serde_json::from_slice(&bytes)?)
}
