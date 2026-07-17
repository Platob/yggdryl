//! [`TemporalScalar<B>`] — one nullable temporal value carried with its column `(unit, tz)`. Its
//! logical value is the concept+width's value type; identity is over the raw parts (present flag,
//! physical count, unit, zone — so it never claims cross-unit instant equality), and it round-trips
//! through any [`IOCursor`] byte sink. A **present** scalar's count is validated on construction /
//! decode, so [`value`](TemporalScalar::value) is total (`Some`) for every non-null scalar.

use core::marker::PhantomData;

use super::time::{unit_from_tag, unit_tag};
use super::{
    Temporal, TemporalBacking, TemporalError, TemporalField, TemporalNative, TemporalSerie,
    TemporalType, TimeUnit, Tz,
};
use crate::io::field_carrier::field_accessors;
use crate::io::{AnyField, Bytes, IOCursor, IoError, ScalarType};

/// The largest temporal count is 12 bytes (`ts96`); a stack scratch of this size encodes one count
/// with no allocation while writing the byte codec.
const MAX_WIDTH: usize = 12;

/// A single, possibly-null temporal value of concept+width `B`, resolution `unit`, zone `tz`.
///
/// ```
/// use yggdryl_core::io::fixed::Ts64Scalar;
/// use yggdryl_core::io::fixed::temporal::{Ts64, TimeUnit, Tz};
///
/// let s = Ts64Scalar::of(Ts64::from_epoch(1_000, TimeUnit::Second, Tz::UTC).unwrap());
/// assert_eq!(s.value().unwrap().epoch_value(), 1_000);
/// assert_eq!(s.unit(), TimeUnit::Second);
/// assert!(Ts64Scalar::null(TimeUnit::Second, Tz::UTC).is_null());
/// ```
pub struct TemporalScalar<B: TemporalBacking> {
    count: Option<i128>,
    /// The value's own [`TemporalField`] descriptor — its name, declared nullability, metadata, and
    /// the `(unit, tz)` dtype params. The `(unit, tz)` join the count in identity; the name /
    /// nullable / metadata are excluded.
    field: TemporalField<B>,
    _backing: PhantomData<B>,
}

impl<B: TemporalBacking> TemporalScalar<B> {
    /// A present scalar from `value`, taking the value's own resolution and zone as the column's.
    pub fn of(value: B::Native) -> Self {
        Self::from_parts(Some(value.to_count()), value.time_unit(), value.timezone())
    }

    /// The null scalar of the given `(unit, tz)` (clamped to what `B` admits).
    pub fn null(unit: TimeUnit, tz: Tz) -> Self {
        Self::from_parts(None, unit, tz)
    }

    /// A scalar from an already-fitted physical count at `(unit, tz)` — the column's bridge to a
    /// scalar (kept crate-only; the count is in range by construction, `unit`/`tz` already clamped).
    pub(crate) fn from_parts(count: Option<i128>, unit: TimeUnit, tz: Tz) -> Self {
        Self {
            count,
            field: TemporalField::new("", unit, tz, false),
            _backing: PhantomData,
        }
    }

    /// The value, or `None` if null.
    pub fn value(&self) -> Option<B::Native> {
        self.count
            .and_then(|count| B::Native::from_count(count, self.unit(), self.timezone()).ok())
    }

    /// The raw physical count, or `None` if null.
    pub fn count(&self) -> Option<i128> {
        self.count
    }

    /// Whether the scalar is null.
    pub fn is_null(&self) -> bool {
        self.count.is_none()
    }

    /// The resolution (from the held field).
    pub fn unit(&self) -> TimeUnit {
        self.field.unit()
    }

    /// The timezone (from the held field).
    pub fn timezone(&self) -> Tz {
        self.field.timezone()
    }

    field_accessors!();

    /// The erased [`AnyField`] this scalar contributes — its **held field** (name + metadata +
    /// unit/tz) with **effective** nullability `self.nullable() || self.is_null()`.
    pub fn field(&self) -> AnyField {
        let mut field = self.field.clone();
        field.set_nullable(self.nullable() || self.is_null());
        AnyField::leaf(field.erase())
    }

    /// Like [`field`](TemporalScalar::field) but **consumes** the scalar.
    pub fn into_field(mut self) -> AnyField {
        let nullable = self.nullable() || self.is_null();
        self.field.set_nullable(nullable);
        AnyField::leaf(self.field.erase())
    }

    /// The typed descriptor.
    pub fn data_type(&self) -> TemporalType<B> {
        TemporalType::new(self.unit(), self.timezone())
    }

