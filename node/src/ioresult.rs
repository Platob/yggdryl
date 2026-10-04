//! JavaScript's native view of the shared [`IOResult`]: the rows one record
//! write read, wrote and skipped.
//!
//! [`JsIOResult`] owns only the core value and is immutable. Every number is
//! the core's: the constructor is [`IOResult::new`], or the value's own
//! three fields where all three are stated, `add` is its `Add`, `toString`
//! its `Display`, and a count crosses as the `number` every other row count
//! of this binding crosses as.

use napi::bindgen_prelude::Result;
use napi_derive::napi;
use yggdryl::IOResult;

use crate::iobase::safe_js_count;
use crate::{exact_u64, ordering_value};

/// The rows one record write read, wrote and skipped.
///
/// Every record write of an `IOBase` answers one: `readRows` is what the
/// write pulled from its source, `writtenRows` what reached the destination,
/// `skippedRows` what was read and not written - the rows a `where` kept
/// out, the part of the last batch a bound cut off.
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
    /// The result of a write that read `readRows` and wrote `writtenRows`,
    /// each `0` when absent, the rest of what it read skipped - or, where
    /// `skippedRows` is stated, the three counts as they are: a sum of
    /// results states its own skipped rows, which `readRows - writtenRows`
    /// need not be.
    #[napi(constructor)]
    pub fn new(
        read_rows: Option<f64>,
        written_rows: Option<f64>,
        skipped_rows: Option<f64>,
    ) -> Result<Self> {
        let read_rows = read_rows.map_or(Ok(0), |count| exact_u64(count, "readRows"))?;
        let written_rows = written_rows.map_or(Ok(0), |count| exact_u64(count, "writtenRows"))?;
        let Some(skipped_rows) = skipped_rows else {
            return Ok(Self::from_core(IOResult::new(read_rows, written_rows)));
        };
        Ok(Self::from_core(IOResult {
            read_rows,
            written_rows,
            skipped_rows: exact_u64(skipped_rows, "skippedRows")?,
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

    /// The two results summed count by count, as one write cut into several
    /// commits answers.
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
