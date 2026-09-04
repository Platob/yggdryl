use super::super::{AsciiEnum, DataType};

#[test]
fn the_member_name_rule_is_applied_once_per_value() {
    // An ASCII letter is kept uppercased, a digit is kept, every other
    // byte becomes `_`, a leading digit takes a `_` in front, and a name
    // that both opens and closes with `_` drops its trailing ones.
    for (value, member) in [
        ("USD", "USD"),
        ("usd", "USD"),
        ("n/a", "N_A"),
        ("-a-", "_A"),
        ("3M", "_3M"),
        ("", "_"),
    ] {
        assert_eq!(AsciiEnum::member_name(value).as_str(), member, "{value:?}");
    }
}

#[test]
fn an_enum_names_its_values_and_renders_one_document() {
    let mut side =
        AsciiEnum::from_members("Side", [("SELL", "S"), ("BUY", "B"), ("BID", "B")]).unwrap();
    assert_eq!(side.name(), "Side");
    assert_eq!(side.len(), 3);
    assert_eq!(side.get("BID"), Some("B"));

    // Two members may name one value; the first by name reads it back, so
    // an alias never changes what a stored code decodes as.
    assert_eq!(side.get_member("B"), Some("BID"));
    assert_eq!(side.get_member("X"), None);
    assert_eq!(
        side.into_members(&DataType::FixedAscii(4)).unwrap(),
        [
            ("BID".into(), 0x4200_0000),
            ("BUY".into(), 0x4200_0000),
            ("SELL".into(), 0x5300_0000),
        ]
    );

    // One enum is one document however it was built, and the document is
    // what comes back.
    let document = side.into_json();
    assert_eq!(
        document,
        r#"{"members":{"BID":"B","BUY":"B","SELL":"S"},"name":"Side"}"#
    );
    assert_eq!(AsciiEnum::from_json(&document).unwrap(), side);
    assert_eq!(
        AsciiEnum::from_json(r#" {"name":"Side","members":{"SELL":"S","BUY":"B","BID":"B"}} "#)
            .unwrap(),
        side
    );
    assert_eq!(
        AsciiEnum::from_json(r#"{"name":"Empty"}"#).unwrap(),
        AsciiEnum::new("Empty").unwrap()
    );
    assert!(AsciiEnum::new("Empty").unwrap().is_empty());

    assert_eq!(side.remove("BID"), Some("B".into()));
    assert_eq!(side.remove("BID"), None);
    assert_eq!(side.get_member("B"), Some("BUY"));
    assert_eq!(
        side.iter().collect::<Vec<_>>(),
        [("BUY", "B"), ("SELL", "S")]
    );

    // A member the width cannot store is refused when the codes are asked
    // for, which is where the width is known.
    let refused = side.into_members(&DataType::Utf8).unwrap_err().to_string();
    assert!(refused.contains("a fixed ASCII width"), "{refused}");
}

#[test]
fn an_enum_refuses_what_a_document_could_not_carry_back() {
    for (name, member) in [("", "BUY"), ("Side", ""), ("Si\u{7}de", "BUY")] {
        assert!(AsciiEnum::from_members(name, [(member, "B")]).is_err());
    }
    for document in [
        "[]",
        "not json",
        r#"{"members":{"BUY":"B"}}"#,
        r#"{"name":7}"#,
        r#"{"name":"Side","members":[]}"#,
        r#"{"name":"Side","members":{"BUY":7}}"#,
        r#"{"name":"Side","members":{"":"B"}}"#,
    ] {
        assert!(AsciiEnum::from_json(document).is_err(), "{document}");
    }
}

#[test]
fn an_ascii_value_packs_into_the_integer_its_storage_reads_as() {
    for (dtype, value, packed) in [
        (DataType::FixedAscii(4), "USD", 0x5553_4400_i128),
        (DataType::FixedAscii(4), "", 0),
        (DataType::FixedAscii(8), "EUREX", 0x4555_5245_5800_0000),
        (
            DataType::FixedAscii(16),
            "US0378331005",
            0x5553_3033_3738_3333_3130_3035_0000_0000,
        ),
    ] {
        assert_eq!(dtype.ascii_packed(value.as_bytes()).unwrap(), packed);
        // The storage padding is the same value, and reads back trimmed.
        let padded = format!("{value}\0");
        assert_eq!(dtype.ascii_packed(padded.as_bytes()).unwrap(), packed);
        assert_eq!(dtype.ascii_value(packed).unwrap(), value);
    }

    // An ASCII byte never sets the sign bit, so the order of the packed
    // integers is the order of the text.
    assert!(
        DataType::FixedAscii(4).ascii_packed(b"EUR").unwrap()
            < DataType::FixedAscii(4).ascii_packed(b"USD").unwrap()
    );
    assert!(
        DataType::FixedAscii(8)
            .ascii_packed(b"\x7f\x7f\x7f\x7f\x7f\x7f\x7f\x7f")
            .unwrap()
            > 0
    );

    // What the width refuses, the packing refuses, in both directions.
    let refused = DataType::FixedAscii(4)
        .ascii_packed(b"EURO!")
        .unwrap_err()
        .to_string();
    assert!(refused.contains("at most 4 bytes"), "{refused}");
    let refused = DataType::FixedAscii(4)
        .ascii_value(-1)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("wider than the width"), "{refused}");
    let refused = DataType::FixedAscii(4)
        .ascii_value(0x1_5553_4400)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("wider than the width"), "{refused}");
    let refused = DataType::FixedAscii(4)
        .ascii_value(0x0055_4400)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("a NUL byte at 0"), "{refused}");
    let refused = DataType::Utf8.ascii_packed(b"USD").unwrap_err().to_string();
    assert!(refused.contains("a fixed ASCII width"), "{refused}");
    assert!(DataType::Utf8.ascii_value(0).is_err());
    // The variable shape has no width, so it has no packed integer; nor
    // does a width wider than the widest integer this crate carries.
    assert!(DataType::Ascii.ascii_packed(b"USD").is_err());
    assert!(DataType::FixedAscii(17).ascii_packed(b"USD").is_err());
}
