//! The `yggdryl.temporal` namespace's **columnar** temporal types — one nullable column per
//! temporal concept+width (`Date32Serie` / `Date64Serie`, `Time32Serie` / `Time64Serie`,
//! `Ts32Serie` / `Ts64Serie` / `Ts96Serie`, `Duration32Serie` / `Duration64Serie`), mirroring
//! `yggdryl_core::io::fixed`'s `TemporalSerie<B>`. They sit beside the temporal **value** classes
//! (`Date32` … `Duration64`) in the same JS namespace.
//!
//! A column fixes one `(unit, tz)` for every element (Arrow's model) and stores only the raw
//! physical counts. Resolutions (**time units**) and **timezones** cross as strings — `"ns"` /
//! `"ms"` / `"s"` and `"UTC"` / `"Europe/Paris"` / `"+02:00"` / `""` (naive) — matching the value
//! classes. A **cell** crosses two ways: as the value's ISO-8601 `string` (`get(index)`), and as
//! its raw epoch / clock / span **count** as a `bigint` (`getEpoch(index)`, an `i128` so the wide
//! `ts96` fits). `getScalar(index)` hands back the element as the matching temporal **value** class
//! (`null` for a null slot).
//!
//! There is no Arrow bridge on the Node side (apache-arrow JS has no standard C Data Interface
//! consumer); the codec (`serializeBytes` / `deserializeBytes`) plus the structural surface are the
//! interop.

use napi::bindgen_prelude::{BigInt, Buffer, Date};
use napi::{Env, JsUnknown};
use napi_derive::napi;

use yggdryl_core::io::fixed::temporal as core;
use yggdryl_core::io::fixed::temporal::TemporalNative as _;
use yggdryl_core::io::DataTypeId;

/// Maps any core error to a thrown JS `Error` (its guided text passes through unchanged).
fn to_error(error: impl std::fmt::Display) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// Parses a time-unit abbreviation / name (`"ns"`, `"millisecond"`), guided error on an unknown one.
fn parse_unit(text: &str) -> napi::Result<core::TimeUnit> {
    core::TimeUnit::parse(text).ok_or_else(|| to_error(format!("unknown time unit: {text:?}")))
}

/// Parses a timezone (`"UTC"`, `"Europe/Paris"`, `"+02:00"`, `""` naive), guided error otherwise.
fn parse_tz(text: &str) -> napi::Result<core::Tz> {
    core::Tz::parse(text).ok_or_else(|| to_error(format!("unknown timezone: {text:?}")))
}

/// Reads an epoch/clock/span **count** out of a JS `bigint`, erroring if it exceeds the 128-bit range.
fn count_from_bigint(value: BigInt) -> napi::Result<i128> {
    let (count, lossless) = value.get_i128();
    if !lossless {
        return Err(to_error("epoch value exceeds the 128-bit range"));
    }
    Ok(count)
}

