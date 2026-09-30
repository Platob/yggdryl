//! `rust/src/enums.rs`: the widths an enum leaf holds its codes at, and the
//! casts every leaf shares - from and to any integer, signed or unsigned, at
//! the root of a column or nested under it.

use std::sync::Arc;

use arrow_array::cast::AsArray as _;
use arrow_array::types::{Int8Type, Int64Type, UInt32Type, UInt64Type};
use arrow_array::{
    Array, ArrayRef, Int8Array, Int16Array, Int64Array, ListArray, StructArray, UInt16Array,
    UInt32Array, UInt64Array,
};
use arrow_schema::DataType as ArrowDataType;
use yggdryl::{
    ArrowCastOptions, DataType, EnumRepr, Field, MarketDataKind, Scalar, Serie, Side, State,
    StructType,
};

fn strict() -> ArrowCastOptions {
    ArrowCastOptions::new().with_safe(false)
}

/// A leaf whose codes fit a byte holds and stores them as one; a leaf whose
/// codes pass 255 as two.
#[test]
fn a_leaf_holds_its_codes_at_the_narrowest_width_they_fit() {
    assert_eq!(<u8 as EnumRepr>::ARROW, ArrowDataType::UInt8);
    assert_eq!(<u16 as EnumRepr>::ARROW, ArrowDataType::UInt16);
    let one: u8 = Side::SellUnd.code();
    let two: u16 = State::DontKnow.code();
    assert_eq!(
        (one, u16::from(MarketDataKind::TradeBatch.code())),
        (17, 25)
    );
    assert!(two > 255, "a state's code passes a byte");
    for (dtype, storage) in [
        (DataType::Side, ArrowDataType::UInt8),
        (DataType::MarketDataKind, ArrowDataType::UInt8),
        (DataType::State, ArrowDataType::UInt16),
        (DataType::MarketDataType, ArrowDataType::UInt16),
    ] {
        let arrow = Field::new("x", dtype.clone(), true)
            .into_arrow_field()
            .unwrap();
        assert_eq!(arrow.data_type(), &storage, "{dtype}");
    }
}

/// Every integer width, signed or unsigned, lands as the codes it holds; a
/// code no member takes is refused naming the row, whatever width held it.
#[test]
fn every_integer_width_lands_as_the_codes_it_holds() {
    let side = Field::new("side", DataType::Side, true);
    let sources: [ArrayRef; 6] = [
        Arc::new(Int8Array::from(vec![1, 2])),
        Arc::new(Int16Array::from(vec![1, 2])),
        Arc::new(Int64Array::from(vec![1, 2])),
        Arc::new(UInt16Array::from(vec![1, 2])),
        Arc::new(UInt32Array::from(vec![1, 2])),
        Arc::new(UInt64Array::from(vec![1, 2])),
    ];
    for source in sources {
        let width = source.data_type().clone();
        let landed = Serie::from_arrow_array(Some(&side), source, strict()).unwrap();
        assert_eq!(
            landed.scalar(0).unwrap(),
            Scalar::Side(Side::Buy),
            "{width}"
        );
        assert_eq!(
            landed.scalar(1).unwrap(),
            Scalar::Side(Side::Sell),
            "{width}"
        );
    }
    let refused = Serie::from_arrow_array(
        Some(&side),
        Arc::new(UInt64Array::from(vec![1, u64::MAX])) as ArrayRef,
        strict(),
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("side"), "{refused}");
}

/// A column casts out to any integer width its codes fit, and to text as
/// the names of its members.
#[test]
fn a_column_casts_out_to_any_integer_and_to_text() {
    let state = Field::new("state", DataType::State, true);
    let states = Serie::from_scalars(
        state,
        [Scalar::State(State::New), Scalar::State(State::Filled)],
    )
    .unwrap();
    let wide = states
        .cast(&Field::new("x", DataType::Int64, true), strict())
        .unwrap()
        .require_arrow_array()
        .unwrap();
    let wide = wide.as_primitive::<Int64Type>();
    assert_eq!(wide.value(0), i64::from(State::New.code()));
    let unsigned = states
        .cast(&Field::new("x", DataType::UInt32, true), strict())
        .unwrap()
        .require_arrow_array()
        .unwrap();
    assert_eq!(
        unsigned.as_primitive::<UInt32Type>().value(1),
        u32::from(State::Filled.code())
    );
    let names = states
        .cast(&Field::new("x", DataType::utf8(), true), strict())
        .unwrap();
    assert_eq!(names.scalar(1).unwrap(), Scalar::from("FILLED"));

    let side = Serie::from_scalars(
        Field::new("side", DataType::Side, true),
        [Scalar::Side(Side::SellUnd)],
    )
    .unwrap();
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
    // A state's code does not fit a byte, so a strict cast into one refuses.
    assert!(
        states
            .cast(&Field::new("x", DataType::Int8, true), strict())
            .is_err()
    );
}

