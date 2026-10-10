//! The market register of `rust/src/market.rs`, over two enum kinds no
//! crate claims: `testenum` at `0xc7`, three members stored as `uint8`, and
//! `testwide` at `0xc8`, a member past one byte stored as `uint16`. A claim
//! is process-wide and never withdrawn, so this target owns its process:
//! every test opens with [`installed`] beside the market crate's own
//! install, and no other harness claims a kind but the market crate's four.
//!
//! What is pinned: the claim and what it registers, every refusal a claim
//! can meet, every intake door reaching the kinds through the register -
//! the parser, the field grammar, the logical names, serde, the value
//! stream, Arrow in both directions, a cast, the digest - and the install
//! rule: a name parsed before its claim is refused naming the missing
//! registration, and resolves after it.

#[path = "support/install.rs"]
mod install;
use std::sync::{Arc, LazyLock, OnceLock};

use arrow_array::{ArrayRef, UInt8Array, UInt16Array};
use arrow_schema::DataType as ArrowDataType;
use yggdryl::market::{claim, kind_for_extension, kind_named, kind_of, kinds};
use yggdryl::{
    ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DataTypeValue, DigestAlgorithm, Error,
    Field, MarketDescriptor, MarketMember, MarketStorage, MarketType, Scalar, Serie,
};

/// The crate these tests claim as.
const BY: &str = "yggdryl-tests";

/// The test enum's members, in code order.
const TEST_MEMBERS: &[MarketMember] = &[
    MarketMember {
        code: 0,
        name: "NONE",
        description: "Nothing stated.",
    },
    MarketMember {
        code: 1,
        name: "ONE",
        description: "The first.",
    },
    MarketMember {
        code: 2,
        name: "TWO",
        description: "The second.",
    },
];

/// The wide enum's members, in code order: one past a byte.
const WIDE_MEMBERS: &[MarketMember] = &[
    MarketMember {
        code: 0,
        name: "NONE",
        description: "Nothing stated.",
    },
    MarketMember {
        code: 7,
        name: "SEVEN",
        description: "The seventh.",
    },
    MarketMember {
        code: 300,
        name: "WIDE",
        description: "Past one byte.",
    },
];

/// A stored name in any case, over `members`: what every test kind reads.
fn read_among(
    members: &'static [MarketMember],
    kind: &'static str,
    text: &str,
) -> Result<u16, Error> {
    members
        .iter()
        .find(|member| member.name.eq_ignore_ascii_case(text.trim()))
        .map(|member| member.code)
        .ok_or_else(|| Error::InvalidDataType {
            kind,
            reason: format!("expected a {kind} member's name, got {text:?}").into(),
        })
}

/// The test enum's reader.
fn read_member(text: &str) -> Result<u16, Error> {
    read_among(TEST_MEMBERS, "testenum", text)
}

/// The wide enum's reader.
fn read_wide(text: &str) -> Result<u16, Error> {
    read_among(WIDE_MEMBERS, "testwide", text)
}

/// Three members, read by their stored names in any case.
static TEST_ENUM: MarketDescriptor =
    enum_like(DataTypeId::market(0xc7), "testenum", "yggdryl.testenum");

/// Three members, one past a byte, stored as `uint16`.
static TEST_WIDE: MarketDescriptor = MarketDescriptor {
    storage: MarketStorage::Code16,
    members: WIDE_MEMBERS,
    read: read_wide,
    ..enum_like(DataTypeId::market(0xc8), "testwide", "yggdryl.testwide")
};

/// An enum kind with the test enum's members under a byte, a name and an
/// extension name of its own: what every refusal is tried with.
const fn enum_like(
    id: DataTypeId,
    name: &'static str,
    extension_name: &'static str,
) -> MarketDescriptor {
    MarketDescriptor {
        id,
        name,
        extension_name,
        storage: MarketStorage::Code8,
        members: TEST_MEMBERS,
        value_rank: MarketDescriptor::RESERVED_ENUM_VALUE_RANK,
        dtype_rank: MarketDescriptor::RESERVED_DTYPE_RANK,
        shape: MarketDescriptor::RESERVED_SHAPE,
        read: read_member,
    }
}

