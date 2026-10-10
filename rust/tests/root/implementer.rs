//! `rust/src/implementer.rs`: the one public door the crates this core is
//! split into reach its crate-private items through.
//!
//! The module is `pub`, so this is a caller-facing test. It names every item
//! the door exports - a name dropped from the door, or one that lost its
//! `pub`, stops it compiling - and then, for each route an item takes, pins
//! that the door answers what its target answers: an item republished from a
//! module the crate root does not publish, a forwarder over an item of a
//! published module, a free forwarder over an associated item, a definition
//! moved into the door, and an exported macro whose expansion names the door.

use std::borrow::Cow;
use std::sync::Arc;

use arrow_array::{ArrayRef, Int32Array, RecordBatch, StringArray};
use yggdryl::graph::{Element, Event};
use yggdryl::implementer::{
    BBG_WIDTH, BOOLEAN_SPELLINGS, CCY_WIDTH, CFI_UNCLASSIFIED, DEFAULT_BATCH_ROW_SIZE, DTI_WIDTH,
    ELF_WIDTH, ERROR_TEXT_LIMIT, FISN_WIDTH, InstantSequence, LEI_WIDTH, Patterns, RIC_WIDTH,
    ROW_OVERHEAD, Resolved, SOH_MARKERS, Segment, Staged, below_threshold, bool_from_text, bool_of,
    canonicalize_uuids, cfi_from_proven, civil_from_days, crosshash, datetime64_from_fix_clock,
    datetime64_from_fix_text, declared_charset, digest_u64, document_behind_prefix, earliest,
    elide_display, elide_to, expected_got, feed_event_facts, field_new_with_metadata, fold_digest,
    folded, folded_spelling, folds_equal, inspect, integer_from_text_as, is_null_like,
    isin_from_proven, json_span, land_unproven_batch, latest, locate, merge_element,
    metadata_shares_storage_with, metadata_storage_address, mic_from_market, mic_from_proven,
    moved, normalized, ordered, payload_at, percent_decode, read_enum_spelling, reader,
    result_reader, right_is_reference, scalar_from_decimal_text, scalar_leaf_display,
    scalar_try_build_sequence, scalar_try_fill_sequence, scalar_try_sequence,
    serie_is_byte_storage, serie_is_string_storage, serie_value_bytes, similarity, stable_hash_of,
    state_from_wire_code, stated, str_from_value, struct_type_from_checked_fields,
    struct_type_from_unique_fields, struct_type_shares_storage_with, struct_type_storage_address,
    time32_from_fix_text, time64_from_fix_text, trim_ascii, whole_u64, write_named_bytes,
};
use yggdryl::text::{TextBytes, TextLine, TextOptions};
use yggdryl::xxhash::{Xxh3, xxh3};
use yggdryl::{
    Cfi, Charset, DataType, Decimal, Error, Field, Isin, Metadata, Mic, Scalar, Serie, State,
    StructType, Time32, Time64, Timezone, Uuid,
};

/// A line at `transunix`: the core's own event, holding one byte of body.
fn line(transunix: i64) -> TextLine {
    let mut line = TextLine::from_bytes(
        0,
        TextBytes::from_bytes(b"x").unwrap(),
        Arc::new(TextOptions::new()),
    )
    .unwrap();
    line.set_transunix(transunix);
    line
}

