//! Bounded canonical scalar defaults for every logical datatype.

use std::collections::TryReserveError;

use smol_str::{SmolStr, format_smolstr};

use crate::types::{Integer, Temporal, push_field_name_path};
use crate::{Error, Field, Result, Scalar, TimeUnit};

use super::DataType;

const MAX_DEFAULT_NODES: usize = 1_000_000;
const MAX_DEFAULT_BYTES: usize = 64 * 1024 * 1024;

enum DefaultPlan {
    Null,
    Bool,
    Signed,
    Unsigned,
    Float,
    Decimal,
    Decimal256,
    Interval(TimeUnit),
    String,
    Bytes(usize),
    EmptySequence,
    Sequence(Vec<Self>),
    Repeated(Box<Self>, usize),
    Union(i8, Box<Self>),
    EmptyMapping,
    /// The 21-byte `POINT EMPTY` Well-Known Binary, a present empty geometry.
    PointEmpty,
    /// The nil identifier, sixteen zero bytes in its hyphenated spelling.
    Uuid,
    /// The minimum canonical version.
    Version,
}

struct Planned {
    plan: DefaultPlan,
    nodes: usize,
    bytes: usize,
    logically_null: bool,
}

enum PlanningError {
    Uninhabited(SmolStr),
    Fatal(SmolStr),
    Public(Error),
}

type PlanningResult<T> = std::result::Result<T, PlanningError>;

