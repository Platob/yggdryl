//! Phase 8 — the shared **vectorized-op + reshape** helpers every `Serie` wrapper delegates to.
//!
//! The core exposes the whole surface on `dyn AnySerie` (element-wise `add`/`sub`/`mul`/`div`/`rem`
//! serie×serie and serie×scalar, `filter`, `fill_null`, and the `to_struct` / `to_list` / `to_map`
//! reshapes), so each napi wrapper method is a 1–3 line delegate: coerce `&self.inner` to
//! `&dyn AnySerie`, call the core, and rewrap the erased result as the wrapper's own concrete class.
//! Centralizing the delegation here keeps every wrapper's methods identical and one-liner thin.
//!
//! **The operand union.** Node has no operators, so `add(other)` (and its siblings) take a single
//! `other: unknown` that is *either* another `Serie` wrapper *or* a JS scalar (`number` / numeric
//! `string` / `boolean` / `bigint`). A wrapper operand is recognized by trying **every** `Serie`
//! `#[napi]` class (the full [`serie_operand`] list — not only the castable-numeric ones) with the
//! derive-generated `napi_instanceof` check ([`ValidateNapiValue`]) — sound, unlike a bare
//! `napi_unwrap` — and is passed straight to the erased op (so cross-type `i32.add(i64)` works,
//! result following the LEFT, and a **cast-anything** `Serie` like a `Utf8Serie` / decimal / temporal
//! column reaches the core and is coerced into the LEFT's element type — only a genuinely
//! non-convertible cell, e.g. a non-numeric utf8, surfaces the core's guided parse error, matching
//! Python).
//! Anything else is coerced, **against the LEFT column's element type**, into a broadcast
//! [`AnyScalar`] by [`arith_scalar`]. (Mirror note for Python parity: Python folds the same two paths
//! into one `__add__`/`add`; there is **no** separate `addScalar` — the single `add` here is that
//! fold, and its scalar coercion rule is identical.)

use napi::bindgen_prelude::{BigInt, Buffer, FromNapiValue, ValidateNapiValue};
use napi::{Env, JsUnknown, NapiRaw, ValueType};

use yggdryl_core::io::fixed::Field as CoreField;
use yggdryl_core::io::nested::{ListSerie as CoreListSerie, StructSerie as CoreStructSerie};
use yggdryl_core::io::{AnyScalar, AnySerie, DataTypeId};

use crate::deccolumn::{D128Serie, D256Serie, D32Serie, D64Serie};
use crate::nested::{js_to_any_scalar, ListSerie, MapSerie, StructSerie};
use crate::nullvalues::NullSerie;
use crate::temporal_column::{
    Date32Serie, Date64Serie, Duration32Serie, Duration64Serie, Time32Serie, Time64Serie,
    Ts32Serie, Ts64Serie, Ts96Serie,
};
use crate::values::{
    from_unknown, js_int_value, F16Serie, F32Serie, F64Serie, I128Serie, I16Serie, I256Serie,
    I32Serie, I64Serie, I8Serie, I96Serie, U128Serie, U16Serie, U256Serie, U32Serie, U64Serie,
    U8Serie, U96Serie,
};
use crate::varvalues::{BinarySerie, Utf8Serie};

/// Maps any core error to a thrown JS `Error` (its guided text passes through unchanged).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// Rewraps an erased column into the wrapper's concrete inner type — every safe op (`filter`,
/// `fill_null`, an arithmetic op whose result follows the LEFT) returns a `Box<dyn AnySerie>` of the
/// **same** concrete class as `self`, so this downcast-and-clone always succeeds.
fn rewrap<T: Clone + 'static>(erased: Box<dyn AnySerie>) -> T {
    erased
        .as_any()
        .downcast_ref::<T>()
        .expect("a same-type op preserves the concrete Serie type")
        .clone()
}

/// `serie.filter(mask)` → the same-class column of the kept rows (a guided error on a mask whose
/// length is not the column length).
pub(crate) fn filter_into<T: Clone + 'static>(
    col: &(dyn AnySerie + 'static),
    mask: Vec<bool>,
) -> napi::Result<T> {
    Ok(rewrap(col.filter(&mask).map_err(to_error)?))
}

