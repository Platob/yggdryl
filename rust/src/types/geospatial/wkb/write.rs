//! Canonical WKT rendering from the decoded geometry model.

use super::{Coord, Dimensions, Geometry};

pub(super) fn into_wkt(geometry: &Geometry) -> String {
    let mut text = String::new();
    write_geometry(&mut text, geometry);
    text
}

/// Write one geometry as WKT: tag, dimension marker, then the body.
fn write_geometry(text: &mut String, geometry: &Geometry) {
    match geometry {
        Geometry::Point {
            dimensions,
            coordinate,
        } => {
            write_tag(text, "POINT", *dimensions);
            match coordinate {
                Some(coordinate) => {
                    text.push('(');
                    write_coordinate(text, coordinate);
                    text.push(')');
                }
                None => text.push_str("EMPTY"),
            }
        }
        Geometry::LineString {
            dimensions,
            coordinates,
        } => {
            write_tag(text, "LINESTRING", *dimensions);
            write_ring(text, coordinates);
        }
        Geometry::Polygon { dimensions, rings } => {
            write_tag(text, "POLYGON", *dimensions);
            write_sequence(text, rings, |text, ring| write_ring(text, ring));
        }
        Geometry::MultiPoint { dimensions, points } => {
            write_tag(text, "MULTIPOINT", *dimensions);
            write_sequence(text, points, |text, point| match point {
                Some(coordinate) => {
                    text.push('(');
                    write_coordinate(text, coordinate);
                    text.push(')');
                }
                None => text.push_str("EMPTY"),
            });
        }
        Geometry::MultiLineString { dimensions, lines } => {
            write_tag(text, "MULTILINESTRING", *dimensions);
            write_sequence(text, lines, |text, line| write_ring(text, line));
        }
        Geometry::MultiPolygon {
            dimensions,
            polygons,
        } => {
            write_tag(text, "MULTIPOLYGON", *dimensions);
            write_sequence(text, polygons, |text, rings| {
                write_sequence(text, rings, |text, ring| write_ring(text, ring));
            });
        }
        Geometry::GeometryCollection {
            dimensions,
            geometries,
        } => {
            write_tag(text, "GEOMETRYCOLLECTION", *dimensions);
            write_sequence(text, geometries, write_geometry);
        }
    }
}

/// Write the uppercase tag and, for Z, M, or ZM, its dimension marker.
fn write_tag(text: &mut String, tag: &str, dimensions: Dimensions) {
    text.push_str(tag);
    let marker = dimensions.wkt_marker();
    if !marker.is_empty() {
        text.push(' ');
        text.push_str(marker);
    }
    text.push(' ');
}

/// Write a comma-separated run between parentheses, or `EMPTY` when the run
/// holds nothing, which is WKT's one spelling for absence.
fn write_sequence<T>(text: &mut String, items: &[T], mut write_item: impl FnMut(&mut String, &T)) {
    if items.is_empty() {
        text.push_str("EMPTY");
        return;
    }
    text.push('(');
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            text.push_str(", ");
        }
        write_item(text, item);
    }
    text.push(')');
}

/// Write one coordinate run - a linestring body or a polygon ring.
fn write_ring(text: &mut String, coordinates: &[Coord]) {
    write_sequence(text, coordinates, write_coordinate);
}

/// Write one position, axes separated by single spaces.
fn write_coordinate(text: &mut String, coordinate: &Coord) {
    text.push_str(&format!("{} {}", coordinate.x, coordinate.y));
    if let Some(z) = coordinate.z {
        text.push_str(&format!(" {z}"));
    }
    if let Some(m) = coordinate.m {
        text.push_str(&format!(" {m}"));
    }
}
