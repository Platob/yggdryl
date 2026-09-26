//! `rust/src/xmla/rowset.rs`: the XMLA rowset - a record field as its
//! `xsd:schema`, record series as `<row>` elements, both read back, the
//! `_xHHHH_` element names and the XML Schema type table.

use yggdryl::xml::{Element, XSD_NAMESPACE, XSI_NAMESPACE};
use yggdryl::xmla::rowset::{ROOT_ELEMENT, ROW_ELEMENT};
use yggdryl::xmla::{
    EXCEPTION_NAMESPACE, ROWSET_NAMESPACE, Rowset, SQL_NAMESPACE, XsdType, decode_name, encode_name,
};
use yggdryl::{DataType, Field, Scalar, Serie, StructType, TimeUnit, Timezone, UnionMode, Uuid};

// Fixtures.

/// A record field named `row` over `columns`.
fn record(columns: impl IntoIterator<Item = Field>) -> Field {
    DataType::from(StructType::from_fields(columns).expect("unique columns")).required_field("row")
}

/// A struct datatype over `children`.
fn structure(children: impl IntoIterator<Item = Field>) -> DataType {
    DataType::from(StructType::from_fields(children).expect("unique children"))
}

/// One row under `field`, from `(name, value)` pairs.
fn row<const N: usize>(entries: [(&str, Scalar); N]) -> Scalar {
    Scalar::from_struct(entries).expect("a named row")
}

/// The rows under `field`, laid out as one record column.
fn rows(field: &Field, rows: impl IntoIterator<Item = Scalar>) -> Serie {
    Serie::from_scalars(field.clone(), rows).unwrap_or_else(|error| panic!("rows: {error}"))
}

/// The whole rowset document `rowset` writes for `batches`.
fn root_text(rowset: &Rowset, batches: Vec<Serie>, schema: bool, data: bool) -> String {
    let mut document = Vec::new();
    rowset
        .write_root(&mut document, batches.into_iter().map(Ok), schema, data)
        .unwrap_or_else(|error| panic!("the rowset is written: {error}"));
    String::from_utf8(document).expect("UTF-8")
}

/// The `xsd:schema` `rowset` writes.
fn schema_text(rowset: &Rowset) -> String {
    let mut document = Vec::new();
    rowset
        .write_schema(&mut document)
        .unwrap_or_else(|error| panic!("the schema is written: {error}"));
    String::from_utf8(document).expect("UTF-8")
}

/// The `<row>` elements `rowset` writes for `batch`.
fn rows_text(rowset: &Rowset, batch: &Serie) -> String {
    let mut document = Vec::new();
    rowset
        .write_rows(&mut document, batch)
        .unwrap_or_else(|error| panic!("the rows are written: {error}"));
    String::from_utf8(document).expect("UTF-8")
}

/// The refusal `write_rows` answers for `batch`.
fn rows_refusal(rowset: &Rowset, batch: &Serie) -> String {
    let mut document = Vec::new();
    rowset
        .write_rows(&mut document, batch)
        .expect_err("a refusal")
        .to_string()
}

/// Read a literal `xsd:schema` document as a rowset.
fn from_schema(document: &str) -> yggdryl::Result<Rowset> {
    let value = yggdryl::from_xml_scalar(document)?;
    let schema = Element::root(&value)?;
    Rowset::from_schema(&schema)
}

/// Read a literal `root` document, with an optional declared field.
fn read_root(document: &str, field: Option<&Field>) -> yggdryl::Result<(Rowset, Serie)> {
    let value = yggdryl::from_xml_scalar(document)?;
    let root = Element::root(&value)?;
    Rowset::read_root(&root, field)
}

/// Read the literal rows of `document` under `rowset`.
fn read_rows(rowset: &Rowset, document: &str) -> yggdryl::Result<Serie> {
    let value = yggdryl::from_xml_scalar(document)?;
    let root = Element::root(&value)?;
    rowset.read_rows(&root)
}

/// A `root` document declaring the rowset namespaces, its schema's `row`
/// type holding `declarations`, then `rows`.
fn document(declarations: &str, rows: &str) -> String {
    format!(
        "<root xmlns=\"{ROWSET_NAMESPACE}\" xmlns:xsi=\"{XSI_NAMESPACE}\" \
         xmlns:xsd=\"{XSD_NAMESPACE}\">\
         <xsd:schema targetNamespace=\"{ROWSET_NAMESPACE}\" xmlns:sql=\"{SQL_NAMESPACE}\" \
         elementFormDefault=\"qualified\">\
         <xsd:complexType name=\"row\"><xsd:sequence>{declarations}</xsd:sequence>\
         </xsd:complexType></xsd:schema>{rows}</root>"
    )
}

/// A `root` document with no schema, holding `rows`.
fn schemaless(rows: &str) -> String {
    format!("<root xmlns=\"{ROWSET_NAMESPACE}\" xmlns:xsi=\"{XSI_NAMESPACE}\">{rows}</root>")
}

/// A `schema` document in the XML Schema namespace under the `xsd` prefix,
/// its `row` type holding `declarations`.
fn schema_document(declarations: &str) -> String {
    format!(
        "<xsd:schema xmlns:xsd=\"{XSD_NAMESPACE}\" xmlns:sql=\"{SQL_NAMESPACE}\">\
         <xsd:complexType name=\"row\"><xsd:sequence>{declarations}</xsd:sequence>\
         </xsd:complexType></xsd:schema>"
    )
}

fn microseconds(text: &str) -> i64 {
    // 2024-01-01T00:00:00Z is 19_723 days after the epoch.
    let day = 19_723_i64 * 86_400_000_000;
    let (hours, rest) = text.split_once(':').expect("hh:mm:ss");
    let (minutes, seconds) = rest.split_once(':').expect("mm:ss");
    day + (hours.parse::<i64>().unwrap() * 3_600
        + minutes.parse::<i64>().unwrap() * 60
        + seconds.parse::<i64>().unwrap())
        * 1_000_000
}

fn naive_at(clock: &str) -> Scalar {
    Scalar::datetime64(microseconds(clock), TimeUnit::Microsecond, Timezone::NAIVE)
        .expect("a naive datetime")
}

fn utc_at(clock: &str) -> Scalar {
    Scalar::datetime64(microseconds(clock), TimeUnit::Microsecond, Timezone::UTC)
        .expect("a UTC instant")
}

fn naive_datetime() -> DataType {
    DataType::datetime64(TimeUnit::Microsecond, Timezone::NAIVE).expect("a naive datetime")
}

fn utc_datetime() -> DataType {
    DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC).expect("a UTC datetime")
}

const ID: u128 = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;
const ID_TEXT: &str = "01234567-89ab-cdef-0123-456789abcdef";

// The two element names.

#[test]
fn the_row_and_root_elements_are_the_names_xmla_gives_them() {
    assert_eq!(ROW_ELEMENT, "row");
    assert_eq!(ROOT_ELEMENT, "root");
}

// The XML Schema type table.

#[test]
fn every_xsd_type_spells_its_qualified_name_under_the_xsd_prefix() {
    let expected = [
        (XsdType::String, "xsd:string", "string"),
        (XsdType::Boolean, "xsd:boolean", "boolean"),
        (XsdType::Byte, "xsd:byte", "byte"),
        (XsdType::UnsignedByte, "xsd:unsignedByte", "unsignedByte"),
        (XsdType::Short, "xsd:short", "short"),
        (XsdType::UnsignedShort, "xsd:unsignedShort", "unsignedShort"),
        (XsdType::Int, "xsd:int", "int"),
        (XsdType::UnsignedInt, "xsd:unsignedInt", "unsignedInt"),
        (XsdType::Long, "xsd:long", "long"),
        (XsdType::UnsignedLong, "xsd:unsignedLong", "unsignedLong"),
        (XsdType::Integer, "xsd:integer", "integer"),
        (XsdType::Float, "xsd:float", "float"),
        (XsdType::Double, "xsd:double", "double"),
        (XsdType::Decimal, "xsd:decimal", "decimal"),
        (XsdType::Date, "xsd:date", "date"),
        (XsdType::Time, "xsd:time", "time"),
        (XsdType::DateTime, "xsd:dateTime", "dateTime"),
        (XsdType::Duration, "xsd:duration", "duration"),
        (XsdType::Base64Binary, "xsd:base64Binary", "base64Binary"),
        (XsdType::AnyUri, "xsd:anyURI", "anyURI"),
        (XsdType::Uuid, "uuid", "uuid"),
    ];
    assert_eq!(
        XsdType::ALL,
        expected.map(|(held, _, _)| held).as_slice(),
        "ALL is every type in declaration order"
    );
    for (held, qualified, local) in expected {
        assert_eq!(held.as_str(), qualified);
        assert_eq!(held.local_name(), local);
        assert_eq!(held.to_string(), qualified, "Display is the qualified name");
    }
}

#[test]
fn each_integer_width_declares_the_xml_schema_type_of_that_width() {
    for (dtype, xsd) in [
        (DataType::Int8, XsdType::Byte),
        (DataType::UInt8, XsdType::UnsignedByte),
        (DataType::Int16, XsdType::Short),
        (DataType::UInt16, XsdType::UnsignedShort),
        (DataType::Int32, XsdType::Int),
        (DataType::UInt32, XsdType::UnsignedInt),
        (DataType::Int64, XsdType::Long),
        (DataType::UInt64, XsdType::UnsignedLong),
    ] {
        assert_eq!(XsdType::of(&dtype), Some(xsd), "{dtype}");
    }
}

#[test]
fn each_other_leaf_declares_the_xml_schema_type_of_its_family() {
    let named = |text: &str| {
        text.parse::<DataType>()
            .unwrap_or_else(|error| panic!("{text}: {error}"))
    };
    for (dtype, xsd) in [
        (DataType::Null, XsdType::String),
        (DataType::Boolean, XsdType::Boolean),
        (DataType::Float16, XsdType::Float),
        (DataType::Float32, XsdType::Float),
        (DataType::Float64, XsdType::Double),
        (DataType::decimal32(9, 2).unwrap(), XsdType::Decimal),
        (DataType::decimal64(18, 4).unwrap(), XsdType::Decimal),
        (DataType::decimal128(38, 0).unwrap(), XsdType::Decimal),
        (DataType::decimal256(76, 20).unwrap(), XsdType::Decimal),
        (DataType::Date32, XsdType::Date),
        (DataType::Date64, XsdType::Date),
        (DataType::Time32(TimeUnit::Second), XsdType::Time),
        (DataType::Time32(TimeUnit::Millisecond), XsdType::Time),
        (DataType::Time64(TimeUnit::Microsecond), XsdType::Time),
        (DataType::Time64(TimeUnit::Nanosecond), XsdType::Time),
        (naive_datetime(), XsdType::DateTime),
        (utc_datetime(), XsdType::DateTime),
        (
            DataType::datetime64(
                TimeUnit::Second,
                Timezone::from_str("Europe/Paris").unwrap(),
            )
            .unwrap(),
            XsdType::DateTime,
        ),
        (DataType::Duration32(TimeUnit::Second), XsdType::Duration),
        (
            DataType::Duration64(TimeUnit::Nanosecond),
            XsdType::Duration,
        ),
        (DataType::Uuid, XsdType::Uuid),
        (DataType::Url, XsdType::AnyUri),
        (DataType::Urn, XsdType::String),
        (DataType::geometry(None).unwrap(), XsdType::Base64Binary),
        (
            DataType::geography(None, None).unwrap(),
            XsdType::Base64Binary,
        ),
        (DataType::binary(), XsdType::Base64Binary),
        (DataType::large_binary(), XsdType::Base64Binary),
        (DataType::binary_view(), XsdType::Base64Binary),
        (DataType::large_binary_view(), XsdType::Base64Binary),
        (DataType::fixed_binary(4).unwrap(), XsdType::Base64Binary),
        (DataType::sized_binary(8).unwrap(), XsdType::Base64Binary),
        (DataType::utf8(), XsdType::String),
        (DataType::large_utf8(), XsdType::String),
        (DataType::utf8_view(), XsdType::String),
        (DataType::fixed_utf8(4).unwrap(), XsdType::String),
        (DataType::ascii(), XsdType::String),
        (DataType::cp1252(), XsdType::String),
        (DataType::sized_cp1252(8).unwrap(), XsdType::String),
        (DataType::Ccy, XsdType::String),
        (DataType::Version, XsdType::String),
        (DataType::Timezone, XsdType::String),
        (DataType::Variant, XsdType::String),
        (named("interval(day_time)"), XsdType::String),
    ] {
        assert_eq!(XsdType::of(&dtype), Some(xsd), "{dtype}");
    }
}

#[test]
fn a_dictionary_or_a_run_end_column_declares_the_type_of_its_values() {
    assert_eq!(
        XsdType::of(&DataType::dictionary(DataType::Int32, DataType::utf8()).unwrap()),
        Some(XsdType::String)
    );
    assert_eq!(
        XsdType::of(&DataType::dictionary(DataType::UInt16, DataType::Int64).unwrap()),
        Some(XsdType::Long)
    );
    let encoded = DataType::run_end_encoded(
        Field::new("run_ends", DataType::Int32, false),
        Field::new("values", DataType::Float64, true),
    )
    .unwrap();
    assert_eq!(XsdType::of(&encoded), Some(XsdType::Double));
}