/// Generates the columnar `Serie` napi wrapper for one temporal concept+width.
macro_rules! napi_temporal_col {
    ($Serie:ident, $Value:path, $Native:ty, $CoreSerie:ty, $id:ident, $lit:literal) => {
        #[doc = concat!("A nullable column of `", $lit, "` values at one `(unit, tz)`.")]
        #[napi(namespace = "temporal")]
        pub struct $Serie {
            pub(crate) inner: $CoreSerie,
        }

        #[napi(namespace = "temporal")]
        impl $Serie {
            /// A column at `(unit, tz)` from an array of ISO-8601-string-or-`null` (empty by
            /// default). `tz` defaults to naive (`""`); it is forced to naive for the zone-less
            /// types. Each present value is re-expressed at the column's unit.
            #[napi(constructor)]
            pub fn new(
                unit: String,
                tz: Option<String>,
                values: Option<Vec<Option<String>>>,
            ) -> napi::Result<Self> {
                let unit = parse_unit(&unit)?;
                let tz = parse_tz(&tz.unwrap_or_default())?;
                match values {
                    None => Ok(Self {
                        inner: <$CoreSerie>::new(unit, tz),
                    }),
                    Some(values) => {
                        let mut options = Vec::with_capacity(values.len());
                        for value in values {
                            options.push(match value {
                                Some(text) => Some(text.parse::<$Native>().map_err(to_error)?),
                                None => None,
                            });
                        }
                        <$CoreSerie>::from_options(unit, tz, &options)
                            .map(|inner| Self { inner })
                            .map_err(to_error)
                    }
                }
            }

            /// A non-null column at `(unit, tz)` from an array of raw epoch / clock / span **counts**
            /// (`bigint`, each in `unit`). The counterpart of [`getEpoch`](Self::get_epoch).
            #[napi(factory)]
            pub fn from_epochs(
                unit: String,
                tz: Option<String>,
                epochs: Vec<BigInt>,
            ) -> napi::Result<Self> {
                let unit = parse_unit(&unit)?;
                let tz = parse_tz(&tz.unwrap_or_default())?;
                let mut values = Vec::with_capacity(epochs.len());
                for epoch in epochs {
                    values.push(
                        <$Native>::from_count(count_from_bigint(epoch)?, unit, tz)
                            .map_err(to_error)?,
                    );
                }
                <$CoreSerie>::from_values(unit, tz, &values)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }

            /// A column at `(unit, tz)` from an array of [`getScalar`](Self::get_scalar)-shaped
            /// **value** wrappers — a `null` / `undefined` item is a null. Each present value is
            /// re-expressed at the column's unit. Round-trips a column through its own scalars.
            #[napi(factory)]
            pub fn from_scalars(
                unit: String,
                tz: Option<String>,
                scalars: Vec<Option<&$Value>>,
            ) -> napi::Result<Self> {
                let unit = parse_unit(&unit)?;
                let tz = parse_tz(&tz.unwrap_or_default())?;
                let scalars: Vec<core::TemporalScalar<_>> = scalars
                    .into_iter()
                    .map(|slot| match slot {
                        Some(value) => core::TemporalScalar::of(value.inner),
                        None => core::TemporalScalar::null(unit, tz),
                    })
                    .collect();
                <$CoreSerie>::from_scalars(unit, tz, &scalars)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }

            /// Appends one element (an ISO-8601 string, or `null` for a null).
            #[napi]
            pub fn push(&mut self, value: Option<String>) -> napi::Result<()> {
                let native = match value {
                    Some(text) => Some(text.parse::<$Native>().map_err(to_error)?),
                    None => None,
                };
                self.inner.push(native).map_err(to_error)
            }

            /// The value at `index` as an ISO-8601 string, or `null` if null or out of range.
            #[napi]
            pub fn get(&self, index: u32) -> Option<String> {
                self.inner
                    .get(index as usize)
                    .map(|value| value.to_string())
            }

            /// The raw epoch / clock / span **count** at `index` as a `bigint` (in the column's
            /// unit), or `null` if null or out of range. The `i128` carries the wide `ts96` losslessly.
            #[napi]
            pub fn get_epoch(&self, index: u32) -> Option<i128> {
                self.inner.get_count(index as usize)
            }

            /// Element `index` as the matching temporal **value** class (carrying the column's
            /// `(unit, tz)`), or `null` if the element is null or out of range.
            #[napi]
            pub fn get_scalar(&self, index: u32) -> Option<$Value> {
                // A local alias lets the `$Value:path` metavariable be used in struct-literal
                // position (a `:path` fragment cannot be immediately followed by `{`).
                type ValueWrapper = $Value;
                self.inner
                    .get(index as usize)
                    .map(|inner| ValueWrapper { inner })
            }

            /// Overwrites element `index` (an ISO-8601 string, or `null`); throws out of range or if
            /// the value does not fit the column's unit.
            #[napi]
            pub fn set(&mut self, index: u32, value: Option<String>) -> napi::Result<()> {
                let native = match value {
                    Some(text) => Some(text.parse::<$Native>().map_err(to_error)?),
                    None => None,
                };
                self.inner.set(index as usize, native).map_err(to_error)
            }

            /// The number of elements.
            #[napi(getter)]
            pub fn length(&self) -> u32 {
                self.inner.len() as u32
            }

            /// The number of null elements.
            #[napi(getter)]
            pub fn null_count(&self) -> u32 {
                self.inner.null_count() as u32
            }

            /// Whether the column carries any nulls.
            #[napi(getter)]
            pub fn has_nulls(&self) -> bool {
                self.inner.has_nulls()
            }

            /// Whether the column is empty.
            #[napi]
            pub fn is_empty(&self) -> bool {
                self.inner.is_empty()
            }

            /// The column resolution as a string (`"ns"`, `"ms"`, `"s"`, `"d"`, …).
            #[napi(getter)]
            pub fn unit(&self) -> String {
                self.inner.unit().abbreviation().to_string()
            }

            /// The column timezone name (empty for naive / the zone-less types).
            #[napi(getter)]
            pub fn timezone(&self) -> String {
                self.inner.timezone().name()
            }

            /// This column's erased [`DataType`](crate::types::DataType) (the concept+width; the
            /// `(unit, tz)` ride the column itself and [`toField`](Self::to_field)'s metadata).
            #[napi]
            pub fn data_type(&self) -> crate::types::DataType {
                crate::types::DataType::of(DataTypeId::$id)
            }

            /// A named [`Field`](crate::types::Field) for this column (nullability inferred from
            /// whether it holds nulls); the `(unit, tz)` ride the field's metadata.
            #[napi]
            pub fn to_field(&self, name: String) -> crate::types::Field {
                crate::types::Field {
                    inner: self.inner.to_field(&name).erase(),
                }
            }

            /// The elements as an array of ISO-8601-string-or-`null`, in order.
            #[napi]
            pub fn to_options(&self) -> Vec<Option<String>> {
                (0..self.inner.len())
                    .map(|index| self.inner.get(index).map(|value| value.to_string()))
                    .collect()
            }

            /// The column's canonical bytes (`[len][unit tag][tz name][flags][validity?][counts]`).
            #[napi]
            pub fn serialize_bytes(&self) -> Buffer {
                self.inner.serialize_bytes().into()
            }

            /// Reconstructs a column from [`serializeBytes`](Self::serialize_bytes).
            #[napi(factory)]
            pub fn deserialize_bytes(bytes: Buffer) -> napi::Result<Self> {
                <$CoreSerie>::deserialize_bytes(&bytes)
                    .map(|inner| Self { inner })
                    .map_err(to_error)
            }

            /// Structural equality (same `(unit, tz)`, length, and per-index present-or-null count).
            #[napi]
            pub fn equals(&self, other: &$Serie) -> bool {
                self.inner == other.inner
            }

            /// An explicit copy.
            #[napi]
            pub fn copy(&self) -> Self {
                Self {
                    inner: self.inner.clone(),
                }
            }

            #[napi(js_name = "toString")]
            pub fn text(&self) -> String {
                format!(
                    "{}(len={}, unit={}, tz={:?}, nullCount={})",
                    stringify!($Serie),
                    self.inner.len(),
                    self.inner.unit().abbreviation(),
                    self.inner.timezone().name(),
                    self.inner.null_count()
                )
            }

            // ---- Phase 8: reshape + row-selection (no arithmetic on a temporal column) -------

            /// A same-`(unit, tz)` column of the rows `mask` keeps (`true` keeps row `i`); throws if
            /// `mask`'s length is not this column's length.
            #[napi]
            pub fn filter(&self, mask: Vec<bool>) -> napi::Result<Self> {
                Ok(Self {
                    inner: crate::ops::filter_into(&self.inner, mask)?,
                })
            }

            /// A same-column with every null replaced by `value` (a JS `null` / `undefined` is a
            /// no-op clone). A temporal has no native JS scalar form, so a real fill value is passed
            /// as a length-1 `Serie` **carrier** of the same `(unit, tz)` — its `value(0)` is used,
            /// and a unit / tz mismatch (or a plain JS native) is a guided error.
            #[napi]
            pub fn fill_null(&self, env: Env, value: JsUnknown) -> napi::Result<Self> {
                Ok(Self {
                    inner: crate::ops::fill_null_into(env, &self.inner, value)?,
                })
            }

            /// This column as a one-field [`StructSerie`](crate::nested::StructSerie) named `name`
            /// (default `"value"`).
            #[napi]
            pub fn to_struct(&self, name: Option<String>) -> crate::nested::StructSerie {
                crate::ops::to_struct_wrapper(&self.inner, name)
            }

            /// This column as a list-of-singletons [`ListSerie`](crate::nested::ListSerie).
            #[napi]
            pub fn to_list(&self) -> crate::nested::ListSerie {
                crate::ops::to_list_wrapper(&self.inner)
            }

            /// This column reshaped toward a map, as its `serializeBytes()` frame (unchanged for a
            /// temporal column; reconstruct with the resulting class's `deserializeBytes`).
            #[napi]
            pub fn to_map(&self) -> napi::Result<Buffer> {
                crate::ops::to_map_frame(&self.inner)
            }

            // ---- Phase 9: random-access set + slice get --------------------------------------

            /// Replaces the nested child column at `index` with the `Serie` `child` — a **leaf** column
            /// is not nested, so the core surfaces a guided error (set a leaf cell with `set` instead).
            /// `child` must be a `Serie` wrapper.
            #[napi]
            pub fn set_child_at(
                &mut self,
                env: Env,
                index: u32,
                child: JsUnknown,
            ) -> napi::Result<()> {
                crate::ops::set_child_at_into(env, &mut self.inner, index, child)
            }

            /// Adds or replaces the nested child column named `name` with the `Serie` `child` — a leaf
            /// column is not nested (guided error). `child` must be a `Serie` wrapper.
            #[napi]
            pub fn set_child_by(
                &mut self,
                env: Env,
                name: String,
                child: JsUnknown,
            ) -> napi::Result<()> {
                crate::ops::set_child_by_into(env, &mut self.inner, name, child)
            }

            /// Overwrites the length-preserving range `[offset, offset + other.length)` with `other`'s
            /// cells (each re-expressed at this column's `(unit, tz)`). Throws on an out-of-range offset
            /// or an incompatible source cell. `other` must be a `Serie` wrapper.
            #[napi]
            pub fn set_slice(
                &mut self,
                env: Env,
                offset: u32,
                other: JsUnknown,
            ) -> napi::Result<()> {
                crate::ops::set_slice_into(env, &mut self.inner, offset, other)
            }

            /// A fresh same-`(unit, tz)` column over rows `[start, start + length)` (the range clamped
            /// to the column, never throws). The Node named mirror of Python's `serie[start:stop]` slice.
            #[napi]
            pub fn slice(&self, start: u32, length: u32) -> Self {
                Self {
                    inner: crate::ops::slice_into(&self.inner, start, length),
                }
            }
        }
    };
}

