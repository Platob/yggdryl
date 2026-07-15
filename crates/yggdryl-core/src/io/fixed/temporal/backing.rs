//! The two **shared temporal traits** the whole columnar `temporal` family is generic over:
//! [`TemporalNative`] (the per-value bridge — its physical count + a width-exact codec, one impl per
//! value type) and [`TemporalBacking`] (a zero-sized marker tying a concept+width to its
//! [`DataTypeId`](crate::io::DataTypeId), name, physical width, unit/timezone *capability*, and
//! default unit).
//!
//! A temporal value is a fixed-width **two's-complement** integer count — days / clock ticks / an
//! epoch count / a span — carried with a [`TimeUnit`] resolution and (for timestamps) a [`Tz`]. The
//! columnar layer fixes one `(unit, tz)` for the whole column (Arrow's model) and stores only the
//! raw counts; these two traits are how the column reads a count out of, and rebuilds a value into,
//! each concrete value type.

use crate::io::DataTypeId;

use super::{
    Date32, Date64, Duration32, Duration64, Temporal, TemporalError, Time32, Time64, TimeUnit,
    Ts32, Ts64, Ts96, Tz,
};

/// The **per-value bridge** every temporal value type implements — its physical integer count (the
/// day / clock / epoch / span count), the total constructor that rebuilds a value from a count at a
/// given `(unit, tz)`, and a width-exact little-endian codec over that count.
///
/// The resolution / timezone accessors come from the [`Temporal`] supertrait
/// ([`time_unit`](Temporal::time_unit) / [`timezone`](Temporal::timezone)), so a column can read
/// those two axes uniformly. `Send + Sync + 'static` because the language bindings hold columns of
/// these across threads.
pub trait TemporalNative:
    Temporal
    + Copy
    + PartialEq
    + Eq
    + core::hash::Hash
    + core::fmt::Debug
    + core::fmt::Display
    + Send
    + Sync
    + 'static
{
    /// The physical count's byte width (`4` / `8` / `12`) — the two's-complement little-endian
    /// width the column stores one count in.
    const WIDTH: usize;

    /// This value's **physical count** — days (`Date32`) / milliseconds (`Date64`), the clock count
    /// (`Time*`), the epoch count (`Ts*`), or the span count (`Duration*`) — in this value's own
    /// [`unit`](Temporal::time_unit).
    fn to_count(&self) -> i128;

    /// The value with physical `count` at resolution `unit` and zone `tz` — total and
    /// **range-checked**: a guided [`OutOfRange`](TemporalError::OutOfRange) (or
    /// [`UnsupportedUnit`](TemporalError::UnsupportedUnit)) if the count/unit does not fit this
    /// type. `unit` / `tz` are ignored by the types that fix them (`Date*` are day/naive).
    fn from_count(count: i128, unit: TimeUnit, tz: Tz) -> Result<Self, TemporalError>
    where
        Self: Sized;

    /// Writes this value's count into the first [`WIDTH`](TemporalNative::WIDTH) little-endian
    /// (two's-complement) bytes of `out` — no allocation.
    fn write_le(&self, out: &mut [u8]) {
        out[..Self::WIDTH].copy_from_slice(&self.to_count().to_le_bytes()[..Self::WIDTH]);
    }

    /// Reads a value from the first [`WIDTH`](TemporalNative::WIDTH) little-endian bytes
    /// (sign-extended to the full count), reconstructed at `unit` / `tz`.
    fn read_le(bytes: &[u8], unit: TimeUnit, tz: Tz) -> Result<Self, TemporalError>
    where
        Self: Sized,
    {
        let mut buf = [0u8; 16];
        buf[..Self::WIDTH].copy_from_slice(&bytes[..Self::WIDTH]);
        // Sign-extend the high bytes when the stored value is negative (two's complement).
        if Self::WIDTH < 16 && buf[Self::WIDTH - 1] & 0x80 != 0 {
            for byte in &mut buf[Self::WIDTH..] {
                *byte = 0xff;
            }
        }
        Self::from_count(i128::from_le_bytes(buf), unit, tz)
    }
}

impl TemporalNative for Date32 {
    const WIDTH: usize = 4;
    fn to_count(&self) -> i128 {
        self.days() as i128
    }
    fn from_count(count: i128, _unit: TimeUnit, _tz: Tz) -> Result<Self, TemporalError> {
        i32::try_from(count)
            .map(Date32::from_days)
            .map_err(|_| TemporalError::OutOfRange { ty: "date32" })
    }
}

impl TemporalNative for Date64 {
    const WIDTH: usize = 8;
    fn to_count(&self) -> i128 {
        self.millis() as i128
    }
    fn from_count(count: i128, _unit: TimeUnit, _tz: Tz) -> Result<Self, TemporalError> {
        i64::try_from(count)
            .map(Date64::from_millis)
            .map_err(|_| TemporalError::OutOfRange { ty: "date64" })
    }
}

