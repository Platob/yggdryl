//! The eight columns every market operation is stated in, beside the
//! market's nineteen.
//!
//! One column per fact [`MarketOperation`] adds, under one name and one
//! datatype each: the category, how long it stands, whether it can trade,
//! the three identifier maps as sorted `map<utf8, utf8>`, and the two lanes
//! as nullable structs.

use super::market_column::{currency_of, tif_of, unit_of};
use super::{Lane, MarketOperation};
use crate::idmap::IdMap;
use crate::{DataType, Decimal18, Field, Result, Scalar, StructType};

/// One column of the facts every market operation answers beside the
/// market's.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OperationColumn {
    /// The stable numeric market-operation category.
    MarketOperationId,
    /// How long the operation stands.
    TimeInForce,
    /// Whether the instrument can trade.
    Tradable,
    /// The accounts the operation is for, source key to account.
    AccountIds,
    /// The users the operation is by, source key to user.
    UserIds,
    /// The operation's own identifiers, source key to identifier.
    AltIds,
    /// The bid lane a quote states.
    Bid,
    /// The ask lane a quote states.
    Ask,
}

impl OperationColumn {
    /// Every operation column in canonical row order.
    pub const ALL: [Self; 8] = [
        Self::MarketOperationId,
        Self::TimeInForce,
        Self::Tradable,
        Self::AccountIds,
        Self::UserIds,
        Self::AltIds,
        Self::Bid,
        Self::Ask,
    ];

    /// The lane struct's children, in order.
    pub const LANE_FIELDS: [&'static str; 6] = [
        "price",
        "spotrate",
        "forwardpoints",
        "currency",
        "quantity",
        "unit",
    ];

    /// The column's name: the fact's, as the traits spell it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::MarketOperationId => "marketoperationid",
            Self::TimeInForce => "tif",
            Self::Tradable => "tradable",
            Self::AccountIds => "accountids",
            Self::UserIds => "userids",
            Self::AltIds => "altids",
            Self::Bid => "bid",
            Self::Ask => "ask",
        }
    }

    /// The column's display name.
    #[must_use]
    pub const fn display(self) -> &'static str {
        match self {
            Self::MarketOperationId => "Market Operation ID",
            Self::TimeInForce => "Time In Force",
            Self::Tradable => "Tradable",
            Self::AccountIds => "Account IDs",
            Self::UserIds => "User IDs",
            Self::AltIds => "Alternate IDs",
            Self::Bid => "Bid",
            Self::Ask => "Ask",
        }
    }

    /// The one datatype the column is built and read at.
    ///
    /// # Errors
    ///
    /// Returns an error when the lane struct cannot be built.
    pub fn datatype(self) -> Result<DataType> {
        Ok(match self {
            Self::MarketOperationId => DataType::Int32,
            Self::TimeInForce => DataType::TimeInForce,
            Self::Tradable => DataType::Boolean,
            Self::AccountIds | Self::UserIds | Self::AltIds => IdMap::dtype(),
            Self::Bid | Self::Ask => lane_datatype()?,
        })
    }

    /// Every operation column may be null: an operation states each of
    /// these only where it knows it.
    #[must_use]
    pub const fn nullable(self) -> bool {
        true
    }

    /// The column as a field, named, typed and displayed.
    ///
    /// # Errors
    ///
    /// Returns an error when the datatype or the display cannot be set.
    pub fn field(self) -> Result<Field> {
        let mut field = Field::new(self.name(), self.datatype()?, self.nullable());
        field.set_display(self.display())?;
        Ok(field)
    }

    /// Every column as a field, in [`Self::ALL`] order.
    ///
    /// # Errors
    ///
    /// Returns an error when a field cannot be built.
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

    /// The column's cell for `operation`: the fact it states, `None` where
    /// it states none.
    pub fn fact<E: MarketOperation + ?Sized>(self, operation: &E) -> Option<Scalar> {
        match self {
            Self::MarketOperationId => operation.get_marketoperationid().map(Scalar::from),
            Self::TimeInForce => operation.get_tif().cloned().map(Scalar::from),
            Self::Tradable => operation.get_tradable().map(Scalar::from),
            Self::AccountIds => map_fact(operation.get_accountids()),
            Self::UserIds => map_fact(operation.get_userids()),
            Self::AltIds => map_fact(operation.get_altids()),
            Self::Bid => operation.get_bid().map(lane_fact),
            Self::Ask => operation.get_ask().map(lane_fact),
        }
    }

    /// Records a cell on `operation`, leniently: a null clears an optional
    /// fact, and an incompatible value is ignored.
    pub fn record<E: MarketOperation + ?Sized>(self, operation: &mut E, value: &Scalar) {
        match self {
            Self::MarketOperationId => operation
                .set_marketoperationid(value.as_i128().and_then(|held| i32::try_from(held).ok())),
            Self::TimeInForce => operation.set_tif(tif_of(value)),
            Self::Tradable => operation.set_tradable(value.as_bool()),
            Self::AccountIds => {
                if let Some(ids) = map_of(value) {
                    let _ = operation.set_accountids(ids);
                }
            }
            Self::UserIds => {
                if let Some(ids) = map_of(value) {
                    let _ = operation.set_userids(ids);
                }
            }
            Self::AltIds => {
                if let Some(ids) = map_of(value) {
                    let _ = operation.set_altids(ids);
                }
            }
            Self::Bid => operation.set_bid(lane_of(value)),
            Self::Ask => operation.set_ask(lane_of(value)),
        }
    }
}

