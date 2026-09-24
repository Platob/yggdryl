//! `rust/src/expression/path.rs`: focused edge cases for the one path grammar.

use yggdryl::expression::Term;
use yggdryl::{DataType, Field, FieldPath, FieldSegment, Scalar, Serie, StructType};

fn parse(text: &str) -> FieldPath {
    FieldPath::from_str(text).expect("path parses")
}

fn render(text: &str) -> String {
    parse(text).to_string()
}

#[test]
fn a_bare_name_is_one_field_segment() {
    let path = parse("price");
    assert_eq!(path.len(), 1);
    assert_eq!(path.as_name(), Some("price"));
    assert_eq!(path.segments(), [FieldSegment::field("price")]);
}

#[test]
fn a_leading_dot_is_optional_and_later_dots_are_not() {
    assert_eq!(parse(".price"), parse("price"));
    assert_eq!(parse("order.line.price").len(), 3);
    assert!(FieldPath::from_str("order line").is_err());
}

#[test]
fn positions_are_bracketed_and_may_count_back() {
    assert_eq!(
        parse("legs[0].price").segments(),
        [
            FieldSegment::field("legs"),
            FieldSegment::index(0),
            FieldSegment::field("price"),
        ]
    );
    assert_eq!(
        parse("legs[-1]").segments(),
        [FieldSegment::field("legs"), FieldSegment::index(-1),]
    );
}

#[test]
fn a_quoted_key_is_a_key_and_never_a_position() {
    let numeric = parse("tags['7']");
    assert_eq!(
        numeric.segments().last().and_then(FieldSegment::as_index),
        None
    );
    assert_eq!(
        numeric.segments().last().and_then(FieldSegment::as_name),
        Some("7")
    );
    assert_eq!(
        parse("legs[7]")
            .segments()
            .last()
            .and_then(FieldSegment::as_index),
        Some(7)
    );
}

#[test]
fn a_name_carrying_a_dot_has_one_spelling_and_is_one_segment() {
    // The ambiguity the plain splitters could not resolve: this is one child
    // named `a.b`, not two levels.
    let path = parse("\"a.b\"");
    assert_eq!(path.len(), 1);
    assert_eq!(path.as_name(), Some("a.b"));
    assert_eq!(parse("a.b").len(), 2);
}

#[test]
fn quotes_double_to_mean_themselves() {
    assert_eq!(parse("\"say \"\"hi\"\"\"").as_name(), Some("say \"hi\""));
    assert_eq!(
        parse("tags['it''s']")
            .segments()
            .last()
            .and_then(FieldSegment::as_name),
        Some("it's")
    );
}

#[test]
fn rendering_round_trips_through_the_parser() {
    for text in [
        "price",
        "order.line.price",
        "legs[0].price",
        "legs[-1]",
        "tags['k']",
        "\"a.b\"",
        "\"say \"\"hi\"\"\"",
        "tags['it''s']",
        "_private.x9",
    ] {
        let once = parse(text);
        let rendered = once.to_string();
        assert_eq!(
            FieldPath::from_str(&rendered).expect("rendered path parses"),
            once,
            "{text} rendered as {rendered}"
        );
    }
}

#[test]
fn a_bare_name_renders_without_a_leading_dot() {
    assert_eq!(render("price"), "price");
    assert_eq!(render(".price"), "price");
    assert_eq!(render("order.line"), "order.line");
    assert_eq!(render("legs[0]"), "legs[0]");
}

#[test]
fn the_empty_path_is_the_root() {
    let root = parse("");
    assert!(root.is_root());
    assert!(root.is_empty());
    assert_eq!(root.to_string(), "");
    assert_eq!(root, FieldPath::root());
    assert_eq!(parse("   "), FieldPath::root());
}

#[test]
fn parents_strip_one_segment_at_a_time() {
    let path = parse("order.line[2].price");
    let parent = path.parent().expect("a four-segment path has a parent");
    assert_eq!(parent.to_string(), "order.line[2]");
    assert_eq!(
        parent.parent().map(|held| held.to_string()).as_deref(),
        Some("order.line")
    );
    assert!(FieldPath::root().parent().is_none());
}

#[test]
fn joining_adds_one_segment_without_reparsing() {
    let path = FieldPath::root()
        .join(FieldSegment::field("order"))
        .join(FieldSegment::index(1));
    assert_eq!(path.to_string(), "order[1]");
    assert_eq!(path, parse("order[1]"));
}

#[test]
fn malformed_paths_name_where_they_stopped() {
    for text in [
        "order.",
        "order[",
        "order[]",
        "order[1",
        "order['k",
        "\"unterminated",
        "order..price",
        "[",
        "order[1.5]",
        "order[-1.5]",
        "order[date32 '2024-01-01']",
    ] {
        let error = FieldPath::from_str(text).expect_err(&format!("{text} must be refused"));
        let rendered = error.to_string();
        assert!(
            rendered.contains("field path"),
            "{text} refused as {rendered}"
        );
    }
}

#[test]
fn a_position_wider_than_sixty_four_bits_is_refused() {
    let error = FieldPath::from_str("legs[99999999999999999999]").expect_err("too wide");
    assert!(error.to_string().contains("64 bits"), "{error}");
}

#[test]
fn segments_order_by_kind_then_by_value() {
    let mut segments = [
        FieldSegment::index(2),
        FieldSegment::field("b"),
        FieldSegment::key(Scalar::from("k")).expect("a text key"),
        FieldSegment::field("a"),
        FieldSegment::index(1),
    ];
    segments.sort();
    assert_eq!(
        segments,
        [
            FieldSegment::field("a"),
            FieldSegment::field("b"),
            FieldSegment::index(1),
            FieldSegment::index(2),
            FieldSegment::key(Scalar::from("k")).expect("a text key"),
        ]
    );
}

#[test]
fn equal_paths_hash_alike_whatever_built_them() {
    let parsed = parse("order[1]");
    let built = FieldPath::new([FieldSegment::field("order"), FieldSegment::index(1)]);
    assert_eq!(parsed, built);
    assert_eq!(parsed.stable_hash(), built.stable_hash());
}

