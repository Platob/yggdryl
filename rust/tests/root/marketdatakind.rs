//! `rust/src/marketdatakind.rs`: the business category of a market data
//! element - FIX's MsgCat code set - as an Enum-family leaf stored as the
//! `int32` code of its member.
//!
//! `DataType` is `#[non_exhaustive]` and the datatype layer carries some sixty
//! wildcard arms, so a new variant compiles clean while behaving wrongly. A
//! green build proves nothing; these are the invariants a wildcard cannot
//! satisfy by accident, one per codec arm the leaf crosses.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int32Array, Int64Array, StringArray};
use arrow_schema::DataType as ArrowDataType;
use yggdryl::{
    ArrowCastOptions, DataType, DataTypeId, DataTypeKind, DigestAlgorithm, EnumValue, Field,
    MarketDataKind, Scalar, Serie,
};

fn strict() -> ArrowCastOptions {
    ArrowCastOptions::new().with_safe(false)
}

/// The MsgCat code set and the four batch categories after it: every member
/// with its code and its four-letter name.
const MEMBERS: [(MarketDataKind, u8, &str); 26] = [
    (MarketDataKind::Unknown, 0, "UKNW"),
    (MarketDataKind::Account, 1, "ACCT"),
    (MarketDataKind::Allocation, 2, "ALLO"),
    (MarketDataKind::Book, 3, "BOOK"),
    (MarketDataKind::Certificate, 4, "CERT"),
    (MarketDataKind::Collateral, 5, "COLL"),
    (MarketDataKind::Communication, 6, "COMM"),
    (MarketDataKind::Confirmation, 7, "CONF"),
    (MarketDataKind::Execution, 8, "EXEC"),
    (MarketDataKind::MarketStructure, 9, "MKST"),
    (MarketDataKind::Order, 10, "ORDR"),
    (MarketDataKind::Payment, 11, "PAYM"),
    (MarketDataKind::Position, 12, "POSN"),
    (MarketDataKind::Parties, 13, "PRTY"),
    (MarketDataKind::Quotation, 14, "QUOT"),
    (MarketDataKind::Registration, 15, "REGI"),
    (MarketDataKind::Risk, 16, "RISK"),
    (MarketDataKind::Securities, 17, "SECU"),
    (MarketDataKind::Session, 18, "SESS"),
    (MarketDataKind::Settlement, 19, "SETL"),
    (MarketDataKind::Stream, 20, "STRM"),
    (MarketDataKind::Trade, 21, "TRAD"),
    (MarketDataKind::OrderBatch, 22, "ORDB"),
    (MarketDataKind::QuoteBatch, 23, "QUOB"),
    (MarketDataKind::ExecutionBatch, 24, "EXEB"),
    (MarketDataKind::TradeBatch, 25, "TRDB"),
];

#[test]
fn a_code_or_a_spelling_that_names_no_kind_is_refused_by_name() {
    assert_eq!(MarketDataKind::from_code(26), None);
    assert!(MarketDataKind::read_code(-1).is_err());
    let refused = MarketDataKind::read_code(4_242).unwrap_err().to_string();
    assert!(refused.contains("4242"), "{refused}");
    assert!(refused.contains("marketdatakind"), "{refused}");
    let refused = MarketDataKind::read_code(i64::from(i32::MAX) + 1)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("2147483648"), "{refused}");

    let refused = MarketDataKind::read("whatever").unwrap_err().to_string();
    assert!(refused.contains("whatever"), "{refused}");
    assert!(refused.contains("marketdatakind"), "{refused}");
    assert_eq!(MarketDataKind::from_spelling(""), None);
    // A stored code is an integer and never text.
    assert_eq!(MarketDataKind::from_spelling("10"), None);
    assert_eq!(MarketDataKind::from_spelling("ORDER_X"), None);
}

