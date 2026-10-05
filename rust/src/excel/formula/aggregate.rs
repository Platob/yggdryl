//! Streaming numeric and logical accumulation over resolved Excel operands.

use super::super::cell::ExcelError;
use super::functions::Function;
use super::number;
use crate::Arithmetic;

/// A resolved spreadsheet aggregate, shared by formula reductions, PivotTables,
/// and summary views. Its spelling is the request and OOXML vocabulary.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Aggregate {
    /// Sum numeric inputs.
    Sum,
    /// Count nonempty inputs.
    Count,
    /// Average numeric inputs.
    Average,
    /// Maximum numeric input.
    Max,
    /// Minimum numeric input.
    Min,
    /// Product of numeric inputs.
    Product,
    /// Count numeric inputs.
    CountNumbers,
    /// Sample standard deviation.
    StdDev,
    /// Population standard deviation.
    StdDevP,
    /// Sample variance.
    Var,
    /// Population variance.
    VarP,
}

impl Aggregate {
    const NAMES: [(&'static str, Self); 11] = [
        ("sum", Self::Sum),
        ("count", Self::Count),
        ("average", Self::Average),
        ("max", Self::Max),
        ("min", Self::Min),
        ("product", Self::Product),
        ("countNumbers", Self::CountNumbers),
        ("stdDev", Self::StdDev),
        ("stdDevP", Self::StdDevP),
        ("var", Self::Var),
        ("varP", Self::VarP),
    ];

    /// The one spelling of this aggregate in typed request documents.
    pub fn as_str(self) -> &'static str {
        Self::NAMES
            .iter()
            .find(|(_, kind)| *kind == self)
            .map(|(name, _)| *name)
            .expect("every aggregate is in its one name table")
    }

    /// ST_DataConsolidateFunction spelling of the resolved aggregate.
    pub(crate) fn subtotal(self) -> &'static str {
        match self {
            Self::CountNumbers => "countNums",
            Self::StdDevP => "stdDevp",
            Self::VarP => "varp",
            _ => self.as_str(),
        }
    }

    /// Resolve the OOXML spelling through the same table as publication.
    pub(crate) fn from_subtotal(name: &str) -> Option<Self> {
        Self::NAMES
            .iter()
            .map(|(_, kind)| *kind)
            .find(|kind| kind.subtotal() == name)
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::NAMES
            .iter()
            .find(|(held, _)| *held == name)
            .map(|(_, kind)| *kind)
    }
}

/// One streaming aggregate over source values in their observed order.
///
/// A grouped subtotal is not a substitute for the original ordered inputs:
/// binary64 addition is not associative, and Excel corrects the final pair.
#[derive(Default)]
pub struct Accumulator {
    prefix: f64,
    last: Option<f64>,
    uncertain: bool,
    count: u64,
    counta: u64,
    min: f64,
    max: f64,
    product: Option<f64>,
    ranks: Option<Vec<(f64, usize)>>,
    squares: Option<u64>,
    discount: Option<(f64, f64)>,
}

impl Accumulator {
    /// Collect numeric ranks only for MEDIAN/MODE; ordinary folds keep no rows.
    pub fn ranked() -> Self {
        Self {
            ranks: Some(Vec::new()),
            ..Self::default()
        }
    }

    /// Admit one already-resolved numeric input without retaining its row.
    pub fn push_number(&mut self, value: f64) -> Result<(), ExcelError> {
        if self.squares.is_some() {
            return self.push_variance(value);
        }
        if let Some(ranks) = &mut self.ranks {
            if value.is_subnormal() {
                self.uncertain = true;
                return Ok(());
            }
            let value = number::finite(value)?;
            if ranks.try_reserve(1).is_err() {
                self.uncertain = true;
                return Ok(());
            }
            ranks.push((value, ranks.len()));
            return Ok(());
        }
        let uncertain_input = value.is_subnormal();
        let value = number::finite(value)?;
        let count = self.count.checked_add(1).ok_or(ExcelError::Num)?;
        let counta = self.counta.checked_add(1).ok_or(ExcelError::Num)?;
        let prefix = match self.last {
            Some(last) => Arithmetic::Add.apply_float(self.prefix, last),
            None => self.prefix,
        };
        // Extrema compare the actual finite binary values. Excel MIN/MAX
        // distinguish close values that its comparison operator calls equal.
        self.min = if self.count == 0 {
            value
        } else {
            self.min.min(value)
        };
        self.max = if self.count == 0 {
            value
        } else {
            self.max.max(value)
        };
        self.count = count;
        self.counta = counta;
        self.uncertain |= uncertain_input || prefix.is_subnormal();
        self.prefix = prefix;
        self.last = Some(value);
        Ok(())
    }

