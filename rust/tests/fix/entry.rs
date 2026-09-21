//! `rust/src/fix/entry.rs`: what a message states, structured - the arrival
//! tree, the residual a fixed row does not project, and zero for an unresolved
//! key.

use super::SoleMessage;
use super::committed_registry;
use super::fixed_codec;
use super::path;

mod residual {
    use std::sync::Arc;

    use super::SoleMessage;
    use yggdryl::graph::{Element, Event, MarketElement};
    use yggdryl::{DataType, Field, FixMsg, FixRegistry, Scalar, StructType, fix_schema};

    const LINE: &[u8] = b"8=FIX.4.4|35=D|11=A1|55=AAPL|38=100|59=0|9999=x|10=0|";
    const PARTIES: &[u8] =
        b"8=FIX.4.4|35=D|11=A1|453=2|448=P1|447=D|452=1|448=P2|447=D|452=11|10=0|";

    fn reader() -> (Arc<FixRegistry>, yggdryl::FixCodec, Field) {
        let registry = super::committed_registry();
        let codec = super::fixed_codec(Arc::clone(&registry));
        let schema = fix_schema(&registry, "fix").expect("the fixed row");
        (registry, codec, schema)
    }

    fn at<'row>(row: &'row Scalar, field: &Field, name: &str) -> &'row Scalar {
        let at = field
            .index_of(name)
            .unwrap_or_else(|| panic!("a {name} column"));
        &row.as_sequence().expect("a row")[at]
    }

    fn narrow(field: &Field, names: &[&str]) -> Field {
        let columns = names
            .iter()
            .map(|name| field.fields()[field.index_of(name).expect("a fixed column")].clone());
        StructType::from_fields(columns)
            .map(DataType::from)
            .expect("a narrow root")
            .required_field("fix")
    }

    fn residual_tags(row: &Scalar, field: &Field) -> Vec<i64> {
        at(row, field, "fixentries")
            .as_sequence()
            .expect("the residual entries")
            .iter()
            .map(|entry| {
                entry.as_sequence().expect("an entry")[0]
                    .as_i64()
                    .unwrap_or_default()
            })
            .collect()
    }

    #[test]
    fn fixed_rows_keep_only_unknown_arrival_entries() {
        let (registry, codec, schema) = reader();
        let message = codec.sole_line(LINE).expect("one order");
        let row = message.into_row(&schema).expect("the fixed row");

        // Symbol, OrderQty and TimeInForce have fixed columns. The unknown tag is
        // the one arrival entry the fixed row retains, and its counter agrees.
        assert_eq!(at(&row, &schema, "nofixentries").as_i128(), Some(1));
        assert_eq!(residual_tags(&row, &schema), [0]);
        let restored = FixMsg::from_row(registry, &schema, &row).expect("the row reads");
        assert_eq!(restored.by_tag(55).expect("Symbol").as_str(), Some("AAPL"));
        assert!(!restored.by_tag(38).expect("OrderQty").is_null());
        assert_eq!(
            restored.by_tag(59).expect("TimeInForce").as_str(),
            Some("0")
        );
        assert_eq!(restored.into_row(&schema).expect("the fixed point"), row);
    }

    #[test]
    fn a_complete_row_keeps_its_identity_and_refills_derived_market_facts() {
        let (registry, codec, schema) = reader();
        let original = codec
            .sole_line(b"8=FIX.4.4|35=D|11=A1|54=1|44=10|38=2|15=USD|10=0|")
            .expect("one order");
        let row = original.into_row(&schema).expect("the fixed row");
        assert_eq!(original.get_side().as_str(), "BUY");

        let restored = FixMsg::from_row(registry, &schema, &row).expect("the row reads");
        assert_eq!(restored.get_side(), original.get_side());
        assert_eq!(restored.get_currency(), original.get_currency());
        assert_eq!(restored.get_qty(), original.get_qty());
        assert_eq!(restored.get_px(), original.get_px());
        assert_eq!(
            (
                restored.get_currunix(),
                restored.get_creaunix(),
                restored.get_currhashcode(),
                restored.get_crosshashcode(),
                restored.get_curruuid(),
                restored.get_crossuuid(),
            ),
            (
                original.get_currunix(),
                original.get_creaunix(),
                original.get_currhashcode(),
                original.get_crosshashcode(),
                original.get_curruuid(),
                original.get_crossuuid(),
            )
        );
        assert_eq!(restored.into_row(&schema).expect("the fixed point"), row);
    }

    #[test]
    fn lifecycle_agrees_after_a_row_reconstructs_content_in_schema_order() {
        let (registry, codec, schema) = reader();
        let direct = codec
            .sole_line(
                b"8=FIX.4.4|35=D|11=A1|54=1|44=10|38=2|15=USD|60=20240102-10:15:30.000|10=0|",
            )
            .expect("one order");
        let row = direct.into_row(&schema).expect("the fixed row");
        let rebuilt =
            FixMsg::from_row(Arc::clone(&registry), &schema, &row).expect("the row reads");

        let direct = codec
            .lifecycle([direct])
            .next()
            .expect("one lifecycle message")
            .expect("the direct message walks");
        let rebuilt = codec
            .lifecycle([rebuilt])
            .next()
            .expect("one lifecycle message")
            .expect("the rebuilt message walks");

        assert_eq!(rebuilt.get_currhashcode(), direct.get_currhashcode());
        assert_eq!(rebuilt.get_curruuid(), direct.get_curruuid());
        assert_eq!(
            rebuilt.into_row(&schema).expect("the fixed point"),
            direct.into_row(&schema).expect("the direct row")
        );
    }

    #[test]
    fn the_content_hash_orders_named_siblings_but_keeps_group_occurrences_ordered() {
        let (_registry, codec, _schema) = reader();
        let forward = codec
            .sole_line(b"8=FIX.4.4|35=D|52=20240102-10:15:30.000|55=AAPL|54=1|10=0|")
            .expect("one order");
        let reordered = codec
            .sole_line(b"8=FIX.4.4|35=D|52=20240102-10:15:30.000|54=1|55=AAPL|10=0|")
            .expect("the same order");
        assert_eq!(forward.get_currhashcode(), reordered.get_currhashcode());

        let split_left = codec
            .sole_line(b"8=FIX.4.4|35=D|52=20240102-10:15:30.000|house_a=bc|10=0|")
            .expect("one unknown entry");
        let split_right = codec
            .sole_line(b"8=FIX.4.4|35=D|52=20240102-10:15:30.000|house_ab=c|10=0|")
            .expect("a distinct unknown entry");
        assert_ne!(
            split_left.get_currhashcode(),
            split_right.get_currhashcode()
        );

        let parties = codec
            .sole_line(b"8=FIX.4.4|35=D|52=20240102-10:15:30.000|453=2|448=A|447=D|452=1|448=B|447=D|452=3|10=0|")
            .expect("two parties");
        let reversed = codec
            .sole_line(b"8=FIX.4.4|35=D|52=20240102-10:15:30.000|453=2|448=B|447=D|452=3|448=A|447=D|452=1|10=0|")
            .expect("the reversed parties");
        assert_ne!(parties.get_currhashcode(), reversed.get_currhashcode());
    }

    #[test]
    fn a_narrow_row_retains_known_entries_it_does_not_project() {
        let (registry, codec, schema) = reader();
        let narrow = narrow(
            &schema,
            &[
                "beginstring",
                "msgtype",
                "currunix",
                "creaunix",
                "currhashcode",
                "crosshashcode",
                "curruuid",
                "crossuuid",
                "crosscode",
                "snapunix",
                "sendingtime",
                "symbol",
                "fixentries",
                "nofixentries",
            ],
        );
        let row = codec
            .sole_line(LINE)
            .expect("one order")
            .into_row(&narrow)
            .expect("the narrow row");

        // Symbol is projected, but TimeInForce has no narrow column and stays in
        // the record beside the unresolved arrival key.
        assert_eq!(at(&row, &narrow, "nofixentries").as_i128(), Some(2));
        assert_eq!(residual_tags(&row, &narrow), [59, 0]);
        let restored = FixMsg::from_row(registry, &narrow, &row).expect("the row reads");
        assert_eq!(restored.by_tag(55).expect("Symbol").as_str(), Some("AAPL"));
        assert_eq!(
            restored.by_tag(59).expect("TimeInForce").as_str(),
            Some("0")
        );
        assert_eq!(restored.into_row(&narrow).expect("the fixed point"), row);
    }

    #[test]
    fn a_fully_represented_group_leaves_no_arrival_entry() {
        let (registry, codec, schema) = reader();
        let row = codec
            .sole_line(PARTIES)
            .expect("one order")
            .into_row(&schema)
            .expect("the fixed row");

        assert_eq!(at(&row, &schema, "nofixentries").as_i128(), Some(0));
        assert!(residual_tags(&row, &schema).is_empty());
        let restored = FixMsg::from_row(registry, &schema, &row).expect("the row reads");
        assert_eq!(restored.by_tag(453).expect("NoPartyIDs").as_i128(), Some(2));
        assert_eq!(restored.into_row(&schema).expect("the fixed point"), row);
    }

    #[test]
    fn an_empty_group_without_its_projection_stays_whole_in_the_residual() {
        let registry = super::committed_registry();
        let schema = fix_schema(&registry, "fix").expect("the fixed row");
        // Use the dictionary's counter scalar and the fixed schema's group so
        // both sides carry the production metadata for tag 453.
        let count = registry.field_by_tag(453).expect("NoPartyIDs").clone();
        let parties = schema.fields()[schema.index_of("parties").expect("Parties")].clone();
        let source = StructType::from_fields([count, parties])
            .map(DataType::from)
            .expect("the source root")
            .required_field("fix");
        let message = FixMsg::with_registry(
            Arc::clone(&registry),
            source,
            Scalar::from_sequence([Scalar::from(0_i32), Scalar::from_sequence([])]),
        )
        .expect("an empty party group");

        // The full group projection owns the counter entry, including the empty
        // occurrence list, so no residual is needed.
        let full = message.into_row(&schema).expect("the full row");
        assert_eq!(at(&full, &schema, "nofixentries").as_i128(), Some(0));
        assert!(residual_tags(&full, &schema).is_empty());

        // Dropping only `parties` leaves its counter scalar. That scalar cannot
        // represent the group shape, even at zero occurrences, so the complete
        // counter entry remains in the arrival record.
        let narrow = StructType::from_fields(
            schema
                .fields()
                .iter()
                .filter(|field| field.name() != "parties")
                .cloned(),
        )
        .map(DataType::from)
        .expect("the narrowed row")
        .required_field("fix");
        let row = message.into_row(&narrow).expect("the narrow row");
        assert_eq!(at(&row, &narrow, "nofixentries").as_i128(), Some(1));
        assert_eq!(residual_tags(&row, &narrow), [453]);
        let entry = at(&row, &narrow, "fixentries")
            .as_sequence()
            .expect("one residual entry")[0]
            .as_sequence()
            .expect("the party counter entry");
        assert_eq!(entry[2].as_str(), Some("0"));
        assert!(
            entry[3]
                .as_sequence()
                .expect("empty occurrences")
                .is_empty()
        );

        let restored = FixMsg::from_row(registry, &narrow, &row).expect("the row reads");
        assert_eq!(restored.by_tag(453).expect("NoPartyIDs").as_i128(), Some(0));
        assert_eq!(restored.into_row(&narrow).expect("the fixed point"), row);
    }

    #[test]
    fn an_omitted_group_stays_whole_in_the_arrival_record() {
        let (registry, codec, schema) = reader();
        let narrow = narrow(
            &schema,
            &[
                "beginstring",
                "msgtype",
                "currunix",
                "creaunix",
                "currhashcode",
                "crosshashcode",
                "curruuid",
                "crossuuid",
                "crosscode",
                "snapunix",
                "sendingtime",
                "timeinforce",
                "fixentries",
                "nofixentries",
            ],
        );
        let row = codec
            .sole_line(PARTIES)
            .expect("one order")
            .into_row(&narrow)
            .expect("the narrow row");

        assert_eq!(at(&row, &narrow, "nofixentries").as_i128(), Some(1));
        assert_eq!(residual_tags(&row, &narrow), [453]);
        let restored = FixMsg::from_row(registry, &narrow, &row).expect("the row reads");
        assert_eq!(restored.by_tag(453).expect("NoPartyIDs").as_i128(), Some(2));
        assert_eq!(restored.into_row(&narrow).expect("the fixed point"), row);
    }

    #[test]
    fn a_lossy_group_fit_keeps_the_whole_counter_entry() {
        let registry = super::committed_registry();
        let party_id = registry.field_by_tag(448).expect("PartyID").clone();
        let party_source = registry.field_by_tag(447).expect("PartyIDSource").clone();
        let mut party_role = registry.field_by_tag(452).expect("PartyRole").clone();
        party_role
            .set_dtype(DataType::utf8())
            .expect("the malformed source spelling");
        let party = StructType::from_fields([party_id, party_source, party_role])
            .map(DataType::from)
            .expect("a party occurrence")
            .required_field("party");
        let mut parties = DataType::list(party).nullable_field("parties");
        parties.as_fix_mut().set_counter(453).expect("NoPartyIDs");
        let root = StructType::from_fields([parties])
            .map(DataType::from)
            .expect("a group message")
            .required_field("fix");
        let message = FixMsg::with_registry(
            Arc::clone(&registry),
            root,
            Scalar::from_sequence([Scalar::from_sequence([Scalar::from_sequence([
                Scalar::from("P1"),
                Scalar::from("D"),
                Scalar::from("not-a-role"),
            ])])]),
        )
        .expect("a malformed party source");
        let schema = fix_schema(&registry, "fix").expect("the fixed row");
        let row = message.into_row(&schema).expect("the fixed row");

        // The source group accepts its text role, but the fixed PartyRole column
        // is integer. Its null fitted descendant leaves the entire group residual.
        assert_eq!(at(&row, &schema, "nofixentries").as_i128(), Some(1));
        assert_eq!(residual_tags(&row, &schema), [453]);
        let mut restored = FixMsg::from_row(registry, &schema, &row).expect("the row reads");
        let rebuilt = restored.into_row(&schema).expect("the fixed point");
        for ((column, before), after) in schema
            .fields()
            .iter()
            .zip(row.as_sequence().expect("a row"))
            .zip(rebuilt.as_sequence().expect("a row"))
        {
            assert_eq!(after, before, "reconstructed {}", column.name());
        }
        let residual = at(&row, &schema, "fixentries").clone();
        restored
            .set(55, Scalar::from("ABC"))
            .expect("an unrelated write");
        let changed = restored.into_row(&schema).expect("the edited row");
        assert_eq!(at(&changed, &schema, "fixentries"), &residual);
    }

    #[test]
    fn residual_count_and_entries_follow_the_schema_names_not_column_order() {
        let (registry, codec, schema) = reader();
        let reordered = narrow(
            &schema,
            &[
                "fixentries",
                "nofixentries",
                "beginstring",
                "msgtype",
                "currunix",
                "creaunix",
                "currhashcode",
                "crosshashcode",
                "curruuid",
                "crossuuid",
                "crosscode",
                "snapunix",
                "sendingtime",
                "symbol",
            ],
        );
        let row = codec
            .sole_line(LINE)
            .expect("one order")
            .into_row(&reordered)
            .expect("the reordered row");

        assert_eq!(at(&row, &reordered, "nofixentries").as_i128(), Some(2));
        assert_eq!(residual_tags(&row, &reordered), [59, 0]);
        let restored = FixMsg::from_row(registry, &reordered, &row).expect("the row reads");
        assert_eq!(restored.into_row(&reordered).expect("the fixed point"), row);
    }

    #[test]
    fn shared_tag_children_remain_residual_and_reconstruct_by_name() {
        let registry = super::committed_registry();
        let mut left = DataType::utf8().required_field("venue_symbol");
        left.as_fix_mut().set_tag(55).expect("a tag");
        let mut right = DataType::utf8().required_field("client_symbol");
        right.as_fix_mut().set_tag(55).expect("a tag");
        let root = StructType::from_fields([left, right])
            .map(DataType::from)
            .expect("a message root")
            .required_field("fix");
        let message = FixMsg::with_registry(
            Arc::clone(&registry),
            root,
            Scalar::from_sequence([Scalar::from("AAA"), Scalar::from("BBB")]),
        )
        .expect("a custom message");
        let schema = fix_schema(&registry, "fix").expect("the fixed row");
        let row = message.into_row(&schema).expect("the fixed row");

        assert_eq!(at(&row, &schema, "nofixentries").as_i128(), Some(2));
        assert_eq!(residual_tags(&row, &schema), [55, 55]);
        let restored = FixMsg::from_row(registry, &schema, &row).expect("the row reads");
        assert_eq!(
            restored.by_name("venue_symbol").expect("left").as_str(),
            Some("AAA")
        );
        assert_eq!(
            restored.by_name("client_symbol").expect("right").as_str(),
            Some("BBB")
        );
        assert_eq!(restored.into_row(&schema).expect("the fixed point"), row);
    }

    #[test]
    fn unknown_nested_and_repeated_trees_stay_residual() {
        let registry = super::committed_registry();
        let vendor_row = StructType::from_fields([
            DataType::utf8().required_field("name"),
            DataType::utf8().required_field("value"),
        ])
        .map(DataType::from)
        .expect("a vendor row")
        .required_field("vendor_row");
        let vendor_rows = DataType::list(vendor_row).nullable_field("vendor_rows");
        let vendor_tag = DataType::utf8().required_field("tag");
        let vendor_tags = DataType::list(vendor_tag).nullable_field("vendor_tags");
        let root = StructType::from_fields([vendor_rows, vendor_tags])
            .map(DataType::from)
            .expect("an unknown source root")
            .required_field("fix");
        let message = FixMsg::with_registry(
            Arc::clone(&registry),
            root,
            Scalar::from_sequence([
                Scalar::from_sequence([
                    Scalar::from_sequence([Scalar::from("alpha"), Scalar::from("one")]),
                    Scalar::from_sequence([Scalar::from("beta"), Scalar::from("two")]),
                ]),
                Scalar::from_sequence([Scalar::from("A"), Scalar::from("B")]),
            ]),
        )
        .expect("an unknown nested message");
        let schema = fix_schema(&registry, "fix").expect("the fixed row");
        let row = message.into_row(&schema).expect("the fixed row");

        // Neither unknown tree has a projected FIX identity, so the generic
        // arrival format owns both shapes rather than flattening their leaves.
        assert_eq!(at(&row, &schema, "nofixentries").as_i128(), Some(2));
        assert_eq!(residual_tags(&row, &schema), [0, 0]);
        let restored = FixMsg::from_row(registry, &schema, &row).expect("the row reads");
        let tags = restored
            .by_name("vendor_tags")
            .expect("the repeated unknown leaves");
        let tags = tags.as_sequence().expect("a repeated sequence");
        assert_eq!(
            tags.iter().map(Scalar::as_str).collect::<Vec<_>>(),
            [Some("A"), Some("B")]
        );
        assert_eq!(restored.into_row(&schema).expect("the fixed point"), row);
    }

    #[test]
    fn an_unfittable_nullable_scalar_stays_residual_and_required_refuses() {
        let (registry, codec, schema) = reader();
        let message = codec
            .sole_line(b"8=FIX.4.4|35=D|11=A1|55=ABCDEF|10=0|")
            .expect("one order");
        let before = message.entries().to_vec();
        let mut columns = schema.fields().to_vec();
        let symbol = schema.index_of("symbol").expect("a symbol column");
        columns[symbol]
            .set_dtype(DataType::fixed_utf8(2).expect("a fixed string"))
            .expect("the replacement type fits the field");
        let nullable = StructType::from_fields(columns.clone())
            .map(DataType::from)
            .expect("a nullable root")
            .required_field("fix");
        let row = message
            .into_row(&nullable)
            .expect("nullable fit is residual");
        assert!(at(&row, &nullable, "symbol").is_null());
        assert_eq!(residual_tags(&row, &nullable), [55]);
        assert_eq!(
            message.entries(),
            before,
            "writing a row does not mutate entries"
        );
        let restored = FixMsg::from_row(Arc::clone(&registry), &nullable, &row)
            .expect("the nullable row reads");
        assert_eq!(restored.into_row(&nullable).expect("the fixed point"), row);

        columns[symbol].set_nullable(false);
        let required = StructType::from_fields(columns)
            .map(DataType::from)
            .expect("a required root")
            .required_field("fix");
        assert!(message.into_row(&required).is_err(), "required fit refuses");
    }
}

