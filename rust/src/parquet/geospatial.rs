//! Parquet's `GEOMETRY` and `GEOGRAPHY` logical types.
//!
//! The pinned parquet crate can spell both logical types, but its Arrow
//! schema conversion only maps them from extension metadata behind a crate
//! feature that pulls new dependencies (`geospatial`). This module closes
//! that gap without it: the writer converts the Arrow schema itself, walks
//! it beside the converted Parquet schema, and attaches the logical type
//! wherever the Arrow field metadata declares the `geoarrow.wkb` extension.
//! A variant column takes the same treatment: the format states it as a
//! group of two `BYTE_ARRAY` children, `metadata` and `value`, annotated
//! `VARIANT(1)`, and the walk attaches that annotation wherever the Arrow
//! field declares the canonical `arrow.parquet.variant` extension.
//!
//! Attaching `GEOMETRY`/`GEOGRAPHY` is also what turns the format's
//! statistics contract on: the Parquet writer refuses min/max value bounds
//! for a geospatial column (their sort order is undefined) and accumulates
//! the format's own [`GeospatialStatistics`] instead - bounding box and
//! geometry type codes - which this module computes from the WKB bytes
//! through [`crate::wkb`], the one WKB implementation.

use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

use arrow_array::{Array, BinaryArray, BinaryViewArray, LargeBinaryArray, RecordBatch};
use arrow_schema::extension::{EXTENSION_TYPE_METADATA_KEY, EXTENSION_TYPE_NAME_KEY};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, FieldRef, Schema};
use parquet::arrow::ArrowSchemaConverter;
use parquet::arrow::ProjectionMask;
use parquet::basic::{EdgeInterpolationAlgorithm, LogicalType, Type as PhysicalType};
use parquet::geospatial::accumulator::{
    GeoStatsAccumulator, GeoStatsAccumulatorFactory, VoidGeoStatsAccumulator,
    init_geo_stats_accumulator_factory,
};
use parquet::geospatial::bounding_box::BoundingBox as ParquetBoundingBox;
use parquet::geospatial::statistics::GeospatialStatistics as ParquetGeospatialStatistics;
use parquet::schema::types::{ColumnDescPtr, SchemaDescriptor, Type, TypePtr};
use smol_str::{SmolStr, format_smolstr};

use crate::EdgeAlgorithm;
use crate::GeospatialParameters;
use crate::IOBase;
use crate::arrow::{Error, Result, from_reader_error};
use crate::wkb;
use crate::{
    DEFAULT_CRS, GEOARROW_WKB_EXTENSION_NAME, VARIANT_EXTENSION_NAME, VARIANT_VERSION,
    is_variant_storage,
};

/// Bounds and geometry types of one geospatial column, in WKB vocabulary.
///
/// This is the answer Parquet's own `GeospatialStatistics` footer field
/// carries: the box every position falls in, and the sorted, deduplicated ISO
/// geometry type codes present (dimension included, so an XYZ point is
/// `1001`). It comes from two places that must agree: the footer a write
/// recorded, exposed on [`super::ColumnStatistics::geospatial`], and a fresh
/// scan of the stored WKB through [`read_geospatial_statistics`].
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GeospatialStatistics {
    /// The box bounding every position, absent when no position bounded one.
    pub bounding_box: Option<wkb::BoundingBox>,
    /// Sorted, deduplicated ISO geometry type codes present in the column.
    pub geometry_types: Vec<i32>,
}

/// Project the footer's own statistics into the shared WKB vocabulary.
pub(super) fn from_footer(statistics: &ParquetGeospatialStatistics) -> GeospatialStatistics {
    GeospatialStatistics {
        bounding_box: statistics.bounding_box().map(|bounds| wkb::BoundingBox {
            xmin: bounds.get_xmin(),
            xmax: bounds.get_xmax(),
            ymin: bounds.get_ymin(),
            ymax: bounds.get_ymax(),
            zmin: bounds.get_zmin(),
            zmax: bounds.get_zmax(),
            mmin: bounds.get_mmin(),
            mmax: bounds.get_mmax(),
        }),
        geometry_types: statistics.geospatial_types().cloned().unwrap_or_default(),
    }
}

