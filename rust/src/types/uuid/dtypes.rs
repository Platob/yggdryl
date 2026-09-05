//! The UUID: one 128-bit universally unique identifier.
//!
//! The value contract, stated once: a value is exactly sixteen bytes, which
//! is what storage holds (Arrow `FixedSizeBinary(16)` under the canonical
//! `arrow.uuid` extension). Every string rendering is the 36-character
//! lowercase hyphenated form RFC 9562 spells, so storage reads back as the
//! identifier that went in. The canonical value is [`Scalar::Uuid`]; text in
//! hyphenated or bare-hex form and [`Scalar::Bytes`] of sixteen bytes are
//! accepted on the way in and canonicalize to that exact leaf.
//!
//! A UUID is the ASCII widths' sibling: one fixed-width value whose integer
//! is its own storage bytes read big-endian, so it is the same integer in
//! every process and is what a stable hash hashes. It is a `u128` rather than
//! an `i128` because every one of the sixteen bytes carries identity, and the
//! top bit of a version-4 identifier is set as often as not.

use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Result, Scalar};

/// The canonical Arrow extension name of the UUID type.
///
/// The storage is `FixedSizeBinary(16)` and the extension metadata is the
/// empty string: the width says everything the type carries.
pub(crate) const UUID_EXTENSION_NAME: &str = "arrow.uuid";

/// The number of bytes one identifier is.
const UUID_BYTES: usize = 16;

/// Where the canonical rendering puts its hyphens, in nibbles.
const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];

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
    /// assert!(DataType::Utf8.uuid_packed(text.as_bytes()).is_err());
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
        Scalar::Text(text) => Some(text.as_str().as_bytes()),
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
pub(crate) fn uuid_text(stored: &[u8; UUID_BYTES]) -> SmolStr {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(36);
    let mut digit = 0;
    for (index, width) in GROUPS.iter().enumerate() {
        if index > 0 {
            text.push('-');
        }
        for _ in 0..*width {
            let byte = stored[digit / 2];
            let nibble = if digit % 2 == 0 {
                byte >> 4
            } else {
                byte & 0x0F
            };
            text.push(char::from(HEX[usize::from(nibble)]));
            digit += 1;
        }
    }
    SmolStr::new(text)
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
