//! Security identifiers: which source names an instrument, the code it gives
//! it, the sorted set a market states, and the associations one ordered
//! lifecycle learns between them.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::hash::Hasher;
use std::mem::{align_of, size_of};
use std::ops::Deref;

use smallvec::SmallVec;
use smol_str::{SmolStr, format_smolstr};

use crate::bloomberg_code::BLOOMBERG_WIDTH;
use crate::code::{folded_spelling, is_null_like};
use crate::graph::{Element, Market};
use crate::idmap::{
    IdMap, entry_refusal, located_at, merge_sorted, sorted_map_scalar, text_entries,
};
use crate::{CfiCode, CusipCode, DataType, Error, FIGICode, IsinCode, Result, Scalar, SedolCode};

/// The most bytes a source key may be once upper-cased.
const KEY_WIDTH: usize = 32;

/// The tag byte that opens an unknown key's buffer: the length byte and the
/// key follow it. Known keys take `0x01..=0x7E`, so every byte stays ASCII.
const UNKNOWN_TAG: u8 = 0x7F;

// ---------------------------------------------------------------------------
// SecType: the source that names an instrument.
// ---------------------------------------------------------------------------

/// The source a security identifier comes from: `ISIN`, `CUSIP`, `BLOOMBERG`
/// or any other upper-cased ASCII key of at most 32 bytes.
///
/// The thirty-three sources FIX's `SecurityIDSource(22)` code set names are
/// [`Self::KNOWN`], reached by their one-character code, their key in any
/// case or the code set's name folded; any other spelling is kept as the
/// upper-cased key it states. `TICKER` is never a source: a ticker is the
/// name a person knows an instrument by, and lives on `set_ticker`.
///
/// ```
/// use yggdryl::SecType;
///
/// assert_eq!(SecType::read("4").unwrap().as_str(), "ISIN");
/// assert_eq!(SecType::read("isin_number").unwrap().as_str(), "ISIN");
/// assert_eq!(SecType::read("bbgsymb").unwrap().fix_source(), Some('A'));
/// assert!(!SecType::read("house").unwrap().is_known());
/// assert!(SecType::read("ticker").is_err());
/// ```
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SecType(SmolStr);

/// The code set's names, folded, and the key each names.
static NAMES: &[(&str, &str)] = &[
    ("bbgsymb", "BLOOMBERG"),
    ("belgian", "BELGIAN"),
    ("bloombergsymbol", "BLOOMBERG"),
    ("cftccommoditycode", "CFTC"),
    ("clearinghouse", "CLEARINGHOUSE"),
    ("common", "COMMON"),
    ("consolidatedtapeassociation", "CTA"),
    ("cusip", "CUSIP"),
    ("digitaltokenidentifier", "DTI"),
    ("dutch", "DUTCH"),
    ("exchangesymbol", "EXCHSYMB"),
    ("fidessainstrumentmnemonic", "FIM"),
    ("figi", "FIGI"),
    ("financialinstrumentglobalidentifier", "FIGI"),
    ("indexname", "INDEX"),
    ("isdacommodityreferenceprice", "ISDACOMMODITY"),
    ("isdafpmlspecification", "FPMLSPEC"),
    ("isdafpmlurl", "FPMLURL"),
    ("isin", "ISIN"),
    ("isinnumber", "ISIN"),
    ("isocountrycode", "ISOCTRY"),
    ("isocurrencycode", "ISOCCY"),
    ("legalentityidentifier", "LEI"),
    ("lei", "LEI"),
    ("letterofcredit", "LOC"),
    ("marketplaceassignedidentifier", "MKTASSIGNED"),
    ("markitredentityclip", "REDENTITY"),
    ("markitredpairclip", "REDPAIR"),
    ("optionpricereportingauthority", "OPRA"),
    ("quik", "QUIK"),
    ("ric", "RIC"),
    ("riccode", "RIC"),
    ("sedol", "SEDOL"),
    ("sicovam", "SICOVAM"),
    ("synthetic", "SYNTHETIC"),
    ("uniformsymbol", "UMTF"),
    ("valoren", "VALOR"),
    ("wertpapier", "WKN"),
];

/// The field-name prefixes that name another instrument's identifier or a
/// name a person uses, never this instrument's source.
const REFUSED_FIELD_PREFIXES: [&str; 5] = ["leg", "underlying", "contra", "related", "benchmark"];

/// The field names that are a ticker, never a source.
const REFUSED_FIELD_NAMES: [&str; 3] = ["ticker", "symbol", "symbolticker"];

/// The refusal one spelling of a source earns, located on the text.
fn source_refusal(text: &str, actual: impl fmt::Display) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new(text),
        reason: crate::text::expected_got(
            format_args!("a security identifier source of 1 to {KEY_WIDTH} ASCII bytes"),
            actual,
        ),
    }
}