napi_temporal_col!(
    Date32Serie,
    crate::temporal::Date32,
    core::Date32,
    yggdryl_core::io::fixed::Date32Serie,
    Date32,
    "date32"
);
napi_temporal_col!(
    Date64Serie,
    crate::temporal::Date64,
    core::Date64,
    yggdryl_core::io::fixed::Date64Serie,
    Date64,
    "date64"
);
napi_temporal_col!(
    Time32Serie,
    crate::temporal::Time32,
    core::Time32,
    yggdryl_core::io::fixed::Time32Serie,
    Time32,
    "time32"
);
napi_temporal_col!(
    Time64Serie,
    crate::temporal::Time64,
    core::Time64,
    yggdryl_core::io::fixed::Time64Serie,
    Time64,
    "time64"
);
napi_temporal_col!(
    Ts32Serie,
    crate::temporal::Ts32,
    core::Ts32,
    yggdryl_core::io::fixed::Ts32Serie,
    Ts32,
    "ts32"
);
napi_temporal_col!(
    Ts64Serie,
    crate::temporal::Ts64,
    core::Ts64,
    yggdryl_core::io::fixed::Ts64Serie,
    Ts64,
    "ts64"
);
napi_temporal_col!(
    Ts96Serie,
    crate::temporal::Ts96,
    core::Ts96,
    yggdryl_core::io::fixed::Ts96Serie,
    Ts96,
    "ts96"
);
napi_temporal_col!(
    Duration32Serie,
    crate::temporal::Duration32,
    core::Duration32,
    yggdryl_core::io::fixed::Duration32Serie,
    Duration32,
    "duration32"
);
napi_temporal_col!(
    Duration64Serie,
    crate::temporal::Duration64,
    core::Duration64,
    yggdryl_core::io::fixed::Duration64Serie,
    Duration64,
    "duration64"
);

