use super::super::{DataType, GeospatialParameters};
use crate::{DataTypeId, DataTypeKind, EdgeAlgorithm};
use crate::{Field, Scalar};

#[test]
fn bare_variant_and_the_member_sugar_parse_to_different_types() {
    // The parenthesis disambiguates, deterministically, in one branch:
    // bare `variant` is the self-describing datatype, and the member form
    // stays the dense-union input sugar. Side by side, so the
    // disambiguation can never be broken silently.
    let bare: DataType = "variant".parse().unwrap();
    assert_eq!(bare, DataType::Variant);

    let sugar: DataType = "variant(a: int32, b: utf8)".parse().unwrap();
    assert_eq!(sugar.id(), DataTypeId::Union);
    assert!(sugar.to_string().starts_with("union(dense,"), "{sugar}");

    // The union spelling never round-trips as `variant`, so nothing that
    // reads canonical text is affected by the new bare meaning.
    let redisplayed: DataType = sugar.to_string().parse().unwrap();
    assert_eq!(redisplayed, sugar);
}

#[test]
fn every_spelling_of_the_geospatial_pair_parses_and_round_trips() {
    for (spelling, canonical) in [
        ("variant", "variant"),
        ("geometry", "geometry"),
        ("geometry()", "geometry"),
        ("geometry('OGC:CRS84')", "geometry"),
        ("geometry('EPSG:4326')", "geometry(\"EPSG:4326\")"),
        ("geography", "geography"),
        ("geography('OGC:CRS84')", "geography"),
        ("geography('OGC:CRS84', 'spherical')", "geography"),
        (
            "geography('OGC:CRS84', 'vincenty')",
            "geography(\"OGC:CRS84\",\"vincenty\")",
        ),
        ("GEOMETRY('EPSG:4326')", "geometry(\"EPSG:4326\")"),
        (
            "geography('EPSG:4326', 'KARNEY')",
            "geography(\"EPSG:4326\",\"karney\")",
        ),
    ] {
        let parsed: DataType = spelling.parse().unwrap_or_else(|error| {
            panic!("{spelling} must parse: {error}");
        });
        assert_eq!(parsed.to_string(), canonical, "{spelling}");
        let reparsed: DataType = parsed.to_string().parse().unwrap();
        assert_eq!(reparsed, parsed, "{spelling}");
    }
}

#[test]
fn adversarial_geospatial_spellings_are_refused_by_name() {
    // A geometry given an edge algorithm: straight planar lines need none.
    let error = "geometry('EPSG:4326', 'vincenty')"
        .parse::<DataType>()
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(
            "expected no edge algorithm for geometry, got \"vincenty\"; \
             geography is the type whose edges take one"
        ),
        "{error}"
    );

    // An unknown algorithm reports the accepted vocabulary.
    let error = "geography('OGC:CRS84', 'euclidean')"
        .parse::<DataType>()
        .unwrap_err()
        .to_string();
    assert!(error.contains("spherical"), "{error}");
    assert!(error.contains("\"euclidean\""), "{error}");

    // An unterminated CRS string fails at a byte position.
    assert!("geometry('EPSG:4326".parse::<DataType>().is_err());

    // An empty CRS names nothing; the absent spelling fills the default.
    let error = "geometry('')".parse::<DataType>().unwrap_err().to_string();
    assert!(error.contains("empty"), "{error}");
}

#[test]
fn the_constructors_fill_and_refuse_what_the_grammar_does() {
    assert_eq!(DataType::variant(), DataType::Variant);

    let bare = DataType::geometry(None).unwrap();
    let explicit = DataType::geometry(Some("OGC:CRS84")).unwrap();
    // Filling the default makes the two spellings one type.
    assert_eq!(bare, explicit);

    let geography = DataType::geography(None, None).unwrap();
    let DataType::Geography(geospatial) = &geography else {
        panic!("expected a geography, got {geography}");
    };
    assert_eq!(geospatial.algorithm(), Some(EdgeAlgorithm::Spherical));
    assert!(geospatial.has_default_crs());

    // The shared value refuses what the grammar refuses.
    assert!(GeospatialParameters::geometry(Some("")).is_err());
    assert_eq!(
        GeospatialParameters::geometry(Some("EPSG:3857"))
            .unwrap()
            .crs(),
        "EPSG:3857"
    );
}

#[test]
fn identity_kind_and_nesting_answer_for_the_new_variants() {
    assert_eq!(DataType::Variant.id(), DataTypeId::Variant);
    assert_eq!(DataType::Variant.kind(), DataTypeKind::Nested);
    // A variant holds a tree, so it is nested; a geometry is one value.
    assert!(DataType::Variant.is_nested());

    let geometry = DataType::geometry(None).unwrap();
    assert_eq!(geometry.id(), DataTypeId::Geometry);
    assert_eq!(geometry.kind(), DataTypeKind::Geospatial);
    assert!(!geometry.is_nested());

    let geography = DataType::geography(None, None).unwrap();
    assert_eq!(geography.id(), DataTypeId::Geography);
    assert_eq!(geography.kind(), DataTypeKind::Geospatial);
}

