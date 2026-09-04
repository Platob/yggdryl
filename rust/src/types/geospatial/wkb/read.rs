//! Bounded WKB decoding and streaming inspection.

use smol_str::{SmolStr, format_smolstr};

use super::{
    Base, BoundingBox, Coord, Dimensions, EWKB_M, EWKB_SRID, EWKB_Z, Geometry, MEMBER_BYTES,
};
use crate::{DataType, Error, Result};

pub(super) fn geometry(bytes: &[u8]) -> Result<Geometry> {
    let mut cursor = Cursor::new(bytes);
    let geometry = read_geometry(&mut cursor, 0)?;
    cursor.finish()?;
    Ok(geometry)
}

pub(super) fn bounding_box(bytes: &[u8]) -> Result<BoundingBox> {
    let mut cursor = Cursor::new(bytes);
    let mut bounds = Bounds::new();
    scan_geometry(&mut cursor, 0, &mut bounds)?;
    cursor.finish()?;
    Ok(bounds.0)
}

pub(super) fn geometry_type_ids(bytes: &[u8]) -> Result<Vec<u32>> {
    let mut cursor = Cursor::new(bytes);
    let mut codes = TypeIds { codes: Vec::new() };
    scan_geometry(&mut cursor, 0, &mut codes)?;
    cursor.finish()?;
    Ok(codes.codes)
}

/// The order one geometry's multi-byte values are stored in. Each nested
/// geometry declares its own, so the order travels with the header.
#[derive(Clone, Copy, Debug)]
enum ByteOrder {
    Big,
    Little,
}

/// One decoded geometry header: the byte order its body uses, the shape,
/// and the axes its coordinates carry.
struct Header {
    order: ByteOrder,
    base: Base,
    dimensions: Dimensions,
}

impl Header {
    /// The ISO type code this header spells, whatever spelling it arrived in.
    const fn type_id(&self) -> u32 {
        self.base.code() + self.dimensions.iso_offset()
    }
}

/// A byte position over the input slice. Every read goes through it so every
/// error carries the exact offset of the bytes it wanted.
struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    /// The bytes not yet consumed.
    const fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }

    /// Consume exactly `len` bytes, or report what was wanted and what is left.
    fn take(&mut self, len: usize, what: &'static str) -> Result<&'a [u8]> {
        let Some(taken) = self.bytes.get(self.position..self.position + len) else {
            return Err(codec(
                self.position,
                format_smolstr!(
                    "expected {len} {word} of {what}, got {remaining}",
                    word = byte_word(len),
                    remaining = self.remaining()
                ),
            ));
        };
        self.position += len;
        Ok(taken)
    }

    /// Consume one byte.
    fn take_u8(&mut self, what: &'static str) -> Result<u8> {
        Ok(self.take(1, what)?[0])
    }

    /// Consume one unsigned 32-bit integer in the geometry's byte order.
    fn take_u32(&mut self, order: ByteOrder, what: &'static str) -> Result<u32> {
        let taken: [u8; 4] = self
            .take(4, what)?
            .try_into()
            .expect("four bytes were just taken");
        Ok(match order {
            ByteOrder::Big => u32::from_be_bytes(taken),
            ByteOrder::Little => u32::from_le_bytes(taken),
        })
    }

    /// Consume one IEEE 754 double in the geometry's byte order.
    fn take_f64(&mut self, order: ByteOrder) -> Result<f64> {
        let taken: [u8; 8] = self
            .take(8, "coordinate")?
            .try_into()
            .expect("eight bytes were just taken");
        Ok(match order {
            ByteOrder::Big => f64::from_be_bytes(taken),
            ByteOrder::Little => f64::from_le_bytes(taken),
        })
    }

    /// Refuse bytes trailing the value: a cell is exactly one geometry.
    fn finish(&self) -> Result<()> {
        if self.position == self.bytes.len() {
            return Ok(());
        }
        Err(codec(
            self.position,
            format_smolstr!(
                "expected the end of the buffer, got {remaining} trailing {word}",
                remaining = self.remaining(),
                word = byte_word(self.remaining())
            ),
        ))
    }
}

/// The unit word for a byte count, so a one-byte message reads as prose.
const fn byte_word(count: usize) -> &'static str {
    if count == 1 { "byte" } else { "bytes" }
}

/// Report a malformed WKB buffer at a byte position.
fn codec(position: usize, reason: SmolStr) -> Error {
    Error::Codec {
        format: "wkb",
        position,
        reason,
    }
}

