//! Native Node.js views over Yggdryl schema, URI, JSON, YAML, TOML, and XML values.

// JavaScript owns its arguments and observes Rust failures as exceptions.
// These signatures intentionally model the Node-API boundary. NAPI reads
// every parameter type syntactically to write the TypeScript declaration, so
// an input union is spelled out at each entry rather than through an alias.
#![allow(
    clippy::inherent_to_string,
    clippy::missing_errors_doc,
    clippy::needless_pass_by_value,
    clippy::return_self_not_must_use,
    clippy::type_complexity
)]

mod avro;
mod cast;
pub mod charset;
mod chunked_serie;
pub mod coding;
mod datatype;
// Discovered through NAPI's generated registration inventory rather than
// ordinary Rust call sites, like `uri` below.
#[allow(dead_code)]
mod enums;
mod expression;
mod field;
mod fix;
// Discovered through NAPI's generated registration inventory rather than
// ordinary Rust call sites, like `uri` below.
#[allow(dead_code)]
mod graph;
// Discovered through NAPI's generated registration inventory, like `enums`.
#[allow(dead_code)]
mod hashing;
mod holder;
mod iceberg;
mod iobase;
mod iomedia;
mod media;
mod text;
mod timezone;
// These private exports are discovered through NAPI's generated registration
// inventory rather than ordinary Rust call sites.
mod serie;
#[allow(dead_code)]
mod uri;
mod value;
mod version;

use std::cmp::Ordering;
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;

use napi::bindgen_prelude::{Env, Error, FromNapiValue, Function, FunctionRef, Generator, Status};
use napi_derive::napi;
use yggdryl::{Error as CoreError, OwnedDifferences};

pub use avro::{
    AvroDecodeLimitsInput, JsAvroBlock, JsAvroBlocks, JsAvroSchema, avro_blocks_native,
    avro_dumps_native, avro_loads_native,
};
pub use cast::JsArrowCastPlan;
pub use chunked_serie::JsChunkedSerie;
pub use datatype::JsDataType;
pub use enums::{JsMediaType, JsMimeType};
pub use expression::{
    ExpressionVocabularies, JsBound, JsBoundSelector, JsExpression, JsFilter, JsPlan, JsRecords,
    JsSelector, JsTerm, PartitionSplit, PlanOrder, expression_needs_quoting,
    expression_vocabularies,
};
pub use field::{JsField, JsProtocolField, MetadataEntry};
pub use fix::{
    FixCaptureView, FixCodecOptions, FixEntryView, FixHeaderView, JsFixCodec, JsFixFieldIterator,
    JsFixMessages, JsFixMsg, JsFixRegistry, JsMsgType, fix_crate_fields, fix_global_registry,
    fix_install_global_registry, fix_schema, fix_schema_carrying, fix_schema_tags,
    fix_ulbridge_rowheader_native,
};
pub use graph::{
    BookRefInput, JsBookEvent, JsBookIterator, JsBookRef, JsBookSide, JsEventIterator, JsExecution,
    JsExecutionEvent, JsLane, JsMarketData, JsMarketDataRowIterator, JsOrder, JsOrderEvent,
    JsQuote, JsQuoteEvent, JsSnapshotEvent, JsSnapshotPartition, JsTradeEvent, LaneInput,
    SnapshotPartitionInput, graph_entry_id_native, graph_entry_ref_id_native,
    graph_followed_altids_native, graph_global_symbol_native,
};
pub use holder::fs::{ArrowFileInfo, FileSelector};
pub use iceberg::{
    FieldBound, FieldCount, FieldSummaryView, IcebergOptionsInput, JsCatalog, JsCompaction,
    JsDataFile, JsIcebergOptions, JsManifestFile, JsNamespace, JsNamespaces, JsPartitionField,
    JsPartitionSpec, JsScanPlan, JsSchemaUpdate, JsSnapshot, JsSnapshotRef, JsTable, JsTables,
    iceberg_assign_field_ids, iceberg_can_promote, iceberg_schema_from_json,
    iceberg_schema_into_json,
};
pub use iobase::{JsFsByteReader, JsFsByteWriter, JsFsRandomAccessReader, JsIOBase};
pub use iomedia::JsBatchReader;
pub use media::options::JsRecordOptions;
pub use serie::{JsSerie, JsSerieIterator, JsSerieReader};
pub use text::codec::{
    CodecLimitsInput, JsScalar, JsScalarIterator, codec_infer_format, codec_loads_inferred_native,
    codec_normalize_format, json_dump_path_native, json_dumps_native, json_lines_dump_all_native,
    json_lines_dump_path_native, json_lines_load_path_native, json_lines_loads_native,
    json_load_path_native, json_loads_native, toml_dump_path_native, toml_dumps_native,
    toml_load_path_native, toml_loads_native, xml_dump_path_native, xml_dumps_native,
    xml_load_path_native, xml_loads_native, yaml_dump_all_native, yaml_dump_all_path_native,
    yaml_dump_path_native, yaml_dumps_native, yaml_load_all_path_native, yaml_load_path_native,
    yaml_loads_all_native, yaml_loads_native,
};
pub use text::options::JsTextOptions;
pub use timezone::{JsTimezone, TimezoneAlias};
pub use uri::{JsArn, JsUri, JsUrl, JsUrn, PartitionEntry};
pub use version::JsVersion;

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

