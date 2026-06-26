//! The [`Frame`] trait — the shared behaviour of a tabular frame, whatever its
//! backing. One contract covers an **eager** frame (rows already in memory) and a
//! **lazy** frame (a query plan yet to run): the same `select` / `filter` /
//! column-access surface, so callers compose pipelines without caring which they
//! hold.

use std::fmt;

use crate::{Column, ColumnError, ExpressionError, Predicate, Schema};

/// Error returned by [`Frame`] operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// A referenced column does not exist in the frame's schema.
    ColumnNotFound(String),
    /// Two frames could not be combined because their schemas differ.
    SchemaMismatch(String),
    /// A column operation failed.
    Column(ColumnError),
    /// Executing the frame (materialising a lazy plan) failed.
    Compute(String),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::ColumnNotFound(name) => write!(f, "column '{name}' not found"),
            FrameError::SchemaMismatch(detail) => write!(f, "schema mismatch: {detail}"),
            FrameError::Column(err) => write!(f, "column error: {err}"),
            FrameError::Compute(detail) => write!(f, "compute error: {detail}"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<ColumnError> for FrameError {
    fn from(err: ColumnError) -> FrameError {
        FrameError::Column(err)
    }
}

impl From<ExpressionError> for FrameError {
    fn from(err: ExpressionError) -> FrameError {
        match err {
            ExpressionError::ColumnNotFound(name) => FrameError::ColumnNotFound(name),
            other => FrameError::Compute(other.to_string()),
        }
    }
}

/// The **object-safe base** of [`Frame`]: the part a held [`Column`] can reach
/// through a `dyn` reference without naming the frame's concrete type.
///
/// `Frame` itself is not object-safe (it has an associated `Column` type and
/// generic methods), so it cannot be used as `dyn Frame`. `FrameHandle` is the
/// object-safe slice of it — every [`Frame`] **is** a `FrameHandle`
/// (`Frame: FrameHandle`) — exposing what a column needs from its holder without
/// knowing whether that holder is eager or lazy. See
/// [`Column::frame`](crate::Column::frame).
pub trait FrameHandle {
    /// The frame's [`Schema`] (column names, types and nullability). Resolvable
    /// without executing a lazy plan.
    fn schema(&self) -> Result<Schema, FrameError>;
}

/// A tabular frame: an ordered set of named [`Column`]s sharing a [`Schema`].
///
/// The trait unifies the two backings the engine will grow:
///
/// - an **eager** frame holds its rows, so [`height`](Frame::height) is known and
///   each transformation runs immediately;
/// - a **lazy** frame holds a plan, so [`height`](Frame::height) is usually `None`
///   and each transformation extends the plan, materialising only on demand.
///
/// The [`schema`](FrameHandle::schema) (inherited from [`FrameHandle`]) is always
/// resolvable (a lazy frame tracks it without executing), which is why the
/// structural defaults below ([`width`](Frame::width),
/// [`column_names`](Frame::column_names), [`drop`](Frame::drop), …) are total.
/// Transformations consume `self` and return the same frame kind, so pipelines
/// compose identically across backings.
///
/// ```
/// use yggdryl_saga::{Frame, FrameError, Schema};
///
/// fn first_two<F: Frame>(frame: F) -> Result<F, FrameError> {
///     // Works for any frame backing — eager or lazy.
///     frame.select(&["id", "px"])?.head(2)
/// }
/// ```
pub trait Frame: FrameHandle + Sized {
    /// The associated [`Column`] type this frame yields.
    type Column: Column;

    /// Accesses a column by name. For a lazy frame this returns a lazy column; for
    /// an eager one, a materialized column.
    fn column(&self, name: &str) -> Result<Self::Column, FrameError>;

    /// The column names, in order.
    fn column_names(&self) -> Result<Vec<String>, FrameError> {
        Ok(self
            .schema()?
            .names()
            .into_iter()
            .map(String::from)
            .collect())
    }

    /// The number of columns.
    fn width(&self) -> Result<usize, FrameError> {
        Ok(self.schema()?.len())
    }

    /// The number of rows, if known without executing (eager: `Some`; lazy:
    /// usually `None`).
    fn height(&self) -> Option<usize> {
        None
    }

    /// Whether the frame is known to be empty (`None` when the height is unknown).
    fn is_empty(&self) -> Option<bool> {
        self.height().map(|h| h == 0)
    }

    /// Whether a column of the given name exists.
    fn contains_column(&self, name: &str) -> Result<bool, FrameError> {
        Ok(self.schema()?.index_of(name).is_some())
    }

    /// Projects the named columns, in the given order.
    fn select<S: AsRef<str>>(self, columns: &[S]) -> Result<Self, FrameError>;

    /// Drops the named columns, keeping the rest in their current order. Defaults
    /// to a [`select`](Frame::select) of the complement.
    fn drop<S: AsRef<str>>(self, columns: &[S]) -> Result<Self, FrameError> {
        let drop: Vec<&str> = columns.iter().map(AsRef::as_ref).collect();
        let keep: Vec<String> = self
            .schema()?
            .names()
            .into_iter()
            .filter(|name| !drop.contains(name))
            .map(String::from)
            .collect();
        self.select(&keep)
    }

    /// Keeps only the rows matching `predicate`.
    ///
    /// The implementation **type-optimises** the predicate against its
    /// [`Schema`] first — [`Predicate::optimize`] casts each literal to its
    /// column's type (e.g. a string ISO date → `timestamp`) — then applies it,
    /// **pushing it down** into storage where it can (a `ParquetFrame` skips row
    /// groups, a `CsvFrame` filters on scan). Use [`optimize_predicate`] to do that
    /// first step.
    ///
    /// [`optimize_predicate`]: Frame::optimize_predicate
    fn filter(self, predicate: Predicate) -> Result<Self, FrameError>;

    /// Type-optimises `predicate` against this frame's [`Schema`] (casting each
    /// literal to its column's type), returning the typed predicate. The helper a
    /// [`filter`](Frame::filter) implementation calls before applying or pushing the
    /// predicate down.
    fn optimize_predicate(&self, predicate: Predicate) -> Result<Predicate, FrameError> {
        Ok(predicate.optimize(&self.schema()?)?)
    }

    /// Keeps at most the first `n` rows.
    fn limit(self, n: usize) -> Result<Self, FrameError>;

    /// Keeps the first `n` rows (an alias for [`limit`](Frame::limit)).
    fn head(self, n: usize) -> Result<Self, FrameError> {
        self.limit(n)
    }

    /// Keeps at most the last `n` rows.
    fn tail(self, n: usize) -> Result<Self, FrameError>;

    /// Keeps `length` rows starting at `offset`.
    fn slice(self, offset: usize, length: usize) -> Result<Self, FrameError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Column, ColumnError, DataType, Field, PrimitiveType, Scalar};

    /// A trivial materialized column for the mock frame.
    struct TestColumn {
        field: Field,
        len: usize,
    }
    impl Column for TestColumn {
        fn field(&self) -> &Field {
            &self.field
        }
        fn is_materialized(&self) -> bool {
            true
        }
        fn len(&self) -> Option<usize> {
            Some(self.len)
        }
        fn rename(mut self, name: impl Into<String>) -> Self {
            self.field = self.field.with_name(name);
            self
        }
        fn cast(mut self, data_type: DataType) -> Result<Self, ColumnError> {
            self.field = self.field.with_data_type(data_type);
            Ok(self)
        }
        fn slice(mut self, offset: usize, length: usize) -> Result<Self, ColumnError> {
            let end = offset.saturating_add(length).min(self.len);
            self.len = end - offset.min(end);
            Ok(self)
        }
        fn tail(self, n: usize) -> Result<Self, ColumnError> {
            let len = self.len;
            self.slice(len.saturating_sub(n), n)
        }
    }

    /// A minimal eager frame to exercise the trait's provided methods.
    struct TestFrame {
        schema: Schema,
        rows: usize,
    }

    impl FrameHandle for TestFrame {
        fn schema(&self) -> Result<Schema, FrameError> {
            Ok(self.schema.clone())
        }
    }

    impl Frame for TestFrame {
        type Column = TestColumn;

        fn column(&self, name: &str) -> Result<TestColumn, FrameError> {
            let field = self
                .schema
                .field_by_name(name)
                .cloned()
                .ok_or_else(|| FrameError::ColumnNotFound(name.to_string()))?;
            Ok(TestColumn {
                field,
                len: self.rows,
            })
        }
        fn select<S: AsRef<str>>(self, columns: &[S]) -> Result<Self, FrameError> {
            let fields = columns
                .iter()
                .map(|name| {
                    self.schema
                        .field_by_name(name.as_ref())
                        .cloned()
                        .ok_or_else(|| FrameError::ColumnNotFound(name.as_ref().to_string()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(TestFrame {
                schema: Schema::new(fields),
                rows: self.rows,
            })
        }
        fn filter(self, predicate: Predicate) -> Result<Self, FrameError> {
            // `filter` does the job: type-optimise against the schema (which also
            // validates the columns), then apply — here a mock keeps all rows.
            let _typed = self.optimize_predicate(predicate)?;
            Ok(self)
        }
        fn limit(mut self, n: usize) -> Result<Self, FrameError> {
            self.rows = self.rows.min(n);
            Ok(self)
        }
        fn tail(mut self, n: usize) -> Result<Self, FrameError> {
            self.rows = self.rows.min(n);
            Ok(self)
        }
        fn slice(mut self, offset: usize, length: usize) -> Result<Self, FrameError> {
            self.rows = length.min(self.rows.saturating_sub(offset));
            Ok(self)
        }

        fn height(&self) -> Option<usize> {
            Some(self.rows)
        }
    }

    fn frame() -> TestFrame {
        TestFrame {
            schema: Schema::new(vec![
                Field::new("id", PrimitiveType::Int64.into(), false),
                Field::new("px", PrimitiveType::Float64.into(), true),
                Field::new("qty", PrimitiveType::Int64.into(), true),
            ]),
            rows: 10,
        }
    }

    #[test]
    fn structural_defaults_use_schema() {
        let f = frame();
        assert_eq!(f.width().unwrap(), 3);
        assert_eq!(f.column_names().unwrap(), ["id", "px", "qty"]);
        assert!(f.contains_column("px").unwrap());
        assert!(!f.contains_column("nope").unwrap());
        assert_eq!(f.height(), Some(10));
        assert_eq!(f.is_empty(), Some(false));
    }

    #[test]
    fn drop_defaults_to_select_complement() {
        let kept = frame().drop(&["px"]).unwrap();
        assert_eq!(kept.column_names().unwrap(), ["id", "qty"]);
    }

    #[test]
    fn head_defaults_to_limit() {
        assert_eq!(frame().head(3).unwrap().height(), Some(3));
        assert_eq!(frame().tail(2).unwrap().height(), Some(2));
        assert_eq!(frame().slice(1, 4).unwrap().height(), Some(4));
    }

    #[test]
    fn column_access_and_errors() {
        let col = frame().column("px").unwrap();
        assert_eq!(col.name(), "px");
        assert_eq!(col.data_type(), &DataType::from(PrimitiveType::Float64));
        assert!(matches!(
            frame().column("nope"),
            Err(FrameError::ColumnNotFound(_))
        ));
        assert!(matches!(
            frame().select(&["nope"]),
            Err(FrameError::ColumnNotFound(_))
        ));
    }

    #[test]
    fn filter_optimizes_against_the_schema() {
        // `filter` itself types the literal against the column (an untyped ISO
        // string vs a float column is accepted) and rejects an unknown column.
        assert!(frame()
            .filter(Predicate::gt("px", Scalar::any("100")))
            .is_ok());
        assert!(matches!(
            frame().filter(Predicate::eq("nope", Scalar::int64(1))),
            Err(FrameError::ColumnNotFound(_))
        ));
    }

    #[test]
    fn generic_pipeline_over_any_frame() {
        fn first_two<F: Frame>(frame: F) -> Result<F, FrameError> {
            frame.select(&["id", "px"])?.head(2)
        }
        let out = first_two(frame()).unwrap();
        assert_eq!(out.column_names().unwrap(), ["id", "px"]);
        assert_eq!(out.height(), Some(2));
    }
}