/// `UKNW` is the zero member's four-letter spelling, and `UNKN`, the
/// spelling it was stored under before, names no kind at any door, so a
/// dictionary whose `msgcatcodeset` still names it is rebuilt, never read
/// back as the member; a stored column holds the code `0` and reads
/// unchanged.
#[test]
fn the_retired_spelling_unkn_names_no_kind() {
    assert_eq!(
        MarketDataKind::from_spelling("UKNW"),
        Some(MarketDataKind::Unknown)
    );
    assert_eq!(
        MarketDataKind::from_spelling("uknw"),
        Some(MarketDataKind::Unknown)
    );
    assert_eq!(
        MarketDataKind::from_spelling("Unknown"),
        Some(MarketDataKind::Unknown)
    );
    for spelling in ["UNKN", "unkn", "Unkn"] {
        assert_eq!(MarketDataKind::from_spelling(spelling), None, "{spelling}");
        let refused = MarketDataKind::read(spelling).unwrap_err().to_string();
        assert!(refused.contains("marketdatakind"), "{refused}");
        assert!(refused.contains(spelling), "{refused}");
    }
    assert!(serde_json::from_str::<MarketDataKind>("\"UNKN\"").is_err());
    assert_eq!(
        serde_json::from_str::<MarketDataKind>("\"UKNW\"").unwrap(),
        MarketDataKind::Unknown
    );
    assert!(
        DataType::MarketDataKind
            .scalar(Scalar::from("UNKN"))
            .is_err()
    );
}

#[test]
fn the_members_are_the_msgcat_code_set_in_code_order() {
    assert_eq!(MarketDataKind::ALL.len(), 26);
    assert_eq!(MarketDataKind::ALL.len(), MEMBERS.len());
    for ((member, code, name), held) in MEMBERS.into_iter().zip(MarketDataKind::ALL) {
        assert_eq!(member, *held, "{name}");
        assert_eq!(member.code(), code, "{name}");
        assert_eq!(member.as_str(), name);
        assert_eq!(member.to_string(), name);
        assert_eq!(MarketDataKind::from_code(code), Some(member), "{name}");
        assert_eq!(MarketDataKind::from_name(name), Some(member), "{name}");
        assert_eq!(MarketDataKind::try_from(code).unwrap(), member);
        assert_eq!(u8::from(member), code);
        assert!(!member.description().is_empty(), "{name}");
        assert!(member.description().ends_with('.'), "{name}");
    }
    // Codes unique and ascending, names unique.
    let codes: Vec<u8> = MarketDataKind::ALL.iter().map(|held| held.code()).collect();
    assert_eq!(codes, (0..26).collect::<Vec<_>>());
    let mut names: Vec<&str> = MarketDataKind::ALL
        .iter()
        .map(|held| held.as_str())
        .collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 26, "one name per member");
    let mut members = MarketDataKind::ALL.to_vec();
    members.reverse();
    members.sort_unstable();
    assert_eq!(members, MarketDataKind::ALL);
    assert_eq!(MarketDataKind::default(), MarketDataKind::Unknown);
    assert_eq!(std::mem::size_of::<MarketDataKind>(), 1);
}

