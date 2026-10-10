//! `rust/src/state.rs`: what state one thing is in, a lifecycle-sorted enum
//! stored as the `int32` code of its member.
//!
//! `DataType` is `#[non_exhaustive]` and the datatype layer carries some sixty
//! wildcard arms, so a new variant compiles clean while behaving wrongly. A
//! green build proves nothing; these are the invariants a wildcard cannot
//! satisfy by accident.
//!
//! What FIX states about a state - each status field read under its own code
//! set, and the state a message type asks for - is `rust/fix/src/state.rs`'s,
//! pinned by `rust/fix/tests/root/state.rs`; the wire codes `OrdStatus(39)` and
//! `ExecType(150)` share stay here, because `State::from_spelling` reads them.

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
    assert!(State::read_code(-1).is_err());
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
    let codes: Vec<u16> = State::ALL.iter().map(|state| state.code()).collect();
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
        assert_eq!(usize::from(place), before, "{state}");
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
        State::Approved,
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
    // `UPDATED` is a working state, so later progress folds over it exactly
    // as it folds over `RUNNING`.
    assert_eq!(State::New.merge_with(State::Updated), State::Updated);
    assert_eq!(
        State::Updated.merge_with(State::PartiallyFilled),
        State::PartiallyFilled
    );
    assert_eq!(State::Updated.merge_with(State::New), State::Updated);
}

#[test]
fn updated_is_the_working_band_stated_anew_over_a_live_predecessor() {
    assert_eq!(State::ALL.len(), 62);
    assert_eq!(State::Updated.code(), 3004);
    assert_eq!(State::Updated.rank(), 30);
    assert_eq!(State::Updated.as_str(), "UPDATED");
    assert_eq!(State::from_spelling("UPDATED"), Some(State::Updated));
    assert_eq!(State::from_spelling("updated"), Some(State::Updated));
    assert_eq!(State::from_code(3004), Some(State::Updated));
    assert!(State::Updated.is_live());
    assert!(State::Updated.description().contains("anew"));
    // New-like: acknowledged and working, or carrying on after a change -
    // never the pending band, so `PENDING_NEW -> NEW` stays `NEW`.
    for state in State::ALL {
        let expected = matches!(state.rank(), 20 | 30)
            || matches!(
                state,
                State::Updated | State::Replaced | State::Restated | State::Amended
            );
        assert_eq!(state.is_new_like(), expected, "{state}");
    }
    for state in [
        State::Accepted,
        State::New,
        State::Starting,
        State::Submitted,
        State::Acknowledged,
        State::Running,
        State::Status,
        State::Triggered,
        State::Active,
        State::Updated,
        State::Replaced,
        State::Restated,
        State::Amended,
    ] {
        assert!(state.is_new_like(), "{state}");
    }
    for state in [
        State::Unknown,
        State::Pending,
        State::PendingNew,
        State::PartiallyFilled,
        State::Paused,
        State::PendingCancel,
        State::Released,
        State::Filled,
        State::Canceled,
        State::Rejected,
    ] {
        assert!(!state.is_new_like(), "{state}");
    }
}

