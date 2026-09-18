//! Globally unique identifier datatypes.

use std::fmt;

use serde::{Deserialize, Serialize};
use smol_str::{SmolStr, format_smolstr};

use crate::types::typed::define_field_types;
use crate::{DataType, Error, Result, Scalar, Value, types};

/// Arrow casts owned by this datatype family.
pub(crate) mod casts {
    use std::sync::Arc;

    use crate::arrow::{Error, Result};
    use crate::types::budget::MaterializationBudget;
    use crate::types::bytes::casts::variable_binary_source;
    use crate::types::cast::arrow_cast_exposed;
    use crate::types::cast::{downcast, internal_target_error};
    use crate::types::cast::columns::is_exposed;
    use crate::types::uuid_parse;
    use crate::{DataType, Field};
    use arrow_array::{Array, ArrayRef, BinaryArray, FixedSizeBinaryArray, StringArray};
    use arrow_buffer::{BooleanBuffer, BooleanBufferBuilder};
    use arrow_schema::DataType as ArrowDataType;

    /// Validates every exposed, non-null value entering a UUID and stores it as
    /// its sixteen bytes.
    ///
    /// Sixteen-byte storage is the same array once validated. Any other fixed
    /// width is a slot holding one of the two text spellings, so the padding is
    /// taken off and the spelling read; variable bytes are read as they are; and
    /// anything else first renders as Utf8 through Arrow's kernel, exactly as an
    /// ASCII width does.
    pub(crate) fn ingest_uuid_array(
        array: &ArrayRef,
        expected: &ArrowDataType,
        safe: bool,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        if !matches!(expected, ArrowDataType::FixedSizeBinary(16)) {
            return Err(internal_target_error("uuid"));
        }
        if let ArrowDataType::FixedSizeBinary(width) = array.data_type() {
            let source = downcast::<FixedSizeBinaryArray>(array.as_ref())?;
            if *width == 16 {
                for index in 0..source.len() {
                    if is_exposed(exposure, index) && source.is_valid(index) {
                        uuid_cell(field, index, source.value(index))?;
                    }
                }
                return Ok(Arc::clone(array));
            }
            // Sixteen bytes are an identifier, in which every byte carries
            // identity and a trailing NUL is one of them. Any other width is a
            // text slot, so its trailing NUL is the slot's padding.
            return uuid_storage(field, source.len(), exposure, budget, |index| {
                source
                    .is_valid(index)
                    .then(|| crate::types::trim_padding(source.value(index)))
            });
        }
        if let Some(bytes) = variable_binary_source(array, field, exposure, budget)? {
            let source = downcast::<BinaryArray>(bytes.as_ref())?;
            return uuid_storage(field, source.len(), exposure, budget, |index| {
                source.is_valid(index).then(|| source.value(index))
            });
        }
        let text = if array.data_type() == &ArrowDataType::Utf8 {
            Arc::clone(array)
        } else {
            arrow_cast_exposed(
                array,
                &ArrowDataType::Utf8,
                safe,
                exposure,
                &Field::new(field.name(), DataType::utf8(), true),
                budget,
            )?
        };
        let source = downcast::<StringArray>(text.as_ref())?;
        uuid_storage(field, source.len(), exposure, budget, |index| {
            source
                .is_valid(index)
                .then(|| source.value(index).as_bytes())
        })
    }

    /// Builds the sixteen stored bytes of a UUID from one cell per row.
    ///
    /// The cell is whatever spelling the source carried - the hyphenated text, the
    /// bare hex, or the sixteen bytes themselves - and the one UUID rule reads all
    /// three, so the reading is stated once for every source layout.
    fn uuid_storage<'a>(
        field: &Field,
        rows: usize,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
        cell: impl Fn(usize) -> Option<&'a [u8]>,
    ) -> Result<ArrayRef> {
        budget.add_array(field.dtype(), rows)?;
        let mut bytes = vec![0_u8; rows * 16];
        let mut validity = BooleanBufferBuilder::new(rows);
        for index in 0..rows {
            let present = is_exposed(exposure, index).then(|| cell(index)).flatten();
            if let Some(value) = present {
                bytes[index * 16..][..16].copy_from_slice(&uuid_cell(field, index, value)?);
            }
            validity.append(present.is_some());
        }
        let nulls = arrow_buffer::NullBuffer::new(validity.finish());
        Ok(Arc::new(FixedSizeBinaryArray::try_new(
            16,
            arrow_buffer::Buffer::from(bytes),
            (nulls.null_count() != 0).then_some(nulls),
        )?))
    }

    /// Validates one cell as a UUID, naming the field and the row beside the rule.
    fn uuid_cell(field: &Field, index: usize, bytes: &[u8]) -> Result<[u8; 16]> {
        uuid_parse(bytes).map_err(|error| {
            let reason = match error {
                crate::Error::InvalidRecord { reason, .. } => reason.to_string(),
                other => other.to_string(),
            };
            Error::IncompatibleSchema(format!("column {:?} row {index}: {reason}", field.name()))
        })
    }
}

