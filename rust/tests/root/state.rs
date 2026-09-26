//! `rust/src/state.rs`: what state one thing is in, a lifecycle-sorted enum
//! stored as the `int32` code of its member.
//!
//! `DataType` is `#[non_exhaustive]` and the datatype layer carries some sixty
//! wildcard arms, so a new variant compiles clean while behaving wrongly. A
//! green build proves nothing; these are the invariants a wildcard cannot
//! satisfy by accident.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int32Array, Int64Array, StringArray};
use arrow_schema::DataType as ArrowDataType;
use yggdryl::{ArrowCastOptions, DataType, DataTypeId, DataTypeKind, Field, Scalar, Serie, State};

fn strict() -> ArrowCastOptions {
    ArrowCastOptions::new().with_safe(false)
}

#[test]
fn a_code_or_a_spelling_that_names_no_state_is_refused_by_name() {
    assert_eq!(State::from_code(1), None);
    assert_eq!(State::from_code(-1), None);
    let refused = State::read_code(4_242).unwrap_err().to_string();
    assert!(refused.contains("4242"), "{refused}");
    let refused = State::read_code(i64::from(i32::MAX) + 1)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("2147483648"), "{refused}");

    let refused = State::read("whatever").unwrap_err().to_string();
    assert!(refused.contains("whatever"), "{refused}");
    assert_eq!(State::from_spelling(""), None);
    // A stored code is an integer and never text: `2001` spells nothing,
    // because `0`, `1` and `2` are FIX wire codes and a number read as text
    // would answer the wrong state for one of the two vocabularies.
    assert_eq!(State::from_spelling("2001"), None);
    // A wire code never folds: `A` is PendingNew and `a` is not a code.
    assert_eq!(State::from_spelling("A"), Some(State::PendingNew));
    assert_eq!(State::from_spelling("a"), None);
}

#[test]
fn the_codes_sort_as_the_members_rank_and_the_hundreds_are_the_rank() {
    // Declared in code order, each code unique and each name unique.
    let codes: Vec<i32> = State::ALL.iter().map(|state| state.code()).collect();
    let mut sorted = codes.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        codes, sorted,
        "the members are declared in the order they sort"
    );
    let mut names: Vec<&str> = State::ALL.iter().map(|state| state.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), State::ALL.len(), "one name per member");

    // The derived order is the codes' order, so a sort of the members is a
    // sort of the stored integers.
    let mut members = State::ALL.to_vec();
    members.reverse();
    members.sort_unstable();
    assert_eq!(members, State::ALL);

    for state in State::ALL {
        assert_eq!(
            i32::from(u8::try_from(state.code() / 100).unwrap()),
            i32::from(state.rank())
        );
        assert_eq!(State::from_code(state.code()), Some(*state));
        assert_eq!(State::from_name(state.as_str()), Some(*state));
        assert_eq!(State::try_from(state.code()).unwrap(), *state);
        assert!(!state.description().is_empty(), "{state}");
        // The units are a member's place in its rank, from zero: a state
        // added to a rank takes the next free number, so nothing moves.
        let place = state.code() % 100;
        let before = State::ALL
            .iter()
            .filter(|other| other.rank() == state.rank() && other.code() < state.code())
            .count();
        assert_eq!(usize::try_from(place).unwrap(), before, "{state}");
    }
    assert_eq!(State::default(), State::Unknown);
    assert_eq!(State::unknown().code(), 0);
    assert_eq!(State::New.code(), 2001);
    assert_eq!(State::Acknowledged.code(), 2004);
    assert_eq!(State::Filled.code(), 8003);
    assert_eq!(State::Rejected.code(), 9502);
}

