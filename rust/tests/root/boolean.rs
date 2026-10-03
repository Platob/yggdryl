//! `rust/src/boolean.rs`.

use yggdryl::DataType;
use yggdryl::boolean;

use super::typed::assert_typed_marker;

#[test]
fn scalar_markers_cover_null_and_boolean() {
    assert_typed_marker::<boolean::NullType>(DataType::Null);
    assert_typed_marker::<boolean::BooleanType>(DataType::Boolean);
}

#[test]
fn a_boolean_cell_reads_every_spelling_its_column_cast_reads() {
    use yggdryl::{ArrowCastOptions, Field, Scalar, Serie};

    // A declared boolean reads FIX's `Y`/`N` and a bridge's `no` as a
    // column cast does; text no boolean spells stays a refusal.
    let spellings = [
        "true",
        "TRUE",
        " t ",
        "tr",
        "tru",
        "yes",
        "Y",
        "ye",
        "on",
        "1",
        "false",
        "F",
        "fa",
        "fal",
        "fals",
        "no",
        "N",
        "off",
        "of",
        "0",
        "n/a",
        "aggressor",
        "2",
        "",
    ];
    let text = Field::new("flag", DataType::utf8(), true);
    let column = Serie::from_scalars(text, spellings.map(Scalar::from)).expect("a text column");
    let cast = column
        .cast(
            &Field::new("flag", DataType::Boolean, true),
            ArrowCastOptions::new().with_safe(true),
        )
        .expect("a boolean column");
    for (row, spelling) in spellings.iter().enumerate() {
        let cell = DataType::Boolean
            .scalar(Scalar::from(*spelling))
            .unwrap_or(Scalar::Null);
        assert_eq!(cell, cast.scalar(row).expect("a row"), "{spelling:?}");
    }
    assert_eq!(
        DataType::Boolean
            .scalar(Scalar::from("no"))
            .expect("a reading"),
        Scalar::from(false)
    );
    assert!(DataType::Boolean.scalar(Scalar::from("n/a")).is_err());
}

mod reading {
    //! The one boolean table, read through the value door, the column cast
    //! and truthiness.

    use std::sync::Arc;