#[test]
fn a_state_answers_the_enum_contract_every_enum_leaf_owes() {
    use yggdryl::EnumValue;

    assert_eq!(State::NAME, "state");
    assert_eq!(State::EXTENSION_NAME, "yggdryl.state");
    assert_eq!(<State as EnumValue>::ALL, State::ALL);
    assert_eq!(EnumValue::code(State::Filled), 8003);
    assert_eq!(EnumValue::as_str(State::Filled), "FILLED");
    assert_eq!(<State as EnumValue>::from_code(8003), Some(State::Filled));
    assert_eq!(<State as EnumValue>::read("Filled").unwrap(), State::Filled);
    assert_eq!(
        <State as EnumValue>::read_code(8003).unwrap(),
        State::Filled
    );
    assert!(<State as EnumValue>::read_code(4242).is_err());
    assert!(DataType::State.is_enum());
    assert!(!DataType::Int32.is_enum());
    let value = Scalar::State(State::Filled);
    assert!(value.is_enum());
    assert_eq!(value.enum_code(), Some(8003));
    assert_eq!(value.enum_name(), Some("FILLED"));
    assert_eq!(Scalar::from(8003_i32).enum_code(), None);
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

/// A trade report awaiting its verification, an allocation awaiting its
/// making and a give-up awaiting its approval were each acknowledged first,
/// so they rank past their acknowledgement and below every answer to them:
/// the fold keeps the answer whichever order it arrives in.
#[test]
fn a_state_awaiting_its_next_step_ranks_past_its_acknowledgement_and_below_its_answers() {
    for (state, code) in [
        (State::PendingVerification, 4006),
        (State::PendingAllocation, 4007),
        (State::PendingApproval, 4008),
    ] {
        assert_eq!(state.code(), code, "{state}");
        assert_eq!(state.rank(), 40, "{state}");
        assert_eq!(State::from_code(code), Some(state), "{state}");
        assert!(!state.is_pending(), "{state}");
        assert!(state.is_live(), "{state}");
        assert!(!state.is_new_like(), "{state}");
    }
    // The codes they were stored under before name nothing.
    for code in 1004..=1006 {
        assert_eq!(State::from_code(code), None, "{code}");
    }
    // Past the acknowledgement, below the answers.
    assert_eq!(
        State::PendingVerification.merge_with(State::Accepted),
        State::PendingVerification
    );
    assert_eq!(
        State::Accepted.merge_with(State::PendingVerification),
        State::PendingVerification
    );
    assert_eq!(
        State::Received.merge_with(State::PendingAllocation),
        State::PendingAllocation
    );
    assert_eq!(
        State::Disputed.merge_with(State::PendingVerification),
        State::Disputed
    );
    assert_eq!(
        State::PendingVerification.merge_with(State::Disputed),
        State::Disputed
    );
    assert_eq!(
        State::PendingCancel.merge_with(State::PendingVerification),
        State::PendingCancel
    );
    assert_eq!(
        State::Verified.merge_with(State::PendingVerification),
        State::Verified
    );
    assert_eq!(
        State::Incomplete.merge_with(State::PendingAllocation),
        State::Incomplete
    );
    assert_eq!(
        State::Allocated.merge_with(State::PendingApproval),
        State::Allocated
    );
    assert_eq!(
        State::PendingApproval.merge_with(State::Approved),
        State::Approved
    );

    // Approved ends an approval as verified ends a verification.
    assert_eq!(State::Approved.code(), 8013);
    assert_eq!(State::Approved.rank(), 80);
    assert!(State::Approved.is_done());
    assert_eq!(State::Approved.as_str(), "APPROVED");
    assert_eq!(State::from_code(8013), Some(State::Approved));
    assert_eq!(State::from_spelling("approved"), Some(State::Approved));
    assert_eq!(State::from_spelling("APPROVED"), Some(State::Approved));
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
fn a_column_is_uint16_codes_under_the_state_extension() {
    let field = Field::new("state", DataType::State, true);
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt16);
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
    let codes = array
        .as_any()
        .downcast_ref::<arrow_array::UInt16Array>()
        .unwrap();
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
        // The canonical four bytes, whatever width a column stores.
        assert_eq!(bytes[2..], i32::from(state.code()).to_le_bytes());
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

/// The wire contract of `state` as the split found it, pinned so that the
/// kind leaving the core (`DataType::Market`, S1; `yggdryl-market`, S4)
/// moves no byte: the identifier byte and name, the Arrow extension and its
/// storage in both directions, the serde tags, the value-stream bytes, the
/// canonical digests and the names that reach it. Generated from the facts
/// the tree answered on `2ae975674`, never typed by hand.
#[test]
fn the_state_wire_contracts_are_pinned() {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, UInt16Array};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{
        ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, Field, FieldRecord,
        Scalar, Serie, StructType,
    };

    // Built through the doors that survive the kind leaving the core: the
    // parsed name and the datatype's own value door.
    let dtype = DataType::from_str("state").unwrap();
    let value = dtype.scalar("PENDING_NEW").unwrap();

    // Byte and name.
    assert_eq!(dtype.id(), DataTypeId::from_u8(0xc1).unwrap());
    assert_eq!(dtype.id().as_u8(), 0xc1);
    assert_eq!(dtype.id().as_str(), "state");
    assert_eq!(dtype.id().kind(), DataTypeKind::Enum);
    assert_eq!(dtype.to_string(), "state");
    assert_eq!(value.kind(), "state");
    assert_eq!(value.id().as_u8(), 0xc1);
    assert_eq!(value.dtype().unwrap(), dtype);
    assert!(dtype.is_enum() && !dtype.is_code());
    assert_eq!(dtype.code_width(), None);
    assert_eq!(value.enum_code(), Some(1001_u16));
    assert_eq!(value.enum_name(), Some("PENDING_NEW"));

    // Names: the parser, the field grammar and the logical names.
    assert_eq!(dtype.id().as_str(), "state");
    assert_eq!(Field::from_str("value state").unwrap().dtype(), &dtype);
    assert_eq!(DataType::from_logical_name("state").unwrap(), dtype);
    let logical: Vec<&str> = DataType::logical_names()
        .iter()
        .filter(|(_, held)| *held == dtype)
        .map(|(logical, _)| *logical)
        .collect();
    assert_eq!(logical, ["state"]);

    // Arrow: the extension name over its storage, both directions.
    let field = dtype.clone().nullable_field("value");
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt16);
    assert_eq!(dtype.id().arrow_extension_name(), Some("yggdryl.state"));
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.state")
    );
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:metadata")
            .map(String::as_str),
        Some("")
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let storage: ArrayRef = Arc::new(UInt16Array::from(vec![1001_u16]));
    let landed = Serie::from_arrow_array(Some(&field), storage, ArrowCastOptions::new()).unwrap();
    assert_eq!(landed.scalar(0).unwrap(), value);
    assert_eq!(landed.field().unwrap(), &field);

    // Serde: the tags of the datatype, the field and the value.
    assert_eq!(
        serde_json::to_string(&dtype).unwrap(),
        r#"{"type":"state"}"#
    );
    assert_eq!(
        serde_json::from_str::<DataType>(r#"{"type":"state"}"#).unwrap(),
        dtype
    );
    assert_eq!(
        serde_json::to_string(&field).unwrap(),
        r#"{"name":"value","dtype":{"type":"state"},"nullable":true,"metadata":{}}"#
    );
    assert_eq!(
        serde_json::from_str::<Field>(
            r#"{"name":"value","dtype":{"type":"state"},"nullable":true,"metadata":{}}"#
        )
        .unwrap(),
        field
    );
    assert_eq!(
        serde_json::to_string(&value).unwrap(),
        r#"{"type":"state","value":"PENDING_NEW"}"#
    );
    assert_eq!(
        serde_json::from_str::<Scalar>(r#"{"type":"state","value":"PENDING_NEW"}"#).unwrap(),
        value
    );

    // Value stream: the bytes of the value, under its own tag and under the
    // datatype.
    let bytes = value.into_value_bytes();
    assert_eq!(bytes, [0, 193, 233, 3, 0, 0]);
    assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
    assert_eq!(
        dtype
            .encode_value_bytes(&Scalar::from("PENDING_NEW"))
            .unwrap(),
        bytes
    );
    assert_eq!(dtype.decode_value_bytes(&bytes).unwrap(), value);

    // Digest: the canonical feed of the value, of a row holding it, and of
    // the datatype.
    assert_eq!(value.stable_hash(), 16325037442088800966);
    assert_eq!(
        value.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(16325037442088800966)
    );
    let root =
        DataType::from(StructType::from_fields([field.clone()]).unwrap()).required_field("row");
    let record = FieldRecord::new(&root, Scalar::from_sequence([value.clone()])).unwrap();
    assert_eq!(
        record.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(5632565356988565939)
    );
    assert_eq!(dtype.stable_hash(), 7917036384617311412);
}

#[cfg(feature = "internals")]
mod internal {
    //! The std-hash feed of `state`, which no caller names.

    /// The std `Hash` feed D19 keeps - the value's rank then its identity,
    /// the datatype's `Shape` position - read through the door that feeds it
    /// to XXH3, pinned so the kind leaving the core moves no persisted digest.
    #[test]
    fn the_state_std_hash_feed_is_pinned() {
        use yggdryl::DataType;

        let dtype = DataType::from_str("state").unwrap();
        let value = dtype.scalar("PENDING_NEW").unwrap();
        assert_eq!(
            yggdryl::internals::scalar::stable_hash_of(&value),
            2086077814929649598
        );
        assert_eq!(
            yggdryl::internals::hashing_stable::stable_hash_of(&dtype),
            4057639545713044219
        );
    }
}
