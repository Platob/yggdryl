//! JavaScript's view of the core [`IOResult`]: immutable, compared and
//! hashed by its three counts; `add` is the core's `Add`, `toString` its
//! `Display`, and a count crosses as a `number`, as every row count does.

use napi::bindgen_prelude::Result;
use napi_derive::napi;
use yggdryl::IOResult;

use crate::iobase::safe_js_count;
use crate::{exact_u64, ordering_value};

/// The rows one record write read, wrote and skipped - what a `where` kept
/// out, or a bound cut off. Every record write of an `IOBase` answers one.
#[napi(js_name = "IOResult")]
#[derive(Clone, Copy)]
pub struct JsIOResult {
    inner: IOResult,
}

impl JsIOResult {
    /// Wrap a value the core answered.
    pub(crate) const fn from_core(inner: IOResult) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsIOResult {
    /// Each count is `0` when absent. Omitted, `skippedRows` is the rows read
    /// and not written; stated, the three counts are taken as they are - a
    /// sum's own.
    #[napi(constructor)]
    pub fn new(
        read_rows: Option<f64>,
        written_rows: Option<f64>,
        skipped_rows: Option<f64>,
    ) -> Result<Self> {
        let read_rows = read_rows.map_or(Ok(0), |count| exact_u64(count, "readRows"))?;
        let written_rows = written_rows.map_or(Ok(0), |count| exact_u64(count, "writtenRows"))?;
        Ok(Self::from_core(match skipped_rows {
            Some(count) => IOResult {
                read_rows,
                written_rows,
                skipped_rows: exact_u64(count, "skippedRows")?,
            },
            None => IOResult::new(read_rows, written_rows),
        }))
    }

    /// The rows the write pulled from its source.
    #[napi(getter)]
    pub fn read_rows(&self) -> i64 {
        safe_js_count(self.inner.read_rows)
    }

    /// The rows that reached the destination.
    #[napi(getter)]
    pub fn written_rows(&self) -> i64 {
        safe_js_count(self.inner.written_rows)
    }

    /// The rows read and not written.
    #[napi(getter)]
    pub fn skipped_rows(&self) -> i64 {
        safe_js_count(self.inner.skipped_rows)
    }

    /// Whether the write read no row at all: its source was empty.
    #[napi]
    pub const fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// The two results summed count by count.
    #[napi]
    pub fn add(&self, other: &JsIOResult) -> Self {
        Self::from_core(self.inner + other.inner)
    }

    /// Whether `other` states the same three counts.
    #[napi]
    pub fn equals(&self, other: &JsIOResult) -> bool {
        self.inner == other.inner
    }

    /// Compare the three counts in the core's order.
    #[napi]
    pub fn compare(&self, other: &JsIOResult) -> i32 {
        ordering_value(self.inner.cmp(&other.inner))
    }

    /// Deterministic hash bits of the three counts.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// A detached copy of this immutable result.
    #[napi(js_name = "clone")]
    pub const fn clone_js(&self) -> Self {
        *self
    }

    /// The core's text: `read 10 rows, wrote 8, skipped 2`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }
}
