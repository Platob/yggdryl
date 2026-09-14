//! UUID values and the typed scalar alias.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{
    DataType, DataTypeId, DataTypeKind, Error, Result, Scalar, ScalarFamily, ScalarValue, types,
};

/// One RFC 9562 identifier stored as its big-endian 128-bit value.
#[repr(transparent)]
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(transparent)]
pub struct Uuid(u128);

const _: () = assert!(std::mem::size_of::<Uuid>() == 16);

const MAX_V7_MICROS: i64 = ((1_i64 << 48) - 1) * 1_000 + 999;
const VARIANT_MASK: u128 = 3 << 62;
const RFC_VARIANT: u128 = 2 << 62;

impl Uuid {
    /// The length of the canonical spelling: 32 digits and four hyphens.
    pub const TEXT_LEN: usize = types::UUID_TEXT_LEN;

    /// Construct from the exact packed identifier.
    pub const fn new(value: u128) -> Self {
        Self(value)
    }

    /// Pack a Unix microsecond instant and the payload's low 62 bits as UUIDv7.
    ///
    /// The first 48 bits hold milliseconds; the 12-bit sub-millisecond
    /// fraction is `floor((unix_micros % 1000) * 4096 / 1000)`, following
    /// [RFC 9562's fractional-clock method](https://www.rfc-editor.org/rfc/rfc9562.html#section-6.2).
    /// Increasing microseconds sort in time order regardless of payload.
    /// This allocates nothing on success, reads no clock, and supplies no
    /// randomness or uniqueness guarantee of its own.
    ///
    /// ```
    /// use yggdryl::types::Uuid;
    /// # fn main() -> yggdryl::Result<()> {
    /// let value = Uuid::from_v7(1_645_557_742_000_456, 0xfedc_ba98_7654_3210)?;
    /// assert_eq!(value.to_string(), "017f22e2-79b0-774b-bedc-ba9876543210");
    /// assert!(Uuid::from_v7(999, u64::MAX)? < Uuid::from_v7(1_000, 0)?);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] at `$` when `unix_micros` is outside
    /// `0..=281474976710655999`, the range a 48-bit millisecond count holds.
    pub fn from_v7(unix_micros: i64, payload: u64) -> Result<Self> {
        if !(0..=MAX_V7_MICROS).contains(&unix_micros) {
            return Err(Error::InvalidRecord {
                path: "$".into(),
                reason: crate::text::expected_got(
                    format_args!("a UUIDv7 Unix microsecond instant in 0..={MAX_V7_MICROS}"),
                    format_args!("{unix_micros}"),
                ),
            });
        }
        // The range check proves unsigned arithmetic and the 48-bit layout.
        let micros = unix_micros.unsigned_abs();
        let milliseconds = u128::from(micros / 1_000);
        let fraction = u128::from((micros % 1_000) * 4_096 / 1_000);
        Ok(Self(
            (milliseconds << 80)
                | (7_u128 << 76)
                | (fraction << 64)
                | RFC_VARIANT
                | (u128::from(payload) & !VARIANT_MASK),
        ))
    }

    /// Set the UUIDv8 version and RFC variant, preserving the other 122 bits.
    ///
    /// The payload is already resolved: this selects no hash algorithm,
    /// reads no clock, and allocates nothing. The
    /// [RFC 9562 illustrative vector](https://www.rfc-editor.org/rfc/rfc9562.html#appendix-B.2)
    /// applies the same mask to the leading 128 bits of its example digest.
    ///
    /// ```
    /// use yggdryl::types::Uuid;
    ///
    /// let value = Uuid::from_v8(0x5c14_6b14_3c52_4afd_938a_375d_0df1_fbf6);
    /// assert_eq!(value.to_string(), "5c146b14-3c52-8afd-938a-375d0df1fbf6");
    /// ```
    pub const fn from_v8(payload: u128) -> Self {
        Self((payload & !((0xf_u128 << 76) | VARIANT_MASK)) | (8_u128 << 76) | RFC_VARIANT)
    }

    /// Pack a signed nanosecond instant and the digest's low 58 bits as UUIDv8.
    ///
    /// Flipping the sign bit orders all instants before distributing their
    /// 64 bits around the version/variant slots. Unit conversion and digest
    /// width validation belong to the time/hash value's boundary.
    pub(crate) fn from_time_hash(unix_nanoseconds: i64, digest: u64) -> Self {
        let instant =
            u128::from(u64::from_be_bytes(unix_nanoseconds.to_be_bytes()) ^ (1_u64 << 63));
        Self::from_v8(
            ((instant >> 16) << 80)
                | (((instant >> 4) & 0xfff) << 64)
                | ((instant & 0xf) << 58)
                | (u128::from(digest) & ((1_u128 << 58) - 1)),
        )
    }

    /// Parse the accepted hyphenated, compact-hex, or 16-byte representation.
    pub fn from_bytes(value: &[u8]) -> Result<Self> {
        Ok(Self(u128::from_be_bytes(types::uuid_parse(value)?)))
    }

    /// Return the packed identifier.
    pub const fn get(self) -> u128 {
        self.0
    }

    /// Return the canonical sixteen storage bytes.
    pub const fn into_bytes(self) -> [u8; 16] {
        self.0.to_be_bytes()
    }

    /// Write the canonical spelling into `slot` and borrow it back.
    ///
    /// Every rendering is exactly [`Self::TEXT_LEN`] bytes, so the caller
    /// holds the slot on the stack and nothing is allocated. This is what a
    /// writer that wants a `&str` asks for; [`Self::to_string`] is the same
    /// characters when an owned one is what the caller needs.
    ///
    /// ```
    /// use yggdryl::types::Uuid;
    ///
    /// let value = Uuid::from_v8(0x5c14_6b14_3c52_4afd_938a_375d_0df1_fbf6);
    /// let mut slot = [0_u8; Uuid::TEXT_LEN];
    /// assert_eq!(value.render(&mut slot), "5c146b14-3c52-8afd-938a-375d0df1fbf6");
    /// ```
    pub fn render(self, slot: &mut [u8; Self::TEXT_LEN]) -> &str {
        types::uuid_rendered(&self.0.to_be_bytes(), slot)
    }
}

impl fmt::Display for Uuid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The rendering is a fixed 36 bytes, so it is written on the stack
        // and never allocates.
        let mut slot = [0_u8; Self::TEXT_LEN];
        formatter.write_str(self.render(&mut slot))
    }
}

impl ScalarFamily for Uuid {
    const KIND: DataTypeKind = DataTypeKind::Uuid;

    fn id(&self) -> DataTypeId {
        DataTypeId::Uuid
    }

    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::Uuid)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Uuid(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Uuid(value) => Some(value),
            _ => None,
        }
    }
}

impl ScalarValue for Uuid {
    type Family = Self;

    const ID: DataTypeId = DataTypeId::Uuid;
    const KIND: DataTypeKind = DataTypeKind::Uuid;

    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::Uuid)
    }

    fn into_family(self) -> Self::Family {
        self
    }

    fn from_family(family: &Self::Family) -> Option<&Self> {
        Some(family)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Uuid(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        <Self as ScalarFamily>::from_scalar(value)
    }
}
