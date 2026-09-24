//! `rust/src/graph/event.rs` and the value types it holds: the sizes of the
//! four holders and of the two smallest things they carry, pinned so a
//! field added without a box shows up as a moved number.

use std::mem::size_of;

use yggdryl::Side;
use yggdryl::graph::{
    BookRef, MarketData, MarketEventData, MarketOperationData, MarketOperationEventData,
};

/// A side is one byte: an enum over its code set, no text.
#[test]
fn a_side_is_one_byte() {
    assert_eq!(size_of::<Side>(), 1);
}

/// The book control an operation carries is one pointer: boxed, and absent
/// on every operation that is not a market-data entry.
#[test]
fn a_boxed_book_control_is_one_pointer() {
    assert_eq!(size_of::<Option<Box<BookRef>>>(), 8);
}

/// The four holders' sizes, first pinned here when the slim `Market` trait
/// landed: `MarketData` is the nineteen market facts, `MarketEventData`
/// adds the clocks, the two operation holders add the boxed lanes and the
/// three identifier maps. A moved number is a design answer, never a number
/// to re-pin from a whole run.
#[test]
fn the_holders_are_the_sizes_the_build_reported_when_first_pinned() {
    assert_eq!(
        (
            size_of::<MarketData>(),
            size_of::<MarketEventData>(),
            size_of::<MarketOperationData>(),
            size_of::<MarketOperationEventData>(),
        ),
        (608, 784, 832, 1008)
    );
}
