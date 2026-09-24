//! `rust/src/side.rs`: FIX's side of a trade as a one-byte enum, read by
//! spelling and refused where a spelling names none.

mod coded {

    use yggdryl::{CodeValue, DataType, Field, Scalar, Side, StringEnum, StructType, Value};

    /// Every side: its wire code, its stored spelling and the name the
    /// specification gives it, in the order of the discriminants.
    const SIDES: [(Option<char>, &str, &str, Side); 18] = [
        (None, "UNKNOWN", "Unknown", Side::Unknown),
        (Some('1'), "BUY", "Buy", Side::Buy),
        (Some('2'), "SELL", "Sell", Side::Sell),
        (Some('3'), "BUYMINUS", "BuyMinus", Side::BuyMinus),
        (Some('4'), "SELLPLUS", "SellPlus", Side::SellPlus),
        (Some('5'), "SSHORT", "SellShort", Side::SShort),
        (Some('6'), "SSHORTEX", "SellShortExempt", Side::SShortEx),
        (Some('7'), "UNDISC", "Undisclosed", Side::Undisc),
        (Some('8'), "CROSS", "Cross", Side::Cross),
        (Some('9'), "CROSSSH", "CrossShort", Side::CrossSh),
        (Some('A'), "CROSSSHX", "CrossShortExempt", Side::CrossShX),
        (Some('B'), "ASDEF", "AsDefined", Side::AsDef),
        (Some('C'), "OPPOSITE", "Opposite", Side::Opposite),
        (Some('D'), "SUBSCR", "Subscribe", Side::Subscr),
        (Some('E'), "REDEEM", "Redeem", Side::Redeem),
        (Some('F'), "LEND", "Lend", Side::Lend),
        (Some('G'), "BORROW", "Borrow", Side::Borrow),
        (Some('H'), "SELLUND", "SellUndisclosed", Side::SellUnd),
    ];

    #[test]
    fn a_side_is_one_byte_whose_discriminant_is_the_position_of_its_wire_code() {
        assert_eq!(std::mem::size_of::<Side>(), 1);
        assert_eq!(std::mem::size_of::<Option<Side>>(), 1);
        assert_eq!(Side::default(), Side::Unknown);

        for (index, (code, stored, _, side)) in SIDES.iter().enumerate() {
            assert_eq!(*side as usize, index, "{stored}");
            assert_eq!(side.fix_code(), *code, "{stored}");
            assert_eq!(side.as_str(), *stored);
            assert_eq!(side.storage().as_str(), *stored);
            assert_eq!(side.to_string(), *stored);
            // The stored handle is shared, never built per value.
            assert!(std::ptr::eq(
                side.storage(),
                Side::from_spelling(stored).unwrap().storage()
            ));
        }
        // The variants order as the wire codes do, `1`..=`9` then `A`..=`H`.
        let mut ordered: Vec<Side> = SIDES.iter().map(|(_, _, _, side)| *side).collect();
        ordered.sort_unstable();
        assert_eq!(
            ordered,
            SIDES
                .iter()
                .map(|(_, _, _, side)| *side)
                .collect::<Vec<_>>()
        );
        // The listing is the eighteen stored spellings, sorted.
        let mut listed: Vec<&str> = SIDES.iter().map(|(_, stored, _, _)| *stored).collect();
        listed.sort_unstable();
        assert_eq!(listed.as_slice(), StringEnum::SIDES);
    }

    #[test]
    fn three_vocabularies_reach_one_side_and_a_wire_code_never_folds() {
        for (code, stored, name, side) in SIDES {
            assert_eq!(Side::from_spelling(stored), Some(side), "{stored}");
            assert_eq!(Side::from_spelling(name), Some(side), "{name}");
            assert_eq!(
                Side::from_spelling(&name.to_ascii_lowercase()),
                Some(side),
                "{name}"
            );
            assert_eq!(Side::read(stored).unwrap(), side);
            assert_eq!(Side::new(name).unwrap(), side);
            assert_eq!(Side::new(String::from(stored)).unwrap(), side);
            if let Some(code) = code {
                let wire = String::from(code);
                assert_eq!(Side::from_spelling(&wire), Some(side), "{wire}");
                // A wire letter does not fold: `a` is not `A`.
                if code.is_ascii_alphabetic() {
                    assert_eq!(
                        Side::from_spelling(&wire.to_ascii_lowercase()),
                        None,
                        "{wire}"
                    );
                }
            }
        }
        // A name folds the way every name in this crate folds.
        for spelling in ["sell_short", "SELL SHORT", "Sell-Short", "sellshort"] {
            assert_eq!(
                Side::from_spelling(spelling),
                Some(Side::SShort),
                "{spelling}"
            );
        }
        // A spelling that names no side is refused naming the datatype, and
        // the width bounds the value read, never the spelling.
        assert_eq!(Side::from_spelling("X"), None);
        assert_eq!(Side::from_spelling(""), None);
        for spelling in ["X", "Z", "TOOLONGSIDE", "BUYS"] {
            let refused = Side::read(spelling).unwrap_err().to_string();
            assert!(refused.contains("side"), "{refused}");
            assert!(refused.contains(spelling), "{refused}");
        }
        assert_eq!(Side::read("SellShortExempt").unwrap(), Side::SShortEx);
    }