/// Compute one geospatial column's statistics by scanning its stored WKB.
///
/// Unlike [`super::read_statistics`], which only reads what the footer
/// recorded, this decodes the named top-level column and folds every non-null
/// value through [`wkb::bounding_box`] and [`wkb::geometry_type_ids`] - so it
/// answers for files whose writer recorded no geospatial statistics at all.
///
/// # Errors
///
/// Returns an error when the handle cannot be read, when `column` does not
/// name a stored WKB binary column, or when a stored value is malformed WKB.
pub fn read_geospatial_statistics<H: IOBase + ?Sized>(
    handle: &H,
    column: &str,
) -> Result<GeospatialStatistics> {
    let builder = super::open_builder(handle)?;
    let schema = Arc::clone(builder.schema());
    let path = format_smolstr!("$.{column}");
    let Ok(index) = schema.index_of(column) else {
        return Err(invalid(
            &path,
            "a stored geospatial column",
            format_smolstr!("no column named {column:?}"),
        ));
    };
    let field = schema.field(index);
    if !matches!(
        field.data_type(),
        ArrowDataType::Binary | ArrowDataType::LargeBinary | ArrowDataType::BinaryView
    ) {
        return Err(invalid(
            &path,
            "WKB binary storage",
            format_smolstr!("{}", field.data_type()),
        ));
    }
    let mask = ProjectionMask::roots(builder.parquet_schema(), [index]);
    let reader = builder.with_projection(mask).build()?;
    let mut fold = WkbFold::default();
    for batch in reader {
        let batch: RecordBatch = batch.map_err(from_reader_error)?;
        fold_binary_array(batch.column(0).as_ref(), &mut fold)?;
    }
    Ok(fold.into_statistics())
}

/// Fold every non-null WKB value of one binary-storage array.
fn fold_binary_array(array: &dyn Array, fold: &mut WkbFold) -> Result<()> {
    if let Some(array) = array.as_any().downcast_ref::<BinaryArray>() {
        for value in array.iter().flatten() {
            fold.update(value).map_err(Error::Core)?;
        }
    } else if let Some(array) = array.as_any().downcast_ref::<LargeBinaryArray>() {
        for value in array.iter().flatten() {
            fold.update(value).map_err(Error::Core)?;
        }
    } else if let Some(array) = array.as_any().downcast_ref::<BinaryViewArray>() {
        for value in array.iter().flatten() {
            fold.update(value).map_err(Error::Core)?;
        }
    }
    Ok(())
}

/// The running fold behind both the writer's accumulator and the reader scan.
#[derive(Debug, Default)]
struct WkbFold {
    /// Merged bounds of every position seen, absent until one bounded them.
    bounds: Option<wkb::BoundingBox>,
    /// Every ISO type code seen, kept sorted and deduplicated.
    codes: BTreeSet<i32>,
}

impl WkbFold {
    /// Fold one WKB value's bounds and type codes into the running state.
    fn update(&mut self, bytes: &[u8]) -> crate::Result<()> {
        let bounds = wkb::bounding_box(bytes)?;
        if !bounds.is_empty() {
            self.bounds = Some(match self.bounds {
                Some(held) => merged(held, bounds),
                None => bounds,
            });
        }
        for code in wkb::geometry_type_ids(bytes)? {
            if let Ok(code) = i32::try_from(code) {
                self.codes.insert(code);
            }
        }
        Ok(())
    }

    /// Finish the fold as the shared statistics value.
    fn into_statistics(self) -> GeospatialStatistics {
        GeospatialStatistics {
            bounding_box: self.bounds,
            geometry_types: self.codes.into_iter().collect(),
        }
    }

    /// Finish the fold as the footer's own statistics value.
    fn into_parquet(self) -> ParquetGeospatialStatistics {
        let statistics = self.into_statistics();
        let bounds = statistics.bounding_box.map(|bounds| {
            let mut boxed =
                ParquetBoundingBox::new(bounds.xmin, bounds.xmax, bounds.ymin, bounds.ymax);
            if let (Some(zmin), Some(zmax)) = (bounds.zmin, bounds.zmax) {
                boxed = boxed.with_zrange(zmin, zmax);
            }
            if let (Some(mmin), Some(mmax)) = (bounds.mmin, bounds.mmax) {
                boxed = boxed.with_mrange(mmin, mmax);
            }
            boxed
        });
        let types = statistics.geometry_types;
        ParquetGeospatialStatistics::new(bounds, (!types.is_empty()).then_some(types))
    }
}

