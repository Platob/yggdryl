//! Recursive compatibility normalization for concrete schema targets.
//!
//! One generic walker owns recursion, path tracking, extension-storage
//! protection, and struct rebuilding for every target. A target contributes
//! only its scalar matrix and its container support table, so adding an
//! engine never duplicates the traversal.

use smol_str::{SmolStr, format_smolstr};

use crate::path::{Path, Segment};
use crate::text::{elide_display, expected_got};
use crate::{Error, Field, Result, Scheme, TimeUnit};

use super::{DataType, preflight_schema, preflight_schema_shape};

const ARROW_EXTENSION_NAME_KEY: &str = "ARROW:extension:name";
const ARROW_EXTENSION_METADATA_KEY: &str = "ARROW:extension:metadata";

/// The normalization target selected by a [`Scheme`] compatibility value.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Target {
    Arrow,
    Spark,
    Polars,
    Pandas,
    Iceberg,
}

impl Target {
    fn from_scheme(scheme: &Scheme) -> Result<Self> {
        if scheme == &Scheme::ARROW {
            Ok(Self::Arrow)
        } else if scheme == &Scheme::SPARK {
            Ok(Self::Spark)
        } else if scheme == &Scheme::POLARS {
            Ok(Self::Polars)
        } else if scheme == &Scheme::PANDAS {
            Ok(Self::Pandas)
        } else if scheme == &Scheme::ICEBERG {
            Ok(Self::Iceberg)
        } else {
            Err(Error::InvalidDataType {
                kind: "Compatibility",
                reason: format_smolstr!(
                    "expected one of {}, got {:?}",
                    compatibility_vocabulary(),
                    scheme.as_str()
                ),
            })
        }
    }

    const fn kind(self) -> &'static str {
        match self {
            Self::Arrow => "ArrowCompatibility",
            Self::Spark => "SparkCompatibility",
            Self::Polars => "PolarsCompatibility",
            Self::Pandas => "PandasCompatibility",
            Self::Iceberg => "IcebergCompatibility",
        }
    }

    /// The engine name used inside a normalization failure message.
    const fn engine(self) -> &'static str {
        match self {
            Self::Arrow => "Arrow",
            Self::Spark => "Spark",
            Self::Polars => "Polars",
            Self::Pandas => "pandas",
            Self::Iceberg => "Iceberg",
        }
    }

    /// Whether the engine has a tagged-union schema equivalent.
    const fn supports_union(self) -> bool {
        false
    }

    /// Whether the engine has a first-class map type.
    const fn supports_map(self) -> bool {
        matches!(self, Self::Arrow | Self::Spark | Self::Iceberg)
    }

    /// Whether the engine keeps a fixed-length list layout distinct from a list.
    const fn supports_fixed_size_list(self) -> bool {
        matches!(self, Self::Arrow | Self::Polars)
    }
}