/// Claim the two kinds once, before anything reads the register.
fn installed() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        claim(&TEST_ENUM, BY).expect("the enum range holds free bytes");
        claim(&TEST_WIDE, BY).expect("the enum range holds free bytes");
    });
}

/// The census leaves out the two kinds sibling tests claim late
/// (`lateclaim`, `lying`): a claim may land at any instant while this test
/// runs, so the listing is read once, filtered, and both sides of the count
/// are that one filtered reading.
#[test]
fn a_claim_registers_the_kind_under_its_byte_its_name_and_its_extension_name() {
    crate::install::installed();
    installed();
    let held = kind_of(DataTypeId::from_u8(0xc7).unwrap()).unwrap();
    assert_eq!(held.name, "testenum");
    assert!(std::ptr::eq(held, &TEST_ENUM));
    assert!(std::ptr::eq(kind_named("testenum").unwrap(), &TEST_ENUM));
    assert!(std::ptr::eq(
        kind_for_extension("yggdryl.testenum").unwrap(),
        &TEST_ENUM
    ));
    assert!(std::ptr::eq(kind_of(TEST_WIDE.id).unwrap(), &TEST_WIDE));
    assert!(std::ptr::eq(kind_named("testwide").unwrap(), &TEST_WIDE));
    // The market crate's four and the two of this process, in byte order -
    // and the two other tests' own claims, where those tests ran first.
    const LATE: [&str; 2] = ["lateclaim", "lying"];
    let claimed: Vec<&str> = kinds()
        .iter()
        .map(|kind| kind.name)
        .filter(|name| !LATE.contains(name))
        .collect();
    assert_eq!(
        claimed,
        [
            "marketdatakind",
            "side",
            "marketdatatype",
            "timeinforce",
            "testenum",
            "testwide"
        ]
    );
    // The listing splices every kind in after the core's own enum.
    let all: Vec<u8> = DataTypeId::all()
        .iter()
        .filter(|id| !LATE.contains(&id.as_str()))
        .map(|id| id.as_u8())
        .collect();
    let first = all.iter().position(|byte| *byte == 0xc7).unwrap();
    assert_eq!(all[first - 1], 0xc5);
    assert_eq!(all[first + 1], 0xc8);
    assert_eq!(all.len(), DataTypeId::ALL.len() + claimed.len());
    assert_eq!(all.len(), 98);
    assert_eq!(DataTypeId::ALL.len(), 92);
    assert!(TEST_ENUM.id.is_registered() && !TEST_ENUM.id.is_core());
    assert_eq!(TEST_ENUM.id.as_str(), "testenum");
    assert_eq!(
        TEST_WIDE.id.arrow_extension_name(),
        Some("yggdryl.testwide")
    );
    assert_eq!(TEST_ENUM.id.code_width(), None);
    assert_eq!(TEST_ENUM.id.kind(), DataTypeKind::Enum);
}

#[test]
fn a_second_claim_is_refused_naming_the_first_claimant() {
    crate::install::installed();
    installed();
    // The byte, the name and the extension name are each claimed once.
    static SAME_BYTE: MarketDescriptor =
        enum_like(DataTypeId::market(0xc7), "otherbyte", "yggdryl.otherbyte");
    static SAME_NAME: MarketDescriptor =
        enum_like(DataTypeId::market(0xc9), "side", "yggdryl.othername");
    static SAME_EXTENSION: MarketDescriptor = enum_like(
        DataTypeId::market(0xc9),
        "otherextension",
        "yggdryl.timeinforce",
    );
    // A reserved kind's own descriptor claimed again by another crate is
    // the conflict too: the keys are read before the numbers it states.
    for (kind, first, key) in [
        (&SAME_BYTE, BY, "0xc7 (testenum)"),
        (&SAME_NAME, "yggdryl-market", "side"),
        (&SAME_EXTENSION, "yggdryl-market", "yggdryl.timeinforce"),
        (&yggdryl_market::SIDE_KIND, "yggdryl-market", "0xc3 (side)"),
    ] {
        let refused = claim(kind, "another").unwrap_err();
        let text = refused.to_string();
        assert!(refused.is_conflict(), "{kind}: {text}");
        assert!(text.contains(first), "{kind}: {text}");
        assert!(text.contains(key), "{kind}: {text}");
    }
    assert!(
        claim(&TEST_ENUM, BY).unwrap_err().is_conflict(),
        "the same crate twice"
    );
    // Nothing of the refused kinds is registered.
    assert!(kind_named("otherbyte").is_none());
    assert!(kind_named("otherextension").is_none());
    assert!(kind_for_extension("yggdryl.othername").is_none());
    assert!(DataTypeId::from_u8(0xc9).is_none());
}