mod unresolved {
    use std::sync::Arc;

    use super::SoleMessage;
    use yggdryl::graph::Element;
    use yggdryl::xxhash::xxh128;
    use yggdryl::{DataType, Error, Field, FixEntry, FixRegistry, Scalar, StructType};

    fn tagged(name: &str, tag: i32) -> Field {
        let mut field = DataType::utf8().nullable_field(name);
        field.as_fix_mut().set_tag(tag).unwrap();
        field
    }

    #[test]
    fn registry_tag_writers_refuse_nonpositive_values_atomically() {
        let mut field = tagged("positive", 1);
        field.as_fix_mut().set_counter(2).unwrap();
        field.as_fix_mut().set_tags(&[3, i32::MAX]).unwrap();
        let before = field.clone();
        for tag in [0, -1, i32::MIN] {
            for (key, result) in [
                ("FIX:tag", field.as_fix_mut().set_tag(tag)),
                ("FIX:counter", field.as_fix_mut().set_counter(tag)),
                ("FIX:tags", field.as_fix_mut().set_tags(&[4, tag])),
            ] {
                let error = result.unwrap_err();
                assert!(
                    matches!(&error, Error::InvalidMetadataValue { key: actual, .. } if actual == key),
                    "{error}"
                );
                assert!(
                    error.to_string().contains("from 1 to 2147483647"),
                    "{error}"
                );
                assert_eq!(field, before, "{key}={tag}");
            }
        }
        field.as_fix_mut().set_tag(i32::MAX).unwrap();
        field.as_fix_mut().set_counter(i32::MAX).unwrap();
        assert_eq!(field.as_fix().tag().unwrap(), Some(i32::MAX));
        assert_eq!(field.as_fix().counter().unwrap(), Some(i32::MAX));
        field.as_fix_mut().set_tags(&[]).unwrap();
        assert!(field.get_metadata("FIX:tags").is_none());
    }