#[derive(Clone, Copy)]
enum PathSegment<'a> {
    Field(&'a str),
    Item,
    Union(i8, &'a str),
    DictionaryValue,
    RunEndValues,
}

impl DataType {
    /// Materializes this datatype's bounded canonical scalar default.
    ///
    /// The result prefers a present value. [`DataType::Null`] and logical
    /// wrappers that contain only null values are the intrinsic exceptions.
    /// Nested Struct and fixed-size-list children use [`Field::default_value`],
    /// so their own nullability remains authoritative.
    pub fn default_value(&self) -> Result<Scalar> {
        preflight_schema(self, "DefaultValue")?;
        let mut path = Vec::new();
        let planned = plan_dtype(self, &mut path).map_err(public_planning_error)?;
        super::value::dtype_scalar(self, materialize(planned.plan)?)
    }

    /// Tests whether a value is this datatype's canonical default.
    ///
    /// The comparison reuses the bounded default planner without
    /// materializing variable-width bytes or repeated child values. This is
    /// useful at foreign scalar boundaries that need to recognize intrinsic
    /// logical-null defaults without allocating the default merely to compare
    /// it.
    pub fn is_default_value(&self, value: &Scalar) -> Result<bool> {
        preflight_schema(self, "DefaultValue")?;
        let mut path = Vec::new();
        let planned = plan_dtype(self, &mut path).map_err(public_planning_error)?;
        Ok(plan_matches_value(&planned.plan, value))
    }

    /// Validates a caller-built schema with a bounded shape walk first.
    ///
    /// The hard nesting and node limits are checked before the recursive
    /// parameter validator runs. Foreign projection boundaries should use
    /// this method before any recursive conversion.
    #[doc(hidden)]
    pub fn validate_bounded(&self) -> Result<()> {
        preflight_schema(self, "DataType")
    }

    /// Returns the type ID selected by the canonical default of a Union.
    ///
    /// This is an interop planning hook for physical dense-union builders. It
    /// avoids visiting or allocating inactive branches while retaining the
    /// core planner as the single authority for branch selection.
    #[doc(hidden)]
    pub fn default_union_type_id(&self) -> Result<Option<i8>> {
        preflight_schema(self, "DefaultValue")?;
        let mut path = Vec::new();
        let planned = plan_dtype(self, &mut path).map_err(public_planning_error)?;
        Ok(match planned.plan {
            DefaultPlan::Union(type_id, _) => Some(type_id),
            _ => None,
        })
    }
}

/// Materializes a Field default while applying its nullability policy.
pub(crate) fn default_value_for_field(field: &Field) -> Result<Scalar> {
    preflight_schema_shape(field.dtype(), "DefaultValue")?;
    field.validate()?;
    let mut path = Vec::new();
    path.push(PathSegment::Field(field.name()));
    let planned = plan_field(field, &mut path).map_err(public_planning_error)?;
    super::value::dtype_scalar(field.dtype(), materialize(planned.plan)?)
}

pub(crate) fn preflight_schema(dtype: &DataType, kind: &'static str) -> Result<()> {
    preflight_schema_shape(dtype, kind)?;
    // The depth walk runs before this recursive validator so caller-built
    // public enum variants cannot turn validation into stack exhaustion.
    dtype.validate()
}

pub(crate) fn preflight_schema_shape(dtype: &DataType, kind: &'static str) -> Result<()> {
    let mut pending = Vec::new();
    reserve_pending(&mut pending, 0, 1, kind)?;
    pending.push((dtype, 0_usize));
    let mut visited = 0_usize;
    while let Some((current, depth)) = pending.pop() {
        if depth >= DataType::PARSE_RECURSION_LIMIT {
            return Err(schema_preflight_error(
                kind,
                "$",
                format_smolstr!(
                    "schema nesting exceeds the hard limit of {}",
                    DataType::PARSE_RECURSION_LIMIT
                ),
            ));
        }
        visited = visited.checked_add(1).ok_or_else(|| {
            schema_preflight_error(
                kind,
                "$",
                "schema node count overflowed the platform size".into(),
            )
        })?;
        if visited > MAX_DEFAULT_NODES {
            return Err(schema_preflight_error(
                kind,
                "$",
                format_smolstr!("schema exceeds the {MAX_DEFAULT_NODES} node safety limit"),
            ));
        }
        let child_depth = depth + 1;
        match current {
            DataType::List(field)
            | DataType::ListView(field)
            | DataType::FixedSizeList(field, _)
            | DataType::LargeList(field)
            | DataType::LargeListView(field) => {
                reserve_pending(&mut pending, visited, 1, kind)?;
                pending.push((field.dtype(), child_depth));
            }
            DataType::Struct(fields) => {
                reserve_pending(&mut pending, visited, fields.len(), kind)?;
                pending.extend(fields.iter().map(|field| (field.dtype(), child_depth)))
            }
            DataType::Union(fields, _) => {
                reserve_pending(&mut pending, visited, fields.len(), kind)?;
                pending.extend(
                    fields
                        .iter()
                        .map(|(_, field)| (field.dtype(), child_depth)),
                );
            }
            DataType::Dictionary(dictionary) => {
                reserve_pending(&mut pending, visited, 2, kind)?;
                pending.push((dictionary.key(), child_depth));
                pending.push((dictionary.value(), child_depth));
            }
            DataType::Map(map) => {
                reserve_pending(&mut pending, visited, 1, kind)?;
                pending.push((map.entries().dtype(), child_depth));
            }
            DataType::RunEndEncoded(encoded) => {
                reserve_pending(&mut pending, visited, 2, kind)?;
                pending.push((encoded.run_ends().dtype(), child_depth));
                pending.push((encoded.values().dtype(), child_depth));
            }
            DataType::Null
            | DataType::Boolean
            | DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Float16
            | DataType::Float32
            | DataType::Float64
            | DataType::DateTime64 { .. }
            | DataType::Date32
            | DataType::Date64
            | DataType::Time32(_)
            | DataType::Time64(_)
            | DataType::Duration32(_)
            | DataType::Duration64(_)
            | DataType::Interval(_)
            | DataType::Binary
            | DataType::FixedSizeBinary(_)
            | DataType::LargeBinary
            | DataType::BinaryView
            | DataType::Utf8
            | DataType::LargeUtf8
            | DataType::Utf8View
            | DataType::Ascii
            | DataType::FixedAscii(_)
            | DataType::Country
            | DataType::Currency
            | DataType::Mic
            | DataType::Cfi
            | DataType::Uuid
            | DataType::Version
            | DataType::Decimal32 { .. }
            | DataType::Decimal64 { .. }
            | DataType::Decimal128 { .. }
            | DataType::Decimal256 { .. }
            // A variant declares its types per value and a geometry is one
            // WKB payload: neither holds child fields for the walk to visit.
            | DataType::Variant
            | DataType::Geometry(_)
            | DataType::Geography(_) => {}
        }
    }
    Ok(())
}

fn reserve_pending(
    pending: &mut Vec<(&DataType, usize)>,
    visited: usize,
    additional: usize,
    kind: &'static str,
) -> Result<()> {
    let discovered = visited.checked_add(pending.len()).ok_or_else(|| {
        schema_preflight_error(
            kind,
            "$",
            "schema node count overflowed the platform size".into(),
        )
    })?;
    let remaining = MAX_DEFAULT_NODES.checked_sub(discovered).ok_or_else(|| {
        schema_preflight_error(
            kind,
            "$",
            format_smolstr!("schema exceeds the {MAX_DEFAULT_NODES} node safety limit"),
        )
    })?;
    if additional > remaining {
        return Err(schema_preflight_error(
            kind,
            "$",
            format_smolstr!("schema exceeds the {MAX_DEFAULT_NODES} node safety limit"),
        ));
    }
    pending.try_reserve(additional).map_err(|error| {
        schema_preflight_error(
            kind,
            "$",
            format_smolstr!("schema traversal allocation could not be reserved: {error}"),
        )
    })
}

#[allow(clippy::too_many_lines)]
fn plan_dtype<'a>(dtype: &'a DataType, path: &mut Vec<PathSegment<'a>>) -> PlanningResult<Planned> {
    use DataType as D;
    match dtype {
        D::Null => scalar(DefaultPlan::Null, true),
        D::Boolean => scalar(DefaultPlan::Bool, false),
        D::Int8
        | D::Int16
        | D::Int32
        | D::Int64
        | D::DateTime64 { .. }
        | D::Date32
        | D::Date64
        | D::Time32(_)
        | D::Time64(_)
        | D::Duration32(_)
        | D::Duration64(_) => scalar(DefaultPlan::Signed, false),
        D::UInt8 | D::UInt16 | D::UInt32 | D::UInt64 => scalar(DefaultPlan::Unsigned, false),
        D::Float16 | D::Float32 | D::Float64 => scalar(DefaultPlan::Float, false),
        D::Interval(unit) if unit.is_interval() => scalar(DefaultPlan::Interval(*unit), false),
        D::Interval(_) => fatal(path, "invalid interval layout"),
        D::Binary | D::LargeBinary | D::BinaryView => plan_bytes(0, path),
        // The nil identifier: sixteen zero bytes, rendered as its hyphenated
        // spelling like every other UUID value.
        D::Uuid => scalar(DefaultPlan::Uuid, false),
        D::Version => scalar(DefaultPlan::Version, false),
        D::FixedSizeBinary(width) => {
            let width = usize::try_from(*width)
                .map_err(|_| fatal_error(path, "fixed binary width is negative"))?;
            plan_bytes(width, path)
        }
        // An ASCII width, and a code over one, defaults to the empty string;
        // storage pads it to all-NUL.
        D::Utf8
        | D::LargeUtf8
        | D::Utf8View
        | D::Ascii
        | D::FixedAscii(_)
        | D::Country
        | D::Currency
        | D::Mic
        | D::Cfi => scalar(DefaultPlan::String, false),
        D::List(_) | D::ListView(_) | D::LargeList(_) | D::LargeListView(_) => {
            scalar(DefaultPlan::EmptySequence, false)
        }
        D::FixedSizeList(field, length) => {
            let length = usize::try_from(*length)
                .map_err(|_| fatal_error(path, "fixed-size-list length is negative"))?;
            if length == 0 {
                return scalar(DefaultPlan::EmptySequence, false);
            }
            path.push(PathSegment::Item);
            path.push(PathSegment::Field(field.name()));
            let child = plan_field(field, path);
            path.pop();
            path.pop();
            let child = child?;
            let nodes = checked_add(
                1,
                checked_mul(child.nodes, length, path, "fixed-size-list node count")?,
                path,
                "fixed-size-list node count",
            )?;
            let bytes = checked_mul(child.bytes, length, path, "fixed-size-list byte count")?;
            ensure_budget(nodes, bytes, path)?;
            Ok(Planned {
                plan: DefaultPlan::Repeated(Box::new(child.plan), length),
                nodes,
                bytes,
                logically_null: false,
            })
        }
        D::Struct(fields) => {
            let mut plans = Vec::new();
            plans
                .try_reserve_exact(fields.len())
                .map_err(|error| allocation_planning_error(path, error))?;
            let mut nodes = 1_usize;
            let mut bytes = 0_usize;
            for field in fields {
                path.push(PathSegment::Field(field.name()));
                let child = plan_field(field, path);
                path.pop();
                let child = child?;
                nodes = checked_add(nodes, child.nodes, path, "struct node count")?;
                bytes = checked_add(bytes, child.bytes, path, "struct byte count")?;
                ensure_budget(nodes, bytes, path)?;
                plans.push(child.plan);
            }
            Ok(Planned {
                plan: DefaultPlan::Sequence(plans),
                nodes,
                bytes,
                logically_null: false,
            })
        }
        D::Union(fields, _) => plan_union(fields, path),
        D::Dictionary(dictionary) => {
            path.push(PathSegment::DictionaryValue);
            let value = plan_dtype(dictionary.value(), path);
            path.pop();
            value
        }
        D::Decimal32 { .. } | D::Decimal64 { .. } | D::Decimal128 { .. } => {
            scalar(DefaultPlan::Decimal, false)
        }
        D::Decimal256 { .. } => scalar(DefaultPlan::Decimal256, false),
        // The variant's present zero value is the variant null: a variant can
        // hold null as a first-class value, so `Scalar::Null` here is a value,
        // not an absence, and the plan is not logically null.
        D::Variant => scalar(DefaultPlan::Null, false),
        // The geospatial pair's present empty value is `POINT EMPTY`.
        D::Geometry(_) | D::Geography(_) => scalar(DefaultPlan::PointEmpty, false),
        D::Map(_) => scalar(DefaultPlan::EmptyMapping, false),
        D::RunEndEncoded(encoded) => {
            path.push(PathSegment::RunEndValues);
            let present = plan_present_field(encoded.values(), path);
            match present {
                Ok(value) => {
                    path.pop();
                    return Ok(value);
                }
                Err(PlanningError::Uninhabited(_)) => {}
                Err(error) => {
                    path.pop();
                    return Err(error);
                }
            }
            let null = plan_physical_null_field(encoded.values(), path);
            path.pop();
            match null? {
                Some(value) => Ok(value),
                None => uninhabited(path, "run-end values have no constructible logical default"),
            }
        }
    }
}

