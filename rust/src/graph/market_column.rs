//! The nineteen columns every market element is stated in.
//!
//! One column per fact [`Market`] answers, under one name and one datatype
//! each, in one order, so every generated schema of a market - an
//! operation's row, a book's, a side's - states the same columns and a
//! reader joins them without a mapping.

use smol_str::SmolStr;

use super::Market;
use crate::securityid::SecurityIds;
use crate::{
    Ccy, CfiCode, DataType, Decimal18, Field, MicCode, Result, Scalar, Side, TimeInForce, Unit,
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
    /// The detailed CFI classification.
    CfiCode,
    /// The market it trades on.
    MicCode,
    /// The price of the last trade it reports.
    LastPx,
    /// The quantity of the last trade it reports.
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
    /// The ticker the instrument goes by.
    Ticker,
    /// The free-form facts it carries, key to value, sorted.
    Metadata,
}

impl MarketColumn {
    /// Every market column in canonical row order.
    pub const ALL: [Self; 19] = [
        Self::Price,
        Self::Currency,
        Self::Quantity,
        Self::Unit,
        Self::Side,
        Self::SecurityIds,
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
            Self::Ticker => "Ticker",
            Self::Metadata => "Metadata",
        }
    }

    /// The one datatype the column is built and read at: the crate's
    /// decimal for every price and quantity, each code's own leaf, a sorted
    /// `map<utf8, utf8>` for the identifiers and the metadata, and `utf8`
    /// for the ticker.
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
            | Self::ForwardPoints => DataType::DECIMAL,
            Self::Currency => DataType::Ccy,
            Self::Unit => DataType::Unit,
            Self::Side => DataType::Side,
            Self::SecurityIds => SecurityIds::dtype(),
            Self::CfiCode => DataType::CfiCode,
            Self::MicCode => DataType::MicCode,
            Self::Ticker => DataType::utf8(),
            Self::Metadata => DataType::map_of(DataType::utf8(), DataType::utf8(), true)
                .expect("a sorted utf8 map is a datatype"),
        }
    }

    /// Whether a row may leave the column null: never for the price, the
    /// currency, the quantity, the unit and the side, which every market
    /// element states, if only as nothing.
    #[must_use]
    pub const fn nullable(self) -> bool {
        !matches!(
            self,
            Self::Price | Self::Currency | Self::Quantity | Self::Unit | Self::Side
        )
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
            Self::Price => Some(element.get_price().into()),
            Self::Currency => Some(element.get_currency().clone().into()),
            Self::Quantity => Some(element.get_quantity().into()),
            Self::Unit => Some(element.get_unit().clone().into()),
            Self::Side => Some(element.get_side().into()),
            Self::SecurityIds => {
                let ids = element.get_securityids();
                (!ids.is_empty()).then(|| ids.to_scalar())
            }
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
    /// fact, and an incompatible value is ignored.
    pub fn record<E: Market + ?Sized>(self, element: &mut E, value: &Scalar) {
        let decimal = || Decimal18::from_scalar(value);
        match self {
            Self::Price => {
                if let Some(held) = decimal() {
                    element.set_price(held);
                }
            }
            Self::Currency => {
                if let Some(held) = currency_of(value) {
                    element.set_currency(held);
                }
            }
            Self::Quantity => {
                if let Some(held) = decimal() {
                    element.set_quantity(held);
                }
            }
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
                    other => other.as_str().and_then(Side::from_spelling),
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
            Self::CfiCode => element.set_cficode(match value {
                Scalar::CfiCode(held) => Some(held.clone()),
                Scalar::Null => None,
                other => other.as_str().and_then(|text| CfiCode::new(text).ok()),
            }),
            Self::MicCode => element.set_miccode(match value {
                Scalar::MicCode(held) => Some(held.clone()),
                Scalar::Null => None,
                other => other.as_str().and_then(|text| MicCode::new(text).ok()),
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

/// The currency a cell states, as the code or as text.
pub(super) fn currency_of(value: &Scalar) -> Option<Ccy> {
    match value {
        Scalar::Ccy(held) => Some(held.clone()),
        other => other.as_str().and_then(|text| Ccy::new(text).ok()),
    }
}

/// The unit a cell states, as the code or as text; `None` for a null.
pub(super) fn unit_of(value: &Scalar) -> Option<Unit> {
    match value {
        Scalar::Unit(held) => Some(held.clone()),
        other => other.as_str().and_then(|text| Unit::new(text).ok()),
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
