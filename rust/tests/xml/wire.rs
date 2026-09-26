//! `rust/src/xml/wire.rs`: the natural XML projection - what a value writes
//! as, and how a declared field reads the text a document leaves behind.

use yggdryl::text::{Formatting, Indent};
use yggdryl::xml;
use yggdryl::{
    DataType, DataTypeId, Field, Limits, Scalar, StructType, TimeUnit, Timezone,
    from_xml_scalar_with_field,
};

fn record<const N: usize>(entries: [(&str, Scalar); N]) -> Scalar {
    Scalar::from_struct(entries).unwrap()
}

fn document(name: &str, value: Scalar) -> Scalar {
    Scalar::from_struct([(name, value)]).unwrap()
}

fn written(value: &Scalar) -> String {
    xml::into_utf8(value).unwrap_or_else(|error| panic!("{value:?}: {error}"))
}

fn refused(value: &Scalar) -> String {
    let error = xml::into_utf8(value).expect_err("a refusal");
    assert_eq!(
        xml::validate_for_write(value)
            .expect_err("the same refusal")
            .to_string(),
        error.to_string(),
        "validation is the writer's own walk"
    );
    error.to_string()
}

#[test]
fn the_root_is_the_one_entry_of_a_record() {
    assert_eq!(written(&document("a", Scalar::from("1"))), "<a>1</a>");
    assert_eq!(written(&document("a", Scalar::Null)), "<a/>");
    assert_eq!(
        written(&document("a", record([]))),
        "<a/>",
        "an empty record is an empty element"
    );

    let message = refused(&Scalar::from("1"));
    assert!(
        message.contains("document root must be a record"),
        "{message}"
    );
    let message = refused(&record([("a", Scalar::from(1)), ("b", Scalar::from(2))]));
    assert!(message.contains("got several"), "{message}");
    let message = refused(&record([]));
    assert!(message.contains("got none"), "{message}");
    let message = refused(&document(
        "a",
        Scalar::from_sequence([Scalar::from(1), Scalar::from(2)]),
    ));
    assert!(message.contains("would repeat the root"), "{message}");
}

#[test]
fn entries_write_as_attributes_text_and_children_and_a_sequence_repeats() {
    let value = document(
        "order",
        record([
            ("@id", Scalar::from(7)),
            ("@when", Scalar::Null),
            ("#text", Scalar::from("note")),
            (
                "leg",
                Scalar::from_sequence([Scalar::from(1), Scalar::from(2)]),
            ),
            ("empty", Scalar::from_sequence([])),
            ("none", Scalar::Null),
            ("venue", record([("mic", Scalar::from("XPAR"))])),
        ]),
    );
    assert_eq!(
        written(&value),
        "<order id=\"7\">note<leg>1</leg><leg>2</leg><none/><venue><mic>XPAR</mic></venue></order>",
        "an absent attribute and an empty sequence write nothing"
    );
}

#[test]
fn what_xml_cannot_spell_is_refused_by_name() {
    let nested = document(
        "a",
        record([(
            "m",
            Scalar::from_sequence([Scalar::from_sequence([Scalar::from(1)])]),
        )]),
    );
    assert!(refused(&nested).contains("sequence inside a sequence"));

    let other_hash = document("a", record([("#comment", Scalar::from("x"))]));
    assert!(refused(&other_hash).contains("`#text` is the only `#` key"));

    let container_attribute = document("a", record([("@k", record([("x", Scalar::from(1))]))]));
    assert!(refused(&container_attribute).contains("attribute `k` must be a scalar"));

    let container_text = document(
        "a",
        record([("#text", Scalar::from_sequence([Scalar::from(1)]))]),
    );
    assert!(refused(&container_text).contains("expected a scalar for `#text`"));

    for name in ["1st", "with space", "", "a<b", "-x", "a\"b"] {
        let message = refused(&document(name, Scalar::from(1)));
        assert!(
            message.contains("expected an XML name"),
            "{name:?}: {message}"
        );
    }
    for name in ["a", "_a", "ns:a", "a-b.c_1", "é", "a:b:c"] {
        assert!(
            xml::into_utf8(&document(name, Scalar::from(1))).is_ok(),
            "{name:?}"
        );
    }

    let control = document("a", Scalar::from("bell\u{7}"));
    assert!(refused(&control).contains("U+0007"));

    let non_string_key = document(
        "a",
        Scalar::from_mapping([(Scalar::from(1), Scalar::from(2))]).unwrap(),
    );
    assert!(refused(&non_string_key).contains("names must be strings"));

    let deep = (0..200).fold(Scalar::from(0), |value, _| record([("d", value)]));
    assert!(refused(&document("a", deep)).contains("depth"));
}

