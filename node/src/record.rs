//! Schema-directed projection of native values into JavaScript.
//!
//! A struct [`yggdryl::Field`] is the schema, so projecting one value means
//! walking that field's children and building the matching JavaScript value.
//! The categories here mirror `data_type_to_js` exactly: whatever that
//! function produces for a datatype is what [`data_type_js_hint`] reports for
//! it, so a caller can pre-size or pre-type a value without projecting one.

use napi::bindgen_prelude::{
    BigInt, Buffer, Env, FnArgs, Function, JsObjectValue, JsValue, Null, Object, Result,
    ToNapiValue, Unknown,
};
use yggdryl::{DataType, Field as CoreField, TimeUnit, Value};

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
/// This mirrors `data_type_to_js`; the two must stay in step.
pub(crate) fn data_type_js_hint(data_type: &DataType) -> Result<JsValueHint> {
    use DataType as D;

    Ok(match data_type {
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
        | D::Interval(TimeUnit::YearMonth) => JsValueHint::Number,
        // 64-bit and wider integers exceed the safe-integer range.
        D::Int64
        | D::UInt64
        | D::Timestamp(..)
        | D::Date64
        | D::Time64(_)
        | D::Duration(_)
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
        D::Utf8 | D::LargeUtf8 | D::Utf8View => JsValueHint::String,
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
        D::Dictionary(dictionary) => data_type_js_hint(dictionary.value())?,
        D::RunEndEncoded(encoded) => data_type_js_hint(encoded.values().data_type())?,
        other => {
            return Err(napi_error(format!(
                "expected a datatype with a JavaScript projection, got {other}"
            )));
        }
    })
}

/// Build the one-column root used to project a single value into JavaScript.
pub(crate) fn field_value_schema(field: &CoreField) -> Result<CoreField> {
    let root_data_type = DataType::from_fields([field.clone()]).map_err(napi_error)?;
    let root = CoreField::new("default", root_data_type, false);
    root.validate_struct_root().map_err(napi_error)?;
    Ok(root)
}

/// Project one value through its field, using a one-column root for paths.
pub(crate) fn field_value_to_js<'env>(
    env: &'env Env,
    field: &CoreField,
    value: &Value,
    root_schema: &CoreField,
) -> Result<Unknown<'env>> {
    value_to_js(env, field, value, root_schema, &[0])
}

fn value_to_js<'env>(
    env: &'env Env,
    field: &CoreField,
    value: &Value,
    root_schema: &CoreField,
    path: &[usize],
) -> Result<Unknown<'env>> {
    data_type_to_js(env, field.data_type(), value, root_schema, path)
}

