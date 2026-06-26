//! Aggregation over a [`DataFrame`]: [`group_by`](DataFrame::group_by) on key
//! columns and [`resample`](DataFrame::resample) on a time column, each finished
//! with [`agg`](GroupBy::agg). Gated behind the `dataframe` feature.
//!
//! **Timeseries optimisations.** Both paths avoid hashing when the data is already
//! ordered: a single-key `group_by` over a **sorted** key, and every `resample`
//! (whose buckets are contiguous in a sorted time column), reduce each group in one
//! linear pass over contiguous row ranges — no hash map, no per-row index vectors.

use std::collections::HashMap;
use std::sync::Arc;

#[allow(unused_imports)]
use crate::log_event;
use crate::{Agg, AggFunc, DataFrame, Field, FrameError, Period, Schema};

use arrow_array::builder::{Float64Builder, Int64Builder};
use arrow_array::cast::AsArray;
use arrow_array::types::{
    Date32Type, Date64Type, Decimal128Type, Float32Type, Float64Type, Int16Type, Int32Type,
    Int64Type, Int8Type, TimestampMicrosecondType, TimestampMillisecondType,
    TimestampNanosecondType, TimestampSecondType, UInt16Type, UInt32Type, UInt64Type, UInt8Type,
};
use arrow_array::{ArrayRef, Int64Array, UInt32Array};
use arrow_schema::{DataType as ArrowType, TimeUnit as ArrowTimeUnit};

impl DataFrame {
    /// Starts a [`GroupBy`] over one or more key columns. Finish with
    /// [`agg`](GroupBy::agg).
    pub fn group_by<S: AsRef<str>>(&self, keys: &[S]) -> GroupBy {
        GroupBy {
            frame: self.clone(),
            keys: keys.iter().map(|k| k.as_ref().to_string()).collect(),
        }
    }

    /// Starts a [`Resample`] that buckets the `time` column into fixed `every`-wide
    /// windows. The time column must be a sorted, non-null `timestamp` / `date`.
    /// Finish with [`agg`](Resample::agg).
    pub fn resample(&self, time: impl Into<String>, every: Period) -> Resample {
        Resample {
            frame: self.clone(),
            time: time.into(),
            every,
        }
    }
}

/// A pending group-by: a [`DataFrame`] plus its key columns, awaiting
/// [`agg`](GroupBy::agg). One output row per distinct key combination; the key
/// columns are carried through with their original types.
pub struct GroupBy {
    frame: DataFrame,
    keys: Vec<String>,
}

