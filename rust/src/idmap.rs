//! Identifiers keyed by name: the accounts, users and alternate ids a market
//! states beside its instrument codes.

use std::fmt;
use std::hash::Hasher;

use smallvec::{Array, SmallVec};
use smol_str::{SmolStr, format_smolstr};

use crate::code::is_null_like;
use crate::{DataType, Error, Field, Result, Scalar, StructType};

/// The most bytes a key may be once upper-cased.
const KEY_WIDTH: usize = 32;

/// The most bytes a value may be once trimmed.
const VALUE_WIDTH: usize = 64;

/// One entry at its widest: the key length, the key and the value.
const ENTRY_WIDTH: usize = 1 + KEY_WIDTH + VALUE_WIDTH;

/// A small sorted map of ASCII identifiers under upper-cased ASCII keys.
///
/// Each entry is one buffer holding the key's length, the key and the value,
/// inline up to 23 bytes, and the two inline slots hold the one or two ids a
/// message usually states without touching the heap. Keys fold to upper case
/// on every door - a read, a write, a removal - so `account`, `Account` and
/// `ACCOUNT` are one key; a value is trimmed, and one that states nothing
/// (`null`, `none`, `n/a`, `[n/a]` or empty) adds nothing. Its Arrow shape
/// is a sorted map of required text keys to required text values.
///
/// ```
/// use yggdryl::IdMap;
///
/// let mut ids = IdMap::new();
/// assert!(ids.insert("account", "ACC-1").unwrap());
/// assert!(!ids.insert("ACCOUNT", "ACC-2").unwrap(), "fill only");
/// assert!(!ids.insert("user", "null").unwrap(), "a null-like adds nothing");
/// assert_eq!(ids.get("Account"), Some("ACC-1"));
/// assert_eq!(ids.to_string(), "{ACCOUNT=ACC-1}");
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct IdMap(SmallVec<[SmolStr; 2]>);

/// The key an entry holds.
fn key_of(entry: &str) -> &str {
    let width = usize::from(entry.as_bytes()[0]);
    &entry[1..=width]
}

/// The value an entry holds.
fn value_of(entry: &str) -> &str {
    let width = usize::from(entry.as_bytes()[0]);
    &entry[1 + width..]
}

/// The key and the value an entry holds.
fn pair_of(entry: &SmolStr) -> (&str, &str) {
    (key_of(entry), value_of(entry))
}

/// The rule a key or a value broke: what a read discards without
/// allocating, and what a write renders into its refusal.
enum Fault {
    Empty,
    NonAscii(usize, u8),
    Control(usize, u8),
    Width(usize),
}

impl fmt::Display for Fault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("\"\""),
            Self::NonAscii(position, byte) => {
                write!(formatter, "a non-ASCII byte 0x{byte:02X} at {position}")
            }
            Self::Control(position, byte) => {
                write!(formatter, "a control byte 0x{byte:02X} at {position}")
            }
            Self::Width(width) => write!(formatter, "{width} bytes"),
        }
    }
}

/// The refusal one key or value earns, located on the key as stated.
fn refusal(key: &str, what: &str, width: usize, fault: &Fault) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new(key),
        reason: crate::text::expected_got(
            format_args!("{what} of 1 to {width} ASCII bytes"),
            fault,
        ),
    }
}

/// `text` trimmed, held to printable ASCII of 1 to `width` bytes.
fn checked(text: &str, width: usize) -> std::result::Result<&str, Fault> {
    let trimmed = text.trim_matches(|c: char| c.is_ascii_whitespace());
    if trimmed.is_empty() {
        return Err(Fault::Empty);
    }
    for (position, byte) in trimmed.bytes().enumerate() {
        if !byte.is_ascii() {
            return Err(Fault::NonAscii(position, byte));
        }
        if byte.is_ascii_control() {
            return Err(Fault::Control(position, byte));
        }
    }
    if trimmed.len() > width {
        return Err(Fault::Width(trimmed.len()));
    }
    Ok(trimmed)
}

/// `key` trimmed and upper-cased into `buffer`, or the rule it broke;
/// nothing allocates either way.
fn folded<'buffer>(
    key: &str,
    buffer: &'buffer mut [u8; KEY_WIDTH],
) -> std::result::Result<&'buffer str, Fault> {
    let trimmed = checked(key, KEY_WIDTH)?;
    for (target, byte) in buffer.iter_mut().zip(trimmed.bytes()) {
        *target = byte.to_ascii_uppercase();
    }
    Ok(std::str::from_utf8(&buffer[..trimmed.len()]).expect("validated ASCII"))
}

/// [`folded`], its fault rendered into the refusal a write answers.
fn fold_key<'buffer>(key: &str, buffer: &'buffer mut [u8; KEY_WIDTH]) -> Result<&'buffer str> {
    folded(key, buffer).map_err(|fault| refusal(key, "a key", KEY_WIDTH, &fault))
}

