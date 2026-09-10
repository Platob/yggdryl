use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use arrow_array::{Array, Int32Array, RecordBatch, StringArray};
use arrow_schema::DataType as ArrowDataType;

use super::super::DataType;
use crate::arrow::{scalar_array, scalar_value};
use crate::{
    ArrowCast, ArrowCastOptions, DataTypeId, DataTypeKind, Error, Field, FieldScalar, Scalar,
    Scheme, Version, VersionField,
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
    DataType::from_fields([field])
        .unwrap()
        .required_field("row")
}

#[test]
fn grammar_canonicalizes_every_supported_separator_and_trailing_zero() {
    assert_eq!(std::mem::size_of::<Version>(), 4);
    assert_eq!(version("5.0.1"), version("005.000.001"));
    assert_eq!(version("5.0.1").to_string(), "5.0.1");
    assert_eq!(version("4.4.0").to_string(), "4.4");
    assert_eq!(version("7").to_string(), "7");

    assert_eq!(version("5.0"), version("5"));

    for text in ["0.0", "4.4", "5.0.2", "1.0.256", "255.255.65535"] {
        let held = version(text);
        assert_eq!(held.to_string().parse::<Version>().unwrap(), held);
    }
}

#[test]
fn every_refusal_names_the_first_bad_byte() {
    // Only the numeric components refuse. Everything a patch tail can say is
    // folded rather than refused, so the refusals are the major, the minor,
    // and text that names no major at all.
    for (text, position) in [
        ("", 0),
        (".1", 0),
        (" 1", 0),
        ("-1", 0),
        ("+1", 0),
        ("v1", 0),
        ("256.0", 2),
        ("999", 2),
        ("1.256", 4),
        ("1.999", 4),
        ("12.256", 5),
    ] {
        assert_eq!(parse_position(text), position, "{text:?}");
    }
}

#[test]
fn a_patch_tail_that_states_no_number_folds_instead_of_refusing() {
    // Every tail the strict grammar refused now parses, and the components it
    // does read stay exactly what the text stated.
    for text in [
        "1-",
        "1.",
        "1..2",
        "1.+2",
        "1.2.3.4",
        "1.0SP",
        "1.0sp65536",
        "1.0.65536",
        "1.0SP-2",
        "1.0SP.2",
        "1.0SP2.3",
        "1.0.2SP3",
        "1.0SP2SP3",
        "1.0SP2_EP250",
        "1.0s250",
        "1.0sp250 ",
        "1.0-rc1",
        "1.0.SP2",
        "1.0+meta",
    ] {
        let held = version(text);
        assert_eq!(held.major(), 1, "{text:?}");
        // The same tail always reads as the same version.
        assert_eq!(held, version(text), "{text:?}");
    }

    // The tail is read after the major and minor, whatever they were.
    assert_eq!(
        (version("4.4.0.0").major(), version("4.4.0.0").minor()),
        (4, 4)
    );
    assert_eq!(version("4.4.0.0"), version("4.4.0.0"));

    // A stated number is still that number, whichever way the tail states it.
    assert_eq!(version("5.0.250"), Version::new(5, 0, 250));
    assert_eq!(version("5.0sp250"), Version::new(5, 0, 250));
    assert_eq!(version("5.0.0"), Version::new(5, 0, 0));

    // A fold never lands on nothing, and unlike tails read as unlike versions.
    assert_ne!(version("1.0-rc1").patch(), 0);
    assert_ne!(version("1.0-rc1"), version("1.0-rc2"));
    assert_ne!(version("1.0-rc1"), version("1.0"));

    // The major and minor a folded tail follows are the stated ones.
    assert_eq!(
        (version("1.2-rc1").major(), version("1.2-rc1").minor()),
        (1, 2)
    );
    assert_eq!(version("255.255-rc1").major(), 255);
}

#[test]
fn compact_fix_service_packs_are_numeric_patches_at_every_boundary() {
    let field = DataType::Version.required_field("release");
    for (text, patch, canonical) in [
        ("5.0sp250", 250, "5.0.250"),
        ("5.0SP250", 250, "5.0.250"),
        ("5.0Sp250", 250, "5.0.250"),
        ("5.0sP250", 250, "5.0.250"),
        ("005.000sp00250", 250, "5.0.250"),
        ("5.0SP0", 0, "5"),
        ("5.0SP255", 255, "5.0.255"),
        ("5.0SP256", 256, "5.0.256"),
        ("5.0sp65535", 65535, "5.0.65535"),
    ] {
        let expected = Version::new(5, 0, patch);
        let parsed = version(text);
        assert_eq!(parsed, expected, "{text}");
        assert_eq!(parsed.to_string(), canonical);
        assert_eq!(digest(&parsed), digest(&expected));
        assert_eq!(parsed.cmp(&expected), std::cmp::Ordering::Equal);
        assert_eq!(parsed.rendered_len(), canonical.len());
        assert_eq!(field.scalar(text).unwrap(), Scalar::Version(expected));
        let json = format!("\"{text}\"");
        assert_eq!(serde_json::from_str::<Version>(&json).unwrap(), expected);
        assert_eq!(
            serde_json::to_string(&parsed).unwrap(),
            format!("\"{canonical}\"")
        );
        let array = field
            .cast_arrow_array(
                Arc::new(StringArray::from(vec![text])),
                ArrowCastOptions::new().with_safe(false),
            )
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
            scalar_value(&field, array.as_ref()).unwrap(),
            Scalar::Version(expected)
        );
    }
    assert!(version("5.0sp250") < version("5.0SP251"));
    assert_eq!(version("255.255sp65535"), Version::MAX);
}

