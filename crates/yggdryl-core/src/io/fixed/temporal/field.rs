//! [`TemporalField<B>`] — a named, nullable temporal column descriptor (a name, a `(unit, tz)`,
//! nullability, and [`Headers`] metadata), with its own exact Arrow round-trip.

use core::marker::PhantomData;

use super::{TemporalBacking, TemporalType, TimeUnit, Tz};
use crate::io::field_carrier::field_setters;
use crate::io::fixed::Field;
use crate::io::{DataTypeId, FieldType, Headers};

/// A named, nullable temporal column of concept+width `B`, resolution `unit`, and timezone `tz`.
///
/// ```
/// use yggdryl_core::io::FieldType;
/// use yggdryl_core::io::fixed::Ts64Field;
/// use yggdryl_core::io::fixed::temporal::{TimeUnit, Tz};
///
/// let f = Ts64Field::new("event_at", TimeUnit::Microsecond, Tz::UTC, true);
/// assert_eq!(f.name(), "event_at");
/// assert_eq!(f.type_name(), "ts64");
/// assert!(f.is_temporal() && f.nullable());
/// ```
pub struct TemporalField<B: TemporalBacking> {
    name: String,
    unit: TimeUnit,
    tz: Tz,
    nullable: bool,
    metadata: Headers,
    _backing: PhantomData<B>,
}

impl<B: TemporalBacking> TemporalField<B> {
    /// A field for this column at `(unit, tz)` with the given name and nullability (empty
    /// metadata). `unit` / `tz` are clamped to what `B` admits (see
    /// [`TemporalType::new`](TemporalType::new)).
    pub fn new(name: &str, unit: TimeUnit, tz: Tz, nullable: bool) -> Self {
        let dt = TemporalType::<B>::new(unit, tz);
        Self {
            name: name.to_string(),
            unit: dt.unit(),
            tz: dt.timezone(),
            nullable,
            metadata: Headers::new(),
            _backing: PhantomData,
        }
    }

    /// The column name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The resolution.
    pub fn unit(&self) -> TimeUnit {
        self.unit
    }

    /// The timezone.
    pub fn timezone(&self) -> Tz {
        self.tz
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

    field_setters!();

    /// The typed descriptor.
    pub fn data_type(&self) -> TemporalType<B> {
        TemporalType::new(self.unit, self.tz)
    }

    /// The erased runtime [`Field`], with the resolution stashed under
    /// [`TIME_UNIT_METADATA_KEY`](DataTypeId::TIME_UNIT_METADATA_KEY) and the timezone under
    /// [`TIMEZONE_METADATA_KEY`](DataTypeId::TIMEZONE_METADATA_KEY) — the axes the erased field's
    /// byte width alone cannot express (a `time32` may be seconds or milliseconds; a timestamp any
    /// fixed unit; a zoned timestamp any zone).
    pub fn erase(&self) -> Field {
        let mut metadata = self.metadata.clone();
        metadata.insert(DataTypeId::TIME_UNIT_METADATA_KEY, self.unit.name());
        metadata.insert(DataTypeId::TIMEZONE_METADATA_KEY, &self.tz.name());
        Field::new(&self.name, &self.data_type(), self.nullable).with_metadata(metadata)
    }

    /// This field as an [`arrow_schema::Field`] — its exact Arrow temporal type (feature `arrow`).
    /// The narrow forms whose plain Arrow mapping is ambiguous (`ts32` / `ts96` /
    /// `duration32` widen or map to `FixedSizeBinary`) are tagged under
    /// [`DataTypeId::METADATA_KEY`] so [`from_arrow`](TemporalField::from_arrow) recovers them.
    #[cfg(feature = "arrow")]
    pub fn to_arrow(&self) -> arrow_schema::Field {
        self.erase().to_arrow()
    }

    /// Builds a temporal field from an [`arrow_schema::Field`], or `None` if its
    /// (metadata-refined) logical type is not this column's. User metadata is preserved (feature
    /// `arrow`).
    #[cfg(feature = "arrow")]
    pub fn from_arrow(field: &arrow_schema::Field) -> Option<Self> {
        let erased = Field::from_arrow(field)?;
        if FieldType::type_id(&erased) != B::TYPE_ID {
            return None;
        }
        // The erased `Field::from_arrow` captured the resolution / zone into the reserved keys.
        let unit = erased
            .metadata()
            .get(DataTypeId::TIME_UNIT_METADATA_KEY)
            .and_then(TimeUnit::parse)
            .unwrap_or(B::DEFAULT_UNIT);
        let tz = erased
            .metadata()
            .get(DataTypeId::TIMEZONE_METADATA_KEY)
            .and_then(Tz::parse)
            .unwrap_or(Tz::NAIVE);
        let mut metadata = erased.metadata().clone();
        metadata.remove(DataTypeId::TIME_UNIT_METADATA_KEY);
        metadata.remove(DataTypeId::TIMEZONE_METADATA_KEY);
        Some(Self::new(erased.name(), unit, tz, erased.nullable()).with_metadata(metadata))
    }
}

impl<B: TemporalBacking> FieldType for TemporalField<B> {
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

impl<B: TemporalBacking> Clone for TemporalField<B> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            unit: self.unit,
            tz: self.tz,
            nullable: self.nullable,
            metadata: self.metadata.clone(),
            _backing: PhantomData,
        }
    }
}
impl<B: TemporalBacking> PartialEq for TemporalField<B> {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.unit == other.unit
            && self.tz == other.tz
            && self.nullable == other.nullable
            && self.metadata == other.metadata
    }
}
impl<B: TemporalBacking> Eq for TemporalField<B> {}
impl<B: TemporalBacking> core::fmt::Debug for TemporalField<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TemporalField")
            .field("name", &self.name)
            .field("type", &B::NAME)
            .field("unit", &self.unit)
            .field("tz", &self.tz)
            .field("nullable", &self.nullable)
            .finish()
    }
}
