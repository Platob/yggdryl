//! The `Serie` napi class — a named, typed, Arrow-backed column (a single dataframe
//! column). A thin wrapper over [`yggdryl_serie`]'s `SerieRef`; all logic lives in the
//! core, so the Node and Python bindings behave identically.

use std::sync::Arc;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::Value as JsonValue;
use yggdryl_schema::DataType as CoreDataType;
use yggdryl_serie::arrow_array::{
    ArrayRef, BinaryArray, BooleanArray, Float64Array, Int64Array, StringArray,
};
use yggdryl_serie::{
    from_array, from_bytes, CategoricalSerie, DisplayOptions, IndexSerie, RangeSerie, Scalar,
    SerieRef, StructSerie,
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
    /// (`boolean` → bool, `number` → int64 if all integral else float64, `string` →
    /// utf8); pass `dtype` (a `DataType` or type string) to cast to a specific type. An
    /// empty / all-null array needs an explicit `dtype`. Use `Serie.binary` for bytes.
    #[napi(constructor)]
    pub fn new(
        name: String,
        values: Vec<JsonValue>,
        dtype: Option<Either<&DataType, String>>,
    ) -> Result<Self> {
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
        wrap(Arc::new(RangeSerie::new(
            name,
            start,
            step,
            length as usize,
        )))
    }

    /// A lazy `uint64` row index of `length` rows (`0..length`).
    #[napi(factory)]
    pub fn index(length: u32) -> Self {
        wrap(Arc::new(IndexSerie::range(length as usize)))
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
        let dt = resolve_dtype(dtype)?;
        self.inner.cast(&dt).map(wrap).map_err(err)
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
