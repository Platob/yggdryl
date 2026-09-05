//! The canonical byte representation of one [`Scalar`].
//!
//! Two spellings, for two different questions.
//!
//! [`Scalar::as_value_bytes`] is the **payload alone** - no tag, no length -
//! so hashing `Scalar::from("AAPL")` equals hashing `b"AAPL"` and agrees with
//! any other xxHash implementation given the same bytes. It borrows wherever
//! the value already holds bytes and never allocates.
//!
//! [`Scalar::write_bytes`] is the **total, prefix-free feed** over every
//! variant, and it is what a digest, a row key, and `stable_hash` all read.
//! The sink is [`std::hash::Hasher`] because every streaming state here
//! already implements it, so one feed serves all of them and no new trait is
//! needed.

use std::hash::Hasher;

use crate::types::decimal::scalars as decimal;
use crate::types::temporal::scalars::temporal_key;
use crate::types::{Decimal, Floating, Integer, Nested, Temporal};
use crate::{DataType, DataTypeId, Digest, DigestAlgorithm, I256, Scalar};

/// The tag byte a value nested past the shared recursion limit feeds instead
/// of descending further.
///
/// [`DataTypeId`] has 50 variants, so `0xff` is not one of them and cannot
/// collide with a real value's tag.
const TOO_DEEP: u8 = 0xff;

