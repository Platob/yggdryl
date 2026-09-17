//! The `Enum` member: its vocabulary name, its serde tag, and its ordinal.

use std::mem::size_of;

use yggdryl::{
    Codec, DataTypeId, DataTypeKind, EdgeAlgorithm, Enum, IOKind, IOMode, Scalar, TimeUnit,
    UnionMode,
};

#[test]
fn the_serde_tag_is_the_name_kind_answers() {
    // The vocabulary name is the Rust type name, so there is one
    // spelling and no casing rule to disagree with. Every vocabulary is
    // checked, not just the two an acronym once broke.
    for (kind, value) in [
        ("Codec", "identity"),
        ("DataTypeId", "null"),
        ("DataTypeKind", "null"),
        ("EdgeAlgorithm", "spherical"),
        ("IOKind", "file"),
        ("IOMode", "append"),
        ("TimeUnit", "s"),
        ("UnionMode", "dense"),
    ] {
        let member = Enum::from_parts(kind, value).expect("a known member");
        assert_eq!(member.kind(), kind);

        let json = serde_json::to_string(&member).expect("Enum serializes");
        assert!(
            json.contains(&format!("\"kind\":\"{kind}\"")),
            "serde wrote {json}, but kind() answers {kind}"
        );
        assert_eq!(
            Enum::from_parts(kind, value).expect("a known member"),
            serde_json::from_str::<Enum>(&json).expect("Enum round-trips")
        );
    }
}

#[test]
fn a_member_is_two_bytes_and_converts_to_the_text_a_column_holds() {
    let member = Enum::from_parts("IOMode", "append").unwrap();
    assert_eq!(member, Enum::IOMode(IOMode::Append));
    assert_eq!(member.kind(), "IOMode");
    assert_eq!(member.as_str(), "append");
    assert_eq!(member.ordinal(), 1);
    assert!(size_of::<Enum>() <= 2);

    // A member's datatype is `string` - there is no `DataType::Enum` - so the
    // scalar is the name, and the vocabulary stays with `Enum`.
    assert_eq!(Scalar::from(IOMode::Append), Scalar::from("append"));
    assert_eq!(Scalar::from(member), Scalar::from("append"));
}

#[test]
fn every_static_vocabulary_round_trips_and_invalid_parts_fail() {
    let members = [
        Enum::from(Codec::Zstd),
        Enum::from(DataTypeId::Int64),
        Enum::from(DataTypeKind::Integer),
        Enum::from(EdgeAlgorithm::Spherical),
        Enum::from(IOKind::File),
        Enum::from(IOMode::Random),
        Enum::from(TimeUnit::Nanosecond),
        Enum::from(UnionMode::Dense),
    ];

    for member in members {
        assert_eq!(
            Enum::from_parts(member.kind(), member.as_str()).unwrap(),
            member
        );
        assert_ne!(member.ordinal(), u8::MAX);
        assert_eq!(Scalar::from(member).as_str(), Some(member.as_str()));
    }

    assert!(Enum::from_parts("missing", "append").is_err());
    assert!(Enum::from_parts("IOMode", "missing").is_err());
}