#[test]
fn serde_and_the_structural_value_round_trip() {
    for dtype in [
        DataType::Variant,
        DataType::geometry(Some("EPSG:4326")).unwrap(),
        DataType::geography(Some("EPSG:4326"), Some(EdgeAlgorithm::Karney)).unwrap(),
        DataType::geography(None, None).unwrap(),
    ] {
        let json = dtype.clone().into_json().unwrap();
        assert_eq!(DataType::from_json(&json).unwrap(), dtype, "{json}");

        let value = dtype.clone().into_value();
        assert_eq!(DataType::from_value(value).unwrap(), dtype);
    }
}

#[test]
fn ordering_and_hashing_are_consistent_for_the_new_variants() {
    let one = DataType::geometry(Some("EPSG:4326")).unwrap();
    let two = DataType::geometry(Some("EPSG:4326")).unwrap();
    assert_eq!(one, two);
    assert_eq!(one.cmp(&two), std::cmp::Ordering::Equal);

    // Hash agrees with Eq, checked pairwise: a datatype carries interior
    // caches elsewhere, so the set spelling trips the mutable-key lint
    // where the direct hash comparison says the same thing.
    fn hash_of(value: &DataType) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }
    assert_eq!(hash_of(&one), hash_of(&two));

    // Distinct parameters are distinct types, ordered deterministically.
    let other = DataType::geometry(Some("EPSG:3857")).unwrap();
    assert_ne!(two, other);
    assert_eq!(two.cmp(&other), other.cmp(&two).reverse());
}

#[test]
fn defaults_are_a_present_variant_null_and_a_point_empty() {
    // The variant's present zero value is the variant null - a value the
    // encoding can spell, not an absence - so a required variant column
    // has a default.
    let variant = DataType::Variant.required_field("payload");
    assert_eq!(variant.default_value().unwrap(), Scalar::Null);

    // The geospatial default is the conventional empty geometry, in the
    // canonical value spelling.
    let geometry = DataType::geometry(None).unwrap().required_field("shape");
    let default = geometry.default_value().unwrap();
    assert!(matches!(default, Scalar::Geospatial(_)), "{default:?}");
    let bytes = default.as_wkb().expect("a WKB payload");
    assert_eq!(
        crate::types::geospatial::wkb::into_wkt(bytes).unwrap(),
        "POINT EMPTY",
        "the default is POINT EMPTY"
    );
}

#[test]
fn rows_validate_through_the_new_columns() {
    let root = |field: Field| {
        DataType::from_fields([field])
            .unwrap()
            .required_field("row")
    };
    let row = |value: Scalar| Scalar::from_sequence([value]);

    // A variant column validates any value, null included: the variant
    // null is a value the encoding can spell, so a required variant
    // holding it is present, not absent.
    let variant = root(DataType::Variant.required_field("payload"));
    variant.validate_value(&row(Scalar::Null)).unwrap();
    variant.validate_value(&row(Scalar::from(12_i64))).unwrap();
    variant
        .validate_value(&row(Scalar::from_mapping([(
            Scalar::from("a"),
            Scalar::from(1_i64),
        )])
        .unwrap()))
        .unwrap();

    // A geospatial column validates the WKB payload's own framing.
    let geometry = root(DataType::geometry(None).unwrap().required_field("shape"));
    let point: Vec<u8> = {
        // POINT (1 2), little-endian ISO WKB.
        let mut bytes = vec![0x01, 0x01, 0x00, 0x00, 0x00];
        bytes.extend_from_slice(&1.0_f64.to_le_bytes());
        bytes.extend_from_slice(&2.0_f64.to_le_bytes());
        bytes
    };
    geometry.validate_value(&row(Scalar::from(point))).unwrap();

    let refusal = geometry
        .validate_value(&row(Scalar::from(vec![0x01_u8, 0xFF])))
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("geometry"), "{refusal}");
    let not_bytes = geometry
        .validate_value(&row(Scalar::from("POINT (1 2)")))
        .unwrap_err()
        .to_string();
    assert!(not_bytes.contains("geometry"), "{not_bytes}");
}

#[test]
fn compatibility_rows_answer_for_every_target() {
    use crate::Scheme;

    let schema = DataType::from_fields([
        DataType::Variant.nullable_field("payload"),
        DataType::geometry(None).unwrap().nullable_field("shape"),
        DataType::geography(None, None)
            .unwrap()
            .nullable_field("region"),
    ])
    .unwrap()
    .required_field("row");

    // Iceberg v3 owns all three, parameters included: pass unchanged.
    let iceberg = schema.clone().into_scheme_compat(&Scheme::ICEBERG).unwrap();
    assert_eq!(iceberg, schema);

    // The engines without the types refuse by name, with a path.
    for scheme in [Scheme::SPARK, Scheme::POLARS, Scheme::PANDAS] {
        let error = schema
            .clone()
            .into_scheme_compat(&scheme)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("payload") || error.contains("variant"),
            "{error}"
        );
    }
}
