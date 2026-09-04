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
//! use yggdryl::types::geospatial::wkb;
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

use crate::{Float64, Result};

mod read;
mod write;

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
        read::geometry(bytes)
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
        write::into_wkt(&self)
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
    read::bounding_box(bytes)
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
    read::geometry_type_ids(bytes)
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

#[cfg(test)]
mod tests;
