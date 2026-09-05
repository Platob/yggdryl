//! Schema-directed projection of native values into JavaScript.
//!
//! A struct [`yggdryl::Field`] is the schema, so projecting one value means
//! walking that field's children and building the matching JavaScript value.
//! The categories here mirror `dtype_to_js` exactly: whatever that
//! function produces for a datatype is what [`dtype_js_hint`] reports for
//! it, so a caller can pre-size or pre-type a value without projecting one.

use napi::bindgen_prelude::{
    BigInt, Buffer, Env, FnArgs, Function, JsObjectValue, JsValue, Null, Object, Result,
    ToNapiValue, Unknown,
};
use yggdryl::types::temporal::Temporal;
use yggdryl::{DataType, Field as CoreField, I256, Scalar, TemporalFamily, TimeUnit};

use crate::napi_error;

/// The JavaScript constructor category a datatype projects into.
///
/// The discriminants are a wire contract with `defaults.js`, which indexes a
/// frozen constructor table by [`JsValueHint::code`]. Do not renumber them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JsValueHint {
    /// Always null.
    Null = 0,
    /// A JavaScript boolean.
    Boolean = 1,
    /// A JavaScript number.
    Number = 2,
    /// A JavaScript `BigInt`, for values outside the safe-integer range.
    BigInt = 3,
    /// A JavaScript array: lists, structs, and interval tuples.
    Array = 4,
    /// A Node `Buffer`.
    Buffer = 5,
    /// A JavaScript string.
    String = 6,
    /// A plain JavaScript object: the `{ typeId, value }` union projection.
    Object = 7,
    /// A JavaScript Map.
    Map = 9,
}

impl JsValueHint {
    /// Return the table index `defaults.js` uses to pick a constructor.
    pub(crate) const fn code(self) -> u8 {
        self as u8
    }
}

/// Report the JavaScript category one datatype projects into.
///
/// This mirrors `dtype_to_js`; the two must stay in step.
pub(crate) fn dtype_js_hint(dtype: &DataType) -> Result<JsValueHint> {
    use DataType as D;

    Ok(match dtype {
        D::Null => JsValueHint::Null,
        D::Boolean => JsValueHint::Boolean,
        // 32-bit and smaller integers and every float fit a JS number.
        D::Int8
        | D::Int16
        | D::Int32
        | D::Date32
        | D::Time32(_)
        | D::UInt8
        | D::UInt16
        | D::UInt32
        | D::Float16
        | D::Float32
        | D::Float64
        | D::Duration32(_)
        | D::Interval(TimeUnit::YearMonth) => JsValueHint::Number,
        // 64-bit and wider integers exceed the safe-integer range.
        D::Int64
        | D::UInt64
        | D::DateTime64 { .. }
        | D::Date64
        | D::Time64(_)
        | D::Duration64(_)
        | D::Decimal32 { .. }
        | D::Decimal64 { .. }
        | D::Decimal128 { .. }
        | D::Decimal256 { .. } => JsValueHint::BigInt,
        // A geospatial value is its Well-Known Binary payload, so the pair
        // projects exactly as the binary family does.
        D::Binary
        | D::LargeBinary
        | D::BinaryView
        | D::FixedSizeBinary(_)
        | D::Geometry(_)
        | D::Geography(_) => JsValueHint::Buffer,
        // An ASCII width reads back as its trimmed text and a GUID as its
        // hyphenated spelling, so both project as the string family does.
        D::Utf8
        | D::LargeUtf8
        | D::Utf8View
        | D::Ascii
        | D::FixedAscii(_)
        | D::Country
        | D::Currency
        | D::Mic
        | D::Cfi
        | D::Guid => JsValueHint::String,
        // Day-time and month-day-nano intervals are integer tuples, and a
        // struct projects positionally, exactly like a list.
        D::Interval(TimeUnit::DayTime | TimeUnit::MonthDayNano)
        | D::List(_)
        | D::ListView(_)
        | D::FixedSizeList(..)
        | D::LargeList(_)
        | D::LargeListView(_)
        | D::Struct(_) => JsValueHint::Array,
        // A union carries its selected type id, so `union_to_js` builds a
        // `{ typeId, value }` object rather than a positional sequence.
        D::Union(..) => JsValueHint::Object,
        D::Interval(_) => return Err(napi_error("invalid native interval layout")),
        D::Map(_) => JsValueHint::Map,
        // Wrappers project as whatever they encode.
        D::Dictionary(dictionary) => dtype_js_hint(dictionary.value())?,
        D::RunEndEncoded(encoded) => dtype_js_hint(encoded.values().dtype())?,
        other => {
            return Err(napi_error(format!(
                "expected a datatype with a JavaScript projection, got {other}"
            )));
        }
    })
}

