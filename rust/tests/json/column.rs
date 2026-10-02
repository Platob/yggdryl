//! `rust/src/json/column.rs`.

use yggdryl::{ArrowCastOptions, DataType, Field, Serie};

/// A nullable field of `expression`; the grammar has no sorted map, so
/// `sorted_map<int64, utf8>` is built.
fn field(expression: &str) -> Field {
    let dtype = if expression == "sorted_map<int64, utf8>" {
        DataType::map_of(DataType::Int64, DataType::utf8(), true).expect("a sorted map")
    } else {
        expression
            .parse::<DataType>()
            .unwrap_or_else(|error| panic!("{expression}: {error}"))
    };
    Field::new("v", dtype, true)
}

/// Columns of every shape the writer walks and of every leaf it hands to
/// the value path (encodings, a union, codes, temporals, a charset's
/// bytes), each row read by the field-directed door, absent rows and absent
/// children under absent parents among them.
fn corpus() -> Vec<Serie> {
    [
        (
            "struct<sym: utf8, px: decimal128(12, 4), qty: int64, ok: boolean, f: float64>",
            vec![
                r#"{"sym":"BRENT","px":"81.2500","qty":3,"ok":true,"f":1.5}"#,
                "null",
                r#"{"sym":"café \"q\" \\ \n","qty":-7,"f":-0.0}"#,
                r#"{"sym":null,"px":"-0.0001","qty":9223372036854775807,"f":1e300}"#,
            ],
        ),
        (
            "serie<struct<k: int8, v: large_utf8>>",
            vec![
                r#"[{"k":1,"v":"a"},null,{"k":null,"v":"\u0001"}]"#,
                "[]",
                "null",
                r#"[{"k":-128}]"#,
            ],
        ),
        (
            "fixed_size_serie<utf8_view, 2>",
            vec![
                r#"["a", "a string longer than twelve bytes"]"#,
                "null",
                r#"[null, ""]"#,
            ],
        ),
        (
            "struct<a: int16, b: int32, c: uint8, d: uint16, e: uint32, f: uint64, g: float32, h: int8>",
            vec![
                r#"{"a":-32768,"b":2147483647,"c":255,"d":65535,"e":4294967295,"f":18446744073709551615,"g":3.4028235e38,"h":-128}"#,
                r#"{"g":1e-45}"#,
                "null",
            ],
        ),
        ("serie_view<int64>", vec!["[1,2]", "null", "[]", "[3]"]),
        (
            "large_serie_view<utf8>",
            vec![r#"["x",null]"#, "[]", "null"],
        ),
        (
            "map<utf8, struct<px: float64, qty: int64>>",
            vec![
                r#"{"WTI":{"qty":1},"BRENT":{"px":81.5,"qty":3}}"#,
                "null",
                "{}",
                r#"{"☃":null}"#,
            ],
        ),
        ("map<int64, utf8>", vec![r#"{"10":"x","2":"y"}"#, "null", "{}"]),
        ("sorted_map<int64, utf8>", vec![r#"{"10":"x","2":"y"}"#, "{}"]),
        (
            "struct<d: date32, t: datetime64(ms, UTC), u: uuid, c: ccy, s: side, a: sized_ascii(4), b: binary, w: url, n: version, h: float32, e: dictionary<int32, utf8>, x: union<0: int64, 1: utf8>, p: string(windows-1252,8), q: fixed_utf8(4)>",
            vec![
                r#"{"d":"2024-01-02","t":"2024-01-02T03:04:05.006Z","u":"00112233-4455-6677-8899-aabbccddeeff","c":"USD","s":"BUYS","a":"ABCD","b":"AQID","w":"https://example.com/a","n":"1.2.3","h":0.1,"e":"z","x":[1,"m"],"p":"café","q":"ab"}"#,
                r#"{"x":[0,5]}"#,
                "null",
            ],
        ),
        (
            "struct<inner: struct<a: int64 not null, b: serie<int8>>, tags: serie<utf8>>",
            vec![
                r#"{"inner":{"a":1,"b":[1,2]},"tags":["x",null]}"#,
                r#"{"inner":null,"tags":null}"#,
                "null",
                r#"{"inner":{"a":2,"b":null},"tags":[]}"#,
            ],
        ),
    ]
    .into_iter()
    .map(|(expression, documents)| {
        let field = field(expression);
        let rows = documents
            .iter()
            .map(|document| {
                yggdryl::json::from_bytes_with_field(document.as_bytes(), &field)
                    .unwrap_or_else(|error| panic!("{expression}: {document}: {error}"))
            })
            .collect::<Vec<_>>();
        Serie::from_scalars(field, rows).unwrap_or_else(|error| panic!("{expression}: {error}"))
    })
    .collect()
}

/// Every column's JSON reads back as the column it was written from,
/// sliced columns included: the writing half and the reading half are one
/// cast apart.
#[test]
fn every_column_writes_the_json_that_reads_back_as_it() {
    let strict = ArrowCastOptions::new().with_safe(false);
    let text = Field::new("json", DataType::utf8(), true);
    for column in corpus() {
        let declared = column.field().expect("a column").clone();
        for window in [
            column.clone(),
            column.slice(1, column.len() - 1).expect("a slice"),
        ] {
            let written = window.cast(&text, strict).expect("JSON");
            assert_eq!(
                written
                    .cast(&declared, strict)
                    .expect("the JSON reads back"),
                window,
                "{}",
                declared.dtype()
            );
        }
    }
}

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::internals::json_column::{rows, rows_by_value};

    use super::{corpus, field};
    use yggdryl::{Scalar, Serie};

    /// The writer's rows are the value path's, byte for byte: every row,
    /// every slice, every leaf the writer reads itself and every leaf it
    /// hands to the value path.
    #[test]
    fn the_writer_spells_every_row_as_the_value_path_does() {
        for column in corpus() {
            let label = column.field().expect("a column").dtype().to_string();
            for offset in 0..column.len() {
                let window = column
                    .slice(offset, column.len() - offset)
                    .expect("a slice");
                assert_eq!(
                    rows(&window).expect("the writer writes"),
                    rows_by_value(&window).expect("the value path writes"),
                    "{label} from row {offset}"
                );
            }
        }
    }

    /// A row JSON cannot spell is refused in the value path's own words,
    /// at the same row.
    #[test]
    fn a_row_json_cannot_spell_is_refused_as_the_value_path_refuses_it() {
        let field = field("serie<float64>");
        let column = Serie::from_scalars(
            field,
            [
                Scalar::from_sequence([Scalar::from(1.5)]),
                Scalar::from_sequence([Scalar::from(f64::NAN)]),
            ],
        )
        .expect("a column");
        let written = rows(&column).expect_err("NaN has no JSON").to_string();
        let by_value = rows_by_value(&column)
            .expect_err("NaN has no JSON")
            .to_string();
        assert_eq!(written, by_value);
        assert!(written.contains("non-finite"), "{written}");
    }
}