/// Refuse a descent past the shared recursion limit, so a hostile buffer of
/// nested collections is bounded the way every other recursive parse in the
/// project is rather than by the size of the native stack.
fn check_depth(cursor: &Cursor<'_>, depth: usize) -> Result<()> {
    if depth >= DataType::PARSE_RECURSION_LIMIT {
        return Err(codec(
            cursor.position,
            format_smolstr!(
                "geometry nesting exceeds the hard limit of {}",
                DataType::PARSE_RECURSION_LIMIT
            ),
        ));
    }
    Ok(())
}

/// Read one geometry header: byte order, type code in either spelling, and
/// the SRID an EWKB code announces, which is read past rather than modeled.
fn read_header(cursor: &mut Cursor<'_>) -> Result<Header> {
    let order_position = cursor.position;
    let order = match cursor.take_u8("byte order")? {
        0 => ByteOrder::Big,
        1 => ByteOrder::Little,
        other => {
            return Err(codec(
                order_position,
                format_smolstr!(
                    "expected byte order 0 (big endian) or 1 (little endian), got {other}"
                ),
            ));
        }
    };
    let code_position = cursor.position;
    let code = cursor.take_u32(order, "geometry type code")?;
    // EWKB flags the extra axes and an SRID in the high bits; ISO adds a
    // thousands digit instead. Both spellings decode to the one Dimensions.
    let flagged_z = code & EWKB_Z != 0;
    let flagged_m = code & EWKB_M != 0;
    let has_srid = code & EWKB_SRID != 0;
    let plain = code & !(EWKB_Z | EWKB_M | EWKB_SRID);
    let level = plain / 1_000;
    let base = Base::from_code(plain % 1_000).filter(|_| level <= 3);
    let Some(base) = base else {
        return Err(codec(
            code_position,
            format_smolstr!(
                "expected a geometry type code (1 through 7, plus 1000, 2000, or 3000 for Z, M, and ZM), got {code}"
            ),
        ));
    };
    let dimensions = Dimensions::new(
        flagged_z || level == 1 || level == 3,
        flagged_m || level == 2 || level == 3,
    );
    if has_srid {
        // The SRID names the reference system; bounds and WKT are the same
        // in every system, so the value is skipped, not stored.
        cursor.take(4, "SRID")?;
    }
    Ok(Header {
        order,
        base,
        dimensions,
    })
}

/// Read a count and refuse one promising more entries than the buffer holds,
/// so a hostile count never sizes an allocation the input cannot back.
fn take_count(
    cursor: &mut Cursor<'_>,
    order: ByteOrder,
    what: &'static str,
    min_item_bytes: usize,
) -> Result<usize> {
    let position = cursor.position;
    let count = cursor.take_u32(order, what)? as usize;
    if count > cursor.remaining() / min_item_bytes {
        return Err(codec(
            position,
            format_smolstr!(
                "expected {count} entries of at least {min_item_bytes} bytes each, got {remaining} bytes",
                remaining = cursor.remaining()
            ),
        ));
    }
    Ok(count)
}

/// Read one coordinate under the header's byte order and axes.
fn read_coordinate(cursor: &mut Cursor<'_>, header: &Header) -> Result<Coord> {
    let x = cursor.take_f64(header.order)?;
    let y = cursor.take_f64(header.order)?;
    let z = header
        .dimensions
        .has_z()
        .then(|| cursor.take_f64(header.order))
        .transpose()?;
    let m = header
        .dimensions
        .has_m()
        .then(|| cursor.take_f64(header.order))
        .transpose()?;
    Ok(Coord { x, y, z, m })
}

/// Read a point body. `POINT EMPTY` has no zero-count spelling in WKB, so
/// writers emit NaN coordinates; a NaN position reads back as the empty point.
fn read_point(cursor: &mut Cursor<'_>, header: &Header) -> Result<Option<Coord>> {
    let coordinate = read_coordinate(cursor, header)?;
    Ok((!coordinate.x.is_nan() && !coordinate.y.is_nan()).then_some(coordinate))
}

