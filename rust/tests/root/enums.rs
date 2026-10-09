//! `rust/src/enums.rs`: the widths an enum leaf holds its codes at, and the
//! casts every leaf shares - from and to any integer, signed or unsigned, at
//! the root of a column or nested under it.

use std::sync::Arc;

use arrow_array::cast::AsArray as _;
use arrow_array::types::{Int8Type, Int64Type, UInt32Type, UInt64Type};
use arrow_array::{
    Array, ArrayRef, Int16Array, Int32Array, Int64Array, ListArray, StructArray, UInt16Array,
    UInt32Array, UInt64Array,
};
use arrow_schema::DataType as ArrowDataType;
use yggdryl::{ArrowCastOptions, DataType, EnumRepr, Field, Scalar, Serie, State, StructType};

fn strict() -> ArrowCastOptions {
    ArrowCastOptions::new().with_safe(false)
}

/// A leaf whose codes fit a byte holds and stores them as one; a leaf whose
/// codes pass 255 as two.
#[test]
fn a_leaf_holds_its_codes_at_the_narrowest_width_they_fit() {
    assert_eq!(<u8 as EnumRepr>::ARROW, ArrowDataType::UInt8);
    assert_eq!(<u16 as EnumRepr>::ARROW, ArrowDataType::UInt16);
    let two: u16 = State::DontKnow.code();
    assert!(two > 255, "a state's code passes a byte");
    let arrow = Field::new("x", DataType::State, true)
        .into_arrow_field()
        .unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt16);
}

/// Every integer width, signed or unsigned, lands as the codes it holds; a
/// code no member takes is refused naming the row, whatever width held it.
#[test]
fn every_integer_width_lands_as_the_codes_it_holds() {
    let state = Field::new("state", DataType::State, true);
    let sources: [ArrayRef; 6] = [
        Arc::new(Int16Array::from(vec![2001, 8003])),
        Arc::new(Int32Array::from(vec![2001, 8003])),
        Arc::new(Int64Array::from(vec![2001, 8003])),
        Arc::new(UInt16Array::from(vec![2001, 8003])),
        Arc::new(UInt32Array::from(vec![2001, 8003])),
        Arc::new(UInt64Array::from(vec![2001, 8003])),
    ];
    for source in sources {
        let width = source.data_type().clone();
        let landed = Serie::from_arrow_array(Some(&state), source, strict()).unwrap();
        assert_eq!(
            landed.scalar(0).unwrap(),
            Scalar::State(State::New),
            "{width}"
        );
        assert_eq!(
            landed.scalar(1).unwrap(),
            Scalar::State(State::Filled),
            "{width}"
        );
    }
    let refused = Serie::from_arrow_array(
        Some(&state),
        Arc::new(UInt64Array::from(vec![2001, u64::MAX])) as ArrayRef,
        strict(),
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("state"), "{refused}");
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

    // The one code under a byte, the state stated as none, fits the
    // narrowest integer.
    let none = Serie::from_scalars(
        Field::new("state", DataType::State, true),
        [Scalar::State(State::Unknown)],
    )
    .unwrap();
    let narrow = none
        .cast(&Field::new("x", DataType::Int8, true), strict())
        .unwrap()
        .require_arrow_array()
        .unwrap();
    assert_eq!(narrow.as_primitive::<Int8Type>().value(0), 0);
    let unsigned = states
        .cast(&Field::new("x", DataType::UInt64, true), strict())
        .unwrap()
        .require_arrow_array()
        .unwrap();
    assert_eq!(
        unsigned.as_primitive::<UInt64Type>().value(1),
        u64::from(State::Filled.code())
    );
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
        Field::new("state", DataType::State, true),
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
                "state",
                ArrowDataType::UInt32,
                true,
            )),
            Arc::new(UInt32Array::from(vec![u32::from(State::Filled.code())])) as ArrayRef,
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
    assert_eq!(row.get(0).unwrap().as_ref(), &Scalar::State(State::Filled));
    let items = row.get(1).unwrap();
    assert_eq!(items.get(0).unwrap().as_ref(), &Scalar::State(State::New));

    // And back out: the same row under integer children.
    let integers = StructType::from_fields([
        Field::new("state", DataType::Int64, true),
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
    assert_eq!(
        out.get(0).unwrap().as_i128(),
        Some(i128::from(State::Filled.code()))
    );
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
    use yggdryl::State;

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
