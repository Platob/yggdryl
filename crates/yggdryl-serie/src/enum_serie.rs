//! [`EnumSerie`] — a categorical/enum view over a column: it scans the backing serie
//! once and holds the **mapping of unique values to their row index** (and to a compact
//! integer *code*), so a value can be looked up to its code or its first row, and a row
//! to its code.

use std::any::Any;
use std::collections::HashMap;

use arrow_array::ArrayRef;
use yggdryl_schema::Field;

use crate::scalar::Scalar;
use crate::serie::{Serie, SerieRef};

/// A canonical, hashable key for a [`Scalar`] (its `Debug` form distinguishes every
/// value and variant — `Int(5)` vs `Utf8("5")`). Floating-point `-0.0` is normalised to
/// `0.0` so the two (which compare equal) share a category.
fn key(value: &Scalar) -> String {
    match value {
        // `x == 0.0` is true for both `0.0` and `-0.0`; collapse them to one key.
        Scalar::Float(x) if *x == 0.0 => format!("{:?}", Scalar::Float(0.0)),
        other => format!("{other:?}"),
    }
}

/// An enum/categorical column: the distinct (non-null) values of a backing serie, in
/// first-seen order, each assigned a **code** (`0..unique_count`) and remembered with
/// its **first row index**. It is itself a [`Serie`], delegating data access to the
/// backing column.
///
/// ```
/// use yggdryl_serie::{EnumSerie, Scalar, VarcharSerie, Serie};
/// use std::sync::Arc;
///
/// let values = VarcharSerie::<i32>::from_values("c", vec![Some("a"), Some("b"), Some("a")]);
/// let enums = EnumSerie::from_serie(Arc::new(values));
/// assert_eq!(enums.unique_count(), 2);                          // "a", "b"
/// assert_eq!(enums.code(&Scalar::Utf8("b".into())), Some(1));   // enum code
/// assert_eq!(enums.first_row(&Scalar::Utf8("a".into())), Some(0));
/// assert_eq!(enums.code_at(2), Some(0));                        // row 2 holds "a"
/// ```
#[derive(Debug, Clone)]
pub struct EnumSerie {
    inner: SerieRef,
    uniques: Vec<Scalar>,
    code_of: HashMap<String, usize>,
    first_row: Vec<usize>,
}

impl EnumSerie {
    /// Scans `serie` and builds the enum mapping. Null cells are not categories (they
    /// keep their null in the backing column and have no code).
    pub fn from_serie(serie: SerieRef) -> EnumSerie {
        let mut uniques = Vec::new();
        let mut code_of: HashMap<String, usize> = HashMap::new();
        let mut first_row = Vec::new();
        for row in 0..serie.len() {
            let value = serie.value_at(row);
            if value.is_null() {
                continue;
            }
            code_of.entry(key(&value)).or_insert_with(|| {
                let code = uniques.len();
                uniques.push(value);
                first_row.push(row);
                code
            });
        }
        EnumSerie {
            inner: serie,
            uniques,
            code_of,
            first_row,
        }
    }

    /// The number of distinct (non-null) values.
    pub fn unique_count(&self) -> usize {
        self.uniques.len()
    }

    /// The distinct values, in first-seen order (indexed by their code).
    pub fn uniques(&self) -> &[Scalar] {
        &self.uniques
    }

    /// The enum **code** of `value` (`0..unique_count`), or `None` if it is not present.
    pub fn code(&self, value: &Scalar) -> Option<usize> {
        self.code_of.get(&key(value)).copied()
    }

    /// The **first row index** at which `value` appears, or `None` if it is not present.
    pub fn first_row(&self, value: &Scalar) -> Option<usize> {
        self.code(value).map(|code| self.first_row[code])
    }

    /// The value with the given `code`, or `None` if out of range.
    pub fn value_of(&self, code: usize) -> Option<&Scalar> {
        self.uniques.get(code)
    }

    /// The enum code of the value at row `index` (`None` for a null or out-of-bounds
    /// cell).
    pub fn code_at(&self, index: usize) -> Option<usize> {
        self.code(&self.inner.value_at(index))
    }

    /// The backing column.
    pub fn inner(&self) -> &SerieRef {
        &self.inner
    }
}

impl Serie for EnumSerie {
    fn field(&self) -> &Field {
        self.inner.field()
    }

    fn array(&self) -> ArrayRef {
        self.inner.array()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn len(&self) -> usize {
        self.inner.len()
    }

    fn null_count(&self) -> usize {
        self.inner.null_count()
    }

    fn is_null(&self, index: usize) -> bool {
        self.inner.is_null(index)
    }

    fn value_at(&self, index: usize) -> Scalar {
        self.inner.value_at(index)
    }
}