#[test]
fn native_components_have_exact_widths_and_roundtrip_at_each_boundary() {
    assert_eq!(Version::MAX_PARTS, 3);
    for major in [0_u8, 1, u8::MAX] {
        for minor in [0_u8, 1, u8::MAX] {
            for patch in [0_u16, 1, 255, 256, u16::MAX] {
                let value = Version::new(major, minor, patch);
                assert_eq!(
                    (value.major(), value.minor(), value.patch()),
                    (major, minor, patch)
                );
                let parsed = version(&value.to_string());
                assert_eq!(parsed, value);
                assert_eq!(digest(&parsed), digest(&value));
                assert_eq!(value.rendered_len(), value.to_string().len());
            }
        }
    }
    assert_eq!(Version::new(1, 0, 256).to_string(), "1.0.256");
}

#[test]
fn ordering_is_numeric_and_eq_hash_ord_agree() {
    let ordered = [
        Version::MIN,
        version("1.0"),
        version("4.2"),
        version("4.4"),
        version("5.0"),
        version("5.0.1"),
        version("5.0.2"),
        version("5.0.10"),
        Version::MAX,
    ];
    assert!(ordered.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(version("1.2.9") < version("1.2.10"));

    let canonical = version("4.4");
    let redundant = version("4.4.0");
    assert_eq!(canonical, redundant);
    assert_eq!(canonical.cmp(&redundant), std::cmp::Ordering::Equal);
    assert_eq!(digest(&canonical), digest(&redundant));

    assert_eq!(Version::MAX, version("255.255.65535"));
    assert_eq!(Version::MIN, version("0.0.0"));
    assert!(version("1.2.255") < version("1.2.256"));
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
    assert_eq!(DataTypeId::Version.as_u8(), 54);
    assert_eq!(DataTypeId::Version.fixed_byte_width(), None);
    // `Version` is no longer last: the coded FIX datatypes, then `Url`, then
    // `Isin` were appended after it, which is what `as_u8` being a wire
    // contract requires.
    assert_eq!(DataTypeId::ALL.last(), Some(&DataTypeId::Isin));
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

    let begin_string = VersionField::new("begin_string", false);
    assert_eq!(begin_string.dtype(), &DataType::Version);
    let typed = FieldScalar::new(begin_string.as_field(), expected.clone()).unwrap();
    assert_eq!(typed.value(), &expected);
}

#[test]
fn arrow_field_values_and_casts_keep_version_identity() {
    let field = Field::new("begin_string", DataType::Version, false);
    let arrow = field.clone().into_arrow().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.version");
    assert_eq!(Field::from_arrow(&arrow).unwrap(), field);

    let stored = scalar_array(&field, &Scalar::from(version("5.0.2"))).unwrap();
    assert_eq!(
        stored
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "5.0.2"
    );
    assert_eq!(
        scalar_value(&field, stored.as_ref()).unwrap(),
        Scalar::from(version("5.0.2"))
    );

    let ingested = field
        .cast_arrow_array(
            Arc::new(StringArray::from(vec!["005.000.001"])),
            ArrowCastOptions::new().with_safe(false),
        )
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
        field
            .cast_arrow_array(
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
    let exact = source_root
        .cast_arrow_batch(batch.clone(), ArrowCastOptions::new().with_safe(false))
        .unwrap();
    assert!(Arc::ptr_eq(exact.column(0), &source));

    let text_root = root(DataType::Utf8.required_field("begin_string"));
    let rendered = text_root
        .cast_arrow_batch(batch.clone(), ArrowCastOptions::new().with_safe(false))
        .unwrap();
    assert_eq!(rendered.column(0).data_type(), &ArrowDataType::Utf8);
    let numeric_root = root(DataType::Int32.required_field("begin_string"));
    let refused = numeric_root
        .cast_arrow_batch(batch, ArrowCastOptions::new().with_safe(false))
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
        .merge_with(&DataType::Utf8, true)
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
            DataType::Utf8,
            "{scheme}"
        );
    }
}

#[cfg(feature = "iceberg")]
#[test]
fn a_closed_exchange_vocabulary_refuses_version_by_name() {
    let error = crate::media::iceberg::PrimitiveType::from_dtype(&DataType::Version)
        .unwrap_err()
        .to_string();
    assert!(error.contains("Iceberg"), "{error}");
    assert!(error.contains("version"), "{error}");
}