/// Merge two bounding boxes into the box covering both.
fn merged(held: wkb::BoundingBox, next: wkb::BoundingBox) -> wkb::BoundingBox {
    wkb::BoundingBox {
        xmin: held.xmin.min(next.xmin),
        xmax: held.xmax.max(next.xmax),
        ymin: held.ymin.min(next.ymin),
        ymax: held.ymax.max(next.ymax),
        zmin: merged_axis(held.zmin, next.zmin, f64::min),
        zmax: merged_axis(held.zmax, next.zmax, f64::max),
        mmin: merged_axis(held.mmin, next.mmin, f64::min),
        mmax: merged_axis(held.mmax, next.mmax, f64::max),
    }
}

/// Merge one optional axis bound; an absent side carries no bound.
fn merged_axis(held: Option<f64>, next: Option<f64>, fold: fn(f64, f64) -> f64) -> Option<f64> {
    match (held, next) {
        (Some(held), Some(next)) => Some(fold(held, next)),
        (bound, None) | (None, bound) => bound,
    }
}

/// The writer-side accumulator producing footer geospatial statistics.
///
/// The Parquet column writer feeds it every non-null WKB value of a column
/// chunk; a malformed value invalidates the chunk's statistics rather than
/// recording a wrong box, which is the accumulator contract.
#[derive(Debug, Default)]
struct WkbStatsAccumulator {
    fold: WkbFold,
    invalid: bool,
}

impl GeoStatsAccumulator for WkbStatsAccumulator {
    fn is_valid(&self) -> bool {
        !self.invalid
    }

    fn update_wkb(&mut self, wkb: &[u8]) {
        if self.invalid {
            return;
        }
        if self.fold.update(wkb).is_err() {
            self.invalid = true;
        }
    }

    fn finish(&mut self) -> Option<Box<ParquetGeospatialStatistics>> {
        let fold = std::mem::take(&mut self.fold);
        if std::mem::take(&mut self.invalid) {
            return None;
        }
        Some(Box::new(fold.into_parquet()))
    }
}

/// The factory the Parquet writer asks for a geospatial column's accumulator.
///
/// Geometry columns get the WKB fold above. Geography columns get the void
/// accumulator: their bounds are edge-algorithm-aware, and a planar vertex
/// fold under-covers a spherical edge, so writing no box is the honest
/// answer - the same refusal the upstream default makes.
#[derive(Debug, Default)]
struct WkbStatsFactory;

impl GeoStatsAccumulatorFactory for WkbStatsFactory {
    fn new_accumulator(&self, descr: &ColumnDescPtr) -> Box<dyn GeoStatsAccumulator> {
        match descr.logical_type_ref() {
            Some(LogicalType::Geometry(_)) => Box::new(WkbStatsAccumulator::default()),
            _ => Box::new(VoidGeoStatsAccumulator::default()),
        }
    }
}

/// Install the WKB statistics factory into the Parquet writer, once.
///
/// The parquet crate holds one process-global factory and the first
/// installation wins; when another library already installed its own, that
/// one keeps producing the statistics and this call is a no-op. The factory
/// only fires for columns whose logical type is `GEOMETRY` or `GEOGRAPHY`,
/// so non-geospatial writers never see it.
pub(super) fn install_wkb_statistics() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let _ = init_geo_stats_accumulator_factory(Arc::new(WkbStatsFactory));
    });
}

