//! The byte value: the payload, held compactly, beside the layout its column
//! stores it under.
//!
//! [`Bytes`] is the one representation, and it is the crate's compact byte
//! string: up to [`INLINE_BYTES`] bytes live inside the value with no heap
//! behind them, a longer payload is one shared `Arc<[u8]>` that clones by
//! reference count, and a `&'static [u8]` costs nothing at all. Equality,
//! order and hashing read the payload alone - a value is one value whichever
//! column holds it - and the parameters ride beside it. A maximum is the
//! column's rule and never the value's: a value read out of `binary(32)` is
//! a `binary`, exactly as an integer read out of a bounded column is an
//! integer.

use std::borrow::{Borrow, Cow};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use smol_str::{SmolStr, format_smolstr};

use super::{BytesLayout, BytesParameters};
use crate::types::Scalar;
use crate::{DataType, DataTypeId, DataTypeKind, Error, Result, ScalarFamily, ScalarValue};

/// How many bytes a [`Bytes`] holds without reaching the heap.
///
/// Thirty: the value has forty bytes to spend beside its parameters, and a
/// digest, a key, a UUID's sixteen bytes or a WKB point all fit under it and
/// never allocate.
pub const INLINE_BYTES: usize = 30;

/// One byte value: its payload and the parameters it is stored under.
///
/// ```
/// use yggdryl::types::{Bytes, BytesLayout, BytesParameters, INLINE_BYTES};
/// use yggdryl::{DataType, Scalar};
///
/// # fn main() -> yggdryl::Result<()> {
/// // A short payload lives inside the value; a longer one is one shared handle.
/// let short = Bytes::new([1_u8, 2, 3]);
/// assert!(short.is_inline());
/// let long = Bytes::new(vec![0_u8; INLINE_BYTES + 1]);
/// assert!(!long.is_inline());
///
/// // A value is one value whichever layout it is stored under.
/// let large = BytesParameters::new(BytesLayout::LargeBinary);
/// let restated = short.clone().try_with_parameters(large)?;
/// assert_eq!(restated, short);
/// assert_eq!(restated.dtype()?, DataType::large_binary());
///
/// // It is the crate's byte string, so it is the `Scalar` bytes too.
/// assert_eq!(Scalar::from(vec![1_u8, 2, 3]), Scalar::Bytes(short));
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct Bytes {
    repr: Repr,
    parameters: BytesParameters,
}

/// Where the payload is.
#[derive(Clone)]
enum Repr {
    /// The payload itself, copied into the value; bytes past `len` are zero.
    Inline { len: u8, bytes: [u8; INLINE_BYTES] },
    /// A payload the binary already holds.
    Static(&'static [u8]),
    /// A payload longer than the inline buffer, shared by reference count.
    Shared(Arc<[u8]>),
}

const _: () = assert!(std::mem::size_of::<Repr>() == 32);
const _: () = assert!(std::mem::size_of::<Bytes>() == 40);

impl Repr {
    /// Copy a payload into the value where it fits, else share it.
    fn owned(payload: &[u8]) -> Self {
        match Self::inline(payload) {
            Some(inline) => inline,
            None => Self::Shared(Arc::from(payload)),
        }
    }

    /// Copy a payload into the value, when it fits.
    fn inline(payload: &[u8]) -> Option<Self> {
        if payload.len() > INLINE_BYTES {
            return None;
        }
        let mut bytes = [0_u8; INLINE_BYTES];
        bytes[..payload.len()].copy_from_slice(payload);
        Some(Self::Inline {
            // The length fits the buffer, and the buffer fits a byte.
            len: payload.len() as u8,
            bytes,
        })
    }

    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Inline { len, bytes } => &bytes[..usize::from(*len)],
            Self::Static(bytes) => bytes,
            Self::Shared(bytes) => bytes,
        }
    }
}

impl Bytes {
    /// A payload the binary already holds, under the default parameters.
    ///
    /// Costs nothing: no copy, no count, so a constant payload is a
    /// constant value.
    #[must_use]
    pub const fn new_static(payload: &'static [u8]) -> Self {
        Self {
            repr: Repr::Static(payload),
            parameters: BytesParameters::new(BytesLayout::Binary),
        }
    }