#[test]
fn a_byte_outside_the_enum_family_a_family_number_a_retired_byte_and_a_core_byte_are_refused() {
    crate::install::installed();
    installed();
    static OUTSIDE: MarketDescriptor = enum_like(DataTypeId::Int32, "outside", "yggdryl.outside");
    static CODE_BYTE: MarketDescriptor =
        enum_like(DataTypeId::Isin, "codebyte", "yggdryl.codebyte");
    static ENUM_FAMILY: MarketDescriptor =
        enum_like(DataTypeId::market(0xc0), "enumfamily", "yggdryl.enumfamily");
    static RETIRED: MarketDescriptor =
        enum_like(DataTypeId::market(0xc6), "retired", "yggdryl.retired");
    static CORE_BYTE: MarketDescriptor =
        enum_like(DataTypeId::market(0xc1), "notstate", "yggdryl.notstate");
    for (kind, reason) in [
        (&OUTSIDE, "got 0x13 in the integer family"),
        (&CODE_BYTE, "got 0x78 in the code family"),
        (&ENUM_FAMILY, "the enum family's own number 0xc0"),
        (&RETIRED, "got 0xc6, which a retired kind held"),
        (&CORE_BYTE, "got 0xc1, which the core's `state` holds"),
    ] {
        let refused = claim(kind, "another").unwrap_err().to_string();
        assert!(refused.contains(reason), "{}: {refused}", kind.name);
        assert!(
            kind_named(kind.name).is_none(),
            "{} was registered",
            kind.name
        );
    }
    // A code is the core's own, flat variant: its bytes are no claim's.
    assert_eq!(DataTypeKind::Code.range(), 0x6a..=0x7f);
    assert_eq!(DataTypeKind::Enum.range(), 0xc0..=0xcf);
    assert!(DataTypeId::Isin.is_core());
    assert!(!DataTypeId::Isin.is_registered());
    assert!(kind_of(DataTypeId::Isin).is_none());
    assert!(kind_named("isin").is_none());
}

