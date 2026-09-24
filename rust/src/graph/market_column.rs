//! The canonical columns of a market element, one per fact
//! [`MarketElement`] answers.

use crate::{
    BloombergCode, Ccy, CfiCode, CusipCode, DataType, Decimal18, FIGICode, Field, IsinCode,
    MicCode, Result, Scalar, SedolCode, Side,
};

use super::MarketElement;

/// One column of the market facts every market element answers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MarketColumn {
    MarketOperationId,
    Price,
    Currency,
    Quantity,
    Unit,
    Side,
    IsinCode,
    CusipCode,
    SedolCode,
    BloombergCode,
    FigiCode,
    CfiCode,
    MicCode,
    LastPx,
    LastQty,
    AvgPx,
    CumQty,
    LeavesQty,
    TimeInForce,
    Tradable,
    SymbolTicker,
    PrevPx,
    PrevQty,
    BidPx,
    BidCurrency,
    BidQty,
    BidUnit,
    AskPx,
    AskCurrency,
    AskQty,
    AskUnit,
}

impl MarketColumn {
    /// Every market column in canonical row order.
    pub const ALL: [Self; 31] = [
        Self::MarketOperationId,
        Self::Price,
        Self::Currency,
        Self::Quantity,
        Self::Unit,
        Self::Side,
        Self::IsinCode,
        Self::CusipCode,
        Self::SedolCode,
        Self::BloombergCode,
        Self::FigiCode,
        Self::CfiCode,
        Self::MicCode,
        Self::LastPx,
        Self::LastQty,
        Self::AvgPx,
        Self::CumQty,
        Self::LeavesQty,
        Self::TimeInForce,
        Self::Tradable,
        Self::SymbolTicker,
        Self::PrevPx,
        Self::PrevQty,
        Self::BidPx,
        Self::BidCurrency,
        Self::BidQty,
        Self::BidUnit,
        Self::AskPx,
        Self::AskCurrency,
        Self::AskQty,
        Self::AskUnit,
    ];