// ---- JS Date bridge (a JS Date is milliseconds since the epoch, UTC) -------------------
//
// The timestamp columns get a native `fromDates` factory mirroring the `Ts64.fromEpochMillis`
// value bridge. JS has no native date-only / time-only / span type, so `Date32*` / `Time*` /
// `Duration*` columns keep their ISO-string constructor and `fromEpochs` factory instead.

/// A JS `Date` (a millisecond instant since the epoch, UTC) re-expressed at `(unit, tz)` as a
/// [`Ts64`](core::Ts64) — the millis bridge shared by the timestamp-column `fromDates` factories
/// (mirroring the `Ts64.fromEpochMillis` value constructor). Truncates to a coarser `unit`.
fn date_to_ts64(date: Date, unit: core::TimeUnit, tz: core::Tz) -> napi::Result<core::Ts64> {
    let millis = date.value_of()?;
    core::Ts64::from_epoch(millis as i128, core::TimeUnit::Millisecond, tz)
        .map_err(to_error)?
        .to_unit(unit)
        .map_err(to_error)
}

#[napi(namespace = "temporal")]
impl Ts32Serie {
    /// A column at `(unit, tz)` from JS `Date`s — each a millisecond instant, re-expressed at the
    /// column's `unit` (a `null` element is a null). Mirrors the `Ts64.fromEpochMillis` bridge.
    #[napi(factory)]
    pub fn from_dates(
        unit: String,
        tz: Option<String>,
        values: Vec<Option<Date>>,
    ) -> napi::Result<Self> {
        let unit = parse_unit(&unit)?;
        let tz = parse_tz(&tz.unwrap_or_default())?;
        let mut options = Vec::with_capacity(values.len());
        for value in values {
            options.push(match value {
                Some(date) => Some(date_to_ts64(date, unit, tz)?.to_ts32().map_err(to_error)?),
                None => None,
            });
        }
        yggdryl_core::io::fixed::Ts32Serie::from_options(unit, tz, &options)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }
}

