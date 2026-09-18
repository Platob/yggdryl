//! The `Vocabulary` member: its vocabulary name, its serde tag, and its ordinal.

use std::mem::size_of;

use yggdryl::{
    Codec, DataTypeId, DataTypeKind, EdgeAlgorithm, Vocabulary, IOKind, IOMode, Scalar, TimeUnit,
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
        let member = Vocabulary::from_parts(kind, value).expect("a known member");
        assert_eq!(member.kind(), kind);

        let json = serde_json::to_string(&member).expect("Vocabulary serializes");
        assert!(
            json.contains(&format!("\"kind\":\"{kind}\"")),
            "serde wrote {json}, but kind() answers {kind}"
        );
        assert_eq!(
            Vocabulary::from_parts(kind, value).expect("a known member"),
            serde_json::from_str::<Vocabulary>(&json).expect("Vocabulary round-trips")
        );
    }
}

#[test]
fn a_member_is_two_bytes_and_converts_to_the_text_a_column_holds() {
    let member = Vocabulary::from_parts("IOMode", "append").unwrap();
    assert_eq!(member, Vocabulary::IOMode(IOMode::Append));
    assert_eq!(member.kind(), "IOMode");
    assert_eq!(member.as_str(), "append");
    assert_eq!(member.ordinal(), 1);
    assert!(size_of::<Vocabulary>() <= 2);

    // A member's datatype is `string` - there is no `DataType::Vocabulary` - so the
    // scalar is the name, and the vocabulary stays with `Vocabulary`.
    assert_eq!(Scalar::from(IOMode::Append), Scalar::from("append"));
    assert_eq!(Scalar::from(member), Scalar::from("append"));
}

#[test]
fn every_static_vocabulary_round_trips_and_invalid_parts_fail() {
    let members = [
        Vocabulary::from(Codec::Zstd),
        Vocabulary::from(DataTypeId::Int64),
        Vocabulary::from(DataTypeKind::Integer),
        Vocabulary::from(EdgeAlgorithm::Spherical),
        Vocabulary::from(IOKind::File),
        Vocabulary::from(IOMode::Random),
        Vocabulary::from(TimeUnit::Nanosecond),
        Vocabulary::from(UnionMode::Dense),
    ];

    for member in members {
        assert_eq!(
            Vocabulary::from_parts(member.kind(), member.as_str()).unwrap(),
            member
        );
        assert_ne!(member.ordinal(), u8::MAX);
        assert_eq!(Scalar::from(member).as_str(), Some(member.as_str()));
    }

    assert!(Vocabulary::from_parts("missing", "append").is_err());
    assert!(Vocabulary::from_parts("IOMode", "missing").is_err());
}

#[test]
fn a_member_reads_by_every_spelling_its_own_vocabulary_accepts() {
    // `from_parts` used to scan `as_str` for each vocabulary, so it accepted
    // only the canonical short spelling. `TimeUnit::as_str` answers `ns`,
    // while the serialized vocabulary is the full `nanosecond` - the only
    // unit spelling on disk, 1,017 times - so this crate wrote a name its own
    // reader refused. Reading through `FromStr` accepts both.
    for spelling in ["ns", "nanosecond", "NANOSECOND", "Nanoseconds"] {
        assert_eq!(
            Vocabulary::from_parts("TimeUnit", spelling).unwrap(),
            Vocabulary::TimeUnit(TimeUnit::Nanosecond),
            "{spelling}"
        );
    }
    for spelling in ["us", "microsecond", "micro seconds"] {
        assert_eq!(
            Vocabulary::from_parts("TimeUnit", spelling).unwrap(),
            Vocabulary::TimeUnit(TimeUnit::Microsecond),
            "{spelling}"
        );
    }

    // `UnionMode` had no `FromStr` at all, which is what stopped `from_parts`
    // routing through the parsers; it has one now.
    for spelling in ["dense", "DENSE", " dense "] {
        assert_eq!(
            Vocabulary::from_parts("UnionMode", spelling).unwrap(),
            Vocabulary::UnionMode(UnionMode::Dense),
            "{spelling}"
        );
    }
    assert!(Vocabulary::from_parts("UnionMode", "packed").is_err());
    assert!(Vocabulary::from_parts("TimeUnit", "fortnight").is_err());
}