#[test]
fn every_ending_is_told_apart_from_every_other_without_reading_a_name() {
    for state in State::ALL {
        let bands = [
            state.is_live(),
            state.is_done(),
            state.is_cancelled(),
            state.is_failed(),
        ];
        assert_eq!(bands.iter().filter(|held| **held).count(), 1, "{state}");
        assert_eq!(state.is_pending(), state.rank() == 10, "{state}");
    }
    for state in [
        State::Pending,
        State::Received,
        State::Acknowledged,
        State::PartiallyFilled,
        State::Locked,
        State::PendingCancel,
        State::Released,
    ] {
        assert!(state.is_live(), "{state}");
    }
    for state in [
        State::Filled,
        State::Allocated,
        State::Settled,
        State::Verified,
    ] {
        assert!(state.is_done(), "{state}");
    }
    for state in [
        State::Canceled,
        State::Reversed,
        State::Removed,
        State::Terminated,
    ] {
        assert!(state.is_cancelled(), "{state}");
    }
    for state in [
        State::Rejected,
        State::DontKnow,
        State::Mismatched,
        State::NotFound,
    ] {
        assert!(state.is_failed(), "{state}");
    }
    for state in [State::PartiallyFilled, State::Trade, State::Filled] {
        assert!(state.is_execution(), "{state}");
    }
    for state in [
        State::InProgress,
        State::Complete,
        State::TradeCorrect,
        State::TradeCancel,
        State::TradeInClearingHold,
        State::TradeReleasedToClearing,
    ] {
        assert!(!state.is_execution(), "{state}");
    }
}

#[test]
fn a_chain_folds_to_the_furthest_state_it_knows() {
    assert_eq!(State::Unknown.merge_with(State::New), State::New);
    assert_eq!(State::New.merge_with(State::Unknown), State::New);
    assert_eq!(State::New.merge_with(State::Filled), State::Filled);
    assert_eq!(State::Filled.merge_with(State::New), State::Filled);
    // Within one rank the state that stood first stands.
    assert_eq!(State::New.merge_with(State::Acknowledged), State::New);
}

#[test]
fn a_state_answers_every_vocabulary_that_names_it() {
    for (spelling, expected) in [
        // The stored name, and its folds.
        ("PARTIALLY_FILLED", State::PartiallyFilled),
        ("partially filled", State::PartiallyFilled),
        // FIX `OrdStatus` and `ExecType` wire codes.
        ("0", State::New),
        ("1", State::PartiallyFilled),
        ("2", State::Filled),
        ("8", State::Rejected),
        ("F", State::Trade),
        ("M", State::Locked),
        ("N", State::Released),
        // The specification's names.
        ("New", State::New),
        ("PartiallyFilled", State::PartiallyFilled),
        ("DoneForDay", State::DoneForDay),
        ("done_for_day", State::DoneForDay),
        ("DONE FOR DAY", State::DoneForDay),
        ("AcceptedForBidding", State::Accepted),
        (
            "TradeHasBeenReleasedToClearing",
            State::TradeReleasedToClearing,
        ),
        // A scheduler's words.
        ("running", State::Running),
        ("succeeded", State::Succeeded),
        ("success", State::Succeeded),
        ("timed out", State::TimedOut),
        ("timeout", State::TimedOut),
        ("failed", State::Failed),
        // The short names a FIX bridge logs.
        ("PartFill", State::PartiallyFilled),
        ("PendNew", State::PendingNew),
        ("PendCancel", State::PendingCancel),
        ("PendReplace", State::PendingReplace),
        ("DoneDay", State::DoneForDay),
        ("Cancel", State::Canceled),
        ("Cancelled", State::Canceled),
        ("Reject", State::Rejected),
        // The financial event states.
        ("acknowledged", State::Acknowledged),
        ("ack", State::Acknowledged),
        ("DK", State::DontKnow),
        ("DontKnow", State::DontKnow),
        ("allocated", State::Allocated),
        ("affirmed", State::Affirmed),
        ("confirmed", State::Confirmed),
        ("settled", State::Settled),
        ("cleared", State::Cleared),
        ("DeemedVerified", State::Verified),
        ("ReversalPending", State::PendingReversal),
        ("QuoteNotFound", State::NotFound),
    ] {
        assert_eq!(State::from_spelling(spelling), Some(expected), "{spelling}");
        // A stored name reads back as itself.
        assert_eq!(State::from_spelling(expected.as_str()), Some(expected));
        assert_eq!(expected.to_string(), expected.as_str());
    }
}