fn data_type_to_js<'env>(
    env: &'env Env,
    data_type: &DataType,
    value: &Value,
    root_schema: &CoreField,
    path: &[usize],
) -> Result<Unknown<'env>> {
    use DataType as D;

    if matches!(value, Value::Null) {
        return Null.into_unknown(env);
    }
    match data_type {
        D::Null => Null.into_unknown(env),
        D::Boolean => value
            .as_bool()
            .ok_or_else(|| napi_error("invalid native boolean record value"))?
            .into_unknown(env),
        D::Int8 | D::Int16 | D::Int32 | D::Date32 | D::Time32(_) => {
            let integer = value
                .as_i128()
                .ok_or_else(|| napi_error("invalid native 32-bit integer record value"))?;
            let integer = i32::try_from(integer)
                .map_err(|_| napi_error("native integer is out of signed 32-bit range"))?;
            integer.into_unknown(env)
        }
        D::UInt8 | D::UInt16 | D::UInt32 => {
            let integer = value
                .as_u128()
                .ok_or_else(|| napi_error("invalid native unsigned record value"))?;
            let integer = u32::try_from(integer)
                .map_err(|_| napi_error("native integer is out of unsigned 32-bit range"))?;
            integer.into_unknown(env)
        }
        D::Int64 | D::Timestamp(..) | D::Date64 | D::Time64(_) | D::Duration(_) => value
            .as_i128()
            .ok_or_else(|| napi_error("invalid native signed integer record value"))
            .and_then(|value| BigInt::from(value).into_unknown(env)),
        D::UInt64 => value
            .as_u128()
            .ok_or_else(|| napi_error("invalid native unsigned integer record value"))
            .and_then(|value| BigInt::from(value).into_unknown(env)),
        D::Float16 | D::Float32 | D::Float64 => value
            .as_f64()
            .ok_or_else(|| napi_error("invalid native floating record value"))?
            .into_unknown(env),
        D::Interval(TimeUnit::YearMonth) => {
            let value = i32::try_from(
                value
                    .as_i128()
                    .ok_or_else(|| napi_error("invalid interval"))?,
            )
            .map_err(napi_error)?;
            value.into_unknown(env)
        }
        D::Interval(unit @ (TimeUnit::DayTime | TimeUnit::MonthDayNano)) => {
            integer_tuple_to_js(env, value, *unit)
        }
        D::Interval(_) => Err(napi_error("invalid native interval layout")),
        D::Binary | D::LargeBinary | D::BinaryView | D::FixedSizeBinary(_) => Buffer::from(
            value
                .as_bytes()
                .ok_or_else(|| napi_error("invalid native binary record value"))?
                .to_vec(),
        )
        .into_unknown(env),
        D::Utf8 | D::LargeUtf8 | D::Utf8View => value
            .as_str()
            .ok_or_else(|| napi_error("invalid native string record value"))?
            .to_owned()
            .into_unknown(env),
        D::List(item)
        | D::ListView(item)
        | D::FixedSizeList(item, _)
        | D::LargeList(item)
        | D::LargeListView(item) => {
            let child_path = appended_path(path, 0);
            sequence_to_js(env, item, value, root_schema, &child_path)
        }
        D::Struct(fields) => struct_to_js(env, fields, value, root_schema, path),
        D::Union(fields, _) => union_to_js(env, fields, value, root_schema, path),
        D::Dictionary(dictionary) => {
            data_type_to_js(env, dictionary.value(), value, root_schema, path)
        }
        D::Decimal32 { .. } | D::Decimal64 { .. } | D::Decimal128 { .. } => value
            .as_i128()
            .ok_or_else(|| napi_error("invalid native decimal record value"))
            .and_then(|value| BigInt::from(value).into_unknown(env)),
        D::Decimal256 { .. } => decimal256_to_js(env, value),
        D::Map(map) => map_to_js(env, map, value, root_schema, path),
        D::RunEndEncoded(encoded) => value_to_js(env, encoded.values(), value, root_schema, path),
        // A geospatial value is its Well-Known Binary payload, so it crosses
        // exactly as the binary family does.
        D::Geometry(_) | D::Geography(_) => Buffer::from(
            value
                .as_wkb()
                .ok_or_else(|| napi_error("invalid native geospatial record value"))?
                .to_vec(),
        )
        .into_unknown(env),
        // A non-null variant value crosses as the Parquet Variant binary
        // encoding, which the Iceberg v3 layer owns; refuse by name until
        // that codec lands rather than inventing a second encoding here.
        D::Variant => Err(napi_error(
            "unsupported native datatype variant: \
             the variant binary encoding lands with the Iceberg v3 layer",
        )),
        _ => Err(napi_error(format!(
            "unsupported native datatype {data_type}"
        ))),
    }
}

fn integer_tuple_to_js<'env>(
    env: &'env Env,
    value: &Value,
    unit: TimeUnit,
) -> Result<Unknown<'env>> {
    let values = value
        .as_sequence()
        .ok_or_else(|| napi_error("invalid native interval tuple"))?;
    let mut output = env.create_array(u32::try_from(values.len()).unwrap_or(u32::MAX))?;
    for (index, value) in values.iter().enumerate() {
        let js_index = u32::try_from(index)
            .map_err(|_| napi_error("interval component index exceeds JavaScript limits"))?;
        let integer = value
            .as_i128()
            .ok_or_else(|| napi_error("invalid native interval component"))?;
        if unit == TimeUnit::MonthDayNano && index == 2 {
            output.set(js_index, BigInt::from(integer))?;
        } else {
            output.set(
                js_index,
                i32::try_from(integer)
                    .map_err(|_| napi_error("native interval component exceeds int32"))?,
            )?;
        }
    }
    output.into_unknown(env)
}

