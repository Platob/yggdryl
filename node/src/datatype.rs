//! Node.js view of the native `DataType` domain.

use napi::bindgen_prelude::{ClassInstance, Either, Either3, Env, Error, Result, Unknown};
use napi_derive::napi;
use yggdryl::{
    DataType as CoreDataType, EdgeAlgorithm as CoreEdgeAlgorithm, Field as CoreField,
    Scheme as CoreScheme, TimeUnit as CoreTimeUnit, UnionMode as CoreUnionMode,
};

use crate::{
    JsDifferenceIterator, exact_i8, exact_i32, exact_u8,
    field::JsField,
    napi_error, ordering_value,
    record::arrow_scalar_to_ipc,
    record::{JsValueHint, data_type_js_hint, field_value_schema, field_value_to_js},
};

pub(crate) fn data_type_from_input(
    value: Either<ClassInstance<'_, JsDataType>, String>,
) -> Result<CoreDataType> {
    match value {
        Either::A(value) => Ok(value.inner.clone()),
        Either::B(value) => CoreDataType::from_str(&value).map_err(napi_error),
    }
}

/// An Arrow-equivalent logical data type backed entirely by Rust.
#[napi(js_name = "DataType")]
pub struct JsDataType {
    pub(crate) inner: CoreDataType,
}

impl Clone for JsDataType {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl JsDataType {
    pub(crate) fn from_core(inner: CoreDataType) -> Self {
        Self { inner }
    }

    fn field_at(&self, index: usize) -> Option<JsField> {
        self.inner.get_field(index).cloned().map(JsField::from_core)
    }

    fn fields(&self) -> impl Iterator<Item = &CoreField> {
        (0..self.inner.field_len()).filter_map(|index| self.inner.get_field(index))
    }
}

#[napi]
impl JsDataType {
    /// Parse a type expression or cheaply clone another native `DataType`.
    #[napi(constructor)]
    pub fn new(value: Either<ClassInstance<'_, JsDataType>, String>) -> Result<Self> {
        data_type_from_input(value).map(Self::from_core)
    }

    /// Infer a type from a native wrapper or type-expression string.
    #[napi(factory, js_name = "from")]
    pub fn from_js(value: Either<ClassInstance<'_, JsDataType>, String>) -> Result<Self> {
        data_type_from_input(value).map(Self::from_core)
    }

    /// Internal direct constructor for parameter-free typed Field factories.
    #[napi(factory, js_name = "_simple", skip_typescript)]
    pub fn simple(kind: String) -> Result<Self> {
        let inner = match kind.as_str() {
            "null" => CoreDataType::Null,
            "boolean" => CoreDataType::Boolean,
            "int8" => CoreDataType::Int8,
            "int16" => CoreDataType::Int16,
            "int32" => CoreDataType::Int32,
            "int64" => CoreDataType::Int64,
            "uint8" => CoreDataType::UInt8,
            "uint16" => CoreDataType::UInt16,
            "uint32" => CoreDataType::UInt32,
            "uint64" => CoreDataType::UInt64,
            "float16" => CoreDataType::Float16,
            "float32" => CoreDataType::Float32,
            "float64" => CoreDataType::Float64,
            "date32" => CoreDataType::Date32,
            "date64" => CoreDataType::Date64,
            "binary" => CoreDataType::Binary,
            "large_binary" => CoreDataType::LargeBinary,
            "binary_view" => CoreDataType::BinaryView,
            "utf8" => CoreDataType::Utf8,
            "large_utf8" => CoreDataType::LargeUtf8,
            "utf8_view" => CoreDataType::Utf8View,
            _ => {
                return Err(Error::from_reason(format!(
                    "{kind:?} is not a parameter-free datatype kind"
                )));
            }
        };
        Ok(Self::from_core(inner))
    }