#[test]
fn every_fix_status_field_answers_by_its_own_code_set() {
    for (tag, code, expected) in [
        (39, "0", Some(State::New)),
        (39, "A", Some(State::PendingNew)),
        (150, "F", Some(State::Trade)),
        (150, "M", Some(State::Locked)),
        (1036, "0", Some(State::Received)),
        (1036, "1", Some(State::Acknowledged)),
        (1036, "2", Some(State::DontKnow)),
        (939, "0", Some(State::Accepted)),
        (939, "8", Some(State::PendingVerification)),
        (939, "10", Some(State::Verified)),
        (939, "11", Some(State::Disputed)),
        (297, "16", Some(State::Active)),
        (297, "6", Some(State::Removed)),
        (297, "9", Some(State::NotFound)),
        // A market warning states no state.
        (297, "12", None),
        (297, "13", None),
        (87, "0", Some(State::Allocated)),
        (87, "7", Some(State::Reversed)),
        (87, "14", Some(State::PendingReversal)),
        (665, "2", Some(State::Mismatched)),
        (665, "4", Some(State::Confirmed)),
        (940, "3", Some(State::Affirmed)),
        (1375, "2", Some(State::Complete)),
        (531, "0", Some(State::Rejected)),
        (531, "7", Some(State::Canceled)),
        (531, "C", Some(State::Canceled)),
        // A code a set does not define, and a tag no status is read off.
        (1036, "9", None),
        (54, "1", None),
    ] {
        assert_eq!(State::from_fix_status(tag, code), expected, "{tag}={code}");
    }
    assert_eq!(State::FIX_STATUS_TAGS[0], 39, "OrdStatus is read first");
    for (msgtype, expected) in [
        ("D", Some(State::PendingNew)),
        ("E", Some(State::PendingNew)),
        ("F", Some(State::PendingCancel)),
        ("G", Some(State::PendingReplace)),
        ("R", Some(State::Pending)),
        ("S", Some(State::Active)),
        ("3", Some(State::Rejected)),
        ("j", Some(State::Rejected)),
        ("8", None),
        ("0", None),
    ] {
        assert_eq!(State::from_fix_msgtype(msgtype), expected, "35={msgtype}");
    }
}

#[test]
fn a_state_is_a_datatype_of_the_enum_family() {
    assert_eq!(DataType::State.id(), DataTypeId::State);
    assert_eq!(DataTypeId::State.kind(), DataTypeKind::Enum);
    assert!(DataTypeKind::Enum.contains(DataTypeId::State));
    assert!(!DataType::State.is_code());
    assert_eq!(DataType::State.code_width(), None);
    assert_eq!(DataType::from_str("state").unwrap(), DataType::State);
    assert_eq!(DataType::State.to_string(), "state");
    assert_eq!(
        DataType::State.default_value().unwrap(),
        Scalar::State(State::Unknown)
    );
}

#[test]
fn the_value_door_reads_a_member_a_code_and_a_spelling() {
    let field = DataType::State.required_field("state");
    assert_eq!(
        field.scalar(Scalar::State(State::Filled)).unwrap(),
        Scalar::State(State::Filled)
    );
    assert_eq!(
        field.scalar(Scalar::from(8003_i32)).unwrap(),
        Scalar::State(State::Filled)
    );
    assert_eq!(
        field.scalar(Scalar::from(8003_i64)).unwrap(),
        Scalar::State(State::Filled)
    );
    assert_eq!(
        field.scalar(Scalar::from("Filled")).unwrap(),
        Scalar::State(State::Filled)
    );
    assert!(field.scalar(Scalar::from(8004_i32 + 90)).is_err());
    assert!(field.scalar(Scalar::from("not a state")).is_err());
    assert!(field.scalar(Scalar::from(true)).is_err());
    let value = Scalar::State(State::PartiallyFilled);
    assert_eq!(value.as_str(), Some("PARTIALLY_FILLED"));
    assert_eq!(value.dtype().unwrap(), DataType::State);
    assert!(!value.is_code());
}