    /// The trait fact's canonical column name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::MarketOperationId => "marketoperationid",
            Self::Price => "price",
            Self::Currency => "currency",
            Self::Quantity => "quantity",
            Self::Unit => "unit",
            Self::Side => "side",
            Self::IsinCode => "isincode",
            Self::CusipCode => "cusipcode",
            Self::SedolCode => "sedolcode",
            Self::BloombergCode => "bloombergcode",
            Self::FigiCode => "figicode",
            Self::CfiCode => "cficode",
            Self::MicCode => "miccode",
            Self::LastPx => "lastpx",
            Self::LastQty => "lastqty",
            Self::AvgPx => "avgpx",
            Self::CumQty => "cumqty",
            Self::LeavesQty => "leavesqty",
            Self::TimeInForce => "tif",
            Self::Tradable => "tradable",
            Self::SymbolTicker => "symbolticker",
            Self::PrevPx => "prevpx",
            Self::PrevQty => "prevqty",
            Self::BidPx => "bidpx",
            Self::BidCurrency => "bidcurrency",
            Self::BidQty => "bidqty",
            Self::BidUnit => "bidunit",
            Self::AskPx => "askpx",
            Self::AskCurrency => "askcurrency",
            Self::AskQty => "askqty",
            Self::AskUnit => "askunit",
        }
    }

    /// The display name used by generated schemas.
    #[must_use]
    pub const fn display(self) -> &'static str {
        match self {
            Self::MarketOperationId => "Market Operation ID",
            Self::Price => "Price",
            Self::Currency => "Currency",
            Self::Quantity => "Quantity",
            Self::Unit => "Unit",
            Self::Side => "Side",
            Self::IsinCode => "ISIN",
            Self::CusipCode => "CUSIP",
            Self::SedolCode => "SEDOL",
            Self::BloombergCode => "Bloomberg Code",
            Self::FigiCode => "FIGI",
            Self::CfiCode => "CFI",
            Self::MicCode => "MIC",
            Self::LastPx => "Last Price",
            Self::LastQty => "Last Quantity",
            Self::AvgPx => "Average Price",
            Self::CumQty => "Cumulative Quantity",
            Self::LeavesQty => "Leaves Quantity",
            Self::TimeInForce => "Time In Force",
            Self::Tradable => "Tradable",
            Self::SymbolTicker => "Symbol",
            Self::PrevPx => "Previous Price",
            Self::PrevQty => "Previous Quantity",
            Self::BidPx => "Bid Price",
            Self::BidCurrency => "Bid Currency",
            Self::BidQty => "Bid Quantity",
            Self::BidUnit => "Bid Unit",
            Self::AskPx => "Ask Price",
            Self::AskCurrency => "Ask Currency",
            Self::AskQty => "Ask Quantity",
            Self::AskUnit => "Ask Unit",
        }
    }

    /// The exact datatype of this market fact.
    pub fn datatype(self) -> DataType {
        match self {
            Self::MarketOperationId => DataType::Int32,
            Self::Price
            | Self::Quantity
            | Self::LastPx
            | Self::LastQty
            | Self::AvgPx
            | Self::CumQty
            | Self::LeavesQty
            | Self::PrevPx
            | Self::PrevQty
            | Self::BidPx
            | Self::BidQty
            | Self::AskPx
            | Self::AskQty => DataType::DECIMAL,
            Self::Currency | Self::BidCurrency | Self::AskCurrency => DataType::Ccy,
            Self::Side => DataType::Side,
            Self::IsinCode => DataType::IsinCode,
            Self::CusipCode => DataType::CusipCode,
            Self::SedolCode => DataType::SedolCode,
            Self::BloombergCode => DataType::BloombergCode,
            Self::FigiCode => DataType::FIGICode,
            Self::CfiCode => DataType::CfiCode,
            Self::MicCode => DataType::MicCode,
            Self::Tradable => DataType::Boolean,
            Self::Unit | Self::TimeInForce | Self::SymbolTicker | Self::BidUnit | Self::AskUnit => {
                DataType::utf8()
            }
        }
    }

    /// Whether the fact may be absent in a row.
    #[must_use]
    pub const fn nullable(self) -> bool {
        !matches!(
            self,
            Self::Price | Self::Currency | Self::Quantity | Self::Unit | Self::Side
        )
    }

    /// The column as a schema field.
    pub fn field(self) -> Result<Field> {
        let mut field = Field::new(self.name(), self.datatype(), self.nullable());
        field.set_display(self.display())?;
        Ok(field)
    }

    /// Every market field in [`Self::ALL`] order.
    pub fn fields() -> Result<Vec<Field>> {
        Self::ALL.into_iter().map(Self::field).collect()
    }

    /// The column one case-insensitive name spells.
    #[must_use]
    pub fn of_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|column| crate::folds_equal(column.name(), name))
    }

    /// What an element states under this column.
    pub fn fact<E: MarketElement + ?Sized>(self, element: &E) -> Option<Scalar> {
        match self {
            Self::MarketOperationId => element.get_marketoperationid().map(Scalar::from),
            Self::Price => Some(element.get_price().into()),
            Self::Currency => Some(element.get_currency().clone().into()),
            Self::Quantity => Some(element.get_quantity().into()),
            Self::Unit => Some(Scalar::from(element.get_unit())),
            Self::Side => Some((*element.get_side()).into()),
            Self::IsinCode => element.get_isincode().cloned().map(Scalar::from),
            Self::CusipCode => element.get_cusipcode().cloned().map(Scalar::from),
            Self::SedolCode => element.get_sedolcode().cloned().map(Scalar::from),
            Self::BloombergCode => element.get_bloombergcode().cloned().map(Scalar::from),
            Self::FigiCode => element.get_figicode().cloned().map(Scalar::from),
            Self::CfiCode => element.get_cficode().cloned().map(Scalar::from),
            Self::MicCode => element.get_miccode().cloned().map(Scalar::from),
            Self::LastPx => element.get_lastpx().map(Scalar::from),
            Self::LastQty => element.get_lastqty().map(Scalar::from),
            Self::AvgPx => element.get_avgpx().map(Scalar::from),
            Self::CumQty => element.get_cumqty().map(Scalar::from),
            Self::LeavesQty => element.get_leavesqty().map(Scalar::from),
            Self::TimeInForce => element.get_tif().map(Scalar::from),
            Self::Tradable => element.get_tradable().map(Scalar::from),
            Self::SymbolTicker => element.get_symbolticker().map(Scalar::from),
            Self::PrevPx => element.get_prevpx().map(Scalar::from),
            Self::PrevQty => element.get_prevqty().map(Scalar::from),
            Self::BidPx => element.get_bidpx().map(Scalar::from),
            Self::BidCurrency => element.get_bidcurrency().cloned().map(Scalar::from),
            Self::BidQty => element.get_bidqty().map(Scalar::from),
            Self::BidUnit => element.get_bidunit().map(Scalar::from),
            Self::AskPx => element.get_askpx().map(Scalar::from),
            Self::AskCurrency => element.get_askcurrency().cloned().map(Scalar::from),
            Self::AskQty => element.get_askqty().map(Scalar::from),
            Self::AskUnit => element.get_askunit().map(Scalar::from),
        }
    }

    /// Records one typed cell through the market trait. A null clears an
    /// optional fact; an incompatible value leaves the element unchanged.
    pub fn record<E: MarketElement + ?Sized>(self, element: &mut E, value: &Scalar) {
        macro_rules! code {
            ($variant:ident, $kind:ty, $set:ident) => {
                element.$set(match value {
                    Scalar::$variant(held) => Some(held.clone()),
                    Scalar::Null => None,
                    other => other.as_str().and_then(|text| <$kind>::new(text).ok()),
                })
            };
        }
        let decimal = || Decimal18::from_scalar(value);
        match self {
            Self::MarketOperationId => element
                .set_marketoperationid(value.as_i128().and_then(|held| i32::try_from(held).ok())),
            Self::Price => {
                if let Some(held) = decimal() {
                    element.set_price(held);
                }
            }
            Self::Currency => {
                if let Some(held) = match value {
                    Scalar::Ccy(held) => Some(held.clone()),
                    other => other.as_str().and_then(|text| Ccy::new(text).ok()),
                } {
                    element.set_currency(held);
                }
            }
            Self::Quantity => {
                if let Some(held) = decimal() {
                    element.set_quantity(held);
                }
            }
            Self::Unit => {
                if let Some(held) = value.as_str() {
                    element.set_unit(held.to_owned());
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
            Self::IsinCode => code!(IsinCode, IsinCode, set_isincode),
            Self::CusipCode => code!(CusipCode, CusipCode, set_cusipcode),
            Self::SedolCode => code!(SedolCode, SedolCode, set_sedolcode),
            Self::BloombergCode => code!(BloombergCode, BloombergCode, set_bloombergcode),
            Self::FigiCode => code!(FIGICode, FIGICode, set_figicode),
            Self::CfiCode => code!(CfiCode, CfiCode, set_cficode),
            Self::MicCode => code!(MicCode, MicCode, set_miccode),
            Self::LastPx => element.set_lastpx(decimal()),
            Self::LastQty => element.set_lastqty(decimal()),
            Self::AvgPx => element.set_avgpx(decimal()),
            Self::CumQty => element.set_cumqty(decimal()),
            Self::LeavesQty => element.set_leavesqty(decimal()),
            Self::TimeInForce => element.set_tif(value.as_str().map(str::to_owned)),
            Self::Tradable => element.set_tradable(value.as_bool()),
            Self::SymbolTicker => element.set_symbolticker(value.as_str().map(str::to_owned)),
            Self::PrevPx => element.set_prevpx(decimal()),
            Self::PrevQty => element.set_prevqty(decimal()),
            Self::BidPx => element.set_bidpx(decimal()),
            Self::BidCurrency => element.set_bidcurrency(currency_of(value)),
            Self::BidQty => element.set_bidqty(decimal()),
            Self::BidUnit => element.set_bidunit(value.as_str().map(str::to_owned)),
            Self::AskPx => element.set_askpx(decimal()),
            Self::AskCurrency => element.set_askcurrency(currency_of(value)),
            Self::AskQty => element.set_askqty(decimal()),
            Self::AskUnit => element.set_askunit(value.as_str().map(str::to_owned)),
        }
    }
}

fn currency_of(value: &Scalar) -> Option<Ccy> {
    match value {
        Scalar::Ccy(held) => Some(held.clone()),
        other => other.as_str().and_then(|text| Ccy::new(text).ok()),
    }
}