/// Read a counted run of coordinates: a linestring body, or one polygon ring.
fn read_line(cursor: &mut Cursor<'_>, header: &Header) -> Result<Vec<Coord>> {
    let count = take_count(
        cursor,
        header.order,
        "coordinate count",
        header.dimensions.coordinate_len() * 8,
    )?;
    // The count was checked against the bytes present, so the reservation is
    // bounded by the input buffer.
    let mut coordinates = Vec::with_capacity(count);
    for _ in 0..count {
        coordinates.push(read_coordinate(cursor, header)?);
    }
    Ok(coordinates)
}

/// Read a polygon body: a counted run of rings.
fn read_rings(cursor: &mut Cursor<'_>, header: &Header) -> Result<Vec<Vec<Coord>>> {
    let count = take_count(cursor, header.order, "ring count", 4)?;
    let mut rings = Vec::with_capacity(count);
    for _ in 0..count {
        rings.push(read_line(cursor, header)?);
    }
    Ok(rings)
}

/// Read one member header of a multi geometry and refuse the wrong shape by
/// name, at the byte where the member started.
fn read_member_header(cursor: &mut Cursor<'_>, expected: Base, container: Base) -> Result<Header> {
    let position = cursor.position;
    let header = read_header(cursor)?;
    if header.base != expected {
        return Err(codec(
            position,
            format_smolstr!(
                "expected a {} in a {}, got a {}",
                expected.name(),
                container.name(),
                header.base.name()
            ),
        ));
    }
    Ok(header)
}

/// Read one whole geometry into the owned model.
fn read_geometry(cursor: &mut Cursor<'_>, depth: usize) -> Result<Geometry> {
    check_depth(cursor, depth)?;
    let header = read_header(cursor)?;
    let dimensions = header.dimensions;
    Ok(match header.base {
        Base::Point => Geometry::Point {
            dimensions,
            coordinate: read_point(cursor, &header)?,
        },
        Base::LineString => Geometry::LineString {
            dimensions,
            coordinates: read_line(cursor, &header)?,
        },
        Base::Polygon => Geometry::Polygon {
            dimensions,
            rings: read_rings(cursor, &header)?,
        },
        Base::MultiPoint => {
            let count = take_count(cursor, header.order, "geometry count", MEMBER_BYTES)?;
            let mut points = Vec::with_capacity(count);
            for _ in 0..count {
                let member = read_member_header(cursor, Base::Point, Base::MultiPoint)?;
                points.push(read_point(cursor, &member)?);
            }
            Geometry::MultiPoint { dimensions, points }
        }
        Base::MultiLineString => {
            let count = take_count(cursor, header.order, "geometry count", MEMBER_BYTES)?;
            let mut lines = Vec::with_capacity(count);
            for _ in 0..count {
                let member = read_member_header(cursor, Base::LineString, Base::MultiLineString)?;
                lines.push(read_line(cursor, &member)?);
            }
            Geometry::MultiLineString { dimensions, lines }
        }
        Base::MultiPolygon => {
            let count = take_count(cursor, header.order, "geometry count", MEMBER_BYTES)?;
            let mut polygons = Vec::with_capacity(count);
            for _ in 0..count {
                let member = read_member_header(cursor, Base::Polygon, Base::MultiPolygon)?;
                polygons.push(read_rings(cursor, &member)?);
            }
            Geometry::MultiPolygon {
                dimensions,
                polygons,
            }
        }
        Base::GeometryCollection => {
            let count = take_count(cursor, header.order, "geometry count", MEMBER_BYTES)?;
            let mut geometries = Vec::with_capacity(count);
            for _ in 0..count {
                geometries.push(read_geometry(cursor, depth + 1)?);
            }
            Geometry::GeometryCollection {
                dimensions,
                geometries,
            }
        }
    })
}

/// What one streaming pass reports: the headers it passes and the positions
/// it reads, without building the geometry they belong to.
trait Scan {
    /// One geometry header's ISO type code, top level and nested alike.
    fn type_code(&mut self, code: u32);
    /// One position, delivered as it is decoded.
    fn coordinate(&mut self, coordinate: Coord);
}

/// Walk one whole geometry, reporting headers and coordinates to `scan`.
///
/// The walk validates exactly what [`read_geometry`] validates - shared
/// header, count, and member-type checks - so a buffer both paths see is
/// refused identically; only the model allocation differs.
fn scan_geometry(cursor: &mut Cursor<'_>, depth: usize, scan: &mut impl Scan) -> Result<()> {
    check_depth(cursor, depth)?;
    let header = read_header(cursor)?;
    scan.type_code(header.type_id());
    scan_body(cursor, &header, depth, scan)
}