    /// Admit a compact run of positive zeros. The first zero moves the
    /// preceding last term into the prefix; the second stabilizes signed
    /// zero. Every later addition of +0 leaves that finite prefix unchanged.
    pub fn push_zero_n(&mut self, count: u64) -> Result<(), ExcelError> {
        debug_assert!(self.ranks.is_none(), "zero runs belong to streaming folds");
        let total = self.count.checked_add(count).ok_or(ExcelError::Num)?;
        let total_present = self.counta.checked_add(count).ok_or(ExcelError::Num)?;
        for _ in 0..count.min(2) {
            self.push_number(0.0)?;
        }
        self.count = total;
        self.counta = total_present;
        Ok(())
    }

    /// Ordered product uses the shared floating kernel. Failure preserves the
    /// previous prefix; the evaluator retains the first numeric error.
    /// A subnormal product is published zero at this observed boundary.
    pub fn push_product(&mut self, value: f64) -> Result<(), ExcelError> {
        let value = number::finite(value)?;
        let next = match self.product {
            Some(prior) => number::finite(Arithmetic::Mul.apply_float(prior, value))?,
            None => value,
        };
        self.product = Some(next);
        Ok(())
    }

    /// The k-th item of an ordered source. Native LARGE indexes from
    /// `trunc(n-k)` while SMALL indexes from `trunc(k-1)`; both validate
    /// the untruncated k against 1..=n first.
    pub fn finish_kth(self, k: f64, largest: bool) -> Result<Option<f64>, ExcelError> {
        if self.uncertain {
            return Ok(None);
        }
        let mut ranks = self.ranks.expect("ranked accumulator construction");
        let n = ranks.len() as f64;
        if !k.is_finite() || k < 1.0 || k > n {
            return Err(ExcelError::Num);
        }
        let index = if largest {
            (n - k).trunc()
        } else {
            (k - 1.0).trunc()
        } as usize;
        // Selection is linear and rearranges the one retained vector in place.
        let (_, selected, _) =
            ranks.select_nth_unstable_by(index, |left, right| left.0.total_cmp(&right.0));
        number::finite(selected.0).map(Some)
    }

    /// Inclusive percentile over the same retained rank values. The native
    /// one-based coordinate is formed before subtracting its integer part:
    /// zero-based `(n-1)*k` loses an observed ULP for decimal k.
    pub fn finish_percentile(self, k: f64) -> Result<Option<f64>, ExcelError> {
        if self.uncertain {
            return Ok(None);
        }
        let mut ranks = self.ranks.expect("ranked accumulator construction");
        if ranks.is_empty() || !k.is_finite() || !(0.0..=1.0).contains(&k) {
            return Err(ExcelError::Num);
        }
        let coordinate = Arithmetic::Add.apply_float(
            1.0,
            Arithmetic::Mul.apply_float((ranks.len() - 1) as f64, k),
        );
        let floor = coordinate.floor();
        let low = floor as usize - 1;
        let (_, selected, above) =
            ranks.select_nth_unstable_by(low, |left, right| left.0.total_cmp(&right.0));
        let lower = selected.0;
        let fraction = Arithmetic::Sub.apply_float(coordinate, floor);
        if fraction == 0.0 {
            return number::finite(lower).map(Some);
        }
        let upper = above
            .iter()
            .map(|entry| entry.0)
            .min_by(f64::total_cmp)
            .expect("a fractional coordinate has an upper neighbour");
        let difference = Arithmetic::Sub.apply_float(upper, lower);
        if difference.is_subnormal() {
            return Ok(None);
        }
        let scaled = Arithmetic::Mul.apply_float(difference, fraction);
        if scaled.is_subnormal() {
            return Ok(None);
        }
        let value = Arithmetic::Add.apply_float(lower, scaled);
        if value.is_subnormal() {
            return Ok(None);
        }
        number::finite(value).map(Some)
    }

