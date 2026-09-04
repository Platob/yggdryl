//! Well-Known Binary geometries: what parses, what bounds, and what prints.

use crate::types::geospatial::wkb::{Coord, Dimensions};

mod identity {
    use std::collections::{BTreeSet, HashSet};
    use std::hash::Hash;

    use crate::types::geospatial::wkb::{BoundingBox, Coord, Geometry};

    #[test]
    fn public_geometry_values_have_total_float_identity() {
        fn assert_traits<T: Clone + Eq + Hash + Ord>() {}
        assert_traits::<Coord>();
        assert_traits::<BoundingBox>();
        assert_traits::<Geometry>();

        let first = Coord {
            x: f64::from_bits(0x7ff8_0000_0000_0001),
            y: 1.0,
            z: None,
            m: None,
        };
        let same_nan = Coord {
            x: f64::from_bits(0x7ff8_0000_0000_0042),
            ..first
        };
        assert_eq!(first, same_nan);
        assert_eq!(HashSet::from([first, same_nan]).len(), 1);

        let positive_zero = Coord { x: 0.0, ..first };
        let negative_zero = Coord { x: -0.0, ..first };
        assert_ne!(positive_zero, negative_zero);
        assert_eq!(BTreeSet::from([positive_zero, negative_zero]).len(), 2);

        let bounds = BoundingBox {
            xmin: f64::NAN,
            xmax: 1.0,
            ymin: 2.0,
            ymax: 3.0,
            zmin: None,
            zmax: None,
            mmin: None,
            mmax: None,
        };
        let same_bounds = BoundingBox {
            xmin: f64::from_bits(0x7ff8_0000_0000_0042),
            ..bounds
        };
        assert_eq!(bounds, same_bounds);
    }
}

/// Both byte orders every geometry must read from; `true` is little endian,
/// which is what the order byte itself spells.
const ORDERS: [bool; 2] = [true, false];

/// Every dimensionality beside its ISO code offset and its WKT marker.
const DIMENSIONS: [(Dimensions, u32, &str); 4] = [
    (Dimensions::Xy, 0, ""),
    (Dimensions::Xyz, 1_000, " Z"),
    (Dimensions::Xym, 2_000, " M"),
    (Dimensions::Xyzm, 3_000, " ZM"),
];

/// Append one geometry header: the order byte, then the type code in that order.
fn push_header(bytes: &mut Vec<u8>, little: bool, code: u32) {
    bytes.push(u8::from(little));
    push_u32(bytes, little, code);
}

/// Append one unsigned 32-bit count or code in the asked byte order.
fn push_u32(bytes: &mut Vec<u8>, little: bool, value: u32) {
    if little {
        bytes.extend(value.to_le_bytes());
    } else {
        bytes.extend(value.to_be_bytes());
    }
}

/// Append a run of doubles in the asked byte order.
fn push_doubles(bytes: &mut Vec<u8>, little: bool, values: &[f64]) {
    for value in values {
        if little {
            bytes.extend(value.to_le_bytes());
        } else {
            bytes.extend(value.to_be_bytes());
        }
    }
}

/// The number of doubles one coordinate holds under `dimensions`.
fn axes(dimensions: Dimensions) -> usize {
    2 + usize::from(dimensions.has_z()) + usize::from(dimensions.has_m())
}

/// One coordinate's values under `dimensions`, counting up from `start` so
/// every axis is distinguishable in an assertion.
fn axis_values(dimensions: Dimensions, start: f64) -> Vec<f64> {
    [0.0, 1.0, 2.0, 3.0][..axes(dimensions)]
        .iter()
        .map(|offset| start + offset)
        .collect()
}

/// The [`Coord`] the parser should build from one run of axis values.
fn model_coordinate(dimensions: Dimensions, values: &[f64]) -> Coord {
    let z = dimensions.has_z().then(|| values[2]);
    let m = dimensions
        .has_m()
        .then(|| values[if dimensions.has_z() { 3 } else { 2 }]);
    Coord {
        x: values[0],
        y: values[1],
        z,
        m,
    }
}