#[test]
fn a_nested_datatype_declares_no_simple_type() {
    let item = || Field::new("item", DataType::Int32, true);
    for dtype in [
        structure([DataType::Int32.required_field("a")]),
        DataType::serie(item()),
        DataType::serie_view(item()),
        DataType::fixed_size_serie(item(), 2).unwrap(),
        DataType::large_serie(item()),
        DataType::large_serie_view(item()),
        DataType::map_of(DataType::utf8(), DataType::Int32, false).unwrap(),
        DataType::map_of(DataType::utf8(), DataType::Int32, true).unwrap(),
        DataType::union(
            [(0, Field::new("number", DataType::Int64, false))],
            UnionMode::Dense,
        )
        .unwrap(),
    ] {
        assert_eq!(XsdType::of(&dtype), None, "{dtype}");
    }
}

#[test]
fn each_xsd_type_reads_back_as_the_widest_datatype_it_proves() {
    for (xsd, dtype) in [
        (XsdType::String, DataType::utf8()),
        (XsdType::Boolean, DataType::Boolean),
        (XsdType::Byte, DataType::Int8),
        (XsdType::UnsignedByte, DataType::UInt8),
        (XsdType::Short, DataType::Int16),
        (XsdType::UnsignedShort, DataType::UInt16),
        (XsdType::Int, DataType::Int32),
        (XsdType::UnsignedInt, DataType::UInt32),
        (XsdType::Long, DataType::Int64),
        (XsdType::UnsignedLong, DataType::UInt64),
        (XsdType::Integer, DataType::decimal128(38, 0).unwrap()),
        (XsdType::Float, DataType::Float32),
        (XsdType::Double, DataType::Float64),
        // The schema states no scale, so exact digits read as their text.
        (XsdType::Decimal, DataType::utf8()),
        (XsdType::Date, DataType::Date32),
        (XsdType::Time, DataType::Time64(TimeUnit::Microsecond)),
        (XsdType::DateTime, naive_datetime()),
        (
            XsdType::Duration,
            DataType::Duration64(TimeUnit::Microsecond),
        ),
        (XsdType::Base64Binary, DataType::binary()),
        (XsdType::AnyUri, DataType::Url),
        (XsdType::Uuid, DataType::Uuid),
    ] {
        assert_eq!(xsd.datatype(), dtype, "{xsd}");
    }
}

#[test]
fn a_datatype_s_xsd_type_reads_back_as_a_datatype_declaring_the_same_type() {
    for held in XsdType::ALL {
        assert_eq!(
            XsdType::of(&held.datatype()),
            Some(match held {
                // A 38-digit integer is a decimal, and `xsd:decimal` is text.
                XsdType::Integer => XsdType::Decimal,
                XsdType::Decimal => XsdType::String,
                other => *other,
            }),
            "{held}"
        );
    }
}

#[test]
fn a_type_name_resolves_in_the_xml_schema_namespace_and_uuid_in_none() {
    assert_eq!(
        XsdType::from_qualified(Some(XSD_NAMESPACE), "int"),
        Some(XsdType::Int)
    );
    assert_eq!(
        XsdType::from_qualified(Some(XSD_NAMESPACE), "dateTime"),
        Some(XsdType::DateTime)
    );
    assert_eq!(XsdType::from_qualified(None, "uuid"), Some(XsdType::Uuid));

    assert_eq!(
        XsdType::from_qualified(None, "int"),
        None,
        "an XML Schema type in no namespace is not one"
    );
    assert_eq!(
        XsdType::from_qualified(Some(XSD_NAMESPACE), "uuid"),
        None,
        "the rowset's own uuid is not an XML Schema type"
    );
    assert_eq!(
        XsdType::from_qualified(Some(ROWSET_NAMESPACE), "int"),
        None,
        "a type in another namespace is another type"
    );
    assert_eq!(
        XsdType::from_qualified(Some(ROWSET_NAMESPACE), "uuid"),
        None
    );
    assert_eq!(
        XsdType::from_qualified(Some(XSD_NAMESPACE), "gYear"),
        None,
        "a type the table does not have"
    );
    assert_eq!(
        XsdType::from_qualified(Some(XSD_NAMESPACE), "Int"),
        None,
        "local names are case sensitive"
    );
    assert_eq!(
        XsdType::from_qualified(Some(XSD_NAMESPACE), "xsd:int"),
        None,
        "the local name carries no prefix"
    );
    assert_eq!(XsdType::from_qualified(Some(XSD_NAMESPACE), ""), None);
}

#[test]
fn every_xsd_type_resolves_back_from_its_qualified_name() {
    for held in XsdType::ALL {
        let namespace = (*held != XsdType::Uuid).then_some(XSD_NAMESPACE);
        assert_eq!(
            XsdType::from_qualified(namespace, held.local_name()),
            Some(*held),
            "{held}"
        );
    }
}

// Element names.

#[test]
fn a_legal_xml_name_encodes_unchanged() {
    for name in [
        "Symbol",
        "TABLE_NAME",
        "_private",
        "a-b.c",
        "price2",
        "a_",
        "_",
        "a_X",
        "café",
        "日本語",
        "\u{1F600}",
        "x\u{B7}y",
    ] {
        assert_eq!(encode_name(name), name, "{name:?}");
        assert_eq!(decode_name(name), name, "{name:?}");
    }
}

#[test]
fn a_character_no_xml_name_may_hold_encodes_as_its_utf16_escape() {
    for (name, encoded) in [
        ("Order Id", "Order_x0020_Id"),
        ("2024", "_x0032_024"),
        ("-a", "_x002D_a"),
        (".a", "_x002E_a"),
        ("a:b", "a_x003A_b"),
        (":a", "_x003A_a"),
        ("a/b", "a_x002F_b"),
        ("a<b", "a_x003C_b"),
        ("a&b", "a_x0026_b"),
        ("a\"b", "a_x0022_b"),
        ("a(b)", "a_x0028_b_x0029_"),
        ("a\tb", "a_x0009_b"),
        ("\u{B7}a", "_x00B7_a"),
        ("a\u{D7}b", "a_x00D7_b"),
        ("\u{300}", "_x0300_"),
        // Past U+EFFFF no name holds a character: a surrogate pair of escapes.
        ("a\u{F0000}", "a_xDB80__xDC00_"),
    ] {
        assert_eq!(encode_name(name), encoded, "{name:?}");
        assert_eq!(decode_name(encoded), name, "{encoded:?}");
    }
}

#[test]
fn a_literal_underscore_x_is_escaped_so_the_name_reads_back() {
    for (name, encoded) in [
        ("a_xb", "a_x005F_xb"),
        ("_x0020_ literal", "_x005F_x0020__x0020_literal"),
        ("Order_x0020_Id", "Order_x005F_x0020_Id"),
        ("__x", "__x005F_x"),
        ("_x", "_x005F_x"),
    ] {
        assert_eq!(encode_name(name), encoded, "{name:?}");
        assert_eq!(decode_name(encoded), name, "{encoded:?}");
    }
}

#[test]
fn every_encoded_name_is_an_xml_name_without_a_prefix_and_decodes_back() {
    for name in [
        "Order Id",
        "2024",
        "a:b:c",
        "a/b\\c",
        "  ",
        "_x0020_",
        "_xZZZZ_",
        "ünïcödé naming",
        "\u{F0000}\u{10FFFF}",
        "tab\there",
        "semi;colon",
        "#hash",
        "@at",
        "Order_x0020_Id",
    ] {
        let encoded = encode_name(name);
        assert!(!encoded.contains(':'), "{name:?} -> {encoded}");
        let parsed = yggdryl::from_xml_scalar(format!("<{encoded}/>"))
            .unwrap_or_else(|error| panic!("{name:?} -> {encoded} is not an XML name: {error}"));
        let root = Element::root(&parsed).expect("one element");
        assert_eq!(root.name(), encoded.as_str());
        assert_eq!(decode_name(&encoded), name, "{encoded}");
    }
}

#[test]
fn encoding_leaves_a_legal_name_alone_and_layers_over_an_escaped_one() {
    let legal = encode_name("Symbol");
    assert_eq!(encode_name(&legal), legal, "a legal name is a fixed point");

    let once = encode_name("Order Id");
    let twice = encode_name(&once);
    assert_eq!(twice, "Order_x005F_x0020_Id", "each `_x` is escaped again");
    assert_eq!(decode_name(&twice), once);
    assert_eq!(decode_name(&decode_name(&twice)), "Order Id");
}

#[test]
fn the_empty_name_encodes_as_the_escape_of_nothing() {
    assert_eq!(encode_name(""), "_x0000_");
    assert_eq!(decode_name("_x0000_"), "");
    assert_eq!(decode_name(""), "");
    assert_eq!(
        decode_name("a_x0000_"),
        "a\u{0}",
        "only the whole name escaping nothing is the empty name"
    );
    assert_eq!(decode_name("_x0000_a"), "\u{0}a");
}

#[test]
fn a_column_named_with_u0000_is_refused_because_it_would_spell_the_empty_name() {
    assert_eq!(encode_name("\u{0}"), encode_name(""));
    for column in [
        DataType::utf8().nullable_field("\u{0}"),
        DataType::utf8().nullable_field("a\u{0}b"),
        structure([DataType::utf8().nullable_field("\u{0}")]).nullable_field("leg"),
    ] {
        let message = Rowset::new(record([DataType::utf8().nullable_field("x"), column]))
            .expect_err("no XML document holds U+0000")
            .to_string();
        assert!(
            message.contains("carries U+0000 in its name, which no XML name spells"),
            "{message}"
        );
    }
}

#[test]
fn a_column_with_the_empty_name_reads_back_under_it() {
    let field = record([
        DataType::utf8().nullable_field(""),
        DataType::Int32.required_field("n"),
    ]);
    let rowset = Rowset::new(field.clone()).expect("the empty name is a column name");
    assert_eq!(rowset.element_names(), ["_x0000_", "n"]);
    let batch = rows(
        &field,
        [row([("", Scalar::from("v")), ("n", Scalar::from(1))])],
    );
    let text = root_text(&rowset, vec![batch.clone()], true, true);
    assert!(
        text.contains("<xsd:element sql:field=\"\" name=\"_x0000_\""),
        "{text}"
    );
    assert!(
        text.contains("<row><_x0000_>v</_x0000_><n>1</n></row>"),
        "{text}"
    );
    let (read, back) = read_root(&text, None).expect("the rowset reads back");
    assert_eq!(read.field(), &field);
    assert_eq!(back, batch);
}

#[test]
fn an_escape_that_is_not_one_stands_as_written() {
    for text in [
        "_x", "_x12", "_x0020", "_x0020x", "_xGGGG_", "_x00G1_", "_x-001_", "_X0020_", "a_x12_b",
        "_x_",
    ] {
        assert_eq!(decode_name(text), text, "{text:?}");
    }
    assert_eq!(
        decode_name("__x0020_"),
        "_ ",
        "the escape opens at the second underscore"
    );
    assert_eq!(
        decode_name("a_x002f_b"),
        "a/b",
        "hex digits read in either case"
    );
}

#[test]
fn a_surrogate_pair_of_escapes_decodes_to_one_character() {
    assert_eq!(decode_name("_xD83D__xDE00_"), "\u{1F600}");
    assert_eq!(decode_name("a_xDB80__xDC00_b"), "a\u{F0000}b");
    assert_eq!(decode_name("_xD83D_"), "\u{FFFD}", "a high half at the end");
    assert_eq!(
        decode_name("_xD83D_a"),
        "\u{FFFD}a",
        "a high half before text"
    );
    assert_eq!(decode_name("_xDE00_"), "\u{FFFD}", "a low half alone");
}

// Rowset::new.

#[test]
fn a_rowset_refuses_a_field_that_is_not_a_struct() {
    let error = Rowset::new(DataType::Int32.required_field("row")).expect_err("a refusal");
    let message = error.to_string();
    assert!(
        message.contains("expected a struct field for the rows of a rowset, got int32"),
        "{message}"
    );
    assert!(message.contains("$.rowset"), "{message}");

    let error = Rowset::new(
        DataType::serie(Field::new("item", DataType::Int32, true)).required_field("row"),
    )
    .expect_err("a sequence is not a record");
    assert!(
        error.to_string().contains("expected a struct field"),
        "{error}"
    );
}

#[test]
fn a_nullable_record_field_is_read_as_the_rows_it_holds() {
    let columns = [DataType::Int32.required_field("a")];
    let rowset = Rowset::new(
        DataType::from(StructType::from_fields(columns.clone()).unwrap()).nullable_field("row"),
    )
    .expect("a nullable record is a rowset");
    assert!(
        !rowset.field().is_nullable(),
        "the rows themselves are never null"
    );
    assert_eq!(rowset.field(), &record(columns));
}