#[napi(namespace = "temporal")]
impl Ts64Serie {
    /// A column at `(unit, tz)` from JS `Date`s — each a millisecond instant, re-expressed at the
    /// column's `unit` (a `null` element is a null). Mirrors the `Ts64.fromEpochMillis` bridge.
    #[napi(factory)]
    pub fn from_dates(
        unit: String,
        tz: Option<String>,
        values: Vec<Option<Date>>,
    ) -> napi::Result<Self> {
        let unit = parse_unit(&unit)?;
        let tz = parse_tz(&tz.unwrap_or_default())?;
        let mut options = Vec::with_capacity(values.len());
        for value in values {
            options.push(match value {
                Some(date) => Some(date_to_ts64(date, unit, tz)?),
                None => None,
            });
        }
        yggdryl_core::io::fixed::Ts64Serie::from_options(unit, tz, &options)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }
}

#[napi(namespace = "temporal")]
impl Ts96Serie {
    /// A column at `(unit, tz)` from JS `Date`s — each a millisecond instant, re-expressed at the
    /// column's `unit` (a `null` element is a null). Mirrors the `Ts64.fromEpochMillis` bridge.
    #[napi(factory)]
    pub fn from_dates(
        unit: String,
        tz: Option<String>,
        values: Vec<Option<Date>>,
    ) -> napi::Result<Self> {
        let unit = parse_unit(&unit)?;
        let tz = parse_tz(&tz.unwrap_or_default())?;
        let mut options = Vec::with_capacity(values.len());
        for value in values {
            options.push(match value {
                Some(date) => Some(date_to_ts64(date, unit, tz)?.to_ts96()),
                None => None,
            });
        }
        yggdryl_core::io::fixed::Ts96Serie::from_options(unit, tz, &options)
            .map(|inner| Self { inner })
            .map_err(to_error)
    }
}