    /// This scalar **broadcast to a length-1 [`TemporalSerie`]** at its own `(unit, tz)` — the
    /// inverse of [`TemporalSerie::as_scalar`](TemporalSerie::as_scalar). Mirrors the fixed family's
    /// [`Scalar::to_serie`](crate::io::fixed::Scalar::to_serie); fallible only because the column
    /// re-expresses each value at its unit (the scalar's value already fits its own `(unit, tz)`, so
    /// it never fails in practice).
    ///
    /// ```
    /// use yggdryl_core::io::fixed::Ts64Scalar;
    /// use yggdryl_core::io::fixed::temporal::{Ts64, TimeUnit, Tz};
    ///
    /// let s = Ts64Scalar::of(Ts64::from_epoch(1_000, TimeUnit::Second, Tz::UTC).unwrap());
    /// let col = s.to_serie().unwrap();
    /// assert_eq!(col.len(), 1);
    /// assert_eq!(col.get(0).unwrap().epoch_value(), 1_000);
    /// ```
    pub fn to_serie(&self) -> Result<TemporalSerie<B>, TemporalError> {
        TemporalSerie::from_scalar(self.clone())
    }

    /// Writes this scalar — `[validity: u8][unit tag: u8][tz name][count: LE]` (the count is zero
    /// when null) — advancing the sink's cursor. The timezone name is length-prefixed (`u16`).
    pub fn write_to<W: IOCursor>(&self, sink: &mut W) -> Result<(), IoError> {
        sink.write_all(&[u8::from(self.count.is_some()), unit_tag(self.unit())])?;
        let name = self.timezone().name();
        sink.write_all(&(name.len() as u16).to_le_bytes())?;
        sink.write_all(name.as_bytes())?;
        let mut scratch = [0u8; MAX_WIDTH];
        let count = self.count.unwrap_or(0);
        scratch[..B::WIDTH].copy_from_slice(&count.to_le_bytes()[..B::WIDTH]);
        sink.write_all(&scratch[..B::WIDTH])
    }

    /// The serialized byte length of this scalar — `[validity][unit tag][tz name len + name][count]`.
    fn encoded_len(&self) -> usize {
        2 + 2 + self.timezone().name().len() + B::WIDTH
    }

    /// Reads a scalar written by [`write_to`](TemporalScalar::write_to), advancing the source cursor.
    /// The decoded `(unit, tz)` are clamped to what `B` admits (so they always agree with the
    /// descriptor), and a **present** count is validated to reconstruct — a guided
    /// [`Unsupported`](IoError::Unsupported) if it does not fit `B`.
    pub fn read_from<R: IOCursor>(source: &mut R) -> Result<Self, IoError> {
        let err = || IoError::Unsupported {
            what: format!("corrupt {} scalar frame", B::NAME),
        };
        let mut head = [0u8; 2];
        source.read_exact(&mut head)?;
        let present = head[0] != 0;
        let unit = unit_from_tag(head[1]).ok_or_else(err)?;
        let mut name_len = [0u8; 2];
        source.read_exact(&mut name_len)?;
        let name = source.read_exact_vec(u16::from_le_bytes(name_len) as usize)?;
        let tz = core::str::from_utf8(&name)
            .ok()
            .and_then(Tz::parse)
            .ok_or_else(err)?;
        let count_bytes = source.read_exact_vec(B::WIDTH)?;
        // Clamp the decoded axes so a stored `(unit, tz)` always agrees with `B`.
        let dt = TemporalType::<B>::new(unit, tz);
        let (unit, tz) = (dt.unit(), dt.timezone());
        let count = if present {
            let raw = read_count_le(&count_bytes, B::WIDTH);
            // A present count must reconstruct, so `value()` stays total and identity is exact.
            B::Native::from_count(raw, unit, tz).map_err(|error| IoError::Unsupported {
                what: format!("invalid {} scalar count {raw}: {error}", B::NAME),
            })?;
            Some(raw)
        } else {
            None
        };
        Ok(Self::from_parts(count, unit, tz))
    }

    /// This scalar's canonical bytes — the [`write_to`](TemporalScalar::write_to) frame as an owned
    /// `Vec`, the exact inverse of [`deserialize_bytes`](TemporalScalar::deserialize_bytes) and the
    /// codec the Python / Node bindings expose (`serialize_bytes` / `serializeBytes`).
    pub fn serialize_bytes(&self) -> Vec<u8> {
        let mut sink = Bytes::with_capacity(self.encoded_len());
        self.write_to(&mut sink)
            .expect("writing to an in-memory buffer is infallible");
        sink.as_slice().to_vec()
    }

    /// Reconstructs a scalar from the bytes produced by
    /// [`serialize_bytes`](TemporalScalar::serialize_bytes), erroring on a truncated frame.
    pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, IoError> {
        Self::read_from(&mut Bytes::from_slice(bytes))
    }
}