/// What a kind states of itself is checked before any key is taken: a
/// reserved kind is claimed at its own numbers by `yggdryl-market` and
/// every other kind takes the reserved numbers, the members are in code
/// order at the storage's width, the name is folded and no word the grammar
/// already reads, and the extension name is the kind's own.
#[test]
fn a_descriptor_disagreeing_with_itself_or_with_the_core_is_refused_before_any_key_is_taken() {
    crate::install::installed();
    installed();
    const fn with_numbers(value_rank: u8, dtype_rank: u8, shape: isize) -> MarketDescriptor {
        let mut kind = enum_like(DataTypeId::market(0xcc), "numbered", "yggdryl.numbered");
        kind.value_rank = value_rank;
        kind.dtype_rank = dtype_rank;
        kind.shape = shape;
        kind
    }
    static CORE_VALUE_RANK: MarketDescriptor = with_numbers(
        5,
        MarketDescriptor::RESERVED_DTYPE_RANK,
        MarketDescriptor::RESERVED_SHAPE,
    );
    static CORE_DTYPE_RANK: MarketDescriptor = with_numbers(
        MarketDescriptor::RESERVED_ENUM_VALUE_RANK,
        30,
        MarketDescriptor::RESERVED_SHAPE,
    );
    static CORE_SHAPE: MarketDescriptor = with_numbers(
        MarketDescriptor::RESERVED_ENUM_VALUE_RANK,
        MarketDescriptor::RESERVED_DTYPE_RANK,
        23,
    );
    const UNORDERED: &[MarketMember] = &[
        MarketMember {
            code: 1,
            name: "ONE",
            description: "one",
        },
        MarketMember {
            code: 0,
            name: "ZERO",
            description: "zero",
        },
    ];
    static NO_MEMBER: LazyLock<MarketDescriptor> = LazyLock::new(|| MarketDescriptor {
        members: &[],
        ..enum_like(DataTypeId::market(0xcc), "nomember", "yggdryl.nomember")
    });
    static OUT_OF_ORDER: LazyLock<MarketDescriptor> = LazyLock::new(|| MarketDescriptor {
        members: UNORDERED,
        ..enum_like(DataTypeId::market(0xcc), "unordered", "yggdryl.unordered")
    });
    static TOO_WIDE: LazyLock<MarketDescriptor> = LazyLock::new(|| MarketDescriptor {
        members: WIDE_MEMBERS,
        ..enum_like(DataTypeId::market(0xcc), "toowide", "yggdryl.toowide")
    });
    static UNFOLDED: MarketDescriptor =
        enum_like(DataTypeId::market(0xcc), "Un_folded", "yggdryl.unfolded");
    static GRAMMAR_WORD: MarketDescriptor =
        enum_like(DataTypeId::market(0xcc), "exchange", "yggdryl.exchange");
    static CORE_WORD: MarketDescriptor =
        enum_like(DataTypeId::market(0xcc), "isin", "yggdryl.wearsisin");
    static CORE_EXTENSION: MarketDescriptor =
        enum_like(DataTypeId::market(0xcc), "wearsuuid", "arrow.uuid");
    static CODE_EXTENSION: MarketDescriptor =
        enum_like(DataTypeId::market(0xcc), "wearsisin", "yggdryl.isin");
    for (kind, reason) in [
        (
            &CORE_VALUE_RANK,
            "expected the reserved (value_rank, dtype_rank, shape) (33, 81, 73), got (5, 81, 73)",
        ),
        (&CORE_DTYPE_RANK, "got (33, 30, 73)"),
        (&CORE_SHAPE, "got (33, 81, 23)"),
        (
            &*NO_MEMBER,
            "expected at least one member of an enum kind, got none",
        ),
        (
            &*OUT_OF_ORDER,
            "expected members in ascending code order, got ONE (1) before ZERO (0)",
        ),
        (
            &*TOO_WIDE,
            "expected every member's code to fit Code8, got WIDE (300)",
        ),
        (&UNFOLDED, "expected a name in its folded spelling"),
        (
            &GRAMMAR_WORD,
            "expected a name the datatype grammar does not read, got \"exchange\"",
        ),
        (
            &CORE_WORD,
            "expected a name the datatype grammar does not read, got \"isin\"",
        ),
        (
            &CORE_EXTENSION,
            "got \"arrow.uuid\", which the core recognizes",
        ),
        (
            &CODE_EXTENSION,
            "got \"yggdryl.isin\", which the core recognizes",
        ),
    ] {
        let refused = claim(kind, "another").unwrap_err().to_string();
        assert!(refused.contains(reason), "{}: {refused}", kind.name);
        assert!(
            kind_named(kind.name).is_none(),
            "{} was registered",
            kind.name
        );
        assert!(
            kind_for_extension(kind.extension_name).is_none(),
            "{}",
            kind.name
        );
    }
    // No kind is claimed in the core's name - a reserved kind is the market
    // crate's to claim - so such a claim is refused before anything else is
    // read.
    static AS_CORE: MarketDescriptor =
        enum_like(DataTypeId::market(0xcc), "ascore", "yggdryl.ascore");
    let refused = claim(&AS_CORE, "yggdryl").unwrap_err().to_string();
    assert!(
        refused.contains("expected the claiming crate's own name"),
        "{refused}"
    );
    // The register holds the two test kinds and nothing of what was
    // refused.
    assert!(kind_named("ascore").is_none());
    assert!(DataTypeId::from_u8(0xcc).is_none());
    assert!(kinds().iter().all(|kind| {
        [
            "marketdatakind",
            "side",
            "marketdatatype",
            "timeinforce",
            "testenum",
            "testwide",
            "lateclaim",
            "lying",
        ]
        .contains(&kind.name)
    }));
}