impl TemporalNative for Time32 {
    const WIDTH: usize = 4;
    fn to_count(&self) -> i128 {
        self.value() as i128
    }
    fn from_count(count: i128, unit: TimeUnit, _tz: Tz) -> Result<Self, TemporalError> {
        let value = i32::try_from(count).map_err(|_| TemporalError::OutOfRange { ty: "time32" })?;
        Time32::new(value, unit)
    }
}

impl TemporalNative for Time64 {
    const WIDTH: usize = 8;
    fn to_count(&self) -> i128 {
        self.value() as i128
    }
    fn from_count(count: i128, unit: TimeUnit, _tz: Tz) -> Result<Self, TemporalError> {
        let value = i64::try_from(count).map_err(|_| TemporalError::OutOfRange { ty: "time64" })?;
        Time64::new(value, unit)
    }
}

impl TemporalNative for Ts32 {
    const WIDTH: usize = 4;
    fn to_count(&self) -> i128 {
        self.epoch_value()
    }
    fn from_count(count: i128, unit: TimeUnit, tz: Tz) -> Result<Self, TemporalError> {
        Ts32::from_epoch(count, unit, tz)
    }
}

impl TemporalNative for Ts64 {
    const WIDTH: usize = 8;
    fn to_count(&self) -> i128 {
        self.epoch_value()
    }
    fn from_count(count: i128, unit: TimeUnit, tz: Tz) -> Result<Self, TemporalError> {
        Ts64::from_epoch(count, unit, tz)
    }
}

impl TemporalNative for Ts96 {
    const WIDTH: usize = 12;
    fn to_count(&self) -> i128 {
        self.epoch_value()
    }
    fn from_count(count: i128, unit: TimeUnit, tz: Tz) -> Result<Self, TemporalError> {
        Ts96::from_epoch(count, unit, tz)
    }
}

impl TemporalNative for Duration32 {
    const WIDTH: usize = 4;
    fn to_count(&self) -> i128 {
        self.value() as i128
    }
    fn from_count(count: i128, unit: TimeUnit, _tz: Tz) -> Result<Self, TemporalError> {
        let value =
            i32::try_from(count).map_err(|_| TemporalError::OutOfRange { ty: "duration32" })?;
        Duration32::new(value, unit)
    }
}

impl TemporalNative for Duration64 {
    const WIDTH: usize = 8;
    fn to_count(&self) -> i128 {
        self.value() as i128
    }
    fn from_count(count: i128, unit: TimeUnit, _tz: Tz) -> Result<Self, TemporalError> {
        let value =
            i64::try_from(count).map_err(|_| TemporalError::OutOfRange { ty: "duration64" })?;
        Duration64::new(value, unit)
    }
}

/// A **temporal column concept+width** — the zero-sized marker (`Date32Kind` … `Duration64Kind`)
/// that ties a [`TemporalNative`] value type to the rest of the columnar family: its
/// [`DataTypeId`], canonical name, physical width, whether it carries a runtime unit / timezone,
/// its default unit, and which units it admits. The columnar descriptors
/// [`TemporalType`](super::TemporalType) / [`TemporalField`](super::TemporalField) /
/// [`TemporalScalar`](super::TemporalScalar) / [`TemporalSerie`](super::TemporalSerie) are all
/// generic over `B: TemporalBacking`.
///
/// `Send + Sync + 'static` — the language bindings hold every column type.
pub trait TemporalBacking: Copy + Default + Send + Sync + 'static {
    /// The value type this column reads out and rebuilds.
    type Native: TemporalNative;

    /// The stable, lower-case type name (`"date32"` … `"duration64"`).
    const NAME: &'static str;
    /// The physical count width in bytes (`4` / `8` / `12`).
    const WIDTH: usize;
    /// The [`DataTypeId`] — `Date32` … `Duration64`.
    const TYPE_ID: DataTypeId;
    /// Whether the column carries a runtime resolution (`false` for `Date32` / `Date64`, whose unit
    /// is fixed by the id — day / millisecond).
    const CARRIES_UNIT: bool;
    /// Whether the column carries a runtime timezone (`true` only for the timestamps).
    const CARRIES_TZ: bool;
    /// The default resolution a rejected / absent unit clamps to.
    const DEFAULT_UNIT: TimeUnit;

    /// Whether `unit` is a valid resolution for this type — `Time32 ⊆ {Second, Millisecond}`,
    /// `Time64 ⊆ {Microsecond, Nanosecond}`, `Date*` fixed to their single unit, and the
    /// timestamps / durations any **fixed** unit (they reject the calendar units `Month` / `Year`).
    fn allows_unit(unit: TimeUnit) -> bool;
}