#[test]
fn serde_round_trips_through_the_canonical_text() {
    let path = parse("order.line[0]['k']");
    let json = serde_json::to_string(&path).expect("serializes");
    assert_eq!(json, "\"order.line[0]['k']\"");
    let back: FieldPath = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back, path);
}

#[test]
fn whitespace_around_steps_is_ignored() {
    assert_eq!(parse(" order . line [ 0 ] "), parse("order.line[0]"));
}

#[test]
fn a_trailing_as_names_what_the_path_reached() {
    let path = parse("order.line[0].price as price");
    assert_eq!(path.len(), 4, "the alias is not a segment");
    assert_eq!(path.alias(), Some("price"));
    assert_eq!(path.column_name(), Some("price"));
    assert_eq!(path.to_string(), "order.line[0].price as price");
}

#[test]
fn the_keyword_is_read_in_any_case_and_the_alias_may_be_quoted() {
    assert_eq!(parse("price AS unit").alias(), Some("unit"));
    assert_eq!(parse("price As unit").alias(), Some("unit"));
    assert_eq!(parse("price as \"unit price\"").alias(), Some("unit price"));
    // Single quotes are accepted at intake and render as the canonical form.
    assert_eq!(parse("price as 'unit price'").alias(), Some("unit price"));
    assert_eq!(
        render("price as 'unit price'"),
        "price as \"unit price\"",
        "one canonical spelling comes back out"
    );
}

#[test]
fn without_an_alias_a_path_is_named_by_its_last_segment() {
    assert_eq!(parse("order.line.price").column_name(), Some("price"));
    assert_eq!(parse("order.line.price").alias(), None);
    // A position names nothing, so neither does a path ending on one.
    assert_eq!(parse("legs[0]").column_name(), None);
    assert_eq!(parse("legs['k']").column_name(), Some("k"));
}

#[test]
fn a_segment_may_still_be_named_as() {
    // The keyword is only read where a path already has something to alias,
    // and a name after a dot is a name.
    let path = parse("order.as");
    assert_eq!(path.len(), 2);
    assert_eq!(path.alias(), None);
    assert_eq!(path.column_name(), Some("as"));
    // And a name merely starting with those letters is one name.
    assert_eq!(parse("assets").column_name(), Some("assets"));
    assert_eq!(parse("assets").alias(), None);
}

#[test]
fn an_alias_survives_rendering_and_reparsing() {
    for text in [
        "price as unit",
        "order.line[0] as first",
        "tags['k'] as key",
        "\"a.b\" as ab",
        "price as \"unit price\"",
        "price as \"say \"\"hi\"\"\"",
    ] {
        let once = parse(text);
        let rendered = once.to_string();
        assert_eq!(
            FieldPath::from_str(&rendered).expect("rendered path parses"),
            once,
            "{text} rendered as {rendered}"
        );
        assert!(once.alias().is_some(), "{text} declares an alias");
    }
}

#[test]
fn a_malformed_alias_names_where_it_stopped() {
    for text in [
        "price as",
        "price as ",
        "price as \"unterminated",
        "price as one two",
        "price as [0]",
        "as name",
    ] {
        let error = FieldPath::from_str(text).expect_err(&format!("{text} must be refused"));
        assert!(
            error.to_string().contains("field path"),
            "{text} refused as {error}"
        );
    }
}

#[test]
fn an_alias_is_part_of_the_value() {
    let plain = parse("price");
    let aliased = parse("price as unit");
    assert_ne!(plain, aliased, "two paths naming differently are different");
    assert_ne!(plain.stable_hash(), aliased.stable_hash());
    assert_eq!(aliased, parse("price as unit"));
    assert_eq!(aliased.stable_hash(), parse("price as unit").stable_hash());
}

#[test]
fn an_alias_is_set_and_cleared_without_reparsing() {
    let path = parse("order.price")
        .try_with_alias("cost")
        .expect("a name to call it");
    assert_eq!(path.to_string(), "order.price as cost");

    let mut cleared = path.clone();
    cleared.set_alias(None).expect("clearing always works");
    assert_eq!(cleared.alias(), None);
    assert_eq!(cleared, parse("order.price"));
}

#[test]
fn an_empty_alias_and_one_on_the_root_are_both_refused() {
    let mut path = parse("order.price");
    let before = path.clone();
    assert!(path.set_alias(Some("")).is_err());
    assert_eq!(path, before, "a refusal leaves the path unchanged");

    let error = FieldPath::root()
        .try_with_alias("x")
        .expect_err("the root reaches what it is applied to");
    assert!(error.to_string().contains("root"), "{error}");
}

#[test]
fn walking_up_or_down_drops_the_alias() {
    let path = parse("order.line.price as cost");
    // An alias names what the whole path reached; a parent reaches something
    // else, and a longer path reaches something else again.
    assert_eq!(
        path.parent()
            .and_then(|held| held.alias().map(str::to_owned)),
        None
    );
    assert_eq!(path.join(FieldSegment::field("net")).alias(), None);
}

#[test]
fn an_alias_serde_round_trips_through_the_canonical_text() {
    let path = parse("order.line[0] as first");
    let json = serde_json::to_string(&path).expect("serializes");
    assert_eq!(json, "\"order.line[0] as first\"");
    assert_eq!(
        serde_json::from_str::<FieldPath>(&json).expect("deserializes"),
        path
    );
}

#[test]
fn a_run_is_bracketed_with_a_colon_and_either_end_is_optional() {
    assert_eq!(
        parse("legs[1:3]").segments(),
        [
            FieldSegment::field("legs"),
            FieldSegment::range(Some(1), Some(3)),
        ]
    );
    assert_eq!(
        parse("legs[:]").segments()[1],
        FieldSegment::range(None, None)
    );
    assert_eq!(
        parse("legs[-2:]").segments()[1],
        FieldSegment::range(Some(-2), None)
    );
    assert_eq!(render("legs[ 1 : 3 ]"), "legs[1:3]");
    assert_eq!(render("legs[:3]"), "legs[:3]");
    assert_eq!(render("legs[:]"), "legs[:]");
    assert!(FieldPath::from_str("legs[1:3:5]").is_err());
    assert!(FieldPath::from_str("legs[a:b]").is_err());
    // A run reaches no one child, so it names nothing.
    assert_eq!(parse("legs[1:3]").column_name(), None);
}

