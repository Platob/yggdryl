//! The aggregation vocabulary: [`AggFunc`] (the reduction) and [`Agg`] (a
//! reduction applied to a column, with an optional output alias). Pure metadata —
//! the [`DataFrame`](crate::DataFrame) executes it in
//! [`group_by`](crate::DataFrame::group_by) / [`resample`](crate::DataFrame::resample).

use std::fmt;

use crate::{DataType, PrimitiveType};

/// A reduction over a column's values within a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AggFunc {
    /// The number of rows in the group.
    Count,
    /// The sum of the (non-null) values.
    Sum,
    /// The smallest (non-null) value.
    Min,
    /// The largest (non-null) value.
    Max,
    /// The arithmetic mean of the (non-null) values.
    Mean,
}

impl AggFunc {
    /// The lowercase name (`count` / `sum` / `min` / `max` / `mean`).
    pub fn as_str(&self) -> &'static str {
        match self {
            AggFunc::Count => "count",
            AggFunc::Sum => "sum",
            AggFunc::Min => "min",
            AggFunc::Max => "max",
            AggFunc::Mean => "mean",
        }
    }
}

impl fmt::Display for AggFunc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An aggregation to compute: an [`AggFunc`] over a column (except
/// [`count`](AggFunc::Count), which needs none), with an optional output name.
///
/// ```
/// use yggdryl_saga::Agg;
///
/// assert_eq!(Agg::sum("px").output_name(), "px_sum");
/// assert_eq!(Agg::mean("px").alias("avg").output_name(), "avg");
/// assert_eq!(Agg::count().output_name(), "count");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Agg {
    func: AggFunc,
    column: Option<String>,
    alias: Option<String>,
}

impl Agg {
    /// Counts the rows in each group (no input column).
    pub fn count() -> Agg {
        Agg {
            func: AggFunc::Count,
            column: None,
            alias: None,
        }
    }

    /// Sums `column` within each group.
    pub fn sum(column: impl Into<String>) -> Agg {
        Agg::over(AggFunc::Sum, column)
    }

    /// The minimum of `column` within each group.
    pub fn min(column: impl Into<String>) -> Agg {
        Agg::over(AggFunc::Min, column)
    }

    /// The maximum of `column` within each group.
    pub fn max(column: impl Into<String>) -> Agg {
        Agg::over(AggFunc::Max, column)
    }

    /// The mean of `column` within each group.
    pub fn mean(column: impl Into<String>) -> Agg {
        Agg::over(AggFunc::Mean, column)
    }

    fn over(func: AggFunc, column: impl Into<String>) -> Agg {
        Agg {
            func,
            column: Some(column.into()),
            alias: None,
        }
    }

    /// Returns a copy with the output column name overridden.
    pub fn alias(mut self, name: impl Into<String>) -> Agg {
        self.alias = Some(name.into());
        self
    }

    /// The reduction.
    pub fn func(&self) -> AggFunc {
        self.func
    }

    /// The input column, if any (`None` for [`count`](AggFunc::Count)).
    pub fn column(&self) -> Option<&str> {
        self.column.as_deref()
    }

    /// The output column name: the alias if set, else `count` or `<column>_<func>`.
    pub fn output_name(&self) -> String {
        if let Some(alias) = &self.alias {
            return alias.clone();
        }
        match &self.column {
            Some(column) => format!("{column}_{}", self.func.as_str()),
            None => self.func.as_str().to_string(),
        }
    }

    /// The output [`DataType`]: `int64` for [`count`](AggFunc::Count), else
    /// `float64` (sums/means/extents are computed in floating point).
    pub fn output_type(&self) -> DataType {
        match self.func {
            AggFunc::Count => PrimitiveType::Int64.into(),
            _ => PrimitiveType::Float64.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_types() {
        assert_eq!(Agg::count().output_name(), "count");
        assert_eq!(
            Agg::count().output_type(),
            DataType::from(PrimitiveType::Int64)
        );
        assert_eq!(Agg::sum("px").output_name(), "px_sum");
        assert_eq!(Agg::mean("px").output_name(), "px_mean");
        assert_eq!(Agg::max("qty").alias("top").output_name(), "top");
        assert_eq!(
            Agg::sum("px").output_type(),
            DataType::from(PrimitiveType::Float64)
        );
    }

    #[test]
    fn accessors() {
        let a = Agg::min("px");
        assert_eq!(a.func(), AggFunc::Min);
        assert_eq!(a.column(), Some("px"));
        assert_eq!(Agg::count().column(), None);
    }
}
