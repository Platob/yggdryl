//! The five columns every operation on the market is stated in, beside the
//! market's thirty-four.
//!
//! One column per fact [`Operation`] adds, under one name and one
//! datatype each: what it ordered, how long it stands, whether it can
//! trade, and its alternate identifiers and the parties it names, each a
//! sorted `map<utf8, utf8>` from an [`IdKey`](crate::IdKey)'s spelling to its
//! value ([`Identifiers::dtype`]).

use super::Operation;
use crate::Identifiers;
use crate::{DataType, Field, Result, Scalar};

/// One column of the facts every operation on the market answers beside the
/// market's.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OperationColumn {
    /// The quantity the operation ordered.
    OrdQty,
    /// How long the operation stands.
    TimeInForce,
    /// Whether the instrument can trade.
    Tradable,
    /// The operation's own identifiers, each with its lineage.
    Identifiers,
    /// The parties the operation names: accounts, traders, firms, users.
    PartyIds,
}

impl OperationColumn {
    /// Every operation column in canonical row order.
    pub const ALL: [Self; 5] = [
        Self::OrdQty,
        Self::TimeInForce,
        Self::Tradable,
        Self::Identifiers,
        Self::PartyIds,
    ];

    /// The column's name: the fact's, as the traits spell it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::OrdQty => "ordqty",
            Self::TimeInForce => "timeinforce",
            Self::Tradable => "tradable",
            Self::Identifiers => "identifiers",
            Self::PartyIds => "partyids",
        }
    }

    /// The column's display name.
    #[must_use]
    pub const fn display(self) -> &'static str {
        match self {
            Self::OrdQty => "Order Quantity",
            Self::TimeInForce => "Time In Force",
            Self::Tradable => "Tradable",
            Self::Identifiers => "Identifiers",
            Self::PartyIds => "Party IDs",
        }
    }

    /// The one datatype the column is built and read at.
    #[must_use]
    pub fn datatype(self) -> DataType {
        match self {
            Self::OrdQty => DataType::Decimal,
            Self::TimeInForce => DataType::TimeInForce,
            Self::Tradable => DataType::Boolean,
            Self::Identifiers | Self::PartyIds => Identifiers::dtype(),
        }
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
    pub fn fact<E: Operation + ?Sized>(self, operation: &E) -> Option<Scalar> {
        match self {
            Self::OrdQty => operation.get_ordqty().map(Scalar::from),
            Self::TimeInForce => operation.get_timeinforce().cloned().map(Scalar::from),
            Self::Tradable => operation.get_tradable().map(Scalar::from),
            Self::Identifiers => ids_fact(operation.get_identifiers()),
            Self::PartyIds => ids_fact(operation.get_partyids()),
        }
    }

    /// Records a cell on `operation`, leniently: a null clears an optional
    /// fact, and an incompatible value is ignored.
    pub fn record<E: Operation + ?Sized>(self, operation: &mut E, value: &Scalar) {
        match self {
            Self::OrdQty => operation.set_ordqty(crate::Decimal::from_scalar(value), true),
            Self::TimeInForce => operation.set_timeinforce(
                <crate::TimeInForce as crate::EnumValue>::from_scalar_value(value),
                true,
            ),
            Self::Tradable => operation.set_tradable(value.as_bool(), true),
            Self::Identifiers => {
                if let Some(ids) = ids_of(value) {
                    let _ = operation.set_identifiers(ids, true);
                }
            }
            Self::PartyIds => {
                if let Some(ids) = ids_of(value) {
                    let _ = operation.set_partyids(ids, true);
                }
            }
        }
    }
}

fn ids_fact(map: &Identifiers) -> Option<Scalar> {
    (!map.is_empty()).then(|| map.into_scalar())
}

fn ids_of(value: &Scalar) -> Option<Identifiers> {
    match value {
        Scalar::Null => Some(Identifiers::new()),
        other => Identifiers::from_scalar(other).ok(),
    }
}