/// Every name the door exports, imported by name and used by nothing here:
/// the tests below use the ones that answer something a caller can compare,
/// and this list is the one place that states the whole door, so a name
/// dropped from it, renamed or no longer `pub` breaks the build.
#[allow(unused_imports)]
mod every_name {
    use yggdryl::implementer::{
        ArrowDataType, ArrowError, BBG_WIDTH, BOOLEAN_SPELLINGS, CCY_WIDTH, CFI_UNCLASSIFIED,
        Closing, DEFAULT_BATCH_ROW_SIZE, DTI_WIDTH, ELF_WIDTH, ERROR_TEXT_LIMIT, Elided,
        ElidedDisplay, ExtensionType, FISN_WIDTH, InstantSequence, LEI_WIDTH, Located, Ordered,
        Path, Patterns, RIC_WIDTH, ROW_OVERHEAD, Resolved, SOH_MARKERS, Segment, Staged,
        adopt_market_code, arrow_serie, below_threshold, bool_from_text, bool_of, bytes_dtypes,
        bytes_scalars, canonical_closing_reader, canonicalize_uuids, cfi_from_proven,
        civil_from_days, crosshash, datetime64_from_fix_clock, datetime64_from_fix_text,
        declared_charset, define_field_types, digest_u64, document_behind_prefix, earliest,
        elide_display, elide_to, enum_leaf, expected_got, feed_event_facts,
        field_from_arrow_schema, field_new_with_metadata, fold_digest, fold_event_instants, folded,
        folded_spelling, folds_equal, follow_element, follow_timed, inspect, integer_from_text_as,
        is_null_like, isin_from_proven, json_span, land_unproven_batch, latest, locate,
        marker_metadata, marker_supports, market_extension, merge_element, merge_event_element,
        merge_timed, metadata_shares_storage_with, metadata_storage_address, mic_from_market,
        mic_from_proven, moved, normalized, ordered, payload_at, percent_decode,
        protocol_field_types, read_enum_spelling, reader, record_parts, restate_event,
        result_reader, right_is_reference, scalar_from_decimal_text, scalar_leaf_display,
        scalar_try_build_sequence, scalar_try_fill_sequence, scalar_try_sequence,
        serie_is_byte_storage, serie_is_string_storage, serie_value_bytes, similarity,
        stable_hash_of, state_from_wire_code, stated, str_from_value, string_scalars,
        struct_type_from_checked_fields, struct_type_from_unique_fields,
        struct_type_shares_storage_with, struct_type_storage_address,
        text_entries_from_bytes_direct_located, time32_from_fix_text, time64_from_fix_text,
        trim_ascii, warn, warned, whole_u64, with_field, write_named_bytes,
    };
}

/// The door as one glob: what a crate that takes it whole writes.
mod glob {
    use yggdryl::implementer::*;

    #[test]
    fn the_whole_door_imports_as_one_glob() {
        assert_eq!(CCY_WIDTH, 8);
        assert!(folds_equal("Part-Filled", "partfilled"));
    }
}