    use arrow_array::builder::StringDictionaryBuilder;
    use arrow_array::types::Int32Type;
    use arrow_array::{
        ArrayRef, BooleanArray, Int32Array, LargeStringArray, RunArray, StringArray,
        StringViewArray, StructArray,
    };
    use arrow_buffer::NullBuffer;
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Fields};
    use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie, StructType};

    /// Every spelling the table reads, in a case and padding of its own, and
    /// what each reads as.
    const SPELLED: [(&str, bool); 19] = [
        ("true", true),
        ("T", true),
        (" tr ", true),
        ("TRU", true),
        ("Yes", true),
        ("y", true),
        ("YE", true),
        ("On", true),
        ("1", true),
        ("FALSE", false),
        ("f", false),
        ("Fa", false),
        ("fal", false),
        ("FALS", false),
        ("No", false),
        ("n", false),
        ("\tOFF\n", false),
        ("of", false),
        (" 0", false),
    ];

    /// Text no boolean spells.
    const UNSPELLED: [&str; 12] = [
        "n/a", "2", "00", "0.0", "truee", "yess", "nope", "ja", "oui", "tt", "o", "-1",
    ];

    fn boolean(nullable: bool) -> Field {
        Field::new("flag", DataType::Boolean, nullable)
    }

    fn text_column(texts: &[&str]) -> Serie {
        let field = Field::new("flag", DataType::utf8(), true);
        Serie::from_scalars(field, texts.iter().copied().map(Scalar::from)).expect("a text column")
    }

    fn cast_text(texts: &[&str], target: &Field, safe: bool) -> yggdryl::arrow::Result<Serie> {
        text_column(texts).cast(target, ArrowCastOptions::new().with_safe(safe))
    }

    #[test]
    fn the_value_door_reads_every_spelling_of_the_one_table_and_nothing_else() {
        for (text, expected) in SPELLED {
            assert_eq!(
                DataType::Boolean.scalar(Scalar::from(text)).expect(text),
                Scalar::from(expected),
                "{text:?}"
            );
        }
        for text in UNSPELLED {
            assert!(
                DataType::Boolean.scalar(Scalar::from(text)).is_err(),
                "{text:?} read as a boolean"
            );
        }
        // A registered code is an identity, never a spelling: Norway is not
        // false.
        let norway = DataType::country()
            .scalar(Scalar::from("NO"))
            .expect("a country");
        assert!(DataType::Boolean.scalar(norway).is_err());
    }

    #[test]
    fn truthiness_is_false_exactly_where_the_table_reads_false_or_the_text_is_blank() {
        // One false set: a text is falsy when the boolean cast reads it false,
        // or when nothing but blanks is there; text the cast cannot read is
        // present, so true.
        let corpus: Vec<&str> = SPELLED
            .iter()
            .map(|(text, _)| *text)
            .chain(UNSPELLED)
            .chain(["", "   ", "\t\n", "falsey", "anything", "NO "])
            .collect();
        let cast = cast_text(&corpus, &boolean(true), true).expect("a boolean column");
        for (row, text) in corpus.iter().enumerate() {
            let expected = match cast.scalar(row).expect("a row") {
                Scalar::Boolean(flag) => flag.get(),
                _ => !text.trim().is_empty(),
            };
            assert_eq!(Scalar::from(*text).is_truthy(), expected, "{text:?}");
        }
        // A code or an enum member is present unless it is the empty member.
        let norway = DataType::country()
            .scalar(Scalar::from("NO"))
            .expect("a country");
        assert!(norway.is_truthy());
    }

    #[test]
    fn a_text_column_casts_into_booleans_through_the_one_table_under_every_text_layout() {
        let texts: Vec<Option<&str>> = SPELLED
            .iter()
            .map(|(text, _)| Some(*text))
            .chain(UNSPELLED.map(Some))
            .chain([None, Some("")])
            .collect();
        let expected: Vec<Option<bool>> = texts
            .iter()
            .map(|text| match text {
                Some(text) => DataType::Boolean
                    .scalar(Scalar::from(*text))
                    .ok()
                    .and_then(|value| value.as_bool()),
                None => None,
            })
            .collect();
        let mut dictionary = StringDictionaryBuilder::<Int32Type>::new();
        for text in &texts {
            dictionary.append_option(*text);
        }
        let runs: ArrayRef = {
            let ends = Int32Array::from(
                (1..=i32::try_from(texts.len()).expect("fits")).collect::<Vec<_>>(),
            );
            let values = StringArray::from(texts.clone());
            Arc::new(RunArray::<Int32Type>::try_new(&ends, &values).expect("a run-end column"))
        };
        let layouts: [(&str, ArrayRef); 5] = [
            ("utf8", Arc::new(StringArray::from(texts.clone()))),
            (
                "large_utf8",
                Arc::new(LargeStringArray::from(texts.clone())),
            ),
            ("utf8_view", Arc::new(StringViewArray::from(texts.clone()))),
            ("dictionary", Arc::new(dictionary.finish())),
            ("run_end", runs),
        ];
        for (name, array) in layouts {
            let column = Serie::from_arrow_array(None, array, ArrowCastOptions::new())
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            let cast = column
                .cast(&boolean(true), ArrowCastOptions::new().with_safe(true))
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            let read: Vec<Option<bool>> = (0..cast.len())
                .map(|row| cast.scalar(row).expect("a row").as_bool())
                .collect();
            assert_eq!(read, expected, "{name}");
        }
    }

    #[test]
    fn a_strict_cast_refuses_text_no_boolean_spells_naming_the_field_and_the_row() {
        let refused = cast_text(&["yes", "n/a", "no"], &boolean(true), false)
            .expect_err("n/a spells no boolean")
            .to_string();
        assert!(
            refused.contains(
                "field \"flag\" row 1: \"n/a\" does not read as boolean: \
                 expected true/false, yes/no, y/n, on/off or 1/0"
            ),
            "{refused}"
        );
        // Under `safe` the same cell is null in a nullable column; a required
        // column refuses it by the value whatever `safe` says, never
        // inventing a default.
        let lenient = cast_text(&["yes", "n/a", "no"], &boolean(true), true).expect("nulls");
        assert_eq!(lenient.scalar(1).expect("a row"), Scalar::Null);
        let required = cast_text(&["yes", "n/a", "no"], &boolean(false), true)
            .expect_err("a required column holds no null")
            .to_string();
        assert!(
            required.contains("row 1: \"n/a\" does not read as boolean"),
            "{required}"
        );
        // An empty cell is absence before any reading, whatever `safe` says.
        let blank =
            cast_text(&["", "on"], &boolean(true), false).expect("absence is not a refusal");
        assert_eq!(blank.scalar(0).expect("a row"), Scalar::Null);
        assert_eq!(blank.scalar(1).expect("a row"), Scalar::from(true));
    }

    #[test]
    fn a_row_an_ancestor_null_hides_is_never_read() {
        // The child cell under a null struct row holds text no boolean
        // spells; a strict cast reads only exposed rows, so it is never seen.
        let child: ArrayRef = Arc::new(StringArray::from(vec![Some("garbage"), Some("yes")]));
        let fields = Fields::from(vec![ArrowField::new("flag", ArrowDataType::Utf8, true)]);
        let rows: ArrayRef = Arc::new(StructArray::new(
            fields,
            vec![child],
            Some(NullBuffer::from(vec![false, true])),
        ));
        let landed =
            Serie::from_arrow_array(None, rows, ArrowCastOptions::new()).expect("the struct lands");
        let target = Field::new(
            "row",
            DataType::from(
                StructType::from_fields([DataType::Boolean.nullable_field("flag")])
                    .expect("one field"),
            ),
            true,
        );
        let cast = landed
            .cast(&target, ArrowCastOptions::new().with_safe(false))
            .expect("the hidden cell is not read");
        assert_eq!(cast.scalar(0).expect("a row"), Scalar::Null);
        let second = cast.scalar(1).expect("a row");
        assert_eq!(
            second.get(0).map(|cell| cell.into_owned()),
            Some(Scalar::from(true)),
            "{second:?}"
        );
    }

    #[test]
    fn a_list_of_text_casts_into_a_list_of_booleans_through_the_same_table() {
        let item = DataType::utf8().nullable_field("item");
        let source = Field::new("flags", DataType::serie(item), true);
        let column = Serie::from_scalars(
            source,
            [Scalar::from_sequence([
                Scalar::from("Y"),
                Scalar::from("off"),
                Scalar::from("maybe"),
            ])],
        )
        .expect("a list column");
        let target = Field::new(
            "flags",
            DataType::serie(DataType::Boolean.nullable_field("item")),
            true,
        );
        let cast = column
            .cast(&target, ArrowCastOptions::new().with_safe(true))
            .expect("a list of booleans");
        assert_eq!(
            cast.scalar(0).expect("a row"),
            Scalar::from_sequence([Scalar::from(true), Scalar::from(false), Scalar::Null])
        );
        let strict = column
            .cast(&target, ArrowCastOptions::new().with_safe(false))
            .expect_err("maybe spells no boolean")
            .to_string();
        assert!(
            strict.contains("\"maybe\" does not read as boolean"),
            "{strict}"
        );
        // The one table also answers a boolean built from a boolean column:
        // the cast onto its own field costs nothing and reads no text.
        let booleans: ArrayRef = Arc::new(BooleanArray::from(vec![Some(true), None]));
        let held = Serie::from_arrow_array(Some(&boolean(true)), booleans, ArrowCastOptions::new())
            .expect("a boolean column");
        assert_eq!(held.scalar(0).expect("a row"), Scalar::from(true));
    }
}