/// The same rule holds below the root: a struct child and a serie's items
/// land from any integer and cast back out to one.
#[test]
fn nested_enum_leaves_cast_from_and_to_any_integer() {
    let target = StructType::from_fields([
        Field::new("side", DataType::Side, true),
        Field::new(
            "states",
            DataType::serie(DataType::State.nullable_field("item")),
            true,
        ),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    let states = ListArray::from_iter_primitive::<arrow_array::types::Int16Type, _, _>([Some(
        vec![Some(i16::try_from(State::New.code()).unwrap())],
    )]);
    let source = StructArray::from(vec![
        (
            Arc::new(arrow_schema::Field::new(
                "side",
                ArrowDataType::UInt32,
                true,
            )),
            Arc::new(UInt32Array::from(vec![2])) as ArrayRef,
        ),
        (
            Arc::new(arrow_schema::Field::new(
                "states",
                states.data_type().clone(),
                true,
            )),
            Arc::new(states) as ArrayRef,
        ),
    ]);
    let landed =
        Serie::from_arrow_array(Some(&target), Arc::new(source) as ArrayRef, strict()).unwrap();
    let row = landed.scalar(0).unwrap();
    assert_eq!(row.get(0).unwrap().as_ref(), &Scalar::Side(Side::Sell));
    let items = row.get(1).unwrap();
    assert_eq!(items.get(0).unwrap().as_ref(), &Scalar::State(State::New));

    // And back out: the same row under integer children.
    let integers = StructType::from_fields([
        Field::new("side", DataType::Int64, true),
        Field::new(
            "states",
            DataType::serie(DataType::UInt16.nullable_field("item")),
            true,
        ),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    let out = landed.cast(&integers, strict()).unwrap();
    let out = out.scalar(0).unwrap();
    assert_eq!(out.get(0).unwrap().as_i128(), Some(2));
    assert_eq!(
        out.get(1).unwrap().get(0).unwrap().as_i128(),
        Some(i128::from(State::New.code()))
    );
}

/// A spelling no exact vocabulary names reads by its words: each read as the
/// word it abbreviates or inflects, the words that say nothing dropped, the
/// set matched against the words of every member's own names.
#[test]
fn a_spelling_reads_by_the_words_it_is_made_of() {
    use yggdryl::{Side, State, TimeInForce};

    for (spelling, state) in [
        ("partfilled", State::PartiallyFilled),
        ("Part-Filled", State::PartiallyFilled),
        ("partial fill", State::PartiallyFilled),
        ("filled partially", State::PartiallyFilled),
        ("PARTIALLY FILLED ORDER", State::PartiallyFilled),
        ("order fill", State::Filled),
        ("ORDER FILLED", State::Filled),
        ("orderfilled", State::Filled),
        ("fill", State::Filled),
        ("fully filled", State::Filled),
        ("order cancelled", State::Canceled),
        ("cxl", State::Canceled),
        ("cancel pending", State::PendingCancel),
        ("pending cxl", State::PendingCancel),
        ("rej", State::Rejected),
        ("order rejected", State::Rejected),
        ("pend replace", State::PendingReplace),
        ("trade corrected", State::TradeCorrect),
    ] {
        assert_eq!(State::from_spelling(spelling), Some(state), "{spelling}");
        assert_eq!(State::read(spelling).unwrap(), state, "{spelling}");
    }
    assert_eq!(Side::from_spelling("short sell"), Some(Side::SShort));
    assert_eq!(
        TimeInForce::from_spelling("good till cancelled"),
        TimeInForce::from_spelling("GoodTillCancel")
    );
}

/// A word no name uses makes the whole spelling unread, the words that say
/// nothing name nothing alone, and a set two members make names neither.
#[test]
fn a_spelling_the_words_do_not_name_once_is_refused() {
    use yggdryl::State;

    for spelling in ["partially frobnicated", "order", "partially", "zzzz", ""] {
        assert_eq!(State::from_spelling(spelling), None, "{spelling:?}");
    }
    let refused = State::read("partially frobnicated")
        .unwrap_err()
        .to_string();
    assert!(refused.contains("partially frobnicated"), "{refused}");
}

/// A text column of free spellings lands as the codes their words name.
#[test]
fn a_text_column_of_free_spellings_lands_as_codes() {
    use arrow_array::{Array, ArrayRef, StringArray, UInt16Array};
    use yggdryl::{ArrowCastOptions, DataType, Field, Serie, State};

    let field = Field::new("state", DataType::State, true);
    let landed = Serie::from_arrow_array(
        Some(&field),
        std::sync::Arc::new(StringArray::from(vec![
            "part filled",
            "order fill",
            "part filled",
        ])) as ArrayRef,
        ArrowCastOptions::new().with_safe(false),
    )
    .unwrap();
    let codes = landed.require_arrow_array().unwrap();
    let codes = codes.as_any().downcast_ref::<UInt16Array>().unwrap();
    assert_eq!(
        codes.values().as_ref(),
        [
            State::PartiallyFilled.code(),
            State::Filled.code(),
            State::PartiallyFilled.code()
        ]
    );
}
