//! The `Serie` napi class — a named, typed, Arrow-backed column (a single dataframe
//! column). A thin wrapper over [`yggdryl_serie`]'s `SerieRef`; all logic lives in the
//! core, so the Node and Python bindings behave identically.

use std::sync::Arc;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::Value as JsonValue;
use yggdryl_scalar::ScalarValue;
use yggdryl_schema::DataType as CoreDataType;
use yggdryl_serie::arrow_array::{
    ArrayRef, BinaryArray, BooleanArray, Float64Array, Int64Array, StringArray,
};
use yggdryl_serie::{
    from_array, from_bytes, CategoricalSerie, DisplayOptions, ListSerie, MapSerie, Scalar,
    SerieRef, StructSerie, UInt64RangeSerie,
};

use crate::datatype::DataType;
use crate::err;
use crate::field::Field;

/// A named, typed, Arrow-backed column. Build one from an array of values
/// (`new Serie("n", [1, 2, 3])`), a lazy range (`Serie.range`) or child columns
/// (`Serie.struct`); read it by index, slice / resize / cast it, navigate nested
/// children, and round-trip it losslessly through `toBytes`.
#[napi]
pub struct Serie {
    pub(crate) inner: SerieRef,
}

fn wrap(inner: SerieRef) -> Serie {
    Serie { inner }
}

/// Borrows a column as a [`StructSerie`] frame, or throws if it is not a struct column —
/// the gate for the frame (DataFrame) operations.
fn as_frame(serie: &SerieRef) -> Result<&StructSerie> {
    serie
        .as_any()
        .downcast_ref::<StructSerie>()
        .ok_or_else(|| err("not a struct column; build a frame with Serie.struct(...)"))
}

/// Borrows a column as a [`UInt64RangeSerie`], or throws if it is not a range/index
/// column — the gate for the index (label ↔ position) operations.
fn as_index(serie: &SerieRef) -> Result<&UInt64RangeSerie> {
    serie
        .as_any()
        .downcast_ref::<UInt64RangeSerie>()
        .ok_or_else(|| {
            err("not a range/index column; build one with Serie.range(...) or Serie.index(...)")
        })
}

/// Borrows a column as a [`CategoricalSerie`], or throws if it is not a categorical
/// column — the gate for the dictionary (category / code) operations.
fn as_categorical(serie: &SerieRef) -> Result<&CategoricalSerie> {
    serie
        .as_any()
        .downcast_ref::<CategoricalSerie>()
        .ok_or_else(|| err("not a categorical column; build one with serie.categorical()"))
}

/// The element kind inferred from a JS array.
enum Kind {
    Bool,
    Num,
    Str,
}

/// Classifies one non-null JSON value into a column [`Kind`] (nested / binary values are
/// not inferable from a plain array — use `Serie.binary` or `Serie.fromBytes`).
fn classify(value: &JsonValue) -> Result<Kind> {
    match value {
        JsonValue::Bool(_) => Ok(Kind::Bool),
        JsonValue::Number(_) => Ok(Kind::Num),
        JsonValue::String(_) => Ok(Kind::Str),
        _ => Err(err(
            "unsupported serie value; the generic constructor takes boolean / number / \
             string / null — use Serie.binary for bytes or Serie.fromBytes",
        )),
    }
}