#[test]
fn a_map_or_a_union_column_is_refused_naming_its_path() {
    let map = DataType::map_of(DataType::utf8(), DataType::Int32, false).unwrap();
    let union = DataType::union(
        [(0, Field::new("number", DataType::Int64, false))],
        UnionMode::Sparse,
    )
    .unwrap();
    for (column, expected) in [
        (
            map.clone().nullable_field("book"),
            "column `book` is a map, which a rowset has no element to spell",
        ),
        (
            DataType::map_of(DataType::utf8(), DataType::Int32, true)
                .unwrap()
                .nullable_field("sorted"),
            "column `sorted` is a map",
        ),
        (
            union.clone().nullable_field("either"),
            "column `either` is a union, which a rowset has no element to spell",
        ),
        (
            structure([
                DataType::Int32.required_field("qty"),
                map.clone().nullable_field("book"),
            ])
            .nullable_field("leg"),
            "column `leg.book` is a map",
        ),
        (
            DataType::serie(Field::new("item", union, true)).nullable_field("legs"),
            "column `legs[]` is a union",
        ),
        (
            DataType::serie(Field::new(
                "item",
                DataType::serie(Field::new("item", DataType::Int32, true)),
                true,
            ))
            .nullable_field("matrix"),
            "column `matrix` is a sequence of sequences, which has no element to repeat",
        ),
        (
            DataType::large_serie(Field::new(
                "item",
                DataType::fixed_size_serie(Field::new("item", DataType::Int32, true), 2).unwrap(),
                true,
            ))
            .nullable_field("grid"),
            "column `grid` is a sequence of sequences",
        ),
    ] {
        let error = Rowset::new(record([DataType::Int32.required_field("id"), column]))
            .expect_err("a refusal");
        let message = error.to_string();
        assert!(message.contains(expected), "{message}");
        assert!(message.contains("$.rowset"), "{message}");
    }
}

#[test]
fn a_rowset_names_each_column_s_element_in_column_order() {
    let field = record([
        DataType::Int32.required_field("Order Id"),
        DataType::utf8().nullable_field("Symbol"),
        DataType::utf8().nullable_field("2nd"),
        DataType::utf8().nullable_field("a_xb"),
    ]);
    let rowset = Rowset::new(field.clone()).expect("a rowset");
    assert_eq!(
        rowset.element_names(),
        ["Order_x0020_Id", "Symbol", "_x0032_nd", "a_x005F_xb"]
    );
    assert_eq!(rowset.field(), &field);
    assert_eq!(rowset.shared_field().as_ref(), &field);
    assert!(std::ptr::eq(rowset.shared_field().as_ref(), rowset.field()));
}

#[test]
fn names_that_differ_only_in_what_an_escape_spells_keep_distinct_elements() {
    let rowset = Rowset::new(record([
        DataType::utf8().nullable_field("a b"),
        DataType::utf8().nullable_field("a_x0020_b"),
        DataType::utf8().nullable_field("a_x005F_x0020_b"),
    ]))
    .expect("a rowset");
    assert_eq!(
        rowset.element_names(),
        [
            "a_x0020_b",
            "a_x005F_x0020_b",
            "a_x005F_x005F_x005F_x0020_b"
        ]
    );
}

#[test]
fn an_empty_record_is_a_rowset_of_no_columns() {
    let field = record([]);
    let rowset = Rowset::new(field.clone()).expect("a rowset of nothing");
    assert!(rowset.element_names().is_empty());
    let batch = rows(
        &field,
        [Scalar::from_sequence([]), Scalar::from_sequence([])],
    );
    assert_eq!(rows_text(&rowset, &batch), "<row></row><row></row>");
}

// Writing.

#[test]
fn the_root_declares_the_rowset_the_instance_the_schema_and_the_exception_namespaces() {
    let rowset = Rowset::new(record([DataType::Int32.required_field("a")])).unwrap();
    let mut document = Vec::new();
    rowset
        .write_root(
            &mut document,
            std::iter::from_fn(|| -> Option<yggdryl::arrow::Result<Serie>> {
                panic!("no batch is pulled when no data is written")
            }),
            false,
            false,
        )
        .expect("an empty root is written");
    assert_eq!(
        String::from_utf8(document).unwrap(),
        format!(
            "<root xmlns=\"{ROWSET_NAMESPACE}\" xmlns:xsi=\"{XSI_NAMESPACE}\" \
             xmlns:xsd=\"{XSD_NAMESPACE}\" xmlns:EX=\"{EXCEPTION_NAMESPACE}\"></root>"
        )
    );
    assert_eq!(
        ROWSET_NAMESPACE,
        "urn:schemas-microsoft-com:xml-analysis:rowset"
    );
    assert_eq!(SQL_NAMESPACE, "urn:schemas-microsoft-com:xml-sql");
}

#[test]
fn the_schema_declares_every_column_under_the_row_type() {
    let rowset = Rowset::new(record([
        DataType::Int32.required_field("Order Id"),
        DataType::utf8().nullable_field("Symbol"),
        DataType::serie(Field::new("item", DataType::utf8(), false)).required_field("tags"),
        structure([
            DataType::Float64.required_field("price"),
            DataType::utf8().nullable_field("venue"),
        ])
        .nullable_field("leg"),
        DataType::Uuid.nullable_field("id"),
        DataType::serie(Field::new(
            "item",
            structure([DataType::Int64.required_field("qty")]),
            true,
        ))
        .required_field("fills"),
        DataType::utf8().required_field("a<b"),
        DataType::Null.required_field("nothing"),
    ]))
    .expect("a rowset");
    assert_eq!(
        schema_text(&rowset),
        format!(
            "<xsd:schema targetNamespace=\"{ROWSET_NAMESPACE}\" xmlns:sql=\"{SQL_NAMESPACE}\" \
             elementFormDefault=\"qualified\">\
             <xsd:element name=\"root\"><xsd:complexType>\
             <xsd:sequence minOccurs=\"0\" maxOccurs=\"unbounded\">\
             <xsd:element name=\"row\" type=\"row\"/>\
             </xsd:sequence></xsd:complexType></xsd:element>\
             <xsd:simpleType name=\"uuid\"><xsd:restriction base=\"xsd:string\">\
             <xsd:pattern value=\"[0-9a-fA-F]{{8}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{4}}-[0-9a-fA-F]{{12}}\"/>\
             </xsd:restriction></xsd:simpleType>\
             <xsd:complexType name=\"row\"><xsd:sequence>\
             <xsd:element sql:field=\"Order Id\" name=\"Order_x0020_Id\" type=\"xsd:int\"/>\
             <xsd:element sql:field=\"Symbol\" name=\"Symbol\" type=\"xsd:string\" minOccurs=\"0\"/>\
             <xsd:element sql:field=\"tags\" name=\"tags\" type=\"xsd:string\" minOccurs=\"0\" maxOccurs=\"unbounded\"/>\
             <xsd:element sql:field=\"leg\" name=\"leg\" minOccurs=\"0\"><xsd:complexType><xsd:sequence>\
             <xsd:element sql:field=\"price\" name=\"price\" type=\"xsd:double\"/>\
             <xsd:element sql:field=\"venue\" name=\"venue\" type=\"xsd:string\" minOccurs=\"0\"/>\
             </xsd:sequence></xsd:complexType></xsd:element>\
             <xsd:element sql:field=\"id\" name=\"id\" type=\"uuid\" minOccurs=\"0\"/>\
             <xsd:element sql:field=\"fills\" name=\"fills\" minOccurs=\"0\" maxOccurs=\"unbounded\">\
             <xsd:complexType><xsd:sequence>\
             <xsd:element sql:field=\"qty\" name=\"qty\" type=\"xsd:long\"/>\
             </xsd:sequence></xsd:complexType></xsd:element>\
             <xsd:element sql:field=\"a&lt;b\" name=\"a_x003C_b\" type=\"xsd:string\"/>\
             <xsd:element sql:field=\"nothing\" name=\"nothing\" type=\"xsd:string\" minOccurs=\"0\"/>\
             </xsd:sequence></xsd:complexType></xsd:schema>"
        )
    );
}

#[test]
fn the_uuid_type_is_declared_whether_or_not_a_column_is_one() {
    let pattern = "<xsd:simpleType name=\"uuid\">";
    let without = Rowset::new(record([DataType::Int32.required_field("a")])).unwrap();
    assert_eq!(schema_text(&without).matches(pattern).count(), 1);
    let with = Rowset::new(record([
        DataType::Uuid.required_field("a"),
        DataType::Uuid.required_field("b"),
    ]))
    .unwrap();
    assert_eq!(schema_text(&with).matches(pattern).count(), 1);
}