/// Project one value through its exact field.
pub(crate) fn field_value_to_js<'env>(
    env: &'env Env,
    field: &CoreField,
    value: &Scalar,
) -> Result<Unknown<'env>> {
    value_to_js(env, field, value)
}

fn value_to_js<'env>(env: &'env Env, field: &CoreField, value: &Scalar) -> Result<Unknown<'env>> {
    dtype_to_js(env, field.dtype(), value)
}

fn dtype_to_js<'env>(env: &'env Env, dtype: &DataType, value: &Scalar) -> Result<Unknown<'env>> {
    use DataType as D;

    if matches!(value, Scalar::Null) {
        return Null.into_unknown(env);
    }
    if let Some(value) = numeric_to_js(env, dtype, value)? {
        return Ok(value);
    }
    if let Some(value) = temporal_to_js(env, dtype, value)? {
        return Ok(value);
    }
    if let Some(value) = text_or_binary_to_js(env, dtype, value)? {
        return Ok(value);
    }
    match dtype {
        D::Null => Null.into_unknown(env),
        D::List(item)
        | D::ListView(item)
        | D::FixedSizeList(item, _)
        | D::LargeList(item)
        | D::LargeListView(item) => sequence_to_js(env, item, value),
        D::Struct(fields) => struct_to_js(env, fields, value),
        D::Union(fields, _) => union_to_js(env, fields, value),
        D::Dictionary(dictionary) => dtype_to_js(env, dictionary.value(), value),
        D::Map(map) => map_to_js(env, map, value),
        D::RunEndEncoded(encoded) => value_to_js(env, encoded.values(), value),
        // A non-null variant value crosses as the Parquet Variant binary
        // encoding, which the Iceberg v3 layer owns; refuse by name until
        // that codec lands rather than inventing a second encoding here.
        D::Variant => Err(napi_error(
            "unsupported native datatype variant: \
             the variant binary encoding lands with the Iceberg v3 layer",
        )),
        _ => Err(napi_error(format!("unsupported native datatype {dtype}"))),
    }
}

fn numeric_to_js<'env>(
    env: &'env Env,
    dtype: &DataType,
    value: &Scalar,
) -> Result<Option<Unknown<'env>>> {
    use DataType as D;

    let output = match dtype {
        D::Boolean => value
            .as_bool()
            .ok_or_else(|| napi_error("invalid native boolean record value"))?
            .into_unknown(env)?,
        D::Int8 | D::Int16 | D::Int32 => {
            let integer = value
                .as_i128()
                .ok_or_else(|| napi_error("invalid native 32-bit integer record value"))?;
            i32::try_from(integer)
                .map_err(|_| napi_error("native integer is out of signed 32-bit range"))?
                .into_unknown(env)?
        }
        D::UInt8 | D::UInt16 | D::UInt32 => {
            let integer = value
                .as_u128()
                .ok_or_else(|| napi_error("invalid native unsigned record value"))?;
            u32::try_from(integer)
                .map_err(|_| napi_error("native integer is out of unsigned 32-bit range"))?
                .into_unknown(env)?
        }
        D::Int64 => BigInt::from(
            value
                .as_i128()
                .ok_or_else(|| napi_error("invalid native signed integer record value"))?,
        )
        .into_unknown(env)?,
        D::UInt64 => BigInt::from(
            value
                .as_u128()
                .ok_or_else(|| napi_error("invalid native unsigned integer record value"))?,
        )
        .into_unknown(env)?,
        D::Float16 | D::Float32 | D::Float64 => value
            .as_f64()
            .ok_or_else(|| napi_error("invalid native floating record value"))?
            .into_unknown(env)?,
        D::Decimal32 { scale, .. } | D::Decimal64 { scale, .. } | D::Decimal128 { scale, .. } => {
            BigInt::from(
                value
                    .decimal_unscaled_at(*scale)
                    .or_else(|| value.as_i128())
                    .ok_or_else(|| napi_error("invalid native decimal record value"))?,
            )
            .into_unknown(env)?
        }
        D::Decimal256 { scale, .. } => decimal256_to_js(env, value, *scale)?,
        _ => return Ok(None),
    };
    Ok(Some(output))
}