/// Walk one geometry body whose header is already consumed.
fn scan_body(
    cursor: &mut Cursor<'_>,
    header: &Header,
    depth: usize,
    scan: &mut impl Scan,
) -> Result<()> {
    match header.base {
        Base::Point => {
            if let Some(coordinate) = read_point(cursor, header)? {
                scan.coordinate(coordinate);
            }
        }
        Base::LineString => scan_line(cursor, header, scan)?,
        Base::Polygon => {
            let count = take_count(cursor, header.order, "ring count", 4)?;
            for _ in 0..count {
                scan_line(cursor, header, scan)?;
            }
        }
        Base::MultiPoint => scan_members(cursor, header, Base::Point, depth, scan)?,
        Base::MultiLineString => scan_members(cursor, header, Base::LineString, depth, scan)?,
        Base::MultiPolygon => scan_members(cursor, header, Base::Polygon, depth, scan)?,
        Base::GeometryCollection => {
            let count = take_count(cursor, header.order, "geometry count", MEMBER_BYTES)?;
            for _ in 0..count {
                scan_geometry(cursor, depth + 1, scan)?;
            }
        }
    }
    Ok(())
}

/// Stream one counted run of coordinates to the scan, allocating nothing.
fn scan_line(cursor: &mut Cursor<'_>, header: &Header, scan: &mut impl Scan) -> Result<()> {
    let count = take_count(
        cursor,
        header.order,
        "coordinate count",
        header.dimensions.coordinate_len() * 8,
    )?;
    for _ in 0..count {
        scan.coordinate(read_coordinate(cursor, header)?);
    }
    Ok(())
}

/// Stream the members of one multi geometry, each checked to the expected shape.
fn scan_members(
    cursor: &mut Cursor<'_>,
    header: &Header,
    expected: Base,
    depth: usize,
    scan: &mut impl Scan,
) -> Result<()> {
    let count = take_count(cursor, header.order, "geometry count", MEMBER_BYTES)?;
    for _ in 0..count {
        let member = read_member_header(cursor, expected, header.base)?;
        scan.type_code(member.type_id());
        scan_body(cursor, &member, depth, scan)?;
    }
    Ok(())
}

/// The running min/max fold behind [`bounding_box`], seeded with the fold
/// identities so an empty geometry yields the empty box.
struct Bounds(BoundingBox);

impl Bounds {
    const fn new() -> Self {
        Self(BoundingBox {
            xmin: f64::INFINITY,
            xmax: f64::NEG_INFINITY,
            ymin: f64::INFINITY,
            ymax: f64::NEG_INFINITY,
            zmin: None,
            zmax: None,
            mmin: None,
            mmax: None,
        })
    }
}

impl Scan for Bounds {
    fn type_code(&mut self, _code: u32) {}

    fn coordinate(&mut self, coordinate: Coord) {
        // A NaN position is the empty-point spelling and bounds nothing.
        if coordinate.x.is_nan() || coordinate.y.is_nan() {
            return;
        }
        self.0.xmin = self.0.xmin.min(coordinate.x);
        self.0.xmax = self.0.xmax.max(coordinate.x);
        self.0.ymin = self.0.ymin.min(coordinate.y);
        self.0.ymax = self.0.ymax.max(coordinate.y);
        fold_axis(&mut self.0.zmin, &mut self.0.zmax, coordinate.z);
        fold_axis(&mut self.0.mmin, &mut self.0.mmax, coordinate.m);
    }
}

/// Fold one optional axis into its running bounds. A NaN carries no bound.
fn fold_axis(min: &mut Option<f64>, max: &mut Option<f64>, value: Option<f64>) {
    let Some(value) = value else { return };
    if value.is_nan() {
        return;
    }
    *min = Some(min.map_or(value, |held| held.min(value)));
    *max = Some(max.map_or(value, |held| held.max(value)));
}

/// The sorted, deduplicated codes behind [`geometry_type_ids`]. At most 28
/// codes exist - seven shapes across four dimensionalities - so the vector
/// is bounded however large the geometry is.
struct TypeIds {
    codes: Vec<u32>,
}

impl Scan for TypeIds {
    fn type_code(&mut self, code: u32) {
        if let Err(slot) = self.codes.binary_search(&code) {
            self.codes.insert(slot, code);
        }
    }

    fn coordinate(&mut self, _coordinate: Coord) {}
}
