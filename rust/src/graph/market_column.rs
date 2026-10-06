//! The thirty-five columns every market element is stated in.
//!
//! One column per fact [`Market`] answers, under one name and one datatype
//! each, in one order, so every generated schema of a market - an
//! operation's row, a book's, a FIX message's - states the same columns
//! right after its element's and its event's, and a reader joins them
//! without a mapping.

use smol_str::SmolStr;

use super::{FxRates, Market};
use crate::{
    Ccy, Cfi, DataType, Decimal, Field, Isin, Mic, Result, Scalar, Side, StructType, TimeUnit,
    Timezone, Unit,
};
use crate::{IdKey, IdType, Identifier, Identifiers};

/// One column of the market facts every market element answers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MarketColumn {
    /// The category the element is filed under, which its holder stamps.
    MarketDataKind,
    /// The type of its kind the element is: its order, quote, trade or book
    /// entry type.
    MarketDataType,
    /// The price the element is about.
    Price,
    /// The price a stop order triggers at.
    StopPx,
    /// The currency it is priced in.
    Currency,
    /// The quantity it is about.
    Quantity,
    /// The part of the quantity shown to the market: an iceberg's peak.
    DisplayQty,
    /// The part of the quantity kept from the market: an iceberg's reserve.
    HiddenQty,
    /// The unit the quantity is counted in.
    Unit,
    /// The side it takes.
    Side,
    /// The security identifiers it names, key to code, sorted: a
    /// `map<utf8, utf8>` keyed as [`IdKey`] spells a key, the base key of
    /// each type its type alone.
    SecurityIds,
    /// The ISIN it names: the `ISIN` security identifier, projected.
    IsinCode,
    /// The detailed CFI classification.
    CfiCode,
    /// The market it trades on.
    MicCode,
    /// When it last executed: the latest execution clock its lifecycle
    /// reached, a nanosecond UTC clock.
    ExecUnix,
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
    /// How much was canceled.
    CxlQty,
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
    /// The strike price of the option the element is about.
    StrikePx,
    /// The free-form facts it carries, key to value, sorted.
    Metadata,
}

impl MarketColumn {
    /// Every market column in canonical row order: the category and the
    /// type first, then what the element is about - its prices, then its
    /// quantities - the instrument, the execution and the quote.
    pub const ALL: [Self; 35] = [
        Self::MarketDataKind,
        Self::MarketDataType,
        Self::Price,
        Self::StopPx,
        Self::Currency,
        Self::Quantity,
        Self::DisplayQty,
        Self::HiddenQty,
        Self::Unit,
        Self::Side,
        Self::SecurityIds,
        Self::IsinCode,
        Self::CfiCode,
        Self::MicCode,
        Self::ExecUnix,
        Self::LastPx,
        Self::LastQty,
        Self::AvgPx,
        Self::CumQty,
        Self::LeavesQty,
        Self::CxlQty,
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
        Self::StrikePx,
        Self::Metadata,
    ];