fn temporal_to_js<'env>(
    env: &'env Env,
    dtype: &DataType,
    value: &Scalar,
) -> Result<Option<Unknown<'env>>> {
    use DataType as D;

    let output = match dtype {
        D::Date32 => temporal_value_to_js(env, value, TemporalFamily::Date, TimeUnit::Day, 32)?,
        D::Date64 => {
            temporal_value_to_js(env, value, TemporalFamily::Date, TimeUnit::Millisecond, 64)?
        }
        D::Time32(unit) => temporal_value_to_js(env, value, TemporalFamily::Time, *unit, 32)?,
        D::Time64(unit) => temporal_value_to_js(env, value, TemporalFamily::Time, *unit, 64)?,
        D::DateTime64 { unit, .. } => {
            temporal_value_to_js(env, value, TemporalFamily::DateTime, *unit, 64)?
        }
        D::Duration32(unit) => {
            temporal_value_to_js(env, value, TemporalFamily::Duration, *unit, 32)?
        }
        D::Duration64(unit) => {
            temporal_value_to_js(env, value, TemporalFamily::Duration, *unit, 64)?
        }
        D::Interval(unit) => interval_to_js(env, value, *unit)?,
        _ => return Ok(None),
    };
    Ok(Some(output))
}

fn temporal_value_to_js<'env>(
    env: &'env Env,
    value: &Scalar,
    family: TemporalFamily,
    unit: TimeUnit,
    bit_width: u8,
) -> Result<Unknown<'env>> {
    let temporal = value.as_temporal();
    if temporal.is_some_and(|value| value.family() != family) {
        return Err(napi_error("invalid native temporal family"));
    }
    let count = temporal
        .and_then(|_| value.temporal_count_at(unit))
        .or_else(|| value.as_i128().and_then(|count| i64::try_from(count).ok()))
        .ok_or_else(|| napi_error("invalid native temporal record value"))?;
    if bit_width == 32 {
        return i32::try_from(count)
            .map_err(|_| napi_error("native temporal count exceeds signed 32 bits"))?
            .into_unknown(env);
    }
    BigInt::from(count).into_unknown(env)
}

fn text_or_binary_to_js<'env>(
    env: &'env Env,
    dtype: &DataType,
    value: &Scalar,
) -> Result<Option<Unknown<'env>>> {
    use DataType as D;

    let output = match dtype {
        D::Binary | D::LargeBinary | D::BinaryView | D::FixedSizeBinary(_) => Buffer::from(
            value
                .as_bytes()
                .ok_or_else(|| napi_error("invalid native binary record value"))?
                .to_vec(),
        )
        .into_unknown(env)?,
        D::Utf8
        | D::LargeUtf8
        | D::Utf8View
        | D::Ascii
        | D::FixedAscii(_)
        | D::Country
        | D::Currency
        | D::Mic
        | D::Cfi => value
            .as_str()
            .ok_or_else(|| napi_error("invalid native string record value"))?
            .to_owned()
            .into_unknown(env)?,
        D::Guid => match value {
            Scalar::Guid(value) => value.to_string().into_unknown(env)?,
            _ => return Err(napi_error("invalid native guid record value")),
        },
        // A geospatial value is its Well-Known Binary payload, so it crosses
        // exactly as the binary family does.
        D::Geometry(_) | D::Geography(_) => Buffer::from(
            value
                .as_wkb()
                .ok_or_else(|| napi_error("invalid native geospatial record value"))?
                .to_vec(),
        )
        .into_unknown(env)?,
        _ => return Ok(None),
    };
    Ok(Some(output))
}