/// The byte class a key or a code is held to: printable ASCII.
fn ascii_refusal(text: &str) -> Option<SmolStr> {
    text.bytes().enumerate().find_map(|(position, byte)| {
        if !byte.is_ascii() {
            Some(format_smolstr!(
                "a non-ASCII byte 0x{byte:02X} at {position}"
            ))
        } else if byte.is_ascii_control() {
            Some(format_smolstr!("a control byte 0x{byte:02X} at {position}"))
        } else {
            None
        }
    })
}

impl SecType {
    /// The sources FIX's `SecurityIDSource(22)` code set names, each with its
    /// one-character code, in the code set's order.
    pub const KNOWN: [(&'static str, char); 33] = [
        ("CUSIP", '1'),
        ("SEDOL", '2'),
        ("QUIK", '3'),
        ("ISIN", '4'),
        ("RIC", '5'),
        ("ISOCCY", '6'),
        ("ISOCTRY", '7'),
        ("EXCHSYMB", '8'),
        ("CTA", '9'),
        ("BLOOMBERG", 'A'),
        ("WKN", 'B'),
        ("DUTCH", 'C'),
        ("VALOR", 'D'),
        ("SICOVAM", 'E'),
        ("BELGIAN", 'F'),
        ("COMMON", 'G'),
        ("CLEARINGHOUSE", 'H'),
        ("FPMLSPEC", 'I'),
        ("OPRA", 'J'),
        ("FPMLURL", 'K'),
        ("LOC", 'L'),
        ("MKTASSIGNED", 'M'),
        ("REDENTITY", 'N'),
        ("REDPAIR", 'P'),
        ("CFTC", 'Q'),
        ("ISDACOMMODITY", 'R'),
        ("FIGI", 'S'),
        ("LEI", 'T'),
        ("SYNTHETIC", 'U'),
        ("FIM", 'V'),
        ("INDEX", 'W'),
        ("UMTF", 'X'),
        ("DTI", 'Y'),
    ];

    /// The known source at `index` in [`Self::KNOWN`]; nothing allocates.
    fn known(index: usize) -> Self {
        Self(SmolStr::new_static(Self::KNOWN[index].0))
    }

    /// Where `key`, already upper-cased, stands in [`Self::KNOWN`].
    fn known_index(key: &str) -> Option<usize> {
        Self::KNOWN.iter().position(|(known, _)| *known == key)
    }

    /// The known source `alias` names: a key in any case, or a folded name.
    fn from_alias(alias: &str) -> Option<Self> {
        if let Some(index) = Self::KNOWN
            .iter()
            .position(|(known, _)| known.eq_ignore_ascii_case(alias))
        {
            return Some(Self::known(index));
        }
        let folded = folded_spelling(alias);
        NAMES
            .binary_search_by(|(name, _)| name.cmp(&folded.as_str()))
            .ok()
            .map(|position| Self(SmolStr::new_static(NAMES[position].1)))
    }

    /// The source one spelling names: a one-character code as FIX writes it
    /// (`4`, `A`, not folded), a known key in any case, the code set's name
    /// folded, or any other ASCII text of 1 to 32 bytes kept upper-cased.
    ///
    /// # Errors
    ///
    /// Empty, non-ASCII or over-wide text, and `TICKER` in any case, which
    /// belongs on `set_ticker`; each located on the text.
    pub fn read(text: &str) -> Result<Self> {
        let trimmed = text.trim_matches(|c: char| c.is_ascii_whitespace());
        let mut chars = trimmed.chars();
        if let (Some(code), None) = (chars.next(), chars.next()) {
            if let Some(known) = Self::from_fix_source(code) {
                return Ok(known);
            }
        }
        if let Some(known) = Self::from_alias(trimmed) {
            return Ok(known);
        }
        if trimmed.is_empty() {
            return Err(source_refusal(text, "\"\""));
        }
        if let Some(reason) = ascii_refusal(trimmed) {
            return Err(source_refusal(text, reason));
        }
        if trimmed.len() > KEY_WIDTH {
            return Err(source_refusal(
                text,
                format_args!("{} bytes", trimmed.len()),
            ));
        }
        if trimmed.eq_ignore_ascii_case("ticker") {
            return Err(Error::InvalidRecord {
                path: SmolStr::new(text),
                reason: SmolStr::new_static(
                    "a ticker is the name a person knows an instrument by, not a security \
                     identifier source: state it through set_ticker",
                ),
            });
        }
        let mut buffer = [0_u8; KEY_WIDTH];
        for (target, byte) in buffer.iter_mut().zip(trimmed.bytes()) {
            *target = byte.to_ascii_uppercase();
        }
        Ok(Self(SmolStr::new(
            std::str::from_utf8(&buffer[..trimmed.len()]).expect("validated ASCII"),
        )))
    }

    /// The source a field is named for, or `None` where the name is not one
    /// instrument's own identifier.
    ///
    /// A leading `#` is dropped and the name folds as every name here folds.
    /// The whole name as a known key or a code-set name names the source;
    /// otherwise `[security] + alias + [code | id | number]`, where the alias
    /// is one, does. `ticker`, `symbol` and `symbolticker` name no source,
    /// and neither does a field of another instrument - `leg*`,
    /// `underlying*`, `contra*`, `related*`, `benchmark*`.
    #[must_use]
    pub fn from_field_name(name: &str) -> Option<Self> {
        let folded = folded_spelling(name.strip_prefix('#').unwrap_or(name));
        let folded = folded.as_str();
        if let Some(known) = Self::from_alias(folded) {
            return Some(known);
        }
        if REFUSED_FIELD_NAMES.contains(&folded)
            || REFUSED_FIELD_PREFIXES
                .iter()
                .any(|prefix| folded.starts_with(prefix))
        {
            return None;
        }
        let alias = folded.strip_prefix("security").unwrap_or(folded);
        if alias.is_empty() {
            return None;
        }
        if alias != folded {
            if let Some(known) = Self::from_alias(alias) {
                return Some(known);
            }
        }
        ["code", "id", "number"]
            .iter()
            .find_map(|suffix| alias.strip_suffix(suffix))
            .filter(|stem| !stem.is_empty())
            .and_then(Self::from_alias)
    }

    /// The FIX `SecurityIDSource(22)` code of a known source.
    #[must_use]
    pub fn fix_source(&self) -> Option<char> {
        Self::known_index(self.as_str()).map(|index| Self::KNOWN[index].1)
    }

    /// The known source one FIX `SecurityIDSource(22)` code names.
    #[must_use]
    pub fn from_fix_source(code: char) -> Option<Self> {
        Self::KNOWN
            .iter()
            .position(|(_, known)| *known == code)
            .map(Self::known)
    }

    /// The upper-cased key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Whether the source is one of [`Self::KNOWN`].
    #[must_use]
    pub fn is_known(&self) -> bool {
        Self::known_index(self.as_str()).is_some()
    }

    /// The most bytes a code of this source may be: the fixed width of a
    /// checked code, `BLOOMBERG_WIDTH` for a Bloomberg identifier, 32 for
    /// every other source.
    #[must_use]
    pub fn max_code_width(&self) -> usize {
        match self.as_str() {
            "ISIN" | "FIGI" => 12,
            "CUSIP" | "VALOR" => 9,
            "SEDOL" => 7,
            "WKN" => 6,
            "ISOCCY" => 3,
            "ISOCTRY" => 2,
            _ => BLOOMBERG_WIDTH,
        }
    }

    /// Whether codes of this source fold to upper case.
    fn folds_case(&self) -> bool {
        matches!(self.as_str(), "ISIN" | "CUSIP" | "SEDOL" | "FIGI" | "WKN")
    }

    /// Whether `code` is a code of this source.
    ///
    /// An ISIN, a CUSIP, a SEDOL and a FIGI must close on their check digit;
    /// a WKN is six of `[0-9A-HJ-NP-Z]`, a Valor number one to nine digits
    /// without a leading zero; a Bloomberg identifier keeps its case and its
    /// inner spaces; every other source takes printable ASCII within
    /// [`Self::max_code_width`]. No source takes an empty or null-like code.
    ///
    /// # Errors
    ///
    /// The checked code's own refusal, or one located on the source naming
    /// the rule the code broke.
    pub fn validate_code(&self, code: &str) -> Result<()> {
        let refusal = |actual: &dyn fmt::Display| Error::InvalidRecord {
            path: self.0.clone(),
            reason: crate::text::expected_got(format_args!("a {} code", self.as_str()), actual),
        };
        if is_null_like(code) {
            return Err(refusal(&format_args!("{code:?}, which states nothing")));
        }
        match self.as_str() {
            "ISIN" => IsinCode::new(code).map(drop),
            "CUSIP" => CusipCode::new(code).map(drop),
            "SEDOL" => SedolCode::new(code).map(drop),
            "FIGI" => FIGICode::new(code).map(drop),
            "WKN" => {
                if code.len() == 6
                    && code
                        .bytes()
                        .all(|byte| matches!(byte.to_ascii_uppercase(), b'0'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z'))
                {
                    Ok(())
                } else {
                    Err(refusal(&format_args!(
                        "{code:?}, not six of [0-9A-HJ-NP-Z]"
                    )))
                }
            }
            "VALOR" => {
                if (1..=9).contains(&code.len())
                    && code.bytes().all(|byte| byte.is_ascii_digit())
                    && !code.starts_with('0')
                {
                    Ok(())
                } else {
                    Err(refusal(&format_args!(
                        "{code:?}, not one to nine digits without a leading zero"
                    )))
                }
            }
            _ => {
                if let Some(reason) = ascii_refusal(code) {
                    return Err(refusal(&reason));
                }
                let width = self.max_code_width();
                if code.len() > width {
                    return Err(refusal(&format_args!(
                        "{} bytes, over the {width} the source allows",
                        code.len()
                    )));
                }
                Ok(())
            }
        }
    }
}

impl fmt::Display for SecType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// SecurityId: one source and the code it gives an instrument.
// ---------------------------------------------------------------------------

/// One instrument identifier: the source it comes from and the code it gives.
///
/// One buffer holds both: a known source is one tag byte, its index in
/// [`SecType::KNOWN`] plus one, and an unknown source is `0x7F`, one length
/// byte and the key; the code follows. Every byte is ASCII, so the buffer is
/// text, inline up to 23 bytes: every checked code and a terminal-length
/// Bloomberg identifier cost no allocation. The code is validated by its
/// source on construction and, where the source folds case, held upper-cased.
///
/// ```
/// use yggdryl::{SecType, SecurityId};
///
/// let apple = SecurityId::new(SecType::read("isin").unwrap(), "us0378331005").unwrap();
/// assert_eq!(apple.to_string(), "ISIN:US0378331005");
/// assert_eq!(apple.code(), "US0378331005");
/// assert!(apple.is_inline());
/// assert!(SecurityId::new(SecType::read("isin").unwrap(), "US0378331006").is_err());
/// ```
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SecurityId(SmolStr);

impl SecurityId {
    /// An identifier of `key`, its `code` validated by the source.
    ///
    /// # Errors
    ///
    /// What [`SecType::validate_code`] refuses.
    pub fn new(key: SecType, code: &str) -> Result<Self> {
        let code = code.trim_matches(|c: char| c.is_ascii_whitespace());
        key.validate_code(code)?;
        let mut buffer = [0_u8; 2 + KEY_WIDTH + BLOOMBERG_WIDTH];
        let head = match SecType::known_index(key.as_str()) {
            Some(index) => {
                buffer[0] = 1 + index as u8;
                1
            }
            None => {
                let text = key.as_str();
                buffer[0] = UNKNOWN_TAG;
                buffer[1] = text.len() as u8;
                buffer[2..2 + text.len()].copy_from_slice(text.as_bytes());
                2 + text.len()
            }
        };
        for (target, byte) in buffer[head..].iter_mut().zip(code.bytes()) {
            *target = if key.folds_case() {
                byte.to_ascii_uppercase()
            } else {
                byte
            };
        }
        Ok(Self(SmolStr::new(
            std::str::from_utf8(&buffer[..head + code.len()]).expect("ASCII"),
        )))
    }

    /// The known index the tag names, or the width of an unknown key.
    fn split(&self) -> (Option<usize>, usize) {
        let bytes = self.0.as_bytes();
        if bytes[0] == UNKNOWN_TAG {
            (None, 2 + usize::from(bytes[1]))
        } else {
            (Some(usize::from(bytes[0]) - 1), 1)
        }
    }

    /// The key, borrowed from the table or the buffer; nothing allocates.
    fn key_str(&self) -> &str {
        match self.split() {
            (Some(index), _) => SecType::KNOWN[index].0,
            (None, head) => &self.0[2..head],
        }
    }

    /// The source: a known one from the table, no allocation.
    #[must_use]
    pub fn sectype(&self) -> SecType {
        match self.split() {
            (Some(index), _) => SecType::known(index),
            (None, head) => SecType(SmolStr::new(&self.0[2..head])),
        }
    }

    /// The code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.0[self.split().1..]
    }

    /// Whether the buffer sits inline rather than on the heap.
    #[must_use]
    pub fn is_inline(&self) -> bool {
        !self.0.is_heap_allocated()
    }
}

impl fmt::Display for SecurityId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.key_str(), self.code())
    }
}

