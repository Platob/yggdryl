//! `rust/src/enums.rs` over the enum leaves `yggdryl-market` claims: the
//! byte its one-byte kinds hold their codes at, the words their spellings
//! read by, and the rule that keeps two enum leaves apart - which takes a
//! second leaf beside the core's `State`, so it is pinned here. The rules
//! over `State` alone are `rust/tests/root/enums.rs`'s.

use arrow_array::cast::AsArray;
use arrow_array::types::{Int8Type, UInt64Type};
use arrow_schema::DataType as ArrowDataType;
use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie};
use yggdryl_market::{MarketDataKind, MarketDataType, Side, TimeInForce};

fn strict() -> ArrowCastOptions {
    ArrowCastOptions::new().with_safe(false)
}

/// A market leaf whose codes fit a byte holds and stores them as one; one
/// whose codes pass 255 as two, and a column casts out to any integer
/// width its codes fit.
#[test]
fn a_market_leaf_holds_its_codes_at_the_narrowest_width_they_fit() {
    crate::install::installed();
    let one: u8 = Side::SellUnd.code();
    assert_eq!(
        (one, u16::from(MarketDataKind::TradeBatch.code())),
        (17, 25)
    );
    for (dtype, storage) in [
        (Side::dtype(), ArrowDataType::UInt8),
        (MarketDataKind::dtype(), ArrowDataType::UInt8),
        (MarketDataType::dtype(), ArrowDataType::UInt16),
    ] {
        let arrow = Field::new("x", dtype.clone(), true)
            .into_arrow_field()
            .unwrap();
        assert_eq!(arrow.data_type(), &storage, "{dtype}");
    }

    let side = Serie::from_scalars(Side::field("side"), [Scalar::from(Side::SellUnd)]).unwrap();
    let narrow = side
        .cast(&Field::new("x", DataType::Int8, true), strict())
        .unwrap()
        .require_arrow_array()
        .unwrap();
    assert_eq!(narrow.as_primitive::<Int8Type>().value(0), 17);
    let unsigned = side
        .cast(&Field::new("x", DataType::UInt64, true), strict())
        .unwrap()
        .require_arrow_array()
        .unwrap();
    assert_eq!(unsigned.as_primitive::<UInt64Type>().value(0), 17);
}

/// A spelling no exact vocabulary names reads by its words, for a market
/// leaf as for a state.
#[test]
fn a_market_spelling_reads_by_the_words_it_is_made_of() {
    crate::install::installed();
    assert_eq!(Side::from_spelling("short sell"), Some(Side::SShort));
    assert_eq!(
        TimeInForce::from_spelling("good till cancelled"),
        TimeInForce::from_spelling("GoodTillCancel")
    );
}

/// The column cast keeps two enum leaves apart as the value door does: a
/// side stores `BUYS` as `1` and a time in force stores `DAY` as `1`, and the
/// engine never reads one leaf's codes as the other's - a side column cast
/// into times in force is refused by name, and so is the reverse, whatever
/// `safe` says and whether the cast is compiled once or run on the column.
/// The rule is the enum family's, not the pair's: a state column is refused
/// the same way. Its own leaf passes untouched, and an integer reads the
/// codes.
#[test]
fn a_column_of_one_enum_leaf_is_never_cast_into_another() {
    use yggdryl::{ArrowCastPlan, State};

    crate::install::installed();
    let sides = Serie::from_scalars(
        Field::new("s", Side::dtype(), false),
        [Scalar::from(Side::Buy), Scalar::from(Side::Sell)],
    )
    .unwrap();
    let tifs = Serie::from_scalars(
        Field::new("t", TimeInForce::dtype(), false),
        [
            Scalar::from(TimeInForce::Day),
            Scalar::from(TimeInForce::GoodTillCancel),
        ],
    )
    .unwrap();
    let states = Serie::from_scalars(
        Field::new("st", DataType::State, false),
        [Scalar::State(State::New), Scalar::State(State::Filled)],
    )
    .unwrap();
    for (source, target) in [
        (&sides, TimeInForce::dtype()),
        (&tifs, Side::dtype()),
        (&states, Side::dtype()),
        (&tifs, DataType::State),
    ] {
        let target = Field::new("x", target, false);
        for safe in [true, false] {
            let refusal = source
                .cast(&target, ArrowCastOptions::new().with_safe(safe))
                .unwrap_err()
                .to_string();
            assert!(
                refusal.contains(&source.field().unwrap().dtype().to_string())
                    && refusal.contains(&target.dtype().to_string()),
                "{refusal}"
            );
            assert!(
                ArrowCastPlan::compile(
                    source.field().unwrap(),
                    &target,
                    ArrowCastOptions::new().with_safe(safe)
                )
                .is_err(),
                "a plan onto {} from {}",
                target.dtype(),
                source.field().unwrap().dtype()
            );
        }
    }
    let same = sides
        .cast(
            &Field::new("same", Side::dtype(), false),
            ArrowCastOptions::new(),
        )
        .unwrap();
    assert_eq!(same.scalar(1).unwrap(), Scalar::from(Side::Sell));
    let codes = sides
        .cast(
            &Field::new("codes", DataType::Int32, false),
            ArrowCastOptions::new(),
        )
        .unwrap();
    assert_eq!(codes.scalar(1).unwrap(), Scalar::from(2_i32));
}