#[test]
fn a_column_is_int32_codes_under_the_state_extension() {
    let field = Field::new("state", DataType::State, true);
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Int32);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.state");
    assert_eq!(arrow.metadata()["ARROW:extension:metadata"], "");
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);

    let values = [
        Scalar::State(State::New),
        Scalar::Null,
        Scalar::State(State::Rejected),
    ];
    let serie = Serie::from_scalars(field.clone(), values.clone()).unwrap();
    let array = serie.require_arrow_array().unwrap();
    let codes = array.as_any().downcast_ref::<Int32Array>().unwrap();
    assert_eq!(codes.values().as_ref(), [2001, 0, 9502]);
    assert!(codes.is_null(1));
    let back = Serie::from_arrow_array(Some(&field), array, ArrowCastOptions::default()).unwrap();
    for (index, value) in values.iter().enumerate() {
        assert_eq!(&back.scalar(index).unwrap(), value, "row {index}");
    }

    // The same storage under another name is not a state.
    let foreign = arrow_schema::Field::new("state", ArrowDataType::Utf8, true).with_metadata(
        [
            (
                "ARROW:extension:name".to_owned(),
                "yggdryl.state".to_owned(),
            ),
            ("ARROW:extension:metadata".to_owned(), String::new()),
        ]
        .into(),
    );
    assert_eq!(
        Field::from_arrow_field(&foreign).unwrap().dtype(),
        &DataType::utf8()
    );
}

#[test]
fn integers_land_as_codes_and_a_code_that_names_no_member_is_refused_by_row() {
    let field = Field::new("state", DataType::State, true);
    let landed = Serie::from_arrow_array(
        Some(&field),
        Arc::new(Int64Array::from(vec![Some(2001), None, Some(8003)])) as ArrayRef,
        strict(),
    )
    .unwrap();
    assert_eq!(landed.scalar(0).unwrap(), Scalar::State(State::New));
    assert_eq!(landed.scalar(1).unwrap(), Scalar::Null);
    assert_eq!(landed.scalar(2).unwrap(), Scalar::State(State::Filled));

    let refused = Serie::from_arrow_array(
        Some(&field),
        Arc::new(Int32Array::from(vec![2001, 7])) as ArrayRef,
        strict(),
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("row 1"), "{refused}");
    // Under `safe`, a nullable column takes the failed code as null.
    let safe = Serie::from_arrow_array(
        Some(&field),
        Arc::new(Int32Array::from(vec![2001, 7])) as ArrayRef,
        ArrowCastOptions::new().with_safe(true),
    )
    .unwrap();
    assert_eq!(safe.scalar(1).unwrap(), Scalar::Null);
}

#[test]
fn text_lands_as_the_state_it_spells_and_a_state_renders_its_name() {
    let field = Field::new("state", DataType::State, false);
    let landed = Serie::from_arrow_array(
        Some(&field),
        Arc::new(StringArray::from(vec!["1", "Filled", "ACKNOWLEDGED"])) as ArrayRef,
        strict(),
    )
    .unwrap();
    assert_eq!(
        landed.scalar(0).unwrap(),
        Scalar::State(State::PartiallyFilled)
    );
    assert_eq!(landed.scalar(1).unwrap(), Scalar::State(State::Filled));
    assert_eq!(
        landed.scalar(2).unwrap(),
        Scalar::State(State::Acknowledged)
    );

    let text = landed
        .cast(&Field::new("state", DataType::utf8(), false), strict())
        .unwrap()
        .require_arrow_array()
        .unwrap();
    let names = text.as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(
        names.iter().collect::<Vec<_>>(),
        [
            Some("PARTIALLY_FILLED"),
            Some("FILLED"),
            Some("ACKNOWLEDGED")
        ]
    );
    let codes = landed
        .cast(&Field::new("state", DataType::Int32, false), strict())
        .unwrap()
        .require_arrow_array()
        .unwrap();
    let codes = codes.as_any().downcast_ref::<Int32Array>().unwrap();
    assert_eq!(codes.values().as_ref(), [4001, 8003, 2004]);
}