fn compatibility_vocabulary() -> String {
    Scheme::COMPATIBILITY_TARGETS
        .iter()
        .map(Scheme::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

impl DataType {
    /// Returns a recursively normalized datatype for a compatibility target.
    ///
    /// `scheme` must satisfy [`Scheme::is_compatibility_target`]. Arrow
    /// normalization validates and cheaply clones the complete native model.
    /// Every other target is deliberately conservative: a layout-only
    /// difference is rewritten, while a difference that would reinterpret
    /// values returns a path-aware error instead of silently changing meaning.
    ///
    /// # Errors
    ///
    /// Returns an error when `scheme` is not a compatibility target, when the
    /// datatype is invalid, or when a node has no lossless representation in
    /// the target.
    pub fn into_scheme_compat(self, scheme: &Scheme) -> Result<Self> {
        let target = Target::from_scheme(scheme)?;
        preflight_schema(&self, target.kind())?;
        if target == Target::Arrow {
            return Ok(self);
        }
        normalize_dtype(target, &self, &Path::root()).map(|(value, _)| value)
    }
}

impl Field {
    /// Returns a recursively normalized field for a compatibility target.
    ///
    /// Field name, nullability, metadata, and an unchanged Arrow projection
    /// cache are retained. A physical rewrite carrying Arrow extension
    /// storage metadata is rejected instead of relabeling the extension.
    ///
    /// # Errors
    ///
    /// Returns an error when `scheme` is not a compatibility target, when the
    /// field is invalid, or when a node has no lossless representation in the
    /// target.
    pub fn into_scheme_compat(self, scheme: &Scheme) -> Result<Self> {
        let target = Target::from_scheme(scheme)?;
        preflight_schema_shape(self.dtype(), target.kind())?;
        self.validate()?;
        if target == Target::Arrow {
            return Ok(self);
        }
        let root = Path::root();
        normalize_field(target, &self, &root.field(self.name())).map(|(value, _)| value)
    }
}

/// Recursively normalizes one datatype, returning the value and whether it changed.
fn normalize_dtype(target: Target, dtype: &DataType, path: &Path<'_>) -> Result<(DataType, bool)> {
    use DataType as D;
    match dtype {
        D::List(field) => {
            let (field, changed) = normalize_item(target, field, path)?;
            if changed {
                Ok((D::list(field), true))
            } else {
                Ok((dtype.clone(), false))
            }
        }
        D::FixedSizeList(field, length) if target.supports_fixed_size_list() => {
            let (field, changed) = normalize_item(target, field, path)?;
            if changed {
                Ok((D::fixed_size_list(field, *length)?, true))
            } else {
                Ok((dtype.clone(), false))
            }
        }
        D::ListView(field)
        | D::FixedSizeList(field, _)
        | D::LargeList(field)
        | D::LargeListView(field) => {
            let (field, _) = normalize_item(target, field, path)?;
            Ok((D::list(field), true))
        }
        D::Struct(fields) => normalize_struct(target, dtype, fields, path),
        D::Union(..) if !target.supports_union() => incompatible(
            target,
            path,
            format_smolstr!(
                "{} has no conservative tagged-union schema equivalent",
                target.engine()
            ),
        ),
        D::Map(map) if target.supports_map() => {
            let entries_path = path.child(Segment::MapEntries);
            let (entries, changed) = normalize_field(target, map.entries(), &entries_path)?;
            if changed {
                Ok((D::map(entries, map.keys_sorted())?, true))
            } else {
                Ok((dtype.clone(), false))
            }
        }
        D::Map(_) => incompatible(
            target,
            path,
            format_smolstr!(
                "{} has no first-class map type; use a list of key/value structs",
                target.engine()
            ),
        ),
        D::Dictionary(dictionary) => {
            let value_path = path.child(Segment::DictionaryValue);
            let (value, _) = normalize_dtype(target, dictionary.value(), &value_path)?;
            Ok((value, true))
        }
        D::RunEndEncoded(encoded) => normalize_run_end_encoded(target, encoded, path),
        scalar => normalize_scalar(target, scalar, path),
    }
}

fn normalize_item(target: Target, field: &Field, path: &Path<'_>) -> Result<(Field, bool)> {
    let item = path.child(Segment::Item);
    normalize_field(target, field, &item.field(field.name()))
}

fn normalize_struct(
    target: Target,
    dtype: &DataType,
    fields: &super::Fields,
    path: &Path<'_>,
) -> Result<(DataType, bool)> {
    for (index, field) in fields.iter().enumerate() {
        let child_path = path.field(field.name());
        let (child_dtype, child_changed) = normalize_dtype(target, field.dtype(), &child_path)?;
        if !child_changed {
            continue;
        }
        let mut transformed = Vec::new();
        transformed
            .try_reserve_exact(fields.len())
            .map_err(|error| {
                compatibility_error(
                    target,
                    path,
                    format_smolstr!(
                        "could not reserve {} transformed fields: {error}",
                        fields.len()
                    ),
                )
            })?;
        transformed.extend(fields.iter().take(index).cloned());
        transformed.push(field_with_dtype(target, field, child_dtype, &child_path)?);
        for remaining in fields.iter().skip(index + 1) {
            let remaining_path = path.field(remaining.name());
            transformed.push(normalize_field(target, remaining, &remaining_path)?.0);
        }
        return Ok((DataType::from_fields(transformed)?, true));
    }
    Ok((dtype.clone(), false))
}

fn normalize_run_end_encoded(
    target: Target,
    encoded: &super::RunEndEncodedType,
    path: &Path<'_>,
) -> Result<(DataType, bool)> {
    if has_extension_storage(encoded.run_ends()) {
        return Err(compatibility_error(
            target,
            &path.child(Segment::RunEnds),
            SmolStr::new_static(
                "dropping run-end encoding would discard run-end extension storage metadata",
            ),
        ));
    }

    let values_path = path.child(Segment::RunEndValues);
    if has_extension_storage(encoded.values()) {
        return Err(compatibility_error(
            target,
            &values_path,
            SmolStr::new_static(
                "dropping run-end encoding would discard logical-value extension storage metadata",
            ),
        ));
    }
    let (values, _) = normalize_dtype(target, encoded.values().dtype(), &values_path)?;
    Ok((values, true))
}

/// Applies the target's scalar matrix to one leaf datatype.
fn normalize_scalar(target: Target, dtype: &DataType, path: &Path<'_>) -> Result<(DataType, bool)> {
    match target {
        Target::Arrow => Ok((dtype.clone(), false)),
        Target::Spark => spark_scalar(dtype, path),
        Target::Polars => polars_scalar(dtype, path),
        Target::Pandas => pandas_scalar(dtype, path),
        Target::Iceberg => iceberg_scalar(dtype, path),
    }
}

/// The conservative Apache Spark SQL / Arrow interchange subset.
fn spark_scalar(dtype: &DataType, path: &Path<'_>) -> Result<(DataType, bool)> {
    use DataType as D;
    match dtype {
        D::Null
        | D::Boolean
        | D::Int8
        | D::Int16
        | D::Int32
        | D::Int64
        | D::Float32
        | D::Float64
        | D::Date32
        | D::Binary
        | D::Utf8 => Ok((dtype.clone(), false)),
        D::UInt8 => Ok((D::Int16, true)),
        D::UInt16 => Ok((D::Int32, true)),
        D::UInt32 => Ok((D::Int64, true)),
        D::UInt64 => Ok((D::decimal128(20, 0)?, true)),
        D::Float16 => Ok((D::Float32, true)),
        D::Timestamp(TimeUnit::Microsecond, _) => Ok((dtype.clone(), false)),
        D::Timestamp(unit, _) => unit_mismatch(Target::Spark, path, "timestamp", *unit, "us"),
        D::Date64 => incompatible(
            Target::Spark,
            path,
            "date64 milliseconds require a value cast to Spark date32 days",
        ),
        D::Time32(unit) | D::Time64(unit) => incompatible(
            Target::Spark,
            path,
            format_smolstr!(
                "time-of-day compatibility is Spark-version-dependent and has no conservative common encoding, got {} of {unit}",
                dtype.name()
            ),
        ),
        D::Duration32(TimeUnit::Microsecond) | D::Duration64(TimeUnit::Microsecond) => {
            Ok((dtype.clone(), false))
        }
        D::Duration32(unit) | D::Duration64(unit) => {
            unit_mismatch(Target::Spark, path, dtype.name(), *unit, "us")
        }
        D::Interval(TimeUnit::YearMonth) => Ok((dtype.clone(), false)),
        D::Interval(TimeUnit::DayTime) => incompatible(
            Target::Spark,
            path,
            "Spark day-time intervals use duration(microsecond), not interval(day_time)",
        ),
        D::Interval(TimeUnit::MonthDayNano) => incompatible(
            Target::Spark,
            path,
            "interval(month_day_nano) is not in the conservative cross-version Spark interchange subset",
        ),
        D::Interval(unit) => incompatible(
            Target::Spark,
            path,
            format_smolstr!("expected an interval layout, got {unit}"),
        ),
        D::FixedSizeBinary(_) | D::LargeBinary | D::BinaryView => Ok((D::Binary, true)),
        // No fixed-width text here; ASCII is text, so it exchanges as `utf8`
        // and the cast trims the padding.
        D::LargeUtf8
        | D::Utf8View
        | D::Ascii
        | D::FixedAscii(_)
        | D::Country
        | D::Currency
        | D::Mic
        | D::Cfi => Ok((D::Utf8, true)),
        // Only Iceberg names an identifier type; everywhere else a GUID
        // rewrites to the hyphenated spelling it renders as.
        D::Guid => Ok((D::Utf8, true)),
        D::Decimal32 { precision, scale }
        | D::Decimal64 { precision, scale }
        | D::Decimal128 { precision, scale } => {
            narrow_decimal(Target::Spark, dtype, *precision, *scale, path)
        }
        D::Decimal256 { precision, scale } => incompatible(
            Target::Spark,
            path,
            format_smolstr!(
                "decimal256({precision}, {scale}) requires a coefficient value cast and Spark precision is limited to 38"
            ),
        ),
        // Spark 4 has a VARIANT, but it is one Spark version old and its
        // Arrow interchange is not settled - outside the conservative subset.
        D::Variant => incompatible(
            Target::Spark,
            path,
            "Spark's variant is Spark-4-only and has no conservative Arrow interchange encoding",
        ),
        D::Geometry(_) | D::Geography(_) => incompatible(
            Target::Spark,
            path,
            format_smolstr!(
                "Spark has no first-class geospatial type; got {}",
                dtype.name()
            ),
        ),
        other => unreachable_container(Target::Spark, other, path),
    }
}

/// The Polars interchange subset.
///
/// Polars keeps native unsigned integers and a fixed-length `Array` layout, so
/// fewer widenings are needed than for Spark. It has no tagged union, no map,
/// and no calendar interval, and its time-of-day and duration resolutions are
/// fixed.
fn polars_scalar(dtype: &DataType, path: &Path<'_>) -> Result<(DataType, bool)> {
    use DataType as D;
    match dtype {
        D::Null
        | D::Boolean
        | D::Int8
        | D::Int16
        | D::Int32
        | D::Int64
        | D::UInt8
        | D::UInt16
        | D::UInt32
        | D::UInt64
        | D::Float32
        | D::Float64
        | D::Date32
        | D::Binary
        | D::Utf8 => Ok((dtype.clone(), false)),
        D::Float16 => Ok((D::Float32, true)),
        // Polars datetimes are millisecond, microsecond, or nanosecond.
        D::Timestamp(TimeUnit::Millisecond | TimeUnit::Microsecond | TimeUnit::Nanosecond, _) => {
            Ok((dtype.clone(), false))
        }
        D::Timestamp(unit, _) => {
            unit_mismatch(Target::Polars, path, "timestamp", *unit, "ms, us, or ns")
        }
        D::Date64 => incompatible(
            Target::Polars,
            path,
            "date64 milliseconds require a value cast to Polars date32 days",
        ),
        // Polars `Time` is nanoseconds since midnight.
        D::Time64(TimeUnit::Nanosecond) => Ok((dtype.clone(), false)),
        D::Time32(unit) | D::Time64(unit) => {
            unit_mismatch(Target::Polars, path, "time-of-day", *unit, "time64 of ns")
        }
        D::Duration32(TimeUnit::Millisecond | TimeUnit::Microsecond | TimeUnit::Nanosecond)
        | D::Duration64(TimeUnit::Millisecond | TimeUnit::Microsecond | TimeUnit::Nanosecond) => {
            Ok((dtype.clone(), false))
        }
        D::Duration32(unit) | D::Duration64(unit) => {
            unit_mismatch(Target::Polars, path, dtype.name(), *unit, "ms, us, or ns")
        }
        D::Interval(unit) => incompatible(
            Target::Polars,
            path,
            format_smolstr!("Polars has no calendar interval type, got interval({unit})"),
        ),
        D::FixedSizeBinary(_) | D::LargeBinary | D::BinaryView => Ok((D::Binary, true)),
        // No fixed-width text here; ASCII is text, so it exchanges as `utf8`
        // and the cast trims the padding.
        D::LargeUtf8
        | D::Utf8View
        | D::Ascii
        | D::FixedAscii(_)
        | D::Country
        | D::Currency
        | D::Mic
        | D::Cfi => Ok((D::Utf8, true)),
        // Only Iceberg names an identifier type; everywhere else a GUID
        // rewrites to the hyphenated spelling it renders as.
        D::Guid => Ok((D::Utf8, true)),
        D::Decimal32 { precision, scale }
        | D::Decimal64 { precision, scale }
        | D::Decimal128 { precision, scale } => {
            narrow_decimal(Target::Polars, dtype, *precision, *scale, path)
        }
        D::Decimal256 { precision, scale } => incompatible(
            Target::Polars,
            path,
            format_smolstr!(
                "decimal256({precision}, {scale}) requires a coefficient value cast and Polars precision is limited to 38"
            ),
        ),
        D::Variant | D::Geometry(_) | D::Geography(_) => incompatible(
            Target::Polars,
            path,
            format_smolstr!("Polars has no {} type; got {}", dtype.kind(), dtype.name()),
        ),
        other => unreachable_container(Target::Polars, other, path),
    }
}

/// The pandas interchange subset.
///
/// pandas materializes through NumPy and its extension dtypes, so temporal
/// values are nanosecond-resolution and there is no fixed-width binary,
/// calendar interval, union, or map dtype.
fn pandas_scalar(dtype: &DataType, path: &Path<'_>) -> Result<(DataType, bool)> {
    use DataType as D;
    match dtype {
        D::Null
        | D::Boolean
        | D::Int8
        | D::Int16
        | D::Int32
        | D::Int64
        | D::UInt8
        | D::UInt16
        | D::UInt32
        | D::UInt64
        | D::Float32
        | D::Float64
        | D::Date32
        | D::Binary
        | D::Utf8 => Ok((dtype.clone(), false)),
        D::Float16 => Ok((D::Float32, true)),
        // `datetime64[ns]` is the pandas timestamp representation.
        D::Timestamp(TimeUnit::Nanosecond, _) => Ok((dtype.clone(), false)),
        D::Timestamp(unit, _) => unit_mismatch(Target::Pandas, path, "timestamp", *unit, "ns"),
        D::Date64 => incompatible(
            Target::Pandas,
            path,
            "date64 milliseconds require a value cast to a pandas date32 day representation",
        ),
        D::Time32(unit) | D::Time64(unit) => incompatible(
            Target::Pandas,
            path,
            format_smolstr!(
                "pandas has no time-of-day dtype and materializes it as opaque objects, got {} of {unit}",
                dtype.name()
            ),
        ),
        // `timedelta64[ns]` is the pandas duration representation.
        D::Duration32(TimeUnit::Nanosecond) | D::Duration64(TimeUnit::Nanosecond) => {
            Ok((dtype.clone(), false))
        }
        D::Duration32(unit) | D::Duration64(unit) => {
            unit_mismatch(Target::Pandas, path, dtype.name(), *unit, "ns")
        }
        D::Interval(unit) => incompatible(
            Target::Pandas,
            path,
            format_smolstr!(
                "a pandas IntervalDtype describes value bounds, not an Arrow calendar interval, got interval({unit})"
            ),
        ),
        D::FixedSizeBinary(_) | D::LargeBinary | D::BinaryView => Ok((D::Binary, true)),
        // No fixed-width text here; ASCII is text, so it exchanges as `utf8`
        // and the cast trims the padding.
        D::LargeUtf8
        | D::Utf8View
        | D::Ascii
        | D::FixedAscii(_)
        | D::Country
        | D::Currency
        | D::Mic
        | D::Cfi => Ok((D::Utf8, true)),
        // Only Iceberg names an identifier type; everywhere else a GUID
        // rewrites to the hyphenated spelling it renders as.
        D::Guid => Ok((D::Utf8, true)),
        D::Decimal32 { precision, scale }
        | D::Decimal64 { precision, scale }
        | D::Decimal128 { precision, scale } => {
            narrow_decimal(Target::Pandas, dtype, *precision, *scale, path)
        }
        D::Decimal256 { precision, scale } => incompatible(
            Target::Pandas,
            path,
            format_smolstr!(
                "decimal256({precision}, {scale}) exceeds the 38-digit decimal precision pandas exchanges through Arrow"
            ),
        ),
        D::Variant | D::Geometry(_) | D::Geography(_) => incompatible(
            Target::Pandas,
            path,
            format_smolstr!(
                "pandas has no {} column type; got {}",
                dtype.kind(),
                dtype.name()
            ),
        ),
        other => unreachable_container(Target::Pandas, other, path),
    }
}

/// The Apache Iceberg table-format subset.
///
/// Iceberg's primitive vocabulary is closed: `boolean`, `int`, `long`, `float`,
/// `double`, `decimal(p<=38, s)`, `date`, `time`, `timestamp`/`timestamptz`,
/// `timestamp_ns`/`timestamptz_ns`, `string`, `uuid`, `fixed[n]`, `binary`, and
/// `unknown`. There is no unsigned integer, so a narrow or unsigned width widens
/// to the signed one that holds it, and there is no elapsed-time or calendar
/// interval type at all. Time-of-day is microseconds and a timestamp is
/// microseconds or nanoseconds, so any other resolution is a value cast.
fn iceberg_scalar(dtype: &DataType, path: &Path<'_>) -> Result<(DataType, bool)> {
    use DataType as D;
    match dtype {
        // `unknown` is Iceberg's always-null primitive.
        D::Null
        | D::Boolean
        | D::Int32
        | D::Int64
        | D::Float32
        | D::Float64
        | D::Date32
        | D::Binary
        | D::Utf8
        // `fixed[n]`, which is also how `uuid` is stored.
        | D::FixedSizeBinary(_)
        // Iceberg is the one target that names an identifier type.
        | D::Guid => Ok((dtype.clone(), false)),
        D::Int8 | D::Int16 | D::UInt8 | D::UInt16 => Ok((D::Int32, true)),
        D::UInt32 => Ok((D::Int64, true)),
        D::UInt64 => Ok((D::decimal128(20, 0)?, true)),
        D::Float16 => Ok((D::Float32, true)),
        D::Date64 => incompatible(
            Target::Iceberg,
            path,
            "date64 milliseconds require a value cast to Iceberg date32 days",
        ),
        // Iceberg `time` is microseconds since midnight.
        D::Time64(TimeUnit::Microsecond) => Ok((dtype.clone(), false)),
        D::Time32(unit) | D::Time64(unit) => {
            unit_mismatch(Target::Iceberg, path, "time-of-day", *unit, "us")
        }
        // `timestamp`/`timestamptz` are microseconds; the `_ns` pair is nanoseconds.
        D::Timestamp(TimeUnit::Microsecond | TimeUnit::Nanosecond, _) => {
            Ok((dtype.clone(), false))
        }
        D::Timestamp(unit, _) => {
            unit_mismatch(Target::Iceberg, path, "timestamp", *unit, "us or ns")
        }
        D::Duration32(unit) | D::Duration64(unit) => incompatible(
            Target::Iceberg,
            path,
            format_smolstr!("Iceberg has no elapsed-time type, got {}({unit})", dtype.name()),
        ),
        D::Interval(unit) => incompatible(
            Target::Iceberg,
            path,
            format_smolstr!("Iceberg has no calendar interval type, got interval({unit})"),
        ),
        D::LargeBinary | D::BinaryView => Ok((D::Binary, true)),
        // Iceberg has `string` and `fixed[n]`; an ASCII column is text, so
        // every Iceberg reader must see `USD`, never the padded bytes.
        D::LargeUtf8
        | D::Utf8View
        | D::Ascii
        | D::FixedAscii(_)
        | D::Country
        | D::Currency
        | D::Mic
        | D::Cfi => Ok((D::Utf8, true)),
        D::Decimal32 { precision, scale }
        | D::Decimal64 { precision, scale }
        | D::Decimal128 { precision, scale } => {
            narrow_decimal(Target::Iceberg, dtype, *precision, *scale, path)
        }
        D::Decimal256 { precision, scale } => incompatible(
            Target::Iceberg,
            path,
            format_smolstr!(
                "decimal256({precision}, {scale}) requires a coefficient value cast and Iceberg precision is limited to 38"
            ),
        ),
        // Iceberg v3 owns all three: `variant`, `geometry(C)`, and
        // `geography(C, A)` are the format's own spellings, parameters
        // included, so each passes unchanged.
        D::Variant | D::Geometry(_) | D::Geography(_) => Ok((dtype.clone(), false)),
        other => unreachable_container(Target::Iceberg, other, path),
    }
}

/// Narrows any exact decimal to the 128-bit interchange width.
fn narrow_decimal(
    target: Target,
    dtype: &DataType,
    precision: u8,
    scale: i8,
    path: &Path<'_>,
) -> Result<(DataType, bool)> {
    if scale < 0 {
        return incompatible(
            target,
            path,
            format_smolstr!(
                "expected a non-negative decimal scale, got {scale}; {} does not exchange negative scales",
                target.engine()
            ),
        );
    }
    let transformed = DataType::decimal128(precision, scale)?;
    let changed = !matches!(dtype, DataType::Decimal128 { .. });
    Ok((transformed, changed))
}

fn normalize_field(target: Target, field: &Field, path: &Path<'_>) -> Result<(Field, bool)> {
    let (dtype, changed) = normalize_dtype(target, field.dtype(), path)?;
    if !changed {
        return Ok((field.clone(), false));
    }
    field_with_dtype(target, field, dtype, path).map(|field| (field, true))
}

fn field_with_dtype(
    target: Target,
    field: &Field,
    dtype: DataType,
    path: &Path<'_>,
) -> Result<Field> {
    if has_extension_storage(field) {
        return incompatible(
            target,
            path,
            format_smolstr!(
                "normalizing {} to {} would relabel Arrow extension storage",
                elide_display(field.dtype()),
                elide_display(&dtype)
            ),
        );
    }
    let mut transformed = field.clone();
    transformed.set_dtype(dtype)?;
    Ok(transformed)
}

/// Reports whether a field still carries a *foreign* Arrow extension label.
///
/// The extensions this workspace owns never reach here: `arrow.parquet.variant`,
/// `geoarrow.wkb`, `yggdryl.ascii`, `arrow.uuid`, and each registered code's
/// own `yggdryl.{country,currency,mic,cfi}` import as the first-class
/// `variant`, `geometry`, `geography`, ASCII-width, `guid` and code datatypes
/// with their `ARROW:extension:*` keys stripped, so a field carrying these
/// keys names an extension the workspace does not model.
/// Rewriting its storage would silently relabel that foreign type, so the
/// walker rejects the rewrite instead.
fn has_extension_storage(field: &Field) -> bool {
    field.has_metadata(ARROW_EXTENSION_NAME_KEY) || field.has_metadata(ARROW_EXTENSION_METADATA_KEY)
}

fn unit_mismatch<T>(
    target: Target,
    path: &Path<'_>,
    label: &str,
    actual: TimeUnit,
    expected: &str,
) -> Result<T> {
    incompatible(
        target,
        path,
        format_smolstr!(
            "{}; a resolution change is a value cast, not a schema normalization",
            expected_got(format_args!("{label} of {expected}"), actual)
        ),
    )
}

/// Reports a container variant that the generic walker should already have handled.
fn unreachable_container<T>(target: Target, dtype: &DataType, path: &Path<'_>) -> Result<T> {
    incompatible(
        target,
        path,
        format_smolstr!(
            "expected a scalar datatype, got {}; this container is handled by the generic walker",
            dtype.name()
        ),
    )
}

fn incompatible<T>(target: Target, path: &Path<'_>, reason: impl Into<SmolStr>) -> Result<T> {
    Err(compatibility_error(target, path, reason.into()))
}

fn compatibility_error(target: Target, path: &Path<'_>, reason: SmolStr) -> Error {
    Error::InvalidDataType {
        kind: target.kind(),
        reason: format_smolstr!("{}: {reason}", path.render()),
    }
}