    /// Internal direct temporal constructor for typed Field factories.
    #[napi(factory, js_name = "_temporal", skip_typescript)]
    pub fn temporal(kind: String, unit: String, timezone: Option<String>) -> Result<Self> {
        let unit = CoreTimeUnit::from_str(&unit).map_err(napi_error)?;
        let inner = match kind.as_str() {
            "timestamp" if unit.is_temporal() => CoreDataType::Timestamp(
                unit,
                // The zone is canonicalized by the core, so an alias or a
                // differently cased spelling names the same datatype.
                timezone
                    .map(|value| yggdryl::Timezone::from_str(&value))
                    .transpose()
                    .map_err(napi_error)?,
            ),
            "time32" => CoreDataType::time32(unit).map_err(napi_error)?,
            "time64" => CoreDataType::time64(unit).map_err(napi_error)?,
            "duration" if unit.is_temporal() => CoreDataType::Duration(unit),
            "interval" if unit.is_interval() => CoreDataType::Interval(unit),
            "timestamp" | "duration" => {
                return Err(Error::from_reason(format!(
                    "{kind} requires a temporal resolution unit"
                )));
            }
            "interval" => {
                return Err(Error::from_reason(
                    "interval requires an interval layout unit",
                ));
            }
            _ => {
                return Err(Error::from_reason(format!(
                    "{kind:?} is not a temporal datatype kind"
                )));
            }
        };
        Ok(Self::from_core(inner))
    }