/// `value` trimmed, `None` where it states nothing, or the rule it broke.
fn trimmed_value<'value>(key: &str, value: &'value str) -> Result<Option<&'value str>> {
    if is_null_like(value.trim_matches(|c: char| c.is_ascii_whitespace())) {
        return Ok(None);
    }
    checked(value, VALUE_WIDTH)
        .map(Some)
        .map_err(|fault| refusal(key, "a value", VALUE_WIDTH, &fault))
}

/// One entry buffer for a folded key and a trimmed value: inline up to 23
/// bytes, one allocation past it.
fn entry(key: &str, value: &str) -> SmolStr {
    let mut buffer = [0_u8; ENTRY_WIDTH];
    buffer[0] = key.len() as u8;
    buffer[1..=key.len()].copy_from_slice(key.as_bytes());
    buffer[1 + key.len()..1 + key.len() + value.len()].copy_from_slice(value.as_bytes());
    SmolStr::new(std::str::from_utf8(&buffer[..1 + key.len() + value.len()]).expect("ASCII"))
}

/// Union `other` into the sorted `into`, `into`'s entry standing where both
/// hold a key; whether anything was added.
pub(crate) fn merge_sorted<A: Array>(
    into: &mut SmallVec<A>,
    other: &[A::Item],
    key: impl Fn(&A::Item) -> &str,
) -> bool
where
    A::Item: Clone,
{
    let mut position = 0;
    let mut added = false;
    for candidate in other {
        let wanted = key(candidate);
        while position < into.len() && key(&into[position]) < wanted {
            position += 1;
        }
        if position < into.len() && key(&into[position]) == wanted {
            position += 1;
            continue;
        }
        into.insert(position, candidate.clone());
        position += 1;
        added = true;
    }
    added
}

impl IdMap {
    /// An empty map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many keys the map holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the map holds no key.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Where the folded key stands, or where it would.
    fn position(&self, folded: &str) -> std::result::Result<usize, usize> {
        self.0
            .binary_search_by(|held| key_of(held).as_bytes().cmp(folded.as_bytes()))
    }

    /// The value under `key`, folded; nothing allocates, a key no door
    /// accepts included.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        let mut buffer = [0_u8; KEY_WIDTH];
        let folded = folded(key, &mut buffer).ok()?;
        let position = self.position(folded).ok()?;
        Some(value_of(&self.0[position]))
    }

    /// Whether `key`, folded, is held.
    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Fill `key` with `value`: `true` when the map gained the key, `false`
    /// when it already held it or when `value` states nothing.
    ///
    /// # Errors
    ///
    /// A key or a value that is empty, not ASCII or too wide, located on the
    /// key as stated.
    pub fn insert(&mut self, key: &str, value: &str) -> Result<bool> {
        let mut buffer = [0_u8; KEY_WIDTH];
        let folded = fold_key(key, &mut buffer)?;
        let Some(value) = trimmed_value(key, value)? else {
            return Ok(false);
        };
        let Err(position) = self.position(folded) else {
            return Ok(false);
        };
        self.0.insert(position, entry(folded, value));
        Ok(true)
    }

    /// Set `key` to `value`, replacing what it held: `true` when the map
    /// changed. A value that states nothing changes nothing.
    ///
    /// # Errors
    ///
    /// As [`Self::insert`].
    pub fn set(&mut self, key: &str, value: &str) -> Result<bool> {
        let mut buffer = [0_u8; KEY_WIDTH];
        let folded = fold_key(key, &mut buffer)?;
        let Some(value) = trimmed_value(key, value)? else {
            return Ok(false);
        };
        match self.position(folded) {
            Ok(position) if value_of(&self.0[position]) == value => Ok(false),
            Ok(position) => {
                self.0[position] = entry(folded, value);
                Ok(true)
            }
            Err(position) => {
                self.0.insert(position, entry(folded, value));
                Ok(true)
            }
        }
    }

    /// Remove `key`, folded, answering the value it held.
    pub fn remove(&mut self, key: &str) -> Option<SmolStr> {
        let mut buffer = [0_u8; KEY_WIDTH];
        let folded = folded(key, &mut buffer).ok()?;
        let position = self.position(folded).ok()?;
        let held = self.0.remove(position);
        Some(SmolStr::new(value_of(&held)))
    }

    /// Every key and value, in key order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &str)> + DoubleEndedIterator {
        self.0.iter().map(pair_of)
    }

    /// Take every key of `other` this map lacks, this map's value standing
    /// where both hold a key; whether anything was added.
    pub fn merge(&mut self, other: &Self) -> bool {
        merge_sorted(&mut self.0, &other.0, |held| key_of(held))
    }

    /// The first of `keys` the map holds, as the key held and its value.
    #[must_use]
    pub fn first_of(&self, keys: &[&str]) -> Option<(&str, &str)> {
        keys.iter().find_map(|key| {
            let mut buffer = [0_u8; KEY_WIDTH];
            let folded = folded(key, &mut buffer).ok()?;
            let position = self.position(folded).ok()?;
            Some(pair_of(&self.0[position]))
        })
    }

    /// Remove every key.
    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Feed every entry in key order: the key's length, the key, the value's
    /// length and the value, lengths as `u32` little-endian.
    #[expect(
        dead_code,
        reason = "fed under the map's label by the graph digest once the market traits carry it"
    )]
    pub(crate) fn digest_into(&self, hasher: &mut impl Hasher) {
        for (key, value) in self.iter() {
            hasher.write(&(key.len() as u32).to_le_bytes());
            hasher.write(key.as_bytes());
            hasher.write(&(value.len() as u32).to_le_bytes());
            hasher.write(value.as_bytes());
        }
    }

    /// The Arrow datatype a map lays out: a sorted map of required text
    /// keys to required text values.
    #[must_use]
    pub fn dtype() -> DataType {
        let entries = StructType::from_unique_fields(vec![
            Field::new("key", DataType::utf8(), false),
            Field::new("value", DataType::utf8(), false),
        ]);
        DataType::map(
            Field::new("entries", DataType::Struct(entries), false),
            true,
        )
        .expect("a non-null key-value entries field is a map")
    }

    /// The map as a sorted-map scalar of text keys and values.
    #[must_use]
    pub fn to_scalar(&self) -> Scalar {
        sorted_map_scalar(self.iter())
    }

    /// Read a map back from the scalar [`Self::to_scalar`] answers.
    ///
    /// # Errors
    ///
    /// Anything but a mapping, and a null or non-text key or value, a key
    /// that repeats once folded or one out of order, each located on the
    /// entry.
    pub fn from_scalar(scalar: &Scalar) -> Result<Self> {
        let mut map = Self::new();
        for (index, key, value) in text_entries(scalar)? {
            let mut buffer = [0_u8; KEY_WIDTH];
            let folded =
                fold_key(key, &mut buffer).map_err(|error| located_at(index, "key", error))?;
            match map.0.last().map(|held| key_of(held).cmp(folded)) {
                Some(std::cmp::Ordering::Equal) => {
                    return Err(entry_refusal(index, "key", "a key held once already"));
                }
                Some(std::cmp::Ordering::Greater) => {
                    return Err(entry_refusal(index, "key", "a key out of order"));
                }
                Some(std::cmp::Ordering::Less) | None => {}
            }
            let Some(value) =
                trimmed_value(key, value).map_err(|error| located_at(index, "value", error))?
            else {
                return Err(entry_refusal(index, "value", "a value that states nothing"));
            };
            map.0.push(entry(folded, value));
        }
        Ok(map)
    }
}

