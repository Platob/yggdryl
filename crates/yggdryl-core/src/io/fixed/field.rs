//! [`Field`] (a named, nullable column descriptor), [`TypedField`] (its typed form), and the
//! [`FixedField`] sub-trait of the root [`FieldType`](crate::io::FieldType).

use core::marker::PhantomData;

use super::{NativeType, PrimitiveType};
use crate::io::{DataType, DataTypeId, FieldType, Headers};

/// The **fixed-width field** sub-trait — a [`FieldType`] over a [`NativeType`], with the
/// typed descriptor mutualized as a default method.
pub trait FixedField: FieldType {
    /// The native element type.
    type Native: NativeType;

    /// The typed data type of the field — mutualized default.
    fn data_type(&self) -> PrimitiveType<Self::Native> {
        PrimitiveType::new()
    }
}

/// A **named, nullable** column descriptor: a name, its [`DataType`]'s essentials (the type
/// name + byte width), and whether it admits nulls. The erased counterpart of
/// [`TypedField`], for a schema that holds columns of differing types side by side.
///
/// DESIGN: it stores the data type's name and width *denormalized* rather than a
/// `Box<dyn DataType>`, so a `Field` is a plain, clonable, hashable value (no trait-object
/// allocation, and it works as a map key) — enough to describe a fixed-width column. It also
/// carries [`Headers`] (the centralized string key/value metadata holder, like Arrow's `Field`
/// metadata) — used, among other things, to pin the exact logical type across a lossy Arrow
/// round-trip.
///
/// ```
/// use yggdryl_core::io::fixed::{Field, PrimitiveType};
///
/// let f = Field::new("id", &<PrimitiveType<i32>>::new(), false)
///     .with_metadata_entry("unit", "count");
/// assert_eq!(f.name(), "id");
/// assert_eq!(f.type_name(), "i32");
/// assert_eq!(f.byte_width(), 4);
/// assert!(!f.nullable());
/// assert_eq!(f.metadata().get("unit"), Some("count"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Field {
    name: String,
    type_name: &'static str,
    byte_width: usize,
    nullable: bool,
    type_id: DataTypeId,
    metadata: Headers,
}

impl Field {
    /// Builds a field from a name, any [`DataType`], and its nullability — capturing the type's
    /// [`type_id`](DataType::type_id) so the erased field can still drill down (`is_integer` …)
    /// without the original descriptor. The [`metadata`](Field::metadata) starts empty.
    pub fn new(name: &str, data_type: &dyn DataType, nullable: bool) -> Self {
        Self {
            name: name.to_string(),
            type_name: data_type.name(),
            byte_width: data_type.byte_width(),
            nullable,
            type_id: data_type.type_id(),
            metadata: Headers::new(),
        }
    }

    /// Builds a field from its parts — a name, a [`DataTypeId`], its byte width, and its
    /// nullability (empty [`metadata`](Field::metadata)). The from-parts constructor for a
    /// runtime-typed field (e.g. the language bindings), where there is no generic descriptor to
    /// hand to [`new`](Field::new).
    pub fn of(name: &str, type_id: DataTypeId, byte_width: usize, nullable: bool) -> Self {
        Self {
            name: name.to_string(),
            type_name: type_id.name(),
            byte_width,
            nullable,
            type_id,
            metadata: Headers::new(),
        }
    }

    /// The field's metadata [`Headers`] (a string key/value map).
    pub fn metadata(&self) -> &Headers {
        &self.metadata
    }

    /// A fresh field with the given metadata [`Headers`] attached (replacing any existing metadata).
    pub fn with_metadata(mut self, metadata: Headers) -> Self {
        self.metadata = metadata;
        self
    }

    /// A fresh field with one extra `key = value` metadata entry — the one-line builder.
    pub fn with_metadata_entry(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key, value);
        self
    }

    /// The column name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The data type's name (e.g. `"i64"`).
    pub fn type_name(&self) -> &'static str {
        self.type_name
    }

    /// The data type's fixed byte width.
    pub fn byte_width(&self) -> usize {
        self.byte_width
    }

    /// Whether the column admits nulls.
    pub fn nullable(&self) -> bool {
        self.nullable
    }

    /// Whether this field's element type is `T` — the optimized typed check (a `&'static str`
    /// pointer/length compare against `T`'s name, no allocation).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::{Field, PrimitiveType};
    ///
    /// let f = Field::new("id", &<PrimitiveType<i32>>::new(), false);
    /// assert!(f.is::<i32>());
    /// assert!(!f.is::<u8>());
    /// ```
    pub fn is<T: NativeType>(&self) -> bool {
        self.type_name == T::NAME
    }
}