    #[test]
    fn a_side_takes_the_bid_lane_the_ask_lane_or_neither() {
        let bid = [Side::Buy, Side::BuyMinus];
        let ask = [
            Side::Sell,
            Side::SellPlus,
            Side::SShort,
            Side::SShortEx,
            Side::SellUnd,
        ];
        for (_, stored, _, side) in SIDES {
            assert_eq!(side.is_bid(), bid.contains(&side), "{stored}");
            assert_eq!(side.is_ask(), ask.contains(&side), "{stored}");
            assert!(!(side.is_bid() && side.is_ask()), "{stored}");
        }
        // A cross, `OPPOSITE`, `ASDEF`, `UNDISC` and a side stated as none
        // take no lane.
        for side in [
            Side::Unknown,
            Side::Cross,
            Side::CrossSh,
            Side::CrossShX,
            Side::AsDef,
            Side::Opposite,
            Side::Undisc,
            Side::Subscr,
            Side::Redeem,
            Side::Lend,
            Side::Borrow,
        ] {
            assert!(!side.is_bid() && !side.is_ask(), "{side}");
        }
    }

    #[test]
    fn a_side_stated_as_none_merges_to_the_other_and_anything_stated_stands() {
        assert_eq!(Side::Unknown.merge_with(&Side::Buy), Side::Buy);
        assert_eq!(Side::Unknown.merge_with(&Side::Unknown), Side::Unknown);
        assert_eq!(Side::Buy.merge_with(&Side::Sell), Side::Buy);
        assert_eq!(Side::Buy.merge_with(&Side::Unknown), Side::Buy);
        assert_eq!(<Side as CodeValue>::WIDTH, 8);
    }

    #[test]
    fn a_side_serializes_as_its_stored_spelling_and_reads_back_by_spelling() {
        assert_eq!(serde_json::to_string(&Side::SShort).unwrap(), "\"SSHORT\"");
        assert_eq!(
            serde_json::to_string(&Side::Unknown).unwrap(),
            "\"UNKNOWN\""
        );
        // Every vocabulary reads back, through a borrowed and an owned door.
        for (spelling, side) in [
            ("\"SSHORT\"", Side::SShort),
            ("\"5\"", Side::SShort),
            ("\"sell_short\"", Side::SShort),
            ("\"UNKNOWN\"", Side::Unknown),
        ] {
            assert_eq!(serde_json::from_str::<Side>(spelling).unwrap(), side);
            let parsed: serde_json::Value = serde_json::from_str(spelling).unwrap();
            assert_eq!(serde_json::from_value::<Side>(parsed).unwrap(), side);
        }
        let refused = serde_json::from_str::<Side>("\"X\"")
            .unwrap_err()
            .to_string();
        assert!(refused.contains("side"), "{refused}");
        assert!(serde_json::from_str::<Side>("1").is_err());

        // And a scalar carries the side, not the text.
        let scalar = Scalar::from(Side::Buy);
        assert_eq!(scalar, DataType::Side.scalar(Scalar::from("1")).unwrap());
        assert_eq!(scalar.as_str(), Some("BUY"));
        assert_eq!(Side::from_scalar(&scalar), Some(&Side::Buy));
        assert_ne!(scalar, Scalar::from("BUY"));
    }

    #[test]
    fn there_is_no_member_meaning_no_answer_and_null_is_how_a_row_says_it() {
        // A row whose line does not say a side has none, and the crate already
        // spells "no answer" one way: `UNKNOWN` is what a value that must state
        // a side states where none was said, as a state's `00UNKNOWN` is, and
        // never what a column says for an absent one.
        assert!(StringEnum::SIDES.contains(&"UNKNOWN"));
        assert!(!StringEnum::SIDES.contains(&"NONE"));

        let field = Field::new("side", DataType::Side, true);
        let row = Field::new(
            "row",
            DataType::from(StructType::from_fields([field.clone()]).unwrap()),
            false,
        );
        let value = row
            .canonicalize_value(Scalar::from_sequence([Scalar::Null]))
            .unwrap();
        row.validate_value(&value).unwrap();
        assert!(value.as_sequence().unwrap()[0].is_null());
        // A required one refuses the same null, so the nullability is the field's
        // and not the datatype's.
        let required = Field::new(
            "row",
            DataType::from(
                StructType::from_fields([Field::new("side", DataType::Side, false)]).unwrap(),
            ),
            false,
        );
        assert!(
            required
                .validate_value(&Scalar::from_sequence([Scalar::Null]))
                .is_err()
        );
    }
}
