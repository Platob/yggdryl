//! The three coded datatypes FIX's constant vocabulary earns.
//!
//! `DataType` is `#[non_exhaustive]` and the datatype layer carries some sixty
//! wildcard arms, so a new variant compiles clean while behaving wrongly. A
//! green build proves nothing; these are the invariants a wildcard cannot
//! satisfy by accident.

use yggdryl::{AsciiEnum, DataType, DataTypeId, DataTypeKind, Field, Scalar, types::AsciiFamily};

/// The three, with the width each fixes and the vocabulary it publishes.
const CODED: [(&str, DataType, i32); 3] = [
    ("side", DataType::Side, 4),
    ("msgtype", DataType::MsgType, 8),
    ("msgdirection", DataType::MsgDirection, 4),
];

#[test]
fn each_coded_datatype_answers_every_invariant_a_wildcard_would_get_wrong() {
    for (name, dtype, width) in &CODED {
        // Naming: one canonical spelling, and the grammar round-trips it.
        assert_eq!(dtype.to_string(), *name, "{name}");
        assert_eq!(DataType::from_str(name).unwrap(), *dtype, "{name}");
        assert_eq!(dtype.name(), *name, "{name}");
        assert_eq!(dtype.code_name(), Some(*name), "{name}");
        // The fold reaches it, exactly as it reaches every other name.
        assert_eq!(DataType::from_logical_name(name).unwrap(), *dtype, "{name}");
        assert_eq!(
            DataType::from_str(&name.to_uppercase()).unwrap(),
            *dtype,
            "{name}"
        );

        // Identity: the discriminant, the family, the width, and the listing.
        assert_eq!(dtype.id().as_str(), *name, "{name}");
        assert_eq!(dtype.kind(), DataTypeKind::Ascii, "{name}");
        assert_eq!(dtype.ascii_width(), Some(*width), "{name}");
        assert_eq!(
            dtype.id().fixed_byte_width(),
            usize::try_from(*width).ok(),
            "{name}"
        );
        assert!(dtype.is_code(), "{name}");
        assert!(dtype.is_ascii(), "{name}");
        assert!(DataTypeId::ALL.contains(&dtype.id()), "{name}");
        assert!(
            DataType::CODES.iter().any(|(held, _, _)| held == name),
            "{name}"
        );

        // Nestedness: a registry places it in the primitive half.
        assert!(!dtype.is_nested(), "{name}");
        assert!(!dtype.id().is_parameterized(), "{name}");

        // Default: it answers one rather than falling through to one.
        let default = dtype.default_value().unwrap();
        assert!(!default.is_null(), "{name}");
        dtype.scalar(default.clone()).unwrap();

        // Serde: the serialized shape is the canonical spelling, and it
        // round-trips.
        let json = dtype.clone().into_json().unwrap();
        assert_eq!(DataType::from_json(&json).unwrap(), *dtype, "{name}");
        let rendered = serde_json::to_string(dtype).unwrap();
        assert_eq!(
            serde_json::from_str::<DataType>(&rendered).unwrap(),
            *dtype,
            "{name}"
        );

        // Arrow: one type and back, losslessly, through a field.
        let field = Field::new(*name, dtype.clone(), false);
        let arrow = field.clone().into_arrow().unwrap();
        assert_eq!(Field::from_arrow(&arrow).unwrap(), field, "{name}");

        // Merge and compatibility: with itself is itself, and a foreign
        // datatype refuses naming both.
        assert_eq!(dtype.merge_with(dtype, false).unwrap(), *dtype, "{name}");
        let refused = dtype.merge_with(&DataType::Int64, false).unwrap_err();
        let message = refused.to_string();
        assert!(message.contains(name), "{message}");
        assert!(message.contains("int64"), "{message}");
    }
}