impl Ord for SecurityId {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.key_str(), self.code()).cmp(&(other.key_str(), other.code()))
    }
}

impl PartialOrd for SecurityId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// ---------------------------------------------------------------------------
// SecurityIds: the identifiers a market states, one code per source.
// ---------------------------------------------------------------------------

/// The identifiers an instrument is stated under, one code per source,
/// sorted by source.
///
/// Two inline slots hold the ISIN and the one other code a market usually
/// states; a third spills to the heap. A source is looked up through
/// [`SecType::read`], so `get("4")`, `get("isin")` and `get("ISINNumber")`
/// are one question. Its Arrow shape is [`IdMap`]'s: a sorted map of
/// required text keys to required text values.
///
/// ```
/// use yggdryl::{SecType, SecurityId, SecurityIds};
///
/// let mut ids = SecurityIds::default();
/// let isin = SecurityId::new(SecType::read("ISIN").unwrap(), "US0378331005").unwrap();
/// assert!(ids.insert(isin));
/// assert_eq!(ids.get("4"), Some("US0378331005"));
/// assert_eq!(ids.len(), 1);
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct SecurityIds(SmallVec<[SecurityId; 2]>);

impl SecurityIds {
    /// Where `key` stands, or where it would.
    fn position(&self, key: &str) -> std::result::Result<usize, usize> {
        self.0.binary_search_by(|held| held.key_str().cmp(key))
    }