    #[test]
    fn externally_stated_zero_identity_is_refused_without_mutating_the_registry() {
        // The alternates are one JSON array, and their elements are held to the
        // same shape as the two scalar tags.
        for key in ["FIX:tag", "FIX:counter", "FIX:tags"] {
            for digits in ["0", "000", "-1", "+1", "2147483648"] {
                let text = if key == "FIX:tags" {
                    format!("[{digits}]")
                } else {
                    digits.to_owned()
                };
                let mut field = tagged("incoming", 90_001);
                field.insert_metadata(key, text.as_str()).unwrap();
                let error = match key {
                    "FIX:tag" => field.as_fix().tag().unwrap_err(),
                    "FIX:counter" => field.as_fix().counter().unwrap_err(),
                    _ => field.as_fix().tags().unwrap_err(),
                };
                assert!(
                    matches!(&error, Error::InvalidMetadataValue { key: actual, .. } if actual == key),
                    "{key}={text}: {error}"
                );
                let mut registry = FixRegistry::new();
                let before = registry.clone();
                assert!(registry.insert(field).is_err(), "{key}={text}");
                assert_eq!(registry, before, "{key}={text}");
            }
        }
        // Leading zeros are still the tag on the two bare decimals, and never in
        // the array: a JSON number spells none, and the array is JSON.
        let mut field = tagged("positive", 1);
        for key in ["FIX:tag", "FIX:counter"] {
            field.insert_metadata(key, "0001").unwrap();
        }
        field.insert_metadata("FIX:tags", "[1]").unwrap();
        assert_eq!(field.as_fix().tag().unwrap(), Some(1));
        assert_eq!(field.as_fix().counter().unwrap(), Some(1));
        assert_eq!(field.as_fix().tags().unwrap(), [1]);
        field.insert_metadata("FIX:tags", "[0001]").unwrap();
        let error = field.as_fix().tags().unwrap_err();
        assert!(
            matches!(&error, Error::InvalidMetadataValue { key, .. } if key == "FIX:tags"),
            "{error}"
        );
    }