    /// Creates the physical time-of-day type selected by its resolution.
    #[napi(factory)]
    pub fn time(unit: String) -> Result<Self> {
        let unit = CoreTimeUnit::from_str(&unit).map_err(napi_error)?;
        CoreDataType::time(unit)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Internal direct fixed-size-binary constructor.
    #[napi(factory, js_name = "_fixedSizeBinary", skip_typescript)]
    pub fn fixed_size_binary(byte_width: f64) -> Result<Self> {
        let inner = CoreDataType::fixed_size_binary(exact_i32(byte_width, "byteWidth")?)
            .map_err(napi_error)?;
        Ok(Self::from_core(inner))
    }

    /// Internal direct exact-decimal constructor.
    #[napi(factory, js_name = "_decimal", skip_typescript)]
    pub fn decimal_kind(kind: String, precision: f64, scale: f64) -> Result<Self> {
        let precision = exact_u8(precision, "precision")?;
        let scale = exact_i8(scale, "scale")?;
        let value = match kind.as_str() {
            "decimal" => CoreDataType::decimal(precision, scale),
            "decimal32" => CoreDataType::decimal32(precision, scale),
            "decimal64" => CoreDataType::decimal64(precision, scale),
            "decimal128" => CoreDataType::decimal128(precision, scale),
            "decimal256" => CoreDataType::decimal256(precision, scale),
            _ => {
                return Err(Error::from_reason(format!(
                    "{kind:?} is not a decimal datatype kind"
                )));
            }
        };
        value.map(Self::from_core).map_err(napi_error)
    }

    /// Internal direct list-layout constructor preserving the child Field.
    #[napi(factory, js_name = "_list", skip_typescript)]
    pub fn list_kind(kind: String, item: &JsField, length: Option<f64>) -> Result<Self> {
        let item = item.inner.clone();
        let inner = match (kind.as_str(), length) {
            ("list", None) => CoreDataType::list(item),
            ("list_view", None) => CoreDataType::list_view(item),
            ("fixed_size_list", Some(length)) => {
                CoreDataType::fixed_size_list(item, exact_i32(length, "length")?)
                    .map_err(napi_error)?
            }
            ("large_list", None) => CoreDataType::large_list(item),
            ("large_list_view", None) => CoreDataType::large_list_view(item),
            _ => {
                return Err(Error::from_reason(format!(
                    "invalid list kind/length combination: {kind:?}"
                )));
            }
        };
        Ok(Self::from_core(inner))
    }

    /// Internal direct Struct constructor preserving exact child Fields.
    #[napi(factory, js_name = "_fromFields", skip_typescript)]
    pub fn from_fields(fields: Vec<ClassInstance<'_, JsField>>) -> Result<Self> {
        let inner = CoreDataType::from_fields(fields.into_iter().map(|field| field.inner.clone()))
            .map_err(napi_error)?;
        Ok(Self::from_core(inner))
    }

    /// Internal direct Union constructor preserving exact child Fields.
    #[napi(factory, js_name = "_union", skip_typescript)]
    pub fn union(
        type_ids: Vec<f64>,
        fields: Vec<ClassInstance<'_, JsField>>,
        mode: String,
    ) -> Result<Self> {
        if type_ids.len() != fields.len() {
            return Err(Error::from_reason(
                "union typeIds and fields must have the same length",
            ));
        }
        let mode = match mode.as_str() {
            "sparse" => CoreUnionMode::Sparse,
            "dense" => CoreUnionMode::Dense,
            _ => return Err(Error::from_reason("union mode must be 'sparse' or 'dense'")),
        };
        let mut members = Vec::with_capacity(fields.len());
        for (index, (type_id, field)) in type_ids.into_iter().zip(fields).enumerate() {
            members.push((
                exact_i8(type_id, &format!("typeIds[{index}]"))?,
                field.inner.clone(),
            ));
        }
        let inner = CoreDataType::union(members, mode).map_err(napi_error)?;
        Ok(Self::from_core(inner))
    }

    /// Internal variant constructor: bare, it is the self-describing Variant
    /// datatype; given members, the dense-union sugar assigning IDs in order.
    /// The loader's `DataType.variant` keeps the parenthesis disambiguation.
    #[napi(factory, js_name = "_variant", skip_typescript)]
    pub fn variant(fields: Option<Vec<ClassInstance<'_, JsField>>>) -> Result<Self> {
        let inner = match fields {
            None => CoreDataType::variant(),
            Some(fields) => {
                CoreDataType::dense_union(fields.into_iter().map(|field| field.inner.clone()))
                    .map_err(napi_error)?
            }
        };
        Ok(Self::from_core(inner))
    }

    /// Creates a geometry datatype: planar features as Well-Known Binary.
    /// Omitting `crs` fills the `OGC:CRS84` default shared with Parquet and
    /// Iceberg; a geometry takes no edge algorithm.
    #[napi(factory)]
    pub fn geometry(crs: Option<String>) -> Result<Self> {
        let inner = CoreDataType::geometry(crs.as_deref()).map_err(napi_error)?;
        Ok(Self::from_core(inner))
    }

    /// Creates a geography datatype: features on a sphere or spheroid.
    /// Omitting `crs` fills the `OGC:CRS84` default and omitting `algorithm`
    /// fills `spherical`; `algorithm` accepts the canonical lowercase names.
    #[napi(factory)]
    pub fn geography(crs: Option<String>, algorithm: Option<String>) -> Result<Self> {
        let algorithm = algorithm
            .as_deref()
            .map(CoreEdgeAlgorithm::from_str)
            .transpose()
            .map_err(napi_error)?;
        let inner = CoreDataType::geography(crs.as_deref(), algorithm).map_err(napi_error)?;
        Ok(Self::from_core(inner))
    }

    /// Internal direct Dictionary constructor.
    #[napi(factory, js_name = "_dictionary", skip_typescript)]
    pub fn dictionary(
        key: Either<ClassInstance<'_, JsDataType>, String>,
        value: Either<ClassInstance<'_, JsDataType>, String>,
    ) -> Result<Self> {
        let inner =
            CoreDataType::dictionary(data_type_from_input(key)?, data_type_from_input(value)?)
                .map_err(napi_error)?;
        Ok(Self::from_core(inner))
    }

    /// Internal direct Map constructor preserving the exact entries Field.
    #[napi(factory, js_name = "_map", skip_typescript)]
    pub fn map(entries: &JsField, keys_sorted: bool) -> Result<Self> {
        let inner = CoreDataType::map(entries.inner.clone(), keys_sorted).map_err(napi_error)?;
        Ok(Self::from_core(inner))
    }

    /// Internal direct logical Map constructor using the native Arrow layout.
    #[napi(factory, js_name = "_mapOf", skip_typescript)]
    pub fn map_of(
        key: Either<ClassInstance<'_, JsDataType>, String>,
        value: Either<ClassInstance<'_, JsDataType>, String>,
        keys_sorted: bool,
    ) -> Result<Self> {
        CoreDataType::map_of(
            data_type_from_input(key)?,
            data_type_from_input(value)?,
            keys_sorted,
        )
        .map(Self::from_core)
        .map_err(napi_error)
    }

    /// Internal direct run-end constructor preserving exact child Fields.
    #[napi(factory, js_name = "_runEndEncoded", skip_typescript)]
    pub fn run_end_encoded(run_ends: &JsField, values: &JsField) -> Result<Self> {
        let inner = CoreDataType::run_end_encoded(run_ends.inner.clone(), values.inner.clone())
            .map_err(napi_error)?;
        Ok(Self::from_core(inner))
    }

    /// Parse canonical, Arrow, SQL, Hive, or Spark type syntax.
    #[napi(factory)]
    pub fn from_string(value: String) -> Result<Self> {
        CoreDataType::from_str(&value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Parse the textual representation of an Arrow-compatible JS value.
    ///
    /// The loader coerces non-string inputs through their `toString` method;
    /// recursive grammar and validation remain in the Rust core.
    #[napi(factory, js_name = "fromArrowString", skip_typescript)]
    pub fn from_arrow(value: String) -> Result<Self> {
        CoreDataType::from_str(&value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Deserialize the native structural JSON representation.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: serde_json::Value) -> Result<Self> {
        serde_json::from_value(value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The parameter-free identity of this variant, such as `decimal128`.
    #[napi(getter)]
    pub fn id(&self) -> String {
        self.inner.id().as_str().to_owned()
    }

    /// The coarse datatype family shared by every variant of one kind.
    #[napi(getter)]
    pub fn kind(&self) -> String {
        self.inner.kind().as_str().to_owned()
    }

    /// Whether this type owns child fields.
    #[napi(getter)]
    pub fn nested(&self) -> bool {
        self.inner.is_nested()
    }

    /// Number of direct child fields.
    #[napi(getter)]
    pub fn length(&self) -> u32 {
        u32::try_from(self.inner.field_len()).unwrap_or(u32::MAX)
    }

    /// Return the child at an Array-compatible positive or negative index.
    #[napi]
    pub fn at(&self, index: i32) -> Option<JsField> {
        let len = i64::from(self.length());
        let index = i64::from(index);
        let resolved = if index < 0 { len + index } else { index };
        usize::try_from(resolved)
            .ok()
            .and_then(|index| self.field_at(index))
    }

    /// Look up a child by zero-based index or exact field name.
    #[napi]
    pub fn get(&self, key: Either<u32, String>) -> Option<JsField> {
        match key {
            Either::A(index) => usize::try_from(index)
                .ok()
                .and_then(|index| self.field_at(index)),
            Either::B(name) => self
                .inner
                .get_field_by_name(&name)
                .cloned()
                .map(JsField::from_core),
        }
    }

    /// Look up a child by exact field name without materializing children.
    #[napi]
    pub fn get_by_name(&self, name: String) -> Option<JsField> {
        self.inner
            .get_field_by_name(&name)
            .cloned()
            .map(JsField::from_core)
    }

    /// Test for a child index, field name, or exact Field value.
    #[napi]
    pub fn contains(&self, value: Either3<u32, String, ClassInstance<'_, JsField>>) -> bool {
        match value {
            Either3::A(index) => usize::try_from(index)
                .ok()
                .and_then(|index| self.inner.get_field(index))
                .is_some(),
            Either3::B(name) => self.inner.get_field_by_name(&name).is_some(),
            Either3::C(field) => self.fields().any(|candidate| candidate == &field.inner),
        }
    }

    /// Child names in physical order.
    #[napi]
    pub fn keys(&self) -> Vec<String> {
        self.fields().map(|field| field.name().to_owned()).collect()
    }

    /// Child fields in physical order.
    #[napi]
    pub fn values(&self) -> Vec<JsField> {
        self.fields().cloned().map(JsField::from_core).collect()
    }

    /// Recursive native equality, optionally ignoring nested Field metadata.
    #[napi]
    pub fn equals(&self, other: &JsDataType, with_metadata: Option<bool>) -> bool {
        self.inner
            .equals(&other.inner, with_metadata.unwrap_or(true))
    }

    /// Return an iterator over stable recursive difference lines.
    #[napi(js_name = "_showDiffs", skip_typescript)]
    pub fn show_diffs_native(
        &self,
        other: &JsDataType,
        with_metadata: Option<bool>,
        return_equal: Option<bool>,
    ) -> JsDifferenceIterator {
        JsDifferenceIterator::from_data_types(
            &self.inner,
            &other.inner,
            with_metadata.unwrap_or(true),
            return_equal.unwrap_or(false),
        )
    }

    /// Join recursive differences, or return `✓ equal`.
    #[napi]
    pub fn show_diff(
        &self,
        other: &JsDataType,
        with_metadata: Option<bool>,
        return_equal: Option<bool>,
    ) -> String {
        self.inner.show_diff(
            &other.inner,
            with_metadata.unwrap_or(true),
            return_equal.unwrap_or(true),
        )
    }

    /// Total native ordering: `-1`, `0`, or `1`.
    #[napi]
    pub fn compare(&self, other: &JsDataType) -> i32 {
        ordering_value(self.inner.cmp(&other.inner))
    }

    /// Deterministic FNV-1a hash of canonical native display text.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Materialize the bounded canonical default through Record's exact
    /// schema-guided JavaScript scalar projection.
    #[napi(js_name = "_defaultJSValueNative", skip_typescript)]
    pub fn default_js_value_native<'env>(
        &self,
        env: &'env Env,
        schema: &JsField,
    ) -> Result<Unknown<'env>> {
        let value = self.inner.default_value().map_err(napi_error)?;
        let field = schema
            .inner
            .get_field(0)
            .ok_or_else(|| napi_error("default DataType projection schema has no value field"))?;
        field_value_to_js(env, field, &value, &schema.inner)
    }

    /// Internal shared one-column schema for repeated JavaScript default
    /// projection.
    #[napi(js_name = "_defaultJSValueSchemaNative", skip_typescript)]
    pub fn default_js_value_schema_native(&self) -> Result<JsField> {
        let field = CoreField::new("value", self.inner.clone(), false);
        field_value_schema(&field).map(JsField::from_core)
    }

    /// Internal allocation-independent JavaScript constructor category.
    #[napi(js_name = "_defaultJSHintNative", skip_typescript)]
    pub fn default_js_hint_native(&self) -> Result<u8> {
        data_type_js_hint(&self.inner).map(JsValueHint::code)
    }

    /// Internal one-row copied IPC projection for Apache Arrow JS scalar
    /// materialization.
    #[napi(js_name = "_defaultArrowScalarIpcNative", skip_typescript)]
    pub fn default_arrow_scalar_ipc_native(&self) -> Result<napi::bindgen_prelude::Buffer> {
        let array = self.inner.default_arrow_array().map_err(napi_error)?;
        let field = CoreField::new("value", self.inner.clone(), false);
        arrow_scalar_to_ipc(&field, array)
    }

    /// Recursively normalize this datatype for one closed compatibility
    /// target without changing the current wrapper.
    #[napi(js_name = "toSchemeCompat", skip_typescript)]
    pub fn to_scheme_compat(&self, target: String) -> Result<Self> {
        let target = CoreScheme::from_str(&target).map_err(napi_error)?;
        self.inner
            .to_scheme_compat(&target)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Make an allocation-free or shared-state native clone.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// Return canonical syntax accepted losslessly by `fromString`.
    #[napi]
    pub fn to_string(&self) -> String {
        self.inner.to_string()
    }

    /// Serialize to version-independent structural JSON.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        serde_json::to_value(&self.inner).map_err(napi_error)
    }
}
