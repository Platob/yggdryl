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
//! has no variant for - travel as a placeholder width of their own
//! ([`UNKNOWN_PLACEHOLDER`], [`VARIANT_PLACEHOLDER`]). Both are restored on
//! the way back by their spelling alone, so the crate's own schema serde is
//! what spells the two names and the official model still validates
//! everything else about the column: its identifier, its name, its
//! requiredness, its defaults, and its place in the tree.

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

/// What an `unknown` column crosses the official boundary as.
///
/// The official model reads any `fixed[n]` as a primitive, its width a `u64`,
/// and checks nothing about a column that the width would answer. A width
/// past `u32::MAX` is one no document this crate reads can state, so each
/// bridged type takes one of those of its own: the official model then sees
/// `unknown`, `variant` and `binary` as three types - a schema changing one
/// into another is a new schema, and its promotion rules judge the change -
/// and the spelling alone says which type a slot stands for, so a schema the
/// official builder adds restores like one it read.
const UNKNOWN_PLACEHOLDER: &str = "fixed[18446744073709551615]";

/// What a `variant` column crosses the official boundary as.
///
/// [`UNKNOWN_PLACEHOLDER`] carries the rule.
const VARIANT_PLACEHOLDER: &str = "fixed[18446744073709551614]";

/// The two placeholder widths, as the official model reads them.
const PLACEHOLDER_WIDTHS: [u64; 2] = [u64::MAX, u64::MAX - 1];

/// The placeholder one v3 type name crosses as, or `None` for every other
/// name.
///
/// A name that would read back as a placeholder is refused rather than
/// passed through, or a document spelling that width would come back
/// spelling `unknown` or `variant`.
fn bridged_spelling(name: &str) -> Result<Option<&'static str>> {
    match name {
        "unknown" => return Ok(Some(UNKNOWN_PLACEHOLDER)),
        "variant" => return Ok(Some(VARIANT_PLACEHOLDER)),
        _ => {}
    }
    // The official reading of a width, trimmed and parsed as its own
    // deserializer does: what it reads is what it would write back.
    if let Some(width) = name
        .starts_with("fixed")
        .then(|| name.trim_start_matches("fixed[").trim_end_matches(']'))
        .and_then(|width| width.parse::<u64>().ok())
        && PLACEHOLDER_WIDTHS.contains(&width)
    {
        return Err(invalid(format_smolstr!(
            "expected a fixed width of at most {}, got {name:?}",
            u32::MAX
        )));
    }
    Ok(None)
}