/// Coerce the two cast answers JavaScript spells separately into one native
/// value.
///
/// `safe` decides whether a present value may be converted; `representation`
/// names what a same-width pair carries. Whether a value may be absent is not
/// an option here: it is the target field's own nullability. Both answers
/// cross explicitly on every cast entry point, so neither is inferred from
/// the other.
pub(crate) fn cast_options(
    safe: Option<bool>,
    representation: Option<&str>,
) -> napi::Result<yggdryl::ArrowCastOptions> {
    let representation = match representation {
        Some(value) => yggdryl::Representation::from_str(value).map_err(napi_error)?,
        None => yggdryl::Representation::Value,
    };
    Ok(yggdryl::ArrowCastOptions::new()
        .with_safe(safe.unwrap_or(true))
        .with_representation(representation))
}

pub(crate) fn napi_error(error: impl std::fmt::Display) -> Error {
    Error::from_reason(error.to_string())
}

/// One fact answered, or `null` where the core holds nothing.
///
/// A plain object states every fact it declares: an absent one is `null`,
/// as every absence at this boundary is, rather than a property left out.
pub(crate) fn or_null<T>(
    value: Option<T>,
) -> napi::bindgen_prelude::Either<T, napi::bindgen_prelude::Null> {
    use napi::bindgen_prelude::{Either, Null};
    value.map_or(Either::B(Null), Either::A)
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

pub(crate) fn exact_u32(value: f64, name: &str) -> napi::Result<u32> {
    if !value.is_finite() || value.fract() != 0.0 || value < 0.0 || value > f64::from(u32::MAX) {
        return Err(Error::from_reason(format!(
            "{name} must be an unsigned 32-bit integer"
        )));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(value as u32)
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

/// Where a JavaScript source behind a stream failed, kept for the stream.
///
/// A core stage pulls its items as values, so a failure inside the pull - an
/// item the loader could not read, an iterator that threw - cannot travel
/// through the stage as an item. It ends the pull instead and lands here, as
/// the status and reason a new error is raised with, because the error
/// itself holds handles that do not cross threads.
#[derive(Clone, Default)]
pub(crate) struct Failed(Arc<Mutex<Option<(Status, String)>>>);

impl Failed {
    /// Record `error`'s status and reason, overwriting whatever was held.
    pub(crate) fn set(&self, error: &napi::Error) {
        if let Ok(mut held) = self.0.lock() {
            *held = Some((error.status, error.reason.clone()));
        }
    }

    /// Take the held failure, once, where the pull suffered one.
    pub(crate) fn take(&self) -> Option<napi::Error> {
        self.0
            .lock()
            .ok()
            .and_then(|mut held| held.take())
            .map(|(status, reason)| napi::Error::new(status, reason))
    }

    /// Clone the held failure without consuming it, for a stage that can
    /// only carry a typed error onward - a sentinel it wraps, travels
    /// through the stage as one, and unwraps back at the surface - while
    /// `take` still answers the original failure itself once that surface
    /// asks for it.
    pub(crate) fn peek(&self) -> Option<napi::Error> {
        self.0
            .lock()
            .ok()
            .and_then(|held| held.clone())
            .map(|(status, reason)| napi::Error::new(status, reason))
    }
}

/// A JavaScript iterable pulled one item at a time into a core stage.
///
/// The loader hands over a bound pull function, `() => item | null`, so the
/// iterable's own protocol runs in JavaScript and each item arrives here
/// already read into the value the stage takes. Like the record bridge in
/// `iomedia`, it is called only on the isolate thread that supplied it and
/// only within the native call advancing the stream, which is what makes the
/// environment it is borrowed back with valid. Exhaustion ends the pull; a
/// failure ends it too and lands in `failed`, so the stage sees a shorter
/// stream and the wrapper around it throws what happened.
pub(crate) struct Pulled<T: FromNapiValue> {
    pull: FunctionRef<(), Option<T>>,
    environment: usize,
    thread: ThreadId,
    /// Where the source failed, cloned out before the pull is chained past
    /// its exhaustion.
    pub(crate) failed: Failed,
    done: bool,
}

impl<T: FromNapiValue> Pulled<T> {
    /// Hold `pull`, bound to the isolate `env` supplied it.
    pub(crate) fn new(env: Env, pull: Function<'_, (), Option<T>>) -> napi::Result<Self> {
        Ok(Self {
            pull: pull.create_ref()?,
            environment: env.raw().expose_provenance(),
            thread: std::thread::current().id(),
            failed: Failed::default(),
            done: false,
        })
    }

    fn pull(&self) -> napi::Result<Option<T>> {
        if std::thread::current().id() != self.thread {
            return Err(napi_error(
                "a JavaScript iterable can only be pulled on the isolate thread that supplied it",
            ));
        }
        let env = Env::from_raw(std::ptr::with_exposed_provenance_mut(self.environment));
        self.pull.borrow_back(&env)?.call(())
    }
}

impl<T: FromNapiValue> Iterator for Pulled<T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        if self.done {
            return None;
        }
        match self.pull() {
            Ok(Some(value)) => Some(value),
            Ok(None) => {
                self.done = true;
                None
            }
            Err(error) => {
                self.done = true;
                self.failed.set(&error);
                None
            }
        }
    }
}

/// A JavaScript failure behind a stream a batch reader pulls, as the core
/// reports one.
///
/// The batch it would have landed in is being pulled by the reader rather
/// than by a JavaScript frame, so it travels as that reader's error and
/// arrives where the batch would have.
pub(crate) fn javascript_failure(error: napi::Error) -> CoreError {
    CoreError::Arrow(arrow_schema::ArrowError::ExternalError(Box::new(
        std::io::Error::other(error.reason.clone()),
    )))
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