impl Scalar {
    /// Return this value's payload bytes, when it has bytes of its own.
    ///
    /// This is the payload and nothing else: no type tag and no length, so the
    /// answer for a string is exactly its UTF-8 and the answer for a byte
    /// value is exactly those bytes. It borrows from the value wherever the
    /// value already holds bytes, and is an inline fixed array otherwise, so
    /// it never allocates and never copies a string or a byte payload.
    ///
    /// `None` is the answer for [`Self::Null`], which has no payload, and for
    /// [`crate::types::nested::Sequence`], [`crate::types::nested::Mapping`], and
    /// [`crate::types::nested::Record`], whose
    /// bytes exist only under a framing. Use [`Self::write_bytes`] for those.
    ///
    /// A decimal answers its coefficient, and a temporal its stored count: the
    /// scale, unit, and zone beside them are the value's type rather than its
    /// payload.
    ///
    /// ```
    /// use yggdryl::{Scalar, xxhash};
    ///
    /// let symbol = Scalar::from("AAPL");
    /// assert_eq!(&*symbol.as_value_bytes().unwrap(), b"AAPL");
    /// // Which is what any other xxHash implementation would be given.
    /// assert_eq!(
    ///     xxhash::xxh3(&symbol.as_value_bytes().unwrap()),
    ///     xxhash::xxh3(b"AAPL"),
    /// );
    ///
    /// assert_eq!(&*Scalar::from(1).as_value_bytes().unwrap(), &[1, 0, 0, 0]);
    /// assert!(Scalar::Null.as_value_bytes().is_none());
    /// assert!(Scalar::from_sequence([]).as_value_bytes().is_none());
    /// ```
    pub fn as_value_bytes(&self) -> Option<ValueBytes<'_>> {
        let inline = match self {
            Self::Null | Self::Nested(_) => return None,
            Self::Text(value) => return Some(ValueBytes::borrowed(value.as_str().as_bytes())),
            Self::Ascii(value) => return Some(ValueBytes::borrowed(value.as_str().as_bytes())),
            Self::Enum(value) => return Some(ValueBytes::borrowed(value.as_str().as_bytes())),
            Self::Bytes(value) => return Some(ValueBytes::borrowed(value.as_bytes())),
            Self::Geospatial(value) => {
                return Some(ValueBytes::borrowed(value.as_bytes()));
            }
            Self::Uuid(value) => ValueBytes::inline(&value.into_bytes()),
            Self::Boolean(value) => ValueBytes::inline(&[u8::from(value.get())]),
            Self::Integer(value) => match value {
                Integer::I8(value) => ValueBytes::inline(&value.get().to_le_bytes()),
                Integer::I16(value) => ValueBytes::inline(&value.get().to_le_bytes()),
                Integer::I32(value) => ValueBytes::inline(&value.get().to_le_bytes()),
                Integer::I64(value) => ValueBytes::inline(&value.get().to_le_bytes()),
                Integer::I128(value) => ValueBytes::inline(&value.get().to_le_bytes()),
                Integer::U8(value) => ValueBytes::inline(&value.get().to_le_bytes()),
                Integer::U16(value) => ValueBytes::inline(&value.get().to_le_bytes()),
                Integer::U32(value) => ValueBytes::inline(&value.get().to_le_bytes()),
                Integer::U64(value) => ValueBytes::inline(&value.get().to_le_bytes()),
                Integer::U128(value) => ValueBytes::inline(&value.get().to_le_bytes()),
            },
            Self::Floating(value) => match value {
                Floating::F16(value) => ValueBytes::inline(&value.as_f16().to_bits().to_le_bytes()),
                Floating::F32(value) => ValueBytes::inline(&value.as_f32().to_bits().to_le_bytes()),
                Floating::F64(value) => ValueBytes::inline(&value.as_f64().to_bits().to_le_bytes()),
            },
            Self::Decimal(value) => match value {
                Decimal::D32(value) => ValueBytes::inline(&value.coefficient().to_le_bytes()),
                Decimal::D64(value) => ValueBytes::inline(&value.coefficient().to_le_bytes()),
                Decimal::D128(value) => ValueBytes::inline(&value.coefficient().to_le_bytes()),
                Decimal::D256(value) => ValueBytes::inline(&value.coefficient().into_le_bytes()),
            },
            Self::Temporal(Temporal::Date32(value)) => {
                ValueBytes::inline(&value.count().to_le_bytes())
            }
            Self::Temporal(Temporal::Time32(value)) => {
                ValueBytes::inline(&value.count().to_le_bytes())
            }
            Self::Temporal(Temporal::Duration32(value)) => {
                ValueBytes::inline(&value.count().to_le_bytes())
            }
            Self::Temporal(Temporal::Date64(value)) => {
                ValueBytes::inline(&value.count().to_le_bytes())
            }
            Self::Temporal(Temporal::Time64(value)) => {
                ValueBytes::inline(&value.count().to_le_bytes())
            }
            Self::Temporal(Temporal::DateTime64(value)) => {
                ValueBytes::inline(&value.count().to_le_bytes())
            }
            Self::Temporal(Temporal::Duration64(value)) => {
                ValueBytes::inline(&value.count().to_le_bytes())
            }
            Self::Temporal(Temporal::Interval(value)) => {
                let mut bytes = [0_u8; 16];
                bytes[..4].copy_from_slice(&value.months().to_le_bytes());
                bytes[4..8].copy_from_slice(&value.days().to_le_bytes());
                bytes[8..].copy_from_slice(&value.nanoseconds().to_le_bytes());
                ValueBytes::inline(&bytes)
            }
        };
        Some(inline)
    }

    /// Feed this value's canonical byte representation into `sink`.
    ///
    /// The feed is **total** - every variant has one - and **prefix-free**:
    /// each value's bytes start with a tag that fixes how many bytes follow,
    /// or that is followed by an explicit length, so concatenated values are
    /// uniquely decodable and `Sequence([a, b])` can never collide with
    /// `Sequence([ab])`.
    ///
    /// It is also **canonical for equality**: equal values feed identical
    /// bytes. Because [`Scalar`] compares across widths - `I8(1)`, `I64(1)`,
    /// and `U8(1)` are one value, and so are `F32(1.5)` and `F64(1.5)`, and
    /// `D128(100, 2)` and `D256(1, 0)` - the feed writes each family's
    /// canonical form rather than its storage width. A digest therefore
    /// identifies the value, not the box it came in.
    ///
    /// ```
    /// use yggdryl::{DigestAlgorithm, Scalar};
    ///
    /// // Equal values, different widths, one digest.
    /// assert_eq!(Scalar::from(1), Scalar::from(1));
    /// assert_eq!(
    ///     Scalar::from(1).digest(DigestAlgorithm::Xxh3),
    ///     Scalar::from(1).digest(DigestAlgorithm::Xxh3),
    /// );
    /// // Values that differ, across variant boundaries as much as within one.
    /// assert_ne!(
    ///     Scalar::from("1").digest(DigestAlgorithm::Xxh3),
    ///     Scalar::from(0x31).digest(DigestAlgorithm::Xxh3),
    /// );
    /// ```
    ///
    /// # Encoding
    ///
    /// Every value begins with one tag byte, which is a [`DataTypeId`]
    /// discriminant ([`DataTypeId::as_u8`]). That byte is a wire contract:
    /// inserting a variant into `DataTypeId` anywhere but the end changes
    /// stored digests, and the test pinning every value is what turns that
    /// into a failure rather than a surprise. Integers of every width feed one
    /// of two tags, `int128` or `uint128`, chosen by sign; the other families
    /// that compare across widths feed their widest member's tag.
    ///
    /// | Variant | Feed after the tag |
    /// | --- | --- |
    /// | `Null` | nothing |
    /// | `Bool` | `0x00` or `0x01` |
    /// | `I8`..`U128` | magnitude as `u128` little-endian; the tag carries the sign |
    /// | `F16`/`F32`/`F64` | the common `f64` reading's IEEE bits, little-endian |
    /// | `D128`/`D256` | normalized coefficient as `i256` little-endian, then scale as one signed byte |
    /// | `String` | length `u64` little-endian, then UTF-8 |
    /// | `Enum` | length-prefixed enum identity, then the member ordinal |
    /// | `Bytes`/`Geospatial` | length `u64` little-endian, then the bytes |
    /// | temporals | unit class byte, normalized count as `i128` little-endian, length-prefixed timezone |
    /// | `Sequence` | element count `u64` little-endian, then each element's feed |
    /// | `Mapping` | entry count `u64` little-endian, then each key feed and value feed in stored order |
    /// | `Record` | entry count `u64` little-endian, then per sorted entry a length-prefixed name and the value's feed |
    ///
    /// Nesting is bounded by the shared structured-value limit
    /// [`DataType::PARSE_RECURSION_LIMIT`]. A value nested deeper feeds a
    /// single reserved `0xff` in place of the subtree, so the feed stays total
    /// and allocation-free for any input; values that differ only below that
    /// depth are indistinguishable, exactly as [`Scalar::dtype`] refuses to
    /// name them.
    pub fn write_bytes(&self, sink: &mut impl Hasher) {
        self.feed(sink, 0);
    }

    /// Return this value's digest under `algorithm`.
    ///
    /// ```
    /// use yggdryl::{DigestAlgorithm, Scalar};
    ///
    /// let digest = Scalar::from("AAPL").digest(DigestAlgorithm::Xxh3);
    /// assert_eq!(digest.algorithm(), DigestAlgorithm::Xxh3);
    /// ```
    pub fn digest(&self, algorithm: DigestAlgorithm) -> Digest {
        let mut digester = algorithm.digester();
        digester.write_scalar(self);
        digester.as_digest()
    }

    /// Feed this value at `depth`, refusing to descend past the shared limit.
    fn feed(&self, sink: &mut impl Hasher, depth: usize) {
        if depth >= DataType::PARSE_RECURSION_LIMIT {
            sink.write(&[TOO_DEEP]);
            return;
        }
        // Every integer width is one value, so the sign picks the tag and the
        // magnitude is the payload: `I8(1)`, `U8(1)`, and `I64(1)` are equal
        // and must feed identically.
        if let Some(integer) = self.as_integer() {
            write_integer(sink, integer.is_negative(), integer.magnitude());
            return;
        }
        // All float widths widen exactly into binary64, which is the reading
        // their equality and ordering already share.
        if let Some(float) = self.as_float() {
            write_float(sink, float.as_f64());
            return;
        }
        // Decimals compare by the number they name, so the feed is the
        // normalized coefficient and scale rather than the stored pair.
        if let Some((unscaled, scale)) = self.as_decimal() {
            write_decimal(sink, unscaled, scale);
            return;
        }
        // Temporals compare by family, normalized count, and zone; the stored
        // width and unit are how the count is spelled, not what it is.
        if let Self::Temporal(Temporal::Interval(value)) = self {
            write_tag(sink, DataTypeId::Interval);
            sink.write(&value.months().to_le_bytes());
            sink.write(&value.days().to_le_bytes());
            sink.write(&value.nanoseconds().to_le_bytes());
            sink.write(&[value.unit() as u8]);
            return;
        }
        if let Some(temporal) = self.as_temporal() {
            write_temporal(
                sink,
                (*temporal).family(),
                (*temporal).count(),
                (*temporal).unit(),
                &temporal.timezone(),
            );
            return;
        }
        match self {
            Self::Null => write_null(sink),
            Self::Boolean(value) => write_bool(sink, value.get()),
            Self::Text(value) => write_string(sink, value.as_str()),
            Self::Ascii(value) => {
                write_tag(sink, DataTypeId::Ascii);
                write_text(sink, value.as_str());
            }
            Self::Uuid(value) => {
                write_tag(sink, DataTypeId::Uuid);
                sink.write(&value.into_bytes());
            }
            Self::Enum(value) => {
                write_tag(sink, DataTypeId::Dictionary);
                write_text(sink, value.kind());
                sink.write(&[value.ordinal()]);
            }
            Self::Bytes(value) => write_binary(sink, value.as_bytes()),
            Self::Geospatial(value) => write_geospatial(sink, value.as_bytes()),
            Self::Nested(Nested::Sequence(values)) => {
                write_sequence_header(sink, values.as_slice().len());
                for value in values.as_slice() {
                    value.feed(sink, depth + 1);
                }
            }
            Self::Nested(Nested::Mapping(entries)) => {
                write_tag(sink, DataTypeId::Map);
                write_len(sink, entries.as_slice().len());
                for (key, value) in entries.as_slice() {
                    key.feed(sink, depth + 1);
                    value.feed(sink, depth + 1);
                }
            }
            Self::Nested(Nested::Record(entries)) => {
                write_tag(sink, DataTypeId::Struct);
                write_len(sink, entries.as_map().len());
                for (name, value) in entries.as_map() {
                    write_text(sink, name);
                    value.feed(sink, depth + 1);
                }
            }
            // Every remaining variant answered one of the family views above.
            Self::Integer(_) => unreachable!("every integer width fed above"),
            Self::Floating(_) => unreachable!("every float width fed above"),
            Self::Decimal(_) => unreachable!("all decimal widths fed above"),
            Self::Temporal(_) => unreachable!("every temporal family fed above"),
        }
    }
}

