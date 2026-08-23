//! Well-Known Binary geometries, read for their bounds and their text form.
//!
//! WKB is the length-prefixed binary spelling of the simple-feature
//! geometries: one byte-order byte, a `u32` type code, then counts and
//! IEEE 754 doubles, with every nested geometry carrying its own byte-order
//! byte. That makes it readable with no dependency, which is why this module
//! exists at all: displaying a geometry column as WKT, casting it to text,
//! and bounding it for Parquet and Iceberg statistics all need the same
//! reader, and none of them needs a geometry engine. A WKT *parser* is
//! deliberately not added: the workspace needs to display and bound
//! geometries, not to accept text geometry input yet.
//!
//! Both type-code spellings are read: the ISO one, where Z, M, and ZM add
//! 1000, 2000, or 3000 to the base code, and the PostGIS EWKB one, where
//! high bits flag the extra axes and an embedded SRID. The SRID is read past
//! rather than modeled, because bounds and text are the same in every
//! reference system. `POINT EMPTY` has no zero-count spelling in WKB, so its
//! conventional NaN coordinates are read back as the empty point.
//!
//! ```
//! use yggdryl::generic::wkb;
//!
//! # fn main() -> yggdryl::Result<()> {
//! // A little-endian XY point: order byte, type code 1, then x and y.
//! let mut point = vec![1, 1, 0, 0, 0];
//! point.extend(10.0_f64.to_le_bytes());
//! point.extend(20.0_f64.to_le_bytes());
//!
//! assert_eq!(wkb::into_wkt(&point)?, "POINT (10 20)");
//! assert_eq!(wkb::geometry_type_ids(&point)?, [1]);
//!
//! let bounds = wkb::bounding_box(&point)?;
//! assert_eq!((bounds.xmin, bounds.ymax), (10.0, 20.0));
//! # Ok(())
//! # }
//! ```

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Float64, Result};

type CoordIdentity = (Float64, Float64, Option<Float64>, Option<Float64>);
type BoundingBoxIdentity = (
    Float64,
    Float64,
    Float64,
    Float64,
    Option<Float64>,
    Option<Float64>,
    Option<Float64>,
    Option<Float64>,
);

/// The EWKB flag naming an elevation on every coordinate.
const EWKB_Z: u32 = 0x8000_0000;
/// The EWKB flag naming a measure on every coordinate.
const EWKB_M: u32 = 0x4000_0000;
/// The EWKB flag announcing an embedded SRID after the type code.
const EWKB_SRID: u32 = 0x2000_0000;

/// The fewest bytes any nested geometry occupies: its byte-order byte plus
/// its type code. Used to bound a member count against the bytes present.
const MEMBER_BYTES: usize = 5;

/// The axes one geometry carries beyond the plane.
///
/// WKB spells the dimensionality inside the type code, and every coordinate
/// of one geometry carries the same axes, so the dimensionality is a property
/// of the geometry itself. Keeping it beside the coordinates is what lets
/// `POINT Z EMPTY` stay distinguishable from `POINT EMPTY` once the
/// coordinates are gone.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Dimensions {
    /// Plane coordinates only.
    Xy,
    /// An elevation beside the plane.
    Xyz,
    /// A measure beside the plane.
    Xym,
    /// Both an elevation and a measure.
    Xyzm,
}

impl Dimensions {
    /// Build the dimensionality carrying the named extra axes.
    pub const fn new(has_z: bool, has_m: bool) -> Self {
        match (has_z, has_m) {
            (false, false) => Self::Xy,
            (true, false) => Self::Xyz,
            (false, true) => Self::Xym,
            (true, true) => Self::Xyzm,
        }
    }

    /// Return whether every coordinate carries an elevation.
    pub const fn has_z(self) -> bool {
        matches!(self, Self::Xyz | Self::Xyzm)
    }

    /// Return whether every coordinate carries a measure.
    pub const fn has_m(self) -> bool {
        matches!(self, Self::Xym | Self::Xyzm)
    }

    /// The offset ISO adds to a base type code for these axes.
    const fn iso_offset(self) -> u32 {
        match self {
            Self::Xy => 0,
            Self::Xyz => 1_000,
            Self::Xym => 2_000,
            Self::Xyzm => 3_000,
        }
    }

