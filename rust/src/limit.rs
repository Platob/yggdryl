//! `Limit`: one price limit of a book side - the price, the quantity resting
//! there and the entries that rest there - with its datatype, its field and
//! its scalar.

use smol_str::{SmolStr, format_smolstr};

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
    /// value door canonicalizes it to; a name the struct lacks is a null.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidRecord`] located at `$.price` for a price that is no
    /// decimal, `$.quantity` for a null or non-decimal quantity, `$.uuids`
    /// for entries that are no serie, `$.uuids[i]` for an entry that is no
    /// uuid, `$.<name>` for a name the struct should not hold, and `$` for
    /// a row of another width or a value of another shape.
    pub fn from_scalar(value: &Scalar) -> Result<Self> {
        if let Some(fields) = value.as_struct() {
            if let Some(name) = fields.keys().find(|name| !NAMES.contains(&name.as_str())) {
                return Err(refusal(
                    format_smolstr!("$.{name}"),
                    SmolStr::new_static("expected price, quantity or uuids, got an unknown field"),
                ));
            }
            let cell = |name: &str| fields.get(name).unwrap_or(&Scalar::Null);
            return Self::from_cells(cell(NAMES[0]), cell(NAMES[1]), cell(NAMES[2]));
        }
        let Some(row) = value.sequence_rows() else {
            return Err(refusal(
                SmolStr::new_static("$"),
                expected_got("a limit struct or its row", value.kind()),
            ));
        };
        let [price, quantity, uuids] = row.as_ref() else {
            return Err(refusal(
                SmolStr::new_static("$"),
                expected_got(
                    format_args!("a row of {} cells", NAMES.len()),
                    format_args!("{} cells", row.len()),
                ),
            ));
        };
        Self::from_cells(price, quantity, uuids)
    }

    fn from_cells(price: &Scalar, quantity: &Scalar, uuids: &Scalar) -> Result<Self> {
        let price = match price {
            Scalar::Null => None,
            held => Some(Decimal::from_scalar(held).ok_or_else(|| {
                refusal(
                    SmolStr::new_static("$.price"),
                    expected_got("a decimal or null", held.kind()),
                )
            })?),
        };
        let quantity = Decimal::from_scalar(quantity).ok_or_else(|| {
            refusal(
                SmolStr::new_static("$.quantity"),
                expected_got("a decimal", quantity.kind()),
            )
        })?;
        let Some(entries) = uuids.sequence_rows() else {
            return Err(refusal(
                SmolStr::new_static("$.uuids"),
                expected_got("a serie of uuids", uuids.kind()),
            ));
        };
        let uuids = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| match entry {
                Scalar::Uuid(uuid) => Ok(*uuid),
                other => Err(refusal(
                    format_smolstr!("$.uuids[{index}]"),
                    expected_got("a uuid", other.kind()),
                )),
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            price,
            quantity,
            uuids,
        })
    }
}

fn refusal(path: SmolStr, reason: SmolStr) -> Error {
    Error::InvalidRecord { path, reason }
}