/// A descriptor is the kind only where it is the one claimed under its
/// byte: a copy stating the same byte - never claimed, or refused at its
/// claim - validates as no kind, so nothing appends a copy's column onto
/// the kind's or compares the two as one datatype through a value door.
#[test]
fn a_descriptor_that_is_not_the_claimed_one_validates_as_no_kind() {
    crate::install::installed();
    installed();
    static COPY: MarketDescriptor =
        enum_like(DataTypeId::market(0xc7), "testenum", "yggdryl.testenum");
    let refused = MarketType::new(&COPY)
        .into_dtype()
        .validate()
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("no registered datatype answers"),
        "{refused}"
    );
    assert!(MarketType::new(&TEST_ENUM).into_dtype().validate().is_ok());
    // The doors that mint a value ask the same: a copy mints nothing.
    for refused in [
        COPY.scalar("one").unwrap_err().to_string(),
        COPY.member(1).unwrap_err().to_string(),
        COPY.default_scalar().unwrap_err().to_string(),
    ] {
        assert!(
            refused.contains("no registered datatype answers"),
            "{refused}"
        );
    }
    // The reader's answer is held to the member table: a code no member
    // holds is refused rather than held, by the value door and by a cast
    // alike, so a kind whose reader lies never lands a code in a column.
    static LYING: MarketDescriptor = MarketDescriptor {
        read: |_| Ok(200),
        ..enum_like(DataTypeId::market(0xcd), "lying", "yggdryl.lying")
    };
    claim(&LYING, BY).unwrap();
    let refused = LYING.scalar("one").unwrap_err().to_string();
    assert!(
        refused.contains("got 200, which no member holds"),
        "{refused}"
    );
    let names = Serie::from_scalars(
        DataType::utf8().nullable_field("member"),
        [Scalar::from("one")],
    )
    .unwrap();
    let lying = LYING.dtype().nullable_field("member");
    assert!(
        names
            .cast(&lying, ArrowCastOptions::new().with_safe(false))
            .is_err()
    );
    assert_eq!(
        names
            .cast(&lying, ArrowCastOptions::new())
            .unwrap()
            .scalar(0)
            .unwrap(),
        Scalar::Null
    );
    assert!(TEST_ENUM.member(200).is_err());
    assert!(TEST_ENUM.member(-1).is_err());
    assert_eq!(TEST_ENUM.default_scalar().unwrap().enum_code(), Some(0));
}