/// A **typed** column descriptor — a name + nullability with the element type `T` fixed at
/// compile time (`U8Field = TypedField<u8>`). [`erase`](TypedField::erase) drops to a runtime
/// [`Field`].
///
/// ```
/// use yggdryl_core::io::DataType;
/// use yggdryl_core::io::fixed::TypedField;
///
/// let f = <TypedField<i32>>::new("price", true);
/// assert_eq!(f.name(), "price");
/// assert!(f.nullable());
/// assert_eq!(f.data_type().byte_width(), 4);
/// assert_eq!(f.erase().type_name(), "i32");
/// ```
pub struct TypedField<T: NativeType> {
    name: String,
    nullable: bool,
    metadata: Headers,
    _type: PhantomData<T>,
}

impl<T: NativeType> TypedField<T> {
    /// Builds a typed field from a name and its nullability, with empty metadata.
    pub fn new(name: &str, nullable: bool) -> Self {
        Self {
            name: name.to_string(),
            nullable,
            metadata: Headers::new(),
            _type: PhantomData,
        }
    }

    /// The column name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the column admits nulls.
    pub fn nullable(&self) -> bool {
        self.nullable
    }

    /// The field's metadata [`Headers`].
    pub fn metadata(&self) -> &Headers {
        &self.metadata
    }

    /// A fresh field with the given metadata [`Headers`] attached.
    pub fn with_metadata(mut self, metadata: Headers) -> Self {
        self.metadata = metadata;
        self
    }

    /// A fresh field with one extra `key = value` metadata entry.
    pub fn with_metadata_entry(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key, value);
        self
    }

    /// The typed data type descriptor.
    pub fn data_type(&self) -> PrimitiveType<T> {
        PrimitiveType::new()
    }

    /// The erased runtime [`Field`], metadata preserved.
    pub fn erase(&self) -> Field {
        Field::new(&self.name, &self.data_type(), self.nullable)
            .with_metadata(self.metadata.clone())
    }
}

// The trait-hierarchy impls: `Field` is the erased implementation, `TypedField<T>` the fixed
// typed one. Bodies read fields/inherent directly (no recursion).
impl FieldType for Field {
    fn name(&self) -> &str {
        &self.name
    }

    fn type_name(&self) -> &'static str {
        self.type_name
    }

    fn byte_width(&self) -> usize {
        self.byte_width
    }

    fn nullable(&self) -> bool {
        self.nullable
    }

    fn type_id(&self) -> DataTypeId {
        self.type_id
    }
}

impl<T: NativeType> FieldType for TypedField<T> {
    fn name(&self) -> &str {
        &self.name
    }

    fn type_name(&self) -> &'static str {
        T::NAME
    }

    fn byte_width(&self) -> usize {
        T::WIDTH
    }

    fn nullable(&self) -> bool {
        self.nullable
    }

    fn type_id(&self) -> DataTypeId {
        T::TYPE_ID
    }
}

impl<T: NativeType> FixedField for TypedField<T> {
    type Native = T;
}

