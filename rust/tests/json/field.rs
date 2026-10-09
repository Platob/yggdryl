//! `rust/src/json/field.rs`.

use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie};

/// A nullable field of `expression`, as a JSON cast's target is; the
/// grammar has no sorted map, so `sorted_map<int64, utf8>` is built.
fn target(expression: &str) -> Field {
    let dtype = if expression == "sorted_map<int64, utf8>" {
        DataType::map_of(DataType::Int64, DataType::utf8(), true).expect("a sorted map")
    } else {
        expression
            .parse::<DataType>()
            .unwrap_or_else(|error| panic!("{expression}: {error}"))
    };
    Field::new("v", dtype, true)
}

/// Fields and documents across every shape the reader plans, the
/// documents a writer of the field and a hand both spell: permuted and
/// absent names, escapes, text into numbers, numbers into text, and the
/// refusals each shape has.
fn corpus() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        (
            "struct<sym: utf8 not null, px: decimal128(12, 4), qty: int64 not null, ok: boolean, f: float64>",
            vec![
                r#"{"sym":"BRENT","px":"81.2500","qty":3,"ok":true,"f":1.5}"#,
                r#"{"qty":3,"sym":"BRENT","f":-0.0,"ok":false,"px":"81.25"}"#,
                r#"{"sym":"WTI","qty":-7}"#,
                r#"{"sym":"WTI","qty":"12","px":100,"f":null}"#,
                r#"{"sym":"café \"q\"","qty":1,"px":null}"#,
                r#"{"sym":"café","qty":18446744073709551615}"#,
                r#"{"sym":"x","qty":1,"f":7}"#,
                r#"{"sym":"x","qty":1,"px":1.5}"#,
                r#"{"sym":"x","qty":1,"extra":1}"#,
                r#"{"sym":"x","qty":1,"qty":2}"#,
                r#"{"sym":null,"qty":1}"#,
                r#"{"qty":1}"#,
                r#"{"sym":1.25,"qty":1}"#,
                r#"["x", "1.5", 1, true, 2.5]"#,
                r#"{"sym":"x","qty":1,}"#,
                r#"{"sym":"x" "qty":1}"#,
                r#"  {"sym" : "x" , "qty" : 1 }  "#,
                r#"{"sym":"x","qty":1} trailing"#,
                r#"{"sym":"\ud800","qty":1}"#,
                r#""""#,
                r#""text""#,
                "null",
                "7",
                "{}",
                "",
                "   ",
            ],
        ),
        (
            "serie<int64>",
            vec![
                "[1,2,3]",
                "[]",
                "[1,null,3]",
                r#"["4", 5]"#,
                "[1.5]",
                "[-0]",
                "[170141183460469231731687303715884105727]",
                "{}",
                r#"{"a":1}"#,
                "[1,]",
                "[[1]]",
            ],
        ),
        (
            "large_serie<utf8 not null>",
            vec![
                r#"["a","b"]"#,
                r#"["a",null]"#,
                "[1, 2.5, true]",
                r#"["\u0000", "\n"]"#,
            ],
        ),
        (
            "fixed_size_serie<int32, 2>",
            vec!["[1,2]", "[1]", "[1,2,3]", "[null,2]", "[2147483648,1]"],
        ),
        (
            "serie_view<struct<k: int8, v: utf8>>",
            vec![
                r#"[{"k":1,"v":"a"},{"v":"b"},null]"#,
                r#"[{"k":300}]"#,
                r#"[{"k":1,"v":"a","w":0}]"#,
                r#"[[1, "a"], {"k":2}, null]"#,
            ],
        ),
        (
            "map<utf8, int64>",
            vec![
                r#"{"b":1,"a":2,"c":null}"#,
                "{}",
                r#"{"a":1,"a":2}"#,
                r#"{"a":"x"}"#,
                r#"{"é":1,"e":2,"E":3}"#,
                "[]",
            ],
        ),
        (
            "sorted_map<int64, utf8>",
            vec![
                r#"{"10":"x","2":"y","-1":"z"}"#,
                r#"{"01":"a","1":"b"}"#,
                r#"{"":"a"}"#,
                r#"{"x":"a"}"#,
                r#"{" 3":"a","2":"b"}"#,
                "{}",
            ],
        ),
        (
            "map<utf8, struct<px: float64, qty: int64>>",
            vec![
                r#"{"BRENT":{"px":81.5,"qty":3},"WTI":{"qty":1}}"#,
                r#"{"BRENT":null}"#,
                r#"{"BRENT":{"px":"81.5"}}"#,
                r#"{"BRENT":{"bad":1}}"#,
                r#"{"WTI":[81.5, 3],"BRENT":{"qty":1}}"#,
            ],
        ),
        (
            "struct<inner: struct<a: serie<int8>, b: map<utf8, utf8>>, tags: serie<utf8>>",
            vec![
                r#"{"inner":{"a":[1,2],"b":{"k":"v"}},"tags":["x",null]}"#,
                r#"{"inner":null}"#,
                r#"{"tags":[]}"#,
                r#"{"inner":{"a":[128]}}"#,
            ],
        ),
        ("struct<>", vec!["{}", r#"{"a":1}"#, "[]"]),
        (
            "struct<a: int64, b: utf8>",
            vec![
                r#"{"a":1,"\u0061":2}"#,
                "\u{feff}{\"a\":1}",
                r#"{"b":"\u0000","a":null}"#,
                r#"{"a":-0}"#,
                r#"{"b":-0}"#,
                r#"{"a":1e2}"#,
            ],
        ),
        (
            "struct<d: decimal256(76, 0), s: utf8, e: state, q: decimal128(4, 2), g: float16, h: float32>",
            vec![
                r#"{"d":170141183460469231731687303715884105728,"s":340282366920938463463374607431768211455,"e":-170141183460469231731687303715884105729}"#,
                r#"{"q":100}"#,
                r#"{"q":"100"}"#,
                r#"{"g":1e300,"h":-1e300}"#,
                r#"{"e":"FILLED","q":"-0.01"}"#,
            ],
        ),
        (
            "map<ccy, int64>",
            vec![r#"{"usd":1,"USD":2}"#, r#"{"usd":1,"eur":2}"#],
        ),
        (
            "struct<i: isin not null, n: int64>",
            vec!["{}", r#"{"n":1}"#],
        ),
        ("fixed_size_serie<int64, 0>", vec!["[]", "[1]"]),
        (
            "struct<p: string(windows-1252,8), r: utf8>",
            vec![
                r#"{"p":"caf\u00e9"}"#,
                r#"{"p":"\u4e2d"}"#,
                r#"{"p":"toolongtext"}"#,
            ],
        ),
        (
            "struct<d: date32, t: datetime64(ms, UTC), u: uuid, c: ccy, s: state, a: sized_ascii(4), b: binary, w: url, n: version, h: float32>",
            vec![
                r#"{"d":"2024-01-02","t":"2024-01-02T03:04:05.006Z","u":"00112233-4455-6677-8899-aabbccddeeff","c":"USD","s":"NEW","a":"ABCD","b":"AQID","w":"https://example.com/a","n":"1.2.3","h":0.1}"#,
                r#"{"d":19724,"t":1700000000000,"c":"usd","s":2001}"#,
                r#"{"a":"ABCDE"}"#,
                r#"{"a":"é"}"#,
                r#"{"b":"not base64!"}"#,
                r#"{"u":"zz"}"#,
                r#"{"d":"2024-13-01"}"#,
                r#"{"c":"US"}"#,
                r#"{"h":1e300}"#,
            ],
        ),
    ]
}

fn door(field: &Field, document: &str) -> Scalar {
    yggdryl::json::from_bytes_with_field(document.as_bytes(), field).unwrap_or(Scalar::Null)
}

/// The column the cast lays a field's documents out as, read row by row,
/// is the one the field-directed door answers each document with: the
/// planned reading is the door's, a refusal null under `safe`.
#[test]
fn a_cast_reads_every_document_as_the_field_directed_door_does() {
    let safe = ArrowCastOptions::new().with_safe(true);
    let text = Field::new("json", DataType::utf8(), true);
    for (expression, documents) in corpus() {
        let field = target(expression);
        let cells = documents
            .iter()
            .map(|document| {
                if document.is_empty() {
                    Scalar::Null
                } else {
                    Scalar::from(*document)
                }
            })
            .collect::<Vec<_>>();
        let column = Serie::from_scalars(text.clone(), cells).expect("a text column");
        let expected = documents
            .iter()
            .map(|document| {
                if document.is_empty() {
                    Scalar::Null
                } else {
                    door(&field, document)
                }
            })
            .collect::<Vec<_>>();
        match column.cast(&field, safe) {
            Ok(read) => {
                for (row, (document, value)) in documents.iter().zip(expected).enumerate() {
                    // The door's value as a column stores it; one the door
                    // reads past what a fresh column admits - an integer
                    // token past a decimal's precision - is compared as read.
                    let stored = match Serie::from_scalars(field.clone(), [value.clone()]) {
                        Ok(stored) => stored.scalar(0).expect("one row"),
                        Err(_) => value,
                    };
                    assert_eq!(
                        read.scalar(row).expect("a row"),
                        stored,
                        "{expression} row {row}: {document}"
                    );
                }
            }
            // A value the door reads that the column cannot store - text a
            // charset cannot spell - fails the column, not the row, on
            // both paths.
            Err(error) => assert!(
                Serie::from_scalars(field.clone(), expected).is_err(),
                "{expression}: the cast failed ({error}) where the door's values lay out"
            ),
        }
    }
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::json_field::{plans, read};

    use super::{corpus, door, target};
    use yggdryl::arrow::scalar_memory_size;
    use yggdryl::{DataType, Field};

    /// A document the reader answers for, it answers with the door's value,
    /// representation and size alike; a document the door refuses, it
    /// leaves to the door.
    #[test]
    fn the_reader_answers_what_the_door_answers_or_leaves_the_document_to_it() {
        let mut answered = 0;
        for (expression, documents) in corpus() {
            let field = target(expression);
            assert!(plans(&field), "{expression} is planned");
            for document in documents {
                let door = yggdryl::json::from_bytes_with_field(document.as_bytes(), &field);
                match (read(&field, document.as_bytes()), door) {
                    (Some(fast), Ok(door)) => {
                        answered += 1;
                        assert_eq!(fast, door, "{expression}: {document}");
                        assert_eq!(
                            format!("{fast:?}"),
                            format!("{door:?}"),
                            "{expression}: {document} is stored alike"
                        );
                        assert_eq!(
                            scalar_memory_size(&fast),
                            scalar_memory_size(&door),
                            "{expression}: {document} costs alike"
                        );
                    }
                    (Some(fast), Err(error)) => {
                        panic!(
                            "{expression}: {document} read as {fast:?}, the door refuses: {error}"
                        )
                    }
                    (None, _) => {}
                }
            }
        }
        // Every document the door reads but the deferred few - a struct
        // spelled as a positional array, a root of empty text - is the
        // reader's.
        assert!(
            answered >= 36,
            "the reader answered only {answered} documents"
        );
    }

    /// A struct document read into columns pushes the cells the row read
    /// holds - one onto each child's column - and a document the reader
    /// leaves to the door leaves every column as it was.
    #[test]
    fn a_struct_read_into_columns_pushes_the_cells_of_the_row_read() {
        use yggdryl::Scalar;
        use yggdryl::internals::json_field::read_cells;
        for (expression, documents) in corpus() {
            let field = target(expression);
            let DataType::Struct(fields) = field.dtype() else {
                continue;
            };
            for document in documents {
                let mut columns = fields
                    .iter()
                    .map(|_| vec![Scalar::from("held")])
                    .collect::<Vec<_>>();
                let pushed = read_cells(&field, document.as_bytes(), &mut columns);
                match read(&field, document.as_bytes()) {
                    Some(Scalar::Null) => {
                        assert_eq!(pushed, Some(false), "{expression}: {document}")
                    }
                    Some(row) => {
                        assert_eq!(pushed, Some(true), "{expression}: {document}");
                        let cells = row.sequence_rows().expect("a struct row");
                        for (column, cell) in columns.iter().zip(cells.iter()) {
                            assert_eq!(column.len(), 2, "{expression}: {document}");
                            assert_eq!(&column[1], cell, "{expression}: {document}");
                            assert_eq!(format!("{:?}", column[1]), format!("{cell:?}"));
                        }
                    }
                    None => {
                        assert_eq!(pushed, None, "{expression}: {document}");
                        assert!(
                            columns.iter().all(|column| column.len() == 1),
                            "{expression}: {document} left a cell behind"
                        );
                    }
                }
            }
        }
    }

    /// A serie document read onto a run of items pushes the items the row
    /// read holds, and a document the reader leaves to the door leaves the
    /// run as it was.
    #[test]
    fn a_serie_read_onto_items_pushes_the_items_of_the_row_read() {
        use yggdryl::Scalar;
        use yggdryl::internals::json_field::read_items;
        for (expression, documents) in corpus() {
            let field = target(expression);
            if field.dtype().serie_item().is_none() {
                continue;
            }
            for document in documents {
                let mut items = vec![Scalar::from("held")];
                let pushed = read_items(&field, document.as_bytes(), &mut items);
                match read(&field, document.as_bytes()) {
                    Some(Scalar::Null) => {
                        assert_eq!(pushed, Some(None), "{expression}: {document}")
                    }
                    Some(row) => {
                        let cells = row.sequence_rows().expect("a serie row");
                        assert_eq!(pushed, Some(Some(cells.len())), "{expression}: {document}");
                        assert_eq!(&items[1..], &*cells, "{expression}: {document}");
                        assert_eq!(format!("{:?}", &items[1..]), format!("{:?}", &*cells));
                    }
                    None => {
                        assert_eq!(pushed, None, "{expression}: {document}");
                        assert_eq!(
                            items.len(),
                            1,
                            "{expression}: {document} left an item behind"
                        );
                    }
                }
            }
        }
    }

    /// A map document read onto a run of entries pushes the entries the
    /// row read holds, in its order, and a document the reader leaves to the
    /// door leaves the run as it was.
    #[test]
    fn a_map_read_onto_entries_pushes_the_entries_of_the_row_read() {
        use yggdryl::Scalar;
        use yggdryl::internals::json_field::read_entries;
        for (expression, documents) in corpus() {
            let field = target(expression);
            if field.dtype().as_mapping().is_none() {
                continue;
            }
            for document in documents {
                let mut entries = vec![(Scalar::from("held"), Scalar::Null)];
                let pushed = read_entries(&field, document.as_bytes(), &mut entries);
                match read(&field, document.as_bytes()) {
                    Some(Scalar::Null) => {
                        assert_eq!(pushed, Some(None), "{expression}: {document}")
                    }
                    Some(row) => {
                        let held = row.as_mapping().expect("a map row");
                        assert_eq!(pushed, Some(Some(held.len())), "{expression}: {document}");
                        assert_eq!(&entries[1..], held, "{expression}: {document}");
                        assert_eq!(format!("{:?}", &entries[1..]), format!("{held:?}"));
                    }
                    None => {
                        assert_eq!(pushed, None, "{expression}: {document}");
                        assert_eq!(
                            entries.len(),
                            1,
                            "{expression}: {document} left an entry behind"
                        );
                    }
                }
            }
        }
    }

    /// The documents a writer spells, and the hand-written ones every shape
    /// takes, are the reader's own: none falls back to the door.
    #[test]
    fn the_reader_answers_the_documents_of_every_planned_shape() {
        for (expression, document) in [
            (
                "struct<sym: utf8 not null, px: decimal128(12, 4), qty: int64 not null, ok: boolean, f: float64>",
                r#"{"qty":3,"sym":"BRENT","f":-0.0,"ok":false,"px":"81.25"}"#,
            ),
            (
                "struct<sym: utf8 not null, qty: int64 not null>",
                r#"{"sym":"x"}"#,
            ),
            ("struct<>", "{}"),
            ("serie<struct<>>", "[{}, {}]"),
            ("serie<int64>", "[1,null,3]"),
            ("large_serie<utf8 not null>", r#"["\u0000", "\n"]"#),
            ("fixed_size_serie<int32, 2>", "[null,2]"),
            (
                "serie_view<struct<k: int8, v: utf8>>",
                r#"[{"k":1,"v":"a"},{"v":"b"},null]"#,
            ),
            ("map<utf8, int64>", r#"{"b":1,"a":2,"c":null}"#),
            ("sorted_map<int64, utf8>", r#"{"10":"x","2":"y","-1":"z"}"#),
            (
                "map<utf8, struct<px: float64, qty: int64>>",
                r#"{"WTI":{"qty":1}}"#,
            ),
            (
                "struct<d: date32, t: datetime64(ms, UTC), u: uuid, c: ccy, s: state, a: sized_ascii(4), b: binary, w: url, n: version, h: float32>",
                r#"{"d":"2024-01-02","t":"2024-01-02T03:04:05.006Z","u":"00112233-4455-6677-8899-aabbccddeeff","c":"USD","s":"NEW","a":"ABCD","b":"AQID","w":"https://example.com/a","n":"1.2.3","h":0.1}"#,
            ),
        ] {
            let field = target(expression);
            let fast = read(&field, document.as_bytes())
                .unwrap_or_else(|| panic!("{expression}: {document} fell back to the door"));
            assert_eq!(fast, door(&field, document), "{expression}: {document}");
        }
    }

    /// A struct wider than a scan and than the stack's bits reads every
    /// name it states, in any order, and leaves a name it repeats - escaped
    /// or not - to the door.
    #[test]
    fn a_wide_struct_reads_by_name_and_leaves_a_repeated_name_to_the_door() {
        let names = (0..300)
            .map(|index| format!("c{index}"))
            .collect::<Vec<_>>();
        let expression = format!(
            "struct<{}>",
            names
                .iter()
                .map(|name| format!("{name}: int64"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let field = target(&expression);
        let document = format!(
            "{{{}}}",
            [299, 3, 150, 64, 0, 255, 256]
                .iter()
                .map(|index| format!("\"c{index}\":{index}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        let fast = read(&field, document.as_bytes()).expect("the reader answers");
        assert_eq!(fast, door(&field, &document));
        let repeated = r#"{"c7":1,"c\u0037":2}"#;
        assert_eq!(read(&field, repeated.as_bytes()), None);
        assert!(yggdryl::json::from_bytes_with_field(repeated.as_bytes(), &field).is_err());
    }

    /// The plan reads a nullable struct, serie or map root - the field a
    /// cast reads through - and leaves a required root, whose `null` is
    /// refused, and a leaf root, whose empty text is absence, to the door.
    #[test]
    fn only_a_nullable_nested_root_is_planned() {
        let required = Field::new(
            "v",
            "struct<a: int64>".parse::<DataType>().expect("a struct"),
            false,
        );
        assert!(!plans(&required));
        assert_eq!(read(&required, b"null"), None);
        assert!(yggdryl::json::from_bytes_with_field(b"null", &required).is_err());
        assert!(!plans(&target("int64")));
        assert!(!plans(&target("utf8")));
        let deep = (0..40).fold("int64".to_owned(), |item, _| format!("serie<{item}>"));
        let deep = target(&deep);
        assert!(!plans(&deep), "a field nested past the plan's depth");
        let document = format!("{}1{}", "[".repeat(40), "]".repeat(40));
        assert!(yggdryl::json::from_bytes_with_field(document.as_bytes(), &deep).is_ok());
        assert!(plans(&target(
            &(0..8).fold("int64".to_owned(), |item, _| { format!("serie<{item}>") })
        )));
    }

    /// What reads by choosing - a union's member, an encoding's values, a
    /// variant, an interval's tuple - and a map holding bytes, prepared whole,
    /// are the door's alone.
    #[test]
    fn a_field_reading_by_choice_is_left_to_the_door() {
        for expression in [
            "struct<u: union<0: int64, 1: utf8>>",
            "serie<dictionary<int32, utf8>>",
            "struct<r: run_end_encoded<int32, utf8>>",
            "struct<v: variant>",
            "struct<i: interval(month_day_nano)>",
            "map<utf8, binary>",
            "struct<n: null>",
        ] {
            let Ok(dtype) = expression.parse::<DataType>() else {
                panic!("{expression} parses");
            };
            assert!(!plans(&Field::new("v", dtype, true)), "{expression}");
        }
    }
}