    #[test]
    fn unresolved_arrivals_keep_their_keys_order_and_dynamic_columns() {
        let codec = super::fixed_codec(super::committed_registry());
        let wire =
            b"35=D|999999=one|0999999=two|OwnThing=three|0=zero|2147483648=wide|55=SYNTH|10=0|";
        let message = codec.sole_line(wire).unwrap();
        // The type and the checksum are the frame's, never entries: the entries
        // are the content row alone, the day order the dictionary derives for
        // an order among them.
        assert_eq!(
            message
                .entries()
                .iter()
                .map(FixEntry::tag)
                .collect::<Vec<_>>(),
            [0, 0, 0, 0, 0, 55, 59]
        );
        // An unresolved key is an entry under its own spelling, folded as every
        // name is, and a child of the row under it.
        for (entry, (key, folded, value)) in message.entries()[..5].iter().zip([
            ("999999", "999999", "one"),
            ("0999999", "0999999", "two"),
            ("OwnThing", "ownthing", "three"),
            ("0", "0", "zero"),
            ("2147483648", "2147483648", "wide"),
        ]) {
            assert_eq!(entry.name(), folded);
            assert_eq!(entry.value(), Some(value));
            assert!(entry.entries().is_empty());
            assert_eq!(
                message.get_by_name(key).as_ref().and_then(Scalar::as_str),
                Some(value)
            );
        }
        assert_eq!(
            message
                .get_by_tag(999_999)
                .as_ref()
                .and_then(Scalar::as_str),
            Some("one")
        );
        assert_eq!(
            message.into_bytes(b'|'),
            b"8=FIX.4.4|35=D|999999=one|0999999=two|ownthing=three|0=zero|2147483648=wide|55=SYNTH|59=0|10=0|"
        );
        let entry = FixEntry::new(0, "999999", Some("one".into()));
        assert_eq!(entry, message.entries()[0]);
    }