fn plan_field<'a>(field: &'a Field, path: &mut Vec<PathSegment<'a>>) -> PlanningResult<Planned> {
    if field.is_nullable() {
        return plan_physical_null(field.dtype(), path)?.ok_or_else(|| {
            uninhabited_error(
                path,
                "nullable physical layout cannot represent logical null",
            )
        });
    }
    plan_present_field(field, path)
}

fn plan_present_field<'a>(
    field: &'a Field,
    path: &mut Vec<PathSegment<'a>>,
) -> PlanningResult<Planned> {
    let value = plan_dtype(field.dtype(), path)?;
    if value.logically_null {
        uninhabited(path, "non-nullable field has only a logical-null default")
    } else {
        Ok(value)
    }
}

fn plan_union<'a>(
    fields: &'a crate::UnionFields,
    path: &mut Vec<PathSegment<'a>>,
) -> PlanningResult<Planned> {
    for (type_id, field) in fields {
        path.push(PathSegment::Union(type_id, field.name()));
        let candidate = plan_present_field(field, path);
        path.pop();
        match candidate {
            Ok(value) => return wrap_union(type_id, value, path),
            Err(PlanningError::Uninhabited(_)) => {}
            Err(error) => return Err(error),
        }
    }
    for (type_id, field) in fields {
        path.push(PathSegment::Union(type_id, field.name()));
        let candidate = plan_physical_null_field(field, path);
        path.pop();
        if let Some(value) = candidate? {
            return wrap_union(type_id, value, path);
        }
    }
    uninhabited(path, "union has no constructible member default")
}

