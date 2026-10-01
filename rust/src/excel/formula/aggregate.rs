//! Ordered numeric accumulation shared by Excel formula and report consumers.

use super::super::cell::ExcelError;
use super::number;
use crate::Arithmetic;

/// One streaming aggregate over source values in their observed order.
///
/// A grouped subtotal is not a substitute for the original ordered inputs:
/// binary64 addition is not associative, and Excel corrects the final pair.
#[derive(Default)]
pub struct Accumulator {
    prefix: f64,
    last: Option<f64>,
    uncertain: bool,
}

impl Accumulator {
    /// Admit one already-resolved numeric input without retaining its row.
    pub fn push_number(&mut self, value: f64) -> Result<(), ExcelError> {
        let uncertain_input = value.is_subnormal();
        let value = number::finite(value)?;
        let prefix = match self.last {
            Some(last) => Arithmetic::Add.apply_float(self.prefix, last),
            None => self.prefix,
        };
        self.uncertain |= uncertain_input || prefix.is_subnormal();
        self.prefix = prefix;
        self.last = Some(value);
        Ok(())
    }

    /// Correct only the final pair of the ordered binary64 fold.
    pub fn finish_sum(self) -> Result<Option<f64>, ExcelError> {
        if self.uncertain {
            return Ok(None);
        }
        match self.last {
            Some(last) => number::add(self.prefix, last, true).transpose(),
            None => Ok(Some(0.0)),
        }
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! Primitive controls for the ordered accumulation owner.

    pub use super::Accumulator;
}
