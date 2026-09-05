//! Native Node.js views over Yggdryl schema, URI, JSON, YAML, and TOML values.

// JavaScript owns its arguments and observes Rust failures as exceptions.
// These signatures intentionally model the Node-API boundary.
#![allow(
    clippy::inherent_to_string,
    clippy::missing_errors_doc,
    clippy::needless_pass_by_value,
    clippy::return_self_not_must_use
)]

pub mod coding;
// Discovered through NAPI's generated registration inventory rather than
// ordinary Rust call sites, like `uri` below.
#[allow(dead_code)]
mod enums;
mod expression;
mod fix;
mod holder;
mod iobase;
mod iomedia;
mod media;
mod text;
mod types;
// These private exports are discovered through NAPI's generated registration
// inventory rather than ordinary Rust call sites.
#[allow(dead_code)]
mod uri;
// Discovered through NAPI's generated registration inventory, like `enums`.
#[allow(dead_code)]
mod xxhash;

use std::cmp::Ordering;

use napi::bindgen_prelude::{Env, Error, Generator};
use napi_derive::napi;
use yggdryl::OwnedDifferences;

pub use enums::{JsMediaType, JsMimeType};
pub use expression::{
    BoundStatementOrder, JsBound, JsBoundStatement, JsExpression, JsStatement, StatementOrder,
};
pub use fix::{
    JsFixFieldIterator, JsFixMsg, JsFixMsgEntries, JsFixRegistry, fix_global_registry,
    fix_install_global_registry, fix_standard_branch_native, fix_standard_tag_limit_native,
};
pub use holder::fs::{ArrowFileInfo, FileSelector};
pub use iobase::{JsFsByteReader, JsFsByteWriter, JsFsRandomAccessReader, JsIOBase};
pub use iomedia::JsBatchReader;
pub use media::avro::{
    AvroDecodeLimitsInput, JsAvroBlock, JsAvroBlocks, JsAvroSchema, avro_blocks_native,
    avro_dumps_native, avro_loads_native,
};
pub use media::iceberg::{
    FieldBound, FieldCount, FieldSummaryView, IcebergOptionsInput, JsCatalog, JsCompaction,
    JsDataFile, JsIcebergOptions, JsManifestFile, JsNamespace, JsNamespaces, JsPartitionField,
    JsPartitionSpec, JsScanPlan, JsSchemaUpdate, JsSnapshot, JsSnapshotRef, JsTable, JsTables,
    iceberg_assign_field_ids, iceberg_can_promote, iceberg_schema_from_json,
    iceberg_schema_into_json,
};
pub use media::options::JsRecordOptions;
pub use media::text::JsTextOptions;
pub use text::codec::{
    CodecLimitsInput, JsScalar, JsScalarIterator, codec_infer_format, codec_loads_inferred_native,
    codec_normalize_format, json_dump_path_native, json_dumps_native, json_lines_dump_all_native,
    json_lines_dump_path_native, json_lines_load_path_native, json_lines_loads_native,
    json_load_path_native, json_loads_native, toml_dump_path_native, toml_dumps_native,
    toml_load_path_native, toml_loads_native, yaml_dump_all_native, yaml_dump_all_path_native,
    yaml_dump_path_native, yaml_dumps_native, yaml_load_all_path_native, yaml_load_path_native,
    yaml_loads_all_native, yaml_loads_native,
};
pub use types::datatype::JsDataType;
pub use types::field::{JsField, JsProtocolField, MetadataEntry};
pub use types::timezone::{JsTimezone, TimezoneAlias};
pub use uri::{JsUri, JsUrl, JsUrn, PartitionEntry};

/// Read a structural JSON document from the object or the text a caller holds.
///
/// `JSON.parse`'s object arrives as the document itself; a string arrives as a
/// JSON string value, which is the one shape that still needs parsing. Bytes
/// have their own reader, because napi cannot discriminate a typed array
/// inside a union.
pub(crate) fn json_document(value: serde_json::Value) -> serde_json::Result<serde_json::Value> {
    match value {
        serde_json::Value::String(text) => serde_json::from_str(&text),
        document => Ok(document),
    }
}

pub(crate) fn napi_error(error: impl std::fmt::Display) -> Error {
    Error::from_reason(error.to_string())
}