impl<K: crate::types::FieldType> crate::TypedScalar<K> {
    /// Return this value's digest under `algorithm`.
    ///
    /// The datatype marker is validation, not content: the digest is the
    /// value's, so a `TypedScalar` and the `Scalar` inside it answer the same.
    pub fn digest(&self, algorithm: DigestAlgorithm) -> Digest {
        self.value().digest(algorithm)
    }
}

/// Write one [`DataTypeId`] discriminant as the value's tag.
fn write_tag(sink: &mut impl Hasher, id: DataTypeId) {
    sink.write(&[id.as_u8()]);
}

// The family writers below are the feed's one definition of each canonical
// form. `xxhash::arrow` reads Arrow buffers straight into them rather than
// materializing a `Scalar` first, so the buffer path and the value path cannot
// drift apart: there is one encoding, reached two ways.

/// Write the tag a value with no payload carries.
pub(super) fn write_null(sink: &mut impl Hasher) {
    write_tag(sink, DataTypeId::Null);
}

/// Write a boolean.
pub(super) fn write_bool(sink: &mut impl Hasher, value: bool) {
    write_tag(sink, DataTypeId::Boolean);
    sink.write(&[u8::from(value)]);
}

/// Write any integer width in its canonical sign-and-magnitude form.
pub(super) fn write_integer(sink: &mut impl Hasher, negative: bool, magnitude: u128) {
    let tag = if negative {
        DataTypeId::Int128
    } else {
        DataTypeId::UInt128
    };
    write_tag(sink, tag);
    sink.write(&magnitude.to_le_bytes());
}

