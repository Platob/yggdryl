//! What a rowset write allocates per row: nothing, the claim
//! `rust/tests/allocations.rs` pins for every writer of the core's that lends
//! its bytes where they lie, pinned here for the medium the core reaches by
//! registration, in the counting allocator both share.

#[path = "../../rust/tests/support/allocations.rs"]
#[allow(dead_code)]
mod allocations;

use std::hint::black_box;

use yggdryl::{DataType, Scalar, Serie, StructType};
use yggdryl_xmla::Rowset;

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

/// A rowset row is written cell by cell off the column leaves - a text cell
/// lends its bytes where they lie, a number, a boolean or a null is spelled
/// straight into the sink - so the per-row path allocates nothing, and what a
/// document costs beyond its sink is the same at every corpus size.
#[test]
fn a_rowset_write_allocates_nothing_per_row() {
    let field = DataType::from(
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
            DataType::Float64.required_field("price"),
            DataType::Boolean.required_field("live"),
        ])
        .expect("a valid root"),
    )
    .required_field("row");
    let rowset = Rowset::new(field.clone()).expect("a rowset over a record field");
    let cost = |rows: usize| {
        let batch = Serie::from_scalars(
            field.clone(),
            (0..rows).map(|index| {
                Scalar::from_sequence([
                    Scalar::from(index as i64),
                    if index % 5 == 0 {
                        Scalar::Null
                    } else {
                        Scalar::from(format!("SYM{index:04}"))
                    },
                    Scalar::from(index as f64 * 0.25),
                    Scalar::from(index % 2 == 0),
                ])
            }),
        )
        .expect("rows under the field");
        // Reserved past what the rows take, so the sink never grows and the
        // count is the row path's alone.
        let mut document = Vec::with_capacity(rows * 160);
        let ((), counts) = allocations::measure(|| {
            rowset
                .write_rows(&mut document, black_box(&batch))
                .expect("the rows are written");
        });
        assert!(
            document.len() < rows * 160,
            "{rows} rows filled the reserved sink"
        );
        assert!(document.ends_with(b"</row>"));
        counts.allocations
    };
    let (small, large) = (cost(64), cost(4_096));
    assert_eq!(
        small, large,
        "64 rows cost {small} allocations, 4096 cost {large}; a rowset row must be written off its leaves"
    );
}