fn sequence_to_js<'env>(
    env: &'env Env,
    field: &CoreField,
    value: &Value,
    root_schema: &CoreField,
    path: &[usize],
) -> Result<Unknown<'env>> {
    let values = value
        .as_sequence()
        .ok_or_else(|| napi_error("invalid native list record value"))?;
    let mut output = env.create_array(u32::try_from(values.len()).unwrap_or(u32::MAX))?;
    for (index, value) in values.iter().enumerate() {
        let js_index = u32::try_from(index)
            .map_err(|_| napi_error("list index exceeds the JavaScript array limit"))?;
        output.set(
            js_index,
            projected_value_to_js(env, field, value, root_schema, path)?,
        )?;
    }
    output.into_unknown(env)
}

fn projected_value_to_js<'env>(
    env: &'env Env,
    field: &CoreField,
    value: &Value,
    root_schema: &CoreField,
    path: &[usize],
) -> Result<Unknown<'env>> {
    if matches!(value, Value::Null) {
        return Null.into_unknown(env);
    }
    value_to_js(env, field, value, root_schema, path)
}

/// Project one struct value as a positional JavaScript array.
fn struct_to_js<'env>(
    env: &'env Env,
    fields: &yggdryl::Fields,
    value: &Value,
    root_schema: &CoreField,
    path: &[usize],
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
        let child_path = appended_path(path, index);
        output.set(
            js_index,
            projected_value_to_js(env, field, value, root_schema, &child_path)?,
        )?;
    }
    output.into_unknown(env)
}

fn union_to_js<'env>(
    env: &'env Env,
    fields: &yggdryl::UnionFields,
    value: &Value,
    root_schema: &CoreField,
    path: &[usize],
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
    let (field_index, (_, field)) = fields
        .iter()
        .enumerate()
        .find(|(_, (candidate, _))| *candidate == type_id)
        .ok_or_else(|| napi_error(format!("unknown native union type id {type_id}")))?;
    let child_path = appended_path(path, field_index);
    let mut output = Object::new(env)?;
    output.set("typeId", i32::from(type_id))?;
    output.set(
        "value",
        value_to_js(env, field, payload, root_schema, &child_path)?,
    )?;
    output.into_unknown(env)
}

fn decimal256_to_js<'env>(env: &'env Env, value: &Value) -> Result<Unknown<'env>> {
    let encoded = value
        .as_str()
        .ok_or_else(|| napi_error("invalid native decimal256 coefficient string"))?;
    let global = env.get_global()?;
    let bigint: Function<'_, FnArgs<(String,)>, Unknown<'_>> =
        global.get_named_property("BigInt")?;
    bigint.call((encoded.to_owned(),).into())
}

fn map_to_js<'env>(
    env: &'env Env,
    map: &yggdryl::MapType,
    value: &Value,
    root_schema: &CoreField,
    path: &[usize],
) -> Result<Unknown<'env>> {
    let entries = value
        .as_mapping()
        .ok_or_else(|| napi_error("invalid native map record value"))?;
    let fields = map
        .entries()
        .data_type()
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
    let entry_path = appended_path(path, 0);
    let key_path = appended_path(&entry_path, 0);
    let value_path = appended_path(&entry_path, 1);
    for (key, value) in entries {
        let key = projected_value_to_js(env, key_field, key, root_schema, &key_path)?;
        if has.apply(object, (key,).into())? {
            return Err(napi_error(
                "native map contains distinct keys that collide under JavaScript Map semantics",
            ));
        }
        let value = projected_value_to_js(env, value_field, value, root_schema, &value_path)?;
        set.apply(object, (key, value).into())?;
    }
    map_value.into_unknown(env)
}

fn appended_path(path: &[usize], child: usize) -> Vec<usize> {
    let mut output = Vec::with_capacity(path.len() + 1);
    output.extend_from_slice(path);
    output.push(child);
    output
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

    let schema = Arc::new(Schema::new([field.to_arrow_ref().map_err(napi_error)?]));
    let options = RecordBatchOptions::new().with_row_count(Some(1));
    let batch =
        RecordBatch::try_new_with_options(schema, vec![array], &options).map_err(napi_error)?;
    let mut writer =
        StreamWriter::try_new(Vec::new(), batch.schema().as_ref()).map_err(napi_error)?;
    writer.write(&batch).map_err(napi_error)?;
    writer.finish().map_err(napi_error)?;
    Ok(writer.into_inner().map_err(napi_error)?.into())
}