/// Sign-extends `width` little-endian bytes into an `i128` count.
fn read_count_le(bytes: &[u8], width: usize) -> i128 {
    let mut buf = [0u8; 16];
    buf[..width].copy_from_slice(&bytes[..width]);
    if width < 16 && buf[width - 1] & 0x80 != 0 {
        for byte in &mut buf[width..] {
            *byte = 0xff;
        }
    }
    i128::from_le_bytes(buf)
}

impl<B: TemporalBacking> ScalarType for TemporalScalar<B> {
    type Data = TemporalType<B>;

    fn data_type(&self) -> TemporalType<B> {
        TemporalType::new(self.unit(), self.timezone())
    }

    fn is_null(&self) -> bool {
        self.count.is_none()
    }
}

// Identity is over the **raw parts** (present flag, physical count, unit, zone) — not the
// reconstructed value, which would be fallible: a present scalar whose count is out of range would
// reconstruct to `None` and wrongly compare equal to a null. Two present scalars are equal iff their
// `(count, unit, tz)` match (so this never claims cross-unit instant equality), a present scalar
// never equals a null, and two nulls are equal regardless of `(unit, tz)` (like the null of any
// type) — so, as with `DecimalScalar`, `serialize_bytes` (which records the unit/tz even for a null)
// is a correct inverse without being a pure function of this identity for nulls.
impl<B: TemporalBacking> PartialEq for TemporalScalar<B> {
    fn eq(&self, other: &Self) -> bool {
        match (self.count, other.count) {
            (Some(a), Some(b)) => {
                a == b && self.unit() == other.unit() && self.timezone() == other.timezone()
            }
            (None, None) => true,
            _ => false,
        }
    }
}
impl<B: TemporalBacking> Eq for TemporalScalar<B> {}
impl<B: TemporalBacking> core::hash::Hash for TemporalScalar<B> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        match self.count {
            Some(count) => {
                state.write_u8(1);
                count.hash(state);
                self.unit().hash(state);
                self.timezone().hash(state);
            }
            None => state.write_u8(0),
        }
    }
}
impl<B: TemporalBacking> Clone for TemporalScalar<B> {
    fn clone(&self) -> Self {
        Self {
            count: self.count,
            field: self.field.clone(),
            _backing: PhantomData,
        }
    }
}
impl<B: TemporalBacking> core::fmt::Debug for TemporalScalar<B> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TemporalScalar")
            .field("type", &B::NAME)
            .field("unit", &self.unit())
            .field("tz", &self.timezone())
            .field("value", &self.value().map(|v| v.to_string()))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::super::time::unit_tag;
    use super::super::{Time32, TimeUnit, Ts64, Tz};
    use crate::io::fixed::{Date32Scalar, Time32Scalar, Ts64Scalar};

    #[test]
    fn present_never_equals_null_and_nulls_are_equal() {
        let present = Ts64Scalar::of(Ts64::from_epoch(1, TimeUnit::Second, Tz::UTC).unwrap());
        let null = Ts64Scalar::null(TimeUnit::Second, Tz::UTC);
        assert_ne!(present, null);
        // Two nulls are equal regardless of `(unit, tz)`.
        assert_eq!(
            Ts64Scalar::null(TimeUnit::Second, Tz::UTC),
            Ts64Scalar::null(TimeUnit::Millisecond, Tz::NAIVE)
        );
    }

    #[test]
    fn deserialize_rejects_an_invalid_present_count() {
        let good = Time32Scalar::of(Time32::from_hms(1, 2, 3).unwrap());
        // A valid scalar round-trips.
        assert_eq!(
            Time32Scalar::deserialize_bytes(&good.serialize_bytes()).unwrap(),
            good
        );
        // 90_000 seconds exceeds a single day (86_400) — a present count that does not fit is an error,
        // never a silently-nulled scalar.
        let mut bytes = good.serialize_bytes();
        let n = bytes.len();
        bytes[n - 4..].copy_from_slice(&90_000i32.to_le_bytes());
        assert!(Time32Scalar::deserialize_bytes(&bytes).is_err());
    }

    #[test]
    fn read_from_clamps_unit_and_tz_to_the_backing() {
        // A foreign `date32` frame that wrongly tagged Second + "UTC".
        let mut frame = vec![1u8, unit_tag(TimeUnit::Second)];
        let name = "UTC";
        frame.extend_from_slice(&(name.len() as u16).to_le_bytes());
        frame.extend_from_slice(name.as_bytes());
        frame.extend_from_slice(&0i32.to_le_bytes());
        let s = Date32Scalar::deserialize_bytes(&frame).unwrap();
        assert_eq!(s.unit(), TimeUnit::Day);
        assert!(s.timezone().is_naive());
        assert_eq!(s.data_type().unit(), TimeUnit::Day);
    }
}