    #[test]
    fn indexed_unknowns_keep_each_arrival_and_their_existing_value_shape() {
        let codec = super::fixed_codec(super::committed_registry());
        let message = codec
            .parse_pairs(
                [("999999[0]", "first"), ("999999[2]", "third")]
                    .map(|(key, value)| (key.as_bytes(), value.as_bytes())),
            )
            .unwrap();
        // One child, a list, read as one entry heading an entry per stated
        // element.
        assert_eq!(
            message.get_by_name("999999").unwrap(),
            Scalar::from_sequence([Scalar::from("first"), Scalar::Null, Scalar::from("third")])
        );
        assert_eq!(message.entries().len(), 1);
        let list = &message.entries()[0];
        assert_eq!((list.tag(), list.name(), list.value()), (0, "999999", None));
        assert_eq!(
            list.entries()
                .iter()
                .map(|element| (element.tag(), element.name(), element.value()))
                .collect::<Vec<_>>(),
            [(0, "999999", Some("first")), (0, "999999", Some("third"))]
        );
    }

    #[test]
    fn a_group_keeps_resolved_members_and_unknown_children_under_the_stated_counter() {
        let mut registry = FixRegistry::new();
        let mut counter = DataType::Int32.nullable_field("norows");
        counter.as_fix_mut().set_tag(90_001).unwrap();
        registry.insert(counter).unwrap();
        let mut group = DataType::list(
            StructType::from_fields([tagged("scopedvalue", 90_002)])
                .map(DataType::from)
                .unwrap()
                .required_field("row"),
        )
        .nullable_field("rows");
        group.as_fix_mut().set_counter(90_001).unwrap();
        registry.insert(group).unwrap();
        assert!(registry.get_field_by_tag(90_002).is_none());
        let codec = super::fixed_codec(Arc::new(registry));
        let message = codec
            .parse_pairs(
                [
                    ("NoRows", "1"),
                    ("Rows[0].ScopedValue", "known"),
                    ("Rows[0].999999", "numeric"),
                    ("Rows[0].OwnThing", "named"),
                ]
                .map(|(key, value)| (key.as_bytes(), value.as_bytes())),
            )
            .unwrap();
        // The group is one entry under its counter, counting its occurrence;
        // the occurrence states nothing of its own and holds its members, the
        // resolved one under its tag and the unknown ones under their keys.
        assert_eq!(message.entries().len(), 1);
        let counter = &message.entries()[0];
        assert_eq!(
            (counter.tag(), counter.name(), counter.value()),
            (90_001, "rows", Some("1"))
        );
        assert_eq!(counter.entries().len(), 1);
        let occurrence = &counter.entries()[0];
        assert_eq!(occurrence.value(), None);
        assert_eq!(
            occurrence
                .entries()
                .iter()
                .map(|member| (member.tag(), member.name(), member.value()))
                .collect::<Vec<_>>(),
            [
                (90_002, "scopedvalue", Some("known")),
                (0, "999999", Some("numeric")),
                (0, "ownthing", Some("named")),
            ]
        );
        for (path, expected) in [
            ("rows[0].scopedvalue", "known"),
            ("rows[0].\"999999\"", "numeric"),
            ("rows[0].ownthing", "named"),
        ] {
            assert_eq!(
                message
                    .get_by_path(&super::path(path))
                    .as_ref()
                    .and_then(Scalar::as_str),
                Some(expected),
                "{path}"
            );
        }
        assert_eq!(
            message.into_bytes(b'|'),
            b"8=FIX.4.4|90001=1|90002=known|999999=numeric|ownthing=named|"
        );
    }

