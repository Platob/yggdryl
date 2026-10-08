//! `rust/src/version.rs`: the version invariant no caller can reach.
//!
//! `rendered_len` is crate-private: it is the length the digest feed writes
//! before a version's canonical text, so it has to agree with what `Display`
//! actually renders for every reachable value. Everything a caller can observe
//! lives beside it here.

#[cfg(feature = "internals")]
mod internal {
    use yggdryl::Version;
    use yggdryl::internals::version::rendered_len;

    #[test]
    fn the_rendered_length_agrees_with_what_display_writes() {
        for text in [
            "0",
            "1",
            "5",
            "5.0.2",
            "5.0.250",
            "1.0.256",
            "65535.65535.65535",
            "5.0SP2",
            "5.0sp250",
            "1.0-rc1",
            "1.0rc1",
            "1..2",
            "1.2.3.4",
            "1.2界",
            "1.0sp250 ",
        ] {
            let value: Version = text.parse().unwrap();
            assert_eq!(rendered_len(&value), value.to_string().len(), "{text}");
        }
        for major in [0_u16, 1, 9, 10, 99, 100, 255, 256, 9_999, 10_000, u16::MAX] {
            for minor in [0_u16, 7, 42, 255, 1_000, u16::MAX] {
                for patch in [
                    None,
                    Some("1"),
                    Some("250"),
                    Some("-rc1"),
                    Some("beta.2"),
                    Some(".x"),
                ] {
                    let value = Version::new(major, minor, patch);
                    assert_eq!(rendered_len(&value), value.to_string().len(), "{value}");
                }
            }
        }
    }
}

mod ordered {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::sync::Arc;

    use arrow_array::{Array, Int32Array, RecordBatch, StringArray};
    use arrow_schema::DataType as ArrowDataType;

    use yggdryl::DataType;
    use yggdryl::{
        ArrowCastOptions, DataTypeId, DataTypeKind, Error, Field, FieldScalar, Scalar, Scheme,
        Serie, StructType, Version, VersionField,
    };

    fn version(text: &str) -> Version {
        text.parse().unwrap()
    }

    fn parse_position(text: &str) -> usize {
        match text.parse::<Version>().unwrap_err() {
            Error::Parse {
                target, position, ..
            } => {
                assert_eq!(target, "version");
                position
            }
            other => panic!("expected a positioned version parse error, got {other}"),
        }
    }

    fn digest(value: &Version) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    fn root(field: Field) -> Field {
        StructType::from_fields([field])
            .map(DataType::from)
            .unwrap()
            .required_field("row")
    }

    #[test]
    fn grammar_canonicalizes_every_supported_separator_and_trailing_zero() {
        assert_eq!(version("5.0.1"), version("005.000.001"));
        assert_eq!(version("5.0.1").to_string(), "5.0.1");
        assert_eq!(version("4.4.0").to_string(), "4.4");
        assert_eq!(version("7").to_string(), "7");
        assert_eq!(version("5.0.007").to_string(), "5.0.7");
        assert_eq!(version("5.0.007"), version("5.0.7"));

        assert_eq!(version("5.0"), version("5"));
        assert_eq!(version("5.0.000"), version("5"));
        assert_eq!(version("5.0.000").patch(), None);

        for text in [
            "0.0",
            "4.4",
            "5.0.2",
            "1.0.256",
            "256.0",
            "65535.65535.65535",
        ] {
            let held = version(text);
            assert_eq!(held.to_string().parse::<Version>().unwrap(), held);
        }
    }

    #[test]
    fn every_refusal_names_the_first_bad_byte() {
        // Only the numeric components refuse. Everything a patch tail can say is
        // kept rather than refused, so the refusals are the major, the minor,
        // and text that names no major at all.
        for (text, position) in [
            ("", 0),
            (".1", 0),
            (" 1", 0),
            ("-1", 0),
            ("+1", 0),
            ("v1", 0),
            ("65536.0", 4),
            ("99999", 4),
            ("0000065536", 9),
            ("1.65536", 6),
            ("1.99999", 6),
            ("12.65536", 7),
        ] {
            assert_eq!(parse_position(text), position, "{text:?}");
        }
        // Sixteen bits each, so what a byte could not hold is a version now.
        assert_eq!(
            (version("256.0").major(), version("1.256").minor()),
            (256, 256)
        );
        assert_eq!(version("65535.65535").to_string(), "65535.65535");
    }