    /// A byte value under the default parameters: the `binary` layout.
    ///
    /// A payload up to [`INLINE_BYTES`] long is copied into the value and
    /// allocates nothing; a longer one is one shared `Arc<[u8]>`.
    pub fn new(payload: impl AsRef<[u8]>) -> Self {
        Self {
            repr: Repr::owned(payload.as_ref()),
            parameters: BytesParameters::default(),
        }
    }

    /// A byte value adopting a handle a reader already holds.
    ///
    /// A payload that fits inline is copied and the handle dropped; a longer
    /// one is shared as it is, so nothing is copied twice.
    #[must_use]
    pub fn from_shared(payload: Arc<[u8]>) -> Self {
        let repr = match Repr::inline(&payload) {
            Some(inline) => inline,
            None => Repr::Shared(payload),
        };
        Self {
            repr,
            parameters: BytesParameters::default(),
        }
    }

    /// The payload a column's own storage holds, under the parameters it
    /// declares, checked when it was written and not again here.
    pub(crate) fn from_storage(payload: &[u8], parameters: BytesParameters) -> Self {
        Self {
            repr: Repr::owned(payload),
            parameters: parameters.without_max(),
        }
    }

    /// Restate the same payload under other parameters.
    ///
    /// The payload does not change and a shared handle is shared, not copied;
    /// what changes is the datatype [`Self::dtype`] declares. The bound is
    /// checked and, on a variable layout, not carried: it is the column's
    /// rule, and the value answers the layout alone. Bytes are never padded,
    /// so a fixed layout takes exactly its width and nothing shorter.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for the fixed layout with no width,
    /// and [`Error::InvalidRecord`] naming the bound and the payload length
    /// when the payload does not fit it.
    pub fn try_with_parameters(mut self, parameters: BytesParameters) -> Result<Self> {
        parameters.validate()?;
        let held = self.len();
        let refusal = |expected: std::fmt::Arguments<'_>| Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: crate::text::expected_got(expected, format_smolstr!("{held} bytes")),
        };
        if let Some(width) = parameters.fixed() {
            if held != width as usize {
                return Err(refusal(format_args!("exactly {width} bytes")));
            }
        } else if let Some(max) = parameters.max() {
            if held > max as usize {
                return Err(refusal(format_args!("at most {max} bytes")));
            }
        }
        self.parameters = parameters.without_max();
        Ok(self)
    }

    /// Borrow the payload.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        self.repr.as_bytes()
    }

    /// The parameters this value is stored under.
    ///
    /// Never a maximum: that is the column's declaration, and a value read
    /// out of a bounded column answers its layout alone.
    #[must_use]
    pub const fn parameters(&self) -> BytesParameters {
        self.parameters
    }

    /// The layout this value is stored in.
    #[must_use]
    pub const fn layout(&self) -> BytesLayout {
        self.parameters.layout()
    }

    /// The exact storage width, on the fixed layout alone.
    #[must_use]
    pub const fn fixed(&self) -> Option<u32> {
        self.parameters.fixed()
    }

    /// Whether the payload lives inside the value with no heap behind it.
    #[must_use]
    pub const fn is_inline(&self) -> bool {
        matches!(self.repr, Repr::Inline { .. })
    }

    /// Consume this value and return its payload as an owned vector.
    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        match self.repr {
            Repr::Inline { .. } | Repr::Static(_) => self.as_bytes().to_vec(),
            Repr::Shared(bytes) => bytes.to_vec(),
        }
    }

    /// Consume this value and return its payload as one shared handle.
    ///
    /// A shared value hands over its handle; the two other representations
    /// allocate one, since neither has a handle to give.
    #[must_use]
    pub fn into_shared(self) -> Arc<[u8]> {
        match self.repr {
            Repr::Shared(bytes) => bytes,
            Repr::Inline { .. } | Repr::Static(_) => Arc::from(self.as_bytes()),
        }
    }

    /// The datatype this value materializes into.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] only for parameters a constructor
    /// would have refused, which no door here builds.
    pub fn dtype(&self) -> Result<DataType> {
        DataType::bytes(self.parameters)
    }
}