#[test]
fn a_coded_value_is_checked_rewritten_and_packed_at_its_own_width() {
    // The value contract accepts the text, rewrites it into the declared
    // representation, and answers an unchanged value untouched.
    let side = DataType::Side.scalar(Scalar::from("1")).unwrap();
    assert!(matches!(side, Scalar::Ascii(AsciiFamily::Side(_))));
    assert_eq!(side.as_str(), Some("1"));
    assert_eq!(DataType::Side.scalar(side.clone()).unwrap(), side);

    // Packing is the crate's existing fixed-ASCII packing at the fixed width:
    // NUL-padded up to it, the padding gone on the way back.
    assert_eq!(
        DataType::Side.ascii_packed(b"1").unwrap(),
        DataType::FixedAscii(4).ascii_packed(b"1").unwrap()
    );
    for (dtype, value) in [
        (DataType::MsgType, "D"),
        (DataType::MsgType, "AB"),
        (DataType::MsgType, "VENUEMSG"),
        (DataType::Side, "1"),
        (DataType::MsgDirection, "SENT"),
        (DataType::MsgDirection, "RECV"),
    ] {
        let packed = dtype.ascii_packed(value.as_bytes()).unwrap();
        let read = dtype.ascii_value(packed).unwrap();
        assert_eq!(read.as_str(), value, "{dtype} {value}");
    }

    // A value longer than the width is the refusal any fixed-ASCII field
    // gives, and it names the type.
    let refused = DataType::Side.scalar(Scalar::from("TOOLONG")).unwrap_err();
    assert!(refused.to_string().contains("4 bytes"), "{refused}");
    assert!(DataType::MsgType.ascii_packed(b"NINECHARS").is_err());
}

#[test]
fn a_msgtype_is_case_sensitive_and_the_crate_fold_never_touches_a_wire_value() {
    // Six pairs the specification distinguishes only by case. A single stray
    // fold turns a quote into a cross.
    let pairs = [
        ("A", "a"),
        ("Q", "q"),
        ("S", "s"),
        ("B", "b"),
        ("C", "c"),
        ("D", "d"),
    ];
    let mut packed = Vec::new();
    for (upper, lower) in pairs {
        let up = DataType::MsgType.ascii_packed(upper.as_bytes()).unwrap();
        let down = DataType::MsgType.ascii_packed(lower.as_bytes()).unwrap();
        assert_ne!(up, down, "{upper} and {lower} must not pack alike");
        packed.push(up);
        packed.push(down);
    }
    packed.sort_unstable();
    packed.dedup();
    assert_eq!(packed.len(), 12, "twelve distinct values");

    // The value keeps its case through the value contract, both ways.
    for value in ["A", "a", "S", "s"] {
        let stored = DataType::MsgType.scalar(Scalar::from(value)).unwrap();
        assert_eq!(stored.as_str(), Some(value));
    }
}