    /// The number of doubles one coordinate occupies.
    const fn coordinate_len(self) -> usize {
        match self {
            Self::Xy => 2,
            Self::Xyz | Self::Xym => 3,
            Self::Xyzm => 4,
        }
    }

    /// The dimension marker WKT writes between the tag and the body.
    const fn wkt_marker(self) -> &'static str {
        match self {
            Self::Xy => "",
            Self::Xyz => "Z",
            Self::Xym => "M",
            Self::Xyzm => "ZM",
        }
    }
}

/// One position: an x and a y, plus the optional elevation and measure the
/// geometry's dimensionality adds.
#[derive(Clone, Copy, Debug)]
pub struct Coord {
    /// The easting, or longitude.
    pub x: f64,
    /// The northing, or latitude.
    pub y: f64,
    /// The elevation, present exactly when the geometry carries Z.
    pub z: Option<f64>,
    /// The measure, present exactly when the geometry carries M.
    pub m: Option<f64>,
}

impl Coord {
    fn identity(&self) -> CoordIdentity {
        (
            Float64::from_f64(self.x),
            Float64::from_f64(self.y),
            self.z.map(Float64::from_f64),
            self.m.map(Float64::from_f64),
        )
    }
}

impl PartialEq for Coord {
    fn eq(&self, other: &Self) -> bool {
        self.identity() == other.identity()
    }
}

impl Eq for Coord {}

impl PartialOrd for Coord {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Coord {
    fn cmp(&self, other: &Self) -> Ordering {
        self.identity().cmp(&other.identity())
    }
}

impl Hash for Coord {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity().hash(state);
    }
}

/// One decoded geometry: the seven simple-feature shapes, owned.
///
/// The model is deliberately lightweight - vectors of [`Coord`] under each
/// shape - because its callers render and inspect; they do not compute
/// geometry. An empty point is `None` rather than a NaN coordinate, so
/// emptiness is a shape the type system shows instead of a value a caller
/// has to remember to test for.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Geometry {
    /// A single position, or the empty point.
    Point {
        /// The axes every coordinate carries.
        dimensions: Dimensions,
        /// The position, absent for `POINT EMPTY`.
        coordinate: Option<Coord>,
    },
    /// An ordered path of positions.
    LineString {
        /// The axes every coordinate carries.
        dimensions: Dimensions,
        /// The positions along the path, in order.
        coordinates: Vec<Coord>,
    },
    /// An outer ring and any number of holes.
    Polygon {
        /// The axes every coordinate carries.
        dimensions: Dimensions,
        /// The rings, outer boundary first as WKB stores them.
        rings: Vec<Vec<Coord>>,
    },
    /// A bag of points.
    MultiPoint {
        /// The axes every coordinate carries.
        dimensions: Dimensions,
        /// The member points, each possibly empty.
        points: Vec<Option<Coord>>,
    },
    /// A bag of paths.
    MultiLineString {
        /// The axes every coordinate carries.
        dimensions: Dimensions,
        /// The member paths.
        lines: Vec<Vec<Coord>>,
    },
    /// A bag of polygons.
    MultiPolygon {
        /// The axes every coordinate carries.
        dimensions: Dimensions,
        /// The member polygons, each a list of rings.
        polygons: Vec<Vec<Vec<Coord>>>,
    },
    /// A bag of arbitrary geometries, which may nest further collections.
    GeometryCollection {
        /// The axes the collection itself declares; members keep their own.
        dimensions: Dimensions,
        /// The member geometries.
        geometries: Vec<Geometry>,
    },
}

impl Geometry {
    /// Parse one WKB geometry from a byte slice.
    ///
    /// Both ISO and EWKB type codes are accepted, in either byte order, and
    /// an EWKB SRID is read past. The whole slice must be one geometry:
    /// trailing bytes are refused, because a caller holding a column cell
    /// wants to know the cell is exactly what it decoded.
    ///
    /// # Errors
    ///
    /// Returns an error naming the byte position when the buffer is
    /// truncated, a byte order or type code is unknown, a member of a multi
    /// geometry has the wrong type, a count promises more entries than the
    /// buffer holds, collections nest past the shared recursion limit, or
    /// bytes trail the geometry.
    pub fn from_slice(bytes: &[u8]) -> Result<Self> {
        let mut cursor = Cursor::new(bytes);
        let geometry = read_geometry(&mut cursor, 0)?;
        cursor.finish()?;
        Ok(geometry)
    }

