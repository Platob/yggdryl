//! Security identifiers: the national number an ISIN embeds and the shape a
//! symbol reads as. What a lifecycle learns between an instrument's
//! identifiers is an [`Instruments`](crate::Instruments)'s.

use crate::{IdKey, IdSource, IdType, Identifier};
use yggdryl::{Cfi, Cusip, Figi, Isin, Ric, Sedol};

// ---------------------------------------------------------------------------
// embedded: the national number an ISIN carries.
// ---------------------------------------------------------------------------

/// The identifier a closing ISIN embeds as its national number, where its
/// country's scheme is one this crate checks: at most one.
///
/// `US` and `CA` carry a CUSIP in positions 2 to 11; `GB`, `IE`, `GG`, `JE`
/// and `IM` a SEDOL in positions 4 to 11 behind `00`; `DE` a WKN in positions
/// 5 to 11 behind `000`; `CH` and `LI` a Valor number in positions 2 to 11
/// with its leading zeros dropped. The number must close on its check digit
/// ([`Isin::is_closed`]) and the embedded code on its own: neither an
/// arbitrary national number nor a number that does not close - a typo, a
/// mask - is enough to name another identifier.
///
/// ```
/// # yggdryl_market::install().unwrap();
/// use yggdryl::Isin;
/// use yggdryl_market::securityid::embedded;
///
/// let apple = Isin::new("US0378331005").unwrap();
/// let cusip = embedded(&apple).next().unwrap();
/// assert_eq!(cusip.to_string(), "derived:cusip=037833100");
/// assert!(embedded(&Isin::new("XS0203470157").unwrap()).next().is_none());
/// assert!(embedded(&Isin::new("US0378331006").unwrap()).next().is_none());
/// ```
pub fn embedded(isin: &Isin) -> impl Iterator<Item = Identifier> {
    let text = isin.as_str();
    let found = if !Isin::is_closed(text) {
        None
    } else {
        let (kind, code) = match &text[..2] {
            "US" | "CA" => (Some(IdType::Cusip), &text[2..11]),
            "GB" | "IE" | "GG" | "JE" | "IM" if &text[2..4] == "00" => {
                (Some(IdType::Sedol), &text[4..11])
            }
            "DE" if &text[2..5] == "000" => (Some(IdType::Wkn), &text[5..11]),
            "CH" | "LI" => (Some(IdType::Valor), text[2..11].trim_start_matches('0')),
            _ => (None, ""),
        };
        kind.and_then(|kind| Identifier::new(IdKey::new(IdSource::Derived, kind), code).ok())
            .filter(|id| id.kind().is_real(id.value()))
    };
    found.into_iter()
}

// ---------------------------------------------------------------------------
// SymbolCode: what a symbol's own shape says it is.
// ---------------------------------------------------------------------------