    #[test]
    fn a_patch_tail_that_states_no_number_is_the_patch_as_written() {
        // The tail after the major and minor, less one separating `.`, is the
        // patch. The canonical text writes the `.` back before a patch opening
        // with a letter, a digit or a dot, and follows the minor directly with
        // any other, so every one of these reads back as itself.
        for (text, patch, canonical) in [
            ("1.2-rc1", "-rc1", "1.2-rc1"),
            ("1.2-RC1", "-RC1", "1.2-RC1"),
            ("1.2rc1", "rc1", "1.2.rc1"),
            ("1.2.rc1", "rc1", "1.2.rc1"),
            ("1.2SP2_EP240", "SP2_EP240", "1.2.SP2_EP240"),
            ("1.2.2.3", "2.3", "1.2.2.3"),
            ("4.4.0.0", "0.0", "4.4.0.0"),
            ("1.2界", "界", "1.2界"),
            ("1.2.-1", "-1", "1.2-1"),
            ("1..2", ".2", "1.0..2"),
            ("1.x", "x", "1.0.x"),
            ("1-", "-", "1.0-"),
            ("1.0SP", "SP", "1.0.SP"),
            ("1.0SP-2", "SP-2", "1.0.SP-2"),
            ("1.0SP.2", "SP.2", "1.0.SP.2"),
            ("1.0SP2.3", "SP2.3", "1.0.SP2.3"),
            ("1.0SP2SP3", "SP2SP3", "1.0.SP2SP3"),
            ("1.0.2SP3", "2SP3", "1.0.2SP3"),
            ("1.0.SP2", "SP2", "1.0.SP2"),
            ("1.0s250", "s250", "1.0.s250"),
            ("1.0sp250 ", "sp250 ", "1.0.sp250 "),
            ("1.0+meta", "+meta", "1.0+meta"),
            ("1.0.0-rc1", "0-rc1", "1.0.0-rc1"),
        ] {
            let held = version(text);
            assert_eq!(held.patch(), Some(patch), "{text:?}");
            assert_eq!(held.to_string(), canonical, "{text:?}");
            assert_eq!(version(canonical), held, "{text:?}");
            assert_eq!(
                Version::new(held.major(), held.minor(), Some(patch)),
                held,
                "{text:?}"
            );
        }

        // A separator stating nothing is no patch.
        for text in ["1.", "1.0."] {
            assert_eq!(version(text).patch(), None, "{text:?}");
            assert_eq!(version(text), version("1"), "{text:?}");
        }

        // The major and minor a tail follows are the stated ones.
        assert_eq!(
            (
                version("255.255-rc1").major(),
                version("255.255-rc1").minor()
            ),
            (255, 255)
        );
        // Past that separator the text is the patch, a trailing dot included.
        assert_eq!(version("1.0.0.").patch(), Some("0."));
        assert_ne!(version("1.0-rc1"), version("1.0-rc2"));
        assert_ne!(version("1.0-rc1"), version("1.0"));
        // A number past sixteen bits is a number all the same.
        assert_eq!(version("1.2.65536").patch(), Some("65536"));
        assert_eq!(
            version("1.2.99999999999999999999").to_string(),
            "1.2.99999999999999999999"
        );
    }

    #[test]
    fn compact_fix_service_packs_are_numeric_patches_at_every_boundary() {
        let field = DataType::Version.required_field("release");
        for (text, patch, canonical) in [
            ("5.0sp250", Some("250"), "5.0.250"),
            ("5.0SP250", Some("250"), "5.0.250"),
            ("5.0Sp250", Some("250"), "5.0.250"),
            ("5.0sP250", Some("250"), "5.0.250"),
            ("005.000sp00250", Some("250"), "5.0.250"),
            ("5.0SP0", None, "5"),
            ("5.0SP255", Some("255"), "5.0.255"),
            ("5.0SP256", Some("256"), "5.0.256"),
            ("5.0sp65535", Some("65535"), "5.0.65535"),
            ("5.0sp65536", Some("65536"), "5.0.65536"),
        ] {
            let expected = Version::new(5, 0, patch);
            let parsed = version(text);
            assert_eq!(parsed, expected, "{text}");
            assert_eq!(parsed.to_string(), canonical);
            assert_eq!(digest(&parsed), digest(&expected));
            assert_eq!(parsed.cmp(&expected), std::cmp::Ordering::Equal);
            assert_eq!(
                field.scalar(text).unwrap(),
                Scalar::Version(expected.clone())
            );
            let json = format!("\"{text}\"");
            assert_eq!(serde_json::from_str::<Version>(&json).unwrap(), expected);
            assert_eq!(
                serde_json::to_string(&parsed).unwrap(),
                format!("\"{canonical}\"")
            );
            let array = Serie::from_arrow_array(
                Some(&field),
                Arc::new(StringArray::from(vec![text])),
                ArrowCastOptions::new().with_safe(false),
            )
            .unwrap()
            .require_arrow_array()
            .unwrap();
            assert_eq!(
                array
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap()
                    .value(0),
                canonical
            );
            assert_eq!(
                Serie::from_arrow_array(Some(&field), array, ArrowCastOptions::default())
                    .unwrap()
                    .scalar(0)
                    .unwrap(),
                Scalar::Version(expected)
            );
        }
        assert!(version("5.0sp250") < version("5.0SP251"));
        assert!(version("5.0sp9") < version("5.0SP10"));
        // Straight after the last number read, so a bare major takes one too.
        assert_eq!(version("5SP2"), version("5.0.2"));
        assert_eq!(version("5.SP2").patch(), Some("SP2"));
    }