    /// Return the axes every coordinate of this geometry carries.
    pub const fn dimensions(&self) -> Dimensions {
        match self {
            Self::Point { dimensions, .. }
            | Self::LineString { dimensions, .. }
            | Self::Polygon { dimensions, .. }
            | Self::MultiPoint { dimensions, .. }
            | Self::MultiLineString { dimensions, .. }
            | Self::MultiPolygon { dimensions, .. }
            | Self::GeometryCollection { dimensions, .. } => *dimensions,
        }
    }

    /// Return this geometry's ISO type code: the base shape plus 1000, 2000,
    /// or 3000 for Z, M, or ZM. An EWKB input reports the same ISO code, so
    /// one vocabulary reaches the statistics writers.
    pub const fn type_id(&self) -> u32 {
        self.base().code() + self.dimensions().iso_offset()
    }

    /// Return whether this geometry holds no member at all: the empty point,
    /// or a zero count. A collection of empty members is itself non-empty,
    /// exactly as WKB spells it.
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Point { coordinate, .. } => coordinate.is_none(),
            Self::LineString { coordinates, .. } => coordinates.is_empty(),
            Self::Polygon { rings, .. } => rings.is_empty(),
            Self::MultiPoint { points, .. } => points.is_empty(),
            Self::MultiLineString { lines, .. } => lines.is_empty(),
            Self::MultiPolygon { polygons, .. } => polygons.is_empty(),
            Self::GeometryCollection { geometries, .. } => geometries.is_empty(),
        }
    }

    /// Spell this geometry as canonical WKT: uppercase tags, `Z`/`M`/`ZM`
    /// markers, parenthesized members of a multipoint, and `EMPTY` for every
    /// absent body. Each coordinate prints the shortest decimal that reads
    /// back as the same double, which is what Rust's float `Display`
    /// promises, so the text loses nothing.
    pub fn into_wkt(self) -> String {
        let mut text = String::new();
        write_geometry(&mut text, &self);
        text
    }

    /// The base shape this variant spells in a type code.
    const fn base(&self) -> Base {
        match self {
            Self::Point { .. } => Base::Point,
            Self::LineString { .. } => Base::LineString,
            Self::Polygon { .. } => Base::Polygon,
            Self::MultiPoint { .. } => Base::MultiPoint,
            Self::MultiLineString { .. } => Base::MultiLineString,
            Self::MultiPolygon { .. } => Base::MultiPolygon,
            Self::GeometryCollection { .. } => Base::GeometryCollection,
        }
    }
}

/// The smallest axis-aligned box around every position of one geometry.
///
/// An empty geometry yields the identity of the min/max fold - `xmin` is
/// positive infinity and `xmax` negative infinity - which [`Self::is_empty`]
/// names, so a statistics writer can skip the box instead of storing it. The
/// Z and M bounds are present exactly when some coordinate carried that axis.
#[derive(Clone, Copy, Debug)]
pub struct BoundingBox {
    /// The smallest x of any position.
    pub xmin: f64,
    /// The largest x of any position.
    pub xmax: f64,
    /// The smallest y of any position.
    pub ymin: f64,
    /// The largest y of any position.
    pub ymax: f64,
    /// The smallest elevation, when any coordinate carried one.
    pub zmin: Option<f64>,
    /// The largest elevation, when any coordinate carried one.
    pub zmax: Option<f64>,
    /// The smallest measure, when any coordinate carried one.
    pub mmin: Option<f64>,
    /// The largest measure, when any coordinate carried one.
    pub mmax: Option<f64>,
}

impl BoundingBox {
    /// Return whether no position bounded this box, which is what an empty
    /// geometry - or one spelled entirely from NaN positions - produces.
    pub fn is_empty(&self) -> bool {
        self.xmin > self.xmax
    }