/// Arrow schema interop (feature `arrow`) — [`Field`] / [`TypedField`] ↔ [`arrow_schema::Field`].
#[cfg(feature = "arrow")]
impl Field {
    /// This field as an [`arrow_schema::Field`] — **total** (every type maps to some Arrow type).
    ///
    /// The field's [`metadata`](Field::metadata) is carried over, and when the Arrow data type is
    /// a *lossy* representation of the logical type (a `u96`/`i96`/`FixedUtf8`/… all collapsing to
    /// `FixedSizeBinary`, or an integer tagged as a `Decimal`), the exact type is recorded under
    /// [`DataTypeId::METADATA_KEY`] so [`from_arrow`](Field::from_arrow) can recover it. Exact
    /// mappings (`i32` → `Int32`, …) add no metadata.
    pub fn to_arrow(&self) -> arrow_schema::Field {
        // A decimal's Arrow type needs a `(precision, scale)` that the type id + byte width cannot
        // supply, so read them from the reserved metadata keys (present when erased from a
        // `DecimalField`), defaulting to the width's max precision and scale 0.
        let data_type = if self.type_id.is_decimal() {
            let precision = self
                .metadata
                .get(DataTypeId::PRECISION_METADATA_KEY)
                .and_then(|value| value.parse::<u8>().ok())
                .unwrap_or_else(|| self.type_id.decimal_max_precision().unwrap_or(38));
            let scale = self
                .metadata
                .get(DataTypeId::SCALE_METADATA_KEY)
                .and_then(|value| value.parse::<i8>().ok())
                .unwrap_or(0);
            self.type_id
                .to_arrow_decimal(precision, scale)
                .expect("a decimal id always maps to an Arrow Decimal")
        } else {
            self.type_id.to_arrow(self.byte_width)
        };
        let mut metadata = self.metadata.to_arrow_metadata();
        // The Arrow Decimal type already carries precision/scale — drop the reserved shadow keys.
        metadata.remove(DataTypeId::PRECISION_METADATA_KEY);
        metadata.remove(DataTypeId::SCALE_METADATA_KEY);
        // Tag the exact logical type only when the plain Arrow mapping can't be reversed to it.
        if DataTypeId::from_arrow(&data_type).map(|(id, _)| id) != Some(self.type_id) {
            metadata.insert(
                DataTypeId::METADATA_KEY.to_string(),
                self.type_id.name().to_string(),
            );
        }
        arrow_schema::Field::new(&self.name, data_type, self.nullable).with_metadata(metadata)
    }

    /// Builds an erased field from an [`arrow_schema::Field`], or `None` for a data type this
    /// crate does not model. The **exact** logical type is recovered from the
    /// [`DataTypeId::METADATA_KEY`] metadata when present (so a `FixedSizeBinary(12)` tagged
    /// `"u96"` reads back as `u96`, not the default `fixed_binary`); the reserved key is then
    /// stripped from the field's user-visible [`metadata`](Field::metadata).
    pub fn from_arrow(field: &arrow_schema::Field) -> Option<Self> {
        let (base_id, byte_width) = DataTypeId::from_arrow(field.data_type())?;
        let mut metadata = Headers::from_arrow_metadata(field.metadata());
        // `Headers::remove` returns a count, so read the reserved tag before stripping it.
        let type_id = metadata
            .get(DataTypeId::METADATA_KEY)
            .and_then(DataTypeId::from_name)
            .unwrap_or(base_id);
        metadata.remove(DataTypeId::METADATA_KEY);
        // For a decimal, capture the Arrow Decimal's precision/scale into the reserved keys so the
        // erased field keeps them (its byte width alone cannot express them).
        if type_id.is_decimal() {
            if let Some((precision, scale)) = DataTypeId::arrow_decimal_params(field.data_type()) {
                metadata.insert(DataTypeId::PRECISION_METADATA_KEY, &precision.to_string());
                metadata.insert(DataTypeId::SCALE_METADATA_KEY, &scale.to_string());
            }
        }
        Some(Self {
            name: field.name().clone(),
            type_name: type_id.name(),
            byte_width,
            nullable: field.is_nullable(),
            type_id,
            metadata,
        })
    }
}

#[cfg(feature = "arrow")]
impl<T: NativeType> TypedField<T> {
    /// This field as an [`arrow_schema::Field`] (via the erased [`Field`], metadata + exact-type
    /// tag included).
    pub fn to_arrow(&self) -> arrow_schema::Field {
        self.erase().to_arrow()
    }

    /// Builds a typed field from an [`arrow_schema::Field`], or `None` if its (metadata-refined)
    /// logical type is not `T`. Any user metadata is preserved.
    pub fn from_arrow(field: &arrow_schema::Field) -> Option<Self> {
        let erased = Field::from_arrow(field)?;
        (FieldType::type_id(&erased) == T::TYPE_ID).then(|| {
            Self::new(erased.name(), erased.nullable()).with_metadata(erased.metadata().clone())
        })
    }
}

impl<T: NativeType> Clone for TypedField<T> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            nullable: self.nullable,
            metadata: self.metadata.clone(),
            _type: PhantomData,
        }
    }
}

impl<T: NativeType> PartialEq for TypedField<T> {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.nullable == other.nullable
            && self.metadata == other.metadata
    }
}

impl<T: NativeType> Eq for TypedField<T> {}

impl<T: NativeType> core::fmt::Debug for TypedField<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TypedField")
            .field("name", &self.name)
            .field("type", &T::NAME)
            .field("nullable", &self.nullable)
            .finish()
    }
}