    #[test]
    fn serde_reads_back_a_patch_its_format_escapes() {
        // A patch may hold what JSON escapes, which no deserializer can hand
        // over as a borrowed slice of its input.
        for patch in ["a\"b", "a\\b", "a\nb", "a\tb", "a\0b", "\nsp"] {
            let value = Version::new(1, 2, Some(patch));
            let json = serde_json::to_string(&value).unwrap();
            assert_eq!(
                serde_json::from_str::<Version>(&json).unwrap(),
                value,
                "{json}"
            );
            assert_eq!(
                serde_json::from_reader::<_, Version>(json.as_bytes()).unwrap(),
                value,
                "{json}"
            );
            assert_eq!(
                serde_json::from_value::<Version>(serde_json::to_value(&value).unwrap()).unwrap(),
                value,
                "{json}"
            );
            let scalar = Scalar::Version(value);
            let wire = serde_json::to_string(&scalar).unwrap();
            assert_eq!(
                serde_json::from_str::<Scalar>(&wire).unwrap(),
                scalar,
                "{wire}"
            );
        }
        assert_eq!(
            serde_json::from_str::<Version>(r#""1\u002e0""#).unwrap(),
            version("1.0")
        );
        let refused = serde_json::from_str::<Version>("5")
            .unwrap_err()
            .to_string();
        assert!(refused.contains("a version string"), "{refused}");
    }

    #[test]
    fn native_components_have_exact_widths_and_roundtrip_at_each_boundary() {
        for major in [0_u16, 1, 255, 256, u16::MAX] {
            for minor in [0_u16, 1, 255, 256, u16::MAX] {
                for patch in [
                    None,
                    Some("1"),
                    Some("65536"),
                    Some("-rc1"),
                    Some("rc1"),
                    Some(".x"),
                    Some("sp2"),
                    Some("0.0"),
                    Some(" x"),
                    Some("界"),
                ] {
                    let value = Version::new(major, minor, patch);
                    assert_eq!(
                        (value.major(), value.minor(), value.patch()),
                        (major, minor, patch)
                    );
                    let parsed = version(&value.to_string());
                    assert_eq!(parsed, value, "{value}");
                    assert_eq!(digest(&parsed), digest(&value));
                }
            }
        }
        assert_eq!(Version::new(1, 0, Some("256")).to_string(), "1.0.256");
        // The constructor holds a patch as the parser does: a number without
        // its leading zeros, and zero or nothing as no patch at all.
        assert_eq!(Version::new(1, 0, Some("007")).patch(), Some("7"));
        assert_eq!(Version::new(1, 0, Some("0")), Version::new(1, 0, None));
        assert_eq!(Version::new(1, 0, Some("")), Version::new(1, 0, None));
        assert_eq!(Version::new(0, 0, None), Version::MIN);
        assert_eq!(Version::default(), Version::MIN);
    }

    #[test]
    fn ordering_is_natural_and_eq_hash_ord_agree() {
        let ordered = [
            Version::MIN,
            version("1.0"),
            version("1.0-rc1"),
            version("1.0-rc2"),
            version("1.0-rc10"),
            version("1.0.rc01"),
            version("1.0.rc1"),
            version("4.2"),
            version("4.4"),
            version("5.0"),
            version("5.0.1"),
            version("5.0.2"),
            version("5.0.2.1"),
            version("5.0.10"),
            version("5.0.10-rc1"),
            version("5.0.65536"),
            version("5.1"),
            version("255.255.65535"),
            version("256.0"),
            version("65535.65535.65535"),
        ];
        assert!(
            ordered.windows(2).all(|pair| pair[0] < pair[1]),
            "{ordered:?}"
        );
        assert!(version("1.2.9") < version("1.2.10"));

        let canonical = version("4.4");
        let redundant = version("4.4.0");
        assert_eq!(canonical, redundant);
        assert_eq!(canonical.cmp(&redundant), std::cmp::Ordering::Equal);
        assert_eq!(digest(&canonical), digest(&redundant));
        assert_eq!(Version::MIN, version("0.0.0"));
        assert!(version("1.2.255") < version("1.2.256"));

        // Equality, order and hash read one value: equal exactly when the order
        // says so, hashing alike when equal, and the order transitive.
        let mut corpus = ordered.to_vec();
        corpus.extend(
            [
                "4.4.0",
                "1.0-rc01",
                "1.0.rc001",
                "5.0.02",
                "5.0.0.2",
                "1.0-",
                "1.0+meta",
                "1.0_x",
                "1.0界",
            ]
            .map(version),
        );
        for a in &corpus {
            for b in &corpus {
                assert_eq!(a == b, a.cmp(b).is_eq(), "{a} {b}");
                assert_eq!(a.cmp(b), b.cmp(a).reverse(), "{a} {b}");
                if a == b {
                    assert_eq!(digest(a), digest(b), "{a} {b}");
                }
                for c in &corpus {
                    if a <= b && b <= c {
                        assert!(a <= c, "{a} {b} {c}");
                    }
                }
            }
        }
    }

    #[test]
    fn datatype_identity_naming_and_serde_are_total() {
        let dtype = DataType::Version;
        assert_eq!(dtype.id(), DataTypeId::Version);
        assert_eq!(dtype.kind(), DataTypeKind::Text);
        assert_eq!(dtype.name(), "version");
        assert_eq!(dtype.to_string(), "version");
        assert_eq!("VERSION".parse::<DataType>().unwrap(), dtype);
        assert_eq!(
            "VERSION".parse::<DataTypeId>().unwrap(),
            DataTypeId::Version
        );
        assert_eq!(DataTypeId::Version.as_str(), "version");
        // In the text family's range, after the eighteen string leaves: `as_u8`
        // is a wire contract laid out by family, so a version sits beside the
        // text it is rather than at the end of the enum.
        assert_eq!(DataTypeId::Version.as_u8(), 0x63);
        assert_eq!(DataTypeId::Version.fixed_byte_width(), None);
        assert_eq!(DataTypeId::all().last(), Some(&yggdryl::TimeInForce::ID));
        assert_eq!(DataTypeId::LargeUtf8StringView.as_u8(), 0x54);
        assert!(!DataTypeId::Version.is_parameterized());
        assert!(DataTypeId::Version.is_string());
        assert!(!dtype.is_nested());
        dtype.validate().unwrap();

        assert_eq!(dtype.clone().into_json().unwrap(), r#"{"type":"version"}"#);
        assert_eq!(DataType::from_json(r#"{"type":"version"}"#).unwrap(), dtype);
        let value = version("005.000.001");
        assert_eq!(serde_json::to_string(&value).unwrap(), r#""5.0.1""#);
        assert_eq!(
            serde_json::from_str::<Version>(r#""5.0.1""#).unwrap(),
            value
        );
    }

    #[test]
    fn scalar_and_field_contracts_rewrite_text_once() {
        let expected = Scalar::Version(version("5.0.1"));
        assert_eq!(DataType::Version.scalar("005.000.001").unwrap(), expected);
        assert_eq!(
            DataType::Version.scalar(expected.clone()).unwrap(),
            expected
        );

        let required = Field::new("begin_string", DataType::Version, false);
        assert_eq!(required.scalar("005.000.001").unwrap(), expected);
        let wrong = required.scalar(5_i32).unwrap_err().to_string();
        assert!(wrong.contains("begin_string"), "{wrong}");
        assert!(wrong.contains("version"), "{wrong}");
        assert!(required.scalar(Scalar::Null).is_err());
        assert_eq!(
            Field::new("begin_string", DataType::Version, true)
                .scalar(Scalar::Null)
                .unwrap(),
            Scalar::Null
        );

        let begin_string = VersionField::unit("begin_string", false);
        assert_eq!(begin_string.dtype(), &DataType::Version);
        let begin_string_field = begin_string.to_field();
        let typed = FieldScalar::new(&begin_string_field, expected.clone()).unwrap();
        assert_eq!(typed.value(), &expected);
    }

    #[test]
    fn arrow_field_values_and_casts_keep_version_identity() {
        let field = Field::new("begin_string", DataType::Version, false);
        let arrow = field.clone().into_arrow_field().unwrap();
        assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
        assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.version");
        assert_eq!(Field::from_arrow_field(&arrow).unwrap(), field);

        let stored = Serie::from_scalars(field.clone(), [Scalar::from(version("5.0.2"))])
            .unwrap()
            .require_arrow_array()
            .unwrap();
        assert_eq!(
            stored
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            "5.0.2"
        );
        assert_eq!(
            Serie::from_arrow_array(Some(&field), stored, ArrowCastOptions::default())
                .unwrap()
                .scalar(0)
                .unwrap(),
            Scalar::from(version("5.0.2"))
        );

        let ingested = Serie::from_arrow_array(
            Some(&field),
            Arc::new(StringArray::from(vec!["005.000.001"])),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap()
        .require_arrow_array()
        .unwrap();
        assert_eq!(
            ingested
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            "5.0.1"
        );
        assert!(
            Serie::from_arrow_array(
                Some(&field),
                Arc::new(Int32Array::from(vec![5])),
                ArrowCastOptions::new().with_safe(false)
            )
            .unwrap_err()
            .to_string()
            .contains("version")
        );

        let source_root = root(field.clone());
        let source_schema = source_root.clone().into_arrow_schema().unwrap();
        let source: Arc<dyn Array> = Arc::new(StringArray::from(vec!["5.0.2"]));
        let batch = RecordBatch::try_new(source_schema, vec![Arc::clone(&source)]).unwrap();
        let exact = Serie::from_arrow_batch(
            Some(&source_root),
            &batch,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap()
        .into_arrow_batch()
        .unwrap();
        // A column holds its leaf as its own typed array, so the `Arc` around it
        // is new; the buffers under it are the caller's.
        assert!(exact.column(0).to_data().ptr_eq(&source.to_data()));

        let text_root = root(DataType::utf8().required_field("begin_string"));
        let rendered = Serie::from_arrow_batch(
            Some(&text_root),
            &batch,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap()
        .into_arrow_batch()
        .unwrap();
        assert_eq!(rendered.column(0).data_type(), &ArrowDataType::Utf8);
        let numeric_root = root(DataType::Int32.required_field("begin_string"));
        let refused = Serie::from_arrow_batch(
            Some(&numeric_root),
            &batch,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("version"), "{refused}");

        assert!(version("5.0.2") < version("5.0.10"));
        assert!("5.0.10" < "5.0.2");
    }

    #[test]
    fn defaults_merges_and_compatibility_do_not_fall_through() {
        assert_eq!(
            DataType::Version.default_value().unwrap(),
            Scalar::Version(Version::MIN)
        );
        assert!(
            DataType::Version
                .is_default_value(&Scalar::Version(Version::MIN))
                .unwrap()
        );
        assert_eq!(
            DataType::Version
                .merge_with(&DataType::Version, true)
                .unwrap(),
            DataType::Version
        );
        let refused = DataType::Version
            .merge_with(&DataType::utf8(), true)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("version"), "{refused}");
        assert!(refused.contains("utf8"), "{refused}");

        assert_eq!(
            DataType::Version
                .clone()
                .into_scheme_compat(&Scheme::ARROW)
                .unwrap(),
            DataType::Version
        );
        for scheme in [
            Scheme::SPARK,
            Scheme::POLARS,
            Scheme::PANDAS,
            Scheme::ICEBERG,
        ] {
            assert_eq!(
                DataType::Version
                    .clone()
                    .into_scheme_compat(&scheme)
                    .unwrap(),
                DataType::utf8(),
                "{scheme}"
            );
        }
    }

    #[cfg(feature = "iceberg")]
    #[test]
    fn a_closed_exchange_vocabulary_refuses_version_by_name() {
        let error = yggdryl::iceberg::PrimitiveType::from_dtype(&DataType::Version)
            .unwrap_err()
            .to_string();
        assert!(error.contains("Iceberg"), "{error}");
        assert!(error.contains("version"), "{error}");
    }
}