impl GroupBy {
    /// Reduces each group with the given aggregations, returning a new frame of the
    /// key columns followed by one column per [`Agg`].
    pub fn agg(self, aggs: &[Agg]) -> Result<DataFrame, FrameError> {
        let batch = self.frame.record_batch();
        let schema = batch.schema();
        let key_indices = self
            .keys
            .iter()
            .map(|name| {
                schema
                    .index_of(name)
                    .map_err(|_| FrameError::ColumnNotFound(name.clone()))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let groups = build_groups(&self.frame, &key_indices)?;
        log_event!(debug, "GroupBy::agg {} groups", groups.len());

        // Key columns: take the first row of each group, preserving their types.
        let first_rows: UInt32Array = groups.iter().map(|g| g.first() as u32).collect();
        let mut fields = Vec::new();
        let mut columns = Vec::new();
        for &index in &key_indices {
            let array = arrow_select::take::take(batch.column(index), &first_rows, None)
                .map_err(|e| FrameError::Compute(e.to_string()))?;
            fields.push(Field::from_arrow(schema.field(index)));
            columns.push(array);
        }

        append_aggregations(&self.frame, &groups, aggs, &mut fields, &mut columns)?;
        DataFrame::new(Schema::new(fields), columns)
    }
}

/// A pending resample: a [`DataFrame`], its time column and the bucket width,
/// awaiting [`agg`](Resample::agg). One output row per time bucket, the bucket
/// start carried as the time column.
pub struct Resample {
    frame: DataFrame,
    time: String,
    every: Period,
}

impl Resample {
    /// Reduces each time bucket with the given aggregations, returning a new frame
    /// of the bucketed time column followed by one column per [`Agg`].
    pub fn agg(self, aggs: &[Agg]) -> Result<DataFrame, FrameError> {
        let batch = self.frame.record_batch();
        let schema = batch.schema();
        let time_index = schema
            .index_of(&self.time)
            .map_err(|_| FrameError::ColumnNotFound(self.time.clone()))?;
        let time_array = batch.column(time_index);

        let unit_nanos = time_unit_nanos(time_array.data_type()).ok_or_else(|| {
            FrameError::Compute(format!(
                "resample needs a timestamp/date column; '{}' is {:?}",
                self.time,
                time_array.data_type()
            ))
        })?;
        if self.every.nanos() % unit_nanos != 0 || self.every.nanos() / unit_nanos == 0 {
            return Err(FrameError::Compute(format!(
                "resample period {} is not a whole multiple of the '{}' column resolution",
                self.every, self.time
            )));
        }
        let period_ticks = self.every.nanos() / unit_nanos;

        // Single contiguous pass over the sorted time column.
        let rows = batch.num_rows();
        let mut buckets: Vec<i64> = Vec::new();
        let mut groups: Vec<GroupRows> = Vec::new();
        let mut start = 0usize;
        let mut current = 0i64;
        let mut previous = i64::MIN;
        for row in 0..rows {
            let tick = time_tick(time_array, row).ok_or_else(|| {
                FrameError::Compute(format!("resample column '{}' has a null value", self.time))
            })?;
            if tick < previous {
                return Err(FrameError::Compute(format!(
                    "resample requires the '{}' column sorted ascending",
                    self.time
                )));
            }
            previous = tick;
            let bucket = tick.div_euclid(period_ticks) * period_ticks;
            if row == start {
                current = bucket;
            } else if bucket != current {
                groups.push(GroupRows::Range(start, row - start));
                buckets.push(current);
                start = row;
                current = bucket;
            }
        }
        if rows > 0 {
            groups.push(GroupRows::Range(start, rows - start));
            buckets.push(current);
        }
        log_event!(debug, "Resample::agg {} buckets", groups.len());

        // The bucket-start column, typed like the time column.
        let bucket_ticks = Int64Array::from(buckets);
        let bucket_array = arrow_cast::cast(&bucket_ticks, time_array.data_type())
            .map_err(|e| FrameError::Compute(e.to_string()))?;
        let mut fields = vec![Field::from_arrow(schema.field(time_index))];
        let mut columns = vec![bucket_array];

        append_aggregations(&self.frame, &groups, aggs, &mut fields, &mut columns)?;
        DataFrame::new(Schema::new(fields), columns)
    }
}

// --- grouping -------------------------------------------------------------

/// The rows of one group: a contiguous `Range(start, len)` (the sorted fast path)
/// or scattered `Indices` (the hash path).
enum GroupRows {
    Range(usize, usize),
    Indices(Vec<usize>),
}

impl GroupRows {
    /// The first row of the group (used to fetch its key values).
    fn first(&self) -> usize {
        match self {
            GroupRows::Range(start, _) => *start,
            GroupRows::Indices(rows) => rows[0],
        }
    }

    /// The number of rows in the group.
    fn count(&self) -> usize {
        match self {
            GroupRows::Range(_, len) => *len,
            GroupRows::Indices(rows) => rows.len(),
        }
    }

    /// Visits each row index in the group.
    fn for_each(&self, mut visit: impl FnMut(usize)) {
        match self {
            GroupRows::Range(start, len) => (*start..*start + *len).for_each(visit),
            GroupRows::Indices(rows) => rows.iter().for_each(|&row| visit(row)),
        }
    }
}

/// A hashable key cell read from a key column.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Key {
    Null,
    Int(i64),
    Str(String),
    Bool(bool),
}

/// Builds the groups, taking the **sorted fast path** for a single, already-sorted
/// key (contiguous runs, no hashing) and falling back to a hash group otherwise.
fn build_groups(frame: &DataFrame, key_indices: &[usize]) -> Result<Vec<GroupRows>, FrameError> {
    let batch = frame.record_batch();
    if key_indices.len() == 1 {
        if let Some(runs) = sorted_runs(batch.column(key_indices[0]))? {
            log_event!(debug, "GroupBy sorted fast path: {} runs", runs.len());
            return Ok(runs
                .into_iter()
                .map(|(start, len)| GroupRows::Range(start, len))
                .collect());
        }
    }
    hash_groups(batch, key_indices)
}

/// Returns contiguous equal-key runs if `array` is non-decreasing, else `None`
/// (signalling the caller to hash instead).
fn sorted_runs(array: &ArrayRef) -> Result<Option<Vec<(usize, usize)>>, FrameError> {
    let rows = array.len();
    let mut keys = Vec::with_capacity(rows);
    for row in 0..rows {
        keys.push(key_at(array, row)?);
    }
    if keys.windows(2).any(|w| w[0] > w[1]) {
        return Ok(None);
    }
    let mut runs = Vec::new();
    let mut start = 0;
    for row in 1..rows {
        if keys[row] != keys[start] {
            runs.push((start, row - start));
            start = row;
        }
    }
    if rows > 0 {
        runs.push((start, rows - start));
    }
    Ok(Some(runs))
}

/// Groups rows by a composite key via a hash map, preserving first-seen order.
fn hash_groups(
    batch: &arrow_array::RecordBatch,
    key_indices: &[usize],
) -> Result<Vec<GroupRows>, FrameError> {
    let mut order: Vec<Vec<usize>> = Vec::new();
    let mut seen: HashMap<Vec<Key>, usize> = HashMap::new();
    for row in 0..batch.num_rows() {
        let key = key_indices
            .iter()
            .map(|&index| key_at(batch.column(index), row))
            .collect::<Result<Vec<_>, _>>()?;
        match seen.get(&key) {
            Some(&group) => order[group].push(row),
            None => {
                seen.insert(key, order.len());
                order.push(vec![row]);
            }
        }
    }
    Ok(order.into_iter().map(GroupRows::Indices).collect())
}

/// Reads a hashable key from a key column, or errors on an un-groupable type
/// (floating point, nested, …).
fn key_at(array: &ArrayRef, row: usize) -> Result<Key, FrameError> {
    if array.is_null(row) {
        return Ok(Key::Null);
    }
    let key = match array.data_type() {
        ArrowType::Boolean => Key::Bool(array.as_boolean().value(row)),
        ArrowType::Int8 => Key::Int(array.as_primitive::<Int8Type>().value(row) as i64),
        ArrowType::Int16 => Key::Int(array.as_primitive::<Int16Type>().value(row) as i64),
        ArrowType::Int32 => Key::Int(array.as_primitive::<Int32Type>().value(row) as i64),
        ArrowType::Int64 => Key::Int(array.as_primitive::<Int64Type>().value(row)),
        ArrowType::UInt8 => Key::Int(array.as_primitive::<UInt8Type>().value(row) as i64),
        ArrowType::UInt16 => Key::Int(array.as_primitive::<UInt16Type>().value(row) as i64),
        ArrowType::UInt32 => Key::Int(array.as_primitive::<UInt32Type>().value(row) as i64),
        ArrowType::UInt64 => Key::Int(array.as_primitive::<UInt64Type>().value(row) as i64),
        ArrowType::Utf8 => Key::Str(array.as_string::<i32>().value(row).to_string()),
        ArrowType::LargeUtf8 => Key::Str(array.as_string::<i64>().value(row).to_string()),
        ArrowType::Date32 | ArrowType::Date64 | ArrowType::Timestamp(_, _) => {
            Key::Int(time_tick(array, row).unwrap_or(0))
        }
        other => {
            return Err(FrameError::Compute(format!(
                "cannot group by a column of type {other:?}"
            )))
        }
    };
    Ok(key)
}

// --- aggregation ----------------------------------------------------------

/// Appends one output column per [`Agg`] to `fields` / `columns`.
fn append_aggregations(
    frame: &DataFrame,
    groups: &[GroupRows],
    aggs: &[Agg],
    fields: &mut Vec<Field>,
    columns: &mut Vec<ArrayRef>,
) -> Result<(), FrameError> {
    let batch = frame.record_batch();
    let schema = batch.schema();
    for agg in aggs {
        let array: ArrayRef = if agg.func() == AggFunc::Count {
            let mut builder = Int64Builder::with_capacity(groups.len());
            for group in groups {
                builder.append_value(group.count() as i64);
            }
            Arc::new(builder.finish())
        } else {
            let name = agg.column().ok_or_else(|| {
                FrameError::Compute(format!("aggregation {} needs a column", agg.func()))
            })?;
            let index = schema
                .index_of(name)
                .map_err(|_| FrameError::ColumnNotFound(name.to_string()))?;
            let column = batch.column(index);
            let mut builder = Float64Builder::with_capacity(groups.len());
            for group in groups {
                builder.append_option(reduce(column, group, agg.func()));
            }
            Arc::new(builder.finish())
        };
        fields.push(Field::new(agg.output_name(), agg.output_type(), true));
        columns.push(array);
    }
    Ok(())
}

/// Reduces one group of `column` with `func`, in a single pass over the (non-null)
/// values. Returns `None` when the group has no non-null value.
fn reduce(column: &ArrayRef, group: &GroupRows, func: AggFunc) -> Option<f64> {
    let mut count = 0u64;
    let mut sum = 0.0;
    let mut min = f64::INFINITY;
    let mut max = f64::NEG_INFINITY;
    group.for_each(|row| {
        if let Some(value) = f64_at(column, row) {
            count += 1;
            sum += value;
            min = min.min(value);
            max = max.max(value);
        }
    });
    if count == 0 {
        return None;
    }
    Some(match func {
        AggFunc::Sum => sum,
        AggFunc::Mean => sum / count as f64,
        AggFunc::Min => min,
        AggFunc::Max => max,
        AggFunc::Count => count as f64, // not reached (count is integer-typed)
    })
}

/// Reads a numeric/temporal/decimal cell as `f64`, or `None` for null / an
/// unsupported type.
fn f64_at(array: &ArrayRef, row: usize) -> Option<f64> {
    if array.is_null(row) {
        return None;
    }
    let value = match array.data_type() {
        ArrowType::Int8 => array.as_primitive::<Int8Type>().value(row) as f64,
        ArrowType::Int16 => array.as_primitive::<Int16Type>().value(row) as f64,
        ArrowType::Int32 => array.as_primitive::<Int32Type>().value(row) as f64,
        ArrowType::Int64 => array.as_primitive::<Int64Type>().value(row) as f64,
        ArrowType::UInt8 => array.as_primitive::<UInt8Type>().value(row) as f64,
        ArrowType::UInt16 => array.as_primitive::<UInt16Type>().value(row) as f64,
        ArrowType::UInt32 => array.as_primitive::<UInt32Type>().value(row) as f64,
        ArrowType::UInt64 => array.as_primitive::<UInt64Type>().value(row) as f64,
        ArrowType::Float32 => array.as_primitive::<Float32Type>().value(row) as f64,
        ArrowType::Float64 => array.as_primitive::<Float64Type>().value(row),
        ArrowType::Date32 | ArrowType::Date64 | ArrowType::Timestamp(_, _) => {
            time_tick(array, row)? as f64
        }
        ArrowType::Decimal128(_, scale) => {
            array.as_primitive::<Decimal128Type>().value(row) as f64 / 10f64.powi(*scale as i32)
        }
        _ => return None,
    };
    Some(value)
}

/// Reads an integer tick from a timestamp/date column (`None` for null or a
/// non-temporal type).
fn time_tick(array: &ArrayRef, row: usize) -> Option<i64> {
    if array.is_null(row) {
        return None;
    }
    let tick = match array.data_type() {
        ArrowType::Date32 => array.as_primitive::<Date32Type>().value(row) as i64,
        ArrowType::Date64 => array.as_primitive::<Date64Type>().value(row),
        ArrowType::Timestamp(unit, _) => match unit {
            ArrowTimeUnit::Second => array.as_primitive::<TimestampSecondType>().value(row),
            ArrowTimeUnit::Millisecond => {
                array.as_primitive::<TimestampMillisecondType>().value(row)
            }
            ArrowTimeUnit::Microsecond => {
                array.as_primitive::<TimestampMicrosecondType>().value(row)
            }
            ArrowTimeUnit::Nanosecond => array.as_primitive::<TimestampNanosecondType>().value(row),
        },
        _ => return None,
    };
    Some(tick)
}

/// The nanoseconds-per-tick of a temporal column (for resample bucketing), or
/// `None` if not a timestamp/date.
fn time_unit_nanos(dt: &ArrowType) -> Option<i64> {
    match dt {
        ArrowType::Timestamp(unit, _) => Some(match unit {
            ArrowTimeUnit::Second => 1_000_000_000,
            ArrowTimeUnit::Millisecond => 1_000_000,
            ArrowTimeUnit::Microsecond => 1_000,
            ArrowTimeUnit::Nanosecond => 1,
        }),
        ArrowType::Date32 => Some(86_400_000_000_000),
        ArrowType::Date64 => Some(1_000_000),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Frame, FrameHandle, Scalar};
    use arrow_array::{Float64Array, Int64Array, StringArray, TimestampNanosecondArray};

    const DAY: i64 = 86_400 * 1_000_000_000;

    fn trades() -> DataFrame {
        // Sorted-by-time trades over two days, two symbols.
        let schema = Schema::from_str(
            "ts: timestamp(ns, UTC) not null, symbol: utf8 not null, px: float64, qty: int64",
        )
        .unwrap();
        let ts = TimestampNanosecondArray::from(vec![
            0,
            3_600 * 1_000_000_000, // +1h
            DAY,                   // day 2
            DAY + 3_600 * 1_000_000_000,
            DAY + 7_200 * 1_000_000_000,
        ])
        .with_timezone("UTC");
        DataFrame::new(
            schema,
            vec![
                Arc::new(ts),
                Arc::new(StringArray::from(vec![
                    "AAPL", "MSFT", "AAPL", "AAPL", "MSFT",
                ])),
                Arc::new(Float64Array::from(vec![10.0, 20.0, 12.0, 14.0, 22.0])),
                Arc::new(Int64Array::from(vec![1, 2, 3, 4, 5])),
            ],
        )
        .unwrap()
    }

    #[test]
    fn group_by_key_with_aggregations() {
        let out = trades()
            .group_by(&["symbol"])
            .agg(&[
                Agg::count(),
                Agg::sum("qty"),
                Agg::mean("px").alias("avg_px"),
            ])
            .unwrap();
        assert_eq!(
            out.schema().unwrap().names(),
            ["symbol", "count", "qty_sum", "avg_px"]
        );
        assert_eq!(out.height(), Some(2)); // AAPL, MSFT

        // AAPL appears first (rows 0,2,3): count 3, qty 1+3+4=8, avg px (10+12+14)/3=12.
        let symbol = out.column("symbol").unwrap();
        let names = symbol.array().as_string::<i32>();
        assert_eq!(names.value(0), "AAPL");
        let count = out.column("count").unwrap();
        assert_eq!(count.array().as_primitive::<Int64Type>().value(0), 3);
        let avg = out.column("avg_px").unwrap();
        assert!((avg.array().as_primitive::<Float64Type>().value(0) - 12.0).abs() < 1e-9);
    }

    #[test]
    fn resample_buckets_by_day() {
        let out = trades()
            .resample("ts", Period::from_str("1d").unwrap())
            .agg(&[Agg::count(), Agg::max("px")])
            .unwrap();
        assert_eq!(out.schema().unwrap().names(), ["ts", "count", "px_max"]);
        assert_eq!(out.height(), Some(2)); // two days

        // Day 1: rows 0,1 → count 2, max px 20; day 2: rows 2,3,4 → count 3, max 22.
        let count = out.column("count").unwrap();
        let counts = count.array().as_primitive::<Int64Type>();
        assert_eq!(counts.value(0), 2);
        assert_eq!(counts.value(1), 3);
        let pxmax = out.column("px_max").unwrap();
        let maxes = pxmax.array().as_primitive::<Float64Type>();
        assert!((maxes.value(0) - 20.0).abs() < 1e-9);
        assert!((maxes.value(1) - 22.0).abs() < 1e-9);
        // The bucket column keeps the timestamp type, floored to the day boundary.
        let bucket = out.column("ts").unwrap();
        assert_eq!(
            bucket
                .array()
                .as_primitive::<TimestampNanosecondType>()
                .value(1),
            DAY
        );
    }

    #[test]
    fn resample_then_filter_composes() {
        use crate::Predicate;
        // resample returns a DataFrame, so the Frame surface keeps working.
        let out = trades()
            .resample("ts", Period::from_str("1h").unwrap())
            .agg(&[Agg::sum("qty")])
            .unwrap()
            .filter(Predicate::gt("qty_sum", Scalar::any("0")))
            .unwrap();
        assert!(out.height().unwrap() <= 5);
    }

    #[test]
    fn resample_rejects_unsorted_time() {
        let schema = Schema::from_str("ts: timestamp(ns, UTC) not null, px: float64").unwrap();
        let ts = TimestampNanosecondArray::from(vec![DAY, 0]).with_timezone("UTC");
        let df = DataFrame::new(
            schema,
            vec![Arc::new(ts), Arc::new(Float64Array::from(vec![1.0, 2.0]))],
        )
        .unwrap();
        assert!(matches!(
            df.resample("ts", Period::from_str("1h").unwrap())
                .agg(&[Agg::count()]),
            Err(FrameError::Compute(_))
        ));
    }
}