/// Convert an Arrow schema whose fields declare geospatial or variant
/// extensions into a Parquet schema carrying the matching logical types.
///
/// Returns `None` when no field in the schema declares one, which is the
/// writer's signal to let the crate's own conversion run untouched.
///
/// # Errors
///
/// Returns an error when a declared extension does not sit on the storage it
/// requires, or when its GeoArrow metadata document is malformed.
pub(super) fn extension_schema(schema: &Schema) -> Result<Option<SchemaDescriptor>> {
    if !schema
        .fields()
        .iter()
        .any(|field| subtree_has_extension(field))
    {
        return Ok(None);
    }
    let converted = ArrowSchemaConverter::new().convert(schema)?;
    let root = converted.root_schema_ptr();
    let mut fields = Vec::with_capacity(schema.fields().len());
    for (field, ty) in schema.fields().iter().zip(root.get_fields()) {
        let path = format!("$.{}", field.name());
        fields.push(annotated(field, ty, &path)?);
    }
    let root = rebuilt_group(root.as_ref(), fields, None)?;
    Ok(Some(SchemaDescriptor::new(Arc::new(root))))
}

/// The Arrow schema a file's columns declare, with the variant extension
/// attached wherever the Parquet schema says `VARIANT`.
///
/// Reading is the writing walk inverted, and it is needed for one reason:
/// the pinned parquet crate maps `VARIANT` to a plain struct, its own
/// mapping being behind a feature that pulls a Variant crate in. A file
/// written here carries its Arrow schema in the footer and already declares
/// the extension, so this answers `None` for it; a file another writer
/// produced is where it pays, and the column reads back as the variant the
/// annotation says it is.
///
/// Answers `None` when no column needs it, which is the writer's signal to
/// read with the schema it already has.
pub(super) fn variant_schema(parquet: &SchemaDescriptor, arrow: &Schema) -> Option<Schema> {
    let root = parquet.root_schema_ptr();
    let nodes = root.get_fields();
    if nodes.len() != arrow.fields().len() {
        return None;
    }
    let mut fields = Vec::with_capacity(nodes.len());
    let mut attached = false;
    for (field, node) in arrow.fields().iter().zip(nodes) {
        let (field, found) = with_variant(field, node);
        attached |= found;
        fields.push(field);
    }
    attached.then(|| Schema::new_with_metadata(fields, arrow.metadata().clone()))
}

/// Attach the variant extension to one field, walking into the containers
/// the writer walks so a variant nested in a struct is found too.
fn with_variant(field: &FieldRef, node: &TypePtr) -> (FieldRef, bool) {
    if field.metadata().contains_key(EXTENSION_TYPE_NAME_KEY) {
        return (Arc::clone(field), false);
    }
    if matches!(
        node.get_basic_info().logical_type_ref(),
        Some(LogicalType::Variant(_))
    ) && is_variant_storage(field.data_type())
    {
        let mut metadata = field.metadata().clone();
        metadata.insert(
            EXTENSION_TYPE_NAME_KEY.to_owned(),
            VARIANT_EXTENSION_NAME.to_owned(),
        );
        metadata.insert(EXTENSION_TYPE_METADATA_KEY.to_owned(), String::new());
        return (
            Arc::new(field.as_ref().clone().with_metadata(metadata)),
            true,
        );
    }
    let ArrowDataType::Struct(children) = field.data_type() else {
        return (Arc::clone(field), false);
    };
    let nodes = node.get_fields();
    if !node.is_group() || nodes.len() != children.len() {
        return (Arc::clone(field), false);
    }
    let mut fields = Vec::with_capacity(children.len());
    let mut attached = false;
    for (child, child_node) in children.iter().zip(nodes) {
        let (child, found) = with_variant(child, child_node);
        attached |= found;
        fields.push(child);
    }
    if !attached {
        return (Arc::clone(field), false);
    }
    let rebuilt = field
        .as_ref()
        .clone()
        .with_data_type(ArrowDataType::Struct(fields.into()));
    (Arc::new(rebuilt), true)
}

/// Return whether this field or any field beneath it declares an extension
/// this module attaches a logical type for.
fn subtree_has_extension(field: &ArrowField) -> bool {
    if matches!(
        field
            .metadata()
            .get(EXTENSION_TYPE_NAME_KEY)
            .map(String::as_str),
        Some(GEOARROW_WKB_EXTENSION_NAME | VARIANT_EXTENSION_NAME)
    ) {
        return true;
    }
    match field.data_type() {
        ArrowDataType::Struct(children) => {
            children.iter().any(|child| subtree_has_extension(child))
        }
        ArrowDataType::List(child)
        | ArrowDataType::LargeList(child)
        | ArrowDataType::FixedSizeList(child, _)
        | ArrowDataType::Map(child, _) => subtree_has_extension(child),
        _ => false,
    }
}

