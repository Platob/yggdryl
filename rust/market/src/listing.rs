//! One listing of an instrument: the market it trades on and what it is
//! known by there - its ticker, its trading currency and the listing codes
//! ([`IdType::is_listing`]) a venue or a vendor names it by on that market.
//! An [`Instrument`](crate::Instrument) holds one per market, in MIC order,
//! or the unlisted one alone while no market is known.

use smol_str::{SmolStr, format_smolstr};

use crate::{IdKey, IdSource, IdType, Identifier, Identifiers};
use yggdryl::implementer::expected_got;
use yggdryl::{Ccy, DataType, Error, Field, Mic, Result, Scalar};

/// The column names of [`Listing::dtype`], in order.
const NAMES: [&str; 4] = ["miccode", "ticker", "currency", "codes"];

/// One listing of an instrument on one market.
///
/// The `miccode` is the market - none for the unlisted listing an
/// instrument holds alone while no market is known, an index's, which the
/// first market learned takes over; the `ticker`, one to
/// [`Listing::MAX_TICKER_WIDTH`] bytes once trimmed, and the trading
/// `currency` are the listing's own - a share listed in London trades in
/// pence whatever it was issued in - and `codes` are the listing codes
/// stated on that market, one per type, at most
/// [`Instrument::MAX_LISTING_CODES`](crate::Instrument::MAX_LISTING_CODES)
/// of them, each held as its type stores it.
///
/// ```
/// use yggdryl::{Ccy, Mic};
/// use yggdryl_market::{IdType, Listing};
///
/// # fn main() -> yggdryl::Result<()> {
/// #     yggdryl_market::install().unwrap();
/// let listing = Listing::new(Some(Mic::new("XSWX")?))
///     .with_ticker(Some(" HOLN ".into()))
///     .with_currency(Some(Ccy::new("CHF")?))
///     .try_with_code(IdType::Ric, "HOLN.S")?;
/// assert_eq!(listing.ticker(), Some("HOLN"), "trimmed");
/// assert_eq!(listing.get(&IdType::Ric), Some("HOLN.S"));
/// assert!(listing.clone().try_with_code(IdType::Cusip, "037833100").is_err(), "no listing code");
/// assert_eq!(Listing::from_scalar(&listing.into_scalar())?, listing);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Listing {
    miccode: Option<Mic>,
    ticker: Option<SmolStr>,
    currency: Option<Ccy>,
    codes: Identifiers,
}

impl Listing {
    /// The most bytes a ticker holds.
    pub const MAX_TICKER_WIDTH: usize = 64;

    /// A listing on `market` - `XXXX` and none the unlisted one - stating
    /// nothing else.
    #[must_use]
    pub fn new(market: Option<Mic>) -> Self {
        Self {
            miccode: market.filter(|code| !code.is_none()),
            ticker: None,
            currency: None,
            codes: Identifiers::new(),
        }
    }

    /// The datatype a listing is: `struct<miccode: mic?, ticker: utf8?,
    /// currency: ccy?, codes: map<utf8, utf8>?>`.
    #[must_use]
    pub fn dtype() -> DataType {
        DataType::Struct(yggdryl::implementer::struct_type_from_unique_fields(vec![
            DataType::Mic.nullable_field(NAMES[0]),
            DataType::utf8().nullable_field(NAMES[1]),
            DataType::ccy().nullable_field(NAMES[2]),
            Identifiers::dtype().nullable_field(NAMES[3]),
        ]))
    }

    /// The required field `listing`: the item of an instrument's `listings`
    /// column.
    #[must_use]
    pub fn field() -> Field {
        Field::new("listing", Self::dtype(), false)
    }

    /// The market; none for the unlisted listing.
    #[must_use]
    pub fn miccode(&self) -> Option<&Mic> {
        self.miccode.as_ref()
    }

    /// Lists this listing on `market`: what the first market learned does
    /// to the unlisted one.
    pub(crate) fn set_miccode(&mut self, market: Mic) {
        self.miccode = Some(market);
    }

    /// The ticker on this market.
    #[must_use]
    pub fn ticker(&self) -> Option<&str> {
        self.ticker.as_deref()
    }

    /// The trading currency on this market.
    #[must_use]
    pub fn currency(&self) -> Option<&Ccy> {
        self.currency.as_ref()
    }

    /// The listing codes stated on this market, by type.
    #[must_use]
    pub fn codes(&self) -> &Identifiers {
        &self.codes
    }

    /// The code of `kind` on this market.
    #[must_use]
    pub fn get(&self, kind: &IdType) -> Option<&str> {
        self.codes.get(kind)
    }

    /// The ticker `ticker` spells once trimmed: one to
    /// [`Self::MAX_TICKER_WIDTH`] bytes, else none.
    pub(crate) fn trimmed_ticker(ticker: &str) -> Option<&str> {
        let trimmed = ticker.trim();
        (1..=Self::MAX_TICKER_WIDTH)
            .contains(&trimmed.len())
            .then_some(trimmed)
    }

    /// The listing with the ticker, trimmed; one outside one to sixty-four
    /// bytes is stored as none.
    #[must_use]
    pub fn with_ticker(mut self, ticker: Option<SmolStr>) -> Self {
        self.set_ticker(ticker.as_deref());
        self
    }

    /// Sets the ticker as [`Self::with_ticker`] does.
    pub(crate) fn set_ticker(&mut self, ticker: Option<&str>) {
        self.ticker = ticker.and_then(Self::trimmed_ticker).map(SmolStr::new);
    }

    /// The listing with the trading currency; `XXX`, no currency, is stored
    /// as none.
    #[must_use]
    pub fn with_currency(mut self, currency: Option<Ccy>) -> Self {
        self.set_currency(currency);
        self
    }