    #[test]
    fn numeric_and_named_aliases_reach_the_canonical_field_and_re_emit_its_tag() {
        let mut registry = super::committed_registry().as_ref().clone();
        let mut symbol = registry.field_by_tag(55).unwrap().clone();
        symbol.as_fix_mut().set_tags(&[9_000_001]).unwrap();
        symbol.as_fix_mut().set_names(["SyntheticSymbol"]).unwrap();
        registry.insert(symbol).unwrap();
        let codec = super::fixed_codec(Arc::new(registry));
        let canonical = codec.sole_line(b"35=D|55=SYNTH|10=0|").unwrap();
        for key in ["55", "00055", "9000001", "SyntheticSymbol"] {
            let wire = format!("35=D|{key}=SYNTH|10=0|");
            let message = codec.sole_line(wire.as_bytes()).unwrap();
            let entry = &message.entries()[0];
            assert_eq!(entry.tag(), 55, "{key}");
            assert_eq!(entry.name(), "symbol", "{key}");
            assert_eq!(message.digest(), canonical.digest(), "{key}");
            // Every spelling is one field, and one field re-emits under its
            // canonical tag.
            assert_eq!(
                message.into_bytes(b'|'),
                canonical.into_bytes(b'|'),
                "{key}"
            );
        }
    }

