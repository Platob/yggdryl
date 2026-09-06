use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use arrow_array::{Array, Int32Array, RecordBatch, StringArray};
use arrow_schema::DataType as ArrowDataType;

use super::super::DataType;
use crate::arrow::{scalar_array, scalar_value};
use crate::{
    ArrowCast, DataTypeId, DataTypeKind, Error, Field, Scalar, Scheme, Version, VersionField,
    VersionScalar,
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
    assert_eq!(std::mem::size_of::<Version>(), 16);
    assert_eq!(version("5.0SP1"), version("5.0.SP1"));
    assert_eq!(version("5.0SP1").to_string(), "5.0SP1");
    assert_eq!(version("1.0.0-rc1").to_string(), "1.0-rc1");
    assert_eq!(version("4.4.0").to_string(), "4.4");
    assert_eq!(version("4.4.0.0"), version("4.4"));
    assert_eq!(version("7").to_string(), "7");

    assert_eq!(version("5.0"), version("5"));
    assert_eq!(version("5SP1"), version("5.0.SP1"));

    for text in ["0.0", "4.4", "5.0-rc1", "5.0SP2", "1.0.2.3"] {
        let held = version(text);
        assert_eq!(held.to_string().parse::<Version>().unwrap(), held);
    }
}

#[test]
fn every_refusal_names_the_first_bad_byte() {
    for (text, position) in [
        ("", 0),
        ("256.0", 2),
        ("1.256", 4),
        ("1.0.65536", 8),
        ("1.2.3.4.5", 8),
        ("1.", 1),
        ("1-", 1),
        ("1..2", 2),
        ("1.+2", 2),
        ("1.0SP$", 5),
        ("1.0ABCDEFGHIJKLMNO", 17),
    ] {
        assert_eq!(parse_position(text), position, "{text:?}");
    }
}

#[test]
fn ordering_is_numeric_and_eq_hash_ord_agree() {
    let ordered = [
        Version::MIN,
        version("1.0"),
        version("4.2"),
        version("4.4"),
        version("5.0-rc1"),
        version("5.0"),
        version("5.0SP1"),
        version("5.0SP2"),
        version("5.0SP10"),
        Version::MAX,
    ];
    assert!(ordered.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(version("1.2.9") < version("1.2.10"));

    let canonical = version("4.4");
    let redundant = version("4.4.0");
    assert_eq!(canonical, redundant);
    assert_eq!(canonical.cmp(&redundant), std::cmp::Ordering::Equal);
    assert_eq!(digest(&canonical), digest(&redundant));

    let upper = version("5.0SP1");
    let lower = version("5.0sp1");
    assert_ne!(upper, lower);
    assert_ne!(upper.cmp(&lower), std::cmp::Ordering::Equal);
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
    assert_eq!(DataTypeId::ALL.last(), Some(&DataTypeId::Version));
    assert!(!DataTypeId::Version.is_parameterized());
    assert!(DataTypeId::Version.is_string());
    assert!(!dtype.is_nested());
    dtype.validate().unwrap();

    assert_eq!(dtype.clone().into_json().unwrap(), r#"{"type":"version"}"#);
    assert_eq!(DataType::from_json(r#"{"type":"version"}"#).unwrap(), dtype);
    let value = version("5.0.SP1");
    assert_eq!(serde_json::to_string(&value).unwrap(), r#""5.0SP1""#);
    assert_eq!(
        serde_json::from_str::<Version>(r#""5.0SP1""#).unwrap(),
        value
    );
}

#[test]
fn scalar_and_field_contracts_rewrite_text_once() {
    let expected = Scalar::Version(version("5.0SP1"));
    assert_eq!(DataType::Version.scalar("5.0.SP1").unwrap(), expected);
    assert_eq!(
        DataType::Version.scalar(expected.clone()).unwrap(),
        expected
    );

    let required = Field::new("begin_string", DataType::Version, false);
    assert_eq!(required.scalar("5.0.SP1").unwrap(), expected);
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

    let typed = VersionScalar::new(expected.clone()).unwrap();
    assert_eq!(typed.value(), &expected);
    assert_eq!(
        VersionField::new("begin_string", false).dtype(),
        &DataType::Version
    );
}

#[test]
fn arrow_field_values_and_casts_keep_version_identity() {
    let field = Field::new("begin_string", DataType::Version, false);
    let arrow = field.clone().into_arrow().unwrap();
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.version");
    assert_eq!(Field::from_arrow(&arrow).unwrap(), field);

    let stored = scalar_array(&field, &Scalar::from(version("5.0SP2"))).unwrap();
    assert_eq!(
        stored
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "5.0SP2"
    );
    assert_eq!(
        scalar_value(&field, stored.as_ref()).unwrap(),
        Scalar::from(version("5.0SP2"))
    );

    let ingested = field
        .cast_arrow_array(Arc::new(StringArray::from(vec!["5.0.SP1"])), false)
        .unwrap();
    assert_eq!(
        ingested
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "5.0SP1"
    );
    assert!(
        field
            .cast_arrow_array(Arc::new(Int32Array::from(vec![5])), false)
            .unwrap_err()
            .to_string()
            .contains("version")
    );

    let source_root = root(field.clone());
    let source_schema = source_root.clone().into_arrow_schema().unwrap();
    let source: Arc<dyn Array> = Arc::new(StringArray::from(vec!["5.0SP2"]));
    let batch = RecordBatch::try_new(source_schema, vec![Arc::clone(&source)]).unwrap();
    let exact = source_root.cast_arrow_batch(batch.clone(), false).unwrap();
    assert!(Arc::ptr_eq(exact.column(0), &source));

    let text_root = root(DataType::Utf8.required_field("begin_string"));
    let rendered = text_root.cast_arrow_batch(batch.clone(), false).unwrap();
    assert_eq!(rendered.column(0).data_type(), &ArrowDataType::Utf8);
    let numeric_root = root(DataType::Int32.required_field("begin_string"));
    let refused = numeric_root
        .cast_arrow_batch(batch, false)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("version"), "{refused}");

    assert!(version("5.0SP2") < version("5.0SP10"));
    assert!("5.0SP10" < "5.0SP2");
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