#[test]
fn a_reserved_word_reached_after_a_dot_renders_quoted() {
    let path = parse("order.as");
    assert_eq!(path.to_string(), "order.\"as\"");
    assert_eq!(
        FieldPath::from_str(&path.to_string()).expect("reparses"),
        path
    );
}

#[test]
fn a_step_types_one_level_and_reads_one_value() {
    let root = StructType::from_fields([
        DataType::serie(DataType::Int64.required_field("item")).required_field("legs"),
        StructType::from_fields([DataType::utf8().nullable_field("ccy")])
            .map(DataType::from)
            .unwrap()
            .required_field("trade"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row");
    let legs = FieldSegment::field("legs").apply_field(&root).unwrap();
    assert!(matches!(legs.dtype(), DataType::Serie(_)));
    let first = FieldSegment::index(0).apply_field(&legs).unwrap();
    assert_eq!(first.dtype(), &DataType::Int64);
    assert!(first.is_nullable(), "a position past the end reads as null");
    let run = FieldSegment::range(Some(1), None)
        .apply_field(&legs)
        .unwrap();
    assert_eq!(run.dtype(), legs.dtype());
    let ccy = FieldSegment::field("ccy")
        .apply_field(&FieldSegment::field("trade").apply_field(&root).unwrap())
        .unwrap();
    assert_eq!(ccy.dtype(), &DataType::utf8());
    assert!(FieldSegment::index(0).apply_field(&root).is_err());
    assert!(FieldSegment::field("nope").apply_field(&root).is_err());

    let row = Scalar::from_sequence([
        Scalar::from_sequence([Scalar::from(10_i64), Scalar::from(20_i64)]),
        Scalar::from_sequence([Scalar::from("EUR")]),
    ]);
    let held = FieldSegment::field("legs")
        .apply_scalar(&root, &row)
        .unwrap();
    assert_eq!(
        FieldSegment::index(-1).apply_scalar(&legs, &held).unwrap(),
        Scalar::from(20_i64)
    );
    assert_eq!(
        FieldSegment::index(5).apply_scalar(&legs, &held).unwrap(),
        Scalar::Null
    );
    assert_eq!(
        FieldSegment::range(Some(1), None)
            .apply_scalar(&legs, &held)
            .unwrap(),
        Scalar::from_sequence([Scalar::from(20_i64)])
    );
    let path = parse("trade.ccy");
    assert_eq!(path.apply_field(&root).unwrap().dtype(), &DataType::utf8());
    assert_eq!(path.apply_scalar(&root, &row).unwrap(), Scalar::from("EUR"));
    let _: &Field = &root;
}

// ---------------------------------------------------------------------------
// The predicate segment
// ---------------------------------------------------------------------------

/// A row holding one serie of structs, the shape a predicate keeps elements of.
fn legs_root() -> Field {
    StructType::from_fields([
        DataType::serie(
            StructType::from_fields([
                DataType::utf8().nullable_field("ccy"),
                DataType::Int64.nullable_field("size"),
                DataType::Boolean.nullable_field("active"),
            ])
            .map(DataType::from)
            .unwrap()
            .nullable_field("item"),
        )
        .nullable_field("legs"),
        DataType::serie(DataType::Int64.nullable_field("item")).nullable_field("xs"),
        DataType::utf8().nullable_field("ccy"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row")
}

fn leg(ccy: Option<&str>, size: Option<i64>, active: Option<bool>) -> Scalar {
    Scalar::from_sequence([
        ccy.map_or(Scalar::Null, Scalar::from),
        size.map_or(Scalar::Null, Scalar::from),
        active.map_or(Scalar::Null, Scalar::from),
    ])
}

#[test]
fn inside_brackets_only_a_whole_number_a_text_and_a_colon_are_not_a_predicate() {
    assert_eq!(parse("legs[1]").segments()[1], FieldSegment::index(1));
    assert_eq!(parse("legs[-1]").segments()[1], FieldSegment::index(-1));
    assert_eq!(
        parse("legs['k']").segments()[1],
        FieldSegment::key(Scalar::from("k")).unwrap()
    );
    assert_eq!(
        parse("legs[1:2]").segments()[1],
        FieldSegment::range(Some(1), Some(2))
    );
    for (text, predicate) in [
        ("legs[ccy = 'EUR']", "ccy = 'EUR'"),
        ("legs[active]", "active"),
        ("legs[true]", "true"),
        ("legs[null]", "null"),
        ("legs[not active and size > 1]", "not active and size > 1"),
        ("legs[1 = size]", "1 = size"),
        ("legs[-size < 0]", "-size < 0"),
        ("legs[1.5 < size]", "1.5 < size"),
        ("legs[ccy in ('EUR', 'USD')]", "ccy in ('EUR', 'USD')"),
        ("legs[tags[0] = 'x']", "tags[0] = 'x'"),
        ("legs[notes[v > 1][0].k = 'b']", "notes[v > 1][0].k = 'b'"),
    ] {
        let path = parse(text);
        let held = path
            .last()
            .and_then(FieldSegment::as_predicate)
            .unwrap_or_else(|| panic!("{text} ends in a predicate"));
        assert_eq!(held, &predicate.parse::<Term>().unwrap(), "{text}");
        assert_eq!(path.last().and_then(FieldSegment::as_name), None);
        assert_eq!(path.last().and_then(FieldSegment::as_index), None);
    }
    // A predicate reaches no one child, so a path ending in one names nothing
    // and an alias is how it is called.
    assert_eq!(parse("legs[active]").column_name(), None);
    assert_eq!(parse("legs[active] as live").column_name(), Some("live"));
}

#[test]
fn a_predicate_segment_renders_and_reparses_in_both_grammars() {
    for text in [
        "legs[ccy = 'EUR']",
        "legs[ccy = 'EUR'][0].price",
        "legs[active]",
        "legs[not active and size > 1]",
        "legs[ccy = 'EUR' or ccy = 'USD'][-1]",
        "legs[size between 1 and 3][1:]",
        "legs[notes[v > 1][0].k = 'b'].ccy",
        "legs[ccy = 'EUR'][size >= 3]",
        "legs[ccy = :ccy]",
        "legs[ccy = 'EUR'] as eur",
        "legs[cast(size as float64) > 1.5]",
    ] {
        let path = parse(text);
        let rendered = path.to_string();
        assert_eq!(rendered, text, "one canonical spelling");
        assert_eq!(FieldPath::from_str(&rendered).unwrap(), path);
        // The term grammar reads the same path as its one leaf.
        let leaf: Term = text.trim_end_matches(" as eur").parse().unwrap();
        assert_eq!(leaf.as_path(), Some(path.segments()));
        assert_eq!(leaf.to_string().parse::<Term>().unwrap(), leaf);
    }
    let built = FieldPath::new([
        FieldSegment::field("legs"),
        FieldSegment::filter("ccy = 'EUR'".parse().unwrap()),
        FieldSegment::index(0),
    ]);
    assert_eq!(built, parse("legs[ccy = 'EUR'][0]"));
    assert_eq!(
        built.stable_hash(),
        parse("legs[ccy = 'EUR'][0]").stable_hash()
    );
    let json = serde_json::to_string(&built).unwrap();
    assert_eq!(json, "\"legs[ccy = 'EUR'][0]\"");
    assert_eq!(serde_json::from_str::<FieldPath>(&json).unwrap(), built);
}

#[test]
fn a_predicate_segment_sorts_after_every_other_kind() {
    let mut segments = [
        FieldSegment::filter("b".parse().unwrap()),
        FieldSegment::range(None, None),
        FieldSegment::filter("a".parse().unwrap()),
        FieldSegment::index(0),
    ];
    segments.sort();
    assert_eq!(
        segments,
        [
            FieldSegment::index(0),
            FieldSegment::range(None, None),
            FieldSegment::filter("a".parse().unwrap()),
            FieldSegment::filter("b".parse().unwrap()),
        ]
    );
}

#[test]
fn a_predicate_segment_types_as_the_serie_it_keeps_elements_of() {
    let root = legs_root();
    let legs = FieldSegment::field("legs").apply_field(&root).unwrap();
    let kept = FieldSegment::filter("ccy = 'EUR'".parse().unwrap())
        .apply_field(&legs)
        .unwrap();
    assert_eq!(kept.dtype(), legs.dtype(), "the same item type");
    assert!(
        kept.is_nullable(),
        "the kept elements of a null list are null"
    );
    assert_eq!(
        parse("legs[active][0].size")
            .apply_field(&root)
            .unwrap()
            .dtype(),
        &DataType::Int64
    );
    // The names inside resolve against the element struct, never the row: the
    // row has a `ccy` column and the element does not have `xs`.
    let unknown = parse("legs[xs = 1]").apply_field(&root).unwrap_err();
    assert!(
        unknown.to_string().contains("ccy, size, active"),
        "{unknown}"
    );
    let not_boolean = parse("legs[size]").apply_field(&root).unwrap_err();
    assert!(
        not_boolean.to_string().contains("boolean predicate"),
        "{not_boolean}"
    );
    let not_a_list = parse("ccy[size = 1]").apply_field(&root).unwrap_err();
    assert!(
        not_a_list.to_string().contains("serie of structs"),
        "{not_a_list}"
    );
    let not_structs = parse("xs[item > 1]").apply_field(&root).unwrap_err();
    assert!(
        not_structs.to_string().contains("serie of structs"),
        "{not_structs}"
    );
}

#[test]
fn a_predicate_segment_reads_the_elements_a_row_holds() {
    let root = legs_root();
    let row = |legs: Scalar| Scalar::from_sequence([legs, Scalar::Null, Scalar::from("EUR")]);
    let full = row(Scalar::from_sequence([
        leg(Some("EUR"), Some(1), Some(true)),
        leg(Some("USD"), Some(2), Some(false)),
        Scalar::Null,
        leg(Some("EUR"), None, None),
        leg(Some("GBP"), Some(4), Some(true)),
    ]));
    let read = |text: &str, row: &Scalar| parse(text).apply_scalar(&root, row).unwrap();
    assert_eq!(
        read("legs[ccy = 'EUR']", &full),
        Scalar::from_sequence([
            leg(Some("EUR"), Some(1), Some(true)),
            leg(Some("EUR"), None, None),
        ]),
        "a null element is dropped, the rest keep their order"
    );
    assert_eq!(
        read("legs[active]", &full),
        Scalar::from_sequence([
            leg(Some("EUR"), Some(1), Some(true)),
            leg(Some("GBP"), Some(4), Some(true)),
        ]),
        "false and unknown both drop an element"
    );
    assert_eq!(
        read("legs[size > 1][0].ccy", &full),
        Scalar::from("USD"),
        "a position and a name compose after a predicate"
    );
    assert_eq!(
        read("legs[ccy = 'EUR'][size is null][-1].ccy", &full),
        Scalar::from("EUR"),
        "predicates chain"
    );
    assert_eq!(
        read("legs[ccy = 'JPY']", &full),
        Scalar::from_sequence([]),
        "no match is the empty list, not null"
    );
    assert_eq!(read("legs[ccy = 'JPY'][0]", &full), Scalar::Null);
    assert_eq!(
        read("legs[true]", &row(Scalar::Null)),
        Scalar::Null,
        "a null list stays null"
    );
    assert_eq!(
        read("legs[true]", &row(Scalar::from_sequence([]))),
        Scalar::from_sequence([])
    );
    let segment = FieldSegment::filter("size >= 2".parse().unwrap());
    let legs = FieldSegment::field("legs").apply_field(&root).unwrap();
    assert_eq!(
        segment
            .apply_scalar(&legs, &full.as_sequence().unwrap()[0])
            .unwrap(),
        Scalar::from_sequence([
            leg(Some("USD"), Some(2), Some(false)),
            leg(Some("GBP"), Some(4), Some(true)),
        ])
    );
    let refused = segment
        .apply_scalar(&root, &full)
        .expect_err("a row is no serie of structs");
    assert!(
        refused.to_string().contains("serie of structs"),
        "{refused}"
    );
}

mod grammar {

    use yggdryl::expression::Term;
    use yggdryl::{DataType, Field, Scalar, StructType, TimeUnit, Timezone};

    // ---------------------------------------------------------------------------
    // The shared fixture
    // ---------------------------------------------------------------------------

    /// A schema that covers one column of every family a comparison can meet.
    fn rows_schema() -> Field {
        Field::new(
            "rows",
            StructType::from_fields([
                Field::new("i", DataType::Int64, true),
                Field::new("f", DataType::Float64, true),
                Field::new("d", DataType::decimal128(9, 2).unwrap(), true),
                Field::new("s", DataType::utf8(), true),
                Field::new("b", DataType::Boolean, true),
                Field::new(
                    "t",
                    DataType::DateTime64 {
                        unit: TimeUnit::Microsecond,
                        timezone: Timezone::UTC,
                    },
                    true,
                ),
                Field::new("n", DataType::Int32, true).with_partition(true),
                Field::new(
                    "nested",
                    DataType::from(
                        StructType::from_fields([Field::new("leg", DataType::utf8(), true)])
                            .unwrap(),
                    ),
                    true,
                ),
                // Temporal text, so a cast into and out of a temporal is one of
                // the pairs the two tiers are compared on.
                Field::new("clock", DataType::utf8(), true),
                // A serie, so a position and a run are compared on both tiers.
                Field::new(
                    "xs",
                    DataType::serie(DataType::Int64.nullable_field("item")),
                    true,
                ),
                // A serie of structs holding a serie of structs, so a predicate
                // segment and one nested in another are compared on both tiers.
                Field::new("legs", DataType::serie(leg_field()), true),
            ])
            .map(DataType::from)
            .unwrap(),
            false,
        )
    }

    /// One leg: a currency, a size, and notes that are themselves a serie of
    /// structs.
    fn leg_field() -> Field {
        StructType::from_fields([
            DataType::utf8().nullable_field("ccy"),
            DataType::Int64.nullable_field("size"),
            DataType::serie(
                StructType::from_fields([
                    DataType::utf8().nullable_field("k"),
                    DataType::Int64.nullable_field("v"),
                ])
                .map(DataType::from)
                .unwrap()
                .nullable_field("item"),
            )
            .nullable_field("notes"),
        ])
        .map(DataType::from)
        .unwrap()
        .nullable_field("item")
    }

    fn leg(ccy: Option<&str>, size: Option<i64>, notes: Option<&[(&str, i64)]>) -> Scalar {
        Scalar::from_sequence([
            ccy.map_or(Scalar::Null, Scalar::from),
            size.map_or(Scalar::Null, Scalar::from),
            notes.map_or(Scalar::Null, |notes| {
                Scalar::from_sequence(
                    notes
                        .iter()
                        .map(|(k, v)| Scalar::from_sequence([Scalar::from(*k), Scalar::from(*v)])),
                )
            }),
        ])
    }

    /// Rows chosen so every operator meets a null, a `nan`, and a boundary.
    fn rows() -> Vec<Scalar> {
        let stamp =
            |micros: i64| Scalar::datetime64(micros, TimeUnit::Microsecond, Timezone::UTC).unwrap();
        let nested =
            |leg: Option<&str>| Scalar::from_sequence([leg.map_or(Scalar::Null, Scalar::from)]);
        let serie =
            |items: &[i64]| Scalar::from_sequence(items.iter().map(|item| Scalar::from(*item)));
        vec![
            Scalar::from_sequence([
                Scalar::from(1),
                Scalar::from(1.5_f64),
                Scalar::d128(150, 2),
                Scalar::from("alpha"),
                Scalar::from(true),
                stamp(1_700_000_000_000_000),
                Scalar::from(2024),
                nested(Some("EUR")),
                Scalar::from("10:23:45"),
                serie(&[1, 2, 3]),
                Scalar::from_sequence([
                    leg(Some("EUR"), Some(1), Some(&[("a", 1), ("b", 2)])),
                    leg(Some("USD"), Some(2), Some(&[])),
                    leg(Some("EUR"), Some(3), None),
                ]),
            ]),
            Scalar::from_sequence([
                Scalar::from(-3),
                Scalar::from(f64::NAN),
                Scalar::d128(-25, 2),
                Scalar::from("beta"),
                Scalar::from(false),
                stamp(0),
                Scalar::from(2024),
                nested(None),
                Scalar::from("25:30:00"),
                serie(&[]),
                // An empty serie keeps nothing and is not null.
                Scalar::from_sequence([]),
            ]),
            Scalar::from_sequence([
                Scalar::Null,
                Scalar::Null,
                Scalar::Null,
                Scalar::Null,
                Scalar::Null,
                Scalar::Null,
                Scalar::from(2024),
                Scalar::Null,
                Scalar::Null,
                Scalar::Null,
                // A null serie stays null through every predicate.
                Scalar::Null,
            ]),
            Scalar::from_sequence([
                Scalar::from(100),
                Scalar::from(f64::INFINITY),
                Scalar::d128(10_000, 2),
                Scalar::from("Alpha"),
                Scalar::Null,
                stamp(-1_000_000),
                Scalar::from(2023),
                nested(Some("USD")),
                Scalar::from("99:59:59"),
                serie(&[7]),
                // A null element is dropped; a null size makes a size test unknown.
                Scalar::from_sequence([Scalar::Null, leg(Some("EUR"), None, Some(&[("a", 5)]))]),
            ]),
            Scalar::from_sequence([
                Scalar::from(0),
                Scalar::from(0.0_f64),
                Scalar::d128(0, 2),
                Scalar::from(""),
                Scalar::from(true),
                stamp(1_700_000_000_000_001),
                Scalar::from(2025),
                nested(Some("eur")),
                Scalar::from("00:00:00.500"),
                serie(&[0, -1]),
                Scalar::from_sequence([
                    leg(Some("eur"), Some(10), Some(&[("c", 3)])),
                    leg(Some("GBP"), Some(0), Some(&[("z", 0)])),
                ]),
            ]),
        ]
    }

    fn batch_of(schema: &Field, rows: &[Scalar]) -> arrow_array::RecordBatch {
        let arrow_schema = schema.clone().into_arrow_schema().unwrap();
        let columns = schema
            .fields()
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let values: Vec<Scalar> = rows
                    .iter()
                    .map(|row| row.as_sequence().unwrap()[index].clone())
                    .collect();
                yggdryl::Serie::from_scalars(field.clone(), values)
                    .unwrap()
                    .require_arrow_array()
                    .unwrap()
            })
            .collect();
        arrow_array::RecordBatch::try_new(arrow_schema, columns).unwrap()
    }

    // ---------------------------------------------------------------------------
    // The predicate segment
    // ---------------------------------------------------------------------------

    #[test]
    fn a_predicate_segment_keeps_the_elements_the_grammar_says() {
        let schema = rows_schema();
        let rows = rows();
        let eur = "legs[ccy = 'EUR']".parse::<Term>().unwrap();
        assert_eq!(
            eur.columns(),
            vec!["legs".to_owned()],
            "the row column, not the element's"
        );
        let bound = eur.bind(&schema).unwrap();
        assert_eq!(bound.field().dtype(), schema.fields()[10].dtype());
        assert!(bound.field().is_nullable());
        assert_eq!(bound.column_names(), vec!["legs".to_owned()]);
        assert_eq!(
            bound.eval(&rows[0]).unwrap(),
            Scalar::from_sequence([
                leg(Some("EUR"), Some(1), Some(&[("a", 1), ("b", 2)])),
                leg(Some("EUR"), Some(3), None),
            ])
        );
        assert_eq!(bound.eval(&rows[1]).unwrap(), Scalar::from_sequence([]));
        assert_eq!(
            bound.eval(&rows[2]).unwrap(),
            Scalar::Null,
            "a null list stays null"
        );
        assert_eq!(
            bound.eval(&rows[3]).unwrap(),
            Scalar::from_sequence([leg(Some("EUR"), None, Some(&[("a", 5)]))]),
            "a null element is dropped"
        );
        assert_eq!(
            bound.eval(&rows[4]).unwrap(),
            Scalar::from_sequence([]),
            "text compares exactly"
        );

        let first = "legs[ccy = 'EUR'][0].size"
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        assert_eq!(first.field().dtype(), &DataType::Int64);
        assert_eq!(first.eval(&rows[0]).unwrap(), Scalar::from(1_i64));
        assert_eq!(first.eval(&rows[1]).unwrap(), Scalar::Null);
        assert_eq!(first.eval(&rows[3]).unwrap(), Scalar::Null);

        // A predicate over the element's own serie of structs, chained.
        let nested = "legs[notes[v > 1][0].k = 'b'][0].ccy"
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        assert_eq!(nested.eval(&rows[0]).unwrap(), Scalar::from("EUR"));
        assert_eq!(nested.eval(&rows[3]).unwrap(), Scalar::Null);
    }

    #[test]
    fn a_predicate_segment_over_a_sliced_large_serie_matches_the_row_tier() {
        let schema = Field::new(
            "rows",
            StructType::from_fields([Field::new("legs", DataType::large_serie(leg_field()), true)])
                .map(DataType::from)
                .unwrap(),
            false,
        );
        let rows: Vec<Scalar> = rows()
            .iter()
            .map(|row| Scalar::from_sequence([row.as_sequence().unwrap()[10].clone()]))
            .collect();
        let batch = batch_of(&schema, &rows).slice(1, 3);
        let bound = "legs[ccy = 'EUR'][size is null or size > 1]"
            .parse::<Term>()
            .unwrap()
            .bind(&schema)
            .unwrap();
        let column = bound.evaluate(&batch).unwrap();
        assert_eq!(column.len(), 3);
        let field = bound.field().clone().with_nullable(true);
        assert_eq!(
            column.data_type(),
            field.as_arrow_field_ref().unwrap().data_type()
        );
        for (position, row) in rows[1..4].iter().enumerate() {
            let held = yggdryl::Serie::from_arrow_array(
                Some(&field),
                column.slice(position, 1),
                yggdryl::ArrowCastOptions::default(),
            )
            .unwrap()
            .scalar(0)
            .unwrap();
            assert_eq!(bound.eval(row).unwrap(), held, "row {position}");
        }
    }

    #[test]
    fn a_predicate_segment_is_refused_where_it_cannot_keep_elements() {
        let schema = rows_schema();
        for (text, expected) in [
            ("i[x = 1]", "serie of structs"),
            ("xs[item > 1]", "serie of structs"),
            ("legs[size]", "boolean predicate"),
            ("legs[nope = 1]", "ccy, size, notes"),
            ("legs[i = 1]", "ccy, size, notes"),
        ] {
            let error = text.parse::<Term>().unwrap().bind(&schema).expect_err(text);
            assert!(error.to_string().contains(expected), "{text}: {error}");
            assert!(
                text.parse::<Term>().unwrap().field(&schema).is_err(),
                "{text} types the same way"
            );
        }
        // A computed value has no column to keep elements of, and the parser
        // says so at the bracket.
        for text in [
            "lower(s)[x = 1]",
            "[1, 2][x = 1]",
            "slice(legs, 0, 1)[ccy = 'EUR']",
        ] {
            let error = text.parse::<Term>().expect_err(text);
            assert!(
                error.to_string().contains("computed value"),
                "{text}: {error}"
            );
        }
        let refused = Term::call(yggdryl::expression::Function::Lower, [Term::column("s")])
            .filter_elements("x = 1".parse().unwrap())
            .expect_err("a computed value");
        assert!(refused.to_string().contains("computed value"), "{refused}");
        assert_eq!(
            Term::column("legs")
                .filter_elements("ccy = 'EUR'".parse().unwrap())
                .unwrap()
                .to_string(),
            "legs[ccy = 'EUR']"
        );
    }

    #[test]
    fn a_predicate_step_on_a_computed_value_has_no_term_to_build() {
        let refused = Term::call(yggdryl::expression::Function::Lower, [Term::column("s")])
            .path([yggdryl::FieldSegment::filter("x = 1".parse().unwrap())])
            .expect_err("a computed value");
        assert!(refused.to_string().contains("computed value"), "{refused}");
        // Every other step reads a computed value through the call that reads it.
        let read = Term::call(yggdryl::expression::Function::Lower, [Term::column("s")])
            .path([
                yggdryl::FieldSegment::field("k"),
                yggdryl::FieldSegment::index(0),
            ])
            .unwrap();
        assert_eq!(read.to_string(), "get(get(lower(s), 'k'), 0)");
    }

    #[test]
    fn a_predicate_segment_is_walked_like_any_other_node() {
        let term: Term = "legs[ccy = 'EUR' or ccy = 'USD'][qty > :floor]"
            .parse()
            .unwrap();
        assert_eq!(term.columns(), vec!["legs".to_owned()]);
        assert_eq!(term.parameters(), vec!["floor".to_owned()]);
        // A path, two predicates, and what they hold: the budget counts inside.
        assert_eq!("legs[ccy = 'EUR']".parse::<Term>().unwrap().node_count(), 4);
        assert_eq!("legs[ccy = 'EUR']".parse::<Term>().unwrap().depth(), 3);
        assert_eq!(
            "legs[notes[v > 1][0].k = 'b']"
                .parse::<Term>()
                .unwrap()
                .depth(),
            5
        );
        assert_eq!(
            term.simplify().to_string(),
            "legs[ccy in ('EUR', 'USD')][qty > :floor]",
            "simplification reaches into a predicate"
        );
        // Nesting predicates past the limit is refused, never a crash.
        let deep = format!("{}x{}", "a[".repeat(40), "]".repeat(40));
        let error = deep.parse::<Term>().expect_err("past the limit");
        assert!(error.to_string().contains("hard limit"), "{error}");
        let document = term.clone().into_json().unwrap();
        assert!(document.contains("\"where\""), "{document}");
        assert_eq!(Term::from_json(&document).unwrap(), term);
    }
}

mod nested {
    use yggdryl::{DataType, Error, Field, FieldPath, StructType};

    #[test]
    fn a_path_uses_the_shared_grammar_for_routes_and_literal_names() {
        let row = StructType::from_fields([
            StructType::from_fields([DataType::Float64.required_field("price")])
                .map(DataType::from)
                .unwrap()
                .required_field("line"),
            DataType::Int64.required_field("a.b"),
            DataType::Boolean.required_field("literal.name"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

        // A route through the graph.
        assert_eq!(row.field_by_path("line.price").unwrap().name(), "price");

        // A dot is a route boundary. A literal dot in one name is quoted through
        // the shared grammar, as is the equivalent text-key spelling.
        assert!(row.get_field_by_path("a.b").is_none());
        assert_eq!(
            row.field_by_path(r#""a.b""#).unwrap().dtype(),
            &DataType::Int64
        );
        assert_eq!(
            row.field_by_path("['literal.name']").unwrap().dtype(),
            &DataType::Boolean
        );
        assert_eq!(row[r#""a.b""#].dtype(), &DataType::Int64);

        // The route names no root `a`, and the literal child carries no `c`.
        assert!(row.get_field_by_path("a.b.c").is_none());

        // Quoting keeps the literal name one segment before the route continues.
        let deep = StructType::from_fields([StructType::from_fields([
            DataType::utf8().required_field("c")
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("a.b")])
        .map(DataType::from)
        .unwrap()
        .required_field("deep");
        assert_eq!(deep.field_by_path(r#""a.b".c"#).unwrap().name(), "c");

        // The same grammar addresses literal names for mutation. The stored name
        // is the segment's value, never its quotes or the replacement's old name.
        let mut changed = row.clone();
        changed
            .set_field_by_path(r#""a.b""#, DataType::utf8().required_field("replacement"))
            .unwrap();
        assert_eq!(changed[r#""a.b""#].dtype(), &DataType::utf8());
        let removed = changed.remove_field_by_path(r#""a.b""#).unwrap();
        assert_eq!(removed.name(), "a.b");
        assert!(changed.get_field_by_path(r#""a.b""#).is_none());

        // A path naming nothing reports the children that do exist.
        let message = row.field_by_path("missing").unwrap_err().to_string();
        assert!(message.contains("line"), "{message}");
    }

    #[test]
    fn a_serie_is_transparent_to_a_dotted_path_when_reading() {
        let item = StructType::from_fields([
            DataType::Float64.required_field("price"),
            StructType::from_fields([DataType::utf8().required_field("id")])
                .map(DataType::from)
                .unwrap()
                .required_field("party"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("item");
        let orders =
            StructType::from_fields([DataType::serie(item.clone()).nullable_field("orders")])
                .map(DataType::from)
                .unwrap()
                .required_field("row");

        // The item is a step the path need not spell, and both spellings agree.
        assert_eq!(
            orders.field_by_path("orders.price").unwrap().name(),
            "price"
        );
        assert_eq!(
            orders.field_by_path("orders.item.price").unwrap().name(),
            "price"
        );
        assert_eq!(
            orders.field_by_path("orders.party.id").unwrap().name(),
            "id"
        );
        assert_eq!(orders["orders"]["price"].name(), "price");
        assert_eq!(
            orders.get_field("orders.price"),
            orders.get_field("orders.item.price")
        );

        // The item's own name still wins outright.
        assert_eq!(orders.field_by_path("orders.item").unwrap().name(), "item");
        assert_eq!(
            orders.field_by_path("orders[0].price").unwrap().name(),
            "price"
        );
        assert_eq!(
            orders.field_by_path("orders[-1].price").unwrap().name(),
            "price"
        );

        // A path that resolves through no child reports the path that failed.
        let message = orders
            .field_by_path("orders.quantity")
            .unwrap_err()
            .to_string();
        assert!(message.contains("orders.quantity"), "{message}");
        assert!(orders.get_field_by_path("orders.quantity").is_none());

        // Every serie layout reads the same way; a map keeps its entries by name.
        let leaf = StructType::from_fields([DataType::Int64.required_field("value")])
            .map(DataType::from)
            .unwrap()
            .required_field("item");
        for layout in [
            DataType::serie(leaf.clone()),
            DataType::large_serie(leaf.clone()),
            DataType::serie_view(leaf.clone()),
            DataType::large_serie_view(leaf.clone()),
            DataType::fixed_size_serie(leaf.clone(), 2).unwrap(),
        ] {
            assert_eq!(
                layout.get_field_by_path("value").map(Field::name),
                Some("value"),
                "{layout}"
            );
            assert_eq!(
                layout.get_field_by_path("item.value").map(Field::name),
                Some("value"),
                "{layout}"
            );
        }
        let map = DataType::map_of(DataType::utf8(), DataType::Int64, false).unwrap();
        assert!(map.get_field_by_path("value").is_none());
        assert_eq!(
            map.get_field_by_path("entries.value").map(Field::name),
            Some("value")
        );

        // A write is not transparent: it addresses the item by its own name, and
        // a serie never grows a second child.
        let mut written = orders.clone();
        written
            .set_field_by_path(
                "orders.item.price",
                DataType::Float32.required_field("price"),
            )
            .unwrap();
        assert_eq!(written["orders"]["price"].dtype(), &DataType::Float32);
        assert_eq!(
            written
                .remove_field_by_path("orders.item.party")
                .unwrap()
                .name(),
            "party"
        );
        assert_eq!(written["orders"].dtype().field_len(), 1);
        assert_eq!(written["orders"]["item"].field_len(), 1);
    }

    #[test]
    fn schema_path_refusals_are_located_and_mutations_are_atomic() {
        let item = StructType::from_fields([DataType::Float64.required_field("price")])
            .map(DataType::from)
            .unwrap()
            .required_field("item");
        let row = StructType::from_fields([
            StructType::from_fields([DataType::Float64.required_field("price")])
                .map(DataType::from)
                .unwrap()
                .required_field("line"),
            DataType::Int64.required_field("id"),
            DataType::serie(item).nullable_field("orders"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

        let malformed = row.field_by_path("line.").unwrap_err();
        assert!(
            matches!(
                &malformed,
                Error::Parse {
                    target: "field path",
                    position: 5,
                    ..
                }
            ),
            "{malformed}"
        );

        // Boolean and null segments parse as predicates; a decimal is refused at
        // the parser boundary because only a whole number can select a serie item.
        for path in ["orders[true].price", "orders[null].price"] {
            assert!(FieldPath::from_str(path).is_ok(), "{path}");
        }
        assert!(matches!(
            FieldPath::from_str("orders[1.5].price"),
            Err(Error::Parse {
                target: "field path",
                ..
            })
        ));

        // These parse as selectors or refuse at the same boundary, but none name
        // one borrowed schema child.
        for path in [
            "line.price as px",
            "orders[0:1].price",
            "orders[price > 0].price",
            "orders[true].price",
            "orders[null].price",
            "orders[1.5].price",
        ] {
            assert!(row.get_field_by_path(path).is_none(), "{path}");
            assert!(row.field_by_path(path).is_err(), "{path}");
        }

        // A missing intermediate, a scalar intermediate, malformed syntax and a
        // computed selection never become a new dotted child. Both mutations
        // finish their validation before committing any rebuilt parent.
        for path in [
            "missing.price",
            "line.missing.price",
            "id.price",
            "line.",
            "line.price as px",
            "orders[0:1].price",
            "orders[price > 0].price",
            "orders[true].price",
            "orders[null].price",
            "orders[1.5].price",
        ] {
            let mut set = row.clone();
            assert!(
                set.set_field_by_path(path, DataType::utf8().required_field("replacement"))
                    .is_err(),
                "set {path}"
            );
            assert_eq!(set, row, "set {path} changed the schema");

            let mut removed = row.clone();
            assert!(removed.remove_field_by_path(path).is_err(), "remove {path}");
            assert_eq!(removed, row, "remove {path} changed the schema");
        }
    }

    #[test]
    fn one_key_reaches_a_child_by_position_or_by_path() {
        let row = StructType::from_fields([StructType::from_fields([
            DataType::Float64.required_field("price")
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("line")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

        // The same call, whichever spelling the caller holds.
        assert_eq!(row.field(0).unwrap().name(), "line");
        assert_eq!(row.field("line").unwrap().name(), "line");
        assert_eq!(row.get_field("line.price").unwrap().name(), "price");
        assert!(row.get_field(9).is_none());
        assert!(row.get_field("absent").is_none());

        // `DataType` answers identically, so descending never changes the calls.
        let dtype = row.dtype();
        assert_eq!(dtype.field(0).unwrap().name(), "line");
        assert_eq!(dtype.field_by_path("line.price").unwrap().name(), "price");
    }

    #[test]
    fn setting_by_path_reaches_a_nested_child() {
        let mut row = StructType::from_fields([StructType::from_fields([
            DataType::Int32.required_field("price")
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("line")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

        row.set_field_by_path("line.price", DataType::Float64.required_field("price"))
            .unwrap();
        assert_eq!(row["line"]["price"].dtype(), &DataType::Float64);

        // Removing reaches the same child, and the parent keeps its own identity.
        assert_eq!(row.remove_field("line.price").unwrap().name(), "price");
        assert_eq!(row["line"].field_len(), 0);
        assert_eq!(row.field_len(), 1);
    }
}

#[test]
fn a_range_over_a_column_is_a_window_of_it() {
    let item = DataType::Int64.nullable_field("item");
    let root = StructType::from_fields([DataType::serie(item.clone()).nullable_field("xs")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let xs = Serie::from_scalars(item, (1..=4_i64).map(Scalar::from)).unwrap();
    let row = Scalar::from_sequence([Scalar::from(xs)]);
    let window = parse("xs[1:3]").apply_scalar(&root, &row).unwrap();
    assert_eq!(
        window,
        Scalar::from_sequence([Scalar::from(2_i64), Scalar::from(3_i64)])
    );
    assert!(
        window.as_serie().is_some_and(Serie::is_column),
        "a window of a column is a column: {window:?}"
    );
}
