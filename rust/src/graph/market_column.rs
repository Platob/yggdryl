//! The twenty-seven columns every market element is stated in.
//!
//! One column per fact [`Market`] answers, under one name and one datatype
//! each, in one order, so every generated schema of a market - an
//! operation's row, a book's - states the same columns and a reader joins
//! them without a mapping.

use smol_str::SmolStr;

use super::{FxRates, Market};
use crate::securityid::{SecType, SecurityId, SecurityIds};
use crate::{
    Ccy, Cfi, DataType, Decimal, Field, Isin, Mic, Result, Scalar, Side, StructType, TimeInForce,
    Unit,
};

/// One column of the market facts every market element answers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MarketColumn {
    /// The price the element is about.
    Price,
    /// The currency it is priced in.
    Currency,
    /// The quantity it is about.
    Quantity,
    /// The unit the quantity is counted in.
    Unit,
    /// The side it takes.
    Side,
    /// The security identifiers it names, key to code, sorted.
    SecurityIds,
    /// The ISIN it names: the `ISIN` security identifier, projected.
    IsinCode,
    /// The detailed CFI classification.
    CfiCode,
    /// The market it trades on.
    MicCode,
    /// The last executed price it reports.
    LastPx,
    /// The last executed quantity it reports.
    LastQty,
    /// The average price of what it traded.
    AvgPx,
    /// How much it has traded.
    CumQty,
    /// How much is left to trade.
    LeavesQty,
    /// The price the step before it settled on.
    PrevPx,
    /// The quantity the step before it settled on.
    PrevQty,
    /// The spot part of an FX forward price.
    SpotRate,
    /// The forward points of an FX forward price.
    ForwardPoints,
    /// The bid price it states.
    BidPx,
    /// The quantity bid.
    BidQty,
    /// The currency the bid is stated in.
    BidCcy,
    /// The ask price it states.
    AskPx,
    /// The quantity offered.
    AskQty,
    /// The currency the ask is stated in.
    AskCcy,
    /// The FX rates it states: target currency to the rate to divide by.
    FxRates,
    /// The ticker the instrument goes by.
    Ticker,
    /// The free-form facts it carries, key to value, sorted.
    Metadata,
}

impl MarketColumn {
    /// Every market column in canonical row order.
    pub const ALL: [Self; 27] = [
        Self::Price,
        Self::Currency,
        Self::Quantity,
        Self::Unit,
        Self::Side,
        Self::SecurityIds,
        Self::IsinCode,
        Self::CfiCode,
        Self::MicCode,
        Self::LastPx,
        Self::LastQty,
        Self::AvgPx,
        Self::CumQty,
        Self::LeavesQty,
        Self::PrevPx,
        Self::PrevQty,
        Self::SpotRate,
        Self::ForwardPoints,
        Self::BidPx,
        Self::BidQty,
        Self::BidCcy,
        Self::AskPx,
        Self::AskQty,
        Self::AskCcy,
        Self::FxRates,
        Self::Ticker,
        Self::Metadata,
    ];

