//! The facts a JavaScript join states beside its keys: the kind, as one of
//! the core's six words, and the options, as one plain object - each read once
//! into the core's [`JoinKind`] and [`JoinOptions`] by the helpers every
//! `joinWith` shares (`Serie`, `ChunkedSerie`, `SerieReader`).
//!
//! The keys themselves are no business of this file: the loader converts
//! them into one `Scalar` and the core reads it ([`JoinKeys::from_scalar`]).
//!
//! [`JoinKeys::from_scalar`]: yggdryl::expression::JoinKeys::from_scalar

use std::str::FromStr;

use napi::bindgen_prelude::{ClassInstance, Either, Null, Result};
use napi_derive::napi;
use yggdryl::{JoinKind, JoinOptions, JoinSide};

use crate::graph::optional;
use crate::napi_error;
use crate::spill::JsSpillOptions;

/// The options of one join, each slot `undefined` or `null` where not given,
/// which is its default.
#[napi(object, object_to_js = false)]
#[derive(Default)]
pub struct JoinOptionsInput<'env> {
    /// Whether a key stated as the same bare column on both sides appears
    /// once, under the left name, left value else right; `true` by default.
    #[napi(ts_type = "boolean | null")]
    pub coalesce: Option<Either<bool, Null>>,
    /// What a right column whose name collides with a left one is suffixed
    /// with; `_right` by default.
    #[napi(ts_type = "string | null")]
    pub suffix: Option<Either<String, Null>>,
    /// Which side is held and hashed: `left` or `right`, or by default the
    /// held side over a stream, else the smaller, else the right.
    #[napi(ts_type = "'left' | 'right' | null")]
    pub build: Option<Either<String, Null>>,
    /// Whether probe rows and batches the build keys cannot match are
    /// dropped before they are hashed - the answer is the same either way;
    /// `true` by default.
    #[napi(ts_type = "boolean | null")]
    pub prune: Option<Either<bool, Null>>,
    /// The bound the build side and every output batch settle under; the
    /// process default (`SpillOptions.fromEnv()`) by default.
    #[napi(ts_type = "SpillOptions | null")]
    pub spill: Option<Either<ClassInstance<'env, JsSpillOptions>, Null>>,
    /// The largest distinct build key set pushed into a probe source's
    /// filter; 10,000 by default.
    #[napi(ts_type = "number | null")]
    pub pushdown_keys: Option<Either<f64, Null>>,
}

/// The kind `how` names through the core's own words - `inner`, `left`,
/// `right`, `full` (or `outer`), `semi`, `anti`, a trailing `outer` or
/// `join` read past - or `inner` where it names none.
pub(crate) fn join_kind(how: Option<String>) -> Result<JoinKind> {
    how.map_or(Ok(JoinKind::Inner), |how| {
        JoinKind::from_str(&how).map_err(napi_error)
    })
}

/// The core options `options` states, read once: every slot not given
/// keeps [`JoinOptions::new`]'s answer.
pub(crate) fn join_options(options: Option<JoinOptionsInput<'_>>) -> Result<JoinOptions> {
    let options = options.unwrap_or_default();
    let mut joined = JoinOptions::new();
    if let Some(coalesce) = optional(options.coalesce) {
        joined = joined.with_coalesce(coalesce);
    }
    if let Some(suffix) = optional(options.suffix) {
        joined = joined.with_suffix(suffix);
    }
    if let Some(build) = optional(options.build) {
        joined = joined.with_build(Some(JoinSide::from_str(&build).map_err(napi_error)?));
    }
    if let Some(prune) = optional(options.prune) {
        joined = joined.with_prune(prune);
    }
    if let Some(spill) = optional(options.spill) {
        joined = joined.with_spill(spill.inner.clone());
    }
    if let Some(keys) = optional(options.pushdown_keys) {
        joined = joined.with_pushdown_keys(crate::exact_usize(keys, "pushdownKeys")?);
    }
    Ok(joined)
}