/// A sorted-map scalar over text pairs whose keys are already unique.
pub(crate) fn sorted_map_scalar<'entry>(
    entries: impl Iterator<Item = (&'entry str, &'entry str)>,
) -> Scalar {
    let mapping =
        Scalar::from_mapping(entries.map(|(key, value)| (Scalar::from(key), Scalar::from(value))))
            .expect("the keys of a sorted map are unique");
    match mapping {
        Scalar::Map(entries) => Scalar::SortedMap(entries),
        other => other,
    }
}

/// The text pairs a mapping scalar holds, each with its position.
///
/// # Errors
///
/// Anything but a mapping, located at the root; a null or non-text key or
/// value, located on the entry.
pub(crate) fn text_entries(scalar: &Scalar) -> Result<Vec<(usize, &str, &str)>> {
    let Some(entries) = scalar.as_mapping() else {
        return Err(Error::InvalidRecord {
            path: SmolStr::new_static("$"),
            reason: crate::text::expected_got(
                format_args!("a sorted map of text keys and values"),
                scalar.kind(),
            ),
        });
    };
    entries
        .iter()
        .enumerate()
        .map(|(index, (key, value))| {
            let key = key
                .as_str()
                .ok_or_else(|| entry_refusal(index, "key", key.kind()))?;
            let value = value
                .as_str()
                .ok_or_else(|| entry_refusal(index, "value", value.kind()))?;
            Ok((index, key, value))
        })
        .collect()
}

/// `error`, raised by one side of a mapping entry, relocated onto it.
pub(crate) fn located_at(index: usize, side: &str, error: Error) -> Error {
    let reason = match error {
        Error::InvalidDataType { reason, .. } | Error::InvalidRecord { reason, .. } => reason,
        other => format_smolstr!("{other}"),
    };
    Error::InvalidRecord {
        path: format_smolstr!("$[{index}].{side}"),
        reason,
    }
}

/// The refusal one entry of a mapping scalar earns.
pub(crate) fn entry_refusal(index: usize, side: &str, actual: impl fmt::Display) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$[{index}].{side}"),
        reason: crate::text::expected_got(format_args!("text under a unique sorted key"), actual),
    }
}

impl fmt::Display for IdMap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("{")?;
        for (index, (key, value)) in self.iter().enumerate() {
            if index > 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{key}={value}")?;
        }
        formatter.write_str("}")
    }
}