/// Builds the Arrow array from a JS array of values (`null` → null). Returns `None` when
/// every value is null (the caller then requires an explicit `dtype`). A numeric column
/// of all-integral values becomes `int64`, otherwise `float64`.
fn build_array(values: &[JsonValue]) -> Result<Option<ArrayRef>> {
    let mut kind = None;
    for value in values {
        if !value.is_null() {
            kind = Some(classify(value)?);
            break;
        }
    }
    let Some(kind) = kind else {
        return Ok(None);
    };
    let array: ArrayRef = match kind {
        Kind::Bool => {
            let out: Vec<Option<bool>> = values
                .iter()
                .map(|v| {
                    if v.is_null() {
                        Ok(None)
                    } else {
                        v.as_bool()
                            .map(Some)
                            .ok_or_else(|| err("mixed serie value types: expected a boolean"))
                    }
                })
                .collect::<Result<_>>()?;
            Arc::new(BooleanArray::from(out))
        }
        Kind::Num => {
            // f64 view of every value; all-integral → int64, else float64.
            let nums: Vec<Option<f64>> = values
                .iter()
                .map(|v| {
                    if v.is_null() {
                        Ok(None)
                    } else {
                        v.as_f64()
                            .map(Some)
                            .ok_or_else(|| err("mixed serie value types: expected a number"))
                    }
                })
                .collect::<Result<_>>()?;
            let integral = nums
                .iter()
                .flatten()
                .all(|n| n.is_finite() && n.fract() == 0.0);
            if integral {
                let ints: Vec<Option<i64>> = nums.iter().map(|n| n.map(|x| x as i64)).collect();
                Arc::new(Int64Array::from(ints))
            } else {
                Arc::new(Float64Array::from(nums))
            }
        }
        Kind::Str => {
            let out: Vec<Option<String>> = values
                .iter()
                .map(|v| {
                    if v.is_null() {
                        Ok(None)
                    } else {
                        v.as_str()
                            .map(|s| Some(s.to_string()))
                            .ok_or_else(|| err("mixed serie value types: expected a string"))
                    }
                })
                .collect::<Result<_>>()?;
            Arc::new(StringArray::from_iter(out))
        }
    };
    Ok(Some(array))
}

/// Resolves a `DataType` class **or** a type string to a core [`CoreDataType`].
fn resolve_dtype(dtype: Either<&DataType, String>) -> Result<CoreDataType> {
    match dtype {
        Either::A(dt) => Ok(dt.inner.clone()),
        Either::B(text) => CoreDataType::from_str(&text).map_err(err),
    }
}

/// Maps a core [`Scalar`] to a JSON value (which napi converts to the JS value): a binary
/// cell becomes a byte array, an out-of-`i64`-range integer a string.
fn scalar_to_json(scalar: &Scalar) -> JsonValue {
    match scalar {
        Scalar::Null => JsonValue::Null,
        Scalar::Boolean(b) => JsonValue::Bool(*b),
        Scalar::Int(i) => i64::try_from(*i)
            .map(JsonValue::from)
            .unwrap_or_else(|_| JsonValue::String(i.to_string())),
        Scalar::Float(f) => serde_json::Number::from_f64(*f)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        Scalar::Utf8(s) => JsonValue::String(s.clone()),
        Scalar::Binary(b) => JsonValue::Array(b.iter().map(|x| JsonValue::from(*x)).collect()),
        Scalar::Other(s) => JsonValue::String(s.clone()),
    }
}

/// Lowercase-hex encoding of the IPC bytes used by `toJSON` / `fromJSON`.
fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Decodes the lowercase-hex string produced by `toJSON`.
fn from_hex(text: &str) -> Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return Err(err("invalid serie hex: odd length"));
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| err("invalid serie hex")))
        .collect()
}

#[napi]
impl Serie {
    /// Build a column named `name` from an array of values. The Arrow type is inferred
    /// from the first non-null value: a scalar gives `boolean` → bool / `number` → int64
    /// if all integral else float64 / `string` → utf8, a nested **array** gives a list
    /// column and an **object** gives a map column (recursively — an array of objects is
    /// `list<map>`, etc.). Pass `dtype` (a `DataType` or type string) to cast the leaf
    /// type. An empty / all-null array needs an explicit `dtype`. Use `Serie.binary` for
    /// bytes.
    #[napi(constructor)]
    pub fn new(
        name: String,
        values: Vec<JsonValue>,
        dtype: Option<Either<&DataType, String>>,
    ) -> Result<Self> {
        // A nested first value infers a list / map column (the element builder is this
        // same constructor, so arbitrarily deep nesting resolves on its own).
        match values.iter().find(|v| !v.is_null()) {
            Some(JsonValue::Array(_)) => return Serie::list_js(name, values, dtype),
            Some(JsonValue::Object(_)) => return Serie::map_js(name, values, None, dtype),
            _ => {}
        }

        let base = build_array(&values)?;
        let inferred = base.is_some();
        let array = base.unwrap_or_else(|| {
            Arc::new(Int64Array::from(vec![None::<i64>; values.len()])) as ArrayRef
        });
        let serie = from_array(&name, array).map_err(err)?;
        match dtype {
            Some(dtype) => {
                let dt = resolve_dtype(dtype)?;
                Ok(wrap(serie.cast(&dt).map_err(err)?))
            }
            None if !inferred && !values.is_empty() => Err(err(
                "cannot infer a dtype from all-null values; pass a dtype",
            )),
            None => Ok(wrap(serie)),
        }
    }