/// `serie.fillNull(value)` → the same-class column with every null replaced by `value`. See
/// [`fill_scalar`] for how `value` is resolved (a `null`, a leaf `Serie` carrier, or a JS native).
pub(crate) fn fill_null_into<T: Clone + 'static>(
    env: Env,
    col: &(dyn AnySerie + 'static),
    value: JsUnknown,
) -> napi::Result<T> {
    let scalar = fill_scalar(env, col, &value)?;
    Ok(rewrap(col.fill_null(&scalar).map_err(to_error)?))
}

/// The fill scalar for [`fill_null_into`]:
/// - a JS `null` / `undefined` → the null scalar (a no-op clone);
/// - a **leaf `Serie` carrier** (any non-nested `Serie` wrapper) → its `value(0)`, which conveys the
///   source column's *exact* leaf type — including a decimal's `(precision, scale)` and a temporal's
///   `(unit, tz)` — so the core fill guard can verify a match (and guide-error on a mismatch). This is
///   the Node mirror of Python's length-1 carrier fill, the only way to fill a decimal / temporal
///   column with a real value (they have no native scalar form in the [`js_to_any_scalar`] bridge);
/// - otherwise a JS native → cast into the column's own leaf type via [`js_to_any_scalar`] (a nested
///   column has no single leaf type, so infer a numeric leaf scalar the core fills into each matching
///   leaf).
fn fill_scalar(
    env: Env,
    col: &(dyn AnySerie + 'static),
    value: &JsUnknown,
) -> napi::Result<AnyScalar> {
    let value_type = value.get_type()?;
    if matches!(value_type, ValueType::Null | ValueType::Undefined) {
        return Ok(AnyScalar::null());
    }
    if matches!(value_type, ValueType::Object) {
        if let Some(carrier) = carrier_scalar(env, value)? {
            return Ok(carrier);
        }
    }
    let id = col.type_id();
    if matches!(id, DataTypeId::Struct | DataTypeId::List | DataTypeId::Map) {
        return nested_broadcast_scalar(env, value);
    }
    let width = id.fixed_byte_width().unwrap_or(0);
    js_to_any_scalar(env, value, id, width)
}

/// The first-cell [`AnyScalar`] of a **leaf `Serie` carrier** (any non-nested `Serie` wrapper), or
/// `None` if `value` is not one — recognized by the derive-generated instanceof check (as in
/// [`serie_operand`]). A length-1 carrier's `value(0)` carries the exact leaf type + scale/unit
/// metadata, so filling a decimal / temporal column with a real value guide-errors on a mismatch in
/// the core (an empty carrier's `value(0)` is a null scalar → a no-op fill).
fn carrier_scalar(env: Env, value: &JsUnknown) -> napi::Result<Option<AnyScalar>> {
    let raw_env = env.raw();
    let raw_val = unsafe { value.raw() };
    macro_rules! try_classes {
        ($($T:ty),+ $(,)?) => {{
            $(
                if unsafe { <&$T as ValidateNapiValue>::validate(raw_env, raw_val) }.is_ok() {
                    let wrapper: &$T =
                        unsafe { <&$T as FromNapiValue>::from_napi_value(raw_env, raw_val)? };
                    return Ok(Some((&wrapper.inner as &dyn AnySerie).value(0)));
                }
            )+
        }};
    }
    try_classes!(
        U8Serie,
        U16Serie,
        U32Serie,
        U64Serie,
        U96Serie,
        U128Serie,
        U256Serie,
        I8Serie,
        I16Serie,
        I32Serie,
        I64Serie,
        I96Serie,
        I128Serie,
        I256Serie,
        F16Serie,
        F32Serie,
        F64Serie,
        Utf8Serie,
        BinarySerie,
        D32Serie,
        D64Serie,
        D128Serie,
        D256Serie,
        Date32Serie,
        Date64Serie,
        Time32Serie,
        Time64Serie,
        Ts32Serie,
        Ts64Serie,
        Ts96Serie,
        Duration32Serie,
        Duration64Serie,
        NullSerie
    );
    Ok(None)
}

/// `serie.toStruct(name?)` → a one-field [`StructSerie`] (`name` defaults to `"value"`); an already
/// struct column returns itself. The result is always a struct, so napi returns the concrete wrapper.
pub(crate) fn to_struct_wrapper(
    col: &(dyn AnySerie + 'static),
    name: Option<String>,
) -> StructSerie {
    let name = name.unwrap_or_else(|| "value".to_string());
    StructSerie {
        inner: rewrap::<CoreStructSerie>(col.to_struct(&name)),
    }
}

/// `serie.toList()` → a list-of-singletons [`ListSerie`]; an already list column returns itself. The
/// result is always a list, so napi returns the concrete wrapper.
pub(crate) fn to_list_wrapper(col: &(dyn AnySerie + 'static)) -> ListSerie {
    ListSerie {
        inner: rewrap::<CoreListSerie>(col.to_list()),
    }
}

/// `serie.toMap()` → the reshaped column's `serializeBytes()` frame. Unlike `toStruct` / `toList`,
/// `to_map`'s result class is **not** statically known — a 2-column struct (or an already map) yields
/// a `MapSerie`, every other shape passes through unchanged — so napi returns the self-describing
/// frame and the caller reconstructs it with the matching `deserializeBytes` (`MapSerie` when a map
/// resulted, else the source class).
pub(crate) fn to_map_frame(col: &(dyn AnySerie + 'static)) -> napi::Result<Buffer> {
    Ok(col.to_map().map_err(to_error)?.serialize_bytes().into())
}

/// The erased column of **any** `Serie`-wrapper JS value (cloned out, so no borrow of the JS object
/// escapes), or `None` if `value` is not a `Serie` wrapper at all (a scalar, a `Buffer`, or a plain
/// object). It recognizes the **full** wrapper set — the same list its sibling [`carrier_scalar`]
/// enumerates, plus the three nested columns — so a *real* `Serie` right operand always reaches the
/// core op even when it is **not** a plain fixed-numeric column (a `Utf8Serie` / decimal / wide /
/// temporal column): the core then **coerces** it into the LEFT's element type (the Phase 9
/// cast-anything ops), surfacing a guided parse error only for a genuinely non-convertible cell —
/// identical to the Python binding (which lets every `Serie` through to the same core op).
/// Each candidate is gated by the derive-generated instanceof check ([`ValidateNapiValue::validate`])
/// **before** the (otherwise type-blind) `napi_unwrap`, so the downcast is sound.
fn serie_operand(env: Env, value: &JsUnknown) -> napi::Result<Option<Box<dyn AnySerie>>> {
    let raw_env = env.raw();
    let raw_val = unsafe { value.raw() };
    macro_rules! try_classes {
        ($($T:ty),+ $(,)?) => {{
            $(
                if unsafe { <&$T as ValidateNapiValue>::validate(raw_env, raw_val) }.is_ok() {
                    let wrapper: &$T =
                        unsafe { <&$T as FromNapiValue>::from_napi_value(raw_env, raw_val)? };
                    return Ok(Some((&wrapper.inner as &dyn AnySerie).clone_box()));
                }
            )+
        }};
    }
    try_classes!(
        U8Serie,
        U16Serie,
        U32Serie,
        U64Serie,
        U96Serie,
        U128Serie,
        U256Serie,
        I8Serie,
        I16Serie,
        I32Serie,
        I64Serie,
        I96Serie,
        I128Serie,
        I256Serie,
        F16Serie,
        F32Serie,
        F64Serie,
        Utf8Serie,
        BinarySerie,
        D32Serie,
        D64Serie,
        D128Serie,
        D256Serie,
        Date32Serie,
        Date64Serie,
        Time32Serie,
        Time64Serie,
        Ts32Serie,
        Ts64Serie,
        Ts96Serie,
        Duration32Serie,
        Duration64Serie,
        NullSerie,
        StructSerie,
        ListSerie,
        MapSerie
    );
    Ok(None)
}

/// Builds the broadcast [`AnyScalar`] for a serie×scalar arithmetic op, **coerced to the LEFT
/// column's element type** — the single rule shared verbatim with the Python binding
/// (`bindings/python/src/nested.rs::arith_scalar`), so both bindings accept exactly the same operands
/// and reject the same ones for every one of these cases:
/// - a JS `null` / `undefined` → the null scalar (an all-null result);
/// - an **integer** leaf column (`u8`…`i64`, `i128`) → a **whole** integer (a `number` / `boolean` /
///   integer `string` / `bigint`) via the shared [`js_int_value`] wholeness check — the *same* one
///   `fillNull` uses — so a fractional value (`2.5` / `"2.5"`) is a guided error, never a silent
///   truncation; range-checked into the column type by the core;
/// - a **float** leaf column (`f16` / `f32` / `f64`) → any `number` / numeric `string` / `bigint` → `f64`;
/// - a **nested** column (struct / list / map) → an inferred whole `i128` / fractional `f64`, which the
///   core broadcasts + casts into each leaf child (see [`nested_broadcast_scalar`]).
fn arith_scalar(
    env: Env,
    value: &JsUnknown,
    left: &(dyn AnySerie + 'static),
) -> napi::Result<AnyScalar> {
    if matches!(value.get_type()?, ValueType::Null | ValueType::Undefined) {
        return Ok(AnyScalar::null());
    }
    let id = left.type_id();
    if id.is_integer() {
        return Ok(leaf_i128(int_operand(env, value)?));
    }
    if id.is_floating() {
        return Ok(leaf_f64(float_operand(env, value)?));
    }
    // A nested column (or any non-leaf-numeric left) infers a whole `i128` / fractional `f64` the
    // core broadcasts into each leaf child.
    nested_broadcast_scalar(env, value)
}

/// A JS value coerced to a **whole** `i128` for an integer leaf column — a `number` / `boolean` /
/// integer `string` via the shared [`js_int_value`] (the *same* wholeness check `fillNull` uses), or
/// a 128-bit `bigint`. A fractional / non-integer value is a guided error (wholeness required). A JS
/// `null` never reaches here — the caller builds the null scalar.
fn int_operand(env: Env, value: &JsUnknown) -> napi::Result<i128> {
    if matches!(value.get_type()?, ValueType::BigInt) {
        let bigint: BigInt = from_unknown(env, value)?;
        let (value, lossless) = bigint.get_i128();
        return if lossless {
            Ok(value)
        } else {
            Err(to_error(
                "the bigint operand exceeds the 128-bit range of an arithmetic scalar",
            ))
        };
    }
    js_int_value(env, value)?
        .ok_or_else(|| to_error("expected an integer arithmetic operand, got null"))
}

/// A JS value coerced to `f64` for a float leaf column — a `number`, a `boolean` (`0` / `1`), a
/// numeric `string`, or a `bigint`. A non-numeric value is a guided error. A JS `null` never reaches
/// here — the caller builds the null scalar.
fn float_operand(env: Env, value: &JsUnknown) -> napi::Result<f64> {
    match value.get_type()? {
        ValueType::Number => from_unknown(env, value),
        ValueType::Boolean => Ok(if from_unknown::<bool>(env, value)? {
            1.0
        } else {
            0.0
        }),
        ValueType::String => {
            let text: String = from_unknown(env, value)?;
            text.parse::<f64>().map_err(|_| {
                to_error(format!(
                    "cannot use {text:?} as an arithmetic operand; expected a number, a numeric \
                     string, or a Serie"
                ))
            })
        }
        ValueType::BigInt => {
            let bigint: BigInt = from_unknown(env, value)?;
            Ok(bigint.get_i128().0 as f64)
        }
        other => Err(to_error(format!(
            "cannot use a {other:?} value as an arithmetic operand; expected a number, a numeric \
             string, or a Serie"
        ))),
    }
}

/// The **nested-column broadcast** inference: `null` → the null scalar; a `boolean` / whole `number`
/// / integer `string` / `bigint` → an `i128` value; a fractional `number` / decimal `string` → an
/// `f64` value. The core then broadcasts + casts it into each leaf child of the struct / list / map.
/// Also the fill scalar a nested `fillNull` uses (its acceptance is unchanged).
fn nested_broadcast_scalar(env: Env, value: &JsUnknown) -> napi::Result<AnyScalar> {
    match value.get_type()? {
        ValueType::Null | ValueType::Undefined => Ok(AnyScalar::null()),
        ValueType::Boolean => Ok(leaf_i128(if from_unknown::<bool>(env, value)? {
            1
        } else {
            0
        })),
        ValueType::Number => {
            let number: f64 = from_unknown(env, value)?;
            // A whole number in the strict `i128` range is an integer; anything else (fractional,
            // non-finite, or out of range) is an `f64` — the core casts either into each leaf.
            if number.is_finite()
                && number.fract() == 0.0
                && number >= i128::MIN as f64
                && number < 2f64.powi(127)
            {
                Ok(leaf_i128(number as i128))
            } else {
                Ok(leaf_f64(number))
            }
        }
        ValueType::String => {
            let text: String = from_unknown(env, value)?;
            if let Ok(value) = text.parse::<i128>() {
                Ok(leaf_i128(value))
            } else if let Ok(value) = text.parse::<f64>() {
                Ok(leaf_f64(value))
            } else {
                Err(to_error(format!(
                    "cannot use {text:?} as an arithmetic operand; expected a number, a numeric \
                     string, or a Serie"
                )))
            }
        }
        ValueType::BigInt => {
            let bigint: BigInt = from_unknown(env, value)?;
            let (value, lossless) = bigint.get_i128();
            if lossless {
                Ok(leaf_i128(value))
            } else {
                Err(to_error(
                    "the bigint operand exceeds the 128-bit range of an arithmetic scalar",
                ))
            }
        }
        other => Err(to_error(format!(
            "cannot use a {other:?} value as an arithmetic operand; expected a number, a numeric \
             string, or a Serie"
        ))),
    }
}

/// An `i128` value as a leaf [`AnyScalar`] (the widest signed integer scalar; the core casts it into
/// the target column's element type, range-checked).
fn leaf_i128(value: i128) -> AnyScalar {
    AnyScalar::leaf(
        CoreField::of("", DataTypeId::I128, 16, false),
        value.to_le_bytes().to_vec(),
    )
}

/// An `f64` value as a leaf [`AnyScalar`].
fn leaf_f64(value: f64) -> AnyScalar {
    AnyScalar::leaf(
        CoreField::of("", DataTypeId::F64, 8, false),
        value.to_le_bytes().to_vec(),
    )
}

/// Generates the five arithmetic delegates (`add`/`sub`/`mul`/`div`/`rem`). Each accepts the operand
/// union: a `Serie` wrapper → the erased element-wise op (result follows the LEFT); otherwise a JS
/// scalar → the erased broadcast. The result is rewrapped as the LEFT wrapper's own class.
macro_rules! arith_into {
    ($name:ident, $serie_op:ident, $scalar_op:ident) => {
        #[doc = concat!("`serie.", stringify!($name), "(other)` — element-wise with another `Serie`, ")]
        #[doc = "else a broadcast of a numeric scalar. The result follows the LEFT operand's type."]
        pub(crate) fn $name<T: Clone + 'static>(
            env: Env,
            left: &(dyn AnySerie + 'static),
            other: JsUnknown,
        ) -> napi::Result<T> {
            if matches!(other.get_type()?, ValueType::Object) {
                if let Some(right) = serie_operand(env, &other)? {
                    return Ok(rewrap(left.$serie_op(right.as_ref()).map_err(to_error)?));
                }
            }
            let scalar = arith_scalar(env, &other, left)?;
            Ok(rewrap(left.$scalar_op(&scalar).map_err(to_error)?))
        }
    };
}

arith_into!(add_into, add, add_scalar);
arith_into!(sub_into, sub, sub_scalar);
arith_into!(mul_into, mul, mul_scalar);
arith_into!(div_into, div, div_scalar);
arith_into!(rem_into, rem, rem_scalar);