// ------------------------------------------------------------------------
// The UUID: one 128-bit universally unique identifier.
//
// The value contract, stated once: a value is exactly sixteen bytes, which
// is what storage holds (Arrow `FixedSizeBinary(16)` under the canonical
// `arrow.uuid` extension). Every string rendering is the 36-character
// lowercase hyphenated form RFC 9562 spells, so storage reads back as the
// identifier that went in. The canonical value is [`Scalar::Uuid`]; text in
// hyphenated or bare-hex form and [`Scalar::Bytes`] of sixteen bytes are
// accepted on the way in and canonicalize to that exact leaf.
//
// A UUID is the ASCII widths' sibling: one fixed-width value whose integer
// is its own storage bytes read big-endian, so it is the same integer in
// every process and is what a stable hash hashes. It is a `u128` rather than
// an `i128` because every one of the sixteen bytes carries identity, and the
// top bit of a version-4 identifier is set as often as not.
// ------------------------------------------------------------------------

/// The canonical Arrow extension name of the UUID type.
///
/// The storage is `FixedSizeBinary(16)` and the extension metadata is the
/// empty string: the width says everything the type carries.
pub(crate) const UUID_EXTENSION_NAME: &str = "arrow.uuid";

/// The number of bytes one identifier is.
pub(crate) const UUID_BYTES: usize = 16;

/// Where the canonical rendering puts its hyphens, in nibbles.
const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];

/// The length of the canonical rendering: 32 digits and four hyphens.
pub(crate) const UUID_TEXT_LEN: usize = UUID_BYTES * 2 + GROUPS.len() - 1;

impl DataType {
    /// Creates the UUID type.
    ///
    /// It takes no parameters: an identifier is 128 bits and nothing else, so
    /// there is no width to select and no vocabulary to register over it.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert_eq!(DataType::uuid(), DataType::Uuid);
    /// assert_eq!(DataType::uuid().to_string(), "uuid");
    /// ```
    #[must_use]
    pub const fn uuid() -> Self {
        Self::Uuid
    }

    /// The 128-bit integer one UUID value is: its storage bytes, big-endian.
    ///
    /// The packed integer is the identifier, not a code for it, so it is the
    /// same integer in every process and orders exactly as the bytes do. It is
    /// unsigned because all sixteen bytes are identity.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let text = "01912d68-783e-7c9a-b1f2-0123456789ab";
    /// let packed = DataType::Uuid.uuid_packed(text.as_bytes())?;
    /// assert_eq!(packed, 0x0191_2d68_783e_7c9a_b1f2_0123_4567_89ab);
    /// assert_eq!(DataType::Uuid.uuid_value(packed)?, text);
    ///
    /// // The bare-hex spelling and upper case are the same identifier.
    /// assert_eq!(DataType::Uuid.uuid_packed(b"01912D68783E7C9AB1F20123456789AB")?, packed);
    /// // So are the sixteen bytes storage holds.
    /// assert_eq!(DataType::Uuid.uuid_packed(&packed.to_be_bytes())?, packed);
    ///
    /// assert!(DataType::Uuid.uuid_packed(b"not-a-uuid").is_err());
    /// assert!(DataType::utf8().uuid_packed(text.as_bytes()).is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the type when this is not `Uuid`, and one
    /// naming the accepted spellings when `value` is not an identifier.
    pub fn uuid_packed(&self, value: &[u8]) -> Result<u128> {
        self.ensure_uuid()?;
        Ok(u128::from_be_bytes(uuid_parse(value)?))
    }

    /// The UUID one packed integer names, in the canonical spelling.
    ///
    /// # Errors
    ///
    /// Returns an error naming the type when this is not `Uuid`.
    pub fn uuid_value(&self, packed: u128) -> Result<SmolStr> {
        self.ensure_uuid()?;
        Ok(uuid_text(&packed.to_be_bytes()))
    }

    fn ensure_uuid(&self) -> Result<()> {
        if matches!(self, Self::Uuid) {
            Ok(())
        } else {
            Err(Error::InvalidDataType {
                kind: "uuid",
                reason: crate::text::expected_got(
                    format_args!("the uuid datatype"),
                    format_args!("{self}"),
                ),
            })
        }
    }
}