#[test]
fn every_intake_door_reaches_a_registered_kind() {
    crate::install::installed();
    installed();
    let dtype = DataType::from_str("testenum").unwrap();
    assert_eq!(dtype, TEST_ENUM.dtype());
    assert_eq!(dtype.id(), TEST_ENUM.id);
    assert_eq!(dtype.to_string(), "testenum");
    assert!(dtype.is_enum() && !dtype.is_code());
    assert_eq!(dtype.code_width(), None);
    assert_eq!(DataTypeId::from_str("TestEnum").unwrap(), TEST_ENUM.id);
    assert_eq!(Field::from_str("value testenum").unwrap().dtype(), &dtype);
    assert_eq!(DataType::from_logical_name("testenum").unwrap(), dtype);
    assert!(
        DataType::logical_names()
            .iter()
            .any(|(name, held)| *name == "testenum" && *held == dtype)
    );
    let wide = DataType::from_str("testwide").unwrap();
    assert_eq!(wide, TEST_WIDE.dtype());
    assert!(wide.is_enum());

    // The value door is the kind's own reader, and the integer door its
    // member table.
    let one = dtype.scalar("one").unwrap();
    assert_eq!(one.kind(), "testenum");
    assert_eq!(one.id(), TEST_ENUM.id);
    assert_eq!(one.enum_code(), Some(1));
    assert_eq!(one.enum_name(), Some("ONE"));
    assert_eq!(one.as_str(), Some("ONE"));
    assert_eq!(
        dtype.scalar(Scalar::from(2_i64)).unwrap().enum_name(),
        Some("TWO")
    );
    assert!(dtype.scalar("three").is_err());
    assert!(dtype.scalar(Scalar::from(3_i64)).is_err());
    let far = wide.scalar("wide").unwrap();
    assert_eq!(far.enum_code(), Some(300));
    assert_eq!(wide.scalar(Scalar::from(300_i64)).unwrap(), far);
    // A value of another kind is a value of another vocabulary, a code's
    // text included.
    assert!(dtype.scalar(far.clone()).is_err());
    assert!(dtype.scalar(DataType::Ccy.scalar("USD").unwrap()).is_err());
    assert!(
        DataType::from_str("side")
            .unwrap()
            .scalar(one.clone())
            .is_err()
    );

    // Serde: the tags of the datatype, the field and the value.
    assert_eq!(
        serde_json::to_string(&dtype).unwrap(),
        r#"{"type":"testenum"}"#
    );
    assert_eq!(
        serde_json::from_str::<DataType>(r#"{"type":"testenum"}"#).unwrap(),
        dtype
    );
    let field = dtype.clone().nullable_field("member");
    assert_eq!(
        serde_json::from_str::<Field>(&serde_json::to_string(&field).unwrap()).unwrap(),
        field
    );
    assert_eq!(
        serde_json::to_string(&one).unwrap(),
        r#"{"type":"testenum","value":"ONE"}"#
    );
    assert_eq!(
        serde_json::from_str::<Scalar>(r#"{"type":"testenum","value":"one"}"#).unwrap(),
        one
    );
    assert_eq!(
        serde_json::from_str::<Scalar>(r#"{"value":"ONE","type":"testenum"}"#).unwrap(),
        one
    );
    assert!(serde_json::from_str::<Scalar>(r#"{"type":"testenum","value":"three"}"#).is_err());

    // The value stream: the kind's byte is the tag, the code four bytes.
    let bytes = one.into_value_bytes();
    assert_eq!(bytes[1], 0xc7, "{bytes:?}");
    assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), one);
    assert_eq!(dtype.decode_value_bytes(&bytes).unwrap(), one);
    let far_bytes = far.into_value_bytes();
    assert_eq!(far_bytes[1], 0xc8, "{far_bytes:?}");
    assert_eq!(Scalar::decode_value_bytes(&far_bytes).unwrap(), far);

    // Arrow: the extension name over the kind's storage, both directions.
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt8);
    assert_eq!(
        arrow
            .metadata()
            .get("ARROW:extension:name")
            .map(String::as_str),
        Some("yggdryl.testenum")
    );
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);
    let storage: ArrayRef = Arc::new(UInt8Array::from(vec![1_u8]));
    let landed = Serie::from_arrow_array(Some(&field), storage, ArrowCastOptions::new()).unwrap();
    assert_eq!(landed.scalar(0).unwrap(), one);
    assert!(landed.as_uint8().is_some());
    let wide_field = wide.clone().nullable_field("member");
    let arrow = wide_field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt16);
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), wide_field);
    let storage: ArrayRef = Arc::new(UInt16Array::from(vec![300_u16]));
    let landed =
        Serie::from_arrow_array(Some(&wide_field), storage, ArrowCastOptions::new()).unwrap();
    assert_eq!(landed.scalar(0).unwrap(), far);
    // A code no member holds is refused where it lands: null under a
    // nullable field where the cast is safe, a refusal otherwise.
    let storage: ArrayRef = Arc::new(UInt8Array::from(vec![9_u8]));
    assert_eq!(
        Serie::from_arrow_array(Some(&field), Arc::clone(&storage), ArrowCastOptions::new())
            .unwrap()
            .scalar(0)
            .unwrap(),
        Scalar::Null
    );
    assert!(
        Serie::from_arrow_array(
            Some(&field),
            Arc::clone(&storage),
            ArrowCastOptions::new().with_safe(false)
        )
        .is_err()
    );
    assert!(
        Serie::from_arrow_array(
            Some(&dtype.clone().required_field("member")),
            storage,
            ArrowCastOptions::new()
        )
        .is_err()
    );

    // A cast reads every cell through the kind's own reader.
    let names = Serie::from_scalars(
        DataType::utf8().nullable_field("member"),
        [Scalar::from("two"), Scalar::from("NONE"), Scalar::Null],
    )
    .unwrap();
    let cast = names.cast(&field, ArrowCastOptions::new()).unwrap();
    assert_eq!(cast.scalar(0).unwrap().enum_name(), Some("TWO"));
    assert_eq!(cast.scalar(1).unwrap().enum_code(), Some(0));
    assert_eq!(cast.scalar(2).unwrap(), Scalar::Null);
    assert!(
        names
            .cast(
                &dtype.clone().required_field("member"),
                ArrowCastOptions::new()
            )
            .is_err()
    );
    let codes = Serie::from_scalars(
        DataType::Int64.nullable_field("member"),
        [Scalar::from(1_i64), Scalar::from(2_i64)],
    )
    .unwrap();
    let cast = codes.cast(&field, ArrowCastOptions::new()).unwrap();
    assert_eq!(cast.scalar(0).unwrap(), one);
    // Another enum's codes are another vocabulary, which no cast crosses.
    assert!(
        cast.cast(
            &DataType::from_str("side").unwrap().nullable_field("member"),
            ArrowCastOptions::new()
        )
        .is_err()
    );

    // The digest is the code at the kind's width: stable, and the datatype
    // apart from every other kind's.
    assert_eq!(
        one.stable_hash(),
        dtype.scalar("ONE").unwrap().stable_hash()
    );
    assert_eq!(
        one.digest(DigestAlgorithm::Xxh3).as_u64(),
        Some(one.stable_hash())
    );
    assert_eq!(dtype.stable_hash(), TEST_ENUM.dtype().stable_hash());
    assert_ne!(dtype.stable_hash(), wide.stable_hash());
    assert_ne!(
        dtype.stable_hash(),
        DataType::from_str("side").unwrap().stable_hash()
    );
}