/// The lane struct: a nullable price, spot rate, forward points, currency,
/// quantity and unit.
///
/// # Errors
///
/// Returns an error when the struct cannot be built.
pub fn lane_datatype() -> Result<DataType> {
    let fields = vec![
        DataType::DECIMAL.nullable_field("price"),
        DataType::DECIMAL.nullable_field("spotrate"),
        DataType::DECIMAL.nullable_field("forwardpoints"),
        DataType::Ccy.nullable_field("currency"),
        DataType::DECIMAL.nullable_field("quantity"),
        DataType::Unit.nullable_field("unit"),
    ];
    Ok(DataType::from(StructType::from_fields(fields)?))
}

fn map_fact(map: &IdMap) -> Option<Scalar> {
    (!map.is_empty()).then(|| map.to_scalar())
}

fn map_of(value: &Scalar) -> Option<IdMap> {
    match value {
        Scalar::Null => Some(IdMap::new()),
        other => IdMap::from_scalar(other).ok(),
    }
}

/// A lane as the struct row its column holds.
pub fn lane_fact(lane: &Lane) -> Scalar {
    let decimal = |held: Option<Decimal18>| held.map_or(Scalar::Null, Scalar::from);
    Scalar::from_sequence([
        decimal(lane.price),
        decimal(lane.spotrate),
        decimal(lane.forwardpoints),
        lane.currency.clone().map_or(Scalar::Null, Scalar::from),
        decimal(lane.quantity),
        lane.unit.clone().map_or(Scalar::Null, Scalar::from),
    ])
}

/// The lane a struct row states; `None` for a null or a row of another
/// shape, and for a lane stating nothing.
pub fn lane_of(value: &Scalar) -> Option<Lane> {
    let rows = value.sequence_rows()?;
    if rows.len() != OperationColumn::LANE_FIELDS.len() {
        return None;
    }
    Lane {
        price: Decimal18::from_scalar(&rows[0]),
        spotrate: Decimal18::from_scalar(&rows[1]),
        forwardpoints: Decimal18::from_scalar(&rows[2]),
        currency: currency_of(&rows[3]),
        quantity: Decimal18::from_scalar(&rows[4]),
        unit: unit_of(&rows[5]),
    }
    .stated()
}

impl From<&Lane> for Scalar {
    fn from(lane: &Lane) -> Self {
        lane_fact(lane)
    }
}