#[cfg(feature = "internals")]
mod internal {
    //! The table itself, which no caller names.

    use yggdryl::internals::boolean::{BOOLEAN_SPELLINGS, bool_from_text, bool_of, truthy_text};
    use yggdryl::{DataType, Scalar};

    #[test]
    fn the_table_is_arrows_string_to_boolean_vocabulary_trimmed_and_case_folded() {
        for text in ["true", "t", "tr", "tru", "yes", "y", "ye", "on", "1"] {
            assert_eq!(bool_from_text(text), Some(true), "{text:?}");
            assert_eq!(bool_from_text(&text.to_ascii_uppercase()), Some(true));
            assert_eq!(bool_from_text(&format!(" {text}\t")), Some(true));
        }
        for text in [
            "false", "f", "fa", "fal", "fals", "no", "n", "off", "of", "0",
        ] {
            assert_eq!(bool_from_text(text), Some(false), "{text:?}");
            assert_eq!(bool_from_text(&text.to_ascii_uppercase()), Some(false));
            assert_eq!(bool_from_text(&format!("\u{a0}{text} ")), Some(false));
        }
        for text in ["", " ", "n/a", "2", "00", "truee", "ja", "o", "-1", "0.0"] {
            assert_eq!(bool_from_text(text), None, "{text:?}");
        }
        assert_eq!(BOOLEAN_SPELLINGS, "true/false, yes/no, y/n, on/off or 1/0");
    }

    #[test]
    fn truthy_text_answers_the_table_else_presence() {
        for (text, expected) in [
            ("no", false),
            (" OF ", false),
            ("0", false),
            ("", false),
            ("  ", false),
            ("yes", true),
            ("1", true),
            ("n/a", true),
            ("0.0", true),
            ("falsey", true),
        ] {
            assert_eq!(truthy_text(text), expected, "{text:?}");
        }
    }

    #[test]
    fn bool_of_reads_a_boolean_or_a_string_leaf_and_never_a_code() {
        assert_eq!(bool_of(&Scalar::from(true)), Some(true));
        assert_eq!(bool_of(&Scalar::from("N")), Some(false));
        assert_eq!(bool_of(&Scalar::from(" on ")), Some(true));
        assert_eq!(bool_of(&Scalar::from("n/a")), None);
        assert_eq!(bool_of(&Scalar::from(1_i64)), None);
        assert_eq!(bool_of(&Scalar::Null), None);
        let norway = DataType::country()
            .scalar(Scalar::from("NO"))
            .expect("a country");
        assert_eq!(bool_of(&norway), None);
    }
}