    /// Alias of the constructor — build a column from an array of values.
    #[napi(factory, js_name = "fromValues")]
    pub fn from_values(
        name: String,
        values: Vec<JsonValue>,
        dtype: Option<Either<&DataType, String>>,
    ) -> Result<Self> {
        Serie::new(name, values, dtype)
    }

    /// Build a binary column named `name` from an array of `Buffer | null` values.
    #[napi(factory)]
    pub fn binary(name: String, values: Vec<Option<Buffer>>) -> Result<Self> {
        let bytes: Vec<Option<Vec<u8>>> =
            values.into_iter().map(|b| b.map(|b| b.to_vec())).collect();
        let array: ArrayRef = Arc::new(BinaryArray::from_iter(bytes));
        from_array(&name, array).map(wrap).map_err(err)
    }

    /// Reconstruct a column from its Arrow-IPC `toBytes` form.
    #[napi(factory, js_name = "fromBytes")]
    pub fn from_bytes_js(data: Buffer) -> Result<Self> {
        from_bytes(&data).map(wrap).map_err(err)
    }

    /// A lazy `uint64` arithmetic range column (`start + i*step`), not materialised.
    #[napi(factory)]
    pub fn range(length: u32, start: Option<i64>, step: Option<i64>, name: Option<String>) -> Self {
        let start = start.unwrap_or(0).max(0) as u64;
        let step = step.unwrap_or(1).max(0) as u64;
        let name = name.unwrap_or_else(|| "range".to_string());
        wrap(Arc::new(UInt64RangeSerie::new(
            name,
            start,
            step,
            length as usize,
        )))
    }

    /// A lazy `uint64` row index of `length` rows (`0..length`) — a `UInt64RangeSerie`
    /// with the label ↔ position lookups.
    #[napi(factory)]
    pub fn index(length: u32) -> Self {
        wrap(Arc::new(UInt64RangeSerie::indices(length as usize)))
    }

    /// Build a struct column named `name` from its child columns (each child's field,
    /// including its name, becomes a struct field). The children stay lazy until
    /// `materialize`.
    #[napi(factory, js_name = "struct")]
    pub fn struct_js(name: String, children: Vec<&Serie>) -> Result<Self> {
        let refs: Vec<SerieRef> = children.iter().map(|c| c.inner.clone()).collect();
        StructSerie::from_children(name, refs)
            .map(|s| wrap(Arc::new(s)))
            .map_err(err)
    }

    /// Build a **list** column named `name` from an array of sub-arrays (each row an array
    /// of elements, or `null` for a null row). The element type is inferred like the
    /// `Serie` constructor; pass `dtype` to cast the elements. An empty / all-empty input
    /// needs an explicit element `dtype`.
    #[napi(factory, js_name = "list")]
    pub fn list_js(
        name: String,
        values: Vec<JsonValue>,
        dtype: Option<Either<&DataType, String>>,
    ) -> Result<Self> {
        let mut flat: Vec<JsonValue> = Vec::new();
        let mut lengths: Vec<Option<usize>> = Vec::with_capacity(values.len());
        for row in values {
            match row {
                JsonValue::Null => lengths.push(None),
                JsonValue::Array(items) => {
                    lengths.push(Some(items.len()));
                    flat.extend(items);
                }
                _ => return Err(err("each list row must be an array of elements or null")),
            }
        }
        let items = Serie::new("item".to_string(), flat, dtype)?;
        ListSerie::<i32>::from_values(name, items.inner, &lengths)
            .map(|s| wrap(Arc::new(s)))
            .map_err(err)
    }