fn interval_to_js<'env>(env: &'env Env, value: &Scalar, unit: TimeUnit) -> Result<Unknown<'env>> {
    let Some(Temporal::Interval(interval)) = value.as_temporal() else {
        return Err(napi_error("invalid native interval value"));
    };
    if interval.unit() != unit {
        return Err(napi_error(
            "native interval layout disagrees with its datatype",
        ));
    }
    match unit {
        TimeUnit::YearMonth => interval.months().into_unknown(env),
        TimeUnit::DayTime => {
            let mut output = env.create_array(2)?;
            output.set(0, interval.days())?;
            output.set(
                1,
                i32::try_from(interval.nanoseconds() / 1_000_000)
                    .map_err(|_| napi_error("native interval milliseconds exceed int32"))?,
            )?;
            output.into_unknown(env)
        }
        TimeUnit::MonthDayNano => {
            let mut output = env.create_array(3)?;
            output.set(0, interval.months())?;
            output.set(1, interval.days())?;
            output.set(2, BigInt::from(interval.nanoseconds()))?;
            output.into_unknown(env)
        }
        _ => Err(napi_error("invalid native interval layout")),
    }
}

fn sequence_to_js<'env>(
    env: &'env Env,
    field: &CoreField,
    value: &Scalar,
) -> Result<Unknown<'env>> {
    let values = value
        .as_sequence()
        .ok_or_else(|| napi_error("invalid native list record value"))?;
    let mut output = env.create_array(u32::try_from(values.len()).unwrap_or(u32::MAX))?;
    for (index, value) in values.iter().enumerate() {
        let js_index = u32::try_from(index)
            .map_err(|_| napi_error("list index exceeds the JavaScript array limit"))?;
        output.set(js_index, projected_value_to_js(env, field, value)?)?;
    }
    output.into_unknown(env)
}

fn projected_value_to_js<'env>(
    env: &'env Env,
    field: &CoreField,
    value: &Scalar,
) -> Result<Unknown<'env>> {
    if matches!(value, Scalar::Null) {
        return Null.into_unknown(env);
    }
    value_to_js(env, field, value)
}

/// Project one struct value as a positional JavaScript array.
fn struct_to_js<'env>(
    env: &'env Env,
    fields: &yggdryl::Fields,
    value: &Scalar,
) -> Result<Unknown<'env>> {
    let values = value
        .as_sequence()
        .ok_or_else(|| napi_error("invalid native struct record value"))?;
    if values.len() != fields.len() {
        return Err(napi_error(format!(
            "expected {} struct slots, got {}",
            fields.len(),
            values.len()
        )));
    }
    let mut output = env.create_array(u32::try_from(values.len()).unwrap_or(u32::MAX))?;
    for (index, (field, value)) in fields.iter().zip(values).enumerate() {
        let js_index = u32::try_from(index)
            .map_err(|_| napi_error("struct index exceeds the JavaScript array limit"))?;
        output.set(js_index, projected_value_to_js(env, field, value)?)?;
    }
    output.into_unknown(env)
}