/// The bytes a UUID value carries, in either accepted spelling.
pub(crate) fn uuid_bytes(value: &Scalar) -> Option<&[u8]> {
    match value {
        Scalar::String(text) => Some(text.as_str().as_bytes()),
        Scalar::Bytes(bytes) => Some(bytes.as_bytes()),
        _ => None,
    }
}

/// Validates bytes as one identifier and answers its sixteen storage bytes.
///
/// The one validator every arm calls: field validation and canonicalization,
/// Arrow ingest, and casts all answer the same sixteen bytes or the same
/// refusal naming the accepted spellings. Sixteen bytes are storage; 32 or 36
/// bytes are the two text spellings, in either case.
///
/// # Errors
///
/// Returns an error naming the accepted spellings when the bytes are neither.
pub(crate) fn uuid_parse(value: &[u8]) -> Result<[u8; UUID_BYTES]> {
    if let Ok(stored) = <[u8; UUID_BYTES]>::try_from(value) {
        return Ok(stored);
    }
    let mut digits = [0_u8; UUID_BYTES * 2];
    let mut written = 0;
    let mut group = 0;
    let mut in_group = 0;
    let hyphenated = value.len() == 36;
    for (position, byte) in value.iter().enumerate() {
        if *byte == b'-' && hyphenated {
            if group >= GROUPS.len() - 1 || in_group != GROUPS[group] {
                return Err(uuid_refusal(
                    value,
                    format_smolstr!("a hyphen at {position}"),
                ));
            }
            group += 1;
            in_group = 0;
            continue;
        }
        let Some(nibble) = hex_nibble(*byte) else {
            return Err(uuid_refusal(
                value,
                format_smolstr!("a non-hexadecimal byte at {position}"),
            ));
        };
        if written == digits.len() {
            return Err(uuid_refusal(
                value,
                SmolStr::new_static("more than 32 digits"),
            ));
        }
        digits[written] = nibble;
        written += 1;
        in_group += 1;
    }
    if written != digits.len() || (hyphenated && (group, in_group) != (GROUPS.len() - 1, 12)) {
        return Err(uuid_refusal(value, format_smolstr!("{written} digits")));
    }
    let mut stored = [0_u8; UUID_BYTES];
    for (index, byte) in stored.iter_mut().enumerate() {
        *byte = digits[index * 2] << 4 | digits[index * 2 + 1];
    }
    Ok(stored)
}

/// The canonical 36-character lowercase rendering of one identifier.
///
/// The rendering is wider than a compact string holds inline, so the one
/// allocation is the returned value's own: [`uuid_rendered`] writes the
/// characters into the caller's stack slot and this copies them in once.
pub(crate) fn uuid_text(stored: &[u8; UUID_BYTES]) -> SmolStr {
    let mut slot = [0_u8; UUID_TEXT_LEN];
    SmolStr::new(uuid_rendered(stored, &mut slot))
}

/// Writes the canonical rendering into `slot` and borrows it back.
///
/// The one renderer: every spelling of an identifier this crate emits - a
/// value's `Display`, a scalar's text, an Arrow column's cells - goes through
/// it, so no arm allocates a `String` it is about to copy out of again.
pub(crate) fn uuid_rendered<'a>(
    stored: &[u8; UUID_BYTES],
    slot: &'a mut [u8; UUID_TEXT_LEN],
) -> &'a str {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut written = 0;
    let mut digit = 0;
    for (index, width) in GROUPS.iter().enumerate() {
        if index > 0 {
            slot[written] = b'-';
            written += 1;
        }
        for _ in 0..*width {
            let byte = stored[digit / 2];
            let nibble = if digit % 2 == 0 {
                byte >> 4
            } else {
                byte & 0x0F
            };
            slot[written] = HEX[usize::from(nibble)];
            written += 1;
            digit += 1;
        }
    }
    // Every byte written is one of `HEX` or a hyphen, so the slot is ASCII.
    std::str::from_utf8(slot).unwrap_or_default()
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn uuid_refusal(value: &[u8], actual: SmolStr) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: crate::text::expected_got(
            format_args!(
                "a UUID as sixteen bytes, 32 hexadecimal digits, or the 36-character \
                 hyphenated spelling"
            ),
            format_args!("{actual} in {} bytes", value.len()),
        ),
    }
}

// ------------------------------------------------------------------------
// UUID field markers.
// ------------------------------------------------------------------------

define_field_types!(UuidType, Uuid);


// ------------------------------------------------------------------------
// UUID values and the typed scalar alias.
// ------------------------------------------------------------------------

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
    /// writer that wants a `&str` asks for; [`ToString::to_string`] is the
    /// same characters when an owned one is what the caller needs.
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

impl Value for Uuid {
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