    /// Build a **map** column named `name` from an array of objects (each row an object of
    /// key → value, or `null` for a null row). Keys are strings; the value type is
    /// inferred like the `Serie` constructor (pass `valueDtype` to cast it, `keyDtype` for
    /// parity with the Python binding).
    #[napi(factory, js_name = "map")]
    pub fn map_js(
        name: String,
        entries: Vec<JsonValue>,
        key_dtype: Option<Either<&DataType, String>>,
        value_dtype: Option<Either<&DataType, String>>,
    ) -> Result<Self> {
        let mut keys: Vec<JsonValue> = Vec::new();
        let mut vals: Vec<JsonValue> = Vec::new();
        let mut lengths: Vec<Option<usize>> = Vec::with_capacity(entries.len());
        for row in entries {
            match row {
                JsonValue::Null => lengths.push(None),
                JsonValue::Object(map) => {
                    lengths.push(Some(map.len()));
                    for (k, v) in map {
                        keys.push(JsonValue::String(k));
                        vals.push(v);
                    }
                }
                _ => return Err(err("each map row must be an object or null")),
            }
        }
        let key_col = Serie::new("key".to_string(), keys, key_dtype)?;
        let value_col = Serie::new("value".to_string(), vals, value_dtype)?;
        MapSerie::from_values(name, key_col.inner, value_col.inner, &lengths)
            .map(|s| wrap(Arc::new(s)))
            .map_err(err)
    }