/// Only an order and an execution are sided: their cross code is stored
/// under their side. A quote holds its bid and its ask in one element, so it
/// is no more sided than a trade or a book. Each batch files many of one
/// single category, which is its item; every other kind is its own item.
#[test]
fn only_orders_and_executions_are_sided_and_each_batch_names_its_item() {
    for (name, kind, sided, batch, item) in [
        (
            "ORDR",
            MarketDataKind::Order,
            true,
            false,
            MarketDataKind::Order,
        ),
        (
            "QUOT",
            MarketDataKind::Quotation,
            false,
            false,
            MarketDataKind::Quotation,
        ),
        (
            "EXEC",
            MarketDataKind::Execution,
            true,
            false,
            MarketDataKind::Execution,
        ),
        (
            "TRAD",
            MarketDataKind::Trade,
            false,
            false,
            MarketDataKind::Trade,
        ),
        (
            "BOOK",
            MarketDataKind::Book,
            false,
            false,
            MarketDataKind::Book,
        ),
        (
            "UKNW",
            MarketDataKind::Unknown,
            false,
            false,
            MarketDataKind::Unknown,
        ),
        (
            "ORDB",
            MarketDataKind::OrderBatch,
            false,
            true,
            MarketDataKind::Order,
        ),
        (
            "QUOB",
            MarketDataKind::QuoteBatch,
            false,
            true,
            MarketDataKind::Quotation,
        ),
        (
            "EXEB",
            MarketDataKind::ExecutionBatch,
            false,
            true,
            MarketDataKind::Execution,
        ),
        (
            "TRDB",
            MarketDataKind::TradeBatch,
            false,
            true,
            MarketDataKind::Trade,
        ),
    ] {
        assert_eq!(kind.as_str(), name);
        assert_eq!(kind.is_sided(), sided, "{name}");
        assert_eq!(kind.is_batch(), batch, "{name}");
        assert_eq!(kind.item(), item, "{name}");
    }
    let sided: Vec<&str> = MarketDataKind::ALL
        .iter()
        .filter(|kind| kind.is_sided())
        .map(|kind| kind.as_str())
        .collect();
    assert_eq!(sided, ["EXEC", "ORDR"]);
    for spelling in [
        "order_batch",
        "OrderBatch",
        "ordb",
        "quotebatch",
        "trade-batch",
    ] {
        assert!(
            MarketDataKind::from_spelling(spelling).is_some_and(MarketDataKind::is_batch),
            "{spelling}"
        );
    }
}

/// A book folds an order, a quote and a book's own snapshot; every other
/// kind - an execution, a trade, a batch, a session message, an unfiled
/// one - is pruned before the fold.
#[test]
fn only_orders_quotes_and_books_are_booked() {
    let booked: Vec<&str> = MarketDataKind::ALL
        .iter()
        .filter(|kind| kind.is_booked())
        .map(|kind| kind.as_str())
        .collect();
    assert_eq!(booked, ["BOOK", "ORDR", "QUOT"]);
    for kind in [
        MarketDataKind::Execution,
        MarketDataKind::Trade,
        MarketDataKind::OrderBatch,
        MarketDataKind::QuoteBatch,
        MarketDataKind::ExecutionBatch,
        MarketDataKind::TradeBatch,
        MarketDataKind::Session,
        MarketDataKind::Unknown,
    ] {
        assert!(!kind.is_booked(), "{}", kind.as_str());
    }
    // A batch's item is booked exactly when it is an order or a quote.
    assert!(MarketDataKind::OrderBatch.item().is_booked());
    assert!(MarketDataKind::QuoteBatch.item().is_booked());
    assert!(!MarketDataKind::ExecutionBatch.item().is_booked());
    assert!(!MarketDataKind::TradeBatch.item().is_booked());
}

#[test]
fn a_kind_answers_its_code_in_any_case_and_its_folded_word() {
    for (spelling, expected) in [
        ("ORDR", MarketDataKind::Order),
        ("ordr", MarketDataKind::Order),
        ("Ordr", MarketDataKind::Order),
        ("order", MarketDataKind::Order),
        ("Order", MarketDataKind::Order),
        ("QUOT", MarketDataKind::Quotation),
        ("quotation", MarketDataKind::Quotation),
        ("EXEC", MarketDataKind::Execution),
        ("execution", MarketDataKind::Execution),
        ("TRAD", MarketDataKind::Trade),
        ("trade", MarketDataKind::Trade),
        ("BOOK", MarketDataKind::Book),
        ("book", MarketDataKind::Book),
        ("MKST", MarketDataKind::MarketStructure),
        ("market_structure", MarketDataKind::MarketStructure),
        ("Market Structure", MarketDataKind::MarketStructure),
        ("UKNW", MarketDataKind::Unknown),
        ("unknown", MarketDataKind::Unknown),
    ] {
        assert_eq!(
            MarketDataKind::from_spelling(spelling),
            Some(expected),
            "{spelling}"
        );
        assert_eq!(MarketDataKind::read(spelling).unwrap(), expected);
        assert_eq!(
            MarketDataKind::from_spelling(expected.as_str()),
            Some(expected)
        );
    }
    assert_eq!(<MarketDataKind as EnumValue>::KIND, "marketdatakind");
    assert_eq!(
        <MarketDataKind as EnumValue>::EXTENSION_NAME,
        "yggdryl.marketdatakind"
    );
}