/// The WKT spelling of one coordinate: its axis values joined by spaces.
fn wkt_coordinate(values: &[f64]) -> String {
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

mod reading {
    use super::{
        DIMENSIONS, ORDERS, axis_values, model_coordinate, push_doubles, push_header, push_u32,
        wkt_coordinate,
    };
    use crate::types::geospatial::wkb::{Coord, Geometry, into_wkt};

    #[test]
    fn a_point_parses_in_every_dimensionality_and_byte_order() {
        for (dimensions, offset, marker) in DIMENSIONS {
            for little in ORDERS {
                let values = axis_values(dimensions, 1.5);
                let mut bytes = Vec::new();
                push_header(&mut bytes, little, 1 + offset);
                push_doubles(&mut bytes, little, &values);

                let geometry = Geometry::from_slice(&bytes).unwrap();
                assert_eq!(
                    geometry,
                    Geometry::Point {
                        dimensions,
                        coordinate: Some(model_coordinate(dimensions, &values)),
                    }
                );
                assert_eq!(geometry.dimensions(), dimensions);
                assert_eq!(geometry.type_id(), 1 + offset);
                assert!(!geometry.is_empty());
                assert_eq!(
                    into_wkt(&bytes).unwrap(),
                    format!("POINT{marker} ({})", wkt_coordinate(&values))
                );
            }
        }
    }

    #[test]
    fn a_linestring_parses_in_every_dimensionality_and_byte_order() {
        for (dimensions, offset, marker) in DIMENSIONS {
            for little in ORDERS {
                let first = axis_values(dimensions, 1.5);
                let second = axis_values(dimensions, 10.5);
                let mut bytes = Vec::new();
                push_header(&mut bytes, little, 2 + offset);
                push_u32(&mut bytes, little, 2);
                push_doubles(&mut bytes, little, &first);
                push_doubles(&mut bytes, little, &second);

                let geometry = Geometry::from_slice(&bytes).unwrap();
                assert_eq!(
                    geometry,
                    Geometry::LineString {
                        dimensions,
                        coordinates: vec![
                            model_coordinate(dimensions, &first),
                            model_coordinate(dimensions, &second),
                        ],
                    }
                );
                assert_eq!(geometry.type_id(), 2 + offset);
                assert_eq!(
                    into_wkt(&bytes).unwrap(),
                    format!(
                        "LINESTRING{marker} ({}, {})",
                        wkt_coordinate(&first),
                        wkt_coordinate(&second)
                    )
                );
            }
        }
    }

    #[test]
    fn a_polygon_parses_in_every_dimensionality_and_byte_order() {
        for (dimensions, offset, marker) in DIMENSIONS {
            for little in ORDERS {
                let ring: Vec<Vec<f64>> = [0.5, 10.5, 20.5]
                    .iter()
                    .map(|start| axis_values(dimensions, *start))
                    .collect();
                let hole: Vec<Vec<f64>> = [30.5, 40.5, 50.5]
                    .iter()
                    .map(|start| axis_values(dimensions, *start))
                    .collect();
                let mut bytes = Vec::new();
                push_header(&mut bytes, little, 3 + offset);
                push_u32(&mut bytes, little, 2);
                for coordinates in [&ring, &hole] {
                    push_u32(&mut bytes, little, 3);
                    for values in coordinates {
                        push_doubles(&mut bytes, little, values);
                    }
                }

                let model_ring = |coordinates: &[Vec<f64>]| -> Vec<Coord> {
                    coordinates
                        .iter()
                        .map(|values| model_coordinate(dimensions, values))
                        .collect()
                };
                let geometry = Geometry::from_slice(&bytes).unwrap();
                assert_eq!(
                    geometry,
                    Geometry::Polygon {
                        dimensions,
                        rings: vec![model_ring(&ring), model_ring(&hole)],
                    }
                );
                assert_eq!(geometry.type_id(), 3 + offset);

                let wkt_ring = |coordinates: &[Vec<f64>]| -> String {
                    coordinates
                        .iter()
                        .map(|values| wkt_coordinate(values))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                assert_eq!(
                    into_wkt(&bytes).unwrap(),
                    format!(
                        "POLYGON{marker} (({}), ({}))",
                        wkt_ring(&ring),
                        wkt_ring(&hole)
                    )
                );
            }
        }
    }

    #[test]
    fn a_multipoint_parses_with_each_member_carrying_its_own_byte_order() {
        for (dimensions, offset, marker) in DIMENSIONS {
            for little in ORDERS {
                let first = axis_values(dimensions, 1.5);
                let second = axis_values(dimensions, 10.5);
                let mut bytes = Vec::new();
                push_header(&mut bytes, little, 4 + offset);
                push_u32(&mut bytes, little, 2);
                // Each nested geometry declares its own order, so the second
                // member deliberately flips it.
                push_header(&mut bytes, little, 1 + offset);
                push_doubles(&mut bytes, little, &first);
                push_header(&mut bytes, !little, 1 + offset);
                push_doubles(&mut bytes, !little, &second);

                let geometry = Geometry::from_slice(&bytes).unwrap();
                assert_eq!(
                    geometry,
                    Geometry::MultiPoint {
                        dimensions,
                        points: vec![
                            Some(model_coordinate(dimensions, &first)),
                            Some(model_coordinate(dimensions, &second)),
                        ],
                    }
                );
                assert_eq!(geometry.type_id(), 4 + offset);
                assert_eq!(
                    into_wkt(&bytes).unwrap(),
                    format!(
                        "MULTIPOINT{marker} (({}), ({}))",
                        wkt_coordinate(&first),
                        wkt_coordinate(&second)
                    )
                );
            }
        }
    }

    #[test]
    fn a_multilinestring_parses_in_every_dimensionality_and_byte_order() {
        for (dimensions, offset, marker) in DIMENSIONS {
            for little in ORDERS {
                let first = axis_values(dimensions, 1.5);
                let second = axis_values(dimensions, 10.5);
                let mut bytes = Vec::new();
                push_header(&mut bytes, little, 5 + offset);
                push_u32(&mut bytes, little, 2);
                for values in [&first, &second] {
                    push_header(&mut bytes, little, 2 + offset);
                    push_u32(&mut bytes, little, 1);
                    push_doubles(&mut bytes, little, values);
                }

                let geometry = Geometry::from_slice(&bytes).unwrap();
                assert_eq!(
                    geometry,
                    Geometry::MultiLineString {
                        dimensions,
                        lines: vec![
                            vec![model_coordinate(dimensions, &first)],
                            vec![model_coordinate(dimensions, &second)],
                        ],
                    }
                );
                assert_eq!(geometry.type_id(), 5 + offset);
                assert_eq!(
                    into_wkt(&bytes).unwrap(),
                    format!(
                        "MULTILINESTRING{marker} (({}), ({}))",
                        wkt_coordinate(&first),
                        wkt_coordinate(&second)
                    )
                );
            }
        }
    }

    #[test]
    fn a_multipolygon_parses_in_every_dimensionality_and_byte_order() {
        for (dimensions, offset, marker) in DIMENSIONS {
            for little in ORDERS {
                let ring: Vec<Vec<f64>> = [0.5, 10.5, 20.5]
                    .iter()
                    .map(|start| axis_values(dimensions, *start))
                    .collect();
                let mut bytes = Vec::new();
                push_header(&mut bytes, little, 6 + offset);
                push_u32(&mut bytes, little, 1);
                push_header(&mut bytes, little, 3 + offset);
                push_u32(&mut bytes, little, 1);
                push_u32(&mut bytes, little, 3);
                for values in &ring {
                    push_doubles(&mut bytes, little, values);
                }

                let geometry = Geometry::from_slice(&bytes).unwrap();
                assert_eq!(
                    geometry,
                    Geometry::MultiPolygon {
                        dimensions,
                        polygons: vec![vec![
                            ring.iter()
                                .map(|values| model_coordinate(dimensions, values))
                                .collect(),
                        ]],
                    }
                );
                assert_eq!(geometry.type_id(), 6 + offset);
                let spelled = ring
                    .iter()
                    .map(|values| wkt_coordinate(values))
                    .collect::<Vec<_>>()
                    .join(", ");
                assert_eq!(
                    into_wkt(&bytes).unwrap(),
                    format!("MULTIPOLYGON{marker} ((({spelled})))")
                );
            }
        }
    }

    #[test]
    fn a_collection_parses_in_every_dimensionality_and_byte_order() {
        for (dimensions, offset, marker) in DIMENSIONS {
            for little in ORDERS {
                let point = axis_values(dimensions, 1.5);
                let line = axis_values(dimensions, 10.5);
                let mut bytes = Vec::new();
                push_header(&mut bytes, little, 7 + offset);
                push_u32(&mut bytes, little, 2);
                push_header(&mut bytes, little, 1 + offset);
                push_doubles(&mut bytes, little, &point);
                push_header(&mut bytes, little, 2 + offset);
                push_u32(&mut bytes, little, 1);
                push_doubles(&mut bytes, little, &line);

                let geometry = Geometry::from_slice(&bytes).unwrap();
                assert_eq!(
                    geometry,
                    Geometry::GeometryCollection {
                        dimensions,
                        geometries: vec![
                            Geometry::Point {
                                dimensions,
                                coordinate: Some(model_coordinate(dimensions, &point)),
                            },
                            Geometry::LineString {
                                dimensions,
                                coordinates: vec![model_coordinate(dimensions, &line)],
                            },
                        ],
                    }
                );
                assert_eq!(geometry.type_id(), 7 + offset);
                assert_eq!(
                    into_wkt(&bytes).unwrap(),
                    format!(
                        "GEOMETRYCOLLECTION{marker} (POINT{marker} ({}), LINESTRING{marker} ({}))",
                        wkt_coordinate(&point),
                        wkt_coordinate(&line)
                    )
                );
            }
        }
    }
}

mod empties {
    use super::{ORDERS, push_doubles, push_header, push_u32};
    use crate::types::geospatial::wkb::{Dimensions, Geometry, into_wkt};

    #[test]
    fn a_zero_count_reads_as_empty_for_every_container_type() {
        for (code, tag) in [
            (2, "LINESTRING"),
            (3, "POLYGON"),
            (4, "MULTIPOINT"),
            (5, "MULTILINESTRING"),
            (6, "MULTIPOLYGON"),
            (7, "GEOMETRYCOLLECTION"),
        ] {
            for little in ORDERS {
                let mut bytes = Vec::new();
                push_header(&mut bytes, little, code);
                push_u32(&mut bytes, little, 0);

                let geometry = Geometry::from_slice(&bytes).unwrap();
                assert!(geometry.is_empty(), "type {code} should read as empty");
                assert_eq!(into_wkt(&bytes).unwrap(), format!("{tag} EMPTY"));
            }
        }
    }

    #[test]
    fn a_nan_point_reads_as_empty_rather_than_failing() {
        for little in ORDERS {
            let mut bytes = Vec::new();
            push_header(&mut bytes, little, 1);
            push_doubles(&mut bytes, little, &[f64::NAN, f64::NAN]);

            let geometry = Geometry::from_slice(&bytes).unwrap();
            assert_eq!(
                geometry,
                Geometry::Point {
                    dimensions: Dimensions::Xy,
                    coordinate: None,
                }
            );
            assert!(geometry.is_empty());
            assert_eq!(into_wkt(&bytes).unwrap(), "POINT EMPTY");
        }
    }

    #[test]
    fn an_empty_point_keeps_its_dimension_marker() {
        // The coordinates are gone, but the type code still says ZM, and the
        // model must not forget it on the way to the text.
        let mut bytes = Vec::new();
        push_header(&mut bytes, true, 3_001);
        push_doubles(&mut bytes, true, &[f64::NAN, f64::NAN, f64::NAN, f64::NAN]);

        assert_eq!(into_wkt(&bytes).unwrap(), "POINT ZM EMPTY");
    }

    #[test]
    fn an_empty_member_stays_visible_inside_a_multipoint() {
        let mut bytes = Vec::new();
        push_header(&mut bytes, true, 4);
        push_u32(&mut bytes, true, 2);
        push_header(&mut bytes, true, 1);
        push_doubles(&mut bytes, true, &[f64::NAN, f64::NAN]);
        push_header(&mut bytes, true, 1);
        push_doubles(&mut bytes, true, &[1.5, 2.5]);

        assert_eq!(into_wkt(&bytes).unwrap(), "MULTIPOINT (EMPTY, (1.5 2.5))");
    }
}

mod nesting {
    use super::{push_doubles, push_header, push_u32};
    use crate::DataType;
    use crate::types::geospatial::wkb::{Coord, Dimensions, Geometry, bounding_box, into_wkt};

    /// Wrap one XY point in `levels` single-member collections.
    fn nested_collections(levels: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        for _ in 0..levels {
            push_header(&mut bytes, true, 7);
            push_u32(&mut bytes, true, 1);
        }
        push_header(&mut bytes, true, 1);
        push_doubles(&mut bytes, true, &[1.5, 2.5]);
        bytes
    }

    #[test]
    fn a_collection_nests_two_levels_deep() {
        let mut bytes = Vec::new();
        push_header(&mut bytes, true, 7);
        push_u32(&mut bytes, true, 2);
        push_header(&mut bytes, true, 7);
        push_u32(&mut bytes, true, 1);
        push_header(&mut bytes, true, 1);
        push_doubles(&mut bytes, true, &[1.5, 2.5]);
        push_header(&mut bytes, true, 2);
        push_u32(&mut bytes, true, 0);

        let geometry = Geometry::from_slice(&bytes).unwrap();
        assert_eq!(
            geometry,
            Geometry::GeometryCollection {
                dimensions: Dimensions::Xy,
                geometries: vec![
                    Geometry::GeometryCollection {
                        dimensions: Dimensions::Xy,
                        geometries: vec![Geometry::Point {
                            dimensions: Dimensions::Xy,
                            coordinate: Some(Coord {
                                x: 1.5,
                                y: 2.5,
                                z: None,
                                m: None,
                            }),
                        }],
                    },
                    Geometry::LineString {
                        dimensions: Dimensions::Xy,
                        coordinates: vec![],
                    },
                ],
            }
        );
        assert_eq!(
            into_wkt(&bytes).unwrap(),
            "GEOMETRYCOLLECTION (GEOMETRYCOLLECTION (POINT (1.5 2.5)), LINESTRING EMPTY)"
        );
    }

    #[test]
    fn nesting_at_the_shared_limit_is_refused_and_below_it_is_not() {
        let below = nested_collections(DataType::PARSE_RECURSION_LIMIT - 1);
        assert!(Geometry::from_slice(&below).is_ok());

        let over = nested_collections(DataType::PARSE_RECURSION_LIMIT);
        let error = Geometry::from_slice(&over).unwrap_err();
        assert!(
            error.to_string().contains(&format!(
                "geometry nesting exceeds the hard limit of {}",
                DataType::PARSE_RECURSION_LIMIT
            )),
            "unexpected message: {error}"
        );
        // The streaming pass is bounded by exactly the same limit.
        assert!(bounding_box(&over).is_err());
    }
}

mod refusals {
    use super::{push_doubles, push_header, push_u32};
    use crate::Error;
    use crate::types::geospatial::wkb::{Geometry, bounding_box};

    /// Unwrap the codec variant every WKB refusal must carry.
    fn codec_parts(error: &Error) -> (&'static str, usize) {
        let Error::Codec {
            format, position, ..
        } = error
        else {
            panic!("expected a codec error, got {error:?}")
        };
        (format, *position)
    }

    #[test]
    fn a_truncated_coordinate_is_refused_at_its_byte_position() {
        let mut bytes = Vec::new();
        push_header(&mut bytes, true, 1);
        push_doubles(&mut bytes, true, &[1.5]);
        bytes.extend(&2.5_f64.to_le_bytes()[..4]);

        // The x at byte 5 reads whole; the y wanted 8 bytes at byte 13.
        let error = Geometry::from_slice(&bytes).unwrap_err();
        assert_eq!(codec_parts(&error), ("wkb", 13));
        assert_eq!(
            error.to_string(),
            "invalid wkb data at byte 13: expected 8 bytes of coordinate, got 4"
        );
        // The streaming pass refuses the same buffer at the same byte.
        assert_eq!(codec_parts(&bounding_box(&bytes).unwrap_err()), ("wkb", 13));
    }

    #[test]
    fn an_empty_buffer_is_refused_at_byte_zero() {
        let error = Geometry::from_slice(&[]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "invalid wkb data at byte 0: expected 1 byte of byte order, got 0"
        );
    }

    #[test]
    fn an_unknown_byte_order_is_refused() {
        let error = Geometry::from_slice(&[2, 1, 0, 0, 0]).unwrap_err();
        assert_eq!(codec_parts(&error), ("wkb", 0));
        assert_eq!(
            error.to_string(),
            "invalid wkb data at byte 0: expected byte order 0 (big endian) or 1 (little endian), got 2"
        );
    }

    #[test]
    fn an_unknown_geometry_type_code_is_refused_by_name() {
        for code in [0, 8, 999, 4_001] {
            let mut bytes = Vec::new();
            push_header(&mut bytes, true, code);

            let error = Geometry::from_slice(&bytes).unwrap_err();
            assert_eq!(codec_parts(&error), ("wkb", 1));
            assert_eq!(
                error.to_string(),
                format!(
                    "invalid wkb data at byte 1: expected a geometry type code \
                     (1 through 7, plus 1000, 2000, or 3000 for Z, M, and ZM), got {code}"
                )
            );
        }
    }

    #[test]
    fn a_mistyped_member_of_a_multi_geometry_is_refused_by_name() {
        let mut bytes = Vec::new();
        push_header(&mut bytes, true, 4);
        push_u32(&mut bytes, true, 1);
        push_header(&mut bytes, true, 2);
        push_u32(&mut bytes, true, 0);

        let error = Geometry::from_slice(&bytes).unwrap_err();
        assert_eq!(codec_parts(&error), ("wkb", 9));
        assert_eq!(
            error.to_string(),
            "invalid wkb data at byte 9: expected a point in a multipoint, got a linestring"
        );
    }

    #[test]
    fn a_count_the_buffer_cannot_back_is_refused_before_allocating() {
        let mut bytes = Vec::new();
        push_header(&mut bytes, true, 2);
        push_u32(&mut bytes, true, u32::MAX);

        let error = Geometry::from_slice(&bytes).unwrap_err();
        assert_eq!(codec_parts(&error), ("wkb", 5));
        assert_eq!(
            error.to_string(),
            "invalid wkb data at byte 5: expected 4294967295 entries of at least \
             16 bytes each, got 0 bytes"
        );
    }

    #[test]
    fn trailing_bytes_after_one_geometry_are_refused() {
        let mut bytes = Vec::new();
        push_header(&mut bytes, true, 1);
        push_doubles(&mut bytes, true, &[1.5, 2.5]);
        bytes.push(0);

        let error = Geometry::from_slice(&bytes).unwrap_err();
        assert_eq!(codec_parts(&error), ("wkb", 21));
        assert_eq!(
            error.to_string(),
            "invalid wkb data at byte 21: expected the end of the buffer, got 1 trailing byte"
        );
    }
}

mod bounds {
    use super::{push_doubles, push_header, push_u32};
    use crate::types::geospatial::wkb::{BoundingBox, bounding_box};

    #[test]
    fn a_mixed_collection_bounds_every_member() {
        let mut bytes = Vec::new();
        push_header(&mut bytes, true, 7);
        push_u32(&mut bytes, true, 2);
        push_header(&mut bytes, true, 1);
        push_doubles(&mut bytes, true, &[10.0, -5.0]);
        push_header(&mut bytes, true, 2);
        push_u32(&mut bytes, true, 2);
        push_doubles(&mut bytes, true, &[0.0, 0.0, 3.0, 7.0]);

        let bounds = bounding_box(&bytes).unwrap();
        assert!(!bounds.is_empty());
        assert_eq!(
            bounds,
            BoundingBox {
                xmin: 0.0,
                xmax: 10.0,
                ymin: -5.0,
                ymax: 7.0,
                zmin: None,
                zmax: None,
                mmin: None,
                mmax: None,
            }
        );
    }

    #[test]
    fn a_zm_linestring_bounds_its_elevation_and_measure() {
        let mut bytes = Vec::new();
        push_header(&mut bytes, false, 3_002);
        push_u32(&mut bytes, false, 2);
        push_doubles(&mut bytes, false, &[1.0, 2.0, 3.0, 4.0]);
        push_doubles(&mut bytes, false, &[0.0, 5.0, -3.0, 9.0]);

        assert_eq!(
            bounding_box(&bytes).unwrap(),
            BoundingBox {
                xmin: 0.0,
                xmax: 1.0,
                ymin: 2.0,
                ymax: 5.0,
                zmin: Some(-3.0),
                zmax: Some(3.0),
                mmin: Some(4.0),
                mmax: Some(9.0),
            }
        );
    }

    #[test]
    fn an_empty_geometry_yields_the_empty_box() {
        let mut bytes = Vec::new();
        push_header(&mut bytes, true, 2);
        push_u32(&mut bytes, true, 0);

        let bounds = bounding_box(&bytes).unwrap();
        assert!(bounds.is_empty());
        assert_eq!(
            bounds,
            BoundingBox {
                xmin: f64::INFINITY,
                xmax: f64::NEG_INFINITY,
                ymin: f64::INFINITY,
                ymax: f64::NEG_INFINITY,
                zmin: None,
                zmax: None,
                mmin: None,
                mmax: None,
            }
        );
    }

    #[test]
    fn a_nan_point_bounds_nothing() {
        let mut bytes = Vec::new();
        push_header(&mut bytes, true, 7);
        push_u32(&mut bytes, true, 2);
        push_header(&mut bytes, true, 1);
        push_doubles(&mut bytes, true, &[f64::NAN, f64::NAN]);
        push_header(&mut bytes, true, 1);
        push_doubles(&mut bytes, true, &[1.0, 2.0]);

        assert_eq!(
            bounding_box(&bytes).unwrap(),
            BoundingBox {
                xmin: 1.0,
                xmax: 1.0,
                ymin: 2.0,
                ymax: 2.0,
                zmin: None,
                zmax: None,
                mmin: None,
                mmax: None,
            }
        );
    }
}

mod type_ids {
    use super::{push_doubles, push_header, push_u32};
    use crate::types::geospatial::wkb::geometry_type_ids;

    #[test]
    fn a_mixed_collection_reports_each_code_once_sorted() {
        let mut bytes = Vec::new();
        push_header(&mut bytes, true, 7);
        push_u32(&mut bytes, true, 3);
        push_header(&mut bytes, true, 1);
        push_doubles(&mut bytes, true, &[1.0, 2.0]);
        push_header(&mut bytes, true, 1_002);
        push_u32(&mut bytes, true, 1);
        push_doubles(&mut bytes, true, &[1.0, 2.0, 3.0]);
        push_header(&mut bytes, true, 4);
        push_u32(&mut bytes, true, 1);
        push_header(&mut bytes, true, 1);
        push_doubles(&mut bytes, true, &[4.0, 5.0]);

        // The collection, its plain point (also spelled by the multipoint's
        // member), the multipoint, and the Z linestring - once each, sorted.
        assert_eq!(geometry_type_ids(&bytes).unwrap(), vec![1, 4, 7, 1_002]);
    }
}

mod ewkb {
    use super::{push_doubles, push_u32};
    use crate::types::geospatial::wkb::{Coord, Dimensions, Geometry, geometry_type_ids};

    /// Append an EWKB header: flags live in the code's high bits, and the
    /// SRID follows the code when its flag is set.
    fn push_ewkb_header(bytes: &mut Vec<u8>, code: u32, srid: Option<u32>) {
        bytes.push(1);
        push_u32(bytes, true, code);
        if let Some(srid) = srid {
            push_u32(bytes, true, srid);
        }
    }

    #[test]
    fn an_ewkb_flagged_code_reads_like_the_iso_code() {
        let mut bytes = Vec::new();
        push_ewkb_header(&mut bytes, 1 | 0x8000_0000, None);
        push_doubles(&mut bytes, true, &[1.0, 2.0, 3.0]);

        let geometry = Geometry::from_slice(&bytes).unwrap();
        assert_eq!(
            geometry,
            Geometry::Point {
                dimensions: Dimensions::Xyz,
                coordinate: Some(Coord {
                    x: 1.0,
                    y: 2.0,
                    z: Some(3.0),
                    m: None,
                }),
            }
        );
        // The reported code is the ISO spelling, whatever spelling came in.
        assert_eq!(geometry.type_id(), 1_001);
        assert_eq!(geometry_type_ids(&bytes).unwrap(), vec![1_001]);
    }

    #[test]
    fn an_ewkb_srid_is_read_past_rather_than_refused() {
        let mut bytes = Vec::new();
        push_ewkb_header(&mut bytes, 1 | 0xC000_0000 | 0x2000_0000, Some(4_326));
        push_doubles(&mut bytes, true, &[1.0, 2.0, 3.0, 4.0]);

        let geometry = Geometry::from_slice(&bytes).unwrap();
        assert_eq!(geometry.dimensions(), Dimensions::Xyzm);
        assert_eq!(geometry.type_id(), 3_001);
    }
}

mod exactness {
    use super::{push_doubles, push_header};
    use crate::types::geospatial::wkb::{Geometry, into_wkt};

    #[test]
    fn extreme_coordinates_round_trip_bit_exactly() {
        // The widest, smallest, subnormal, and signed-zero doubles: each must
        // come back holding exactly the bits it was stored with.
        let values = [f64::MAX, -f64::MIN_POSITIVE, 5e-324, -0.0];
        let mut bytes = Vec::new();
        push_header(&mut bytes, false, 3_001);
        push_doubles(&mut bytes, false, &values);

        let geometry = Geometry::from_slice(&bytes).unwrap();
        let Geometry::Point {
            coordinate: Some(coordinate),
            ..
        } = geometry
        else {
            panic!("expected a point, got {geometry:?}")
        };
        assert_eq!(coordinate.x.to_bits(), f64::MAX.to_bits());
        assert_eq!(coordinate.y.to_bits(), (-f64::MIN_POSITIVE).to_bits());
        assert_eq!(coordinate.z.unwrap().to_bits(), 5e-324_f64.to_bits());
        assert_eq!(coordinate.m.unwrap().to_bits(), (-0.0_f64).to_bits());
    }

    #[test]
    fn wkt_prints_the_shortest_round_trip_decimal() {
        let mut bytes = Vec::new();
        push_header(&mut bytes, true, 1);
        push_doubles(&mut bytes, true, &[0.1, -2.5]);

        assert_eq!(into_wkt(&bytes).unwrap(), "POINT (0.1 -2.5)");
    }
}