    /// The code under `key`, read as [`SecType::read`] reads a source.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.get_id(key).map(SecurityId::code)
    }

    /// The identifier under `key`, read as [`SecType::read`] reads a source.
    #[must_use]
    pub fn get_id(&self, key: &str) -> Option<&SecurityId> {
        let key = SecType::read(key).ok()?;
        let position = self.position(key.as_str()).ok()?;
        Some(&self.0[position])
    }

    /// Whether a code is held under `key`.
    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.get_id(key).is_some()
    }

    /// Fill `id`'s source: `true` when the set gained it, `false` when the
    /// source was held already.
    pub fn insert(&mut self, id: SecurityId) -> bool {
        match self.position(id.key_str()) {
            Ok(_) => false,
            Err(position) => {
                self.0.insert(position, id);
                true
            }
        }
    }

    /// Set `id`'s source to `id`, replacing what it held: `true` when the
    /// set changed.
    pub fn set(&mut self, id: SecurityId) -> bool {
        match self.position(id.key_str()) {
            Ok(position) if self.0[position] == id => false,
            Ok(position) => {
                self.0[position] = id;
                true
            }
            Err(position) => {
                self.0.insert(position, id);
                true
            }
        }
    }

    /// Remove `key`'s identifier, answering it.
    pub fn remove(&mut self, key: &SecType) -> Option<SecurityId> {
        let position = self.position(key.as_str()).ok()?;
        Some(self.0.remove(position))
    }

    /// Take every source of `other` this set lacks, this set's identifier
    /// standing where both hold a source; whether anything was added.
    pub fn merge(&mut self, other: &Self) -> bool {
        merge_sorted(&mut self.0, &other.0, SecurityId::key_str)
    }

    /// Every identifier, in source order.
    pub fn iter(&self) -> std::slice::Iter<'_, SecurityId> {
        self.0.iter()
    }

    /// How many sources the set holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the set holds no source.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Feed every identifier in source order: the key's length, the key, the
    /// code's length and the code, lengths as `u32` little-endian.
    #[expect(
        dead_code,
        reason = "fed under its label by the graph digest once the market traits carry the set"
    )]
    pub(crate) fn digest_into(&self, hasher: &mut impl Hasher) {
        for id in &self.0 {
            let (key, code) = (id.key_str(), id.code());
            hasher.write(&(key.len() as u32).to_le_bytes());
            hasher.write(key.as_bytes());
            hasher.write(&(code.len() as u32).to_le_bytes());
            hasher.write(code.as_bytes());
        }
    }

    /// The Arrow datatype the set lays out: [`IdMap::dtype`].
    #[must_use]
    pub fn dtype() -> DataType {
        IdMap::dtype()
    }

    /// The set as a sorted-map scalar of source keys to codes.
    #[must_use]
    pub fn to_scalar(&self) -> Scalar {
        sorted_map_scalar(self.0.iter().map(|id| (id.key_str(), id.code())))
    }

    /// Read a set back from the scalar [`Self::to_scalar`] answers.
    ///
    /// # Errors
    ///
    /// Anything but a mapping, a null or non-text key or value, a source
    /// that repeats or stands out of order, and a code its source refuses,
    /// each located on the entry.
    pub fn from_scalar(scalar: &Scalar) -> Result<Self> {
        let mut ids = Self::default();
        for (index, key, code) in text_entries(scalar)? {
            let key = SecType::read(key).map_err(|error| located_at(index, "key", error))?;
            match ids.0.last().map(|held| held.key_str().cmp(key.as_str())) {
                Some(Ordering::Equal) => {
                    return Err(entry_refusal(index, "key", "a source held once already"));
                }
                Some(Ordering::Greater) => {
                    return Err(entry_refusal(index, "key", "a source out of order"));
                }
                Some(Ordering::Less) | None => {}
            }
            ids.0.push(
                SecurityId::new(key, code).map_err(|error| located_at(index, "value", error))?,
            );
        }
        Ok(ids)
    }
}