#[test]
fn a_kind_is_a_datatype_of_the_enum_family() {
    assert_eq!(DataType::MarketDataKind.id(), DataTypeId::MarketDataKind);
    assert_eq!(DataTypeId::MarketDataKind.as_u8(), 0xc2);
    assert_eq!(DataTypeId::MarketDataKind.as_str(), "marketdatakind");
    assert_eq!(DataTypeId::MarketDataKind.kind(), DataTypeKind::Enum);
    assert!(DataTypeKind::Enum.contains(DataTypeId::MarketDataKind));
    assert!(DataType::MarketDataKind.is_enum());
    assert!(!DataType::MarketDataKind.is_code());
    assert!(!DataType::MarketDataKind.is_string());
    assert_eq!(DataType::MarketDataKind.code_width(), None);
    assert_eq!(DataType::MarketDataKind.code_name(), None);
    assert_eq!(
        DataType::from_str("marketdatakind").unwrap(),
        DataType::MarketDataKind
    );
    assert_eq!(
        DataType::from_str("MarketDataKind").unwrap(),
        DataType::MarketDataKind
    );
    assert_eq!(DataType::MarketDataKind.to_string(), "marketdatakind");
    assert_eq!(DataType::marketdatakind(), DataType::MarketDataKind);
    assert_eq!(
        DataType::MarketDataKind.default_value().unwrap(),
        Scalar::MarketDataKind(MarketDataKind::Unknown)
    );
    assert!(
        DataType::MarketDataKind
            .is_default_value(&Scalar::MarketDataKind(MarketDataKind::Unknown))
            .unwrap()
    );
    assert!(
        !DataType::MarketDataKind
            .is_default_value(&Scalar::MarketDataKind(MarketDataKind::Order))
            .unwrap()
    );
    // The datatype's own wire is the one spelling, and it round-trips.
    let json = DataType::MarketDataKind.into_json().unwrap();
    assert_eq!(json, r#"{"type":"marketdatakind"}"#);
    assert_eq!(
        DataType::from_json(&json).unwrap(),
        DataType::MarketDataKind
    );
    let rendered = serde_json::to_string(&DataType::MarketDataKind).unwrap();
    assert_eq!(rendered, r#"{"type":"marketdatakind"}"#);
    assert_eq!(
        serde_json::from_str::<DataType>(&rendered).unwrap(),
        DataType::MarketDataKind
    );
    assert_eq!(
        DataType::from_logical_name("marketdatakind").unwrap(),
        DataType::MarketDataKind
    );
}

#[test]
fn the_value_door_reads_a_member_a_code_and_a_spelling() {
    let field = DataType::MarketDataKind.required_field("marketdatakind");
    let order = Scalar::MarketDataKind(MarketDataKind::Order);
    assert_eq!(field.scalar(order.clone()).unwrap(), order);
    assert_eq!(field.scalar(Scalar::from(10_i32)).unwrap(), order);
    assert_eq!(field.scalar(Scalar::from(10_i64)).unwrap(), order);
    assert_eq!(field.scalar(Scalar::from("ORDR")).unwrap(), order);
    assert_eq!(field.scalar(Scalar::from("order")).unwrap(), order);
    assert!(field.scalar(Scalar::from(26_i32)).is_err());
    assert!(field.scalar(Scalar::from("not a kind")).is_err());
    assert!(field.scalar(Scalar::from(true)).is_err());
    // A member of another enum leaf is a value of another vocabulary.
    assert!(
        field
            .scalar(Scalar::State(yggdryl::State::Unknown))
            .is_err()
    );
    assert_eq!(order.as_str(), Some("ORDR"));
    assert_eq!(order.dtype().unwrap(), DataType::MarketDataKind);
    assert_eq!(order.id(), DataTypeId::MarketDataKind);
    assert_eq!(order.kind(), "marketdatakind");
    assert!(order.is_enum());
    assert!(!order.is_code());
    assert_eq!(order.enum_code(), Some(10));
    assert_eq!(order.enum_name(), Some("ORDR"));
    assert_eq!(Scalar::from(MarketDataKind::Order), order);
    assert_eq!(
        yggdryl::Value::from_scalar(&order),
        Some(&MarketDataKind::Order)
    );
    // The members order by code, and a kind is one kind apart from a state
    // of the same code.
    assert!(
        Scalar::MarketDataKind(MarketDataKind::Book)
            < Scalar::MarketDataKind(MarketDataKind::Trade)
    );
    assert_ne!(order, Scalar::from(10_i32));
    assert_ne!(order, Scalar::from("ORDR"));
}

#[test]
fn a_column_is_uint8_codes_under_the_marketdatakind_extension() {
    let field = Field::new("marketdatakind", DataType::MarketDataKind, true);
    let arrow = field.clone().into_arrow_field().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt8);
    assert_eq!(
        arrow.metadata()["ARROW:extension:name"],
        "yggdryl.marketdatakind"
    );
    assert_eq!(arrow.metadata()["ARROW:extension:metadata"], "");
    assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);

    let values = [
        Scalar::MarketDataKind(MarketDataKind::Order),
        Scalar::Null,
        Scalar::MarketDataKind(MarketDataKind::Trade),
    ];
    let serie = Serie::from_scalars(field.clone(), values.clone()).unwrap();
    assert!(matches!(serie, Serie::MarketDataKind(_)));
    let array = serie.require_arrow_array().unwrap();
    let codes = array
        .as_any()
        .downcast_ref::<arrow_array::UInt8Array>()
        .unwrap();
    assert_eq!(codes.values().as_ref(), [10, 0, 21]);
    assert!(codes.is_null(1));
    let back = Serie::from_arrow_array(Some(&field), array, ArrowCastOptions::default()).unwrap();
    for (index, value) in values.iter().enumerate() {
        assert_eq!(&back.scalar(index).unwrap(), value, "row {index}");
    }
    let one = Serie::from_scalars(
        field.clone(),
        [Scalar::MarketDataKind(MarketDataKind::Quotation)],
    )
    .unwrap()
    .into_arrow_scalar()
    .unwrap();
    assert_eq!(
        one.into_inner()
            .as_any()
            .downcast_ref::<arrow_array::UInt8Array>()
            .unwrap()
            .value(0),
        14
    );

    // The same storage under another name is not a kind, and a state's codes
    // under the kind's name are a foreign field wearing it.
    let foreign = arrow_schema::Field::new("marketdatakind", ArrowDataType::Utf8, true)
        .with_metadata(
            [
                (
                    "ARROW:extension:name".to_owned(),
                    "yggdryl.marketdatakind".to_owned(),
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
fn integers_and_text_land_as_members_and_a_stranger_is_refused_by_row() {
    let field = Field::new("marketdatakind", DataType::MarketDataKind, true);
    let landed = Serie::from_arrow_array(
        Some(&field),
        Arc::new(Int64Array::from(vec![Some(10), None, Some(21)])) as ArrayRef,
        strict(),
    )
    .unwrap();
    assert_eq!(
        landed.scalar(0).unwrap(),
        Scalar::MarketDataKind(MarketDataKind::Order)
    );
    assert_eq!(landed.scalar(1).unwrap(), Scalar::Null);
    assert_eq!(
        landed.scalar(2).unwrap(),
        Scalar::MarketDataKind(MarketDataKind::Trade)
    );

    let refused = Serie::from_arrow_array(
        Some(&field),
        Arc::new(Int32Array::from(vec![10, 26])) as ArrayRef,
        strict(),
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("row 1"), "{refused}");
    let safe = Serie::from_arrow_array(
        Some(&field),
        Arc::new(Int32Array::from(vec![10, 26])) as ArrayRef,
        ArrowCastOptions::new().with_safe(true),
    )
    .unwrap();
    assert_eq!(safe.scalar(1).unwrap(), Scalar::Null);

    let required = Field::new("marketdatakind", DataType::MarketDataKind, false);
    let landed = Serie::from_arrow_array(
        Some(&required),
        Arc::new(StringArray::from(vec!["ORDR", "quotation", "exec"])) as ArrayRef,
        strict(),
    )
    .unwrap();
    assert_eq!(
        landed.scalar(0).unwrap(),
        Scalar::MarketDataKind(MarketDataKind::Order)
    );
    assert_eq!(
        landed.scalar(1).unwrap(),
        Scalar::MarketDataKind(MarketDataKind::Quotation)
    );
    assert_eq!(
        landed.scalar(2).unwrap(),
        Scalar::MarketDataKind(MarketDataKind::Execution)
    );
    let text = landed
        .cast(
            &Field::new("marketdatakind", DataType::utf8(), false),
            strict(),
        )
        .unwrap()
        .require_arrow_array()
        .unwrap();
    let names = text.as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(
        names.iter().collect::<Vec<_>>(),
        [Some("ORDR"), Some("QUOT"), Some("EXEC")]
    );
    let codes = landed
        .cast(
            &Field::new("marketdatakind", DataType::Int32, false),
            strict(),
        )
        .unwrap()
        .require_arrow_array()
        .unwrap();
    let codes = codes.as_any().downcast_ref::<Int32Array>().unwrap();
    assert_eq!(codes.values().as_ref(), [10, 14, 8]);
    // A state's codes are not a kind's: the leaf is re-read, and `2001` names
    // no kind.
    let states = Serie::from_scalars(
        Field::new("state", DataType::State, false),
        [Scalar::State(yggdryl::State::New)],
    )
    .unwrap();
    assert!(states.cast(&required, strict()).is_err());
}

#[test]
fn a_kind_crosses_the_value_stream_the_digest_and_the_structured_codecs() {
    for member in [
        MarketDataKind::Unknown,
        MarketDataKind::Order,
        MarketDataKind::Trade,
    ] {
        let value = Scalar::MarketDataKind(member);
        let bytes = value.into_value_bytes();
        assert_eq!(bytes[1], DataTypeId::MarketDataKind.as_u8());
        // The canonical four bytes, whatever width a column stores.
        assert_eq!(bytes[2..], i32::from(member.code()).to_le_bytes());
        assert_eq!(Scalar::decode_value_bytes(&bytes).unwrap(), value);
        // A digest reads the code under the leaf's tag: a member's name is
        // free to change, and a state of the same code is another value.
        assert_ne!(
            value.stable_hash(),
            Scalar::MarketDataKind(MarketDataKind::Book).stable_hash(),
            "{member}"
        );
        assert_ne!(
            value.digest(DigestAlgorithm::Xxh3),
            Scalar::State(yggdryl::State::from_code(u16::from(member.code())).unwrap_or_default())
                .digest(DigestAlgorithm::Xxh3),
            "{member}"
        );
        assert_ne!(
            value.digest(DigestAlgorithm::Xxh3),
            Scalar::from(member.code()).digest(DigestAlgorithm::Xxh3),
            "{member}"
        );
    }
    // The leaf's own serde: the name out, a code or a spelling in.
    let json: serde_json::Value = serde_json::to_value(MarketDataKind::Order).unwrap();
    assert_eq!(json, serde_json::json!("ORDR"));
    let read: MarketDataKind = serde_json::from_value(serde_json::json!(10)).unwrap();
    assert_eq!(read, MarketDataKind::Order);
    let read: MarketDataKind = serde_json::from_value(serde_json::json!("order")).unwrap();
    assert_eq!(read, MarketDataKind::Order);
    assert!(serde_json::from_value::<MarketDataKind>(serde_json::json!("ORDERS")).is_err());
    // The scalar's structural wire names the leaf.
    let value = Scalar::MarketDataKind(MarketDataKind::Order);
    let wire = serde_json::to_string(&value).unwrap();
    assert_eq!(wire, r#"{"type":"marketdatakind","value":"ORDR"}"#);
    assert_eq!(serde_json::from_str::<Scalar>(&wire).unwrap(), value);
    // Every structured codec writes the name.
    let row = Scalar::from_struct([("kind", value.clone())]).unwrap();
    assert_eq!(
        yggdryl::into_json_scalar(&row).unwrap(),
        r#"{"kind":"ORDR"}"#
    );
    assert_eq!(
        yggdryl::into_toml_scalar(&row).unwrap(),
        "\"kind\" = \"ORDR\"\n"
    );
    assert!(yggdryl::into_yaml_scalar(&row).unwrap().contains("ORDR"));
    assert_eq!(
        yggdryl::into_xml_scalar(&Scalar::from_struct([("row", row)]).unwrap()).unwrap(),
        "<row><kind>ORDR</kind></row>"
    );
    let variant = value.into_variant().unwrap();
    assert_eq!(variant.scalar().unwrap(), Scalar::from("ORDR"));
    assert_eq!(
        yggdryl::media::partition::partition_text(&value).unwrap(),
        "ORDR"
    );
}

#[test]
fn a_kind_filters_and_casts_by_its_member_in_an_expression() {
    use yggdryl::expression::{Expression, Filter};

    let root = DataType::from(
        yggdryl::StructType::from_fields([DataType::MarketDataKind.nullable_field("kind")])
            .unwrap(),
    )
    .required_field("row");
    let members = [
        MarketDataKind::Order,
        MarketDataKind::Quotation,
        MarketDataKind::Execution,
        MarketDataKind::Trade,
    ];
    let column = Serie::from_scalars(
        root.fields()[0].clone(),
        members
            .iter()
            .map(|member| Scalar::MarketDataKind(*member))
            .collect::<Vec<_>>(),
    )
    .unwrap()
    .require_arrow_array()
    .unwrap();
    let batch =
        arrow_array::RecordBatch::try_new(root.clone().into_arrow_schema().unwrap(), vec![column])
            .unwrap();
    for (clause, kept) in [
        ("kind = 'ORDR'", 1),
        ("kind in ('ORDR', 'quotation')", 2),
        ("kind = marketdatakind 'TRAD'", 1),
        ("cast('exec' as marketdatakind) = kind", 1),
    ] {
        let filter: Filter = clause.parse().unwrap();
        assert_eq!(
            filter.apply_arrow_batch(&batch).unwrap().num_rows(),
            kept,
            "{clause}"
        );
    }
    for (clause, kept) in [
        ("where kind < 10", 1),
        ("where kind >= 14", 2),
        ("where cast(kind as int64) = 21", 1),
    ] {
        let expression: Expression = clause.parse().unwrap();
        assert_eq!(
            expression.apply_arrow_batch(&batch).unwrap().num_rows(),
            kept,
            "{clause}"
        );
    }
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::marketdatakind::stored_side;
    use yggdryl::{MarketDataKind, Side};

    /// The side a cross code and a chain are keyed by is the stated side of
    /// an order or an execution, and no side for every other kind: a quote
    /// stating `Side(54)` is still one chain under `14:0:`.
    #[test]
    fn a_stored_side_is_the_side_of_a_sided_kind_alone() {
        for kind in MarketDataKind::ALL {
            for side in [Side::Unknown, Side::Buy, Side::Sell, Side::SShort] {
                let expected = if kind.is_sided() { side } else { Side::Unknown };
                assert_eq!(stored_side(*kind, side), expected, "{kind:?} {side:?}");
            }
        }
        assert_eq!(stored_side(MarketDataKind::Order, Side::Buy), Side::Buy);
        assert_eq!(
            stored_side(MarketDataKind::Execution, Side::Sell),
            Side::Sell
        );
        assert_eq!(
            stored_side(MarketDataKind::Quotation, Side::Buy),
            Side::Unknown
        );
        assert_eq!(
            stored_side(MarketDataKind::Quotation, Side::Sell),
            Side::Unknown
        );
        assert_eq!(stored_side(MarketDataKind::Trade, Side::Buy), Side::Unknown);
        assert_eq!(stored_side(MarketDataKind::Book, Side::Sell), Side::Unknown);
    }
}
