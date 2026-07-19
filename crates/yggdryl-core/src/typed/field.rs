//! [`Field`] — a **column's metadata**: its name, element [`DataTypeId`], and nullability, carried
//! in a [`Headers`] map.
//!
//! A `Field` is what a schema holds and a [`Serie`](super::Serie) reports about itself. The concrete
//! [`HeaderField`] *is* a `Headers` (the project's one metadata map): the name lives in
//! [`Headers::NAME`], the type in [`Headers::TYPE_ID`], the nullable flag in [`Headers::NULLABLE`] —
//! so a field serializes, hashes, and travels exactly like any other metadata, and arbitrary extra
//! annotations ride alongside for free.

use crate::datatype_id::DataTypeId;
use crate::headers::Headers;
use crate::io::memory::IoError;

/// The **promoted / structural** header keys a [`HeaderField`] represents through its typed
/// accessors (name / type / nullable + the decimal precision·scale + the fixed-size byte width) —
/// the ones handled structurally by a `cast_field`, and therefore **excluded** from the free-form
/// annotations it copies (see [`HeaderField::extra_annotations`]).
const STRUCTURAL_KEYS: [&str; 6] = [
    Headers::NAME,
    Headers::TYPE_ID,
    Headers::NULLABLE,
    Headers::PRECISION,
    Headers::SCALE,
    Headers::BYTE_WIDTH,
];

/// A typed column descriptor: `name`, element type, and nullability — plus the open [`Headers`] map
/// the metadata lives in.
pub trait Field {
    /// The element [`DataTypeId`].
    fn data_type_id(&self) -> DataTypeId;

    /// Whether the column admits nulls.
    fn nullable(&self) -> bool;

    /// The backing metadata map (the field's name/type/nullable live here, alongside any extras).
    fn headers(&self) -> &Headers;

    /// The column **name** — total: the stored [`X-Name`](Headers::NAME) when set, else the element
    /// type's name as the default (an unnamed `i64` field names itself `"i64"`). The default is
    /// **not** written back into the stored bytes, so an unnamed field still round-trips as unnamed —
    /// read [`headers().name()`](Headers::name) directly for the raw stored name.
    fn name(&self) -> &str {
        self.headers()
            .name()
            .unwrap_or_else(|| self.data_type_id().name())
    }
}

/// A [`Field`] backed by a [`Headers`] map — the metadata `name` / `type_id` / `nullable` are three
/// entries in the map, so the field is a plain, serializable, hashable value.
///
/// ```
/// use yggdryl_core::typed::{Field, HeaderField};
/// use yggdryl_core::datatype_id::DataTypeId;
///
/// let field = HeaderField::new(Some("price"), DataTypeId::I64, true);
/// assert_eq!(field.name(), "price");
/// assert_eq!(field.data_type_id(), DataTypeId::I64);
/// assert!(field.nullable());
///
/// // An unnamed field derives its name from the element type; the raw X-Name stays unset.
/// let unnamed = HeaderField::new(None, DataTypeId::I64, false);
/// assert_eq!(unnamed.name(), "i64");
/// assert_eq!(unnamed.headers().name(), None);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct HeaderField {
    headers: Headers,
}

impl HeaderField {
    /// A field from its `name`, element `type_id`, and `nullable` flag.
    pub fn new(name: Option<&str>, type_id: DataTypeId, nullable: bool) -> Self {
        let mut headers = Headers::new();
        if let Some(name) = name {
            headers.set_name(name);
        }
        headers.set_type_id(type_id);
        headers.set_nullable(nullable);
        HeaderField { headers }
    }

    /// A **decimal** field — sets the `precision` (max significant digits) and `scale` (decimal
    /// places) alongside the name / type / nullable.
    pub fn decimal(
        name: Option<&str>,
        type_id: DataTypeId,
        precision: u32,
        scale: i32,
        nullable: bool,
    ) -> Self {
        let mut field = Self::new(name, type_id, nullable);
        field.headers.set_precision(precision);
        field.headers.set_scale(scale);
        field
    }

    /// A **fixed-size** field — sets the fixed element `byte_width` (the parameterized length)
    /// alongside the name / type / nullable.
    pub fn fixed_size(
        name: Option<&str>,
        type_id: DataTypeId,
        byte_width: u32,
        nullable: bool,
    ) -> Self {
        let mut field = Self::new(name, type_id, nullable);
        field.headers.set_byte_width(byte_width);
        field
    }

    /// A field wrapping an existing [`Headers`] map (its `type_id`/`name`/`nullable` are read from it).
    pub fn from_headers(headers: Headers) -> Self {
        HeaderField { headers }
    }

    /// The fixed element **byte width** (the parameterized length), if this field carries it.
    pub fn byte_width(&self) -> Option<u32> {
        self.headers.byte_width()
    }

    /// The decimal **precision** (max significant digits), if this field carries it.
    pub fn precision(&self) -> Option<u32> {
        self.headers.precision()
    }

    /// The decimal **scale** (decimal places), if this field carries it.
    pub fn scale(&self) -> Option<i32> {
        self.headers.scale()
    }