    fn identity(&self) -> BoundingBoxIdentity {
        (
            Float64::from_f64(self.xmin),
            Float64::from_f64(self.xmax),
            Float64::from_f64(self.ymin),
            Float64::from_f64(self.ymax),
            self.zmin.map(Float64::from_f64),
            self.zmax.map(Float64::from_f64),
            self.mmin.map(Float64::from_f64),
            self.mmax.map(Float64::from_f64),
        )
    }
}

impl PartialEq for BoundingBox {
    fn eq(&self, other: &Self) -> bool {
        self.identity() == other.identity()
    }
}

impl Eq for BoundingBox {}

impl PartialOrd for BoundingBox {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BoundingBox {
    fn cmp(&self, other: &Self) -> Ordering {
        self.identity().cmp(&other.identity())
    }
}

impl Hash for BoundingBox {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identity().hash(state);
    }
}

/// Compute the bounds of one WKB geometry in a single pass over the bytes.
///
/// The geometry is never materialized: coordinates stream through the fold
/// one at a time, so bounding a large polygon allocates nothing per vertex.
/// NaN positions - the empty-point spelling - bound nothing, and a NaN
/// elevation or measure likewise carries no bound.
///
/// # Errors
///
/// Returns the same malformed-buffer errors as [`Geometry::from_slice`],
/// each naming its byte position.
pub fn bounding_box(bytes: &[u8]) -> Result<BoundingBox> {
    let mut cursor = Cursor::new(bytes);
    let mut bounds = Bounds::new();
    scan_geometry(&mut cursor, 0, &mut bounds)?;
    cursor.finish()?;
    Ok(bounds.0)
}

/// List the ISO type codes present in one WKB geometry, sorted ascending and
/// deduplicated, in a single pass over the bytes.
///
/// Every header encountered contributes its code, so a collection reports
/// itself and its members, and a multipoint reports both `4` and the `1` its
/// member points spell. EWKB codes are normalized to the ISO spelling, which
/// is the vocabulary the Parquet geospatial statistics carry.
///
/// # Errors
///
/// Returns the same malformed-buffer errors as [`Geometry::from_slice`],
/// each naming its byte position.
pub fn geometry_type_ids(bytes: &[u8]) -> Result<Vec<u32>> {
    let mut cursor = Cursor::new(bytes);
    let mut codes = TypeIds { codes: Vec::new() };
    scan_geometry(&mut cursor, 0, &mut codes)?;
    cursor.finish()?;
    Ok(codes.codes)
}

/// Render one WKB geometry as canonical WKT.
///
/// The writer parses the owned model first rather than streaming: WKT nests
/// by structure, and the output string already costs the allocation a
/// streaming pass would be saving.
///
/// # Errors
///
/// Returns the same malformed-buffer errors as [`Geometry::from_slice`],
/// each naming its byte position.
pub fn into_wkt(bytes: &[u8]) -> Result<String> {
    Ok(Geometry::from_slice(bytes)?.into_wkt())
}

/// The seven base shapes a type code can name.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum Base {
    Point,
    LineString,
    Polygon,
    MultiPoint,
    MultiLineString,
    MultiPolygon,
    GeometryCollection,
}

impl Base {
    /// The shape a base code names, when it names one.
    const fn from_code(code: u32) -> Option<Self> {
        match code {
            1 => Some(Self::Point),
            2 => Some(Self::LineString),
            3 => Some(Self::Polygon),
            4 => Some(Self::MultiPoint),
            5 => Some(Self::MultiLineString),
            6 => Some(Self::MultiPolygon),
            7 => Some(Self::GeometryCollection),
            _ => None,
        }
    }

    /// The base type code this shape spells.
    const fn code(self) -> u32 {
        match self {
            Self::Point => 1,
            Self::LineString => 2,
            Self::Polygon => 3,
            Self::MultiPoint => 4,
            Self::MultiLineString => 5,
            Self::MultiPolygon => 6,
            Self::GeometryCollection => 7,
        }
    }

    /// The lowercase name errors call this shape.
    const fn name(self) -> &'static str {
        match self {
            Self::Point => "point",
            Self::LineString => "linestring",
            Self::Polygon => "polygon",
            Self::MultiPoint => "multipoint",
            Self::MultiLineString => "multilinestring",
            Self::MultiPolygon => "multipolygon",
            Self::GeometryCollection => "geometrycollection",
        }
    }
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

#[cfg(test)]
mod tests;
