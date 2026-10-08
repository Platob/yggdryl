//! The five columns every operation on the market is stated in, beside the
//! market's thirty-five.
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

    /// What the column holds, for a catalog.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::OrdQty => "The quantity the operation asked for.",
            Self::TimeInForce => "How long the operation stands.",
            Self::Tradable => "Whether the operation trades; null where it states nothing.",
            Self::Identifiers => {
                "The operation's own identifiers, one per type, sorted by key: the type to its value, each with its lineage; what each source stated is side information in metadata under identifiers.src:type."
            }
            Self::PartyIds => {
                "The parties the operation names, one per role, sorted by key: accounts, traders, firms and users; what each source stated is side information in metadata under partyids.src:type."
            }
        }
    }

    /// The one datatype the column is built and read at.
    #[must_use]
    pub fn datatype(self) -> DataType {
        match self {
            Self::OrdQty => DataType::Decimal,
            Self::TimeInForce => DataType::timeinforce(),
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

    /// The column as a field, named, typed and nullable, with its display
    /// and description for a catalog.
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
    /// fact, and an incompatible value is ignored. A flag is read as every
    /// flag in the crate is ([`crate::boolean`]'s table), so `yes` and `0`
    /// are cells and `maybe` is incompatible.
    pub fn record<E: Operation + ?Sized>(self, operation: &mut E, value: &Scalar) {
        match self {
            Self::OrdQty => match value {
                Scalar::Null => operation.set_ordqty(None, true),
                _ => {
                    if let Some(held) = crate::Decimal::from_scalar(value) {
                        operation.set_ordqty(Some(held), true);
                    }
                }
            },
            Self::TimeInForce => match value {
                Scalar::Null => operation.set_timeinforce(None, true),
                _ => {
                    if let Some(held) =
                        <crate::TimeInForce as crate::EnumValue>::from_scalar_value(value)
                    {
                        operation.set_timeinforce(Some(held), true);
                    }
                }
            },
            Self::Tradable => match value {
                Scalar::Null => operation.set_tradable(None, true),
                _ => {
                    if let Some(held) = crate::boolean::bool_of(value) {
                        operation.set_tradable(Some(held), true);
                    }
                }
            },
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