/// Write a signed integer of any width.
#[cfg(feature = "arrow")]
pub(super) fn write_signed(sink: &mut impl Hasher, value: i128) {
    write_integer(sink, value < 0, value.unsigned_abs());
}

/// Write an unsigned integer of any width.
#[cfg(feature = "arrow")]
pub(super) fn write_unsigned(sink: &mut impl Hasher, value: u128) {
    write_integer(sink, false, value);
}

/// Write any float width as its common binary64 reading.
///
/// A NaN is normalized here rather than at the call site, so a raw Arrow
/// buffer holding a non-canonical NaN payload feeds what the equivalent
/// `Scalar` feeds.
pub(super) fn write_float(sink: &mut impl Hasher, value: f64) {
    let value = if value.is_nan() { f64::NAN } else { value };
    write_tag(sink, DataTypeId::Float64);
    sink.write(&value.to_bits().to_le_bytes());
}

/// Write an exact decimal as the number it names.
pub(super) fn write_decimal(sink: &mut impl Hasher, unscaled: I256, scale: i8) {
    let (unscaled, scale) = decimal::normalize(unscaled, scale);
    write_tag(sink, DataTypeId::Decimal256);
    sink.write(&unscaled.into_le_bytes());
    sink.write(&scale.to_le_bytes());
}