    /// One-based rank of a member present in the source. Ties take their
    /// first position. A nonzero order selects ascending, including a
    /// negative or fractional order argument.
    pub fn finish_rank(self, target: f64, ascending: bool) -> Result<Option<f64>, ExcelError> {
        if self.uncertain || target.is_subnormal() {
            return Ok(None);
        }
        let ranks = self.ranks.expect("ranked accumulator construction");
        let target = number::finite(target)?;
        if !ranks.iter().any(|(value, _)| *value == target) {
            return Err(ExcelError::NA);
        }
        let before = ranks
            .iter()
            .filter(|(value, _)| {
                if ascending {
                    *value < target
                } else {
                    *value > target
                }
            })
            .count();
        number::finite((before + 1) as f64).map(Some)
    }

    /// The median of admitted binary64 numbers. Excel halves the
    /// difference before adding the lower middle value; each intermediate
    /// passes the published numeric cache boundary.
    pub fn finish_median(self) -> Result<Option<f64>, ExcelError> {
        if self.uncertain {
            return Ok(None);
        }
        let mut ranks = self.ranks.expect("ranked accumulator construction");
        if ranks.is_empty() {
            return Err(ExcelError::Num);
        }
        ranks.sort_unstable_by(|left, right| left.0.total_cmp(&right.0));
        let upper = ranks[ranks.len() / 2].0;
        if ranks.len() % 2 == 1 {
            return number::finite(upper).map(Some);
        }
        let lower = ranks[ranks.len() / 2 - 1].0;
        let difference = number::finite(crate::Arithmetic::Sub.apply_float(upper, lower))?;
        let half = number::finite(crate::Arithmetic::Div.apply_float(difference, 2.0))?;
        let middle = crate::Arithmetic::Add.apply_float(lower, half);
        if middle.is_subnormal() {
            return Ok(None);
        }
        number::finite(middle).map(Some)
    }

    /// Most frequent admitted binary64 number; equal-frequency ties keep the
    /// first source occurrence. No repeated value is #N/A.
    pub fn finish_mode(self) -> Result<Option<f64>, ExcelError> {
        if self.uncertain {
            return Ok(None);
        }
        let mut ranks = self.ranks.expect("ranked accumulator construction");
        if ranks.is_empty() {
            return Err(ExcelError::NA);
        }
        ranks.sort_unstable_by(|left, right| left.0.total_cmp(&right.0));
        let mut best = (0usize, 0usize, usize::MAX);
        let mut start = 0;
        while start < ranks.len() {
            let mut end = start + 1;
            let mut first = ranks[start].1;
            while end < ranks.len() && ranks[end].0 == ranks[start].0 {
                first = first.min(ranks[end].1);
                end += 1;
            }
            if end - start > best.1 || (end - start == best.1 && first < best.2) {
                best = (start, end - start, first);
            }
            start = end;
        }
        if best.1 < 2 {
            return Err(ExcelError::NA);
        }
        number::finite(ranks[best.0].0).map(Some)
    }

    /// Return the product or zero when no numeric operand was admitted.
    pub fn finish_product(self) -> f64 {
        self.product.unwrap_or(0.0)
    }

    /// Admit a nonempty value when only its presence is requested.
    pub fn push_present(&mut self) -> Result<(), ExcelError> {
        self.push_count_n(1)
    }