/// What a symbol is, read off its shape: a ticker a venue gives is often an
/// identifier a standard gives, and its length alone says which one to try
/// before any check runs.
///
/// | length | shape | reads as |
/// | --- | --- | --- |
/// | 21 to 26 | `{ISIN}_{MIC}_{CCY}` | [`Self::Instrument`], each part its own type's |
/// | 12 | `BBG` then nine | [`Self::Figi`] |
/// | 12 | any other | [`Self::Isin`] |
/// | 9 | | [`Self::Cusip`] |
/// | 7 | | [`Self::Sedol`] |
/// | 6 | upper-case letters, a detailed classification | [`Self::Cfi`] |
/// | any | a dot between a code and a venue - `AAPL.OQ` | [`Self::Ric`] |
/// | any | a Bloomberg yellow key last - `AAPL US Equity` | [`Self::Bloomberg`] |
///
/// Every candidate closes on its own type's check - an ISIN's, a FIGI's, a
/// CUSIP's and a SEDOL's check digit, a CFI's grammar past its category
/// and group - so a symbol that is
/// only the length of one reads as nothing, and a symbol reading as nothing
/// is a ticker and nothing more. A currency pair is the FIX layer's
/// [`FxSymbol`](yggdryl::FxSymbol) reading, which knows its tenors.
///
/// ```
/// # yggdryl_market::install().unwrap();
/// use yggdryl_market::securityid::SymbolCode;
///
/// assert!(matches!(SymbolCode::from_symbol("US0378331005"), Some(SymbolCode::Isin(_))));
/// assert!(matches!(SymbolCode::from_symbol("BBG000BLNQ16"), Some(SymbolCode::Figi(_))));
/// assert!(matches!(
///     SymbolCode::from_symbol("CH0012214059_XSWX_CHF"),
///     Some(SymbolCode::Instrument { .. })
/// ));
/// assert!(matches!(
///     SymbolCode::from_symbol("CH0012214059_XSWX_USDT"),
///     Some(SymbolCode::Instrument { .. })
/// ));
/// assert!(matches!(SymbolCode::from_symbol("ESVUFR"), Some(SymbolCode::Cfi(_))));
/// assert!(matches!(SymbolCode::from_symbol("AAPL.OQ"), Some(SymbolCode::Ric(_))));
/// assert!(matches!(SymbolCode::from_symbol("HOLN SW Equity"), Some(SymbolCode::Bloomberg(_))));
/// assert_eq!(SymbolCode::from_symbol("AAPL"), None);
/// assert_eq!(SymbolCode::from_symbol("US0378331006"), None);
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SymbolCode {
    /// An instrument key: its ISIN, the MIC it trades on and the currency
    /// it trades in, `_` between them - twelve and four characters and a
    /// currency of three to eight, ISO 4217's or a digital-asset ticker -
    /// each part read by its own type and `None` where that type refuses
    /// it, so one bad part leaves the others answering.
    Instrument {
        /// The instrument.
        isin: Option<Isin>,
        /// Where it trades: an ISO 10383 MIC, or the Reuters mnemonic FIX
        /// 4.2 spelled one in.
        mic: Option<yggdryl::Mic>,
        /// What it trades in.
        ccy: Option<yggdryl::Ccy>,
    },
    /// An ISIN.
    Isin(Isin),
    /// A FIGI.
    Figi(Figi),
    /// A CUSIP.
    Cusip(Cusip),
    /// A SEDOL.
    Sedol(Sedol),
    /// A CFI code.
    Cfi(Cfi),
    /// A Reuters instrument code.
    Ric(Ric),
    /// A Bloomberg identifier ending in its yellow key.
    Bloomberg(yggdryl::Bbg),
}

/// The fewest bytes an instrument key's currency is: ISO 4217's three.
const INSTRUMENT_CURRENCY: usize = 3;

/// The shortest and longest instrument key: an ISIN, a MIC and a currency
/// of three up to a currency's bound, `_` between them.
const INSTRUMENT_SHORTEST: usize = 12 + 1 + 4 + 1 + INSTRUMENT_CURRENCY;
const INSTRUMENT_LONGEST: usize = 12 + 1 + 4 + 1 + yggdryl::implementer::CCY_WIDTH;

/// The Bloomberg yellow keys a terminal identifier ends with.
const YELLOW_KEYS: [&str; 9] = [
    "Equity", "Comdty", "Curncy", "Index", "Govt", "Corp", "Mtge", "Muni", "Pfd",
];

impl SymbolCode {
    /// The security identifier this symbol names, as its type and code: an
    /// instrument key's ISIN, else the code itself; a CFI code names a
    /// classification and no identifier. What a ticker of this shape
    /// derives ([`Market::fill_market`]), and what a view of that
    /// derivation reads back as.
    pub(crate) fn identifier(&self) -> Option<(IdType, &str)> {
        match self {
            Self::Instrument { isin, .. } => {
                isin.as_ref().map(|isin| (IdType::Isin, isin.as_str()))
            }
            Self::Isin(isin) => Some((IdType::Isin, isin.as_str())),
            Self::Figi(figi) => Some((IdType::Figi, figi.as_str())),
            Self::Cusip(cusip) => Some((IdType::Cusip, cusip.as_str())),
            Self::Sedol(sedol) => Some((IdType::Sedol, sedol.as_str())),
            Self::Ric(ric) => Some((IdType::Ric, ric.as_str())),
            Self::Bloomberg(bbg) => Some((IdType::Bloomberg, bbg.as_str())),
            Self::Cfi(_) => None,
        }
    }

