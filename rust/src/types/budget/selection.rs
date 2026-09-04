//! Bounded selections over source-array rows.

use crate::arrow::{Error, Result};

#[derive(Clone, Copy)]
pub(crate) enum SourceSelection<'a> {
    Indices(&'a [u32]),
    Ranges(&'a [(usize, usize)]),
}

impl SourceSelection<'_> {
    pub(crate) fn row_count(self, upper_bound: usize) -> Result<usize> {
        let mut rows = 0usize;
        self.try_for_each(upper_bound, |_| {
            rows = rows.checked_add(1).ok_or_else(|| {
                Error::IncompatibleSchema(
                    "masked Arrow source selection length exceeds usize".to_owned(),
                )
            })?;
            Ok(())
        })?;
        Ok(rows)
    }

    pub(crate) fn try_for_each(
        self,
        upper_bound: usize,
        mut visit: impl FnMut(usize) -> Result<()>,
    ) -> Result<()> {
        match self {
            Self::Indices(indices) => {
                for index in indices {
                    let index = usize::try_from(*index).map_err(|_| {
                        Error::IncompatibleSchema(
                            "masked Arrow source index exceeds usize".to_owned(),
                        )
                    })?;
                    if index >= upper_bound {
                        return Err(Error::IncompatibleSchema(
                            "masked Arrow source index exceeds its array length".to_owned(),
                        ));
                    }
                    visit(index)?;
                }
            }
            Self::Ranges(ranges) => {
                for &(start, end) in ranges {
                    if start > end || end > upper_bound {
                        return Err(Error::IncompatibleSchema(
                            "masked Arrow source range exceeds its array length".to_owned(),
                        ));
                    }
                    for index in start..end {
                        visit(index)?;
                    }
                }
            }
        }
        Ok(())
    }
}
