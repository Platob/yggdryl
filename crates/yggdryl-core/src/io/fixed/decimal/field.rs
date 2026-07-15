//! [`DecimalField<B>`] — a named, nullable decimal column descriptor (a name, a `(precision, scale)`,
//! nullability, and [`Headers`] metadata), with its own exact Arrow `Decimal(precision, scale)`
//! round-trip.

use core::marker::PhantomData;

use super::{DecimalBacking, DecimalType};
use crate::io::fixed::Field;
use crate::io::{DataTypeId, FieldType, Headers};

/// A named, nullable decimal column of width `B`, precision `precision`, and scale `scale`.
///
/// ```
/// use yggdryl_core::io::FieldType;
/// use yggdryl_core::io::fixed::D128Field;
///
/// let f = D128Field::new("amount", 20, 4, true);
/// assert_eq!(f.name(), "amount");
/// assert_eq!(f.type_name(), "d128");
/// assert!(f.is_decimal() && f.nullable());
/// ```
pub struct DecimalField<B: DecimalBacking> {
    name: String,
    precision: u8,
    scale: i8,
    nullable: bool,
    metadata: Headers,
    _backing: PhantomData<B>,
}

impl<B: DecimalBacking> DecimalField<B> {
    /// A field for `Decimal(precision, scale)` with the given name and nullability (empty
    /// metadata). `precision`/`scale` are clamped to the valid range (see
    /// [`DecimalType::new`](DecimalType::new)).
    pub fn new(name: &str, precision: u8, scale: i8, nullable: bool) -> Self {
        let dt = DecimalType::<B>::new(precision, scale);
        Self {
            name: name.to_string(),
            precision: dt.precision(),
            scale: dt.scale(),
            nullable,
            metadata: Headers::new(),
            _backing: PhantomData,
        }
    }

    /// The column name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The precision.
    pub fn precision(&self) -> u8 {
        self.precision
    }

    /// The scale.
    pub fn scale(&self) -> i8 {
        self.scale
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

    /// The typed descriptor.
    pub fn data_type(&self) -> DecimalType<B> {
        DecimalType::new(self.precision, self.scale)
    }

    /// The erased runtime [`Field`], with the precision/scale stashed in metadata (under the
    /// reserved [`PRECISION_METADATA_KEY`](DataTypeId::PRECISION_METADATA_KEY) /
    /// [`SCALE_METADATA_KEY`](DataTypeId::SCALE_METADATA_KEY)) so the erased field's Arrow mapping
    /// keeps the exact `Decimal(precision, scale)` that its byte width alone cannot express.
    pub fn erase(&self) -> Field {
        let mut metadata = self.metadata.clone();
        metadata.insert(
            DataTypeId::PRECISION_METADATA_KEY,
            &self.precision.to_string(),
        );
        metadata.insert(DataTypeId::SCALE_METADATA_KEY, &self.scale.to_string());
        Field::new(&self.name, &self.data_type(), self.nullable).with_metadata(metadata)
    }

    /// This field as an [`arrow_schema::Field`] — an exact `Decimal(precision, scale)` (feature
    /// `arrow`). For the widths whose plain Arrow mapping is ambiguous (`Decimal128`/`Decimal256`
    /// default to the wide *integers*), the exact `d128`/`d256` logical type is tagged under
    /// [`DataTypeId::METADATA_KEY`] so [`from_arrow`](DecimalField::from_arrow) recovers it.
    #[cfg(feature = "arrow")]
    pub fn to_arrow(&self) -> arrow_schema::Field {
        let data_type = self.data_type().to_arrow();
        let mut metadata = self.metadata.to_arrow_metadata();
        if DataTypeId::from_arrow(&data_type).map(|(id, _)| id) != Some(B::TYPE_ID) {
            metadata.insert(DataTypeId::METADATA_KEY.to_string(), B::NAME.to_string());
        }
        arrow_schema::Field::new(&self.name, data_type, self.nullable).with_metadata(metadata)
    }

    /// Builds a decimal field from an [`arrow_schema::Field`], or `None` if it is not this width's
    /// `Decimal` (or is explicitly tagged as a different logical type — e.g. an `i128` integer
    /// stored as `Decimal128`). User metadata is preserved (feature `arrow`).
    #[cfg(feature = "arrow")]
    pub fn from_arrow(field: &arrow_schema::Field) -> Option<Self> {
        let dt = DecimalType::<B>::from_arrow(field.data_type())?;
        let mut metadata = Headers::from_arrow_metadata(field.metadata());
        // Reject a value explicitly tagged as a *different* logical type.
        if let Some(tag) = metadata.get(DataTypeId::METADATA_KEY) {
            if DataTypeId::from_name(tag) != Some(B::TYPE_ID) {
                return None;
            }
        }
        metadata.remove(DataTypeId::METADATA_KEY);
        Some(
            Self::new(
                field.name(),
                dt.precision(),
                dt.scale(),
                field.is_nullable(),
            )
            .with_metadata(metadata),
        )
    }
}

impl<B: DecimalBacking> FieldType for DecimalField<B> {
    fn name(&self) -> &str {
        &self.name
    }

    fn type_name(&self) -> &'static str {
        B::NAME
    }

    fn byte_width(&self) -> usize {
        B::WIDTH
    }

    fn nullable(&self) -> bool {
        self.nullable
    }

    fn type_id(&self) -> DataTypeId {
        B::TYPE_ID
    }
}

impl<B: DecimalBacking> Clone for DecimalField<B> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            precision: self.precision,
            scale: self.scale,
            nullable: self.nullable,
            metadata: self.metadata.clone(),
            _backing: PhantomData,
        }
    }
}
impl<B: DecimalBacking> PartialEq for DecimalField<B> {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.precision == other.precision
            && self.scale == other.scale
            && self.nullable == other.nullable
            && self.metadata == other.metadata
    }
}
impl<B: DecimalBacking> Eq for DecimalField<B> {}
impl<B: DecimalBacking> core::fmt::Debug for DecimalField<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DecimalField")
            .field("name", &self.name)
            .field("type", &B::NAME)
            .field("precision", &self.precision)
            .field("scale", &self.scale)
            .field("nullable", &self.nullable)
            .finish()
    }
}