/// Throw a real JavaScript `TypeError`, then report the pending exception.
///
/// An `Error` returned from a binding method always arrives as a plain
/// `Error`, whatever status it carries, so a refusal a caller tells apart by
/// class has to throw that class itself and report that an exception is
/// already pending.
pub(crate) fn napi_type_error(env: Env, reason: String) -> Error {
    match env.throw_type_error(&reason, None) {
        Ok(()) => Error::new(napi::Status::PendingException, reason),
        Err(error) => error,
    }
}

pub(crate) fn ordering_value(ordering: Ordering) -> i32 {
    match ordering {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

pub(crate) fn exact_i32(value: f64, name: &str) -> napi::Result<i32> {
    if !value.is_finite()
        || value.fract() != 0.0
        || value < f64::from(i32::MIN)
        || value > f64::from(i32::MAX)
    {
        return Err(Error::from_reason(format!(
            "{name} must be a signed 32-bit integer"
        )));
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(value as i32)
}

pub(crate) fn exact_i128(value: &napi::bindgen_prelude::BigInt, name: &str) -> napi::Result<i128> {
    let (parsed, lossless) = value.get_i128();
    if !lossless {
        return Err(Error::from_reason(format!(
            "{name} must be a signed 128-bit integer"
        )));
    }
    Ok(parsed)
}

pub(crate) fn exact_i8(value: f64, name: &str) -> napi::Result<i8> {
    if !value.is_finite()
        || value.fract() != 0.0
        || value < f64::from(i8::MIN)
        || value > f64::from(i8::MAX)
    {
        return Err(Error::from_reason(format!(
            "{name} must be a signed 8-bit integer"
        )));
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(value as i8)
}

pub(crate) fn exact_i64(value: f64, name: &str) -> napi::Result<i64> {
    // A JavaScript number carries every integer up to 2^53 exactly, so anything
    // past that is a value the caller has already lost rather than one this
    // boundary may silently truncate.
    if !value.is_finite() || value.fract() != 0.0 || value.abs() > 9_007_199_254_740_992.0 {
        return Err(Error::from_reason(format!(
            "{name} must be a whole number of at most 2^53"
        )));
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(value as i64)
}

pub(crate) fn exact_u64(value: f64, name: &str) -> napi::Result<u64> {
    if !value.is_finite()
        || value.fract() != 0.0
        || !(0.0..=9_007_199_254_740_992.0).contains(&value)
    {
        return Err(Error::from_reason(format!(
            "{name} must be a non-negative whole number of at most 2^53"
        )));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(value as u64)
}

pub(crate) fn exact_f64(value: u64, name: &str) -> napi::Result<f64> {
    if value > 9_007_199_254_740_992 {
        return Err(Error::from_reason(format!(
            "{name} cannot be represented exactly as a JavaScript number; expected at most 2^53"
        )));
    }
    #[allow(clippy::cast_precision_loss)]
    Ok(value as f64)
}

pub(crate) fn exact_u8(value: f64, name: &str) -> napi::Result<u8> {
    if !value.is_finite() || value.fract() != 0.0 || value < 0.0 || value > f64::from(u8::MAX) {
        return Err(Error::from_reason(format!(
            "{name} must be an unsigned 8-bit integer"
        )));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(value as u8)
}

/// Snapshot iterator over stable native schema-difference lines.
#[napi(iterator, js_name = "DifferenceIterator")]
pub struct JsDifferenceIterator {
    inner: OwnedDifferences,
}

impl JsDifferenceIterator {
    pub(crate) fn from_fields(
        left: &yggdryl::Field,
        right: &yggdryl::Field,
        with_metadata: bool,
        return_equal: bool,
    ) -> Self {
        Self {
            inner: OwnedDifferences::from_fields(left, right, with_metadata, return_equal),
        }
    }

    pub(crate) fn from_dtypes(
        left: &yggdryl::DataType,
        right: &yggdryl::DataType,
        with_metadata: bool,
        return_equal: bool,
    ) -> Self {
        Self {
            inner: OwnedDifferences::from_dtypes(left, right, with_metadata, return_equal),
        }
    }
}

impl Generator for JsDifferenceIterator {
    type Yield = String;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<Self::Next>) -> Option<Self::Yield> {
        self.inner.next()
    }
}