    #[test]
    fn unknown_numeric_digests_include_the_raw_key_in_the_existing_zero_tag_frame() {
        let codec = super::fixed_codec(super::committed_registry());
        let mut digests = Vec::new();
        for key in ["999999", "0999999", "999998", "OwnThing"] {
            let message = codec
                .parse_pairs([(key.as_bytes(), b"x".as_slice())])
                .unwrap();
            assert_eq!(message.entries().len(), 1);
            assert_eq!(message.entries()[0].tag(), 0);
            // Independent framing: tag, name length/name, value length/value,
            // children - the name under the fold every key resolves by.
            let name = message.entries()[0].name();
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&0_i32.to_be_bytes());
            bytes.extend_from_slice(&(name.len() as u32).to_be_bytes());
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(&1_u32.to_be_bytes());
            bytes.push(b'x');
            bytes.extend_from_slice(&0_u32.to_be_bytes());
            assert_eq!(message.digest(), xxh128(&bytes), "{key}");
            assert!(!digests.contains(&message.digest()), "{key}");
            digests.push(message.digest());
        }
    }

    #[test]
    fn an_unresolved_counter_at_the_root_heads_what_arrived_under_it() {
        let codec = super::fixed_codec(super::committed_registry());
        let message = codec
            .parse_pairs(
                [
                    ("999999", "2"),
                    ("999999[0].OwnThing", "a"),
                    ("999999[1].999998", "b"),
                ]
                .map(|(key, value)| (key.as_bytes(), value.as_bytes())),
            )
            .unwrap();
        let text = |path: &str| {
            message
                .get_by_path(&super::path(path))
                .as_ref()
                .and_then(Scalar::as_str)
                .map(ToOwned::to_owned)
        };
        // A group built from indexed keys is sorted by what each occurrence
        // states, so the one stating only the later member comes first: each
        // occurrence carries the members its own index stated, and no other's.
        assert_eq!(text("999999[1].ownthing").as_deref(), Some("a"));
        assert_eq!(text("999999[0].\"999998\"").as_deref(), Some("b"));
        assert_eq!(text("999999[1].\"999998\""), None);
    }

    #[test]
    fn a_header_tag_is_the_headers_fact_and_stays_out_of_the_content_code() {
        let read = |sequence: &str| {
            super::fixed_codec(super::committed_registry())
                .parse_pairs([
                    (b"34".as_slice(), sequence.as_bytes()),
                    (b"11".as_slice(), b"A".as_slice()),
                ])
                .unwrap()
        };
        // The session layer is the envelope around what a message says, so the
        // sequence number is the header's own fact: it is no entry of the
        // content row, it goes back on the wire from the header, and it is the
        // frame's and not the content's - a capture logs one message at every
        // hop it passes, each hop framing it in a session of its own - so two
        // messages differing in it alone digest to one content code.
        assert_eq!(read("7").header().msgseqnum(), Some(7));
        assert_eq!(read("8").header().msgseqnum(), Some(8));
        assert!(!read("7").entries().iter().any(|entry| entry.tag() == 34));
        assert_eq!(read("7").into_bytes(b'|'), b"8=FIX.4.4|34=7|11=A|");
        assert_eq!(read("7").get_currhashcode(), read("8").get_currhashcode());
    }
}