/// Return `ty` with the logical type this field's extension declares, walking
/// into nested containers to find declarations beneath.
fn annotated(field: &ArrowField, ty: &TypePtr, path: &str) -> Result<TypePtr> {
    match field
        .metadata()
        .get(EXTENSION_TYPE_NAME_KEY)
        .map(String::as_str)
    {
        Some(GEOARROW_WKB_EXTENSION_NAME) => Ok(Arc::new(geospatial_primitive(field, ty, path)?)),
        Some(VARIANT_EXTENSION_NAME) => Ok(Arc::new(variant_group(field, ty, path)?)),
        _ => descend(field, ty, path),
    }
}

/// Walk one container level of the Arrow field beside its Parquet type.
///
/// Only containers the Parquet writer can represent are walked - struct, the
/// list family, and map - which covers every layout a declaration can reach:
/// the remaining containers (union, run-end encoding) have no Parquet
/// conversion at all, so a subtree under them never gets this far.
fn descend(field: &ArrowField, ty: &TypePtr, path: &str) -> Result<TypePtr> {
    if !subtree_has_extension(field) || !ty.is_group() {
        return Ok(Arc::clone(ty));
    }
    match field.data_type() {
        ArrowDataType::Struct(children) => {
            let mut fields = Vec::with_capacity(children.len());
            for (child, child_ty) in children.iter().zip(ty.get_fields()) {
                let child_path = format!("{path}.{}", child.name());
                fields.push(annotated(child, child_ty, &child_path)?);
            }
            Ok(Arc::new(rebuilt_group(ty, fields, None)?))
        }
        ArrowDataType::List(child)
        | ArrowDataType::LargeList(child)
        | ArrowDataType::FixedSizeList(child, _) => {
            // Three-level list: group (LIST) { repeated group { element } }.
            let [middle] = ty.get_fields() else {
                return Ok(Arc::clone(ty));
            };
            let [element] = middle.get_fields() else {
                return Ok(Arc::clone(ty));
            };
            let child_path = format!("{path}[]");
            let element = annotated(child, element, &child_path)?;
            let middle = Arc::new(rebuilt_group(middle, vec![element], None)?);
            Ok(Arc::new(rebuilt_group(ty, vec![middle], None)?))
        }
        ArrowDataType::Map(entries, _) => {
            // Map: group (MAP) { repeated group key_value { key, value } }.
            let [middle] = ty.get_fields() else {
                return Ok(Arc::clone(ty));
            };
            let ArrowDataType::Struct(children) = entries.data_type() else {
                return Ok(Arc::clone(ty));
            };
            let mut fields = Vec::with_capacity(children.len());
            for (child, child_ty) in children.iter().zip(middle.get_fields()) {
                let child_path = format!("{path}.{}", child.name());
                fields.push(annotated(child, child_ty, &child_path)?);
            }
            let middle = Arc::new(rebuilt_group(middle, fields, None)?);
            Ok(Arc::new(rebuilt_group(ty, vec![middle], None)?))
        }
        _ => Ok(Arc::clone(ty)),
    }
}

/// Rebuild one `geoarrow.wkb` primitive with its geospatial logical type.
fn geospatial_primitive(field: &ArrowField, ty: &Type, path: &str) -> Result<Type> {
    if !ty.is_primitive() || ty.get_physical_type() != PhysicalType::BYTE_ARRAY {
        return Err(invalid(
            path,
            "BYTE_ARRAY storage for a geoarrow.wkb column",
            storage_name(ty),
        ));
    }
    let document = field.metadata().get(EXTENSION_TYPE_METADATA_KEY);
    let logical = geoarrow_logical_type(document.map(String::as_str), path)?;
    let info = ty.get_basic_info();
    let mut builder = Type::primitive_type_builder(ty.name(), PhysicalType::BYTE_ARRAY)
        .with_logical_type(Some(logical))
        .with_id(info.has_id().then(|| info.id()));
    if info.has_repetition() {
        builder = builder.with_repetition(info.repetition());
    }
    Ok(builder.build()?)
}