impl Deref for SecurityIds {
    type Target = [SecurityId];

    fn deref(&self) -> &[SecurityId] {
        &self.0
    }
}

impl<'ids> IntoIterator for &'ids SecurityIds {
    type Item = &'ids SecurityId;
    type IntoIter = std::slice::Iter<'ids, SecurityId>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

// ---------------------------------------------------------------------------
// embedded: the national number an ISIN carries.
// ---------------------------------------------------------------------------

/// The identifier a canonical ISIN embeds as its national number, where its
/// country's scheme is one this crate checks: at most one.
///
/// `US` and `CA` carry a CUSIP in positions 2 to 11; `GB`, `IE`, `GG`, `JE`
/// and `IM` a SEDOL in positions 4 to 11 behind `00`; `DE` a WKN in positions
/// 5 to 11 behind `000`; `CH` and `LI` a Valor number in positions 2 to 11
/// with its leading zeros dropped. The embedded code must close on its own
/// check too: neither an arbitrary national number nor an unchecked ISIN is
/// enough to name another identifier.
///
/// ```
/// use yggdryl::IsinCode;
/// use yggdryl::securityid::embedded;
///
/// let apple = IsinCode::new("US0378331005").unwrap();
/// let cusip = embedded(&apple).next().unwrap();
/// assert_eq!(cusip.to_string(), "CUSIP:037833100");
/// assert!(embedded(&IsinCode::new("XS0203470157").unwrap()).next().is_none());
/// ```
pub fn embedded(isin: &IsinCode) -> impl Iterator<Item = SecurityId> {
    let text = isin.as_str();
    let found = if !IsinCode::is_canonical(text) {
        None
    } else {
        let (key, code) = match &text[..2] {
            "US" | "CA" => ("CUSIP", &text[2..11]),
            "GB" | "IE" | "GG" | "JE" | "IM" if &text[2..4] == "00" => ("SEDOL", &text[4..11]),
            "DE" if &text[2..5] == "000" => ("WKN", &text[5..11]),
            "CH" | "LI" => ("VALOR", text[2..11].trim_start_matches('0')),
            _ => ("", ""),
        };
        SecType::known_index(key)
            .and_then(|index| SecurityId::new(SecType::known(index), code).ok())
    };
    found.into_iter()
}

// ---------------------------------------------------------------------------
// SecurityIdRegistry: what one ordered lifecycle learns about an instrument.
// ---------------------------------------------------------------------------

/// One lifecycle reserves at most 32 MiB for learned associations. Each first
/// valid ISIN is charged conservatively for its full key set and hash-table
/// growth; a known ISIN needs no further reservation.
const MAX_REGISTRY_BYTES: usize = 32 * 1024 * 1024;
const ENTRY_CHARGE: usize = 2048;
const HASH_CONTROL_BYTES: usize = 1;
const MAX_BLOOMBERG_HEAP_ALLOWANCE: usize =
    BLOOMBERG_WIDTH + 2 * size_of::<usize>() + 2 * align_of::<usize>();

/// The most sources one instrument's associations are learned under.
const MAX_KEYS_PER_INSTRUMENT: usize = 8;

// Four buckets per entry cover the supported standard HashMap's load slack
// plus old and new tables during growth. Each learned key is charged its
// slot and, once, the widest retained code's heap payload, Arc counters and
// allocator rounding.
const _: () = assert!(
    ENTRY_CHARGE
        >= 4 * (size_of::<(SecurityId, Learned)>() + HASH_CONTROL_BYTES)
            + MAX_KEYS_PER_INSTRUMENT * (size_of::<Slot>() + MAX_BLOOMBERG_HEAP_ALLOWANCE)
);
const _: () = assert!(MAX_REGISTRY_BYTES / ENTRY_CHARGE == 16_384);

/// A second invariant cap independent of byte-accounting constants.
const MAX_INSTRUMENTS: usize = 65_536;

#[derive(Default)]
enum Association<T> {
    #[default]
    Missing,
    Known(T),
    Ambiguous,
}

impl<T: Clone + PartialEq> Association<T> {
    /// Learn `value`: a first sighting is known, a second different one
    /// makes the association ambiguous for good.
    fn observe(&mut self, value: &T) {
        *self = match self {
            Self::Missing => Self::Known(value.clone()),
            Self::Known(known) if known == value => return,
            Self::Known(_) | Self::Ambiguous => Self::Ambiguous,
        };
    }