    /// The mutable metadata map — annotate the field with any extra headers.
    pub fn headers_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }

    /// Consumes the field into its [`Headers`].
    pub fn into_headers(self) -> Headers {
        self.headers
    }

    // ---- metadata (the whole backing map + single-key annotations) ----------------------

    /// The whole backing metadata map (borrowed) — the field's `name` / `type_id` / `nullable`
    /// entries plus any extra annotations. The read counterpart of
    /// [`metadata_mut`](HeaderField::metadata_mut) (and a synonym of the [`Field`] trait's
    /// [`headers`](Field::headers)).
    pub fn metadata(&self) -> &Headers {
        &self.headers
    }

    /// The whole backing metadata map (mutable) — set any header on the field. Aliases
    /// [`headers_mut`](HeaderField::headers_mut).
    pub fn metadata_mut(&mut self) -> &mut Headers {
        &mut self.headers
    }

    /// Sets one arbitrary annotation `key` to `value` (replace semantics) — delegates to
    /// [`Headers::insert`](Headers::insert).
    ///
    /// ```
    /// use yggdryl_core::typed::HeaderField;
    /// use yggdryl_core::datatype_id::DataTypeId;
    ///
    /// let mut field = HeaderField::new(Some("price"), DataTypeId::I64, true);
    /// field.set_metadata("unit", "USD");
    /// assert_eq!(field.metadata_value("unit").as_deref(), Some("USD"));
    /// ```
    pub fn set_metadata(&mut self, key: &str, value: &str) {
        self.headers.insert(key, value);
    }

    /// The value of one arbitrary annotation `key`, if present and valid UTF-8 — delegates to
    /// [`Headers::get`](Headers::get).
    pub fn metadata_value(&self, key: &str) -> Option<String> {
        self.headers.get(key).map(str::to_owned)
    }

    /// [`set_metadata`](HeaderField::set_metadata), chainable.
    pub fn with_metadata(mut self, key: &str, value: &str) -> Self {
        self.set_metadata(key, value);
        self
    }

    // ---- the ergonomic set_* / with_* trio over the promoted typed fields ---------------

    /// Sets the field **name** (the promoted [`Headers::NAME`](Headers::NAME) entry).
    pub fn set_name(&mut self, name: &str) {
        self.headers.set_name(name);
    }

    /// [`set_name`](HeaderField::set_name), chainable.
    pub fn with_name(mut self, name: &str) -> Self {
        self.set_name(name);
        self
    }

    /// Sets whether the field admits nulls (the promoted [`Headers::NULLABLE`](Headers::NULLABLE) entry).
    pub fn set_nullable(&mut self, nullable: bool) {
        self.headers.set_nullable(nullable);
    }

    /// [`set_nullable`](HeaderField::set_nullable), chainable.
    pub fn with_nullable(mut self, nullable: bool) -> Self {
        self.set_nullable(nullable);
        self
    }

    /// Sets the element [`DataTypeId`] (the promoted [`Headers::TYPE_ID`](Headers::TYPE_ID) entry).
    pub fn set_data_type_id(&mut self, type_id: DataTypeId) {
        self.headers.set_type_id(type_id);
    }

    /// [`set_data_type_id`](HeaderField::set_data_type_id), chainable.
    pub fn with_data_type_id(mut self, type_id: DataTypeId) -> Self {
        self.set_data_type_id(type_id);
        self
    }

    /// The **non-structural** annotations — every entry but the promoted `name` / `type_id` /
    /// `nullable` / `precision` / `scale` / `byte_width` keys — collected into a fresh [`Headers`]
    /// in insertion order. The free-form metadata a `cast_field` copies onto a column.
    pub(crate) fn extra_annotations(&self) -> Headers {
        let mut extra = Headers::new();
        for (name, value) in self.headers.iter() {
            let structural = STRUCTURAL_KEYS
                .iter()
                .any(|key| name.eq_ignore_ascii_case(key.as_bytes()));
            if !structural {
                extra.append_bytes(name, value);
            }
        }
        extra
    }
}

/// The guided [`IoError::TypedCast`] for a **nullable → non-nullable** cast that still holds
/// `nulls` real nulls — names the offending count and the fix. Shared by the column and scalar
/// casts so their messages read identically.
pub(crate) fn cast_null_error(nulls: usize) -> IoError {
    IoError::TypedCast {
        detail: format!(
            "cannot cast a column with {nulls} nulls to a non-nullable field: fill or drop the \
             nulls first"
        ),
    }
}

/// The guided [`IoError::TypedCast`] for a `cast_field` whose target names a **different element
/// type** than the compile-time-typed `container` (`"FixedSerie"` / `"FixedScalar"`) carries —
/// the typed layer keeps its element type, so a dtype change belongs to the erased layer.
pub(crate) fn cast_dtype_error(
    container: &str,
    current: DataTypeId,
    target: DataTypeId,
) -> IoError {
    IoError::TypedCast {
        detail: format!(
            "cast_field on a typed {container}<{current}> keeps its element type: change dtype \
             through the erased Serie.cast_field (bindings) or resize the buffer with \
             IOBase::resize_dtype — target {target} != column {current}"
        ),
    }
}

impl Field for HeaderField {
    fn data_type_id(&self) -> DataTypeId {
        self.headers.type_id()
    }

    fn nullable(&self) -> bool {
        self.headers.nullable()
    }

    fn headers(&self) -> &Headers {
        &self.headers
    }
}