#[test]
fn write_root_writes_the_schema_then_every_batch_s_rows_in_order() {
    let field = record([
        DataType::Int32.required_field("n"),
        DataType::utf8().nullable_field("s"),
    ]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let first = rows(
        &field,
        [
            row([("n", Scalar::from(1)), ("s", Scalar::from("a"))]),
            row([("n", Scalar::from(2)), ("s", Scalar::Null)]),
        ],
    );
    let second = rows(
        &field,
        [row([("n", Scalar::from(3)), ("s", Scalar::from("c"))])],
    );
    let empty = rows(&field, []);
    let text = root_text(
        &rowset,
        vec![first.clone(), empty, second.clone()],
        true,
        true,
    );
    let open = format!(
        "<root xmlns=\"{ROWSET_NAMESPACE}\" xmlns:xsi=\"{XSI_NAMESPACE}\" \
         xmlns:xsd=\"{XSD_NAMESPACE}\" xmlns:EX=\"{EXCEPTION_NAMESPACE}\">"
    );
    assert_eq!(
        text,
        format!(
            "{open}{}<row><n>1</n><s>a</s></row><row><n>2</n></row><row><n>3</n><s>c</s></row></root>",
            schema_text(&rowset)
        )
    );

    assert_eq!(
        root_text(&rowset, vec![first.clone(), second.clone()], false, true),
        format!(
            "{open}<row><n>1</n><s>a</s></row><row><n>2</n></row><row><n>3</n><s>c</s></row></root>"
        ),
        "rows alone"
    );
    assert_eq!(
        root_text(&rowset, vec![first, second], true, false),
        format!("{open}{}</root>", schema_text(&rowset)),
        "the schema alone"
    );
}

#[test]
fn a_failing_batch_is_refused_at_the_rows() {
    let field = record([DataType::Int32.required_field("n")]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let batches: Vec<yggdryl::arrow::Result<Serie>> = vec![
        Ok(rows(&field, [row([("n", Scalar::from(1))])])),
        Err(yggdryl::arrow::Error::Unsupported {
            kind: "stream",
            reason: "the source went away".to_owned(),
        }),
    ];
    let mut document = Vec::new();
    let error = rowset
        .write_root(&mut document, batches, true, true)
        .expect_err("the failing batch is refused");
    let message = error.to_string();
    assert!(message.contains("$.rows"), "{message}");
    assert!(message.contains("the source went away"), "{message}");
    let text = String::from_utf8(document).unwrap();
    assert!(
        text.ends_with("<row><n>1</n></row>"),
        "the batches before it are written as they are pulled: {text}"
    );
}

#[test]
fn a_failing_sink_is_refused() {
    struct Full;
    impl std::io::Write for Full {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("the sink is full"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let field = record([DataType::Int32.required_field("n")]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let batch = rows(&field, [row([("n", Scalar::from(1))])]);
    for error in [
        rowset
            .write_root(&mut Full, std::iter::once(Ok(batch.clone())), true, true)
            .expect_err("the root"),
        rowset.write_schema(&mut Full).expect_err("the schema"),
        rowset.write_rows(&mut Full, &batch).expect_err("the rows"),
    ] {
        assert!(error.to_string().contains("the sink is full"), "{error}");
    }
}

#[test]
fn a_null_cell_writes_no_element() {
    let field = record([
        DataType::Int32.nullable_field("n"),
        DataType::utf8().nullable_field("s"),
        DataType::binary().nullable_field("b"),
        DataType::Date32.nullable_field("d"),
        DataType::serie(Field::new("item", DataType::Int32, true)).nullable_field("xs"),
        structure([DataType::Int32.required_field("a")]).nullable_field("leg"),
    ]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let batch = rows(
        &field,
        [row([
            ("n", Scalar::Null),
            ("s", Scalar::Null),
            ("b", Scalar::Null),
            ("d", Scalar::Null),
            ("xs", Scalar::Null),
            ("leg", Scalar::Null),
        ])],
    );
    assert_eq!(rows_text(&rowset, &batch), "<row></row>");
}

#[test]
fn an_empty_text_cell_is_an_element_with_an_empty_body() {
    let field = record([
        DataType::utf8().nullable_field("s"),
        DataType::binary().nullable_field("b"),
    ]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let batch = rows(
        &field,
        [row([
            ("s", Scalar::from("")),
            ("b", Scalar::from(Vec::<u8>::new())),
        ])],
    );
    assert_eq!(rows_text(&rowset, &batch), "<row><s></s><b></b></row>");
}

/// The columns of every leaf family, one row of each, and the text each is
/// written as.
fn leaves() -> (Field, Scalar, &'static str) {
    let field = record([
        DataType::Boolean.required_field("flag"),
        DataType::Int8.required_field("i8"),
        DataType::UInt8.required_field("u8"),
        DataType::Int16.required_field("i16"),
        DataType::UInt16.required_field("u16"),
        DataType::Int32.required_field("i32"),
        DataType::UInt32.required_field("u32"),
        DataType::Int64.required_field("i64"),
        DataType::UInt64.required_field("u64"),
        DataType::Float32.required_field("f32"),
        DataType::Float64.required_field("f64"),
        DataType::decimal128(10, 2)
            .unwrap()
            .required_field("amount"),
        DataType::Date32.required_field("day"),
        DataType::Time64(TimeUnit::Microsecond).required_field("clock"),
        naive_datetime().required_field("wall"),
        utc_datetime().required_field("instant"),
        DataType::Duration64(TimeUnit::Microsecond).required_field("span"),
        DataType::binary().required_field("blob"),
        DataType::Uuid.required_field("id"),
        DataType::Url.required_field("link"),
        DataType::utf8().required_field("note"),
        DataType::cp1252().required_field("legacy"),
        DataType::ascii().required_field("code"),
    ]);
    let value = row([
        ("flag", Scalar::from(true)),
        ("i8", Scalar::from(-8_i8)),
        ("u8", Scalar::from(200_u8)),
        ("i16", Scalar::from(-300_i16)),
        ("u16", Scalar::from(60_000_u16)),
        ("i32", Scalar::from(-70_000_i32)),
        ("u32", Scalar::from(4_000_000_000_u32)),
        ("i64", Scalar::from(i64::MIN)),
        ("u64", Scalar::from(u64::MAX)),
        ("f32", Scalar::from(1.5_f32)),
        ("f64", Scalar::from(0.1_f64)),
        ("amount", Scalar::d128(1250, 2)),
        ("day", Scalar::date32(19_723)),
        (
            "clock",
            Scalar::time64(
                (9 * 3_600 + 30 * 60) * 1_000_000 + 500_000,
                TimeUnit::Microsecond,
                Timezone::NAIVE,
            )
            .unwrap(),
        ),
        ("wall", naive_at("09:30:00")),
        ("instant", utc_at("09:30:00")),
        (
            "span",
            Scalar::duration64(90_000_000, TimeUnit::Microsecond).unwrap(),
        ),
        ("blob", Scalar::from(vec![0_u8, 0xFF, b'h', b'i'])),
        ("id", Scalar::Uuid(Uuid::new(ID))),
        ("link", Scalar::from("https://example.com/a?b=1&c=2")),
        ("note", Scalar::from("a <b> & c")),
        ("legacy", Scalar::from("café")),
        ("code", Scalar::from("XPAR")),
    ]);
    let text = "<row><flag>true</flag><i8>-8</i8><u8>200</u8><i16>-300</i16><u16>60000</u16>\
         <i32>-70000</i32><u32>4000000000</u32><i64>-9223372036854775808</i64>\
         <u64>18446744073709551615</u64><f32>1.5</f32><f64>0.1</f64><amount>12.50</amount>\
         <day>2024-01-01</day><clock>09:30:00.500000</clock>\
         <wall>2024-01-01T09:30:00.000000</wall><instant>2024-01-01T09:30:00.000000Z</instant>\
         <span>PT90.000000S</span><blob>AP9oaQ==</blob>\
         <id>01234567-89ab-cdef-0123-456789abcdef</id>\
         <link>https://example.com/a?b=1&amp;c=2</link><note>a &lt;b&gt; &amp; c</note>\
         <legacy>café</legacy><code>XPAR</code></row>";
    (field, value, text)
}

#[test]
fn each_leaf_writes_the_spelling_its_xml_schema_type_reads() {
    let (field, value, text) = leaves();
    let rowset = Rowset::new(field.clone()).unwrap();
    let batch = rows(&field, [value]);
    assert_eq!(rows_text(&rowset, &batch), text);
}

#[test]
fn a_false_boolean_and_the_float_specials_write_their_xml_schema_spellings() {
    let field = record([
        DataType::Boolean.required_field("b"),
        DataType::Float64.required_field("x"),
    ]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let batch = rows(
        &field,
        [
            row([("b", Scalar::from(false)), ("x", Scalar::from(f64::NAN))]),
            row([
                ("b", Scalar::from(true)),
                ("x", Scalar::from(f64::INFINITY)),
            ]),
            row([
                ("b", Scalar::from(false)),
                ("x", Scalar::from(f64::NEG_INFINITY)),
            ]),
            row([("b", Scalar::from(false)), ("x", Scalar::from(-0.0_f64))]),
        ],
    );
    assert_eq!(
        rows_text(&rowset, &batch),
        "<row><b>false</b><x>NaN</x></row><row><b>true</b><x>INF</x></row>\
         <row><b>false</b><x>-INF</x></row><row><b>false</b><x>-0.0</x></row>"
    );
}

#[test]
fn a_zoned_datetime_writes_its_offset() {
    let paris = Timezone::from_str("+02:00").expect("a fixed offset");
    let field = record([DataType::datetime64(TimeUnit::Second, paris)
        .unwrap()
        .required_field("at")]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let batch = rows(
        &field,
        [row([(
            "at",
            Scalar::datetime64(
                microseconds("09:30:00") / 1_000_000,
                TimeUnit::Second,
                paris,
            )
            .unwrap(),
        )])],
    );
    assert_eq!(
        rows_text(&rowset, &batch),
        "<row><at>2024-01-01T11:30:00+02:00</at></row>"
    );
}

#[test]
fn each_temporal_width_writes_its_iso_8601_spelling_and_reads_back() {
    let field = record([
        DataType::Time32(TimeUnit::Second).required_field("t32s"),
        DataType::Time32(TimeUnit::Millisecond).required_field("t32ms"),
        DataType::Time64(TimeUnit::Nanosecond).required_field("t64ns"),
        DataType::Date64.required_field("d64"),
        DataType::datetime64(TimeUnit::Second, Timezone::NAIVE)
            .unwrap()
            .required_field("dts"),
        DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)
            .unwrap()
            .required_field("dtns"),
        DataType::Duration32(TimeUnit::Second).required_field("dur32"),
        DataType::Duration64(TimeUnit::Millisecond).required_field("dur64"),
    ]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let clock = 9 * 3_600 + 30 * 60;
    let day_ms = 19_723_i64 * 86_400_000;
    let batch = rows(
        &field,
        [row([
            (
                "t32s",
                Scalar::time32(clock, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            ),
            (
                "t32ms",
                Scalar::time32(clock * 1_000 + 250, TimeUnit::Millisecond, Timezone::NAIVE)
                    .unwrap(),
            ),
            (
                "t64ns",
                Scalar::time64(
                    i64::from(clock) * 1_000_000_000 + 1,
                    TimeUnit::Nanosecond,
                    Timezone::NAIVE,
                )
                .unwrap(),
            ),
            ("d64", Scalar::date64(day_ms)),
            (
                "dts",
                Scalar::datetime64(
                    microseconds("09:30:00") / 1_000_000,
                    TimeUnit::Second,
                    Timezone::NAIVE,
                )
                .unwrap(),
            ),
            (
                "dtns",
                Scalar::datetime64(
                    microseconds("09:30:00") * 1_000 + 1,
                    TimeUnit::Nanosecond,
                    Timezone::UTC,
                )
                .unwrap(),
            ),
            ("dur32", Scalar::duration32(90, TimeUnit::Second).unwrap()),
            (
                "dur64",
                Scalar::duration64(-1_500, TimeUnit::Millisecond).unwrap(),
            ),
        ])],
    );
    let text = rows_text(&rowset, &batch);
    assert_eq!(
        text,
        "<row><t32s>09:30:00</t32s><t32ms>09:30:00.250</t32ms>\
         <t64ns>09:30:00.000000001</t64ns><d64>2024-01-01</d64>\
         <dts>2024-01-01T09:30:00</dts><dtns>2024-01-01T09:30:00.000000001Z</dtns>\
         <dur32>PT90S</dur32><dur64>-PT1.500S</dur64></row>"
    );
    assert_eq!(read_rows(&rowset, &schemaless(&text)).unwrap(), batch);
}

#[test]
fn a_float32_writes_the_shortest_double_that_reads_back_as_it() {
    let field = record([DataType::Float32.required_field("f")]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let batch = rows(&field, [row([("f", Scalar::from(0.1_f32))])]);
    let text = rows_text(&rowset, &batch);
    assert_eq!(text, "<row><f>0.10000000149011612</f></row>");
    assert_eq!(read_rows(&rowset, &schemaless(&text)).unwrap(), batch);
}

#[test]
fn a_sequence_cell_repeats_its_element_per_item() {
    let field = record([
        DataType::serie(Field::new("item", DataType::utf8(), true)).required_field("tag s"),
        DataType::serie(Field::new("item", DataType::Date32, true)).required_field("days"),
        DataType::large_serie(Field::new("item", DataType::Int64, true)).required_field("sizes"),
    ]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let batch = rows(
        &field,
        [
            row([
                (
                    "tag s",
                    Scalar::from_sequence([Scalar::from("x"), Scalar::from("<y>")]),
                ),
                ("days", Scalar::from_sequence([Scalar::date32(19_723)])),
                (
                    "sizes",
                    Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(3_i64)]),
                ),
            ]),
            row([
                ("tag s", Scalar::from_sequence([])),
                ("days", Scalar::from_sequence([])),
                ("sizes", Scalar::from_sequence([])),
            ]),
        ],
    );
    assert_eq!(
        rows_text(&rowset, &batch),
        "<row><tag_x0020_s>x</tag_x0020_s><tag_x0020_s>&lt;y&gt;</tag_x0020_s>\
         <days>2024-01-01</days><sizes>1</sizes><sizes>3</sizes></row>\
         <row></row>"
    );
}

#[test]
fn a_null_item_of_a_sequence_keeps_its_place() {
    // The module: "a sequence column ... repeats its element per item".
    let field =
        record(
            [DataType::serie(Field::new("item", DataType::Int64, true)).required_field("sizes")],
        );
    let rowset = Rowset::new(field.clone()).unwrap();
    let batch = rows(
        &field,
        [row([(
            "sizes",
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::Null, Scalar::from(3_i64)]),
        )])],
    );
    let text = rows_text(&rowset, &batch);
    assert_eq!(
        text.matches("<sizes").count(),
        3,
        "one element per item: {text}"
    );
    assert_eq!(
        read_rows(&rowset, &schemaless(&text)).map_err(|error| error.to_string()),
        Ok(batch),
        "{text}"
    );
}

#[test]
fn a_sequence_of_one_null_item_is_refused_or_reads_back_as_itself() {
    // One `<sizes/>` cannot say whether it is a null item or a null column:
    // the XML codec refuses to write it, and a rowset may not write what it
    // then reads back as something else.
    let field =
        record(
            [DataType::serie(Field::new("item", DataType::Int64, true)).required_field("sizes")],
        );
    let rowset = Rowset::new(field.clone()).unwrap();
    let batch = rows(
        &field,
        [row([("sizes", Scalar::from_sequence([Scalar::Null]))])],
    );
    let mut document = Vec::new();
    match rowset.write_rows(&mut document, &batch) {
        Err(error) => assert!(
            error.to_string().contains("sizes"),
            "the refusal names its column: {error}"
        ),
        Ok(()) => {
            let text = String::from_utf8(document).unwrap();
            assert_eq!(
                read_rows(&rowset, &schemaless(&text)).map_err(|error| error.to_string()),
                Ok(batch),
                "{text}"
            );
        }
    }
}

#[test]
fn an_empty_sequence_in_a_required_column_reads_back_as_empty() {
    // An empty sequence repeats its element no time, and a repeated element
    // occurring no time is the empty sequence.
    let field = record([
        DataType::Int32.required_field("n"),
        DataType::serie(Field::new("item", DataType::Int64, true)).required_field("sizes"),
        DataType::serie(Field::new(
            "item",
            structure([DataType::Int64.required_field("qty")]),
            true,
        ))
        .required_field("fills"),
    ]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let batch = rows(
        &field,
        [row([
            ("n", Scalar::from(1)),
            ("sizes", Scalar::from_sequence([])),
            ("fills", Scalar::from_sequence([])),
        ])],
    );
    let text = rows_text(&rowset, &batch);
    assert_eq!(text, "<row><n>1</n></row>");
    assert_eq!(
        read_rows(&rowset, &schemaless(&text)).map_err(|error| error.to_string()),
        Ok(batch)
    );
}

#[test]
fn a_struct_cell_writes_its_children_as_elements() {
    let field = record([
        structure([
            DataType::Float64.required_field("price"),
            DataType::utf8().nullable_field("venue"),
        ])
        .nullable_field("leg"),
        DataType::serie(Field::new(
            "item",
            structure([DataType::Int64.required_field("qty")]),
            true,
        ))
        .required_field("fills"),
    ]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let batch = rows(
        &field,
        [row([
            (
                "leg",
                row([
                    ("price", Scalar::from(1.5)),
                    ("venue", Scalar::from("XPAR")),
                ]),
            ),
            (
                "fills",
                Scalar::from_sequence([
                    row([("qty", Scalar::from(3_i64))]),
                    row([("qty", Scalar::from(4_i64))]),
                ]),
            ),
        ])],
    );
    assert_eq!(
        rows_text(&rowset, &batch),
        "<row><leg><price>1.5</price><venue>XPAR</venue></leg>\
         <fills><qty>3</qty></fills><fills><qty>4</qty></fills></row>"
    );
}

#[test]
fn a_struct_cell_writes_its_children_in_the_order_its_schema_declares() {
    // The schema declares the children as an `xsd:sequence`, which fixes
    // their order: `venue` before `price`.
    let field = record([structure([
        DataType::utf8().required_field("venue"),
        DataType::Float64.required_field("price"),
    ])
    .required_field("leg")]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let schema = schema_text(&rowset);
    assert!(
        schema.find("name=\"venue\"").unwrap() < schema.find("name=\"price\"").unwrap(),
        "{schema}"
    );
    let batch = rows(
        &field,
        [row([(
            "leg",
            row([
                ("venue", Scalar::from("XPAR")),
                ("price", Scalar::from(1.5)),
            ]),
        )])],
    );
    assert_eq!(
        rows_text(&rowset, &batch),
        "<row><leg><venue>XPAR</venue><price>1.5</price></leg></row>"
    );
}

#[test]
fn a_struct_child_is_written_under_the_element_name_its_schema_declares() {
    let field = record([structure([
        DataType::utf8().required_field("venue code"),
        DataType::utf8().required_field("a_xb"),
    ])
    .required_field("leg")]);
    let rowset = Rowset::new(field.clone()).expect("a struct child named anything is a column");
    let schema = schema_text(&rowset);
    assert!(
        schema.contains("<xsd:element sql:field=\"venue code\" name=\"venue_x0020_code\""),
        "{schema}"
    );
    assert!(
        schema.contains("<xsd:element sql:field=\"a_xb\" name=\"a_x005F_xb\""),
        "{schema}"
    );
    let batch = rows(
        &field,
        [row([(
            "leg",
            row([
                ("venue code", Scalar::from("XPAR")),
                ("a_xb", Scalar::from("z")),
            ]),
        )])],
    );
    let mut document = Vec::new();
    let written = rowset.write_rows(&mut document, &batch);
    assert!(
        written.is_ok(),
        "a column the rowset accepted is written: {:?}",
        written.err().map(|error| error.to_string())
    );
    assert_eq!(
        String::from_utf8(document).unwrap(),
        "<row><leg><venue_x0020_code>XPAR</venue_x0020_code><a_x005F_xb>z</a_x005F_xb></leg></row>"
    );
}

#[test]
fn write_rows_refuses_a_batch_that_is_not_a_record_of_its_columns() {
    let field = record([
        DataType::Int32.required_field("n"),
        DataType::utf8().nullable_field("Symbol"),
    ]);
    let rowset = Rowset::new(field.clone()).unwrap();

    let column = rows(&DataType::Int32.required_field("x"), [Scalar::from(1)]);
    let message = rows_refusal(&rowset, &column);
    assert!(
        message.contains("expected a record column for the rows of a rowset, got x"),
        "{message}"
    );

    let run = Serie::new(vec![Scalar::from(1), Scalar::from(2)]);
    let message = rows_refusal(&rowset, &run);
    assert!(
        message.contains("expected a record column for the rows of a rowset, got a run"),
        "{message}"
    );

    let narrow = record([DataType::Int32.required_field("n")]);
    let message = rows_refusal(&rowset, &rows(&narrow, [row([("n", Scalar::from(1))])]));
    assert!(
        message.contains("expected 2 columns for the rowset, got 1"),
        "{message}"
    );

    let retyped = record([
        DataType::Int32.required_field("n"),
        DataType::Int64.nullable_field("Symbol"),
    ]);
    let message = rows_refusal(
        &rowset,
        &rows(
            &retyped,
            [row([
                ("n", Scalar::from(1)),
                ("Symbol", Scalar::from(2_i64)),
            ])],
        ),
    );
    assert!(
        message.contains("column `Symbol` holds int64, the rowset declares utf8"),
        "{message}"
    );
    assert!(message.contains("$.rowset"), "{message}");
}

#[test]
fn a_refused_batch_writes_none_of_its_rows() {
    let field = record([DataType::Int32.required_field("n")]);
    let rowset = Rowset::new(field).unwrap();
    let other = record([DataType::Int64.required_field("n")]);
    let mut document = Vec::new();
    rowset
        .write_rows(
            &mut document,
            &rows(&other, [row([("n", Scalar::from(1_i64))])]),
        )
        .expect_err("another field's rows");
    assert!(
        document.is_empty(),
        "{}",
        String::from_utf8_lossy(&document)
    );
}

#[test]
fn a_temporal_with_no_iso_spelling_is_refused_naming_its_column() {
    // Day 3,000,000 falls in the year 10183, past what ISO 8601 spells.
    let field = record([DataType::Date32.required_field("booked")]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let batch = rows(&field, [row([("booked", Scalar::date32(3_000_000))])]);
    let message = rows_refusal(&rowset, &batch);
    assert!(
        message.contains("the date32 count 3000000 under `booked` has no ISO 8601 spelling"),
        "{message}"
    );

    let field = record([naive_datetime().required_field("at")]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let batch = rows(
        &field,
        [row([(
            "at",
            Scalar::datetime64(i64::MAX, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
        )])],
    );
    let message = rows_refusal(&rowset, &batch);
    assert!(message.contains("under `at`"), "{message}");
    assert!(message.contains("ISO 8601"), "{message}");
}

#[test]
fn a_text_cell_xml_cannot_carry_is_refused_naming_its_column() {
    // `write_root`: "a cell with no XML spelling, naming its column".
    let messages: Vec<(DataType, String)> = [DataType::utf8(), DataType::cp1252()]
        .into_iter()
        .map(|dtype| {
            let field = record([dtype.clone().required_field("memo")]);
            let rowset = Rowset::new(field.clone()).unwrap();
            let batch = rows(&field, [row([("memo", Scalar::from("bell\u{7}"))])]);
            (dtype, rows_refusal(&rowset, &batch))
        })
        .collect();
    for (dtype, message) in &messages {
        assert!(message.contains("U+0007"), "{dtype}: {message}");
    }
    assert!(
        messages.iter().all(|(_, message)| message.contains("memo")),
        "the refusal names its column: {messages:?}"
    );
}

// Reading a schema.

#[test]
fn a_literal_schema_reads_back_its_columns() {
    let rowset = from_schema(&schema_document(
        "<xsd:element sql:field=\"Order Id\" name=\"Order_x0020_Id\" type=\"xsd:int\"/>\
         <xsd:element sql:field=\"Symbol\" name=\"Symbol\" type=\"xsd:string\" minOccurs=\"0\"/>\
         <xsd:element name=\"tag_x0020_s\" type=\"xsd:string\" maxOccurs=\"unbounded\"/>\
         <xsd:element sql:field=\"leg\" name=\"leg\" minOccurs=\"0\"><xsd:complexType><xsd:sequence>\
         <xsd:element sql:field=\"price\" name=\"price\" type=\"xsd:double\"/>\
         <xsd:element name=\"venue_x0020_code\" type=\"xsd:string\" minOccurs=\"0\"/>\
         </xsd:sequence></xsd:complexType></xsd:element>\
         <xsd:element sql:field=\"id\" name=\"id\" type=\"uuid\" minOccurs=\"0\"/>\
         <xsd:element sql:field=\"when\" name=\"when\" type=\"xsd:dateTime\"/>\
         <xsd:element sql:field=\"amount\" name=\"amount\" type=\"xsd:decimal\"/>\
         <xsd:element sql:field=\"big\" name=\"big\" type=\"xsd:integer\"/>",
    ))
    .expect("the schema reads");
    let expected = Field::new(
        yggdryl::media::DEFAULT_ROOT_NAME,
        structure([
            DataType::Int32.required_field("Order Id"),
            DataType::utf8().nullable_field("Symbol"),
            DataType::serie(Field::new("item", DataType::utf8(), true)).required_field("tag s"),
            structure([
                DataType::Float64.required_field("price"),
                DataType::utf8().nullable_field("venue code"),
            ])
            .nullable_field("leg"),
            DataType::Uuid.nullable_field("id"),
            naive_datetime().required_field("when"),
            DataType::utf8().required_field("amount"),
            DataType::decimal128(38, 0).unwrap().required_field("big"),
        ]),
        false,
    );
    assert_eq!(rowset.field(), &expected);
    assert_eq!(
        rowset.element_names(),
        [
            "Order_x0020_Id",
            "Symbol",
            "tag_x0020_s",
            "leg",
            "id",
            "when",
            "amount",
            "big"
        ]
    );
}

#[test]
fn a_schema_under_other_prefixes_reads_the_same_columns() {
    let rowset = from_schema(&format!(
        "<xs:schema xmlns:xs=\"{XSD_NAMESPACE}\" xmlns:s=\"{SQL_NAMESPACE}\">\
         <xs:complexType name=\"row\"><xs:sequence>\
         <xs:element s:field=\"Order Id\" name=\"Order_x0020_Id\" type=\"xs:int\"/>\
         <xs:element s:field=\"Symbol\" name=\"Symbol\" type=\"xs:string\" minOccurs=\"0\"/>\
         </xs:sequence></xs:complexType></xs:schema>"
    ))
    .expect("the schema reads");
    assert_eq!(
        rowset.field(),
        &record([
            DataType::Int32.required_field("Order Id"),
            DataType::utf8().nullable_field("Symbol"),
        ])
    );
    assert_eq!(rowset.element_names(), ["Order_x0020_Id", "Symbol"]);
}

#[test]
fn a_type_named_under_an_unbound_prefix_or_outside_the_table_reads_as_text() {
    let rowset = from_schema(&schema_document(
        "<xsd:element name=\"a\" type=\"xsd:gYear\"/>\
         <xsd:element name=\"b\" type=\"other:int\"/>\
         <xsd:element name=\"c\" type=\"int\"/>\
         <xsd:element name=\"d\"/>\
         <xsd:element name=\"e\" type=\"sql:int\"/>",
    ))
    .expect("an unknown type is text");
    assert_eq!(
        rowset.field(),
        &record([
            DataType::utf8().required_field("a"),
            DataType::utf8().required_field("b"),
            DataType::utf8().required_field("c"),
            DataType::utf8().required_field("d"),
            DataType::utf8().required_field("e"),
        ])
    );
}

#[test]
fn every_xsd_type_named_in_a_schema_reads_as_its_datatype() {
    let declarations: String = XsdType::ALL
        .iter()
        .enumerate()
        .map(|(index, held)| format!("<xsd:element name=\"c{index}\" type=\"{held}\"/>"))
        .collect();
    let rowset = from_schema(&schema_document(&declarations)).expect("the schema reads");
    for (index, (column, held)) in rowset.field().fields().iter().zip(XsdType::ALL).enumerate() {
        assert_eq!(column.name(), format!("c{index}"));
        assert_eq!(column.dtype(), &held.datatype(), "{held}");
    }
}

#[test]
fn min_occurs_zero_is_a_nullable_column_and_max_occurs_past_one_a_sequence() {
    let rowset = from_schema(&schema_document(
        "<xsd:element name=\"a\" type=\"xsd:int\" minOccurs=\"0\"/>\
         <xsd:element name=\"b\" type=\"xsd:int\" minOccurs=\" 0 \"/>\
         <xsd:element name=\"c\" type=\"xsd:int\" minOccurs=\"1\"/>\
         <xsd:element name=\"d\" type=\"xsd:int\" maxOccurs=\"1\"/>\
         <xsd:element name=\"e\" type=\"xsd:int\" maxOccurs=\"5\"/>\
         <xsd:element name=\"f\" type=\"xsd:int\" minOccurs=\"0\" maxOccurs=\"unbounded\"/>",
    ))
    .expect("the schema reads");
    let item = || Field::new("item", DataType::Int32, true);
    assert_eq!(
        rowset.field(),
        &record([
            DataType::Int32.nullable_field("a"),
            DataType::Int32.nullable_field("b"),
            DataType::Int32.required_field("c"),
            DataType::Int32.required_field("d"),
            DataType::serie(item()).required_field("e"),
            DataType::serie(item()).nullable_field("f"),
        ])
    );
}

#[test]
fn a_written_schema_reads_back_as_the_columns_its_types_prove() {
    let written = Rowset::new(record([
        DataType::Int16.required_field("Order Id"),
        DataType::large_utf8().nullable_field("Symbol"),
        DataType::decimal64(12, 3).unwrap().required_field("price"),
        DataType::Date64.nullable_field("day"),
        DataType::serie(Field::new("item", DataType::Int64, true)).required_field("sizes"),
        structure([DataType::Boolean.required_field("live")]).nullable_field("state"),
    ]))
    .unwrap();
    let value = yggdryl::from_xml_scalar(root_text(&written, Vec::new(), true, false))
        .expect("the document is XML");
    let root = Element::root(&value).expect("a root");
    let schema = root
        .child(Some(XSD_NAMESPACE), "schema")
        .expect("the root holds its schema");
    let read = Rowset::from_schema(&schema).expect("the schema reads back");
    assert_eq!(read.element_names(), written.element_names());
    assert_eq!(
        read.field(),
        &record([
            DataType::Int16.required_field("Order Id"),
            DataType::utf8().nullable_field("Symbol"),
            DataType::utf8().required_field("price"),
            DataType::Date32.nullable_field("day"),
            // Items that may be null make the repeated element optional.
            DataType::serie(Field::new("item", DataType::Int64, true)).nullable_field("sizes"),
            structure([DataType::Boolean.required_field("live")]).nullable_field("state"),
        ])
    );
}

#[test]
fn a_schema_declaring_no_row_type_is_refused() {
    for document in [
        format!("<xsd:schema xmlns:xsd=\"{XSD_NAMESPACE}\"/>"),
        format!(
            "<xsd:schema xmlns:xsd=\"{XSD_NAMESPACE}\">\
             <xsd:complexType name=\"other\"><xsd:sequence/></xsd:complexType></xsd:schema>"
        ),
        format!(
            "<xsd:schema xmlns:xsd=\"{XSD_NAMESPACE}\" xmlns:o=\"urn:other\">\
             <o:complexType name=\"row\"><xsd:sequence/></o:complexType></xsd:schema>"
        ),
        format!(
            "<xsd:schema xmlns:xsd=\"{XSD_NAMESPACE}\">\
             <xsd:complexType><xsd:sequence/></xsd:complexType></xsd:schema>"
        ),
    ] {
        let message = from_schema(&document).expect_err("no row type").to_string();
        assert!(
            message.contains("the rowset schema declares no `row` type"),
            "{message}"
        );
        assert!(message.contains("$.rowset"), "{message}");
    }
}

#[test]
fn a_row_type_declaring_no_sequence_is_refused() {
    let message = from_schema(&format!(
        "<xsd:schema xmlns:xsd=\"{XSD_NAMESPACE}\">\
         <xsd:complexType name=\"row\"><xsd:all/></xsd:complexType></xsd:schema>"
    ))
    .expect_err("no sequence")
    .to_string();
    assert!(
        message.contains("the rowset schema's `row` type declares no sequence"),
        "{message}"
    );
}

#[test]
fn an_empty_sequence_is_a_rowset_of_no_columns() {
    let rowset = from_schema(&format!(
        "<xsd:schema xmlns:xsd=\"{XSD_NAMESPACE}\">\
         <xsd:complexType name=\"row\"><xsd:sequence></xsd:sequence></xsd:complexType></xsd:schema>"
    ))
    .expect("no columns");
    assert_eq!(rowset.field(), &record([]));
}

#[test]
fn a_column_declaration_without_a_name_is_refused() {
    for declarations in [
        "<xsd:element type=\"xsd:int\"/>",
        "<xsd:element name=\"leg\"><xsd:complexType><xsd:sequence>\
         <xsd:element type=\"xsd:int\"/></xsd:sequence></xsd:complexType></xsd:element>",
    ] {
        let message = from_schema(&schema_document(declarations))
            .expect_err("a nameless column")
            .to_string();
        assert!(
            message.contains("a column declaration without a `name`"),
            "{message}"
        );
    }
}

#[test]
fn two_declarations_of_one_column_are_refused() {
    for declarations in [
        "<xsd:element name=\"a\" type=\"xsd:int\"/><xsd:element name=\"a\" type=\"xsd:string\"/>",
        "<xsd:element sql:field=\"a b\" name=\"x\" type=\"xsd:int\"/>\
         <xsd:element name=\"a_x0020_b\" type=\"xsd:int\"/>",
    ] {
        let message = from_schema(&schema_document(declarations))
            .expect_err("a duplicated column")
            .to_string();
        assert!(message.contains("duplicate"), "{message}");
    }
}

// Reading rows.

#[test]
fn literal_rows_read_back_as_the_values_the_writer_writes() {
    let (field, value, text) = leaves();
    let rowset = Rowset::new(field.clone()).unwrap();
    let back = read_rows(&rowset, &schemaless(text)).expect("the rows read");
    assert_eq!(back, rows(&field, [value.clone()]));

    // Through the writer as well: what it writes reads back.
    let written = root_text(&rowset, vec![rows(&field, [value.clone()])], false, true);
    let back = read_rows(&rowset, &written).expect("the written rows read");
    assert_eq!(back, rows(&field, [value]));
}

#[test]
fn rows_read_in_document_order() {
    let field = record([DataType::Int32.required_field("n")]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let back = read_rows(
        &rowset,
        &schemaless("<row><n>3</n></row><row><n>1</n></row><row><n>2</n></row>"),
    )
    .unwrap();
    assert_eq!(
        back,
        rows(&field, [3, 1, 2].map(|n| row([("n", Scalar::from(n))])))
    );
}

#[test]
fn a_missing_nullable_cell_and_an_xsi_nil_cell_are_null() {
    let field = record([
        DataType::Int32.required_field("n"),
        DataType::utf8().nullable_field("s"),
        DataType::Date32.nullable_field("d"),
    ]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let back = read_rows(
        &rowset,
        &schemaless(
            "<row><n>1</n></row>\
             <row><n>2</n><s xsi:nil=\"true\"/><d xsi:nil=\"1\"></d></row>\
             <row><n>3</n><s/><d/></row>",
        ),
    )
    .expect("the rows read");
    assert_eq!(
        back,
        rows(
            &field,
            [1, 2, 3].map(|n| row([
                ("n", Scalar::from(n)),
                ("s", Scalar::Null),
                ("d", Scalar::Null)
            ]))
        )
    );
}

#[test]
fn a_nil_marked_under_another_prefix_for_the_instance_namespace_is_null() {
    let field = record([
        DataType::Int32.required_field("n"),
        DataType::utf8().nullable_field("s"),
    ]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let back = read_rows(
        &rowset,
        &format!(
            "<root xmlns=\"{ROWSET_NAMESPACE}\" xmlns:i=\"{XSI_NAMESPACE}\">\
             <row><n>1</n><s i:nil=\"true\"/></row></root>"
        ),
    );
    assert_eq!(
        back.as_ref().map_err(ToString::to_string),
        Ok(&rows(
            &field,
            [row([("n", Scalar::from(1)), ("s", Scalar::Null)])]
        )),
        "`i:nil` is `nil` in the XML Schema instance namespace"
    );
}

#[test]
fn an_empty_text_cell_is_the_empty_text_and_empty_elsewhere_is_null() {
    let field = record([
        DataType::utf8().nullable_field("s"),
        DataType::Int32.nullable_field("n"),
        DataType::binary().nullable_field("b"),
    ]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let back = read_rows(&rowset, &schemaless("<row><s></s><n></n><b></b></row>")).unwrap();
    assert_eq!(
        back,
        rows(
            &field,
            [row([
                ("s", Scalar::from("")),
                ("n", Scalar::Null),
                ("b", Scalar::from(Vec::<u8>::new())),
            ])]
        )
    );
}

#[test]
fn text_keeps_its_whitespace_and_every_other_leaf_is_trimmed() {
    let field = record([
        DataType::utf8().required_field("s"),
        DataType::Int32.required_field("n"),
        DataType::Date32.required_field("d"),
        DataType::Boolean.required_field("b"),
    ]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let back = read_rows(
        &rowset,
        &schemaless("<row><s>  AAPL \n</s><n>\n 7 </n><d> 2024-01-01 </d><b>\ttrue\t</b></row>"),
    )
    .unwrap();
    assert_eq!(
        back,
        rows(
            &field,
            [row([
                ("s", Scalar::from("  AAPL \n")),
                ("n", Scalar::from(7)),
                ("d", Scalar::date32(19_723)),
                ("b", Scalar::from(true)),
            ])]
        )
    );
}

#[test]
fn a_cell_reads_back_its_escaped_and_unicode_text() {
    let field = record([DataType::utf8().required_field("s")]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let back = read_rows(
        &rowset,
        &schemaless(
            "<row><s>a &lt;b&gt; &amp; c</s></row><row><s>trädes €&#x1F600;</s></row>\
             <row><s><![CDATA[<raw>]]></s></row>",
        ),
    )
    .unwrap();
    assert_eq!(
        back,
        rows(
            &field,
            ["a <b> & c", "trädes €\u{1F600}", "<raw>"].map(|s| row([("s", Scalar::from(s))]))
        )
    );
}

#[test]
fn a_missing_required_cell_is_refused_naming_its_row_and_column() {
    // `read_rows`: "An absent element and one marked `xsi:nil` are null", and
    // a null under a required column is refused, as the nil is below.
    let field = record([
        DataType::Int32.required_field("Order Id"),
        DataType::utf8().nullable_field("Symbol"),
    ]);
    let rowset = Rowset::new(field).unwrap();
    let message = match read_rows(
        &rowset,
        &schemaless(
            "<row><Order_x0020_Id>1</Order_x0020_Id></row><row><Symbol>AAPL</Symbol></row>",
        ),
    ) {
        Ok(back) => panic!(
            "the second row has no id, and reads as {:?}",
            back.scalar(1)
        ),
        Err(error) => error.to_string(),
    };
    assert!(message.contains("$[1]"), "{message}");
    assert!(message.contains("Order Id"), "{message}");
}

#[test]
fn a_nil_or_empty_required_cell_is_refused_naming_its_row_and_column() {
    let field = record([
        DataType::Int32.required_field("Order Id"),
        DataType::utf8().nullable_field("Symbol"),
    ]);
    let rowset = Rowset::new(field).unwrap();
    for cell in [
        "<Order_x0020_Id xsi:nil=\"true\"/>",
        "<Order_x0020_Id/>",
        "<Order_x0020_Id></Order_x0020_Id>",
        "<Order_x0020_Id>  </Order_x0020_Id>",
    ] {
        let message = read_rows(
            &rowset,
            &schemaless(&format!(
                "<row><Order_x0020_Id>1</Order_x0020_Id></row><row>{cell}</row>"
            )),
        )
        .expect_err("a required cell read as null")
        .to_string();
        assert!(message.contains("$[1]"), "{cell}: {message}");
        assert!(message.contains("Order Id"), "{cell}: {message}");
        assert!(
            message.contains("non-nullable field received null"),
            "{cell}: {message}"
        );
    }
}

#[test]
fn a_malformed_cell_is_refused_naming_its_row_and_its_column() {
    for (dtype, good, bad) in [
        (DataType::Int32, "1", "seven"),
        (DataType::UInt8, "1", "300"),
        (DataType::Float64, "1.5", "1.2.3"),
        (DataType::Boolean, "true", "yes please"),
        (DataType::Date32, "2024-01-01", "2024-13-01"),
        (naive_datetime(), "2024-01-01T09:30:00", "yesterday"),
        (DataType::Uuid, ID_TEXT, "not-a-uuid"),
        (DataType::decimal128(10, 2).unwrap(), "1.25", "12.5.0"),
        (DataType::binary(), "AAEC", "not base64!"),
    ] {
        let field = record([
            DataType::Int32.required_field("n"),
            dtype.clone().required_field("the cell"),
        ]);
        let rowset = Rowset::new(field).unwrap();
        let message = read_rows(
            &rowset,
            &schemaless(&format!(
                "<row><n>1</n><the_x0020_cell>{good}</the_x0020_cell></row>\
                 <row><n>2</n><the_x0020_cell>{bad}</the_x0020_cell></row>"
            )),
        )
        .expect_err("a malformed cell")
        .to_string();
        assert!(message.contains("$[1]"), "{dtype}: {message}");
        assert!(message.contains("the cell"), "{dtype}: {message}");
    }
}

#[test]
fn an_element_no_column_declares_is_refused() {
    let field = record([DataType::Int32.required_field("n")]);
    let rowset = Rowset::new(field).unwrap();
    let message = read_rows(&rowset, &schemaless("<row><n>1</n><Extra>x</Extra></row>"))
        .expect_err("an undeclared element")
        .to_string();
    assert!(message.contains("$[0]"), "{message}");
    assert!(message.contains("Extra"), "{message}");
}

#[test]
fn a_repeated_element_under_a_leaf_column_is_refused() {
    let field = record([DataType::utf8().required_field("s")]);
    let rowset = Rowset::new(field).unwrap();
    let message = read_rows(&rowset, &schemaless("<row><s>a</s><s>b</s></row>"))
        .expect_err("one cell spelled twice")
        .to_string();
    assert!(message.contains("$[0]"), "{message}");
}

#[test]
fn a_sequence_column_reads_its_repeated_elements_and_one_element_as_one_item() {
    let field = record([
        DataType::serie(Field::new("item", DataType::Int32, true)).required_field("xs"),
        DataType::serie(Field::new(
            "item",
            structure([DataType::Int64.required_field("qty")]),
            true,
        ))
        .nullable_field("fills"),
    ]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let back = read_rows(
        &rowset,
        &schemaless(
            "<row><xs>1</xs><xs>2</xs><xs/><fills><qty>3</qty></fills><fills><qty>4</qty></fills></row>\
             <row><xs>5</xs><fills><qty>6</qty></fills></row>\
             <row><xs>7</xs></row>",
        ),
    )
    .expect("the rows read");
    let fill = |qty: i64| row([("qty", Scalar::from(qty))]);
    assert_eq!(
        back,
        rows(
            &field,
            [
                row([
                    (
                        "xs",
                        Scalar::from_sequence([Scalar::from(1), Scalar::from(2), Scalar::Null])
                    ),
                    ("fills", Scalar::from_sequence([fill(3), fill(4)])),
                ]),
                row([
                    ("xs", Scalar::from_sequence([Scalar::from(5)])),
                    ("fills", Scalar::from_sequence([fill(6)])),
                ]),
                // A repeated element occurring no time is the empty sequence.
                row([
                    ("xs", Scalar::from_sequence([Scalar::from(7)])),
                    ("fills", Scalar::from_sequence([])),
                ]),
            ]
        )
    );
}

#[test]
fn a_self_closed_row_is_a_row_of_absent_cells() {
    // `<row/>` and `<row></row>` are one element with no content in XML 1.0.
    let field = record([
        DataType::Int32.nullable_field("n"),
        DataType::utf8().nullable_field("s"),
    ]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let expected = rows(&field, [row([("n", Scalar::Null), ("s", Scalar::Null)])]);
    assert_eq!(
        read_rows(&rowset, &schemaless("<row></row>")).map_err(|error| error.to_string()),
        Ok(expected.clone())
    );
    assert_eq!(
        read_rows(&rowset, &schemaless("<row/>")).map_err(|error| error.to_string()),
        Ok(expected)
    );
}

#[test]
fn a_struct_column_reads_its_child_elements_in_any_order() {
    let field = record([structure([
        DataType::utf8().required_field("venue"),
        DataType::Float64.required_field("price"),
        DataType::Int32.nullable_field("qty"),
    ])
    .nullable_field("leg")]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let back = read_rows(
        &rowset,
        &schemaless(
            "<row><leg><price> 1.5 </price><venue>XPAR</venue></leg></row><row><leg/></row>",
        ),
    )
    .expect("the rows read");
    assert_eq!(
        back,
        rows(
            &field,
            [
                row([(
                    "leg",
                    row([
                        ("venue", Scalar::from("XPAR")),
                        ("price", Scalar::from(1.5)),
                        ("qty", Scalar::Null),
                    ]),
                )]),
                row([("leg", Scalar::Null)]),
            ]
        )
    );
}

#[test]
fn a_struct_of_null_children_is_present_and_a_null_struct_is_absent() {
    let field = record([structure([
        DataType::Int32.nullable_field("a"),
        DataType::utf8().nullable_field("b"),
    ])
    .nullable_field("leg")]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let batch = rows(
        &field,
        [
            row([("leg", row([("a", Scalar::Null), ("b", Scalar::Null)]))]),
            row([("leg", Scalar::Null)]),
        ],
    );
    let text = rows_text(&rowset, &batch);
    assert_eq!(text, "<row><leg></leg></row><row></row>");
    assert_eq!(read_rows(&rowset, &schemaless(&text)).unwrap(), batch);
}

#[test]
fn a_struct_cell_missing_a_required_child_is_refused_naming_it() {
    let field = record([structure([
        DataType::utf8().required_field("venue"),
        DataType::Float64.required_field("price"),
    ])
    .nullable_field("leg")]);
    let rowset = Rowset::new(field).unwrap();
    let message = read_rows(
        &rowset,
        &schemaless("<row><leg><venue>XPAR</venue><price>1</price></leg></row><row><leg><venue>XPAR</venue></leg></row>"),
    )
    .expect_err("the second leg has no price")
    .to_string();
    assert!(message.contains("$[1]"), "{message}");
    assert!(message.contains("price"), "{message}");
}

#[test]
fn a_struct_child_under_its_encoded_element_name_reads_back() {
    let (rowset, back) = read_root(
        &document(
            "<xsd:element sql:field=\"leg\" name=\"leg\"><xsd:complexType><xsd:sequence>\
             <xsd:element sql:field=\"venue code\" name=\"venue_x0020_code\" type=\"xsd:string\"/>\
             </xsd:sequence></xsd:complexType></xsd:element>",
            "<row><leg><venue_x0020_code>XPAR</venue_x0020_code></leg></row>",
        ),
        None,
    )
    .map_err(|error| error.to_string())
    .expect("a nested column spelled as its schema declares it reads");
    assert_eq!(
        back,
        rows(
            rowset.field(),
            [row([("leg", row([("venue code", Scalar::from("XPAR"))]))])]
        )
    );
}

#[test]
fn an_element_name_reads_as_the_column_it_decodes_to() {
    let field = record([
        DataType::Int32.required_field("Order Id"),
        DataType::utf8().nullable_field("Symbol"),
    ]);
    let rowset = Rowset::new(field.clone()).unwrap();
    // A writer that escapes a character no rule required still names the
    // column the escape decodes to.
    let back = read_rows(
        &rowset,
        &schemaless(
            "<row><Order_x0020_Id>7</Order_x0020_Id><_x0053_ymbol>AAPL</_x0053_ymbol></row>",
        ),
    )
    .expect("the rows read");
    assert_eq!(
        back,
        rows(
            &field,
            [row([
                ("Order Id", Scalar::from(7)),
                ("Symbol", Scalar::from("AAPL"))
            ])]
        )
    );
}

#[test]
fn rows_under_any_prefix_bound_to_the_rowset_namespace_or_in_none_are_read() {
    let field = record([DataType::Int32.required_field("n")]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let expected = rows(&field, [row([("n", Scalar::from(1))])]);

    let prefixed =
        format!("<r:root xmlns:r=\"{ROWSET_NAMESPACE}\"><r:row><r:n>1</r:n></r:row></r:root>");
    assert_eq!(read_rows(&rowset, &prefixed).unwrap(), expected);

    let unqualified = "<root><row><n>1</n></row></root>";
    assert_eq!(read_rows(&rowset, unqualified).unwrap(), expected);

    let foreign = format!(
        "<root xmlns=\"{ROWSET_NAMESPACE}\" xmlns:o=\"urn:other\">\
         <row><n>1</n></row><o:row><n>2</n></o:row></root>"
    );
    assert_eq!(
        read_rows(&rowset, &foreign).unwrap(),
        expected,
        "a row in another namespace is another element"
    );
}

#[test]
fn attributes_are_neither_cells_nor_part_of_a_cell_s_value() {
    let field = record([DataType::Int32.required_field("n")]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let back = read_rows(
        &rowset,
        &schemaless(
            "<row id=\"9\" xmlns:x=\"urn:x\"><n x:tag=\"z\">1</n></row>\
             <row id=\"10\"><n>2</n></row>",
        ),
    )
    .expect("the rows read");
    assert_eq!(
        back,
        rows(&field, [1, 2].map(|n| row([("n", Scalar::from(n))])))
    );
}

#[test]
fn a_root_of_no_rows_reads_as_no_rows() {
    let field = record([DataType::Int32.required_field("n")]);
    let rowset = Rowset::new(field.clone()).unwrap();
    for text in [
        schemaless(""),
        "<root/>".to_owned(),
        "<root></root>".to_owned(),
    ] {
        let back = read_rows(&rowset, &text).expect("no rows");
        assert_eq!(back.len(), 0, "{text}");
        assert_eq!(back.field(), Some(&field), "{text}");
    }
}

// Reading a whole root.

#[test]
fn read_root_without_a_field_reads_the_columns_its_schema_declares() {
    let (rowset, back) = read_root(
        &document(
            "<xsd:element sql:field=\"Order Id\" name=\"Order_x0020_Id\" type=\"xsd:int\"/>\
             <xsd:element sql:field=\"Symbol\" name=\"Symbol\" type=\"xsd:string\" minOccurs=\"0\"/>",
            "<row><Order_x0020_Id>7</Order_x0020_Id><Symbol>AAPL</Symbol></row>\
             <row><Order_x0020_Id>8</Order_x0020_Id></row>",
        ),
        None,
    )
    .expect("the rowset reads");
    let field = record([
        DataType::Int32.required_field("Order Id"),
        DataType::utf8().nullable_field("Symbol"),
    ]);
    assert_eq!(rowset.field(), &field);
    assert_eq!(
        back,
        rows(
            &field,
            [
                row([
                    ("Order Id", Scalar::from(7)),
                    ("Symbol", Scalar::from("AAPL"))
                ]),
                row([("Order Id", Scalar::from(8)), ("Symbol", Scalar::Null)]),
            ]
        )
    );
}

#[test]
fn read_root_with_a_field_reads_the_rows_under_it_and_not_the_schema() {
    let declared = record([
        DataType::Int64.required_field("Order Id"),
        DataType::Date32.nullable_field("Symbol"),
    ]);
    let (rowset, back) = read_root(
        &document(
            "<xsd:element sql:field=\"Order Id\" name=\"Order_x0020_Id\" type=\"xsd:string\"/>\
             <xsd:element sql:field=\"Symbol\" name=\"Symbol\" type=\"xsd:string\" minOccurs=\"0\"/>",
            "<row><Order_x0020_Id>7</Order_x0020_Id><Symbol>2024-01-01</Symbol></row>",
        ),
        Some(&declared),
    )
    .expect("the rowset reads under the declared field");
    assert_eq!(rowset.field(), &declared);
    assert_eq!(
        back,
        rows(
            &declared,
            [row([
                ("Order Id", Scalar::from(7_i64)),
                ("Symbol", Scalar::date32(19_723)),
            ])]
        )
    );

    // A document a foreign provider wrote with no schema at all.
    let (_, back) = read_root(
        &schemaless("<row><Order_x0020_Id>9</Order_x0020_Id></row>"),
        Some(&declared),
    )
    .expect("the declared field types a schemaless root");
    assert_eq!(
        back,
        rows(
            &declared,
            [row([
                ("Order Id", Scalar::from(9_i64)),
                ("Symbol", Scalar::Null)
            ])]
        )
    );
}

#[test]
fn a_decimal_column_reads_back_through_its_schema_as_its_exact_text() {
    let field = record([DataType::decimal128(10, 2)
        .unwrap()
        .required_field("amount")]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let batch = rows(&field, [row([("amount", Scalar::d128(1250, 2))])]);
    let (read, back) = read_root(&root_text(&rowset, vec![batch], true, true), None)
        .expect("the rowset reads back");
    let text = record([DataType::utf8().required_field("amount")]);
    assert_eq!(read.field(), &text, "the schema states no scale");
    assert_eq!(
        back,
        rows(&text, [row([("amount", Scalar::from("12.50"))])])
    );
}

#[test]
fn an_xsd_integer_column_reads_as_a_scale_free_decimal() {
    let (read, back) = read_root(
        &document(
            "<xsd:element sql:field=\"big\" name=\"big\" type=\"xsd:integer\"/>",
            "<row><big>12345678901234567890123456789012345678</big></row><row><big>-7</big></row>",
        ),
        None,
    )
    .expect("the rowset reads");
    let field = record([DataType::decimal128(38, 0).unwrap().required_field("big")]);
    assert_eq!(read.field(), &field);
    assert_eq!(
        back,
        rows(
            &field,
            [
                row([(
                    "big",
                    Scalar::d128(12_345_678_901_234_567_890_123_456_789_012_345_678, 0)
                )]),
                row([("big", Scalar::d128(-7, 0))]),
            ]
        )
    );
}

#[test]
fn a_uuid_cell_reads_in_either_case() {
    let field = record([DataType::Uuid.required_field("id")]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let back = read_rows(
        &rowset,
        &schemaless(&format!(
            "<row><id>{ID_TEXT}</id></row><row><id>{}</id></row>",
            ID_TEXT.to_ascii_uppercase()
        )),
    )
    .expect("the rows read");
    let id = || row([("id", Scalar::Uuid(Uuid::new(ID)))]);
    assert_eq!(back, rows(&field, [id(), id()]));
}

#[test]
fn a_base64_cell_reads_through_the_whitespace_xml_schema_collapses() {
    // `xsd:base64Binary` collapses whitespace and allows a single space
    // between groups; an indenting writer leaves both.
    let field = record([DataType::binary().required_field("b")]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let results: Vec<(&str, Result<Serie, String>)> = [" AP9oaQ== ", "\n  AP9oaQ==\n", "AP9o aQ=="]
        .into_iter()
        .map(|text| {
            (
                text,
                read_rows(&rowset, &schemaless(&format!("<row><b>{text}</b></row>")))
                    .map_err(|error| error.to_string()),
            )
        })
        .collect();
    let expected = rows(
        &field,
        [row([("b", Scalar::from(vec![0_u8, 0xFF, b'h', b'i']))])],
    );
    assert!(
        results
            .iter()
            .all(|(_, back)| back.as_ref() == Ok(&expected)),
        "{results:?}"
    );
}

#[test]
fn a_rowset_read_from_the_schema_it_wrote_equals_it() {
    let field = record([
        DataType::Int32.required_field("Order Id"),
        DataType::utf8().nullable_field("Symbol"),
        DataType::serie(Field::new("item", DataType::Int64, true)).nullable_field("sizes"),
        structure([DataType::Boolean.required_field("live")]).nullable_field("state"),
        DataType::Uuid.required_field("id"),
    ]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let (read, back) = read_root(&root_text(&rowset, Vec::new(), true, false), None)
        .expect("the schema reads back");
    assert_eq!(read, rowset);
    assert_eq!(read.clone(), rowset);
    assert_eq!(back.len(), 0);
    assert_ne!(
        read,
        Rowset::new(record([DataType::Int32.required_field("Order Id")])).unwrap()
    );
}

#[test]
fn read_root_finds_a_schema_under_any_prefix_bound_to_the_xml_schema_namespace() {
    let (read, back) = read_root(
        &format!(
            "<root xmlns=\"{ROWSET_NAMESPACE}\" xmlns:xs=\"{XSD_NAMESPACE}\">\
             <xs:schema xmlns:sql=\"{SQL_NAMESPACE}\"><xs:complexType name=\"row\"><xs:sequence>\
             <xs:element sql:field=\"n\" name=\"n\" type=\"xs:long\"/>\
             </xs:sequence></xs:complexType></xs:schema><row><n>5</n></row></root>"
        ),
        None,
    )
    .expect("the rowset reads");
    let field = record([DataType::Int64.required_field("n")]);
    assert_eq!(read.field(), &field);
    assert_eq!(back, rows(&field, [row([("n", Scalar::from(5_i64))])]));
}

#[test]
fn read_root_refuses_a_declared_field_it_cannot_spell() {
    let error = read_root(
        &schemaless(""),
        Some(&DataType::Int32.required_field("row")),
    )
    .expect_err("not a record");
    assert!(
        error.to_string().contains("expected a struct field"),
        "{error}"
    );
}

#[test]
fn a_root_with_no_schema_and_no_field_is_refused() {
    let message = read_root(&schemaless("<row><n>1</n></row>"), None)
        .expect_err("nothing states the columns")
        .to_string();
    assert!(
        message
            .contains("the rowset carries no schema and no field is declared to read its rows by"),
        "{message}"
    );
    assert!(message.contains("$.rowset"), "{message}");
}

#[test]
fn a_whole_rowset_round_trips_through_its_own_schema() {
    let field = record([
        DataType::Int32.required_field("Order Id"),
        DataType::utf8().nullable_field("Symbol"),
        DataType::Float64.required_field("price"),
        DataType::Boolean.nullable_field("live"),
        DataType::Date32.nullable_field("day"),
        DataType::Time64(TimeUnit::Microsecond).nullable_field("clock"),
        naive_datetime().nullable_field("wall"),
        DataType::Duration64(TimeUnit::Microsecond).nullable_field("span"),
        DataType::binary().nullable_field("blob"),
        DataType::Uuid.nullable_field("id"),
        DataType::Url.nullable_field("link"),
        structure([DataType::Int64.required_field("qty")]).nullable_field("leg"),
    ]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let batches = vec![
        rows(
            &field,
            [row([
                ("Order Id", Scalar::from(7)),
                ("Symbol", Scalar::from("AAPL")),
                ("price", Scalar::from(187.5)),
                ("live", Scalar::from(true)),
                ("day", Scalar::date32(19_723)),
                (
                    "clock",
                    Scalar::time64(1_000_001, TimeUnit::Microsecond, Timezone::NAIVE).unwrap(),
                ),
                ("wall", naive_at("23:59:59")),
                (
                    "span",
                    Scalar::duration64(-1_500_000, TimeUnit::Microsecond).unwrap(),
                ),
                ("blob", Scalar::from(b"\x00\x01".to_vec())),
                ("id", Scalar::Uuid(Uuid::new(ID))),
                ("link", Scalar::from("https://example.com/")),
                ("leg", row([("qty", Scalar::from(3_i64))])),
            ])],
        ),
        rows(
            &field,
            [row([
                ("Order Id", Scalar::from(8)),
                ("Symbol", Scalar::Null),
                ("price", Scalar::from(-0.25)),
                ("live", Scalar::Null),
                ("day", Scalar::Null),
                ("clock", Scalar::Null),
                ("wall", Scalar::Null),
                ("span", Scalar::Null),
                ("blob", Scalar::Null),
                ("id", Scalar::Null),
                ("link", Scalar::Null),
                ("leg", Scalar::Null),
            ])],
        ),
    ];
    let text = root_text(&rowset, batches.clone(), true, true);
    let (read, back) = read_root(&text, None).expect("the rowset reads back");
    assert_eq!(read.field(), &field);
    assert_eq!(read.element_names(), rowset.element_names());
    let expected: Vec<Scalar> = batches
        .iter()
        .flat_map(|batch| (0..batch.len()).map(|index| batch.scalar(index).unwrap()))
        .collect();
    assert_eq!(back, rows(&field, expected));
}

// Which `xsd:dateTime` columns are instants.

#[test]
fn a_date_time_column_whose_first_value_spells_a_zone_reads_as_utc_instants() {
    for (first, second) in [
        ("2024-01-01T09:30:00Z", "2024-01-01T10:00:00Z"),
        ("2024-01-01T09:30:00z", "2024-01-01T10:00:00Z"),
        ("2024-01-01T11:30:00+02:00", "2024-01-01T05:00:00-05:00"),
        (
            " 2024-01-01T09:30:00.000000Z ",
            "2024-01-01T10:00:00.000000+00:00",
        ),
    ] {
        let (rowset, back) = read_root(
            &document(
                "<xsd:element sql:field=\"at\" name=\"at\" type=\"xsd:dateTime\" minOccurs=\"0\"/>\
                 <xsd:element sql:field=\"n\" name=\"n\" type=\"xsd:int\"/>",
                &format!(
                    "<row><n>1</n></row><row><at>{first}</at><n>2</n></row>\
                     <row><at>{second}</at><n>3</n></row>"
                ),
            ),
            None,
        )
        .unwrap_or_else(|error| panic!("{first}: {error}"));
        let field = record([
            utc_datetime().nullable_field("at"),
            DataType::Int32.required_field("n"),
        ]);
        assert_eq!(rowset.field(), &field, "{first}");
        assert_eq!(
            back,
            rows(
                &field,
                [
                    row([("at", Scalar::Null), ("n", Scalar::from(1))]),
                    row([("at", utc_at("09:30:00")), ("n", Scalar::from(2))]),
                    row([("at", utc_at("10:00:00")), ("n", Scalar::from(3))]),
                ]
            ),
            "{first}"
        );
    }
}

#[test]
fn a_date_time_column_whose_first_value_is_a_wall_reading_stays_naive() {
    let (rowset, back) = read_root(
        &document(
            "<xsd:element sql:field=\"at\" name=\"at\" type=\"xsd:dateTime\"/>",
            "<row><at>2024-01-01T09:30:00</at></row><row><at>2024-01-01T10:00:00.5</at></row>",
        ),
        None,
    )
    .expect("the rowset reads");
    let field = record([naive_datetime().required_field("at")]);
    assert_eq!(rowset.field(), &field);
    assert_eq!(
        back,
        rows(
            &field,
            [
                row([("at", naive_at("09:30:00"))]),
                row([(
                    "at",
                    Scalar::datetime64(
                        microseconds("10:00:00") + 500_000,
                        TimeUnit::Microsecond,
                        Timezone::NAIVE
                    )
                    .unwrap()
                )]),
            ]
        )
    );
}

#[test]
fn each_date_time_column_is_decided_by_its_own_first_value() {
    let (rowset, _) = read_root(
        &document(
            "<xsd:element sql:field=\"wall\" name=\"wall\" type=\"xsd:dateTime\" minOccurs=\"0\"/>\
             <xsd:element sql:field=\"instant\" name=\"instant\" type=\"xsd:dateTime\" minOccurs=\"0\"/>\
             <xsd:element sql:field=\"never\" name=\"never\" type=\"xsd:dateTime\" minOccurs=\"0\"/>",
            "<row><wall>2024-01-01T09:30:00</wall></row>\
             <row><instant>2024-01-01T09:30:00Z</instant><wall>2024-01-01T09:30:00</wall></row>",
        ),
        None,
    )
    .expect("the rowset reads");
    assert_eq!(
        rowset.field(),
        &record([
            naive_datetime().nullable_field("wall"),
            utc_datetime().nullable_field("instant"),
            naive_datetime().nullable_field("never"),
        ])
    );
}

#[test]
fn a_date_time_column_under_an_escaped_name_is_decided_by_its_element() {
    let (rowset, _) = read_root(
        &document(
            "<xsd:element sql:field=\"traded at\" name=\"traded_x0020_at\" type=\"xsd:dateTime\"/>",
            "<row><traded_x0020_at>2024-01-01T09:30:00Z</traded_x0020_at></row>",
        ),
        None,
    )
    .expect("the rowset reads");
    assert_eq!(
        rowset.field(),
        &record([utc_datetime().required_field("traded at")])
    );
}

#[test]
fn a_later_zoned_value_does_not_redecide_a_naive_column() {
    let message = read_root(
        &document(
            "<xsd:element sql:field=\"at\" name=\"at\" type=\"xsd:dateTime\"/>",
            "<row><at>2024-01-01T09:30:00</at></row><row><at>2024-01-01T09:30:00Z</at></row>",
        ),
        None,
    )
    .expect_err("the column is decided once, by its first value")
    .to_string();
    assert!(message.contains("$[1]"), "{message}");
    assert!(message.contains("at"), "{message}");
    assert!(
        message.contains("a naive datetime carries no zone"),
        "{message}"
    );
}

#[test]
fn an_empty_first_date_time_is_no_value_and_does_not_decide_the_column() {
    // `<at></at>` reads as null under a datetime column, so the first
    // *present* value is the second row's.
    let (rowset, back) = read_root(
        &document(
            "<xsd:element sql:field=\"at\" name=\"at\" type=\"xsd:dateTime\" minOccurs=\"0\"/>",
            "<row><at></at></row><row><at>2024-01-01T09:30:00Z</at></row>",
        ),
        None,
    )
    .map_err(|error| error.to_string())
    .expect("the rowset reads");
    let field = record([utc_datetime().nullable_field("at")]);
    assert_eq!(rowset.field(), &field);
    assert_eq!(
        back,
        rows(
            &field,
            [
                row([("at", Scalar::Null)]),
                row([("at", utc_at("09:30:00"))])
            ]
        )
    );
}

#[test]
fn a_declared_field_is_never_retyped_by_its_rows() {
    let declared = record([naive_datetime().required_field("at")]);
    let result = read_root(
        &document(
            "<xsd:element sql:field=\"at\" name=\"at\" type=\"xsd:dateTime\"/>",
            "<row><at>2024-01-01T09:30:00</at></row>",
        ),
        Some(&declared),
    )
    .expect("the rows read under the declared field");
    assert_eq!(result.0.field(), &declared);
    assert_eq!(
        result.1,
        rows(&declared, [row([("at", naive_at("09:30:00"))])])
    );
}

#[test]
fn a_utc_column_written_by_the_rowset_reads_back_as_utc() {
    let field = record([
        utc_datetime().required_field("instant"),
        naive_datetime().required_field("wall"),
    ]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let batch = rows(
        &field,
        [row([
            ("instant", utc_at("09:30:00")),
            ("wall", naive_at("09:30:00")),
        ])],
    );
    let (read, back) = read_root(&root_text(&rowset, vec![batch.clone()], true, true), None)
        .expect("the rowset reads back");
    assert_eq!(read.field(), &field);
    assert_eq!(back, batch);
}

#[test]
fn a_date_time_in_a_named_zone_reads_back_as_the_instant_it_is() {
    let paris = Timezone::from_str("Europe/Paris").expect("a named zone");
    let field = record([DataType::datetime64(TimeUnit::Microsecond, paris)
        .unwrap()
        .required_field("at")]);
    let rowset = Rowset::new(field.clone()).unwrap();
    let batch = rows(
        &field,
        [row([(
            "at",
            Scalar::datetime64(microseconds("09:30:00"), TimeUnit::Microsecond, paris).unwrap(),
        )])],
    );
    let text = root_text(&rowset, vec![batch], true, true);
    let (read, back) = read_root(&text, None)
        .map_err(|error| error.to_string())
        .unwrap_or_else(|error| panic!("{error}\n{text}"));
    let utc = record([utc_datetime().required_field("at")]);
    assert_eq!(read.field(), &utc, "{text}");
    assert_eq!(
        back,
        rows(&utc, [row([("at", utc_at("09:30:00"))])]),
        "{text}"
    );
}