fn union_to_js<'env>(
    env: &'env Env,
    fields: &yggdryl::UnionFields,
    value: &Scalar,
) -> Result<Unknown<'env>> {
    let values = value
        .as_sequence()
        .ok_or_else(|| napi_error("invalid native union record value"))?;
    let [type_id, payload] = values else {
        return Err(napi_error(
            "native union value must contain type id and payload",
        ));
    };
    let type_id = i8::try_from(
        type_id
            .as_i128()
            .ok_or_else(|| napi_error("invalid native union type id"))?,
    )
    .map_err(napi_error)?;
    let (_, field) = fields
        .iter()
        .find(|(candidate, _)| *candidate == type_id)
        .ok_or_else(|| napi_error(format!("unknown native union type id {type_id}")))?;
    let mut output = Object::new(env)?;
    output.set("typeId", i32::from(type_id))?;
    output.set("value", value_to_js(env, field, payload)?)?;
    output.into_unknown(env)
}

fn decimal256_to_js<'env>(env: &'env Env, value: &Scalar, scale: i8) -> Result<Unknown<'env>> {
    let encoded = value
        .decimal256_unscaled_at(scale)
        .or_else(|| value.as_i128().map(I256::from_i128))
        .map(|unscaled| unscaled.to_string())
        .ok_or_else(|| napi_error("invalid native decimal256 record value"))?;
    let global = env.get_global()?;
    let bigint: Function<'_, FnArgs<(String,)>, Unknown<'_>> =
        global.get_named_property("BigInt")?;
    bigint.call((encoded,).into())
}

fn map_to_js<'env>(
    env: &'env Env,
    map: &yggdryl::MapType,
    value: &Scalar,
) -> Result<Unknown<'env>> {
    let entries = value
        .as_mapping()
        .ok_or_else(|| napi_error("invalid native map record value"))?;
    let fields = map
        .entries()
        .dtype()
        .as_fields()
        .ok_or_else(|| napi_error("invalid native map entry schema"))?;
    let [key_field, value_field] = fields else {
        return Err(napi_error("invalid native map entry schema"));
    };
    let global = env.get_global()?;
    let constructor: Function<'_, (), Unknown<'_>> = global.get_named_property("Map")?;
    let map_value = constructor.new_instance(())?;
    let object = map_value.coerce_to_object()?;
    let set: Function<'_, FnArgs<(Unknown<'_>, Unknown<'_>)>, Unknown<'_>> =
        required_property(&object, "set")?;
    let has: Function<'_, FnArgs<(Unknown<'_>,)>, bool> = required_property(&object, "has")?;
    for (key, value) in entries {
        let key = projected_value_to_js(env, key_field, key)?;
        if has.apply(object, (key,).into())? {
            return Err(napi_error(
                "native map contains distinct keys that collide under JavaScript Map semantics",
            ));
        }
        let value = projected_value_to_js(env, value_field, value)?;
        set.apply(object, (key, value).into())?;
    }
    map_value.into_unknown(env)
}

fn required_property<T>(object: &Object<'_>, name: &str) -> Result<T>
where
    T: napi::bindgen_prelude::FromNapiValue,
{
    object
        .get::<T>(name)?
        .ok_or_else(|| napi_error(format!("missing JavaScript property {name:?}")))
}

pub(crate) fn arrow_scalar_to_ipc(
    field: &yggdryl::Field,
    array: arrow_array::ArrayRef,
) -> Result<napi::bindgen_prelude::Buffer> {
    use std::sync::Arc;

    use arrow_array::{RecordBatch, RecordBatchOptions};
    use arrow_ipc::writer::StreamWriter;
    use arrow_schema::Schema;

    let schema = Arc::new(Schema::new([field
        .clone()
        .into_arrow_ref()
        .map_err(napi_error)?]));
    let options = RecordBatchOptions::new().with_row_count(Some(1));
    let batch =
        RecordBatch::try_new_with_options(schema, vec![array], &options).map_err(napi_error)?;
    let mut writer =
        StreamWriter::try_new(Vec::new(), batch.schema().as_ref()).map_err(napi_error)?;
    writer.write(&batch).map_err(napi_error)?;
    writer.finish().map_err(napi_error)?;
    Ok(writer.into_inner().map_err(napi_error)?.into())
}