    /// The column's name: the fact's, as the traits spell it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Price => "price",
            Self::Currency => "currency",
            Self::Quantity => "quantity",
            Self::Unit => "unit",
            Self::Side => "side",
            Self::SecurityIds => "securityids",
            Self::IsinCode => "isincode",
            Self::CfiCode => "cficode",
            Self::MicCode => "miccode",
            Self::LastPx => "lastpx",
            Self::LastQty => "lastqty",
            Self::AvgPx => "avgpx",
            Self::CumQty => "cumqty",
            Self::LeavesQty => "leavesqty",
            Self::PrevPx => "prevpx",
            Self::PrevQty => "prevqty",
            Self::SpotRate => "spotrate",
            Self::ForwardPoints => "forwardpoints",
            Self::BidPx => "bidpx",
            Self::BidQty => "bidqty",
            Self::BidCcy => "bidccy",
            Self::AskPx => "askpx",
            Self::AskQty => "askqty",
            Self::AskCcy => "askccy",
            Self::FxRates => "fxrates",
            Self::Ticker => "ticker",
            Self::Metadata => "metadata",
        }
    }

    /// The column's display name.
    #[must_use]
    pub const fn display(self) -> &'static str {
        match self {
            Self::Price => "Price",
            Self::Currency => "Currency",
            Self::Quantity => "Quantity",
            Self::Unit => "Unit",
            Self::Side => "Side",
            Self::SecurityIds => "Security IDs",
            Self::IsinCode => "ISIN",
            Self::CfiCode => "CFI",
            Self::MicCode => "MIC",
            Self::LastPx => "Last Price",
            Self::LastQty => "Last Quantity",
            Self::AvgPx => "Average Price",
            Self::CumQty => "Cumulative Quantity",
            Self::LeavesQty => "Leaves Quantity",
            Self::PrevPx => "Previous Price",
            Self::PrevQty => "Previous Quantity",
            Self::SpotRate => "Spot Rate",
            Self::ForwardPoints => "Forward Points",
            Self::BidPx => "Bid Price",
            Self::BidQty => "Bid Quantity",
            Self::BidCcy => "Bid Currency",
            Self::AskPx => "Ask Price",
            Self::AskQty => "Ask Quantity",
            Self::AskCcy => "Ask Currency",
            Self::FxRates => "FX Rates",
            Self::Ticker => "Ticker",
            Self::Metadata => "Metadata",
        }
    }

    /// The one datatype the column is built and read at: the crate's
    /// decimal for every price and quantity, each code's own leaf, a sorted
    /// `map<utf8, utf8>` for the identifiers and the metadata, a sorted
    /// `map<ccy, decimal>` for the rates - keys and values required - and
    /// `utf8` for the ticker.
    #[must_use]
    pub fn datatype(self) -> DataType {
        match self {
            Self::Price
            | Self::Quantity
            | Self::LastPx
            | Self::LastQty
            | Self::AvgPx
            | Self::CumQty
            | Self::LeavesQty
            | Self::PrevPx
            | Self::PrevQty
            | Self::SpotRate
            | Self::ForwardPoints
            | Self::BidPx
            | Self::BidQty
            | Self::AskPx
            | Self::AskQty => DataType::Decimal,
            Self::Currency | Self::BidCcy | Self::AskCcy => DataType::Ccy,
            Self::Unit => DataType::Unit,
            Self::Side => DataType::Side,
            Self::SecurityIds => SecurityIds::dtype(),
            Self::IsinCode => DataType::Isin,
            Self::CfiCode => DataType::Cfi,
            Self::MicCode => DataType::Mic,
            Self::FxRates => fxrates_datatype(),
            Self::Ticker => DataType::utf8(),
            Self::Metadata => DataType::map_of(DataType::utf8(), DataType::utf8(), true)
                .expect("a sorted utf8 map is a datatype"),
        }
    }

    /// Whether a row may leave the column null: never for the currency,
    /// the unit and the side, which every market element states, if only as
    /// nothing - `XXX`, the empty unit, `UNKNOWN`.
    #[must_use]
    pub const fn nullable(self) -> bool {
        !matches!(self, Self::Currency | Self::Unit | Self::Side)
    }

    /// The column as a field, named, typed and displayed.
    ///
    /// # Errors
    ///
    /// Returns an error when the display cannot be set.
    pub fn field(self) -> Result<Field> {
        let mut field = Field::new(self.name(), self.datatype(), self.nullable());
        field.set_display(self.display())?;
        Ok(field)
    }

    /// Every column as a field, in [`Self::ALL`] order.
    ///
    /// # Errors
    ///
    /// Returns an error when a display cannot be set.
    pub fn fields() -> Result<Vec<Field>> {
        Self::ALL.into_iter().map(Self::field).collect()
    }

    /// The column a name spells, ignoring ASCII case.
    #[must_use]
    pub fn of_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|column| crate::folds_equal(column.name(), name))
    }

    /// The column's cell for `element`: the fact it states, `None` where it
    /// states none.
    pub fn fact<E: Market + ?Sized>(self, element: &E) -> Option<Scalar> {
        match self {
            Self::Price => element.get_price().map(Scalar::from),
            Self::Currency => Some(element.get_currency().clone().into()),
            Self::Quantity => element.get_quantity().map(Scalar::from),
            Self::Unit => Some(element.get_unit().clone().into()),
            Self::Side => Some(element.get_side().into()),
            Self::SecurityIds => {
                let ids = element.get_securityids();
                (!ids.is_empty()).then(|| ids.to_scalar())
            }
            Self::IsinCode => element
                .get_isincode()
                .and_then(|code| Isin::new(code).ok())
                .map(Scalar::Isin),
            Self::CfiCode => element.get_cficode().cloned().map(Scalar::from),
            Self::MicCode => element.get_miccode().cloned().map(Scalar::from),
            Self::LastPx => element.get_lastpx().map(Scalar::from),
            Self::LastQty => element.get_lastqty().map(Scalar::from),
            Self::AvgPx => element.get_avgpx().map(Scalar::from),
            Self::CumQty => element.get_cumqty().map(Scalar::from),
            Self::LeavesQty => element.get_leavesqty().map(Scalar::from),
            Self::PrevPx => element.get_prevpx().map(Scalar::from),
            Self::PrevQty => element.get_prevqty().map(Scalar::from),
            Self::SpotRate => element.get_spotrate().map(Scalar::from),
            Self::ForwardPoints => element.get_forwardpoints().map(Scalar::from),
            Self::BidPx => element.get_bidpx().map(Scalar::from),
            Self::BidQty => element.get_bidqty().map(Scalar::from),
            Self::BidCcy => element.get_bidccy().cloned().map(Scalar::Ccy),
            Self::AskPx => element.get_askpx().map(Scalar::from),
            Self::AskQty => element.get_askqty().map(Scalar::from),
            Self::AskCcy => element.get_askccy().cloned().map(Scalar::Ccy),
            Self::FxRates => {
                let rates = element.get_fxrates();
                (!rates.is_empty()).then(|| {
                    Scalar::from_mapping(
                        rates.iter().map(|(target, rate)| {
                            (Scalar::Ccy(target.clone()), Scalar::from(*rate))
                        }),
                    )
                    .ok()
                })?
            }
            Self::Ticker => element.get_ticker().map(Scalar::from),
            Self::Metadata => {
                let metadata = element.get_metadata();
                (!metadata.is_empty()).then(|| {
                    Scalar::from_mapping(metadata.iter().map(|(key, value)| {
                        (Scalar::from(key.as_str()), Scalar::from(value.as_str()))
                    }))
                    .ok()
                })?
            }
        }
    }

    /// Records a cell on `element`, leniently: a null clears an optional
    /// fact, and an incompatible value is ignored. The ISIN is a projection
    /// of the security identifiers: a cell fills an absent `ISIN` and a
    /// disagreeing one is ignored, as a null is - the strict door is the
    /// Arrow reader.
    pub fn record<E: Market + ?Sized>(self, element: &mut E, value: &Scalar) {
        let decimal = || Decimal::from_scalar(value);
        match self {
            Self::Price => match value {
                Scalar::Null => element.set_price(None),
                _ => {
                    if let Some(held) = decimal() {
                        element.set_price(Some(held));
                    }
                }
            },
            Self::Currency => {
                if let Some(held) = currency_of(value) {
                    element.set_currency(held);
                }
            }
            Self::Quantity => match value {
                Scalar::Null => element.set_quantity(None),
                _ => {
                    if let Some(held) = decimal() {
                        element.set_quantity(Some(held));
                    }
                }
            },
            Self::Unit => {
                let unit = match value {
                    Scalar::Unit(held) => Some(held.clone()),
                    Scalar::Null => Some(Unit::none()),
                    other => other.as_str().and_then(|text| Unit::new(text).ok()),
                };
                if let Some(unit) = unit {
                    element.set_unit(unit);
                }
            }
            Self::Side => {
                if let Some(held) = match value {
                    Scalar::Side(held) => Some(*held),
                    other => other
                        .as_i128()
                        .and_then(|code| i32::try_from(code).ok())
                        .and_then(Side::from_code)
                        .or_else(|| other.as_str().and_then(Side::from_spelling)),
                } {
                    element.set_side(held);
                }
            }
            Self::SecurityIds => {
                if let Some(ids) = match value {
                    Scalar::Null => Some(SecurityIds::default()),
                    other => SecurityIds::from_scalar(other).ok(),
                } {
                    let _ = element.set_securityids(ids);
                }
            }
            Self::IsinCode => {
                let code = match value {
                    Scalar::Isin(held) => Some(held.clone()),
                    other => other.as_str().and_then(|text| Isin::new(text).ok()),
                };
                if let Some(code) = code {
                    if element.get_isincode().is_none() {
                        if let Ok(id) = SecType::read("ISIN")
                            .and_then(|key| SecurityId::new(key, code.as_str()))
                        {
                            let _ = element.insert_securityid(id);
                        }
                    }
                }
            }
            Self::CfiCode => element.set_cficode(match value {
                Scalar::Cfi(held) => Some(held.clone()),
                Scalar::Null => None,
                other => other.as_str().and_then(|text| Cfi::new(text).ok()),
            }),
            Self::MicCode => element.set_miccode(match value {
                Scalar::Mic(held) => Some(held.clone()),
                Scalar::Null => None,
                other => other.as_str().and_then(|text| Mic::new(text).ok()),
            }),
            Self::LastPx => element.set_lastpx(decimal()),
            Self::LastQty => element.set_lastqty(decimal()),
            Self::AvgPx => element.set_avgpx(decimal()),
            Self::CumQty => element.set_cumqty(decimal()),
            Self::LeavesQty => element.set_leavesqty(decimal()),
            Self::PrevPx => element.set_prevpx(decimal()),
            Self::PrevQty => element.set_prevqty(decimal()),
            Self::SpotRate => element.set_spotrate(decimal()),
            Self::ForwardPoints => element.set_forwardpoints(decimal()),
            Self::BidPx => element.set_bidpx(decimal()),
            Self::BidQty => element.set_bidqty(decimal()),
            Self::BidCcy => element.set_bidccy(currency_of(value)),
            Self::AskPx => element.set_askpx(decimal()),
            Self::AskQty => element.set_askqty(decimal()),
            Self::AskCcy => element.set_askccy(currency_of(value)),
            Self::FxRates => element.set_fxrates(
                value
                    .as_mapping()
                    .map(|entries| {
                        entries
                            .iter()
                            .filter_map(|(target, rate)| {
                                Some((currency_of(target)?, Decimal::from_scalar(rate)?))
                            })
                            .collect::<FxRates>()
                    })
                    .unwrap_or_default(),
            ),
            Self::Ticker => element.set_ticker(value.as_str().map(SmolStr::new)),
            Self::Metadata => element.set_metadata(value.as_mapping().map(|entries| {
                entries
                    .iter()
                    .filter_map(|(key, value)| {
                        Some((SmolStr::new(key.as_str()?), SmolStr::new(value.as_str()?)))
                    })
                    .collect()
            })),
        }
    }
}

/// The `fxrates` column's datatype: a sorted `map<ccy, decimal>`, its keys
/// and its values required.
fn fxrates_datatype() -> DataType {
    let entries = StructType::from_unique_fields(vec![
        Field::new("key", DataType::Ccy, false),
        Field::new("value", DataType::Decimal, false),
    ]);
    DataType::map(
        Field::new("entries", DataType::Struct(entries), false),
        true,
    )
    .expect("a non-null key-value entries field is a map")
}

/// The currency a cell states, as the code or as text.
pub(super) fn currency_of(value: &Scalar) -> Option<Ccy> {
    match value {
        Scalar::Ccy(held) => Some(held.clone()),
        other => other.as_str().and_then(|text| Ccy::new(text).ok()),
    }
}

/// The time in force a cell states, as the code or as any spelling the
/// code set reads.
pub(super) fn tif_of(value: &Scalar) -> Option<TimeInForce> {
    match value {
        Scalar::TimeInForce(held) => Some(held.clone()),
        other => other.as_str().and_then(TimeInForce::from_spelling),
    }
}