    fn known(&self) -> Option<&T> {
        match self {
            Self::Known(value) => Some(value),
            Self::Missing | Self::Ambiguous => None,
        }
    }
}

impl Association<CfiCode> {
    fn classify(&mut self, value: Option<&CfiCode>) {
        let Some(value) = value else { return };
        match self {
            Self::Ambiguous => return,
            Self::Known(known) if known == value => return,
            _ => {}
        }
        if !CfiCode::is_classified(value.as_str())
            || value.as_str().as_bytes()[2..]
                .iter()
                .all(|byte| *byte == b'X')
        {
            return;
        }
        if let Self::Known(known) = self {
            if known
                .as_str()
                .bytes()
                .zip(value.as_str().bytes())
                .any(|(left, right)| left != right && left != b'X' && right != b'X')
            {
                *self = Self::Ambiguous;
            } else if let Some(merged) = CfiCode::merged(known.as_str(), value.as_str()) {
                *known = CfiCode::new(merged).expect("merged validated CFI codes");
            }
        } else {
            *self = Self::Known(value.clone());
        }
    }
}

/// One source's association for one instrument.
type Slot = (SecType, Association<SecurityId>);

#[derive(Default)]
struct Learned {
    cfi: Association<CfiCode>,
    keys: SmallVec<[Slot; 2]>,
}

/// The associations one ordered lifecycle learns between an instrument's
/// ISIN, its other identifiers and its classification, never by a codec.
pub(crate) struct SecurityIdRegistry {
    by_isin: HashMap<SecurityId, Learned>,
    reserved_bytes: usize,
    byte_budget: usize,
    /// Sources whose code names one listing of an instrument - its ISIN
    /// with a market and a currency - rather than the instrument the ISIN
    /// numbers. One ISIN has as many listings as markets, so what one
    /// message states under such a source is no association to fill onto
    /// another, and none is learned.
    listings: SmallVec<[SecType; 2]>,
}

impl Default for SecurityIdRegistry {
    fn default() -> Self {
        Self {
            by_isin: HashMap::new(),
            reserved_bytes: 0,
            byte_budget: MAX_REGISTRY_BYTES,
            listings: SmallVec::new(),
        }
    }
}

impl SecurityIdRegistry {
    /// A registry that never learns an association under one of
    /// `listings`, the sources naming a listing rather than an instrument.
    pub(crate) fn with_listings(listings: impl IntoIterator<Item = SecType>) -> Self {
        Self {
            listings: listings.into_iter().collect(),
            ..Self::default()
        }
    }

