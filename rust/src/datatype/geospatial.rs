//! The geospatial pair: geometry and geography, and the value they share.

use std::sync::Arc;

use smol_str::SmolStr;

use crate::enums::EdgeAlgorithm;
use crate::{DataType, Error, Result};

/// The coordinate reference system both formats fill when none is given.
///
/// `OGC:CRS84` is longitude/latitude on WGS 84 - the default Parquet's
/// `GEOMETRY`/`GEOGRAPHY` logical types and Iceberg v3's geospatial types
/// share, so it is the one this workspace fills too.
pub const DEFAULT_CRS: &str = "OGC:CRS84";

/// The parameters a geometry or geography column carries.
///
/// One shared value beside [`crate::MapType`], [`crate::DictionaryType`], and
/// [`crate::RunEndEncodedType`]: the coordinate reference system both carry,
/// and the edge algorithm only a geography has - a geometry connects vertices
/// with straight planar lines, so a geometry given an algorithm is refused by
/// name, and a geography given none fills [`EdgeAlgorithm::Spherical`], the
/// default Parquet and Iceberg share.
#[derive(Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct GeospatialType {
    /// The coordinate reference system, `OGC:CRS84` filled when none is given.
    crs: SmolStr,
    /// The edge interpolation, present exactly on a geography.
    algorithm: Option<EdgeAlgorithm>,
}

impl GeospatialType {
    /// The parameters of a geometry: a CRS and no edge algorithm.
    ///
    /// # Errors
    ///
    /// Returns an error when `crs` is empty - an empty reference system names
    /// nothing, and the absent spelling is `None`, which fills the default.
    pub fn geometry(crs: Option<&str>) -> Result<Self> {
        Ok(Self {
            crs: validated_crs(crs)?,
            algorithm: None,
        })
    }

    /// The parameters of a geography: a CRS and an edge algorithm.
    ///
    /// A geography given no algorithm fills [`EdgeAlgorithm::Spherical`] - the
    /// default both Parquet and Iceberg fill - so the field is never absent on
    /// a geography and never present on a geometry.
    ///
    /// # Errors
    ///
    /// Returns an error when `crs` is empty.
    pub fn geography(crs: Option<&str>, algorithm: Option<EdgeAlgorithm>) -> Result<Self> {
        Ok(Self {
            crs: validated_crs(crs)?,
            algorithm: Some(algorithm.unwrap_or_default()),
        })
    }

    /// The coordinate reference system, never empty.
    pub fn crs(&self) -> &str {
        &self.crs
    }

    /// Return whether the CRS is the `OGC:CRS84` default both formats fill.
    pub fn has_default_crs(&self) -> bool {
        self.crs == DEFAULT_CRS
    }

    /// The edge algorithm, present exactly when this describes a geography.
    pub const fn algorithm(&self) -> Option<EdgeAlgorithm> {
        self.algorithm
    }
}

/// Fill or validate the CRS: `None` is the shared default, empty is refused.
fn validated_crs(crs: Option<&str>) -> Result<SmolStr> {
    match crs {
        None => Ok(SmolStr::new_static(DEFAULT_CRS)),
        Some("") => Err(Error::InvalidDataType {
            kind: "geospatial",
            reason: SmolStr::new_static(
                "expected a coordinate reference system name, got an empty string; \
                 omit it to fill the OGC:CRS84 default",
            ),
        }),
        Some(crs) => Ok(SmolStr::new(crs)),
    }
}

impl DataType {
    /// Creates the self-describing semi-structured Variant type.
    ///
    /// It takes no parameters - shredding is a physical layout, not part of
    /// the logical type - and a variant *value* is a [`crate::Value`]: a
    /// self-describing tree, so the binary form is an encoding of the one
    /// value model, never a second one. The finite union of declared members
    /// is [`Self::dense_union`]; in the grammar the parenthesis
    /// disambiguates, so bare `variant` parses to this type and
    /// `variant(...)` stays the union sugar.
    #[must_use]
    pub const fn variant() -> Self {
        Self::Variant
    }

    /// Creates a geometry: planar geospatial features as Well-Known Binary.
    ///
    /// `None` fills the `OGC:CRS84` default. A geometry has no edge
    /// algorithm - straight planar lines need none - so there is nothing else
    /// to give; [`Self::geography`] is the type that takes one.
    ///
    /// # Errors
    ///
    /// Returns an error when `crs` is empty.
    pub fn geometry(crs: Option<&str>) -> Result<Self> {
        Ok(Self::Geometry(Arc::new(GeospatialType::geometry(crs)?)))
    }

    /// Creates a geography: geospatial features on a sphere or spheroid.
    ///
    /// `None` fills the `OGC:CRS84` default and the
    /// [`EdgeAlgorithm::Spherical`] default, so `geography(None, None)` is
    /// the type both formats spell bare.
    ///
    /// # Errors
    ///
    /// Returns an error when `crs` is empty.
    pub fn geography(crs: Option<&str>, algorithm: Option<EdgeAlgorithm>) -> Result<Self> {
        Ok(Self::Geography(Arc::new(GeospatialType::geography(
            crs, algorithm,
        )?)))
    }
}