#[test]
fn special_characters_are_escaped_where_they_would_change_the_reading() {
    let value = document(
        "a",
        record([
            ("@k", Scalar::from("q\"uote <tab\t> line\n & ret\r")),
            ("#text", Scalar::from("a < b && c > d\r\n\"ok\" 'fine' ]]>")),
        ]),
    );
    let encoded = written(&value);
    assert_eq!(
        encoded,
        "<a k=\"q&quot;uote &lt;tab&#9;&gt; line&#10; &amp; ret&#13;\">a &lt; b &amp;&amp; c &gt; d&#13;\n\"ok\" 'fine' ]]&gt;</a>"
    );
    assert_eq!(
        xml::from_utf8(&encoded).unwrap(),
        value,
        "and reads back exactly"
    );
}

#[test]
fn leaves_write_their_interoperable_spellings() {
    let value = document(
        "row",
        record([
            ("bool", Scalar::from(true)),
            ("int", Scalar::from(-12_i64)),
            ("big", Scalar::from(u128::MAX)),
            ("float", Scalar::from(413.75_f64)),
            ("whole", Scalar::from(1.0_f64)),
            ("nan", Scalar::from(f64::NAN)),
            ("inf", Scalar::from(f64::NEG_INFINITY)),
            ("decimal", Scalar::d128(1250, 2)),
            ("bytes", Scalar::from(vec![0_u8, 255])),
            ("date", Scalar::date32(19_876)),
            (
                "time",
                Scalar::time32(27_120_100, TimeUnit::Millisecond, Timezone::NAIVE).unwrap(),
            ),
            (
                "at",
                Scalar::datetime64(1_700_000_000, TimeUnit::Second, Timezone::UTC).unwrap(),
            ),
            (
                "naive",
                Scalar::datetime64(0, TimeUnit::Second, Timezone::NAIVE).unwrap(),
            ),
            (
                "span",
                DataType::interval(TimeUnit::YearMonth)
                    .unwrap()
                    .scalar(Scalar::from(14))
                    .unwrap(),
            ),
        ]),
    );
    let encoded = written(&value);
    for expected in [
        "<bool>true</bool>",
        "<int>-12</int>",
        "<big>340282366920938463463374607431768211455</big>",
        "<float>413.75</float>",
        "<whole>1.0</whole>",
        "<nan>NaN</nan>",
        "<inf>-INF</inf>",
        "<decimal>12.50</decimal>",
        "<bytes>AP8=</bytes>",
        "<date>2024-06-02</date>",
        "<time>07:32:00.100</time>",
        "<at>2023-11-14T22:13:20Z</at>",
        "<naive>1970-01-01T00:00:00</naive>",
        "<span>14</span>",
    ] {
        assert!(encoded.contains(expected), "{expected} in {encoded}");
    }
}