/// The record the row doors widen rows under.
fn record() -> Field {
    StructType::from_fields([
        DataType::Int32.required_field("id"),
        DataType::utf8().nullable_field("name"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row")
}

/// Two rows of the record, as native scalars.
fn rows() -> Vec<Scalar> {
    vec![
        Scalar::from_sequence([Scalar::from(1_i32), Scalar::from("a")]),
        Scalar::from_sequence([Scalar::from(2_i32), Scalar::Null]),
    ]
}

// ------------------------------------------------------------------------
// Republished items: the door shows the item itself.
// ------------------------------------------------------------------------

#[test]
fn the_fold_is_the_crates_one_fold() {
    assert!(folds_equal("UTC_Timestamp", "utc-timestamp"));
    assert!(!folds_equal("price", "qty"));
    assert_eq!(normalized("Part-Filled_X"), "partfilledx");
    assert_eq!(folded("A-b c").collect::<String>(), "abc");
    // Two spellings one fold calls one digest, so an index keyed by it finds
    // either.
    assert_eq!(fold_digest("Part Filled"), fold_digest("part_filled"));
    assert_ne!(fold_digest("price"), fold_digest("qty"));
    // The forwarder over the code fold agrees with the parser's.
    assert_eq!(folded_spelling("Part-Filled_X").as_str(), "partfilledx");
    assert!(is_null_like("N/A"));
    assert!(is_null_like(""));
    assert!(!is_null_like("ACME"));
}

#[test]
fn a_walks_path_renders_from_its_root() {
    let root = yggdryl::implementer::Path::root();
    let list = root.field("rows");
    let third = list.child(Segment::Index(3));
    let quoted = third.field("zip code");
    assert_eq!(root.render(), "$");
    assert_eq!(list.render(), "$.rows");
    assert_eq!(third.to_string(), "$.rows[3]");
    assert_eq!(quoted.render(), "$.rows[3][\"zip code\"]");
}

#[test]
fn bounded_text_is_elided_at_the_error_budget() {
    assert_eq!(ERROR_TEXT_LIMIT, 64);
    let long = "x".repeat(100);
    let elided = elide_to(&long, ERROR_TEXT_LIMIT).to_string();
    assert_eq!(elided.chars().count(), ERROR_TEXT_LIMIT + 1);
    assert!(elided.ends_with('\u{2026}'));
    assert_eq!(elide_to("short", ERROR_TEXT_LIMIT).to_string(), "short");
    assert_eq!(elide_display(&"short").to_string(), "short");
    assert_eq!(
        expected_got("a number", 5).as_str(),
        "expected a number, got 5"
    );
}

#[test]
fn native_rows_widen_into_bounded_batches() {
    assert_eq!(
        DEFAULT_BATCH_ROW_SIZE,
        yggdryl::media::DEFAULT_RECORD_BATCH_ROW_SIZE
    );
    let mut batches = reader(&record(), rows(), Some(1), None, None).unwrap();
    assert_eq!(batches.next().unwrap().unwrap().num_rows(), 1);
    assert_eq!(batches.next().unwrap().unwrap().num_rows(), 1);
    assert!(batches.next().is_none());

    let mut results =
        result_reader(&record(), rows().into_iter().map(Ok), None, None, None).unwrap();
    assert_eq!(results.next().unwrap().unwrap().num_rows(), 2);
    assert!(results.next().is_none());

    let landed = reader(&record(), rows(), None, None, None).unwrap();
    assert!(yggdryl::implementer::arrow_serie(landed).is_ok());
}

#[test]
fn a_batch_lands_as_the_record_of_its_root_with_every_leaf_read() {
    let root = record();
    let batch = RecordBatch::try_new(
        root.clone().into_arrow_schema().unwrap(),
        vec![
            Arc::new(Int32Array::from(vec![1, 2])) as ArrayRef,
            Arc::new(StringArray::from(vec![Some("a"), None])) as ArrayRef,
        ],
    )
    .unwrap();
    let serie = land_unproven_batch(&Resolved::of(Arc::new(root)), batch).unwrap();
    assert_eq!(serie.len(), 2);
}

#[test]
fn the_ordered_map_answers_in_the_items_order() {
    let doubled: Vec<u32> = ordered(0..100_u32, 4, 3, |n| n * 2).collect();
    assert_eq!(doubled, (0..100_u32).map(|n| n * 2).collect::<Vec<_>>());
    // One thread answers each item as it is pulled.
    let alone: Vec<u32> = ordered(0..10_u32, 1, 3, |n| n + 1).collect();
    assert_eq!(alone, (1..=10_u32).collect::<Vec<_>>());
}

#[test]
fn the_stable_hash_is_the_structural_one() {
    let dtype = DataType::from_str("state").unwrap();
    // The std `Hash` feed XXH3 reads, which `root/state.rs` pins through the
    // `internals` door as the same number.
    assert_eq!(stable_hash_of(&dtype), 4_057_639_545_713_044_219);
    assert_eq!(stable_hash_of(&dtype), stable_hash_of(&dtype.clone()));
    assert_ne!(stable_hash_of(&dtype), stable_hash_of(&DataType::Int32));
}

#[test]
fn spelling_patterns_read_a_member_by_its_words() {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Force {
        Good,
        Immediate,
    }

    let patterns = Patterns::new(
        [
            ("GOOD_TILL_CANCEL", Force::Good),
            ("IMMEDIATE_OR_CANCEL", Force::Immediate),
        ],
        [],
    );
    assert_eq!(patterns.read("GOOD_TILL_CANCEL"), Some(Force::Good));
    assert_eq!(patterns.read(" good till cancel "), Some(Force::Good));
    assert_eq!(patterns.read("IMMEDIATE_OR_CANCEL"), Some(Force::Immediate));
    assert_eq!(patterns.read("zzzz"), None);
}

#[test]
fn the_line_scan_reads_prose_as_carrying_no_frame() {
    assert_eq!(trim_ascii(b"  a b \n"), b"a b".as_slice());
    assert_eq!(SOH_MARKERS.len(), 4);
    assert_eq!(SOH_MARKERS[0], b"^A".as_slice());
    assert!(SOH_MARKERS.contains(&b"<SOH>".as_slice()));
    assert!(json_span(b"plain prose").is_none());
    assert!(payload_at(b"plain prose").is_none());
    assert!(document_behind_prefix(b"plain prose").is_none());
    assert!(inspect(b"plain prose").msgtype().is_none());
}

// ------------------------------------------------------------------------
// Forwarders over items of published modules.
// ------------------------------------------------------------------------

#[test]
fn the_boolean_door_reads_the_one_table_every_flag_reads() {
    assert_eq!(BOOLEAN_SPELLINGS, "true/false, yes/no, y/n, on/off or 1/0");
    assert_eq!(bool_from_text(" Y "), Some(true));
    assert_eq!(bool_from_text("off"), Some(false));
    assert_eq!(bool_from_text("maybe"), None);
    assert_eq!(bool_of(&Scalar::from(true)), Some(true));
    assert_eq!(bool_of(&Scalar::from("no")), Some(false));
    assert_eq!(bool_of(&Scalar::Null), None);
}

#[test]
fn the_integer_door_narrows_exactly_and_never_wraps() {
    assert_eq!(integer_from_text_as::<u8>("255"), Some(255));
    assert_eq!(integer_from_text_as::<u8>("256"), None);
    assert_eq!(integer_from_text_as::<i8>("-128"), Some(-128));
    assert_eq!(integer_from_text_as::<i32>(" 42 "), Some(42));
    assert_eq!(integer_from_text_as::<i32>("4x"), None);
    assert_eq!(
        integer_from_text_as::<u128>("340282366920938463463374607431768211455"),
        Some(u128::MAX)
    );
}

#[test]
fn the_element_doors_fold_two_statements_of_one_element() {
    let mut this = line(0);
    let mut other = line(0);
    other.set_crosscode("T-1".to_owned());
    other.set_srcuuids(vec![Uuid::from_v8(7)]);

    assert!(merge_element(&mut this, &other));
    // A line's cross code is stored as it was stated.
    assert_eq!(this.get_crosscode(), "T-1");
    assert_eq!(this.get_srcuuids(), [Uuid::from_v8(7)]);
    assert!(!merge_element(&mut this, &other), "nothing left to take");

    assert_eq!(crosshash("T-1"), xxh3(b"T-1"));
    let mut uuids = vec![Uuid::from_v8(3), Uuid::from_v8(1), Uuid::from_v8(3)];
    canonicalize_uuids(&mut uuids);
    assert_eq!(uuids, [Uuid::from_v8(1), Uuid::from_v8(3)]);
}

#[test]
fn the_instant_doors_pick_the_earlier_the_later_and_the_stated() {
    assert_eq!(earliest(Some(3), Some(5)), Some(3));
    assert_eq!(earliest(None, Some(5)), Some(5));
    assert_eq!(latest(Some(3), Some(5)), Some(5));
    assert_eq!(latest(Some(3), None), Some(3));
    assert_eq!(latest(None, None), None);
    assert_eq!(stated(Some(1), Some(2), false), Some(1));
    assert_eq!(stated(Some(1), Some(2), true), Some(2));
    assert_eq!(stated(None, Some(2), false), Some(2));

    let mut held = 1;
    assert!(moved(held, 2, |next| held = next));
    assert_eq!(held, 2);
    assert!(!moved(held, 2, |_| unreachable!(
        "equal facts move nothing"
    )));

    // The most recently recorded statement leads; equal clocks fall back to
    // the later event instant, and an exact tie keeps the left.
    assert!(right_is_reference(Some(1), 9, Some(2), 0));
    assert!(!right_is_reference(Some(2), 0, Some(1), 9));
    assert!(right_is_reference(None, 0, Some(1), 0));
    assert!(!right_is_reference(Some(1), 0, None, 9));
    assert!(right_is_reference(None, 1, None, 2));
    assert!(!right_is_reference(None, 2, None, 2));
}

#[test]
fn the_digest_doors_feed_what_an_event_digest_reads() {
    // What the door feeds is the state and the predecessor's identity: the
    // cross code and the instant are the holder's to feed.
    let digest = |event: &TextLine| {
        let mut state = Xxh3::new();
        feed_event_facts(&mut state, event);
        state.as_u64()
    };
    let plain = line(1);
    let mut filled = line(1);
    filled.set_state(State::Filled);
    assert_eq!(digest(&plain), digest(&plain.clone()));
    assert_ne!(digest(&plain), digest(&filled));

    let framed = |value: &Scalar| {
        let mut sink = Xxh3::new();
        write_named_bytes(&mut sink, [("id", value)].into_iter(), 0);
        sink.as_u64()
    };
    let (one, two) = (Scalar::from(1_i32), Scalar::from(2_i32));
    assert_eq!(framed(&one), framed(&one));
    assert_ne!(framed(&one), framed(&two));
}

#[test]
fn a_cell_states_the_whole_number_or_the_digest_bits_it_holds() {
    assert_eq!(whole_u64(&Scalar::from(5_i32)), Some(5));
    assert_eq!(whole_u64(&Scalar::from(-1_i32)), None);
    assert_eq!(whole_u64(&Scalar::Null), None);
    assert_eq!(digest_u64(&Scalar::from(5_u64)), Some(5));
    // A table with no unsigned type stores an XXH3-64 as the `long` of its
    // width, so a negative cell is the bit pattern it is.
    assert_eq!(digest_u64(&Scalar::from(-1_i64)), Some(u64::MAX));
}

#[test]
fn the_enum_doors_read_a_spelling_as_the_member_it_names() {
    assert_eq!(
        read_enum_spelling(&DataType::State, "Filled").unwrap(),
        Scalar::State(State::Filled)
    );
    assert!(read_enum_spelling(&DataType::State, "zzzz").is_err());
    assert!(read_enum_spelling(&DataType::Int32, "Buy").is_err());

    assert_eq!(state_from_wire_code("F"), Some(State::Trade));
    assert_eq!(state_from_wire_code("a"), None);
}

#[test]
fn the_temporal_doors_read_what_fix_spells() {
    let clock = Time32::from_text("09:30:00").unwrap();
    assert_eq!(time32_from_fix_text("09:30:00").unwrap(), clock);
    // FIX's own spelling: a clock that stops at its minutes.
    assert_eq!(time32_from_fix_text("09:30").unwrap(), clock);
    assert!(time32_from_fix_text("09:30:00Z").is_err());
    assert_eq!(
        time64_from_fix_text("09:30:00").unwrap(),
        Time64::from_text("09:30:00").unwrap()
    );

    assert!(datetime64_from_fix_text("20240102-10:00:00.500", Timezone::UTC).is_ok());
    assert!(datetime64_from_fix_text("not a time", Timezone::UTC).is_err());
    assert!(datetime64_from_fix_clock("10:00:00", Timezone::UTC).is_ok());
    assert!(datetime64_from_fix_clock("20240102-10:00:00", Timezone::UTC).is_err());

    assert_eq!(civil_from_days(0), (1970, 1, 1));
    assert_eq!(civil_from_days(19_723), (2024, 1, 1));
}

#[test]
fn percent_escapes_decode_to_the_text_they_stand_for() {
    assert_eq!(percent_decode("a%2Fb%20c", "uri").unwrap(), "a/b c");
    assert!(matches!(
        percent_decode("plain", "uri").unwrap(),
        Cow::Borrowed("plain")
    ));
    assert!(percent_decode("%zz", "uri").is_err());
    assert!(percent_decode("%FF", "uri").is_err());
}

#[test]
fn an_xml_declaration_names_the_charset_the_document_is_in() {
    assert_eq!(
        declared_charset(b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><a/>").unwrap(),
        Some(Charset::Utf8)
    );
    assert_eq!(
        declared_charset(b"<?xml version=\"1.0\" encoding='ISO-8859-1'?><a/>").unwrap(),
        Some(Charset::Latin1)
    );
    assert_eq!(
        declared_charset(b"<?xml version=\"1.0\"?><a/>").unwrap(),
        None
    );
    assert_eq!(declared_charset(b"plain text").unwrap(), None);
    assert!(declared_charset(b"<?xml version=\"1.0\" encoding=\"no-such-charset\"?>").is_err());
}

#[test]
fn a_folder_that_holds_no_table_is_located_as_none() {
    let mut root = yggdryl::local::LocalFolder::temporary()
        .unwrap()
        .path()
        .unwrap();
    root.push(format!("yggdryl-implementer-locate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a writable temporary root");

    let folder = yggdryl::local::LocalFolder::new(&root).unwrap();
    assert!(locate(&folder).unwrap().is_none());
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn short_names_score_by_their_distance_and_their_lengths_alone_can_rule_one_out() {
    let alike = |left: &str, right: &str, expected: f64| {
        assert!(
            (similarity(left, right) - expected).abs() < f64::EPSILON,
            "{left} {right}"
        );
    };
    alike("", "", 1.0);
    alike("ACME CORP", "ACME CORP", 1.0);
    alike("abc", "xyz", 0.0);
    // Lengths 1 and 100 differ by more than 15% of the longer: no distance
    // can make up for it.
    assert!(below_threshold(1, 100, 0.85));
    assert!(!below_threshold(10, 10, 0.85));
}

// ------------------------------------------------------------------------
// Free forwarders over associated items.
// ------------------------------------------------------------------------

#[test]
fn a_proven_code_is_adopted_as_the_code_it_is() {
    assert_eq!(
        isin_from_proven("US0378331005"),
        Isin::new("US0378331005").unwrap()
    );
    assert_eq!(mic_from_proven("XNAS"), Mic::new("XNAS").unwrap());
    assert_eq!(cfi_from_proven("ESVUFR"), Cfi::new("ESVUFR").unwrap());
    assert_eq!(mic_from_market("XNAS"), Some(Mic::new("XNAS").unwrap()));
    assert_eq!(mic_from_market("not a market"), None);
    assert_eq!(CFI_UNCLASSIFIED, "XXXXXX");
    assert_eq!(CFI_UNCLASSIFIED.len(), Cfi::LENGTH);
}

#[test]
fn the_width_constants_are_the_widths_the_code_datatypes_state() {
    assert_eq!(DataType::Ccy.code_width(), Some(CCY_WIDTH));
    assert_eq!(DataType::Bbg.code_width(), Some(BBG_WIDTH));
    assert_eq!(DataType::Ric.code_width(), Some(RIC_WIDTH));
    assert_eq!(DataType::lei().code_width(), Some(LEI_WIDTH));
    assert_eq!(DataType::dti().code_width(), Some(DTI_WIDTH));
    assert_eq!(DataType::elf().code_width(), Some(ELF_WIDTH));
    assert_eq!(DataType::fisn().code_width(), Some(FISN_WIDTH));
    assert_eq!(ROW_OVERHEAD, 16);
}

#[test]
fn a_known_width_sequence_is_built_where_it_is_stored() {
    let expected = Scalar::from_sequence([
        Scalar::from(0_i32),
        Scalar::from(1_i32),
        Scalar::from(2_i32),
    ]);
    let cell = |index: usize| Scalar::from(i32::try_from(index).unwrap());

    assert_eq!(
        scalar_try_sequence(3, |index| Ok(cell(index))).unwrap(),
        expected
    );
    assert_eq!(
        scalar_try_fill_sequence(3, |index, slot| {
            *slot = cell(index);
            Ok(())
        })
        .unwrap(),
        expected
    );
    assert_eq!(
        scalar_try_build_sequence(3, |slots| {
            for (index, slot) in slots.iter_mut().enumerate() {
                *slot = cell(index);
            }
            Ok(())
        })
        .unwrap(),
        expected
    );

    let refused = scalar_try_sequence(2, |index| {
        if index == 1 {
            Err(Error::InvalidRecord {
                path: "$[1]".into(),
                reason: "refused".into(),
            })
        } else {
            Ok(Scalar::Null)
        }
    });
    assert!(refused.is_err());
}

#[test]
fn a_decimal_reads_at_the_scale_its_datatype_declares() {
    assert_eq!(
        scalar_from_decimal_text(&DataType::Decimal, "41.25").unwrap(),
        Scalar::from(Decimal::parse("41.25").unwrap())
    );
    assert!(scalar_from_decimal_text(&DataType::Int32, "41").is_err());

    assert_eq!(
        scalar_leaf_display(&Scalar::from(7_i32)).map(ToString::to_string),
        Some("7".to_owned())
    );
    assert!(scalar_leaf_display(&Scalar::from("text")).is_none());
    assert_eq!(
        str_from_value(&Scalar::from("abc")).unwrap().unwrap(),
        "abc"
    );
}

#[test]
fn a_struct_is_built_over_children_already_known_to_be_valid() {
    let children = || {
        vec![
            DataType::Int32.required_field("a"),
            DataType::utf8().nullable_field("b"),
        ]
    };
    let expected = StructType::from_fields(children()).unwrap();
    assert_eq!(struct_type_from_unique_fields(children()), expected);
    assert_eq!(
        struct_type_from_checked_fields(children()).unwrap(),
        expected
    );
    assert!(
        struct_type_from_checked_fields(vec![
            DataType::Int32.required_field("a"),
            DataType::Int64.required_field("a"),
        ])
        .is_err()
    );

    let shared = expected.clone();
    assert!(struct_type_shares_storage_with(&expected, &shared));
    assert_eq!(
        struct_type_storage_address(&expected),
        struct_type_storage_address(&shared)
    );
    let empty = StructType::from_fields([]).unwrap();
    assert_eq!(struct_type_storage_address(&empty), 0);
}

#[test]
fn metadata_and_a_field_around_it_cross_by_their_storage() {
    let metadata = Metadata::from_entries([("SPARK:x", "1")]).unwrap();
    let shared = metadata.clone();
    let apart = Metadata::from_entries([("SPARK:x", "1")]).unwrap();
    assert!(metadata_shares_storage_with(&metadata, &shared));
    assert_eq!(
        metadata_storage_address(&metadata),
        metadata_storage_address(&shared)
    );
    assert!(!metadata_shares_storage_with(&metadata, &apart));

    let mut expected = DataType::Int64.required_field("price");
    expected.insert_metadata("SPARK:x", "1").unwrap();
    assert_eq!(
        field_new_with_metadata("price", DataType::Int64, false, metadata),
        expected
    );
}

#[test]
fn a_column_lends_the_bytes_of_its_text_and_byte_cells() {
    let text = Serie::from_scalars(
        Field::new("t", DataType::utf8(), true),
        [Scalar::from("ab"), Scalar::Null],
    )
    .unwrap();
    assert!(serie_is_string_storage(&text));
    assert!(!serie_is_byte_storage(&text));
    assert_eq!(serie_value_bytes(&text, 0), Some(b"ab".as_slice()));
    assert_eq!(serie_value_bytes(&text, 1), None, "an absent row");
    assert_eq!(serie_value_bytes(&text, 2), None, "past the end");

    let bytes = Serie::from_scalars(
        Field::new("b", DataType::binary(), true),
        [Scalar::from(b"xy".as_slice())],
    )
    .unwrap();
    assert!(serie_is_byte_storage(&bytes));
    assert!(!serie_is_string_storage(&bytes));
    assert_eq!(serie_value_bytes(&bytes, 0), Some(b"xy".as_slice()));
}

// ------------------------------------------------------------------------
// Definitions moved into the door.
// ------------------------------------------------------------------------

#[test]
fn an_instant_sequence_places_a_stream_by_order_and_by_content() {
    // By order: the next place of the instant's run, a new instant starting
    // a run of its own.
    let mut run = InstantSequence::default();
    let mut events: Vec<TextLine> = [5, 5, 5, 6].into_iter().map(line).collect();
    for event in &mut events {
        run.place_naming_sources(event);
    }
    assert_eq!(
        events.iter().map(TextLine::get_seqnum).collect::<Vec<_>>(),
        [0, 1, 2, 0]
    );

    // By content: a content the run placed takes its place again.
    let mut run = InstantSequence::default();
    assert_eq!(run.place(5, Some(11)), 0);
    assert_eq!(run.place(5, Some(12)), 1);
    assert_eq!(run.place(5, Some(11)), 0);
    assert_eq!(run.place(6, Some(12)), 0);
    assert_eq!(run.place(7, None), 0);
    assert_eq!(run.place(7, None), 1);
}

#[test]
fn staged_facts_digest_as_the_bytes_written_through() {
    let mut staged_state = Xxh3::new();
    {
        let mut staged = Staged::new(&mut staged_state);
        staged.feed("price", b"100");
        staged.write(b"tail");
        // Longer than the stage: written through, the stage flushed first.
        staged.write(&[7_u8; 600]);
    }

    let mut direct = Xxh3::new();
    direct.write_bytes(b"price");
    direct.write_bytes(&[0]);
    direct.write_bytes(b"100");
    direct.write_bytes(&[0]);
    direct.write_bytes(b"tail");
    direct.write_bytes(&[7_u8; 600]);
    assert_eq!(staged_state.as_u64(), direct.as_u64());
}

// ------------------------------------------------------------------------
// Exported macros.
// ------------------------------------------------------------------------

#[test]
fn the_pattern_macros_match_every_leaf_of_a_family() {
    use yggdryl::implementer::{bytes_dtypes, bytes_scalars, string_scalars};

    let strings = [
        Scalar::from("a"),
        DataType::sized_ascii(4)
            .unwrap()
            .scalar(Scalar::from("ab"))
            .unwrap(),
        DataType::fixed_cp1252(2)
            .unwrap()
            .scalar(Scalar::from("ab"))
            .unwrap(),
    ];
    for text in &strings {
        assert!(matches!(text, string_scalars!(_)), "{text:?}");
        assert!(!matches!(text, bytes_scalars!(_)), "{text:?}");
    }
    let bytes = [
        Scalar::from(b"ab".as_slice()),
        DataType::fixed_binary(2)
            .unwrap()
            .scalar(Scalar::from(b"ab".as_slice()))
            .unwrap(),
    ];
    for value in &bytes {
        assert!(matches!(value, bytes_scalars!(_)), "{value:?}");
        assert!(!matches!(value, string_scalars!(_)), "{value:?}");
    }

    assert!(matches!(DataType::binary(), bytes_dtypes!()));
    assert!(matches!(
        DataType::sized_binary(3).unwrap(),
        bytes_dtypes!()
    ));
    assert!(!matches!(DataType::utf8(), bytes_dtypes!()));
}

/// The view builder, taken through the door: the macro the FIX crate mints
/// its views with, written by a crate that has only this one path to it.
mod view {
    use yggdryl::implementer::protocol_field_types;
    use yggdryl::{DataType, Scheme};

    protocol_field_types!(
        pub,
        Scheme::from_str("door").expect("a custom scheme"),
        DoorField,
        DoorFieldMut,
        "Door"
    );

    #[test]
    fn the_builder_is_reached_through_the_door() {
        let mut field = DataType::Int64.required_field("price");
        DoorFieldMut::new(&mut field)
            .insert("x", "1")
            .expect("a property of the view's protocol");
        assert_eq!(field.get_metadata("DOOR:x"), Some("1"));
        assert_eq!(DoorField::new(&field).get("x"), Some("1"));
        assert_eq!(DoorFieldMut::new(&mut field).as_protocol().prefix(), "DOOR");
    }
}

/// `warned!` is exported, and its expansion names `implementer::warn`, so a
/// crate that invokes it reaches nothing of the core's that is private.
mod warning {
    use std::sync::{Arc, Mutex, PoisonError};

    use yggdryl::implementer::warned;
    use yggdryl::logging::{self, Formatter, Handler, HandlerState, Level, Record};

    #[derive(Default)]
    struct Collect {
        state: HandlerState,
        lines: Mutex<Vec<String>>,
    }

    impl Handler for Collect {
        fn state(&self) -> &HandlerState {
            &self.state
        }

        fn emit(&self, _record: &Record<'_>, line: &str) -> yggdryl::Result<()> {
            self.lines
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(line.to_owned());
            Ok(())
        }
    }

    #[test]
    fn a_warning_is_said_once_per_key_under_the_module_that_raised_it() {
        logging::install().expect("the tree");
        // This test crate is a foreign one to the tree, so the logger is its
        // module path with the dots a logger name has.
        let logger = logging::get_logger("root.implementer.warning");
        let collect = Arc::new(Collect::default());
        collect.set_formatter(Formatter::default());
        let handler: Arc<dyn Handler> = collect.clone();
        logger.set_level(Level::WARNING);
        logger.set_propagating(false);
        logger.add_handler(Arc::clone(&handler));

        for _ in 0..3 {
            warned!("a value was passed over", "price", "{} was not a price", 7);
        }
        logger.remove_handler(&handler);

        let lines = collect
            .lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("a value was passed over (price): 7 was not a price"),
            "{lines:?}"
        );
    }
}
