//! `Limit`: one price limit of a book side - the price, the quantity resting
//! there and the entries that rest there - with its datatype, its field and
//! its scalar.

use smol_str::SmolStr;

use crate::text::expected_got;
use crate::{DataType, Decimal, Error, Field, Result, Scalar, StructType, Uuid};

/// The cells a limit states, in the order its datatype declares them.
const NAMES: [&str; 3] = ["price", "quantity", "uuids"];

/// One price limit of a book side: what rests at one price, best first.
///
/// A side answers one per price it holds, best first, and one more last
/// for every entry that states no price. It is a value of its own rather
/// than a datatype: its datatype is the struct [`Self::dtype`] names, its
/// field the item [`Self::field`] of a `limits` column, and its scalar the
/// named struct [`Self::into_scalar`] hands over, read back by
/// [`Self::from_scalar`] in that shape or in the ordered row a
/// datatype's own value door answers.
///
/// ```
/// use yggdryl::{Decimal, Limit, Uuid};
///
/// # fn main() -> yggdryl::Result<()> {
/// let limit = Limit {
///     price: Some("101.5".parse()?),
///     quantity: Decimal::from_int(300),
///     uuids: vec![Uuid::from_v8(1), Uuid::from_v8(2)],
/// };
/// // The named struct, and the ordered row the datatype canonicalizes it to.
/// assert_eq!(Limit::from_scalar(&limit.into_scalar())?, limit);
/// let row = Limit::dtype().scalar(limit.into_scalar())?;
/// assert_eq!(Limit::from_scalar(&row)?, limit);
/// // The limit folding every unpriced entry states no price.
/// let unpriced = Limit { price: None, ..limit };
/// assert_eq!(Limit::from_scalar(&unpriced.into_scalar())?.price, None);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Limit {
    /// The limit's price; `None` on the one limit that folds every unpriced entry.
    pub price: Option<Decimal>,
    /// The exact sum of the quantities the entries state; an entry stating none adds nothing.
    pub quantity: Decimal,
    /// The entries' `curruuid`s in live order (best position first).
    pub uuids: Vec<Uuid>,
}

impl Limit {
    /// The datatype a limit is: `struct<price: decimal?, quantity: decimal,
    /// uuids: serie<uuid>>`, each decimal [`DataType::Decimal`].
    #[must_use]
    pub fn dtype() -> DataType {
        DataType::Struct(StructType::from_unique_fields(vec![
            DataType::Decimal.nullable_field(NAMES[0]),
            DataType::Decimal.required_field(NAMES[1]),
            DataType::serie(DataType::Uuid.required_field("uuid")).required_field(NAMES[2]),
        ]))
    }

    /// The required field `limit`: the item of a `limits` column.
    #[must_use]
    pub fn field() -> Field {
        Field::new("limit", Self::dtype(), false)
    }

    /// The limit as the named struct of its three cells, a missing price a
    /// null.
    #[must_use]
    pub fn into_scalar(&self) -> Scalar {
        Scalar::from_struct([
            (NAMES[0], self.price.map_or(Scalar::Null, Scalar::from)),
            (NAMES[1], Scalar::from(self.quantity)),
            (
                NAMES[2],
                Scalar::from_sequence(self.uuids.iter().copied().map(Scalar::Uuid)),
            ),
        ])
        .expect("three distinct names")
    }

    /// Reads a limit back from the named struct [`Self::into_scalar`]
    /// answers or from the ordered row of three cells [`Self::dtype`]'s
    /// value door canonicalizes it to. The value passes that one door first,
    /// through [`Self::field`], so a cell is read exactly as a `limits`
    /// column would hold it - a text or a number the decimal datatype
    /// restates is read as it restates it - except that a name the struct
    /// lacks is a null rather than the default the door fills a required
    /// cell with: a limit states its quantity, never a zero it was not given.
    ///
    /// # Errors
    ///
    /// The refusal [`Self::field`]'s [`Field::scalar`] answers, located
    /// under `$.limit`: a price that is no decimal, a null, missing or
    /// non-decimal quantity, entries that are missing or no serie of uuids,
    /// a name the struct should not hold, a row of another width or a value
    /// of another shape.
    pub fn from_scalar(value: &Scalar) -> Result<Self> {
        let value = match value.as_struct() {
            Some(fields) if NAMES.iter().any(|name| !fields.contains_key(*name)) => {
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
            _ => value.clone(),
        };
        let row = Self::field().scalar(value)?;
        let cells = row.sequence_rows();
        let Some([price, Scalar::Decimal(quantity), uuids]) = cells.as_deref() else {
            return Err(unread(&row));
        };
        let price = match price {
            Scalar::Decimal(price) => Some(*price),
            Scalar::Null => None,
            _ => return Err(unread(&row)),
        };
        let uuids = uuids
            .sequence_rows()
            .ok_or_else(|| unread(&row))?
            .iter()
            .map(|entry| match entry {
                Scalar::Uuid(uuid) => Ok(*uuid),
                _ => Err(unread(&row)),
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            price,
            quantity: *quantity,
            uuids,
        })
    }
}

/// The value door answered a row this reading does not know: the one
/// contract and this reading disagree, which no caller value can cause.
fn unread(row: &Scalar) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$.limit"),
        reason: expected_got("the canonical limit row", row.kind()),
    }
}
