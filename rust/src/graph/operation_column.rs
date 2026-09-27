//! The three columns every operation on the market is stated in, beside the
//! market's twenty-seven.
//!
//! One column per fact [`Operation`] adds, under one name and one
//! datatype each: how long it stands, whether it can trade, and its
//! alternate identifiers as a sorted `map<utf8, utf8>`.

use super::Operation;
use super::market_column::tif_of;
use crate::idmap::IdMap;
use crate::{DataType, Field, Result, Scalar};

/// One column of the facts every operation on the market answers beside the
/// market's.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OperationColumn {
    /// How long the operation stands.
    TimeInForce,
    /// Whether the instrument can trade.
    Tradable,
    /// The operation's own identifiers, source key to identifier.
    AltIds,
}

impl OperationColumn {
    /// Every operation column in canonical row order.
    pub const ALL: [Self; 3] = [Self::TimeInForce, Self::Tradable, Self::AltIds];

    /// The column's name: the fact's, as the traits spell it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::TimeInForce => "tif",
            Self::Tradable => "tradable",
            Self::AltIds => "altids",
        }
    }

    /// The column's display name.
    #[must_use]
    pub const fn display(self) -> &'static str {
        match self {
            Self::TimeInForce => "Time In Force",
            Self::Tradable => "Tradable",
            Self::AltIds => "Alternate IDs",
        }
    }

    /// The one datatype the column is built and read at.
    #[must_use]
    pub fn datatype(self) -> DataType {
        match self {
            Self::TimeInForce => DataType::TimeInForce,
            Self::Tradable => DataType::Boolean,
            Self::AltIds => IdMap::dtype(),
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
            Self::TimeInForce => operation.get_tif().cloned().map(Scalar::from),
            Self::Tradable => operation.get_tradable().map(Scalar::from),
            Self::AltIds => map_fact(operation.get_altids()),
        }
    }

    /// Records a cell on `operation`, leniently: a null clears an optional
    /// fact, and an incompatible value is ignored.
    pub fn record<E: Operation + ?Sized>(self, operation: &mut E, value: &Scalar) {
        match self {
            Self::TimeInForce => operation.set_tif(tif_of(value)),
            Self::Tradable => operation.set_tradable(value.as_bool()),
            Self::AltIds => {
                if let Some(ids) = map_of(value) {
                    let _ = operation.set_altids(ids);
                }
            }
        }
    }
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