#[test]
fn an_interval_of_several_components_repeats_its_element() {
    let day_time = DataType::interval(TimeUnit::DayTime)
        .unwrap()
        .scalar(Scalar::from_sequence([
            Scalar::from(3),
            Scalar::from(4_000),
        ]))
        .unwrap();
    let value = document("row", record([("span", day_time)]));
    assert_eq!(
        written(&value),
        "<row><span>3</span><span>4000</span></row>"
    );

    let month_day_nano = DataType::interval(TimeUnit::MonthDayNano).unwrap();
    let field = StructType::from_fields([month_day_nano.clone().required_field("span")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let interval = month_day_nano
        .scalar(Scalar::from_sequence([
            Scalar::from(1),
            Scalar::from(2),
            Scalar::from(3),
        ]))
        .unwrap();
    let encoded = written(&document("row", record([("span", interval.clone())])));
    assert_eq!(
        encoded,
        "<row><span>1</span><span>2</span><span>3</span></row>"
    );
    let typed = from_xml_scalar_with_field(&encoded, &field).unwrap();
    assert_eq!(typed, Scalar::from_sequence([interval]));

    let at_root = refused(&document("span", day_time_value()));
    assert!(at_root.contains("several components"), "{at_root}");
}

fn day_time_value() -> Scalar {
    DataType::interval(TimeUnit::DayTime)
        .unwrap()
        .scalar(Scalar::from_sequence([
            Scalar::from(3),
            Scalar::from(4_000),
        ]))
        .unwrap()
}

#[test]
fn indentation_puts_each_element_on_its_line_except_beside_text() {
    let value = document(
        "row",
        record([
            ("id", Scalar::from(1)),
            (
                "tags",
                Scalar::from_sequence([Scalar::from("a"), Scalar::from("b")]),
            ),
            (
                "venue",
                record([("@mic", Scalar::from("XPAR")), ("rank", Scalar::from(2))]),
            ),
            (
                "p",
                record([
                    ("#text", Scalar::from("Hello !")),
                    ("b", Scalar::from("world")),
                ]),
            ),
            ("none", Scalar::Null),
        ]),
    );
    let compact = written(&value);
    assert!(!compact.contains('\n'));
    assert_eq!(
        xml::into_utf8_with_formatting(&value, Formatting::compact()).unwrap(),
        compact,
        "no indentation is the default, one line"
    );

    let pretty = xml::into_utf8_with_formatting(&value, Formatting::indented(2)).unwrap();
    assert_eq!(
        pretty,
        "<row>\n  <id>1</id>\n  <none/>\n  <p>Hello !<b>world</b></p>\n  <tags>a</tags>\n  <tags>b</tags>\n  <venue mic=\"XPAR\">\n    <rank>2</rank>\n  </venue>\n</row>"
    );
    assert_eq!(
        xml::from_utf8(&pretty).unwrap(),
        xml::from_utf8(&compact).unwrap(),
        "layout never changes the value"
    );

    let tabs = xml::into_utf8_with_formatting(&value, Formatting::new().with_indent(Indent::Tabs))
        .unwrap();
    assert!(tabs.contains("\n\t<id>1</id>"), "{tabs}");
    assert_eq!(
        xml::from_utf8(&tabs).unwrap(),
        xml::from_utf8(&compact).unwrap()
    );
}

fn typed_row() -> Field {
    StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("note"),
        DataType::decimal128(10, 2).unwrap().required_field("price"),
        DataType::serie(DataType::Int64.required_field("item")).required_field("sizes"),
        DataType::serie(DataType::utf8().required_field("tag")).nullable_field("tags"),
        StructType::from_fields([
            DataType::utf8().required_field("@mic"),
            DataType::Int32.required_field("rank"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("venue"),
        DataType::date32().required_field("day"),
        DataType::binary().required_field("payload"),
        DataType::Boolean.required_field("live"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row")
}

#[test]
fn a_field_types_the_text_a_document_leaves_behind() {
    let source = "<row><id> 7 </id><note> keep me </note><price>12.5</price>\
                  <sizes>100</sizes><tags>a</tags><tags>b</tags>\
                  <venue mic=\"XPAR\"><rank>2</rank></venue><day>2024-06-02</day>\
                  <payload>AP8=</payload><live>true</live></row>";
    let value = from_xml_scalar_with_field(source, &typed_row()).unwrap();
    let cells = value.as_sequence().unwrap();
    assert_eq!(
        cells[0],
        Scalar::from(7_i64),
        "text is trimmed under a number"
    );
    assert_eq!(
        cells[1],
        Scalar::from(" keep me "),
        "text under a text column is kept exactly"
    );
    assert_eq!(cells[2], Scalar::d128(1250, 2));
    assert_eq!(
        cells[3],
        Scalar::from_sequence([Scalar::from(100_i64)]),
        "one repeated element read once is one item"
    );
    assert_eq!(
        cells[4],
        Scalar::from_sequence([Scalar::from("a"), Scalar::from("b")])
    );
    assert_eq!(
        cells[5],
        Scalar::from_sequence([Scalar::from("XPAR"), Scalar::from(2_i32)]),
        "an attribute is a child named with its prefix"
    );

    let empty_tag = "<row><id>7</id><price>1</price><tags></tags><venue mic=\"XPAR\"><rank>2</rank></venue>\
                     <day>2024-06-02</day><payload>AP8=</payload><live>true</live></row>";
    let value = from_xml_scalar_with_field(empty_tag, &typed_row()).unwrap();
    assert_eq!(
        value.as_sequence().unwrap()[4],
        Scalar::from_sequence([Scalar::from("")]),
        "an element with an empty body under a sequence of text is one empty item"
    );
    assert_eq!(
        written(&document(
            "row",
            record([("tags", Scalar::from_sequence([Scalar::from("")]))])
        )),
        "<row><tags></tags></row>",
        "which is how the writer spells it"
    );
    assert_eq!(cells[6].id(), DataTypeId::Date32);
    assert_eq!(cells[7], Scalar::from(vec![0_u8, 255]));
    assert_eq!(cells[8], Scalar::from(true));
}

#[test]
fn absence_reads_as_the_field_says() {
    let source = "<row><id>7</id><price>1</price><venue mic=\"XPAR\"><rank>2</rank></venue>\
                  <day>2024-06-02</day><payload>AP8=</payload><live> false </live><note/></row>";
    let value = from_xml_scalar_with_field(source, &typed_row()).unwrap();
    let cells = value.as_sequence().unwrap();
    assert_eq!(cells[1], Scalar::Null, "an empty element is null");
    assert_eq!(
        cells[3],
        Scalar::from_sequence([]),
        "a repeated element occurring no time is the empty sequence"
    );
    assert_eq!(
        cells[4],
        Scalar::from_sequence([]),
        "under a nullable sequence too: zero occurrences is zero items"
    );
    assert_eq!(
        cells[8],
        Scalar::from(false),
        "a boolean is trimmed like any other non-text leaf"
    );

    let nil_sizes = "<row><id>7</id><price>1</price><sizes/><venue mic=\"XPAR\"><rank>2</rank></venue>\
                     <day>2024-06-02</day><payload>AP8=</payload><live>true</live></row>";
    let error = from_xml_scalar_with_field(nil_sizes, &typed_row()).unwrap_err();
    assert!(
        error.to_string().contains("sizes"),
        "an empty element under a required sequence is a null it refuses: {error}"
    );

    let blank_id = "<row><id>  </id><price>1</price><venue mic=\"XPAR\"><rank>2</rank></venue>\
                    <day>2024-06-02</day><payload>AP8=</payload><live>true</live></row>";
    let error = from_xml_scalar_with_field(blank_id, &typed_row()).unwrap_err();
    assert!(
        error.to_string().contains("id"),
        "blank text under a required number is null, which it refuses: {error}"
    );
}

#[test]
fn a_sequence_inside_a_sequence_travels_under_the_items_own_name() {
    let inner = DataType::serie(DataType::Int64.required_field("value")).required_field("item");
    let field = StructType::from_fields([DataType::serie(inner).required_field("matrix")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let source = "<row><matrix><value>1</value><value>2</value></matrix><matrix><value>3</value></matrix></row>";
    let value = from_xml_scalar_with_field(source, &field).unwrap();
    assert_eq!(
        value,
        Scalar::from_sequence([Scalar::from_sequence([
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(2_i64)]),
            Scalar::from_sequence([Scalar::from(3_i64)]),
        ])])
    );

    let with_empty = "<row><matrix></matrix><matrix><value>4</value></matrix></row>";
    let value = from_xml_scalar_with_field(with_empty, &field).unwrap();
    assert_eq!(
        value,
        Scalar::from_sequence([Scalar::from_sequence([
            Scalar::from_sequence([]),
            Scalar::from_sequence([Scalar::from(4_i64)]),
        ])]),
        "an item element with an empty body is an empty inner sequence"
    );

    let none = "<row></row>";
    let value = from_xml_scalar_with_field(none, &field).unwrap();
    assert_eq!(
        value,
        Scalar::from_sequence([Scalar::from_sequence([])]),
        "a root with an empty body is a record whose sequences occur no time"
    );

    // A record item whose one child shares the item field's name is a record
    // read as one: only an item that is itself a sequence holds its items as
    // children named after them.
    let record_item = StructType::from_fields([DataType::utf8().required_field("item")])
        .map(DataType::from)
        .unwrap()
        .required_field("item");
    let field = StructType::from_fields([DataType::serie(record_item).required_field("sizes")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let value =
        from_xml_scalar_with_field("<row><sizes><item>x</item></sizes></row>", &field).unwrap();
    assert_eq!(
        value,
        Scalar::from_sequence([Scalar::from_sequence([Scalar::from_sequence([
            Scalar::from("x")
        ])])])
    );
}

#[test]
fn a_variant_writes_as_the_value_it_holds() {
    let held = record([("id", Scalar::from(1))]);
    let variant = DataType::Variant.scalar(held.clone()).unwrap();
    assert_eq!(
        written(&document("row", variant.clone())),
        "<row><id>1</id></row>"
    );
    assert_eq!(written(&variant), written(&held));

    let indented = Formatting::indented(2);
    assert_eq!(
        xml::into_utf8_with_formatting(&document("row", variant), indented).unwrap(),
        xml::into_utf8_with_formatting(&document("row", held), indented).unwrap(),
        "the layout sees through the variant as the writer does"
    );
}

#[test]
fn empty_text_and_absence_write_apart_and_read_back_apart() {
    assert_eq!(written(&document("a", Scalar::from(""))), "<a></a>");
    assert_eq!(written(&document("a", Scalar::Null)), "<a/>");
    assert_eq!(written(&document("a", record([]))), "<a/>");
    assert_eq!(
        written(&document(
            "a",
            record([("tags", Scalar::from_sequence([]))])
        )),
        "<a></a>",
        "a record holding only sequences occurring no time is present and empty"
    );
    for value in [Scalar::from(""), Scalar::Null] {
        let document = document("a", value);
        assert_eq!(xml::from_utf8(&written(&document)).unwrap(), document);
    }

    let beside_an_attribute = document(
        "a",
        record([("@k", Scalar::from("v")), ("#text", Scalar::from(""))]),
    );
    assert_eq!(written(&beside_an_attribute), "<a k=\"v\"></a>");
    assert_eq!(
        xml::from_utf8("<a k=\"v\"></a>").unwrap(),
        beside_an_attribute
    );

    let one_null = document("a", record([("x", Scalar::from_sequence([Scalar::Null]))]));
    let message = refused(&one_null);
    assert!(message.contains("one null"), "{message}");
    assert!(message.contains("`x`"), "{message}");
}

#[test]
fn the_writer_counts_depth_as_the_parser_does() {
    // The writer recurses three frames per element, and a debug build's
    // frames at the ceiling outgrow the two mebibytes a test thread has; a
    // release build's are a fraction of that, and the ceiling is what keeps
    // them bounded at all.
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(writer_depth_parity)
        .expect("a thread")
        .join()
        .expect("the parity holds");
}

fn writer_depth_parity() {
    let nested = |depth: usize| {
        document(
            "d",
            (1..depth).fold(Scalar::from("x"), |value, _| record([("d", value)])),
        )
    };
    let limits = |depth: usize| Limits::new(depth, usize::MAX, usize::MAX, 1);
    let spelled = |depth: usize| format!("{}x{}", "<d>".repeat(depth), "</d>".repeat(depth));

    let at_limit = nested(128);
    assert_eq!(
        written(&at_limit),
        spelled(128),
        "the default limit is 128 elements deep"
    );
    assert!(xml::validate_for_write_with_limits(&at_limit, limits(128)).is_ok());
    assert_eq!(
        xml::from_utf8_with_limits(&spelled(128), limits(128)).unwrap(),
        at_limit,
        "what writes at a depth reads at the same limit"
    );

    let over = xml::validate_for_write_with_limits(&nested(129), limits(128)).unwrap_err();
    assert!(over.to_string().contains("depth"), "{over}");
    let over = xml::from_utf8_with_limits(&spelled(129), limits(128)).unwrap_err();
    assert!(over.to_string().contains("depth"), "{over}");

    let generous = Limits::new(usize::MAX, usize::MAX, usize::MAX, 1);
    assert!(xml::validate_for_write_with_limits(&nested(xml::MAX_PARSER_DEPTH), generous).is_ok());
    let ceiling = xml::validate_for_write_with_limits(&nested(xml::MAX_PARSER_DEPTH + 1), generous)
        .unwrap_err();
    assert!(
        ceiling.to_string().contains("hard limit"),
        "the writer refuses what the parser could never read back: {ceiling}"
    );
}

#[test]
fn a_temporal_with_no_iso_8601_spelling_is_refused_rather_than_written_as_a_count() {
    let message = refused(&document("d", Scalar::date64(1)));
    assert!(message.contains("no ISO 8601 spelling"), "{message}");
    assert!(message.contains("date64"), "{message}");
    assert!(message.contains("`d`"), "{message}");
    assert_eq!(
        written(&document("d", Scalar::date64(86_400_000))),
        "<d>1970-01-02</d>",
        "a whole day has one"
    );
}

#[test]
fn a_union_crosses_as_its_type_id_and_its_value_repeated_under_one_name() {
    let union = DataType::dense_union([
        DataType::Int64.required_field("n"),
        DataType::utf8().required_field("s"),
    ])
    .unwrap();
    let field = StructType::from_fields([union.clone().required_field("u")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let value = union
        .scalar(Scalar::from_sequence([
            Scalar::from(1_i8),
            Scalar::from("x"),
        ]))
        .unwrap();
    let encoded = written(&document("row", record([("u", value.clone())])));
    assert_eq!(encoded, "<row><u>1</u><u>x</u></row>");
    let typed = from_xml_scalar_with_field(&encoded, &field).unwrap();
    assert_eq!(
        typed,
        Scalar::from_sequence([value]),
        "the id crossed as digits and came back as the id"
    );
}