/// Write a temporal as its family, normalized count, and zone.
pub(super) fn write_temporal(
    sink: &mut impl Hasher,
    family: crate::TemporalFamily,
    count: i64,
    unit: crate::TimeUnit,
    zone: &crate::Timezone,
) {
    let tag = match family {
        crate::TemporalFamily::Date => DataTypeId::Date64,
        crate::TemporalFamily::Time => DataTypeId::Time64,
        crate::TemporalFamily::DateTime => DataTypeId::DateTime64,
        crate::TemporalFamily::Duration => DataTypeId::Duration64,
        crate::TemporalFamily::Interval => DataTypeId::Interval,
    };
    let (class, count) = temporal_key(count, unit);
    write_tag(sink, tag);
    sink.write(&[class]);
    sink.write(&count.to_le_bytes());
    write_text(sink, zone.as_str());
}

/// Write UTF-8 text as a string value.
pub(super) fn write_string(sink: &mut impl Hasher, text: &str) {
    write_tag(sink, DataTypeId::Utf8);
    write_text(sink, text);
}

/// Write opaque bytes as a byte value.
pub(super) fn write_binary(sink: &mut impl Hasher, bytes: &[u8]) {
    write_tag(sink, DataTypeId::Binary);
    write_len(sink, bytes.len());
    sink.write(bytes);
}

/// Write Well-Known Binary as a geospatial value.
pub(super) fn write_geospatial(sink: &mut impl Hasher, bytes: &[u8]) {
    write_tag(sink, DataTypeId::Geometry);
    write_len(sink, bytes.len());
    sink.write(bytes);
}

/// Write the tag and element count an ordered sequence starts with.
///
/// The elements follow, each feeding itself; a row is a sequence of its
/// columns, which is what lets a row digest be built without materializing the
/// row.
pub(super) fn write_sequence_header(sink: &mut impl Hasher, count: usize) {
    write_tag(sink, DataTypeId::List);
    write_len(sink, count);
}

/// Write a length as `u64` little-endian.
///
/// `Hasher`'s own `write_usize` and `write_u64` use native-endian bytes, which
/// would make a stored digest disagree between a big-endian and a
/// little-endian machine. Every integer in this feed goes through explicit
/// little-endian bytes for that reason.
fn write_len(sink: &mut impl Hasher, length: usize) {
    sink.write(&(length as u64).to_le_bytes());
}

/// Write UTF-8 text with its byte length in front.
fn write_text(sink: &mut impl Hasher, text: &str) {
    write_len(sink, text.len());
    sink.write(text.as_bytes());
}

/// One value's payload bytes, borrowed or inline.
///
/// Dereferences to the bytes themselves and compares as those bytes. The
/// inline form holds the widest fixed payload a value has - a 256-bit decimal
/// coefficient - so no width allocates.
#[derive(Clone, Copy)]
pub struct ValueBytes<'a>(Payload<'a>);

#[derive(Clone, Copy)]
enum Payload<'a> {
    Borrowed(&'a [u8]),
    Inline([u8; 32], u8),
}

impl<'a> ValueBytes<'a> {
    /// Borrow bytes the value already holds.
    fn borrowed(bytes: &'a [u8]) -> Self {
        Self(Payload::Borrowed(bytes))
    }

    /// Copy a fixed-width payload into the inline buffer.
    fn inline(bytes: &[u8]) -> Self {
        let mut inline = [0_u8; 32];
        inline[..bytes.len()].copy_from_slice(bytes);
        Self(Payload::Inline(inline, bytes.len() as u8))
    }
}

impl std::ops::Deref for ValueBytes<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match &self.0 {
            Payload::Borrowed(bytes) => bytes,
            Payload::Inline(bytes, length) => &bytes[..*length as usize],
        }
    }
}

impl AsRef<[u8]> for ValueBytes<'_> {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl std::fmt::Debug for ValueBytes<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, formatter)
    }
}

impl PartialEq for ValueBytes<'_> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl Eq for ValueBytes<'_> {}

impl std::hash::Hash for ValueBytes<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (**self).hash(state);
    }
}

/// The widest inline payload, so a decimal coefficient still borrows nothing.
const _: () = assert!(size_of::<I256>() == 32);