fn plan_physical_null_field<'a>(
    field: &'a Field,
    path: &mut Vec<PathSegment<'a>>,
) -> PlanningResult<Option<Planned>> {
    if !field.is_nullable() {
        return Ok(None);
    }
    plan_physical_null(field.dtype(), path)
}

fn plan_physical_null<'a>(
    dtype: &'a DataType,
    path: &mut Vec<PathSegment<'a>>,
) -> PlanningResult<Option<Planned>> {
    match dtype {
        DataType::Union(fields, _) => {
            for (type_id, field) in fields {
                path.push(PathSegment::Union(type_id, field.name()));
                let candidate = plan_physical_null_field(field, path);
                path.pop();
                if let Some(value) = candidate? {
                    return wrap_union(type_id, value, path).map(Some);
                }
            }
            Ok(None)
        }
        DataType::RunEndEncoded(encoded) => {
            path.push(PathSegment::RunEndValues);
            let value = plan_physical_null_field(encoded.values(), path);
            path.pop();
            value
        }
        _ => scalar(DefaultPlan::Null, true).map(Some),
    }
}

fn wrap_union(type_id: i8, value: Planned, path: &[PathSegment<'_>]) -> PlanningResult<Planned> {
    let nodes = checked_add(value.nodes, 2, path, "union node count")?;
    ensure_budget(nodes, value.bytes, path)?;
    Ok(Planned {
        plan: DefaultPlan::Union(type_id, Box::new(value.plan)),
        nodes,
        bytes: value.bytes,
        logically_null: value.logically_null,
    })
}

fn plan_bytes(width: usize, path: &[PathSegment<'_>]) -> PlanningResult<Planned> {
    ensure_budget(1, width, path)?;
    Ok(Planned {
        plan: DefaultPlan::Bytes(width),
        nodes: 1,
        bytes: width,
        logically_null: false,
    })
}

fn scalar(plan: DefaultPlan, logically_null: bool) -> PlanningResult<Planned> {
    Ok(Planned {
        plan,
        nodes: 1,
        bytes: 0,
        logically_null,
    })
}

fn checked_add(
    left: usize,
    right: usize,
    path: &[PathSegment<'_>],
    what: &str,
) -> PlanningResult<usize> {
    left.checked_add(right)
        .ok_or_else(|| fatal_error(path, format_smolstr!("{what} overflowed")))
}

fn checked_mul(
    left: usize,
    right: usize,
    path: &[PathSegment<'_>],
    what: &str,
) -> PlanningResult<usize> {
    left.checked_mul(right)
        .ok_or_else(|| fatal_error(path, format_smolstr!("{what} overflowed")))
}

fn ensure_budget(nodes: usize, bytes: usize, path: &[PathSegment<'_>]) -> PlanningResult<()> {
    if nodes > MAX_DEFAULT_NODES {
        return fatal(
            path,
            format_smolstr!("default exceeds the {MAX_DEFAULT_NODES} node safety limit"),
        );
    }
    if bytes > MAX_DEFAULT_BYTES {
        return fatal(
            path,
            format_smolstr!("default exceeds the {MAX_DEFAULT_BYTES} byte safety limit"),
        );
    }
    Ok(())
}

fn materialize(plan: DefaultPlan) -> Result<Scalar> {
    match plan {
        DefaultPlan::Null => Ok(Scalar::Null),
        DefaultPlan::Bool => Ok(Scalar::from(false)),
        DefaultPlan::Signed => Ok(Scalar::from(0_i64)),
        DefaultPlan::Unsigned => Ok(Scalar::from(0_u64)),
        DefaultPlan::Float => Ok(Scalar::from(0.0_f64)),
        DefaultPlan::Decimal => Ok(Scalar::from(0_i128)),
        DefaultPlan::Decimal256 => Ok(Scalar::d256(crate::I256::ZERO, 0)),
        DefaultPlan::Interval(unit) => crate::types::Interval::new(0, 0, 0, unit)
            .map(|value| Scalar::Temporal(Temporal::Interval(value))),
        DefaultPlan::String => Ok(Scalar::from("")),
        DefaultPlan::Bytes(width) => {
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(width)
                .map_err(|error| allocation_error("$", error))?;
            bytes.resize(width, 0);
            Ok(Scalar::from(bytes))
        }
        DefaultPlan::EmptySequence => Ok(Scalar::from_sequence([])),
        DefaultPlan::Sequence(plans) => {
            let mut values = Vec::new();
            values
                .try_reserve_exact(plans.len())
                .map_err(|error| allocation_error("$", error))?;
            for plan in plans {
                values.push(materialize(plan)?);
            }
            Ok(Scalar::from_sequence(values))
        }
        DefaultPlan::Repeated(plan, length) => {
            let value = materialize(*plan)?;
            let mut values = Vec::new();
            values
                .try_reserve_exact(length)
                .map_err(|error| allocation_error("$", error))?;
            values.resize(length, value);
            Ok(Scalar::from_sequence(values))
        }
        DefaultPlan::Union(type_id, value) => Ok(Scalar::from_sequence([
            Scalar::from(i64::from(type_id)),
            materialize(*value)?,
        ])),
        DefaultPlan::EmptyMapping => Scalar::from_mapping([]),
        // Little-endian `POINT EMPTY`: the conventional empty geometry, spelled
        // as a point whose coordinates are NaN, in the canonical geospatial
        // value spelling.
        DefaultPlan::PointEmpty => crate::types::Geometry::new(POINT_EMPTY_WKB.as_slice())
            .map(|value| Scalar::Geospatial(crate::types::Geospatial::Geometry(value))),
        DefaultPlan::Uuid => Ok(Scalar::Uuid(crate::types::Uuid::new(0))),
        DefaultPlan::Version => Ok(Scalar::Version(crate::Version::MIN)),
    }
}

/// `POINT EMPTY` in little-endian ISO WKB: order byte, type 1, NaN NaN.
pub(crate) const POINT_EMPTY_WKB: [u8; 21] = [
    0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF8, 0x7F, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0xF8, 0x7F,
];

fn plan_matches_value(plan: &DefaultPlan, value: &Scalar) -> bool {
    match plan {
        DefaultPlan::Null => matches!(value, Scalar::Null),
        DefaultPlan::Bool => value.as_bool() == Some(false),
        // A temporal zero read back off an Arrow column carries its unit and
        // zone; it is the same datum the plan's bare zero spells, so both
        // spellings are the default.
        DefaultPlan::Signed => match value {
            Scalar::Integer(Integer::I8(value)) => value.get() == 0,
            Scalar::Integer(Integer::I16(value)) => value.get() == 0,
            Scalar::Integer(Integer::I32(value)) => value.get() == 0,
            Scalar::Integer(Integer::I64(value)) => value.get() == 0,
            Scalar::Temporal(Temporal::Date32(value)) => value.count() == 0,
            Scalar::Temporal(Temporal::Date64(value)) => value.count() == 0,
            Scalar::Temporal(Temporal::Time32(value)) => value.count() == 0,
            Scalar::Temporal(Temporal::Time64(value)) => value.count() == 0,
            Scalar::Temporal(Temporal::DateTime64(value)) => value.count() == 0,
            Scalar::Temporal(Temporal::Duration32(value)) => value.count() == 0,
            Scalar::Temporal(Temporal::Duration64(value)) => value.count() == 0,
            _ => false,
        },
        DefaultPlan::Unsigned => {
            matches!(value.as_integer(), Some(integer) if !integer.is_negative() && integer.magnitude() == 0)
        }
        DefaultPlan::Float => value
            .as_f64()
            .is_some_and(|value| value.to_bits() == 0_f64.to_bits()),
        // A zero coefficient is zero at every scale.
        DefaultPlan::Decimal | DefaultPlan::Decimal256 => {
            value.as_i128() == Some(0)
                || value
                    .as_decimal()
                    .is_some_and(|(coefficient, _)| coefficient == crate::I256::ZERO)
        }
        DefaultPlan::Interval(unit) => interval_is_zero(value, *unit),
        DefaultPlan::String => value.as_str() == Some(""),
        DefaultPlan::Bytes(width) => value
            .as_bytes()
            .is_some_and(|bytes| bytes.len() == *width && bytes.iter().all(|byte| *byte == 0)),
        DefaultPlan::EmptySequence => value.as_sequence().is_some_and(<[Scalar]>::is_empty),
        DefaultPlan::Sequence(plans) => value.as_sequence().is_some_and(|values| {
            values.len() == plans.len()
                && plans
                    .iter()
                    .zip(values)
                    .all(|(plan, value)| plan_matches_value(plan, value))
        }),
        DefaultPlan::Repeated(plan, length) => value.as_sequence().is_some_and(|values| {
            values.len() == *length && values.iter().all(|value| plan_matches_value(plan, value))
        }),
        DefaultPlan::Union(type_id, payload) => value.as_sequence().is_some_and(|values| {
            let [actual_type_id, actual_payload] = values else {
                return false;
            };
            actual_type_id.as_i128() == Some(i128::from(*type_id))
                && plan_matches_value(payload, actual_payload)
        }),
        DefaultPlan::EmptyMapping => value
            .as_mapping()
            .is_some_and(<[(Scalar, Scalar)]>::is_empty),
        DefaultPlan::PointEmpty => value.as_wkb().is_some_and(|bytes| bytes == POINT_EMPTY_WKB),
        DefaultPlan::Uuid => match value {
            Scalar::Uuid(value) => value.get() == 0,
            _ => crate::types::uuid_bytes(value)
                .and_then(|bytes| crate::types::uuid_parse(bytes).ok())
                .is_some_and(|stored| stored == [0_u8; 16]),
        },
        DefaultPlan::Version => {
            matches!(value, Scalar::Version(version) if version == &crate::Version::MIN)
        }
    }
}

fn interval_is_zero(value: &Scalar, unit: TimeUnit) -> bool {
    if let Scalar::Temporal(Temporal::Interval(value)) = value {
        return value.unit() == unit
            && value.months() == 0
            && value.days() == 0
            && value.nanoseconds() == 0;
    }
    match unit {
        TimeUnit::YearMonth => value.as_i128() == Some(0),
        TimeUnit::DayTime => value.as_sequence().is_some_and(|values| {
            matches!(values, [days, milliseconds]
                if days.as_i128() == Some(0) && milliseconds.as_i128() == Some(0))
        }),
        TimeUnit::MonthDayNano => value.as_sequence().is_some_and(|values| {
            matches!(values, [months, days, nanoseconds]
                if months.as_i128() == Some(0)
                    && days.as_i128() == Some(0)
                    && nanoseconds.as_i128() == Some(0))
        }),
        _ => false,
    }
}

pub(crate) fn value_is_logically_null(dtype: &DataType, value: &Scalar) -> bool {
    // A variant can *spell* null: the variant null is a present value the
    // encoding writes, so `Null` in a variant column is a value, never the
    // absence a validity bitmap records - which is exactly why a required
    // variant column can hold it.
    if matches!(dtype, DataType::Variant) {
        return false;
    }
    if matches!(value, Scalar::Null) {
        return true;
    }
    match dtype {
        DataType::Union(fields, _) => {
            let Some([type_id, payload]) = value.as_sequence() else {
                return false;
            };
            let Some(type_id) = type_id.as_i128().and_then(|value| i8::try_from(value).ok()) else {
                return false;
            };
            fields
                .iter()
                .find(|(candidate, _)| *candidate == type_id)
                .is_some_and(|(_, field)| value_is_logically_null(field.dtype(), payload))
        }
        DataType::RunEndEncoded(encoded) => {
            value_is_logically_null(encoded.values().dtype(), value)
        }
        DataType::Dictionary(dictionary) => value_is_logically_null(dictionary.value(), value),
        _ => false,
    }
}

fn uninhabited<T>(path: &[PathSegment<'_>], reason: impl Into<SmolStr>) -> PlanningResult<T> {
    Err(uninhabited_error(path, reason))
}

fn uninhabited_error(path: &[PathSegment<'_>], reason: impl Into<SmolStr>) -> PlanningError {
    PlanningError::Uninhabited(format_smolstr!("{}: {}", format_path(path), reason.into()))
}

fn fatal<T>(path: &[PathSegment<'_>], reason: impl Into<SmolStr>) -> PlanningResult<T> {
    Err(fatal_error(path, reason))
}

fn fatal_error(path: &[PathSegment<'_>], reason: impl Into<SmolStr>) -> PlanningError {
    PlanningError::Fatal(format_smolstr!("{}: {}", format_path(path), reason.into()))
}

fn allocation_planning_error(path: &[PathSegment<'_>], error: TryReserveError) -> PlanningError {
    PlanningError::Public(allocation_error(&format_path(path), error))
}

fn public_planning_error(error: PlanningError) -> Error {
    match error {
        PlanningError::Uninhabited(reason) | PlanningError::Fatal(reason) => {
            Error::InvalidDataType {
                kind: "DefaultValue",
                reason,
            }
        }
        PlanningError::Public(error) => error,
    }
}

fn allocation_error(path: &str, error: TryReserveError) -> Error {
    default_error(
        path,
        format_smolstr!("default allocation could not be reserved: {error}"),
    )
}

fn default_error(path: &str, reason: SmolStr) -> Error {
    schema_preflight_error("DefaultValue", path, reason)
}

fn schema_preflight_error(kind: &'static str, path: &str, reason: SmolStr) -> Error {
    Error::InvalidDataType {
        kind,
        reason: format_smolstr!("{path}: {reason}"),
    }
}

fn format_path(path: &[PathSegment<'_>]) -> String {
    let mut rendered = String::from("$");
    for segment in path {
        match segment {
            PathSegment::Field(name) => {
                push_field_name_path(&mut rendered, name);
            }
            PathSegment::Item => rendered.push_str("[]"),
            PathSegment::Union(type_id, name) => {
                rendered.push_str(".union[");
                rendered.push_str(&type_id.to_string());
                rendered.push(']');
                push_field_name_path(&mut rendered, name);
            }
            PathSegment::DictionaryValue => rendered.push_str(".dictionary_value"),
            PathSegment::RunEndValues => rendered.push_str(".run_end_values"),
        }
    }
    rendered
}