    /// Sets the currency as [`Self::with_currency`] does.
    pub(crate) fn set_currency(&mut self, currency: Option<Ccy>) {
        self.currency = currency.filter(|code| !code.is_none());
    }

    /// States `value` as the code of `kind` on this market, held as the type
    /// stores it, replacing the one held.
    ///
    /// # Errors
    ///
    /// A type that is no listing type ([`IdType::is_listing`]), a value the
    /// type refuses, a new type past
    /// [`Instrument::MAX_LISTING_CODES`](crate::Instrument::MAX_LISTING_CODES).
    pub fn set_code(&mut self, kind: IdType, value: &str) -> Result<()> {
        if !kind.is_listing() {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$.{kind}"),
                reason: format_smolstr!("expected a listing code type, got {kind}"),
            });
        }
        let id = Identifier::new(IdKey::base(kind), value)?;
        if !self.adopt(id) {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$.{}", self.codes.len()),
                reason: format_smolstr!(
                    "expected at most {} listing codes, got one more",
                    crate::Instrument::MAX_LISTING_CODES
                ),
            });
        }
        Ok(())
    }

    /// Holds `id`, a listing code its type proved, under its base key;
    /// answers whether it landed, a new type past the bound passed over.
    pub(crate) fn adopt(&mut self, id: Identifier) -> bool {
        if !self.accepts(id.kind()) {
            return false;
        }
        self.codes.set(id);
        true
    }

    /// Whether a code of `kind` lands: a type held, or room under
    /// [`Instrument::MAX_LISTING_CODES`](crate::Instrument::MAX_LISTING_CODES)
    /// for a new one - the bound counts types, each held under its base key
    /// whatever named sources state it beside.
    pub(crate) fn accepts(&self, kind: &IdType) -> bool {
        self.codes.contains_kind(kind)
            || self.codes.iter().filter(|id| id.key().is_base()).count()
                < crate::Instrument::MAX_LISTING_CODES
    }

    /// [`Self::set_code`], consuming.
    ///
    /// # Errors
    ///
    /// What [`Self::set_code`] refuses.
    pub fn try_with_code(mut self, kind: IdType, value: &str) -> Result<Self> {
        self.set_code(kind, value)?;
        Ok(self)
    }

    /// The listing as the named struct of its four cells, an unstated fact
    /// a null, no codes a null.
    #[must_use]
    pub fn into_scalar(&self) -> Scalar {
        Scalar::from_struct([
            (
                NAMES[0],
                self.miccode.clone().map_or(Scalar::Null, Scalar::from),
            ),
            (NAMES[1], self.ticker().map_or(Scalar::Null, Scalar::from)),
            (
                NAMES[2],
                self.currency.clone().map_or(Scalar::Null, Scalar::from),
            ),
            (
                NAMES[3],
                if self.codes.is_empty() {
                    Scalar::Null
                } else {
                    self.codes.into_scalar()
                },
            ),
        ])
        .expect("four distinct names")
    }

    /// The listing as the ordered run of its four cells in [`Self::dtype`]'s
    /// order - what a snapshot streams, one allocation and no name.
    // Built from borrowed facts, as `into_scalar` beside it is.
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn into_row(&self) -> Scalar {
        Scalar::from_sequence([
            self.miccode.clone().map_or(Scalar::Null, Scalar::from),
            self.ticker().map_or(Scalar::Null, Scalar::from),
            self.currency.clone().map_or(Scalar::Null, Scalar::from),
            if self.codes.is_empty() {
                Scalar::Null
            } else {
                self.codes.into_scalar()
            },
        ])
    }

    /// Reads a listing back from the named struct [`Self::into_scalar`]
    /// answers - a name it lacks a null - or from the ordered row
    /// [`Self::dtype`]'s value door canonicalizes it to.
    ///
    /// # Errors
    ///
    /// The refusal [`Self::field`]'s [`Field::scalar`] answers, a code that
    /// is no listing code, a value its type refuses and more than the bound.
    pub fn from_scalar(value: &Scalar) -> Result<Self> {
        let value = match value.as_struct() {
            Some(fields) => {
                let absent = NAMES
                    .iter()
                    .filter(|name| !fields.contains_key(**name))
                    .map(|name| (SmolStr::new_static(name), Scalar::Null));
                Scalar::from_struct(
                    fields
                        .iter()
                        .map(|(name, cell)| (name.clone(), cell.clone()))
                        .chain(absent),
                )?
            }
            None => value.clone(),
        };
        let row = Self::field().scalar(value)?;
        let unread = || Error::InvalidRecord {
            path: SmolStr::new_static("$.listing"),
            reason: expected_got("the canonical listing row", row.kind()),
        };
        let cells = row.sequence_rows().ok_or_else(unread)?;
        let [market, ticker, currency, codes] = cells.as_ref() else {
            return Err(unread());
        };
        let market = match market {
            Scalar::Mic(code) => Some(code.clone()),
            Scalar::Null => None,
            _ => return Err(unread()),
        };
        let mut listing = Self::new(market)
            .with_ticker(ticker.as_str().map(SmolStr::new))
            .with_currency(match currency {
                Scalar::Ccy(code) => Some(code.clone()),
                _ => None,
            });
        if !matches!(codes, Scalar::Null) {
            for id in Identifiers::from_scalar(codes)?.iter() {
                if *id.key().src() == IdSource::Derived {
                    continue;
                }
                if id.key().is_base() {
                    listing.set_code(id.kind().clone(), id.value())?;
                } else if id.kind().is_listing() {
                    // A named source's statement, kept under its own key
                    // beside the base key it fills.
                    listing.adopt(id.clone());
                }
            }
        }
        Ok(listing)
    }
}