#[test]
fn a_name_parsed_before_its_claim_is_refused_naming_the_registration_and_resolves_after_it() {
    crate::install::installed();
    installed();
    // Not yet claimed: every intake door refuses, naming the registration
    // the word lacks and no install - `lateclaim` is no reserved kind.
    for refused in [
        DataType::from_str("lateclaim").unwrap_err().to_string(),
        DataTypeId::from_str("lateclaim").unwrap_err().to_string(),
        serde_json::from_str::<DataType>(r#"{"type":"lateclaim"}"#)
            .unwrap_err()
            .to_string(),
        serde_json::from_str::<Scalar>(r#"{"type":"lateclaim","value":"ONE"}"#)
            .unwrap_err()
            .to_string(),
        Scalar::decode_value_bytes(&[0, 0xcb, 1, 0, 0, 0])
            .unwrap_err()
            .to_string(),
    ] {
        assert!(
            refused.contains("no registered datatype answers"),
            "{refused}"
        );
        assert!(!refused.contains("yggdryl_market::install()"), "{refused}");
    }
    assert!(DataTypeId::from_u8(0xcb).is_none());
    // The Arrow fallback is lossless: an unregistered name imports as its storage.
    let foreign = arrow_schema::Field::new("value", ArrowDataType::UInt8, true).with_metadata(
        [(
            "ARROW:extension:name".to_owned(),
            "yggdryl.lateclaim".to_owned(),
        )]
        .into_iter()
        .collect(),
    );
    assert_eq!(
        Field::from_arrow_field(&foreign).unwrap().dtype(),
        &DataType::UInt8
    );

    // Claimed: the same doors resolve.
    static LATE_ENUM: MarketDescriptor =
        enum_like(DataTypeId::market(0xcb), "lateclaim", "yggdryl.lateclaim");
    claim(&LATE_ENUM, BY).unwrap();
    let dtype = DataType::from_str("lateclaim").unwrap();
    assert_eq!(dtype, LATE_ENUM.dtype());
    assert_eq!(DataTypeId::from_u8(0xcb).unwrap(), LATE_ENUM.id);
    assert_eq!(
        serde_json::from_str::<DataType>(r#"{"type":"lateclaim"}"#).unwrap(),
        dtype
    );
    assert_eq!(Field::from_arrow_field(&foreign).unwrap().dtype(), &dtype);
    assert_eq!(dtype.scalar("one").unwrap().enum_code(), Some(1));
    assert_eq!(
        Scalar::decode_value_bytes(&[0, 0xcb, 1, 0, 0, 0]).unwrap(),
        dtype.scalar("one").unwrap()
    );
}