    /// The column's name: the fact's, as the traits spell it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::MarketDataKind => "marketdatakind",
            Self::MarketDataType => "marketdatatype",
            Self::Price => "price",
            Self::StopPx => "stoppx",
            Self::Currency => "currency",
            Self::Quantity => "quantity",
            Self::DisplayQty => "displayqty",
            Self::HiddenQty => "hiddenqty",
            Self::Unit => "unit",
            Self::Side => "side",
            Self::SecurityIds => "securityids",
            Self::IsinCode => "isincode",
            Self::CfiCode => "cficode",
            Self::MicCode => "miccode",
            Self::ExecUnix => "execunix",
            Self::LastPx => "lastpx",
            Self::LastQty => "lastqty",
            Self::AvgPx => "avgpx",
            Self::CumQty => "cumqty",
            Self::LeavesQty => "leavesqty",
            Self::CxlQty => "cxlqty",
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
            Self::StrikePx => "strikepx",
            Self::Metadata => "metadata",
        }
    }

    /// The column's display name.
    #[must_use]
    pub const fn display(self) -> &'static str {
        match self {
            Self::MarketDataKind => "Market Data Kind",
            Self::MarketDataType => "Market Data Type",
            Self::Price => "Price",
            Self::StopPx => "Stop Price",
            Self::Currency => "Currency",
            Self::Quantity => "Quantity",
            Self::DisplayQty => "Display Quantity",
            Self::HiddenQty => "Hidden Quantity",
            Self::Unit => "Unit",
            Self::Side => "Side",
            Self::SecurityIds => "Security IDs",
            Self::IsinCode => "ISIN Code",
            Self::CfiCode => "CFI Code",
            Self::MicCode => "MIC Code",
            Self::ExecUnix => "Execution Time",
            Self::LastPx => "Last Price",
            Self::LastQty => "Last Quantity",
            Self::AvgPx => "Average Price",
            Self::CumQty => "Cumulative Quantity",
            Self::LeavesQty => "Leaves Quantity",
            Self::CxlQty => "Canceled Quantity",
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
            Self::StrikePx => "Strike Price",
            Self::Metadata => "Metadata",
        }
    }

    /// What the column holds, for a catalog.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::MarketDataKind => {
                "The category the element is filed under: an order, a quote, an execution, a trade, a book or another market data kind; UNKN where none is stated."
            }
            Self::MarketDataType => {
                "The type of its kind the element is: its order, quote, trade or book entry type; UNKN where none is stated."
            }
            Self::Price => "The price the element is about.",
            Self::StopPx => "The price a stop order triggers at.",
            Self::Currency => "The currency the element is priced in; XXX where it states none.",
            Self::Quantity => "The quantity the element is about.",
            Self::DisplayQty => "The part of the quantity shown to the market: an iceberg's peak.",
            Self::HiddenQty => {
                "The part of the quantity kept from the market: an iceberg's reserve."
            }
            Self::Unit => "The unit the quantity is counted in; empty where it states none.",
            Self::Side => "The side the element takes; UNKN where it states none.",
            Self::SecurityIds => {
                "The security identifiers the element names, one per type, sorted by key: the type to its value, the type's answer; what each source stated of a type is side information in metadata under securityids.src:type."
            }
            Self::IsinCode => "The ISIN the element names: its isin security identifier.",
            Self::CfiCode => "The detailed CFI classification of the instrument.",
            Self::MicCode => "The market the instrument trades on, as its MIC.",
            Self::ExecUnix => {
                "When the element last executed: the latest execution its lifecycle reached, in UTC to the nanosecond."
            }
            Self::LastPx => "The last executed price the element reports.",
            Self::LastQty => "The last executed quantity the element reports.",
            Self::AvgPx => "The average price of what the element traded.",
            Self::CumQty => "How much the element has traded.",
            Self::LeavesQty => "How much the element has left to trade.",
            Self::CxlQty => "How much of the element was canceled.",
            Self::PrevPx => "The price the step before the element settled on.",
            Self::PrevQty => "The quantity the step before the element settled on.",
            Self::SpotRate => "The spot part of an FX forward price.",
            Self::ForwardPoints => "The forward points of an FX forward price.",
            Self::BidPx => "The bid price the element states.",
            Self::BidQty => "The quantity bid.",
            Self::BidCcy => "The currency the bid is stated in.",
            Self::AskPx => "The ask price the element states.",
            Self::AskQty => "The quantity offered.",
            Self::AskCcy => "The currency the ask is stated in.",
            Self::FxRates => {
                "The FX rates the element states: a target currency to the rate an amount is divided by."
            }
            Self::Ticker => "The ticker the instrument goes by.",
            Self::StrikePx => "The strike price of the option the element is about.",
            Self::Metadata => {
                "The free-form facts the element carries beside the side information of its identifier maps - what each source stated of a type, under the map's name and the key, securityids.src:type - sorted by key."
            }
        }
    }

    /// The one datatype the column is built and read at: the
    /// [`MarketDataKind`](crate::MarketDataKind) code for the category and the
    /// [`MarketDataType`](crate::MarketDataType) code for the type, the crate's
    /// decimal for every price and quantity, each code's own leaf, the
    /// execution clock at nanoseconds UTC, a sorted
    /// `map<utf8, utf8>` for the identifiers and the metadata, a sorted
    /// `map<ccy, decimal>` for the rates - keys and values required - and
    /// `utf8` for the ticker.
    #[must_use]
    pub fn datatype(self) -> DataType {
        match self {
            Self::MarketDataKind => DataType::MarketDataKind,
            Self::MarketDataType => DataType::MarketDataType,
            Self::Price
            | Self::StopPx
            | Self::StrikePx
            | Self::Quantity
            | Self::DisplayQty
            | Self::HiddenQty
            | Self::CxlQty
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
            Self::SecurityIds => Identifiers::dtype(),
            Self::IsinCode => DataType::Isin,
            Self::CfiCode => DataType::Cfi,
            Self::MicCode => DataType::Mic,
            Self::ExecUnix => DataType::DateTime64 {
                unit: TimeUnit::Nanosecond,
                timezone: Timezone::UTC,
            },
            Self::FxRates => fxrates_datatype(),
            Self::Ticker => DataType::utf8(),
            Self::Metadata => DataType::map_of(DataType::utf8(), DataType::utf8(), true)
                .expect("a sorted utf8 map is a datatype"),
        }
    }

    /// Whether a row may leave the column null: never for the category,
    /// the type, the currency, the unit and the side, which every market
    /// element states, if only as nothing - `UNKN`, `UNKN`, `XXX`, the empty
    /// unit, `UNKN`.
    #[must_use]
    pub const fn nullable(self) -> bool {
        !matches!(
            self,
            Self::MarketDataKind | Self::MarketDataType | Self::Currency | Self::Unit | Self::Side
        )
    }

    /// The column as a field, named, typed and nullable as its fact is, with
    /// its display and description for a catalog.
    ///
    /// # Errors
    ///
    /// Returns an error when the display or the description cannot be set.
    pub fn field(self) -> Result<Field> {
        let mut field = Field::new(self.name(), self.datatype(), self.nullable());
        field.set_display(self.display())?;
        field.set_description(self.description())?;
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
            Self::MarketDataKind => Some(Scalar::MarketDataKind(element.marketdatakind())),
            Self::MarketDataType => Some(Scalar::MarketDataType(element.get_marketdatatype())),
            Self::Price => element.get_price().map(Scalar::from),
            Self::StopPx => element.get_stoppx().map(Scalar::from),
            Self::StrikePx => element.get_strikepx().map(Scalar::from),
            Self::DisplayQty => element.get_displayqty().map(Scalar::from),
            Self::HiddenQty => element.get_hiddenqty().map(Scalar::from),
            Self::CxlQty => element.get_cxlqty().map(Scalar::from),
            Self::Currency => Some(element.get_currency().clone().into()),
            Self::Quantity => element.get_quantity().map(Scalar::from),
            Self::Unit => Some(element.get_unit().clone().into()),
            Self::Side => Some(element.get_side().into()),
            Self::SecurityIds => {
                let ids = element.get_securityids();
                (!ids.is_empty()).then(|| ids.into_scalar())
            }
            Self::IsinCode => element
                .get_isincode()
                .and_then(|code| Isin::new(code).ok())
                .map(Scalar::Isin),
            Self::CfiCode => element.get_cficode().cloned().map(Scalar::from),
            Self::MicCode => element.get_miccode().cloned().map(Scalar::from),
            Self::ExecUnix => element.get_execunix().and_then(|unix| {
                Scalar::datetime64(unix, TimeUnit::Nanosecond, Timezone::UTC).ok()
            }),
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
    /// Arrow reader. The category is the holder's own, which [`Market`]
    /// states no setter for - an order is filed `ORDR` because it is an
    /// order - so a cell of it records nothing.
    pub fn record<E: Market + ?Sized>(self, element: &mut E, value: &Scalar) {
        let decimal = || Decimal::from_scalar(value);
        match self {
            Self::MarketDataKind => {}
            Self::MarketDataType => element.set_marketdatatype(
                <crate::MarketDataType as crate::EnumValue>::from_scalar_value(value)
                    .unwrap_or_default(),
                true,
            ),
            Self::Price => match value {
                Scalar::Null => element.set_price(None, true),
                _ => {
                    if let Some(held) = decimal() {
                        element.set_price(Some(held), true);
                    }
                }
            },
            Self::Currency => {
                if let Some(held) = currency_of(value) {
                    element.set_currency(held, true);
                }
            }
            Self::Quantity => match value {
                Scalar::Null => element.set_quantity(None, true),
                _ => {
                    if let Some(held) = decimal() {
                        element.set_quantity(Some(held), true);
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
                    element.set_unit(unit, true);
                }
            }
            Self::Side => {
                if let Some(held) = <Side as crate::EnumValue>::from_scalar_value(value) {
                    element.set_side(held, true);
                }
            }
            Self::SecurityIds => {
                if let Some(ids) = match value {
                    Scalar::Null => Some(Identifiers::new()),
                    other => Identifiers::from_scalar(other).ok(),
                } {
                    let _ = element.set_securityids(ids, true);
                }
            }
            Self::IsinCode => {
                let code = match value {
                    Scalar::Isin(held) => Some(held.clone()),
                    other => other.as_str().and_then(|text| Isin::new(text).ok()),
                };
                // A projection of `securityids`: it fills an absent ISIN,
                // replaces one ranking below it and leaves another real one
                // standing - what `insert` decides.
                if let Some(code) = code
                    && let Ok(id) = Identifier::new(IdKey::base(IdType::Isin), code.as_str())
                {
                    let _ = element.insert_securityid(id);
                }
            }
            Self::CfiCode => element.set_cficode(
                match value {
                    Scalar::Cfi(held) => Some(held.clone()),
                    Scalar::Null => None,
                    other => other.as_str().and_then(|text| Cfi::new(text).ok()),
                },
                true,
            ),
            Self::MicCode => element.set_miccode(
                match value {
                    Scalar::Mic(held) => Some(held.clone()),
                    Scalar::Null => None,
                    other => other.as_str().and_then(|text| Mic::new(text).ok()),
                },
                true,
            ),
            Self::ExecUnix => {
                element.set_execunix(value.temporal_count_at(TimeUnit::Nanosecond), true);
            }
            Self::StopPx => element.set_stoppx(decimal(), true),
            Self::StrikePx => element.set_strikepx(decimal(), true),
            Self::DisplayQty => element.set_displayqty(decimal(), true),
            Self::HiddenQty => element.set_hiddenqty(decimal(), true),
            Self::CxlQty => element.set_cxlqty(decimal(), true),
            Self::LastPx => element.set_lastpx(decimal(), true),
            Self::LastQty => element.set_lastqty(decimal(), true),
            Self::AvgPx => element.set_avgpx(decimal(), true),
            Self::CumQty => element.set_cumqty(decimal(), true),
            Self::LeavesQty => element.set_leavesqty(decimal(), true),
            Self::PrevPx => element.set_prevpx(decimal(), true),
            Self::PrevQty => element.set_prevqty(decimal(), true),
            Self::SpotRate => element.set_spotrate(decimal(), true),
            Self::ForwardPoints => element.set_forwardpoints(decimal(), true),
            Self::BidPx => element.set_bidpx(decimal(), true),
            Self::BidQty => element.set_bidqty(decimal(), true),
            Self::BidCcy => element.set_bidccy(currency_of(value), true),
            Self::AskPx => element.set_askpx(decimal(), true),
            Self::AskQty => element.set_askqty(decimal(), true),
            Self::AskCcy => element.set_askccy(currency_of(value), true),
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
                true,
            ),
            Self::Ticker => element.set_ticker(value.as_str().map(SmolStr::new), true),
            Self::Metadata => element.set_metadata(
                value.as_mapping().map(|entries| {
                    entries
                        .iter()
                        .filter_map(|(key, value)| {
                            Some((SmolStr::new(key.as_str()?), SmolStr::new(value.as_str()?)))
                        })
                        .collect()
                }),
                true,
            ),
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