/// Rebuild one `arrow.parquet.variant` group with the `VARIANT` logical
/// type the format states for it.
///
/// The two children keep the names and the `BYTE_ARRAY` storage the Arrow
/// struct gave them and take no annotation of their own: the format reads
/// them by name, and Iceberg requires that neither carry a field id, which
/// is what a group rebuilt from its own children preserves.
fn variant_group(field: &ArrowField, ty: &TypePtr, path: &str) -> Result<Type> {
    if !ty.is_group() || !is_variant_storage(field.data_type()) {
        return Err(invalid(
            path,
            "a group of `metadata` and `value` binaries for a variant column",
            storage_name(ty),
        ));
    }
    rebuilt_group(
        ty,
        ty.get_fields().to_vec(),
        Some(LogicalType::variant(Some(
            i8::try_from(VARIANT_VERSION).unwrap_or(1),
        ))),
    )
}

/// Rebuild one group node, preserving its identity and optionally attaching
/// a logical type it did not have.
fn rebuilt_group(ty: &Type, fields: Vec<TypePtr>, logical: Option<LogicalType>) -> Result<Type> {
    let info = ty.get_basic_info();
    let mut builder = Type::group_type_builder(ty.name())
        .with_converted_type(info.converted_type())
        .with_logical_type(logical.or_else(|| info.logical_type_ref().cloned()))
        .with_fields(fields)
        .with_id(info.has_id().then(|| info.id()));
    if info.has_repetition() {
        builder = builder.with_repetition(info.repetition());
    }
    Ok(builder.build()?)
}

/// Parse one GeoArrow metadata document into the logical type it declares.
///
/// The parse is [`GeospatialParameters::from_geoarrow_json`] - the same one the
/// field layer's import runs - so the two readers cannot drift. The
/// `OGC:CRS84` default and the `spherical` default fold to Parquet's absent
/// spellings, so a bare column writes the format's bare logical type.
fn geoarrow_logical_type(document: Option<&str>, path: &str) -> Result<LogicalType> {
    let geospatial = GeospatialParameters::from_geoarrow_json(document).map_err(|error| {
        invalid(
            path,
            "a GeoArrow JSON metadata document",
            format_smolstr!("{:?} ({error})", document.unwrap_or("")),
        )
    })?;
    let crs = Some(geospatial.crs().to_owned()).filter(|crs| crs != DEFAULT_CRS);
    Ok(match geospatial.algorithm() {
        // No edge algorithm is what distinguishes a geometry: a geography
        // always carries one, `spherical` included.
        None => LogicalType::geometry(crs),
        Some(algorithm) => LogicalType::geography(
            crs,
            (algorithm != EdgeAlgorithm::Spherical).then(|| parquet_algorithm(algorithm)),
        ),
    })
}

/// The Parquet spelling of one edge algorithm.
const fn parquet_algorithm(algorithm: EdgeAlgorithm) -> EdgeInterpolationAlgorithm {
    match algorithm {
        EdgeAlgorithm::Spherical => EdgeInterpolationAlgorithm::SPHERICAL,
        EdgeAlgorithm::Vincenty => EdgeInterpolationAlgorithm::VINCENTY,
        EdgeAlgorithm::Thomas => EdgeInterpolationAlgorithm::THOMAS,
        EdgeAlgorithm::Andoyer => EdgeInterpolationAlgorithm::ANDOYER,
        EdgeAlgorithm::Karney => EdgeInterpolationAlgorithm::KARNEY,
    }
}

/// Name one Parquet node's storage for an error message.
fn storage_name(ty: &Type) -> SmolStr {
    if ty.is_group() {
        SmolStr::new_static("a group node")
    } else {
        format_smolstr!("{:?} storage", ty.get_physical_type())
    }
}

/// An expected/got failure at one schema path.
fn invalid(path: &str, expected: &str, actual: impl std::fmt::Display) -> Error {
    Error::InvalidValue {
        path: SmolStr::new(path),
        expected: SmolStr::new(expected),
        actual: format_smolstr!("{actual}"),
    }
}