    /// The retained slot for `isin`, registering it only after a checked
    /// reservation. A rejected registration does not clone the key or ask
    /// the map to reserve.
    fn learned_for(&mut self, isin: &SecurityId) -> Option<&mut Learned> {
        if self.by_isin.contains_key(isin) {
            return self.by_isin.get_mut(isin);
        }
        if self.by_isin.len() >= MAX_INSTRUMENTS {
            return None;
        }
        let reserved_bytes = self.reserved_bytes.checked_add(ENTRY_CHARGE)?;
        if reserved_bytes > self.byte_budget {
            return None;
        }
        self.by_isin.insert(isin.clone(), Learned::default());
        self.reserved_bytes = reserved_bytes;
        self.by_isin.get_mut(isin)
    }

    /// Learn what `ids` and `cfi` state about the instrument `ids` names by
    /// its ISIN: one association per stated source but a listing's, at most
    /// `MAX_KEYS_PER_INSTRUMENT` of them, and the classification where it
    /// is detailed. Nothing without an ISIN.
    pub(crate) fn learn(&mut self, ids: &SecurityIds, cfi: Option<&CfiCode>) {
        let Some(isin) = ids.get_id("ISIN") else {
            return;
        };
        let learnable: SmallVec<[&SecurityId; 8]> = ids
            .iter()
            .filter(|id| {
                id.key_str() != "ISIN"
                    && !self
                        .listings
                        .iter()
                        .any(|listing| listing.as_str() == id.key_str())
            })
            .collect();
        let Some(learned) = self.learned_for(isin) else {
            return;
        };
        learned.cfi.classify(cfi);
        for id in learnable {
            let position = learned
                .keys
                .iter()
                .position(|(key, _)| key.as_str() == id.key_str());
            let slot = match position {
                Some(position) => &mut learned.keys[position],
                None if learned.keys.len() < MAX_KEYS_PER_INSTRUMENT => {
                    learned.keys.push((id.sectype(), Association::Missing));
                    learned.keys.last_mut().expect("just pushed")
                }
                None => continue,
            };
            slot.1.observe(id);
        }
    }