    /// Count one matched sparse run without visiting every absent cell.
    pub fn push_count_n(&mut self, count: u64) -> Result<(), ExcelError> {
        self.counta = self.counta.checked_add(count).ok_or(ExcelError::Num)?;
        Ok(())
    }

    /// Exact numeric count; larger counts need a settled rounding rule.
    pub fn finish_count(self) -> Option<f64> {
        (self.count <= (1u64 << 53)).then_some(self.count as f64)
    }

    /// Exact nonempty count, including text and errors.
    pub fn finish_counta(self) -> Option<f64> {
        (self.counta <= (1u64 << 53)).then_some(self.counta as f64)
    }

    /// Smallest finite numeric input, or zero when no number was admitted.
    pub fn finish_min(self) -> f64 {
        self.min
    }

    /// Largest finite numeric input, or zero when no number was admitted.
    pub fn finish_max(self) -> f64 {
        self.max
    }

    /// Divide the observed ordered sum by its exact numeric input count.
    /// A subnormal final quotient keeps the prior cache until its raw/cache
    /// boundary is independently settled.
    pub fn finish_average(self) -> Result<Option<f64>, ExcelError> {
        let count = self.count;
        if count == 0 {
            return Err(ExcelError::Div0);
        }
        if count > (1u64 << 53) {
            return Ok(None);
        }
        match self.finish_sum()? {
            Some(total) => {
                let value = Arithmetic::Div.apply_float(total, count as f64);
                if value.is_subnormal() {
                    Ok(None)
                } else {
                    number::finite(value).map(Some)
                }
            }
            None => Ok(None),
        }
    }

    /// NPV folds terms starting at exponent zero, then discounts the sum once.
    /// Only this mode holds a growth factor and running denominator; prefix
    /// and uncertain are shared with the other constant-space numeric folds.
    pub fn discounted(rate: f64) -> Self {
        Self {
            discount: Some((Arithmetic::Add.apply_float(1.0, rate), 1.0)),
            uncertain: rate.is_subnormal() || !rate.is_finite(),
            ..Self::default()
        }
    }

    /// Admit a cash flow in source order without SUM's final-pair correction.
    pub fn push_discounted(&mut self, value: f64) -> Result<(), ExcelError> {
        let (growth, denominator) = self.discount.as_mut().expect("discounted construction");
        if *denominator == 0.0 && *growth == 0.0 {
            return Err(ExcelError::Div0);
        }
        if value.is_subnormal()
            || *denominator == 0.0
            || denominator.is_subnormal()
            || !denominator.is_finite()
        {
            self.uncertain = true;
            return Ok(());
        }
        let value = number::finite(value)?;
        let discounted = Arithmetic::Div.apply_float(value, *denominator);
        let sum = Arithmetic::Add.apply_float(self.prefix, discounted);
        if discounted.is_subnormal() || sum.is_subnormal() {
            self.uncertain = true;
        }
        let sum = number::finite(sum)?;
        self.prefix = sum;
        // An overflowing next denominator only matters if another flow arrives.
        *denominator = Arithmetic::Mul.apply_float(*denominator, *growth);
        Ok(())
    }

    /// Finish the original ordered cash-flow fold; no terms are retained.
    pub fn finish_discounted(self) -> Result<Option<f64>, ExcelError> {
        if self.uncertain {
            return Ok(None);
        }
        let (growth, _) = self.discount.expect("discounted construction");
        if growth == 0.0 {
            return Err(ExcelError::Div0);
        }
        let value = Arithmetic::Div.apply_float(self.prefix, growth);
        if value.is_subnormal() {
            return Ok(None);
        }
        number::finite(value).map(Some)
    }
    /// Use exact integral moments only; uncertain inputs retain the old cache.
    /// The range walker and input-origin rules remain the ordinary aggregate's.
    pub fn variance() -> Self {
        Self {
            squares: Some(0),
            ..Self::default()
        }
    }

