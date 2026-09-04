//! A target the expression module has never heard of.
//!
//! [`ApplyExpression`] inverts who owns evaluation: the target says how an
//! expression applies to it. This listing lives here, outside
//! `expression/`, and reaches that module through its public surface
//! alone - if it ever needed a line inside `expression/` beyond a `use`,
//! the trait would be shaped wrong.
//!
//! [`ApplyExpression`]: crate::expression::ApplyExpression

use crate::expression::{ApplyExpression, Bound, Expression};
use crate::{DataType, Result, Scalar, Url};

/// An owned listing, seen as a target: applying a predicate yields the
/// positions of the entries it does not rule out, in listing order.
struct Listing(Vec<Url>);

impl ApplyExpression for Listing {
    type Output = Vec<usize>;

    fn apply_expression(&self, bound: &Bound) -> Result<Vec<usize>> {
        let mut kept = Vec::new();
        for (position, entry) in self.0.iter().enumerate() {
            // A `Url` already answers holder attributes, so the listing
            // composes the public conservative verb rather than walking
            // the expression itself.
            if bound.matches_holder(entry)? {
                kept.push(position);
            }
        }
        Ok(kept)
    }
}

#[test]
fn a_listing_defined_outside_the_expression_module_is_a_target() {
    let listing = Listing(
        [
            "file:///lake/year=2024/part-0.parquet",
            "file:///lake/year=2023/part-0.parquet",
            "file:///lake/year=2024/part-1.parquet",
        ]
        .into_iter()
        .map(|url| Url::from_str(url).unwrap())
        .collect(),
    );
    // The same empty-column schema the holder filters in this module bind
    // against: the predicate reads no row, only the holder.
    let schema = DataType::from_fields([]).unwrap().required_field("holder");
    let bound = "&holder.partition['year'] = '2024'"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(listing.apply_expression(&bound).unwrap(), vec![0, 2]);

    // A predicate a path cannot answer rules nothing out, exactly as the
    // holder target promises.
    let unknown = "&holder.size > 0"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    assert_eq!(listing.apply_expression(&unknown).unwrap(), vec![0, 1, 2]);
}

#[test]
fn the_row_target_answers_through_the_same_trait() {
    // The trait is also how the built-in targets are reached: one row
    // applies to the value the expression computes.
    let schema = DataType::from_fields([DataType::Int64.named_field("i", true)])
        .unwrap()
        .required_field("row");
    let bound = "i + 1"
        .parse::<Expression>()
        .unwrap()
        .bind(&schema)
        .unwrap();
    let row = Scalar::from_sequence([Scalar::I64(41)]);
    assert_eq!(row.apply_expression(&bound).unwrap(), Scalar::I64(42));
}