    // ---- metadata ----

    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_string()
    }

    #[napi(getter, js_name = "dataType")]
    pub fn data_type(&self) -> DataType {
        DataType {
            inner: self.inner.data_type().clone(),
        }
    }

    #[napi(getter)]
    pub fn field(&self) -> Field {
        Field {
            inner: self.inner.field().clone(),
        }
    }

    /// The type category (`primitive` / `logical` / `nested` / `any`).
    #[napi(getter)]
    pub fn category(&self) -> String {
        self.inner.data_type().category().as_str().to_string()
    }

    #[napi(getter)]
    pub fn nullable(&self) -> bool {
        self.inner.is_nullable()
    }

    #[napi(getter, js_name = "numRows")]
    pub fn num_rows(&self) -> u32 {
        self.inner.num_rows() as u32
    }

    #[napi(getter, js_name = "nullCount")]
    pub fn null_count(&self) -> u32 {
        self.inner.null_count() as u32
    }

    #[napi(getter, js_name = "isMaterialized")]
    pub fn is_materialized(&self) -> bool {
        self.inner.is_materialized()
    }

    #[napi(js_name = "isNull")]
    pub fn is_null(&self, index: u32) -> bool {
        self.inner.is_null(index as usize)
    }

    #[napi(js_name = "isValid")]
    pub fn is_valid(&self, index: u32) -> bool {
        self.inner.is_valid(index as usize)
    }

    // ---- values ----

    /// The value at `index` (`null` for a null or out-of-bounds cell).
    #[napi(js_name = "valueAt")]
    pub fn value_at(&self, index: u32) -> JsonValue {
        scalar_to_json(&self.inner.value_at(index as usize))
    }

    /// The value at `index`, supporting negative indices; throws on out of range.
    #[napi]
    pub fn get(&self, index: i64) -> Result<JsonValue> {
        let len = self.inner.len() as i64;
        let idx = if index < 0 { index + len } else { index };
        if idx < 0 || idx >= len {
            return Err(Error::new(
                Status::GenericFailure,
                "serie index out of range",
            ));
        }
        Ok(scalar_to_json(&self.inner.value_at(idx as usize)))
    }

    /// Every value as an array.
    #[napi(js_name = "toList")]
    pub fn to_list(&self) -> Vec<JsonValue> {
        (0..self.inner.len())
            .map(|i| scalar_to_json(&self.inner.value_at(i)))
            .collect()
    }

    /// A copy of the column with the cell at `index` replaced by `value` (a `Scalar`).
    /// With `safe` (default `true`) the value is cast to the column's type first, so any
    /// value can be written. Functional — returns a new column.
    #[napi(js_name = "setAt")]
    pub fn set_at(
        &self,
        index: u32,
        value: &crate::scalar::Scalar,
        safe: Option<bool>,
    ) -> Result<Self> {
        let scalar = value.inner.clone().into_scalar();
        self.inner
            .set_at(index as usize, scalar.as_ref(), safe.unwrap_or(true))
            .map(wrap)
            .map_err(err)
    }

    /// A copy of the column with `value` (a `Scalar`) appended as a new last row.
    #[napi]
    pub fn push(&self, value: &crate::scalar::Scalar, safe: Option<bool>) -> Result<Self> {
        let scalar = value.inner.clone().into_scalar();
        self.inner
            .push(scalar.as_ref(), safe.unwrap_or(true))
            .map(wrap)
            .map_err(err)
    }

    // ---- shape ----

    /// A zero-copy slice of `length` values starting at `offset`.
    #[napi]
    pub fn slice(&self, offset: u32, length: u32) -> Self {
        wrap(self.inner.slice(offset as usize, length as usize))
    }

    /// The first `n` rows (a zero-copy slice).
    #[napi]
    pub fn head(&self, n: u32) -> Self {
        wrap(self.inner.slice(0, (n as usize).min(self.inner.len())))
    }

    /// A column of length `newLen`: a slice when shrinking, or extended with fill (nulls
    /// if nullable, else the type default) when growing.
    #[napi]
    pub fn resize(&self, new_len: u32) -> Result<Self> {
        self.inner.resize(new_len as usize).map(wrap).map_err(err)
    }

    // ---- transform ----

    /// Cast the column to `dtype` (a `DataType` or type string), converting the values
    /// (lossy / narrowing casts yield null on overflow).
    #[napi]
    pub fn cast(&self, dtype: Either<&DataType, String>) -> Result<Self> {
        // A `DataType` casts directly; a type string leverages the core's `cast_str`
        // (one `DataType::from_str` path), so both spellings share one implementation.
        match dtype {
            Either::A(dt) => self.inner.cast(&dt.inner).map(wrap).map_err(err),
            Either::B(text) => self.inner.cast_str(&text).map(wrap).map_err(err),
        }
    }

    /// A dictionary-encoded (categorical) view of the column for repeated values.
    #[napi]
    pub fn categorical(&self) -> Result<Self> {
        CategoricalSerie::from_serie(self.inner.as_ref())
            .map(|c| wrap(Arc::new(c)))
            .map_err(err)
    }

    /// A fully-materialised, independent copy (a lazy column is computed into a real
    /// array).
    #[napi]
    pub fn materialize(&self) -> Self {
        wrap(self.inner.materialize())
    }

    // ---- nested ----

    /// Navigate a child node path (`"a.b.c"`, `"tags.0"`, `'["a.b"].c'`) into a
    /// descendant column. Returns `null` for a leaf column or an unresolved path; throws
    /// on a malformed path.
    #[napi]
    pub fn select(&self, path: String) -> Result<Option<Serie>> {
        self.inner
            .select(&path)
            .map(|opt| opt.map(wrap))
            .map_err(err)
    }

    /// A child column by index (number) or by name (string, case-sensitive then
    /// -insensitive), or `null`.
    #[napi]
    pub fn child(&self, key: Either<u32, String>) -> Option<Serie> {
        let nested = self.inner.as_nested()?;
        let found = match key {
            Either::A(index) => nested.child(index as usize),
            Either::B(name) => nested.child_by_name(&name),
        };
        found.map(wrap)
    }

    /// All child columns (empty unless this is a nested column).
    #[napi]
    pub fn children(&self) -> Vec<Serie> {
        match self.inner.as_nested() {
            Some(nested) => nested.children().into_iter().map(wrap).collect(),
            None => Vec::new(),
        }
    }

    // ---- range / index ----

    /// Whether this is a canonical `0..len` `uint64` range index (`start == 0`, `step ==
    /// 1`, the implicit row index) — `false` for any other column.
    #[napi(getter, js_name = "isRange")]
    pub fn is_range(&self) -> bool {
        self.inner
            .as_any()
            .downcast_ref::<UInt64RangeSerie>()
            .is_some_and(UInt64RangeSerie::is_range)
    }

    /// The integer label at row `index` (`null` when out of bounds). Requires a
    /// range/index column.
    #[napi]
    pub fn at(&self, index: u32) -> Result<Option<i64>> {
        Ok(as_index(&self.inner)?.at(index as usize).map(|v| v as i64))
    }

    /// The first row whose label equals `label`, or `null`.
    #[napi]
    pub fn position(&self, label: i64) -> Result<Option<u32>> {
        if label < 0 {
            return Ok(None);
        }
        Ok(as_index(&self.inner)?
            .position(label as u64)
            .map(|p| p as u32))
    }

    /// Whether `label` is one of the index labels.
    #[napi]
    pub fn contains(&self, label: i64) -> Result<bool> {
        if label < 0 {
            return Ok(false);
        }
        Ok(as_index(&self.inner)?.contains(label as u64))
    }

    // ---- categorical ----

    /// The number of distinct categories. Throws if the column is not categorical.
    #[napi(getter, js_name = "categoryCount")]
    pub fn category_count(&self) -> Result<u32> {
        Ok(as_categorical(&self.inner)?.category_count() as u32)
    }

    /// The distinct values (the dictionary) as a column named `"categories"`.
    #[napi]
    pub fn categories(&self) -> Result<Serie> {
        as_categorical(&self.inner)?
            .categories()
            .map(wrap)
            .map_err(err)
    }

    /// The dictionary **code** at row `index` (`null` when null / out of bounds).
    #[napi(js_name = "codeAt")]
    pub fn code_at(&self, index: u32) -> Result<Option<i32>> {
        Ok(as_categorical(&self.inner)?.code_at(index as usize))
    }

    // ---- frame (DataFrame) ----

    /// The frame shape as `[rows, columns]` (struct columns only).
    #[napi(getter)]
    pub fn shape(&self) -> Result<Vec<u32>> {
        let (rows, cols) = as_frame(&self.inner)?.shape();
        Ok(vec![rows as u32, cols as u32])
    }

    /// The number of columns (struct columns only).
    #[napi(getter, js_name = "numColumns")]
    pub fn num_columns(&self) -> Result<u32> {
        Ok(as_frame(&self.inner)?.num_columns() as u32)
    }

    /// The column names, in order (struct columns only).
    #[napi(getter, js_name = "columnNames")]
    pub fn column_names(&self) -> Result<Vec<String>> {
        Ok(as_frame(&self.inner)?
            .column_names()
            .iter()
            .map(|s| s.to_string())
            .collect())
    }

    /// Project the frame to the named columns, in the requested order.
    #[napi(js_name = "selectColumns")]
    pub fn select_columns(&self, names: Vec<String>) -> Result<Self> {
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        as_frame(&self.inner)?
            .select_columns(&refs)
            .map(|s| wrap(Arc::new(s)))
            .map_err(err)
    }

    /// Project **and cast** the frame to an explicit list of `Field`s: each takes the
    /// source column of the same name cast to its type (or a filled column if absent), in
    /// the target order, dropping unlisted columns.
    #[napi(js_name = "selectFields")]
    pub fn select_fields(&self, fields: Vec<&Field>) -> Result<Self> {
        let fields: Vec<yggdryl_schema::Field> = fields.iter().map(|f| f.inner.clone()).collect();
        as_frame(&self.inner)?
            .select_fields(fields)
            .map(|s| wrap(Arc::new(s)))
            .map_err(err)
    }

    /// A new frame with `column` appended (or replacing an existing column of the same
    /// name). The column length must match the frame's row count.
    #[napi(js_name = "withColumn")]
    pub fn with_column(&self, column: &Serie) -> Result<Self> {
        as_frame(&self.inner)?
            .with_column(column.inner.clone())
            .map(|s| wrap(Arc::new(s)))
            .map_err(err)
    }

    /// A new frame without the named columns (absent names are ignored).
    #[napi(js_name = "dropColumns")]
    pub fn drop_columns(&self, names: Vec<String>) -> Result<Self> {
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        as_frame(&self.inner)?
            .drop_columns(&refs)
            .map(|s| wrap(Arc::new(s)))
            .map_err(err)
    }

    /// A new frame with column `old` renamed to `newName` (a no-op if `old` is absent).
    #[napi]
    pub fn rename(&self, old: String, new_name: String) -> Result<Self> {
        as_frame(&self.inner)?
            .rename(&old, &new_name)
            .map(|s| wrap(Arc::new(s)))
            .map_err(err)
    }

    /// The last `n` rows, as a new frame (a zero-copy row slice).
    #[napi]
    pub fn tail(&self, n: u32) -> Result<Self> {
        as_frame(&self.inner)?
            .tail(n as usize)
            .map(|s| wrap(Arc::new(s)))
            .map_err(err)
    }

    /// Keep the rows where `mask` is `true` (the mask length must equal the row count).
    #[napi]
    pub fn filter(&self, mask: Vec<bool>) -> Result<Self> {
        as_frame(&self.inner)?
            .filter(&mask)
            .map(|s| wrap(Arc::new(s)))
            .map_err(err)
    }

    /// A new frame with the rows sorted by column `column` (ascending unless `descending`),
    /// reordering every column by the same permutation.
    #[napi(js_name = "sortBy")]
    pub fn sort_by(&self, column: String, descending: Option<bool>) -> Result<Self> {
        as_frame(&self.inner)?
            .sort_by(&column, descending.unwrap_or(false))
            .map(|s| wrap(Arc::new(s)))
            .map_err(err)
    }

    /// Stack `other`'s rows below this frame's (both must share column names and types).
    #[napi]
    pub fn vstack(&self, other: &Serie) -> Result<Self> {
        let other = as_frame(&other.inner)?;
        as_frame(&self.inner)?
            .vstack(other)
            .map(|s| wrap(Arc::new(s)))
            .map_err(err)
    }

    /// A new frame with a `0..rows` integer index column named `name` prepended (a lazy
    /// `uint64` range, so it costs nothing until materialised).
    #[napi(js_name = "withRowIndex")]
    pub fn with_row_index(&self, name: String) -> Result<Self> {
        as_frame(&self.inner)?
            .with_row_index(&name)
            .map(|s| wrap(Arc::new(s)))
            .map_err(err)
    }

    /// The record at `index` as a `Scalar` struct — one typed value per column.
    #[napi]
    pub fn row(&self, index: u32) -> Result<crate::scalar::Scalar> {
        let record = as_frame(&self.inner)?.row(index as usize).map_err(err)?;
        Ok(crate::scalar::Scalar {
            inner: record.into(),
        })
    }

    /// The frame's rows as an array of native objects (`[{ col: value, ... }, ...]`) — the
    /// pandas / polars `toDicts` projection.
    #[napi(js_name = "toDicts")]
    pub fn to_dicts(&self) -> Result<Vec<JsonValue>> {
        let frame = as_frame(&self.inner)?;
        let rows = frame.shape().0;
        (0..rows)
            .map(|i| {
                let row: ScalarValue = frame.row(i).map_err(err)?.into();
                Ok(crate::scalar::value_to_json(&row))
            })
            .collect()
    }

    /// The frame as an **Arrow IPC stream** — bytes any Arrow library reads back as a
    /// multi-column table.
    #[napi(js_name = "toArrowIpc")]
    pub fn to_arrow_ipc(&self) -> Result<Buffer> {
        as_frame(&self.inner)?
            .to_ipc_bytes()
            .map(Buffer::from)
            .map_err(err)
    }

    /// Build a frame named `name` from an **Arrow IPC stream** (as written by
    /// `toArrowIpc` or any Arrow library).
    #[napi(factory, js_name = "fromArrowIpc")]
    pub fn from_arrow_ipc(name: String, data: Buffer) -> Result<Self> {
        StructSerie::from_ipc_bytes(name, &data)
            .map(|s| wrap(Arc::new(s)))
            .map_err(err)
    }

    // ---- display / serialisation ----

    /// Render the column to a readable string.
    #[napi]
    pub fn display(
        &self,
        max_rows: Option<u32>,
        header: Option<bool>,
        width: Option<u32>,
    ) -> String {
        let mut opts = DisplayOptions::default().with_header(header.unwrap_or(true));
        if let Some(m) = max_rows {
            opts = opts.with_max_rows(m as usize);
        }
        if let Some(w) = width {
            opts = opts.with_width(w as usize);
        }
        self.inner.display(&opts)
    }

    /// Serialise to lossless Arrow-IPC bytes (round-trips via `fromBytes`).
    #[napi(js_name = "toBytes")]
    pub fn to_bytes(&self) -> Result<Buffer> {
        self.inner.to_bytes().map(Buffer::from).map_err(err)
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner
            .display(&DisplayOptions::default().with_max_rows(10))
    }

    /// Value equality: same name / type and the same values.
    #[napi]
    pub fn equals(&self, other: &Serie) -> bool {
        if self.inner.field() != other.inner.field() || self.inner.len() != other.inner.len() {
            return false;
        }
        (0..self.inner.len()).all(|i| self.inner.value_at(i) == other.inner.value_at(i))
    }

    /// Serialise to a lossless string (hex of the Arrow-IPC bytes) for `JSON.stringify`.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> Result<String> {
        self.inner.to_bytes().map(|b| to_hex(&b)).map_err(err)
    }

    /// Reconstruct from the string produced by `toJSON`.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: String) -> Result<Self> {
        from_bytes(&from_hex(&value)?).map(wrap).map_err(err)
    }
}