#[test]
fn a_listing_is_a_vocabulary_and_never_a_gate_on_the_value() {
    // These declare a vocabulary exactly as `Mic` does: a venue's own message
    // type, and a side no version defines, are held rather than refused.
    for (dtype, outside) in [
        (DataType::Side, "Z"),
        (DataType::MsgType, "VENUE1"),
        (DataType::MsgDirection, "BOTH"),
    ] {
        let stored = dtype.scalar(Scalar::from(outside)).unwrap();
        assert_eq!(stored.as_str(), Some(outside), "{dtype}");
        let packed = dtype.ascii_packed(outside.as_bytes()).unwrap();
        assert_eq!(dtype.ascii_value(packed).unwrap().as_str(), outside);
    }

    // The listing is what a name resolves from, and two readers answer the
    // same members because it is a constant.
    for (name, count) in [
        ("side", AsciiEnum::SIDES.len()),
        ("msgdirection", AsciiEnum::DIRECTIONS.len()),
    ] {
        let built = AsciiEnum::from_logical_name(name).unwrap();
        assert_eq!(built.len(), count, "{name}");
        assert_eq!(built, AsciiEnum::from_logical_name(name).unwrap(), "{name}");
    }
    // `msgtype` is deliberately not prebuilt: a `field:enum` member name is
    // upper-cased, so `A` and `a` would name one member and twenty-three
    // values would be lost. The datatype keeps its constant vocabulary and
    // the enum answers none.
    assert!(AsciiEnum::from_logical_name("msgtype").unwrap().is_empty());
    assert_eq!(AsciiEnum::MSGTYPES.len(), 152);
    assert_eq!(AsciiEnum::DIRECTIONS, &["RECV", "SENT"][..]);
    // Every prebuilt member fits the width its own datatype fixes.
    for (name, dtype) in [
        ("side", DataType::Side),
        ("msgdirection", DataType::MsgDirection),
    ] {
        AsciiEnum::from_logical_name(name)
            .unwrap()
            .into_members(&dtype)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

#[test]
fn there_is_no_member_meaning_no_answer_and_null_is_how_a_row_says_it() {
    // A row whose line does not say which way it moved has no direction, and
    // the crate already spells "no answer" one way.
    assert!(!AsciiEnum::DIRECTIONS.contains(&"UNKNOWN"));
    assert!(!AsciiEnum::DIRECTIONS.contains(&"NONE"));

    let field = Field::new("direction", DataType::MsgDirection, true);
    let row = Field::new(
        "row",
        DataType::from_fields([field.clone()]).unwrap(),
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
        DataType::from_fields([Field::new("direction", DataType::MsgDirection, false)]).unwrap(),
        false,
    );
    assert!(
        required
            .validate_value(&Scalar::from_sequence([Scalar::Null]))
            .is_err()
    );
}

#[test]
fn a_coded_column_casts_to_text_and_back_and_refuses_a_number() {
    for (dtype, value) in [
        (DataType::Side, "1"),
        (DataType::MsgType, "D"),
        (DataType::MsgDirection, "SENT"),
    ] {
        let stored = dtype.scalar(Scalar::from(value)).unwrap();
        // To text, which is what the value already is.
        let text = DataType::Utf8
            .scalar(Scalar::from(stored.as_str().unwrap()))
            .unwrap();
        assert_eq!(text.as_str(), Some(value));
        // And back, through the same contract.
        assert_eq!(dtype.scalar(text.clone()).unwrap(), stored);
        // A number is not one of these, and the refusal names the type.
        let refused = dtype.scalar(Scalar::from(7_i64)).unwrap_err();
        assert!(
            refused.to_string().contains(&dtype.to_string()),
            "{refused}"
        );
    }
}

#[test]
fn a_direction_is_the_verb_in_front_of_the_payload_and_nothing_else() {
    use yggdryl::types::MsgDirection;

    // Read, with the marker taken off the body.
    for (line, direction, body) in [
        (
            "sending >> 8=FIX.4.2|9=176|35=D|10=203|",
            Some(MsgDirection::SENT),
            ">> 8=FIX.4.2|9=176|35=D|10=203|",
        ),
        (
            "recv 8=FIX.4.4|35=0|10=017|",
            Some(MsgDirection::RECV),
            "8=FIX.4.4|35=0|10=017|",
        ),
        (
            "Receiving XmlApi: <Execution ExecID='E1'/>",
            Some(MsgDirection::RECV),
            "XmlApi: <Execution ExecID='E1'/>",
        ),
        (
            "[OUT] 8=FIX.4.4|35=D|",
            Some(MsgDirection::SENT),
            "8=FIX.4.4|35=D|",
        ),
        ("(in) ACCOUNT=A1", Some(MsgDirection::RECV), "ACCOUNT=A1"),
    ] {
        assert_eq!(MsgDirection::infer_text(line), direction, "{line}");
        assert_eq!(MsgDirection::split_text(line).1, body, "{line}");
    }

    // Nothing read is nothing removed, and these are the shapes that must
    // read nothing.
    for line in [
        // English that merely contains the letters.
        "sending in session 3",
        "received out of order",
        // A route endpoint and a session name, where a word boundary alone
        // would have been enough to get it wrong.
        "direct:out 8=FIX.4.4|35=D|",
        "MCFID-IN-XPAR 8=FIX.4.4|35=D|",
        // Both verbs in one prefix: none a reading can prefer.
        "sending and receiving 8=FIX.4.4|35=D|",
        // A verb only inside the payload is the payload's word.
        "8=FIX.4.4|35=8|58=sent earlier|10=1|",
        "ACCOUNT=A1|TEXT=received late|",
        // No verb at all.
        "8=FIX.4.4|35=D|",
        "no level printed by this plugin",
    ] {
        assert_eq!(MsgDirection::infer_text(line), None, "{line}");
        assert_eq!(MsgDirection::split_text(line).1, line, "{line}");
    }
}