#[test]
fn a_state_crosses_the_value_stream_and_the_structured_codecs_as_itself() {
    for state in [State::Unknown, State::New, State::DontKnow] {
        let value = Scalar::State(state);
        let bytes = value.into_value_bytes();
        assert_eq!(bytes[1], DataTypeId::State.as_u8());
        assert_eq!(bytes[2..], state.code().to_le_bytes());
        assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
        // A digest reads the code, so a member's name is free to change.
        assert_ne!(
            value.stable_hash(),
            Scalar::State(State::Filled).stable_hash(),
            "{state}"
        );
    }
    let json: serde_json::Value = serde_json::to_value(State::Acknowledged).unwrap();
    assert_eq!(json, serde_json::json!("ACKNOWLEDGED"));
    let read: State = serde_json::from_value(serde_json::json!(2004)).unwrap();
    assert_eq!(read, State::Acknowledged);
    let read: State = serde_json::from_value(serde_json::json!("Acknowledged")).unwrap();
    assert_eq!(read, State::Acknowledged);
}

#[test]
fn a_state_filters_casts_partitions_and_embeds_by_its_member() {
    use yggdryl::expression::{Expression, Filter};
    use yggdryl::text::{self, Format, Loading, Placeholders};

    let root = DataType::from(
        yggdryl::StructType::from_fields([DataType::State.nullable_field("state")]).unwrap(),
    )
    .required_field("row");
    let states = [
        State::New,
        State::PartiallyFilled,
        State::Filled,
        State::Rejected,
    ];
    let column = Serie::from_scalars(
        root.fields()[0].clone(),
        states
            .iter()
            .map(|state| Scalar::State(*state))
            .collect::<Vec<_>>(),
    )
    .unwrap()
    .require_arrow_array()
    .unwrap();
    let batch =
        arrow_array::RecordBatch::try_new(root.clone().into_arrow_schema().unwrap(), vec![column])
            .unwrap();

    // A text constant meets the column as the member it spells, and the
    // codes order by lifecycle.
    for (clause, kept) in [
        ("state = 'FILLED'", 1),
        ("state in ('NEW', 'PartFill')", 2),
        ("state >= 'FILLED'", 2),
        ("state = state 'REJECTED'", 1),
        ("cast('2' as state) = state", 1),
    ] {
        let filter: Filter = clause.parse().unwrap();
        assert_eq!(
            filter.apply_arrow_batch(&batch).unwrap().num_rows(),
            kept,
            "{clause}"
        );
    }
    // An integer meets the column as the code it stores: a member's code
    // coerces into the column, and any other bound compares as an integer.
    for (clause, kept) in [
        ("where state < 8000", 2),
        ("where state < 8500", 3),
        ("where state >= 9000", 1),
        ("where cast(state as int64) = 4001", 1),
    ] {
        let expression: Expression = clause.parse().unwrap();
        assert_eq!(
            expression.apply_arrow_batch(&batch).unwrap().num_rows(),
            kept,
            "{clause}"
        );
    }

    // A partition directory names the member, the spelling the column reads
    // back, and embedded text is the same name.
    assert_eq!(
        yggdryl::media::partition::partition_text(&Scalar::State(State::Filled)).unwrap(),
        "FILLED"
    );
    let loading = Loading::new()
        .with_placeholders(Placeholders::new().with_variable("S", Scalar::State(State::New)));
    let value = text::from_utf8_with("value = \"is {{ S }}\"\n", Format::Toml, &loading).unwrap();
    assert_eq!(value.get_key_str("value"), Some(&Scalar::from("is NEW")));
}