    fn push_variance(&mut self, value: f64) -> Result<(), ExcelError> {
        const EXACT: i128 = 1i128 << 53;
        if !value.is_finite() {
            return Err(ExcelError::Num);
        }
        self.count = self.count.checked_add(1).ok_or(ExcelError::Num)?;
        if self.uncertain {
            return Ok(());
        }
        if value.fract() != 0.0 || value.abs() > EXACT as f64 {
            self.uncertain = true;
            return Ok(());
        }
        // Check before casting back: 2^53+1 would otherwise round to the
        // admissible endpoint. i128 holds every admitted operand's square.
        let integer = value as i128;
        let sum = self.prefix as i128 + integer;
        let squares = i128::from(self.squares.expect("variance construction")) + integer * integer;
        if sum.abs() > EXACT || squares > EXACT {
            self.uncertain = true;
            return Ok(());
        }
        self.prefix = sum as f64;
        self.squares = Some(squares as u64);
        Ok(())
    }

    /// Exact raw and centered sums agree inside this representability domain.
    /// A dyadic mean makes centered differences integral after power-of-two
    /// scaling. Their squared sum is bounded by the scaled sum of squares.
    /// All raw/population intermediates are exact too. Only the final division
    /// rounds; no magnitude-dependent choice of variance algorithm is made.
    pub fn finish_variance(self, sample: bool) -> Result<Option<f64>, ExcelError> {
        const EXACT: i128 = 1i128 << 53;
        if self.count <= u64::from(sample) {
            return Err(ExcelError::Div0);
        }
        if self.uncertain {
            return Ok(None);
        }
        let count = i128::from(self.count);
        if count > EXACT {
            return Ok(None);
        }
        let sum = self.prefix as i128;
        let squares = i128::from(self.squares.expect("variance construction"));
        let odd_count = self.count >> self.count.trailing_zeros();
        if sum % i128::from(odd_count) != 0
            || sum * sum > EXACT
            || count * squares > EXACT
            || count * count > EXACT
        {
            return Ok(None);
        }
        let fractional_bits = self
            .count
            .trailing_zeros()
            .saturating_sub(sum.unsigned_abs().trailing_zeros());
        let scale_squared = 1i128 << (2 * fractional_bits);
        if scale_squared > EXACT || squares > EXACT / scale_squared {
            return Ok(None);
        }
        let correction = Arithmetic::Div.apply_float((sum * sum) as f64, count as f64);
        let centered = Arithmetic::Sub.apply_float(squares as f64, correction);
        debug_assert!(centered >= 0.0 && centered <= EXACT as f64);
        let value = Arithmetic::Div.apply_float(centered, (self.count - u64::from(sample)) as f64);
        number::finite(value).map(Some)
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

/// One constant-space fold; errors and reference inclusion stay with intake.
/// Keeping all three bits avoids per-input function dispatch and count overflow.
pub(crate) struct LogicalAccumulator {
    seen: bool,
    all: bool,
    any: bool,
    odd: bool,
}

impl Default for LogicalAccumulator {
    fn default() -> Self {
        Self {
            seen: false,
            all: true,
            any: false,
            odd: false,
        }
    }
}

impl LogicalAccumulator {
    pub(crate) fn push(&mut self, value: bool) {
        self.seen = true;
        self.all &= value;
        self.any |= value;
        self.odd ^= value;
    }

    pub(crate) fn finish(self, function: Function) -> Result<bool, ExcelError> {
        if !self.seen {
            return Err(ExcelError::Value);
        }
        Ok(match function {
            Function::And => self.all,
            Function::Or => self.any,
            Function::Xor => self.odd,
            _ => unreachable!("logical intake admits only AND, OR and XOR"),
        })
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! Primitive controls for the ordered accumulation owner.

    pub use super::Accumulator;

    /// Exercise the constant-space native discount fold without an evaluator.
    pub fn discounted(rate: f64, values: &[f64]) -> Result<Option<f64>, super::ExcelError> {
        let mut fold = super::Accumulator::discounted(rate);
        for &value in values {
            fold.push_discounted(value)?;
        }
        fold.finish_discounted()
    }
}