    /// Fill what `ids` and `cfi` leave unstated about the instrument `ids`
    /// names by its ISIN, from what was learned: only absent sources, and a
    /// classification only where the learned one refines the stated one.
    /// Whether anything was filled.
    pub(crate) fn fill(&self, ids: &mut SecurityIds, cfi: &mut Option<CfiCode>) -> bool {
        let Some(learned) = ids.get_id("ISIN").and_then(|isin| self.by_isin.get(isin)) else {
            return false;
        };
        let mut changed = false;
        if let Some(known) = learned.cfi.known() {
            let replacement = match cfi.as_ref() {
                None => Some(known.clone()),
                Some(stated) if stated != known => {
                    CfiCode::merged(stated.as_str(), known.as_str())
                        .filter(|merged| {
                            // Unknown positions can fill; a stated classification
                            // attribute is never overwritten by a learned default.
                            stated
                                .as_str()
                                .bytes()
                                .zip(merged.bytes())
                                .all(|(old, new)| old == b'X' || old == new)
                                && merged.as_str() != stated.as_str()
                        })
                        .and_then(|merged| CfiCode::new(merged).ok())
                }
                Some(_) => None,
            };
            if let Some(code) = replacement {
                *cfi = Some(code);
                changed = true;
            }
        }
        for (_, association) in &learned.keys {
            if let Some(known) = association.known() {
                changed |= ids.insert(known.clone());
            }
        }
        changed
    }
}

/// Learn what `event` states about its instrument - its security identifiers
/// and its CFI - then fill what it left unstated: each absent key the
/// registry knows for the ISIN is derived onto the event, never stated, and
/// the element is finalized where anything moved.
pub(crate) fn enrich<E: Market + Element>(registry: &mut SecurityIdRegistry, event: &mut E) {
    let mut ids = event.get_securityids().clone();
    let mut cfi = event.get_cficode().cloned();
    registry.learn(&ids, cfi.as_ref());
    if !registry.fill(&mut ids, &mut cfi) {
        return;
    }
    let mut changed = false;
    if cfi.as_ref() != event.get_cficode() {
        event.set_cficode(cfi);
        changed = true;
    }
    for id in ids.iter() {
        if !event.get_securityids().contains_key(id.sectype().as_str()) {
            changed |= event.derive_securityid(id.clone());
        }
    }
    if changed {
        event.finalize();
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/securityid.rs` pins and a caller cannot reach.
    //!
    //! The registry is a lifecycle's own: nothing above it names one, and what
    //! it learned is only ever read off the events it filled. What it costs -
    //! one reservation per first valid ISIN, none for a known one, and a full
    //! registry that still learns about the instruments it already holds - is
    //! read here through a wrapper, so the registry, its budget and its
    //! reservation stay exactly as private as they were.
    use std::collections::HashMap;

    use crate::graph::{Element, Market};
    use crate::{CfiCode, SecurityIds};

    /// The instrument associations one ordered lifecycle learns.
    #[derive(Default)]
    pub struct SecurityIdRegistry(super::SecurityIdRegistry);

    impl SecurityIdRegistry {
        /// A registry that may reserve at most `byte_budget` bytes.
        pub fn with_budget(byte_budget: usize) -> Self {
            Self(super::SecurityIdRegistry {
                by_isin: HashMap::new(),
                reserved_bytes: 0,
                byte_budget,
                listings: SmallVec::new(),
            })
        }

        /// A registry whose reservation arithmetic already stands at the top
        /// of `usize`, so the next charge can only overflow.
        pub fn saturated() -> Self {
            Self(super::SecurityIdRegistry {
                by_isin: HashMap::new(),
                reserved_bytes: usize::MAX,
                byte_budget: usize::MAX,
                listings: SmallVec::new(),
            })
        }

        /// Learn what `event` states, then fill what it left unstated.
        pub fn enrich<E: Market + Element>(&mut self, event: &mut E) {
            super::enrich(&mut self.0, event);
        }

        /// Learn what `ids` and `cfi` state.
        pub fn learn(&mut self, ids: &SecurityIds, cfi: Option<&CfiCode>) {
            self.0.learn(ids, cfi);
        }

        /// Fill what `ids` and `cfi` leave unstated; whether anything was.
        pub fn fill(&self, ids: &mut SecurityIds, cfi: &mut Option<CfiCode>) -> bool {
            self.0.fill(ids, cfi)
        }

        /// How many instruments the registry holds.
        pub fn instruments(&self) -> usize {
            self.0.by_isin.len()
        }

        /// The bytes it has reserved for them.
        pub fn reserved_bytes(&self) -> usize {
            self.0.reserved_bytes
        }
    }

    /// What one instrument's first sighting reserves.
    pub const ENTRY_CHARGE: usize = super::ENTRY_CHARGE;

    /// The most sources one instrument's associations are learned under.
    pub const MAX_KEYS_PER_INSTRUMENT: usize = super::MAX_KEYS_PER_INSTRUMENT;
}