impl Default for Bytes {
    fn default() -> Self {
        Self::new_static(&[])
    }
}

impl Deref for Bytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// Sound because [`Eq`], [`Ord`] and [`Hash`] read the payload alone, as a
/// slice's do; folding the parameters into any of them would break every
/// keyed lookup by slice.
impl Borrow<[u8]> for Bytes {
    fn borrow(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl fmt::Display for Bytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.as_bytes() {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Bytes {
    /// The payload in hex, and the parameters when they are not the default,
    /// so two values that compare equal but declare different columns print
    /// apart in a failing assertion.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "0x{self}")?;
        if self.parameters != BytesParameters::default() {
            write!(formatter, " as {}", self.parameters)?;
        }
        Ok(())
    }
}

impl PartialEq for Bytes {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for Bytes {}

impl PartialEq<[u8]> for Bytes {
    fn eq(&self, other: &[u8]) -> bool {
        self.as_bytes() == other
    }
}

impl PartialEq<&[u8]> for Bytes {
    fn eq(&self, other: &&[u8]) -> bool {
        self.as_bytes() == *other
    }
}

impl<const N: usize> PartialEq<[u8; N]> for Bytes {
    fn eq(&self, other: &[u8; N]) -> bool {
        self.as_bytes() == other
    }
}

impl<const N: usize> PartialEq<&[u8; N]> for Bytes {
    fn eq(&self, other: &&[u8; N]) -> bool {
        self.as_bytes() == *other
    }
}

impl PartialEq<Vec<u8>> for Bytes {
    fn eq(&self, other: &Vec<u8>) -> bool {
        self.as_bytes() == other.as_slice()
    }
}

impl PartialEq<Bytes> for [u8] {
    fn eq(&self, other: &Bytes) -> bool {
        self == other.as_bytes()
    }
}

impl PartialEq<Bytes> for Vec<u8> {
    fn eq(&self, other: &Bytes) -> bool {
        self.as_slice() == other.as_bytes()
    }
}

impl PartialOrd for Bytes {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Bytes {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

impl Hash for Bytes {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

impl From<&[u8]> for Bytes {
    fn from(value: &[u8]) -> Self {
        Self::new(value)
    }
}

impl<const N: usize> From<&[u8; N]> for Bytes {
    fn from(value: &[u8; N]) -> Self {
        Self::new(value)
    }
}

impl<const N: usize> From<[u8; N]> for Bytes {
    fn from(value: [u8; N]) -> Self {
        Self::new(value)
    }
}

impl From<Vec<u8>> for Bytes {
    fn from(value: Vec<u8>) -> Self {
        match Repr::inline(&value) {
            Some(repr) => Self {
                repr,
                parameters: BytesParameters::default(),
            },
            None => Self::from_shared(Arc::from(value)),
        }
    }
}

impl From<Box<[u8]>> for Bytes {
    fn from(value: Box<[u8]>) -> Self {
        match Repr::inline(&value) {
            Some(repr) => Self {
                repr,
                parameters: BytesParameters::default(),
            },
            None => Self::from_shared(Arc::from(value)),
        }
    }
}

impl From<Arc<[u8]>> for Bytes {
    fn from(value: Arc<[u8]>) -> Self {
        Self::from_shared(value)
    }
}

impl From<Cow<'_, [u8]>> for Bytes {
    fn from(value: Cow<'_, [u8]>) -> Self {
        match value {
            Cow::Borrowed(bytes) => Self::new(bytes),
            Cow::Owned(bytes) => Self::from(bytes),
        }
    }
}

impl From<Bytes> for Vec<u8> {
    fn from(value: Bytes) -> Self {
        value.into_vec()
    }
}

impl From<Bytes> for Arc<[u8]> {
    fn from(value: Bytes) -> Self {
        value.into_shared()
    }
}

impl FromIterator<u8> for Bytes {
    fn from_iter<I: IntoIterator<Item = u8>>(iter: I) -> Self {
        Self::from(iter.into_iter().collect::<Vec<u8>>())
    }
}

/// The serde representation of a byte value that declares more than its
/// payload.
///
/// The ordinary value - the `binary` layout - serializes its payload and
/// nothing else, exactly as it always has; a layout or a fixed width is what
/// makes a value carry more than that.
#[derive(Deserialize, Serialize)]
struct Declared<'a> {
    layout: BytesLayout,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fixed: Option<u32>,
    bytes: Cow<'a, [u8]>,
}

impl Serialize for Bytes {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        if self.parameters == BytesParameters::default() {
            return serializer.collect_seq(self.as_bytes());
        }
        Declared {
            layout: self.layout(),
            fixed: self.fixed(),
            bytes: Cow::Borrowed(self.as_bytes()),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Representation<'a> {
            Plain(Cow<'a, [u8]>),
            Declared(Declared<'a>),
        }

        match Representation::deserialize(deserializer)? {
            Representation::Plain(bytes) => Ok(Self::from(bytes)),
            Representation::Declared(declared) => {
                let mut parameters = BytesParameters::new(declared.layout);
                if let Some(width) = declared.fixed {
                    parameters = parameters
                        .try_with_bound(width)
                        .map_err(serde::de::Error::custom)?;
                }
                Self::from(declared.bytes)
                    .try_with_parameters(parameters)
                    .map_err(serde::de::Error::custom)
            }
        }
    }
}

impl ScalarValue for Bytes {
    type Family = Bytes;

    /// The family's default layout; [`ScalarFamily::id`] answers the exact
    /// layout a value is stored in.
    const ID: DataTypeId = DataTypeId::Binary;
    const KIND: DataTypeKind = DataTypeKind::Bytes;

    fn dtype(&self) -> Result<DataType> {
        Self::dtype(self)
    }

    fn into_family(self) -> Self::Family {
        self
    }

    fn from_family(family: &Self::Family) -> Option<&Self> {
        Some(family)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Bytes(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Bytes(value) => Some(value),
            _ => None,
        }
    }
}

impl ScalarFamily for Bytes {
    const KIND: DataTypeKind = DataTypeKind::Bytes;

    fn id(&self) -> DataTypeId {
        self.layout().id()
    }

    fn dtype(&self) -> Result<DataType> {
        Self::dtype(self)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Bytes(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Bytes(value) => Some(value),
            _ => None,
        }
    }
}

impl From<Bytes> for Scalar {
    fn from(value: Bytes) -> Self {
        Self::Bytes(value)
    }
}

impl From<Vec<u8>> for Scalar {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(Bytes::from(value))
    }
}

impl From<&[u8]> for Scalar {
    fn from(value: &[u8]) -> Self {
        Self::Bytes(Bytes::new(value))
    }
}

impl<const N: usize> From<&[u8; N]> for Scalar {
    fn from(value: &[u8; N]) -> Self {
        Self::Bytes(Bytes::new(value))
    }
}

impl From<Arc<[u8]>> for Scalar {
    fn from(value: Arc<[u8]>) -> Self {
        Self::Bytes(Bytes::from_shared(value))
    }
}

/// The byte payload a value spells, shared rather than copied.
///
/// A byte column stores one payload per row and several kinds already hold
/// one: a byte value is cloned, a geospatial value shares its handle, and a
/// UUID is its sixteen canonical bytes. A code spells its padded slot,
/// because that is the payload the fixed column stores - the value keeps
/// only the trimmed text - and every other value that spells text spells
/// that text's bytes.
pub(crate) fn bytes_from_value(value: &Scalar) -> Option<Bytes> {
    match value {
        Scalar::Bytes(bytes) => Some(bytes.clone()),
        Scalar::Geospatial(geospatial) => {
            Some(Bytes::from_shared(Arc::clone(geospatial.storage())))
        }
        Scalar::Uuid(uuid) => Some(Bytes::new(uuid.into_bytes())),
        Scalar::Code(code) => {
            let mut padded = vec![0_u8; code.width()];
            crate::types::ascii_padded(&mut padded, code.as_str());
            Some(Bytes::from(padded))
        }
        _ => value.as_str().map(|text| Bytes::new(text.as_bytes())),
    }
}

#[cfg(test)]
mod tests {
    use super::{Bytes, INLINE_BYTES};
    use crate::types::{BytesLayout, BytesParameters};
    use crate::{DataType, Scalar};

    #[test]
    fn a_short_payload_is_inline_and_a_long_one_is_shared() {
        let short = Bytes::new(vec![7_u8; INLINE_BYTES]);
        assert!(short.is_inline());
        let long = Bytes::new(vec![7_u8; INLINE_BYTES + 1]);
        assert!(!long.is_inline());
        assert_eq!(long.len(), INLINE_BYTES + 1);
        assert!(!Bytes::new_static(b"held").is_inline());
        assert_eq!(Bytes::default(), b"");
        assert_eq!(std::mem::size_of::<Bytes>(), 40);
    }

    #[test]
    fn equality_order_and_hash_read_the_payload_only() {
        use std::collections::HashSet;

        let plain = Bytes::new(b"abc");
        let large = Bytes::new(b"abc")
            .try_with_parameters(BytesParameters::new(BytesLayout::LargeBinary))
            .unwrap();
        assert_eq!(plain, large);
        assert_eq!(HashSet::from([plain.clone(), large.clone()]).len(), 1);
        assert_ne!(plain.parameters(), large.parameters());
        assert!(Bytes::new(b"b") > Bytes::new(b"a"));
        assert_eq!(format!("{large:?}"), "0x616263 as large_binary");
        assert_eq!(format!("{plain:?}"), "0x616263");
    }

    #[test]
    fn restating_checks_but_never_carries_a_maximum() {
        let bounded = BytesParameters::new(BytesLayout::Binary)
            .try_with_bound(4)
            .unwrap();
        let value = Bytes::new(b"abcd").try_with_parameters(bounded).unwrap();
        assert_eq!(value.parameters(), BytesParameters::default());
        assert_eq!(value.dtype().unwrap(), DataType::binary());
        let refused = Bytes::new(b"abcde")
            .try_with_parameters(bounded)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("at most 4 bytes"), "{refused}");
    }

    #[test]
    fn a_fixed_layout_takes_exactly_its_width() {
        let fixed = BytesParameters::new(BytesLayout::FixedSizeBinary)
            .try_with_bound(4)
            .unwrap();
        let value = Bytes::new(b"abcd").try_with_parameters(fixed).unwrap();
        assert_eq!(value.fixed(), Some(4));
        assert_eq!(
            value.dtype().unwrap(),
            DataType::fixed_size_binary(4).unwrap()
        );
        assert!(Bytes::new(b"abc").try_with_parameters(fixed).is_err());
        assert!(Bytes::new(b"abcde").try_with_parameters(fixed).is_err());
        assert!(
            Bytes::new(b"x")
                .try_with_parameters(BytesParameters::new(BytesLayout::FixedSizeBinary))
                .is_err()
        );
    }

    #[test]
    fn serde_writes_the_payload_alone_unless_the_value_declares_more() {
        let plain = Bytes::new(b"ab");
        assert_eq!(serde_json::to_string(&plain).unwrap(), "[97,98]");
        assert_eq!(serde_json::from_str::<Bytes>("[97,98]").unwrap(), plain);
        let fixed = Bytes::new(b"ab")
            .try_with_parameters(
                BytesParameters::new(BytesLayout::FixedSizeBinary)
                    .try_with_bound(2)
                    .unwrap(),
            )
            .unwrap();
        let document = serde_json::to_string(&fixed).unwrap();
        assert_eq!(
            document,
            r#"{"layout":"fixed_size_binary","fixed":2,"bytes":[97,98]}"#
        );
        let back = serde_json::from_str::<Bytes>(&document).unwrap();
        assert_eq!(back.parameters(), fixed.parameters());
        assert_eq!(back, fixed);
    }

    #[test]
    fn a_byte_scalar_names_its_own_datatype() {
        assert_eq!(
            Scalar::from(vec![1_u8]).dtype().unwrap(),
            DataType::binary()
        );
        let view = Bytes::new(b"x")
            .try_with_parameters(BytesParameters::new(BytesLayout::BinaryView))
            .unwrap();
        assert_eq!(
            Scalar::Bytes(view).dtype().unwrap(),
            DataType::binary_view()
        );
    }
}
