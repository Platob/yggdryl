//! [`TemporalType<B>`] — the **columnar temporal descriptor**: a temporal concept+width `B`
//! together with a runtime `(unit, tz)`, mapping to Arrow's `Date*` / `Time*` / `Timestamp` /
//! `Duration` (and, for the wide `ts96`, `FixedSizeBinary(12)`).
//!
//! Unlike the self-describing value types (whose unit / zone ride each value), a *column* fixes one
//! `(unit, tz)` for every element — Arrow's model — so the descriptor carries them and the
//! [`TemporalScalar`](super::TemporalScalar) / [`TemporalSerie`](super::TemporalSerie) store only
//! raw physical counts.

use core::marker::PhantomData;

use super::{TemporalBacking, TemporalField, TimeUnit, Tz};
use crate::io::{DataType, DataTypeId, TypedDataType};

/// The descriptor of a temporal column of concept+width `B`, resolution `unit`, and timezone `tz`.
///
/// [`new`](TemporalType::new) clamps the request to what `B` admits: an unsupported `unit` falls
/// back to `B`'s default, and `tz` is forced to [`Tz::NAIVE`] for the zone-less types (`Date*` /
/// `Time*` / `Duration*`).
pub struct TemporalType<B: TemporalBacking> {
    unit: TimeUnit,
    tz: Tz,
    _backing: PhantomData<B>,
}

impl<B: TemporalBacking> TemporalType<B> {
    /// A descriptor for this column at `(unit, tz)`. **Clamps** `unit` to
    /// [`B::allows_unit`](TemporalBacking::allows_unit) (falling back to
    /// [`B::DEFAULT_UNIT`](TemporalBacking::DEFAULT_UNIT)) and `tz` to [`Tz::NAIVE`] when `B` does
    /// not carry a timezone — so the mapping to Arrow is always valid.
    pub fn new(unit: TimeUnit, tz: Tz) -> Self {
        let unit = if B::allows_unit(unit) {
            unit
        } else {
            B::DEFAULT_UNIT
        };
        let tz = if B::CARRIES_TZ { tz } else { Tz::NAIVE };
        Self {
            unit,
            tz,
            _backing: PhantomData,
        }
    }

    /// The resolution.
    pub fn unit(&self) -> TimeUnit {
        self.unit
    }

    /// The timezone ([`Tz::NAIVE`] for the zone-less types).
    pub fn timezone(&self) -> Tz {
        self.tz
    }

    /// A [`TemporalField`] naming a column of this type.
    pub fn field(&self, name: &str, nullable: bool) -> TemporalField<B> {
        TemporalField::new(name, self.unit, self.tz, nullable)
    }

    /// This descriptor as an [`arrow_schema::DataType`] — the column's Arrow temporal type (feature
    /// `arrow`); a `unit` Arrow cannot model falls back to the id's default.
    #[cfg(feature = "arrow")]
    pub fn to_arrow(&self) -> arrow_schema::DataType {
        B::TYPE_ID
            .to_arrow_temporal(self.unit, self.tz)
            .unwrap_or_else(|| B::TYPE_ID.to_arrow(B::WIDTH))
    }

    /// Builds a descriptor from an [`arrow_schema::DataType`], or `None` if it is not the Arrow type
    /// this width **unambiguously** maps back to (a bare `Timestamp` recovers as `ts64`; the narrow
    /// `ts32` / `ts96` need the field's logical-type tag — see
    /// [`TemporalField::from_arrow`](super::TemporalField::from_arrow)) (feature `arrow`).
    #[cfg(feature = "arrow")]
    pub fn from_arrow(data_type: &arrow_schema::DataType) -> Option<Self> {
        if DataTypeId::from_arrow(data_type).map(|(id, _)| id) != Some(B::TYPE_ID) {
            return None;
        }
        let (unit, tz) = DataTypeId::arrow_temporal_params(data_type)?;
        Some(Self::new(unit, tz))
    }
}

impl<B: TemporalBacking> DataType for TemporalType<B> {
    fn name(&self) -> &'static str {
        B::NAME
    }

    fn byte_width(&self) -> usize {
        B::WIDTH
    }

    fn type_id(&self) -> DataTypeId {
        B::TYPE_ID
    }

    // Override the centralized default: the erased `type_id().to_arrow(byte_width)` cannot know the
    // resolution / timezone, so the descriptor supplies them.
    #[cfg(feature = "arrow")]
    fn to_arrow(&self) -> arrow_schema::DataType {
        self.to_arrow()
    }
}

impl<B: TemporalBacking> TypedDataType for TemporalType<B> {
    /// The logical element is the self-describing value type.
    type Native = B::Native;
}

// Value semantics (a plain, comparable, hashable descriptor).
impl<B: TemporalBacking> Clone for TemporalType<B> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<B: TemporalBacking> Copy for TemporalType<B> {}
impl<B: TemporalBacking> PartialEq for TemporalType<B> {
    fn eq(&self, other: &Self) -> bool {
        self.unit == other.unit && self.tz == other.tz
    }
}
impl<B: TemporalBacking> Eq for TemporalType<B> {}
impl<B: TemporalBacking> core::hash::Hash for TemporalType<B> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        B::NAME.hash(state);
        self.unit.hash(state);
        self.tz.hash(state);
    }
}
impl<B: TemporalBacking> core::fmt::Debug for TemporalType<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(self, f)
    }
}

impl<B: TemporalBacking> core::fmt::Display for TemporalType<B> {
    /// The type signature `name[unit]` (or `name[unit, tz]` for a zoned timestamp), e.g.
    /// `ts64[us, UTC]`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}[{}", B::NAME, self.unit.abbreviation())?;
        if B::CARRIES_TZ && !self.tz.is_naive() {
            write!(f, ", {}", self.tz.name())?;
        }
        f.write_str("]")
    }
}