/// The v3 type name one placeholder stands for, or `None` for every other
/// name.
fn restored_spelling(name: &str) -> Option<&'static str> {
    match name {
        UNKNOWN_PLACEHOLDER => Some("unknown"),
        VARIANT_PLACEHOLDER => Some("variant"),
        _ => None,
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

/// Parse metadata through Apache Iceberg, temporarily representing the v1
/// snapshot shape and the v3 column types its current model rejects.
pub(super) fn parse_table_metadata(
    document: &Scalar,
) -> Result<(OfficialTableMetadata, V1SnapshotManifests)> {
    let (bridged, v1_manifests) = bridge_v1_manifests(document)?;
    let bridged = rewrite_schemas(&bridged, &mut bridge_schema)?;
    let bytes = crate::json::into_bytes(&bridged)?;
    let metadata = serde_json::from_slice(&bytes)?;
    Ok((metadata, v1_manifests))
}

/// Serialize official metadata and restore the direct v1 manifest arrays and
/// the v3 column types that have no representation in the official model.
pub(super) fn table_metadata_document(
    metadata: &OfficialTableMetadata,
    v1_manifests: &V1SnapshotManifests,
) -> Result<Scalar> {
    let document = crate::json::from_bytes(&serde_json::to_vec(metadata)?)?;
    let document = restore_v1_manifests(&document, v1_manifests)?;
    rewrite_schemas(&document, &mut restore_schema)
}

/// Rewrite every schema a metadata document holds - its `schemas` and the
/// v1 `schema` - through one schema rewrite.
fn rewrite_schemas(
    document: &Scalar,
    rewrite: &mut dyn FnMut(&Scalar) -> Result<Scalar>,
) -> Result<Scalar> {
    let mut rewritten = document.clone();
    if let Some(schemas) = document.get_key_str("schemas").and_then(Scalar::as_serie) {
        let mut replaced = Vec::with_capacity(schemas.len());
        for schema in schemas.iter() {
            replaced.push(rewrite(&schema)?);
        }
        rewritten = with_name(&rewritten, "schemas", Scalar::from_sequence(replaced))?;
    }
    if let Some(schema) = document.get_key_str("schema") {
        let replaced = rewrite(schema)?;
        rewritten = with_name(&rewritten, "schema", replaced)?;
    }
    Ok(rewritten)
}

/// Bridge one schema object: each v3 type goes in as its placeholder.
fn bridge_schema(schema: &Scalar) -> Result<Scalar> {
    walk_v3_types(schema, &mut bridged_spelling)
}

/// Restore one schema object: each placeholder comes back as its v3 type.
fn restore_schema(schema: &Scalar) -> Result<Scalar> {
    walk_v3_types(schema, &mut |name| Ok(restored_spelling(name)))
}

/// Rewrite the bare type names of one struct object's field tree.
///
/// `rename` sees every typed slot's bare type name - a field's, a list
/// element's, a map key's or value's - and answers the name to write in its
/// place, or `None` to leave the slot as it is. Nested structs recurse; an
/// object without `fields` is returned untouched.
fn walk_v3_types(
    object: &Scalar,
    rename: &mut dyn FnMut(&str) -> Result<Option<&'static str>>,
) -> Result<Scalar> {
    let Some(fields) = object.get_key_str("fields").and_then(Scalar::as_serie) else {
        return Ok(object.clone());
    };
    let mut replaced = Vec::with_capacity(fields.len());
    for field in fields.iter() {
        let Some(type_json) = field.get_key_str("type") else {
            replaced.push(field.into_owned());
            continue;
        };
        let type_json = walk_v3_type(type_json, rename)?;
        replaced.push(with_name(&field, "type", type_json)?);
    }
    with_name(object, "fields", Scalar::from_sequence(replaced))
}

/// Rewrite one typed slot, recursing into the nested types.
fn walk_v3_type(
    type_json: &Scalar,
    rename: &mut dyn FnMut(&str) -> Result<Option<&'static str>>,
) -> Result<Scalar> {
    if let Some(name) = type_json.as_str() {
        return Ok(match rename(name)? {
            Some(renamed) => Scalar::from(renamed),
            None => type_json.clone(),
        });
    }
    match type_json.get_key_str("type").and_then(Scalar::as_str) {
        Some("struct") => walk_v3_types(type_json, rename),
        Some("list") => {
            let Some(element) = type_json.get_key_str("element") else {
                return Ok(type_json.clone());
            };
            let element = walk_v3_type(element, rename)?;
            with_name(type_json, "element", element)
        }
        Some("map") => {
            let mut rewritten = type_json.clone();
            if let Some(key) = type_json.get_key_str("key") {
                rewritten = with_name(&rewritten, "key", walk_v3_type(key, rename)?)?;
            }
            if let Some(value) = type_json.get_key_str("value") {
                rewritten = with_name(&rewritten, "value", walk_v3_type(value, rename)?)?;
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
    let Some(snapshots) = snapshots.as_serie() else {
        return Ok((document.clone(), v1_manifests));
    };
    let mut bridged = Vec::with_capacity(snapshots.len());
    for snapshot in snapshots.iter() {
        let Some(paths) = snapshot.get_key_str("manifests") else {
            bridged.push(snapshot.into_owned());
            continue;
        };
        let snapshot_id = snapshot
            .get_key_str("snapshot-id")
            .and_then(Scalar::as_i64)
            .ok_or_else(|| invalid("expected a snapshot-id beside v1 direct manifests"))?;
        let paths = paths
            .as_serie()
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
        let snapshot = without_name(&snapshot, "manifests")?;
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
    let bridged = bridge_schema(document)?;
    let bytes = crate::json::into_bytes(&bridged)?;
    let schema: OfficialSchema = serde_json::from_slice(&bytes)?;
    let mut identifier_ids: Vec<i32> = schema.identifier_field_ids().collect();
    identifier_ids.sort_unstable();

    let normalized = crate::json::from_bytes(&serde_json::to_vec(&schema)?)?;
    let normalized = restore_schema(&normalized)?;
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
    let mut renamed = false;
    let bridged = walk_v3_types(document, &mut |name| {
        let spelling = bridged_spelling(name)?;
        renamed |= spelling.is_some();
        Ok(spelling)
    })?;
    Ok(renamed.then_some(bridged))
}

/// Parse one schema document into the official model, the two v3 types
/// bridged as placeholders.
///
/// What comes back is the official crate's own reading, so a caller asking it
/// about a bridged column sees its placeholder width; every other question -
/// identifiers, names, requiredness, defaults, nesting - is answered as it
/// is.
pub(super) fn parse_schema(document: &Scalar) -> Result<OfficialSchema> {
    let bridged = bridge_schema(document)?;
    let bytes = crate::json::into_bytes(&bridged)?;
    Ok(serde_json::from_slice(&bytes)?)
}