    /// What `symbol` is, by its length first and its type's check second,
    /// or `None` where it is no identifier this crate checks.
    #[must_use]
    pub fn from_symbol(symbol: &str) -> Option<Self> {
        let text = symbol.trim();
        let bytes = text.as_bytes();
        let upper = |part: &[u8]| part.iter().all(u8::is_ascii_uppercase);
        let alphanumeric = |part: &[u8]| part.iter().all(u8::is_ascii_alphanumeric);
        let found = match bytes.len() {
            INSTRUMENT_SHORTEST..=INSTRUMENT_LONGEST if bytes[12] == b'_' && bytes[17] == b'_' => {
                Self::instrument(text)
            }
            12 if text.starts_with("BBG") => Figi::new(text)
                .ok()
                .filter(|figi| Figi::is_closed(figi.as_str()))
                .map(Self::Figi),
            12 if alphanumeric(bytes) => Isin::new(text)
                .ok()
                .filter(|isin| Isin::is_closed(isin.as_str()))
                .map(Self::Isin),
            9 if alphanumeric(bytes) => Cusip::new(text)
                .ok()
                .filter(|cusip| Cusip::is_closed(cusip.as_str()))
                .map(Self::Cusip),
            7 if alphanumeric(bytes) => Sedol::new(text)
                .ok()
                .filter(|sedol| Sedol::is_closed(sedol.as_str()))
                .map(Self::Sedol),
            6 if upper(bytes) && Cfi::is_detailed(text) => Cfi::new(text).ok().map(Self::Cfi),
            _ => None,
        };
        found.or_else(|| {
            let yellow = text.contains(' ')
                && text.rsplit(' ').next().is_some_and(|key| {
                    YELLOW_KEYS
                        .iter()
                        .any(|yellow| yellow.eq_ignore_ascii_case(key))
                });
            if yellow {
                return yggdryl::Bbg::new(text).ok().map(Self::Bloomberg);
            }
            let (code, venue) = text.split_once('.')?;
            (!code.is_empty()
                && !venue.is_empty()
                && !venue.contains('.')
                && alphanumeric(venue.as_bytes())
                && !text.contains(' '))
            .then(|| Ric::new(text).ok().map(Self::Ric))
            .flatten()
        })
    }

    /// The instrument key `{ISIN}_{MIC}_{CCY}` spells - twelve and four
    /// ASCII letters and digits and a currency of three to eight, `_USDT`
    /// as well as `_CHF`, `_` between them -
    /// each part read by its own type, or `None` where the text is not that
    /// shape or no part reads.
    #[must_use]
    pub fn instrument(text: &str) -> Option<Self> {
        let mut parts = text.split('_');
        let (isin, mic, ccy) = (parts.next()?, parts.next()?, parts.next()?);
        let shaped = |part: &str, widths: std::ops::RangeInclusive<usize>| {
            widths.contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        };
        if parts.next().is_some()
            || !shaped(isin, 12..=12)
            || !shaped(mic, 4..=4)
            || !shaped(ccy, INSTRUMENT_CURRENCY..=yggdryl::implementer::CCY_WIDTH)
        {
            return None;
        }
        let isin = Isin::new(isin)
            .ok()
            .filter(|held| Isin::is_closed(held.as_str()));
        let mic = yggdryl::implementer::mic_from_market(mic);
        let ccy = yggdryl::Ccy::new(ccy).ok();
        (isin.is_some() || mic.is_some() || ccy.is_some()).then_some(Self::Instrument {
            isin,
            mic,
            ccy,
        })
    }
}
